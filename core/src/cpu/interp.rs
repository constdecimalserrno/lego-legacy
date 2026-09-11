//! The interpreter, and the cache between decoding and executing (ADR-0004).
//!
//! Decoding costs several times what executing costs, and guest code runs the
//! same bytes over and over, so an instruction is decoded once per address and
//! executed from that record every time after.

use super::decode::{self, MAX_INSN};
use super::insn::{Insn, Kind, Width};
use super::{exec, Cpu, CpuEngine, Mode, Stop, CS};
use crate::guest::{Arena, GuestMem};

/// A decode-cached interpreter.
pub struct Interp {
    mode: Mode,
    /// Decoded instructions, in runs of straight-line code: each run is the
    /// instructions from where the guest first stood through the next
    /// transfer of control, next to each other, so executing one after
    /// another reads memory in order.
    //
    // ponytail: no invalidation, and a run is decoded ahead of where the guest
    // has been: code rewritten after its run was decoded, even before it first
    // runs, runs its old form. Nothing on the roster has been measured doing
    // so; the first title that does gets a write watch over cached ranges.
    insns: Vec<Insn>,
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
}

const PAGE: usize = 1 << 12;

/// The most instructions one run is decoded to.
const MOST_IN_A_RUN: u32 = 64;

impl Interp {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            insns: Vec::new(),
            runs: Vec::new(),
            directory: Box::new([0; 1024]),
            regions: Vec::new(),
            pages: Vec::new(),
        }
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// How many instructions this engine has decoded and kept.
    #[must_use]
    pub fn cached(&self) -> usize {
        self.insns.len()
    }

    /// Forget every decoded instruction. What a loader calls after it has
    /// placed new bytes where old ones were.
    pub fn forget(&mut self) {
        self.insns.clear();
        self.runs.clear();
        self.pages.clear();
        self.regions.clear();
        self.directory.fill(0);
    }

    /// The run of instructions starting at a linear address, decoding it if
    /// this is the first time the engine has stood there.
    fn run_at(&mut self, lin: u32, mem: &Arena) -> Result<(u32, u32), Stop> {
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
            return Ok(self.runs[hit as usize - 1]);
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
        let place = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
        if self.directory[region] == 0 {
            self.regions.push(Box::new([0; 1024]));
            self.directory[region] = place(self.regions.len());
        }
        let r = self.directory[region] as usize - 1;
        if self.regions[r][page] == 0 {
            self.pages.push(Box::new([0; PAGE]));
            self.regions[r][page] = place(self.pages.len());
        }
        let p = self.regions[r][page] as usize - 1;
        self.pages[p][slot] = place(self.runs.len());
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
        match exec::execute(cpu, mem, insn, self.mode) {
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
                    Ok(()) => retired += 1,
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

/// The width an instruction pointer is truncated to in a mode.
#[must_use]
pub fn ip_width(mode: Mode) -> Width {
    mode.default_width()
}
