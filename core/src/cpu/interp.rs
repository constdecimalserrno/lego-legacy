//! The interpreter, and the cache between decoding and executing (ADR-0004).
//!
//! Decoding costs several times what executing costs, and guest code runs the
//! same bytes over and over, so an instruction is decoded once per address and
//! executed from a compact 16-byte [`FastInsn`] record every time after (#253).

use super::decode::{self, MAX_INSN};
use super::insn::{FastInsn, Insn, Kind};
use super::x87::X87Op;
use super::{exec, Cpu, CpuEngine, Mode, Stop, CS};
use crate::guest::{Arena, GuestMem};
use crate::shim::Boundary;

/// One decoded straight-line run inside [`Interp::fast_insns`].
#[derive(Clone, Copy, Debug)]
struct Run {
    /// Index of the run's first instruction in `fast_insns`.
    first: u32,
    /// Total encoded x86 byte length across all instructions in this run.
    bytes: u16,
    /// Number of instructions in this run (`0` means invalidated by a code write).
    len: u8,
    /// Whether the final instruction can transfer control (`transfers(kind)`).
    ends_with_transfer: bool,
}

/// A decode-cached interpreter.
///
/// **Runs are not linked to one another, and that was measured (#71).** A
/// taken branch leaves the loop and comes back through `Interp::run_at`, and
/// recording a direct branch's target run so the loop steps straight to it
/// would save exactly that lookup. Measured by making the loop pay for the
/// lookup twice — the same run, the same results — over Alpha Team's menu
/// route: 42.8 s against 44.2 s, so **the whole lookup is 3.3% of the run**.
/// Linking only removes it for direct transfers whose target is already
/// decoded: `ret` and every indirect jump and call still look up, and those
/// are about six instructions in every hundred against nine or ten direct
/// ones. So the realistic saving is nearer 2%.
///
/// Against that, a link must never reach a run a write has killed, and Alpha
/// Team — the title this is for — writes over its own code pages thousands of
/// times a second. Two percent is not worth a new way for that to go wrong.
pub struct Interp {
    mode: Mode,
    /// Decoded instructions in compact 16-byte flat32 IR, laid out in runs of
    /// straight-line code through the next transfer of control (#253).
    //
    // A write to a page this has decoded out of drops the runs decoded from
    // that page (#65), which is what a title that loads its own code needs:
    // Alpha Team unpacks DLLs out of its archives into memory it has run
    // before (#58).
    fast_insns: Vec<FastInsn>,
    /// Cold full [`Insn`] records referenced only by [`FastOp::Slow`] entries.
    slow_insns: Vec<Insn>,
    /// Compact 2-byte pre-decoded x87 ops referenced by `X87Reg`/`X87Mem`.
    x87_ops: Vec<X87Op>,
    /// Metadata for each cached run.
    runs: Vec<Run>,
    /// Where each run starts, by the linear address of its first instruction,
    /// as a page table: for each of the 1,024 regions of 4 MiB, its table's
    /// place in `regions` plus one; in a region's table, for each of its 1,024
    /// pages of 4 KiB, the page's place in `pages` plus one; and in a page, the
    /// run's place in `runs` plus one. Zero is nothing there yet. Loads rather
    /// than a hash, because this is paid for every run entered, and small
    /// enough to make an engine for one test vector.
    directory: Box<[u32; 1024]>,
    regions: Vec<Box<[u32; 1024]>>,
    pages: Vec<Box<[u32; PAGE]>>,
    /// Which runs were decoded out of each of `pages`, so that a write over
    /// one page kills those runs and leaves every other run standing (#65).
    /// A run spanning two pages is in both their lists.
    page_runs: Vec<Vec<u32>>,
}

const PAGE: usize = 1 << 12;

/// The most instructions one run is decoded to.
const MOST_IN_A_RUN: u8 = 64;

/// The most decoded instructions kept before the cache is thrown away whole.
const MOST_KEPT: usize = 1 << 20;

