//! The interpreter, and the cache between decoding and executing (ADR-0004).
//!
//! Decoding costs several times what executing costs, and guest code runs the
//! same bytes over and over, so an instruction is decoded once per address and
//! executed from that record every time after.

use super::decode::{self, MAX_INSN};
use super::insn::{Insn, Kind};
use super::{exec, Cpu, CpuEngine, Mode, Stop, CS};
use crate::guest::{Arena, GuestMem};

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
/// The burst boundary and the `outside` check stay whatever happens; they are
/// most of what the loop costs, and a link does not remove them.
pub struct Interp {
    mode: Mode,
    /// Decoded instructions, in runs of straight-line code: each run is the
    /// instructions from where the guest first stood through the next
    /// transfer of control, next to each other, so executing one after
    /// another reads memory in order.
    //
    // A write to a page this has decoded out of drops the runs decoded from
    // that page (#65), which is what a title that loads its own code needs:
    // Alpha Team unpacks DLLs out of its archives into memory it has run
    // before (#58).
    insns: Vec<Insn>,
    /// The shape of each of `insns`, worked out when it was decoded, so that
    /// running one does not work it out again (#70).
    shapes: Vec<exec::Fast>,
    /// Each run, as where its first instruction is in `insns` and how many
    /// there are.
    runs: Vec<(u32, u32)>,
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
const MOST_IN_A_RUN: u32 = 64;

/// The most decoded instructions kept before the cache is thrown away whole.
const MOST_KEPT: usize = 1 << 20;

impl Interp {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            insns: Vec::new(),
            shapes: Vec::new(),
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
        self.insns.len()
    }

    /// Forget every decoded instruction: what `run_at` does when more pages
    /// were written over than the arena keeps a list of, and when the cache
    /// reaches its ceiling.
    pub fn forget(&mut self) {
        self.insns.clear();
        self.shapes.clear();
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
                entry.1 = 0;
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
    fn run_at(&mut self, lin: u32, mem: &mut Arena) -> Result<(u32, u32), Stop> {
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
        // instructions, so `insns` only grows, and it still does not grow
        // this far. The free list stays unwritten.
        if self.insns.len() > MOST_KEPT {
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
            if run.1 != 0 {
                return Ok(run);
            }
        }
        let first = u32::try_from(self.insns.len()).unwrap_or(u32::MAX);
        let mut at = lin;
        loop {
            let insn = match decode_at(at, mem, self.mode) {
                Ok(insn) => insn,
                // Only the instruction the guest stands on has to decode: one
                // further along may be data after a branch the guest takes.
                Err(stop) if at == lin => return Err(stop),
                Err(_) => break,
            };
            self.shapes.push(exec::shape_of(&insn, self.mode));
            self.insns.push(insn);
            at = at.wrapping_add(u32::from(insn.len));
            let count = u32::try_from(self.insns.len()).unwrap_or(u32::MAX) - first;
            if count >= MOST_IN_A_RUN || transfers(insn.kind) {
                break;
            }
        }
        let run = (
            first,
            u32::try_from(self.insns.len()).unwrap_or(u32::MAX) - first,
        );
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

    /// Execute one decoded instruction, the guest standing at its start.
    #[inline(always)]
    fn execute_one(&self, index: u32, cpu: &mut Cpu, mem: &mut Arena) -> Result<(), Stop> {
        let insn = &self.insns[index as usize];
        let at = cpu.eip;
        // The instruction pointer moves before the instruction runs, so that a
        // relative branch is relative to what comes after it.
        // The instruction pointer is not truncated on the way past an
        // instruction, even in real mode: a 386 lets eip run past 64K and only
        // a branch with a sixteen-bit operand size brings it back.
        cpu.eip = cpu.eip.wrapping_add(u32::from(insn.len));
        match exec::execute_fast(self.shapes[index as usize], cpu, mem, insn, self.mode) {
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

impl CpuEngine for Interp {
    fn step(&mut self, cpu: &mut Cpu, mem: &mut Arena) -> Result<(), Stop> {
        let (first, _) = self.run_at(cpu.seg_base[CS].wrapping_add(cpu.eip), mem)?;
        self.execute_one(first, cpu, mem)
    }

    fn run(
        &mut self,
        cpu: &mut Cpu,
        mem: &mut Arena,
        count: u64,
        outside: std::ops::Range<u32>,
    ) -> (u64, Result<(), Stop>) {
        let mut retired = 0;
        while retired < count && !outside.contains(&cpu.eip) {
            let (first, length) = match self.run_at(cpu.seg_base[CS].wrapping_add(cpu.eip), mem) {
                Ok(run) => run,
                Err(stop) => return (retired, Err(stop)),
            };
            for index in first..first + length {
                if retired >= count || outside.contains(&cpu.eip) {
                    break;
                }
                let (next, base) = (
                    cpu.eip
                        .wrapping_add(u32::from(self.insns[index as usize].len)),
                    cpu.seg_base[CS],
                );
                match self.execute_one(index, cpu, mem) {
                    Ok(()) => {
                        retired += 1;
                        cpu.tsc += 1;
                    }
                    Err(stop @ Stop::Interrupt(_)) => return (retired + 1, Err(stop)),
                    Err(stop) => return (retired, Err(stop)),
                }
                // Somewhere else now: the run it is in is another one.
                if cpu.eip != next || cpu.seg_base[CS] != base {
                    break;
                }
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