impl Interp {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            fast_insns: Vec::new(),
            slow_insns: Vec::new(),
            x87_ops: Vec::new(),
            runs: Vec::new(),
            directory: Box::new([0; 1024]),
            regions: Vec::new(),
            pages: Vec::new(),
            page_runs: Vec::new(),
        }
    }

    /// How many instructions this engine has decoded and kept.
    #[must_use]
    pub fn cached(&self) -> usize {
        self.fast_insns.len()
    }

    /// Forget every decoded instruction: what `run_at` does when more pages
    /// were written over than the arena keeps a list of, and when the cache
    /// reaches its ceiling.
    pub fn forget(&mut self) {
        self.fast_insns.clear();
        self.slow_insns.clear();
        self.x87_ops.clear();
        self.runs.clear();
        self.pages.clear();
        self.page_runs.clear();
        self.regions.clear();
        self.directory.fill(0);
    }

    /// Forget only what was decoded out of one page, which is what a write
    /// over it actually invalidated (#65).
    ///
    /// A run is killed by giving it no instructions: whatever still points at
    /// it falls through and decodes again. The page stops being watched, so
    /// the next write over it costs nothing until code is decoded there
    /// again — a title that keeps its data on the same page as its code, as
    /// Alpha Team's own loader does, writes over that page thousands of times
    /// a second.
    fn forget_page(&mut self, page: u32, mem: &mut Arena) {
        mem.unmark_code(page.saturating_mul(PAGE as u32));
        let lin = page.saturating_mul(PAGE as u32);
        let (region, within) = ((lin >> 22) as usize, (lin >> 12) as usize & 0x3ff);
        let Some(r) = self.directory.get(region).copied().filter(|r| *r != 0) else {
            return;
        };
        let Some(p) = self.regions[r as usize - 1]
            .get(within)
            .copied()
            .filter(|p| *p != 0)
        else {
            return;
        };
        for run in std::mem::take(&mut self.page_runs[p as usize - 1]) {
            if let Some(entry) = self.runs.get_mut(run as usize) {
                entry.len = 0;
            }
        }
    }

    /// The table for the page an address is on, made if it is not there yet.
    fn table_for(&mut self, lin: u32) -> usize {
        let (region, within) = ((lin >> 22) as usize, (lin >> 12) as usize & 0x3ff);
        let place = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
        if self.directory[region] == 0 {
            self.regions.push(Box::new([0; 1024]));
            self.directory[region] = place(self.regions.len());
        }
        let r = self.directory[region] as usize - 1;
        if self.regions[r][within] == 0 {
            self.pages.push(Box::new([0; PAGE]));
            self.page_runs.push(Vec::new());
            self.regions[r][within] = place(self.pages.len());
        }
        self.regions[r][within] as usize - 1
    }

    /// The run of instructions starting at a linear address, decoding it if
    /// this is the first time the engine has stood there.
    fn run_at(&mut self, lin: u32, mem: &mut Arena) -> Result<Run, Stop> {
        // Bytes have been written over something this decoded (#58): only the
        // pages written over are dropped (#65).
        if std::mem::take(&mut mem.code_written) {
            if std::mem::take(&mut mem.written_all) {
                mem.written_code.clear();
                self.forget();
            } else {
                for page in std::mem::take(&mut mem.written_code) {
                    self.forget_page(page, mem);
                }
            }
        }
        // A title that writes over its code as often as it runs it decodes
        // the same addresses again and again, and nothing here is ever
        // reclaimed until everything is. This is the ceiling.
        //
        // ponytail: a million instructions, thrown away whole when it is
        // reached; a free list per run would keep it flat if a title ever
        // makes this the cost that shows. **Checked against the title that
        // ought to make it show (#83).** Alpha Team rewrites its own code
        // pages thousands of times a second, and over both its committed
        // routes this is reached exactly zero times: the menu route ends
        // holding 292,904 instructions and the first-mission route 311,647,
        // against the 1,048,576 here — under a third of it. Per-page
        // forgetting (#65) kills a run's slot without reclaiming its
        // instructions, so `fast_insns` only grows, and it still does not grow
        // this far. The free list stays unwritten.
        if self.fast_insns.len() > MOST_KEPT {
            self.forget();
        }
        let (region, page, slot) = (
            (lin >> 22) as usize,
            (lin >> 12) as usize & 0x3ff,
            lin as usize & (PAGE - 1),
        );
        let hit = match self.directory[region] as usize {
            0 => 0,
            r => match self.regions[r - 1][page] as usize {
                0 => 0,
                p => self.pages[p - 1][slot],
            },
        };
        if hit != 0 {
            // A run with no instructions is one a write killed: whatever
            // still points at it decodes again (#65).
            let run = self.runs[hit as usize - 1];
            if run.len != 0 {
                return Ok(run);
            }
        }
        let first = u32::try_from(self.fast_insns.len()).unwrap_or(u32::MAX);
        let mut at = lin;
        let mut count = 0u8;
        let mut bytes = 0u16;
        let mut ends_with_transfer = false;
        loop {
            // Never decode a non-initial instruction inside the boundary range.
            if count > 0 && Boundary::RANGE.contains(&at) {
                break;
            }
            let insn = match decode_at(at, mem, self.mode) {
                Ok(insn) => insn,
                // Only the instruction the guest stands on has to decode: one
                // further along may be data after a branch the guest takes.
                Err(stop) if count == 0 => return Err(stop),
                Err(_) => break,
            };
            let fast =
                exec::compile_fast(&insn, self.mode, &mut self.slow_insns, &mut self.x87_ops);
            self.fast_insns.push(fast);
            bytes += u16::from(insn.len);
            count += 1;
            at = at.wrapping_add(u32::from(insn.len));
            if transfers(insn.kind) {
                ends_with_transfer = true;
                break;
            }
            if count >= MOST_IN_A_RUN {
                break;
            }
        }
        let run = Run {
            first,
            bytes,
            len: count,
            ends_with_transfer,
        };
        self.runs.push(run);
        let index = u32::try_from(self.runs.len() - 1).unwrap_or(u32::MAX);
        let here = self.table_for(lin);
        self.pages[here][slot] = index + 1;
        // Every page this run was decoded out of is now watched, and knows
        // this run came out of it.
        let mut watched = lin;
        loop {
            mem.mark_code(watched);
            let table = self.table_for(watched);
            self.page_runs[table].push(index);
            if watched / 4096 >= at.wrapping_sub(1) / 4096 {
                break;
            }
            watched = (watched / 4096 + 1) * 4096;
        }
        Ok(run)
    }
}

/// Whether an instruction can send the guest somewhere other than the next
/// one, which is where a run of straight-line code ends.
fn transfers(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Jmp
            | Kind::Jcc(_)
            | Kind::Call
            | Kind::Ret
            | Kind::Loop { .. }
            | Kind::Jecxz
            | Kind::Int(_)
            | Kind::Hlt
            | Kind::MovSegTo
            | Kind::Pop0f
    )
}

/// Decode the instruction at a linear address.
fn decode_at(lin: u32, mem: &Arena, mode: Mode) -> Result<Insn, Stop> {
    // The window is as much as an instruction can be, or as much as is left
    // before the end of the arena.
    let room = (mem.len() - (lin as usize).min(mem.len())).min(MAX_INSN);
    let bytes = mem.slice(lin, room).map_err(Stop::Fault)?;
    let insn = decode::decode(bytes, mode).map_err(|u| Stop::Undecoded {
        eip: lin,
        byte: u.byte,
        group: u.group,
        why: u.why,
    })?;
    if usize::from(insn.len) > room {
        return Err(Stop::Fault(crate::guest::Fault {
            addr: lin,
            len: usize::from(insn.len),
            write: false,
        }));
    }
    Ok(insn)
}

#[cold]
#[inline(never)]
fn sum_lengths(insns: &[FastInsn]) -> u32 {
    insns.iter().map(|ins| u32::from(ins.len)).sum()
}

#[cold]
#[inline(never)]
fn clamp_to_outside(insns: &[FastInsn], start_eip: u32, outside: &std::ops::Range<u32>) -> usize {
    let mut eip = start_eip;
    let mut limit = 0;
    for insn in insns {
        if outside.contains(&eip) {
            break;
        }
        eip = eip.wrapping_add(u32::from(insn.len));
        limit += 1;
    }
    limit
}

impl CpuEngine for Interp {
    fn step(&mut self, cpu: &mut Cpu, mem: &mut Arena) -> Result<(), Stop> {
        let lin = if self.mode.real {
            cpu.seg_base[CS].wrapping_add(cpu.eip)
        } else {
            cpu.eip
        };
        let run = self.run_at(lin, mem)?;
        let insn = &self.fast_insns[run.first as usize];
        let at = cpu.eip;
        // The instruction pointer moves before the instruction runs, so that a
        // relative branch is relative to what comes after it.
        // The instruction pointer is not truncated on the way past an
        // instruction, even in real mode: a 386 lets eip run past 64K and only
        // a branch with a sixteen-bit operand size brings it back.
        cpu.eip = at.wrapping_add(u32::from(insn.len));
        match exec::execute_one_fast(insn, cpu, mem, &self.slow_insns, &self.x87_ops, self.mode) {
            Ok(()) => Ok(()),
            // A halt or an interrupt retires and then stops the run: the
            // instruction pointer belongs after it, which is where a return
            // address would point.
            Err(stop @ (Stop::Halted | Stop::Interrupt(_))) => Err(stop),
            Err(stop) => {
                // A wall leaves the cursor where the guest stood, so the run
                // log names the instruction that could not be retired rather
                // than the one after it.
                cpu.eip = at;
                Err(stop)
            }
        }
    }

    fn run(
        &mut self,
        cpu: &mut Cpu,
        mem: &mut Arena,
        count: u64,
        outside: std::ops::Range<u32>,
    ) -> (u64, Result<(), Stop>) {
        let mut retired = 0u64;
        while retired < count && !outside.contains(&cpu.eip) {
            let lin = if self.mode.real {
                cpu.seg_base[CS].wrapping_add(cpu.eip)
            } else {
                cpu.eip
            };
            let run = match self.run_at(lin, mem) {
                Ok(run) => run,
                Err(stop) => return (retired, Err(stop)),
            };
            let start = run.first as usize;
            let len = usize::from(run.len);
            let run_insns = &self.fast_insns[start..start + len];
            let mut steps = (len as u64).min(count - retired) as usize;
            let start_eip = cpu.eip;

            // Hoist the `outside` range check once per run: unless this run's
            // straight-line EIP span overlaps `outside` (or wraps u32), no
            // instruction inside it can land in `outside`.
            if !outside.is_empty()
                && start_eip
                    .checked_add(u32::from(run.bytes))
                    .is_none_or(|end| start_eip < outside.end && end > outside.start)
            {
                steps = clamp_to_outside(&run_insns[..steps], start_eip, &outside);
                if steps == 0 {
                    break;
                }
            }

            let has_terminal = run.ends_with_transfer && steps == len;
            let straight_len = if has_terminal { steps - 1 } else { steps };
            let base_tsc = cpu.tsc;

            // Non-terminal straight-line instructions are guaranteed by
            // `transfers()` not to modify EIP or CS, so per-instruction count,
            // boundary, EIP, and CS checks are hoisted out of this loop.
            for (i, insn) in run_insns[..straight_len].iter().enumerate() {
                if let Err(stop) = exec::execute_straight(
                    insn,
                    cpu,
                    mem,
                    &self.slow_insns,
                    &self.x87_ops,
                    self.mode,
                    base_tsc + i as u64,
                ) {
                    cpu.eip = start_eip.wrapping_add(sum_lengths(&run_insns[..i]));
                    cpu.tsc = base_tsc + i as u64;
                    return (retired + i as u64, Err(stop));
                }
            }

            let done = straight_len as u64;
            if has_terminal {
                let term = &run_insns[straight_len];
                let at = start_eip.wrapping_add(u32::from(run.bytes) - u32::from(term.len));
                cpu.eip = at.wrapping_add(u32::from(term.len));
                match exec::execute_terminal(
                    term,
                    cpu,
                    mem,
                    &self.slow_insns,
                    self.mode,
                    base_tsc + done,
                ) {
                    Ok(()) => {
                        let total = done + 1;
                        retired += total;
                        cpu.tsc = base_tsc + total;
                    }
                    Err(stop @ Stop::Interrupt(_)) => {
                        cpu.tsc = base_tsc + done;
                        return (retired + done + 1, Err(stop));
                    }
                    Err(stop @ Stop::Halted) => {
                        cpu.tsc = base_tsc + done;
                        return (retired + done, Err(stop));
                    }
                    Err(stop) => {
                        cpu.eip = at;
                        cpu.tsc = base_tsc + done;
                        return (retired + done, Err(stop));
                    }
                }
            } else {
                let advance = if steps == len {
                    u32::from(run.bytes)
                } else {
                    sum_lengths(&run_insns[..straight_len])
                };
                cpu.eip = start_eip.wrapping_add(advance);
                retired += done;
                cpu.tsc = base_tsc + done;
            }
        }
        (retired, Ok(()))
    }

    fn name(&self) -> &'static str {
        "interp"
    }
}

/// Decode one instruction, for a caller that wants to look rather than run.
///
/// # Errors
/// When the bytes are not an instruction this engine has been taught.
pub fn decode_one(bytes: &[u8], mode: Mode) -> Result<Insn, decode::Undecoded> {
    decode::decode(bytes, mode)
}
