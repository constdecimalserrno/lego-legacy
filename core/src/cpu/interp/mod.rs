//! The interpreter, and the cache between decoding and executing (ADR-0004).
//!
//! Decoding costs several times what executing costs, and guest code runs the
//! same bytes over and over, so an instruction is decoded once per address and
//! executed from a compact 16-byte [`FastInsn`] record every time after (#253).
//!
//! Co-located host acceleration seam (ADR-0009, #256):
//! - `rust.rs`: canonical pure-Rust oracle and fallback (selected when
//!   `crate::arch::use_rust()`, in `Mode::REAL`, when `mem.is_watching()`, or
//!   on non-`aarch64`/non-`x86_64` targets).
//! - `aarch64.rs`: direct-threaded `#[unsafe(naked)]` + `naked_asm!` flat32 loop.
//! - `x86_64.rs`: direct-threaded `#[unsafe(naked)]` + `naked_asm!` flat32 loop
//!   (SysV and Win64 ABIs).

#[cfg(target_arch = "aarch64")]
mod aarch64;
mod rust;
#[cfg(target_arch = "x86_64")]
mod x86_64;

#[cfg(target_arch = "aarch64")]
use aarch64 as native;
#[cfg(target_arch = "x86_64")]
use x86_64 as native;

use super::decode::{self, MAX_INSN};
use super::insn::{FastInsn, FastOp, Insn, Kind, NO_REG};
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use super::ZERO_REG;
use super::{exec, Cpu, CpuEngine, Mode, Stop, CS, FS};
use crate::guest::{Arena, GuestMem};
use crate::shim::Boundary;

/// Return status codes from `run_fast_block` back to the Rust outer loop.
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub(crate) const STATUS_DONE: u32 = 0;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub(crate) const STATUS_MISS: u32 = 1;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub(crate) const STATUS_BUDGET_SHORT: u32 = 2;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub(crate) const STATUS_OUTSIDE_OVERLAP: u32 = 3;
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub(crate) const STATUS_FALLBACK: u32 = 4;

/// One decoded straight-line run inside [`Interp::fast_insns`].
///
/// Fixed 8-byte `#[repr(C)]` layout so the `naked_asm!` dispatcher can load
/// all four fields in one 64-bit read.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run {
    /// Index of the run's first instruction in `fast_insns`.
    pub(crate) first: u32,
    /// Total encoded x86 byte length across all instructions in this run.
    pub(crate) bytes: u16,
    /// Number of instructions in this run (`0` means invalidated by a code write).
    pub(crate) len: u8,
    /// Whether the final instruction can transfer control (`0` = no, `1` = yes).
    pub(crate) ends_with_transfer: u8,
}

/// Number of entries in the direct-mapped L1 block cache (`4096 × 16 B = 64 KiB`, #279).
pub(crate) const L1_BITS: u32 = 12;
pub(crate) const L1_SIZE: usize = 1 << L1_BITS;
pub(crate) const L1_MASK: u32 = (L1_SIZE as u32) - 1;

/// One 16-byte direct-mapped L1 block-cache entry (#279).
///
/// Fixed `#[repr(C, align(16))]` layout lets `aarch64` load the whole entry in
/// one `ldp x0, x2, [slot]` and lets `x86_64` probe `eip` and load `run` from
/// one 16-byte aligned slot. `run.len == 0` marks an empty slot (so even
/// `eip == 0` never false-hits).
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct L1Slot {
    /// Cached flat32 linear `eip`.
    pub(crate) eip: u32,
    /// `Interp::runs` index + 1 (`0` = empty), reused to patch static chains on L1 hit.
    pub(crate) run_idx: u32,
    /// Copy of the cached [`Run`] metadata.
    pub(crate) run: Run,
}

impl L1Slot {
    const EMPTY: Self = Self {
        eip: 0,
        run_idx: 0,
        run: Run {
            first: 0,
            bytes: 0,
            len: 0,
            ends_with_transfer: 0,
        },
    };
}

/// Direct-mapped L1 slot index for a flat32 linear address (`(eip ^ (eip >> 12)) & 0xfff`).
#[inline(always)]
fn l1_slot_index(lin: u32) -> usize {
    ((lin ^ (lin >> 12)) & L1_MASK) as usize
}

/// Single-pointer integer-only context passed to `run_fast_block` (ADR-0009).
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[repr(C)]
pub(crate) struct FastCtx {
    pub regs: *mut u32,
    pub arena_base: *mut u8,
    pub code_bitmap: *const u64,
    pub arena_limit4: u64,
    pub remaining: u64,
    pub retired: u64,
    pub fast_insns: *mut FastInsn,
    pub runs: *const Run,
    pub directory: *const u32,
    pub region_ptrs: *const *const u32,
    pub page_ptrs: *const *const u32,
    pub fault_pc: *const FastInsn,
    pub run_first_pc: *const FastInsn,
    pub eip: u32,
    pub flags: u32,
    pub fs_base: u32,
    pub arena_floor: u32,
    pub outside_start: u32,
    pub outside_end: u32,
    pub outside_len: u32,
    pub run_start_eip: u32,
    pub run_bytes: u32,
    pub run_len: u32,
    pub l1_cache: *mut L1Slot,
    pub outside_danger_len: u64,
    pub outside_danger_start: u32,
    pub _pad: u32,
    pub fpu: *mut super::x87::X87,
    /// Conservative `[code_min.saturating_sub(7), code_max)` range covering
    /// every 1..=8-byte store that could overlap a marked code page (#283).
    /// Packed as two adjacent `u32`s at 8-byte-aligned offset 176 so `aarch64`
    /// loads both into `x21` in one 64-bit `ldr` and `x86_64` pins start in
    /// `r9d` while comparing against `[rbp + 180]`. `(0, 0)` when empty.
    pub code_bbox_start: u32,
    pub code_bbox_len: u32,
}

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
#[inline(always)]
fn encode_code_bbox(mem: &Arena) -> Option<(u32, u32)> {
    let (code_min, code_max) = mem.code_bbox();
    if code_min >= code_max {
        return Some((0, 0));
    }
    let start = code_min.saturating_sub(7);
    let end = if code_max == u32::MAX {
        1u64 << 32
    } else {
        u64::from(code_max)
    };
    let len = end - u64::from(start);
    u32::try_from(len).ok().map(|len| (start, len))
}

/// Sentinel [`FastInsn`] appended after non-transfer runs for `naked_asm!`.
const END_RUN: FastInsn = FastInsn {
    op: FastOp::EndRun,
    len: 0,
    dst: NO_REG,
    src: NO_REG,
    base: NO_REG,
    index: NO_REG,
    scale: 0,
    aux: 0,
    disp: 0,
    imm: 0,
};

/// A decode-cached interpreter.
pub struct Interp {
    pub(crate) mode: Mode,
    /// Decoded instructions in compact 16-byte flat32 IR, laid out in runs of
    /// straight-line code through the next transfer of control (#253).
    pub(crate) fast_insns: Vec<FastInsn>,
    /// Count of decoded guest instructions kept (excluding `EndRun` sentinels).
    cached_insns: usize,
    /// Cold full [`Insn`] records referenced only by [`FastOp::Slow`] entries.
    pub(crate) slow_insns: Vec<Insn>,
    /// Metadata for each cached run.
    pub(crate) runs: Vec<Run>,
    /// 3-level page table mapping linear address -> `runs` index + 1.
    directory: Box<[u32; 1024]>,
    regions: Vec<Box<[u32; 1024]>>,
    region_ptrs: Vec<*const u32>,
    pages: Vec<Box<[u32; PAGE]>>,
    page_ptrs: Vec<*const u32>,
    /// Which runs were decoded out of each of `pages`, so that a write over
    /// one page kills those runs and leaves every other run standing (#65).
    page_runs: Vec<Vec<u32>>,
    /// Direct-mapped L1 block cache (`4096 × 16 B = 64 KiB`, #279), populated
    /// on flat32 decode and L1 misses and cleared on `forget()` / `forget_page()`.
    l1: Box<[L1Slot; L1_SIZE]>,
}

const PAGE: usize = 1 << 12;

/// The most instructions one run is decoded to.
const MOST_IN_A_RUN: u8 = 64;

/// Maximum possible byte span of any decoded run (`64 × 15 = 960` bytes).
const MAX_RUN_BYTES: u32 = (MOST_IN_A_RUN as u32) * (MAX_INSN as u32);

/// The most decoded instructions kept before the cache is thrown away whole.
const MOST_KEPT: usize = 1 << 20;

impl Interp {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            fast_insns: Vec::new(),
            cached_insns: 0,
            slow_insns: Vec::new(),
            runs: Vec::new(),
            directory: Box::new([0; 1024]),
            regions: Vec::new(),
            region_ptrs: Vec::new(),
            pages: Vec::new(),
            page_ptrs: Vec::new(),
            page_runs: Vec::new(),
            l1: Box::new([L1Slot::EMPTY; L1_SIZE]),
        }
    }

    /// How many instructions this engine has decoded and kept.
    #[must_use]
    pub fn cached(&self) -> usize {
        self.cached_insns
    }

    /// Forget every decoded instruction.
    pub fn forget(&mut self) {
        self.fast_insns.clear();
        self.cached_insns = 0;
        self.slow_insns.clear();
        self.runs.clear();
        self.pages.clear();
        self.page_ptrs.clear();
        self.page_runs.clear();
        self.regions.clear();
        self.region_ptrs.clear();
        self.directory.fill(0);
        self.l1.fill(L1Slot::EMPTY);
    }

    /// Forget only what was decoded out of one page (#65).
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
        let runs_on_page = std::mem::take(&mut self.page_runs[p as usize - 1]);
        if runs_on_page.is_empty() {
            return;
        }
        for run in runs_on_page {
            if let Some(entry) = self.runs.get_mut(run as usize) {
                entry.len = 0;
            }
        }
        self.l1.fill(L1Slot::EMPTY);
    }

    /// Flush any pending self-modifying code page invalidations.
    #[inline(always)]
    fn flush_code_written(&mut self, mem: &mut Arena) {
        if std::mem::take(&mut mem.code_written) {
            if std::mem::take(&mut mem.written_all) {
                mem.written_code.clear();
                mem.clear_code();
                self.forget();
            } else {
                for page in std::mem::take(&mut mem.written_code) {
                    self.forget_page(page, mem);
                }
            }
        }
    }

    /// The table for the page an address is on, made if it is not there yet.
    fn table_for(&mut self, lin: u32) -> usize {
        let (region, within) = ((lin >> 22) as usize, (lin >> 12) as usize & 0x3ff);
        let place = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
        if self.directory[region] == 0 {
            let boxed = Box::new([0u32; 1024]);
            let ptr = boxed.as_ptr();
            self.regions.push(boxed);
            self.region_ptrs.push(ptr);
            self.directory[region] = place(self.regions.len());
        }
        let r = self.directory[region] as usize - 1;
        if self.regions[r][within] == 0 {
            let boxed = Box::new([0u32; PAGE]);
            let ptr = boxed.as_ptr();
            self.pages.push(boxed);
            self.page_ptrs.push(ptr);
            self.page_runs.push(Vec::new());
            self.regions[r][within] = place(self.pages.len());
        }
        self.regions[r][within] as usize - 1
    }

    /// The run of instructions starting at a linear address, decoding it if
    /// this is the first time the engine has stood there.
    pub(crate) fn run_at(&mut self, lin: u32, mem: &mut Arena) -> Result<Run, Stop> {
        self.flush_code_written(mem);
        if self.cached_insns > MOST_KEPT {
            mem.clear_code();
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
            let run = self.runs[hit as usize - 1];
            if run.len != 0 {
                if !self.mode.real {
                    self.l1[l1_slot_index(lin)] = L1Slot {
                        eip: lin,
                        run_idx: hit,
                        run,
                    };
                }
                return Ok(run);
            }
        }
        let first = u32::try_from(self.fast_insns.len()).unwrap_or(u32::MAX);
        let mut at = lin;
        let mut count = 0u8;
        let mut bytes = 0u16;
        let mut ends_with_transfer = false;
        loop {
            if count > 0 && Boundary::RANGE.contains(&at) {
                break;
            }
            let insn = match decode_at(at, mem, self.mode) {
                Ok(insn) => insn,
                Err(stop) if count == 0 => return Err(stop),
                Err(_) => break,
            };
            let mut fast = exec::compile_fast(&insn, self.mode, &mut self.slow_insns);
            if fast.op == FastOp::JccRel {
                // Zero bytes 4..8 (`base..=aux`) to initialize the fallthrough chain slot to 0.
                fast.base = 0;
                fast.index = 0;
                fast.scale = 0;
                fast.aux = 0;
            }
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
        debug_assert!(u32::from(bytes) <= MAX_RUN_BYTES);
        if !self.mode.real && count > 1 {
            let start = first as usize;
            promote_dead_flags(&mut self.fast_insns[start..start + usize::from(count)]);
        }
        if !ends_with_transfer {
            self.fast_insns.push(END_RUN);
        }
        self.cached_insns += usize::from(count);
        let run = Run {
            first,
            bytes,
            len: count,
            ends_with_transfer: u8::from(ends_with_transfer),
        };
        self.runs.push(run);
        let index = u32::try_from(self.runs.len() - 1).unwrap_or(u32::MAX);
        let here = self.table_for(lin);
        self.pages[here][slot] = index + 1;
        if !self.mode.real {
            self.l1[l1_slot_index(lin)] = L1Slot {
                eip: lin,
                run_idx: index + 1,
                run,
            };
        }
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

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    fn run_native(
        &mut self,
        cpu: &mut Cpu,
        mem: &mut Arena,
        count: u64,
        outside: std::ops::Range<u32>,
    ) -> (u64, Result<(), Stop>) {
        let mut retired = 0u64;
        let outside_len = if outside.is_empty() {
            0
        } else {
            outside.end - outside.start
        };
        let (outside_danger_start, outside_danger_len) = if outside.is_empty() {
            (0u32, 0u64)
        } else {
            (
                outside.start.wrapping_sub(MAX_RUN_BYTES),
                u64::from(outside_len) + u64::from(MAX_RUN_BYTES),
            )
        };

        let (arena_base, arena_floor, arena_len, code_bitmap) = mem.raw_parts_mut();
        if arena_len < arena_floor as usize + 4 {
            return rust::run(self, cpu, mem, count, outside);
        }

        // 9-element GPR window with permanent zero slot `fast_regs[ZERO_REG] == 0`
        // so `run_fast_block` computes `disp + r[base] + (r[index] << scale)`
        // branchlessly while leaving `Cpu::r: [u32; 8]` (and `state_digest`) unchanged (#283).
        let mut fast_regs = [0u32; 9];
        let mut ctx = FastCtx {
            regs: fast_regs.as_mut_ptr(),
            arena_base,
            code_bitmap,
            arena_limit4: (arena_len - arena_floor as usize - 4) as u64,
            remaining: count,
            retired: 0,
            fast_insns: self.fast_insns.as_mut_ptr(),
            runs: self.runs.as_ptr(),
            directory: self.directory.as_ptr(),
            region_ptrs: self.region_ptrs.as_ptr(),
            page_ptrs: self.page_ptrs.as_ptr(),
            fault_pc: core::ptr::null(),
            run_first_pc: core::ptr::null(),
            eip: cpu.eip,
            flags: native::pack_flags(&cpu.flags),
            fs_base: cpu.seg_base[FS],
            arena_floor,
            outside_start: outside.start,
            outside_end: outside.end,
            outside_len,
            run_start_eip: 0,
            run_bytes: 0,
            run_len: 0,
            l1_cache: self.l1.as_mut_ptr(),
            outside_danger_len,
            outside_danger_start,
            _pad: 0,
            fpu: &raw mut cpu.fpu,
            code_bbox_start: 0,
            code_bbox_len: 0,
        };

        while retired < count && !outside.contains(&cpu.eip) {
            if mem.code_written {
                self.flush_code_written(mem);
            }
            if mem.is_watching() {
                match rust::run_one_block(self, cpu, mem, count, &outside, &mut retired) {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(stop) => return (retired, Err(stop)),
                }
            }
            let Some((code_bbox_start, code_bbox_len)) = encode_code_bbox(mem) else {
                match rust::run_one_block(self, cpu, mem, count, &outside, &mut retired) {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(stop) => return (retired, Err(stop)),
                }
            };

            fast_regs[..8].copy_from_slice(&cpu.r);
            fast_regs[ZERO_REG] = 0;
            ctx.regs = fast_regs.as_mut_ptr();
            ctx.fpu = &raw mut cpu.fpu;
            ctx.fast_insns = self.fast_insns.as_mut_ptr();
            ctx.runs = self.runs.as_ptr();
            ctx.directory = self.directory.as_ptr();
            ctx.region_ptrs = self.region_ptrs.as_ptr();
            ctx.page_ptrs = self.page_ptrs.as_ptr();
            ctx.l1_cache = self.l1.as_mut_ptr();
            ctx.eip = cpu.eip;
            ctx.flags = native::pack_flags(&cpu.flags);
            ctx.fs_base = cpu.seg_base[FS];
            ctx.remaining = count - retired;
            ctx.retired = 0;
            ctx.code_bbox_start = code_bbox_start;
            ctx.code_bbox_len = code_bbox_len;

            // Safety: `ctx` holds live pointers to `fast_regs` (9 elements with
            // `fast_regs[8] == 0`), `mem`, and `self`'s cache tables,
            // `arena_len >= arena_floor + 4`, and `!mem.is_watching()`.
            let status = unsafe { native::run_fast_block(&mut ctx) };

            debug_assert_eq!(
                fast_regs[ZERO_REG], 0,
                "constant-zero NO_REG slot r[8] was mutated by run_fast_block"
            );
            cpu.r.copy_from_slice(&fast_regs[..8]);
            native::unpack_flags(ctx.flags, &mut cpu.flags);
            cpu.eip = ctx.eip;
            retired += ctx.retired;
            cpu.tsc += ctx.retired;

            match status {
                STATUS_DONE => break,
                STATUS_MISS => {
                    if let Err(stop) = self.run_at(cpu.eip, mem) {
                        return (retired, Err(stop));
                    }
                }
                STATUS_BUDGET_SHORT | STATUS_OUTSIDE_OVERLAP => {
                    match rust::run_one_block(self, cpu, mem, count, &outside, &mut retired) {
                        Ok(true) => {}
                        Ok(false) => break,
                        Err(stop) => return (retired, Err(stop)),
                    }
                }
                STATUS_FALLBACK => {
                    let done = (ctx.fault_pc as usize - ctx.run_first_pc as usize)
                        / core::mem::size_of::<FastInsn>();
                    retired += done as u64;
                    cpu.tsc += done as u64;
                    let first = (ctx.run_first_pc as usize - self.fast_insns.as_ptr() as usize)
                        / core::mem::size_of::<FastInsn>();
                    let len = ctx.run_len as usize;
                    let ends_with_transfer = u8::from(
                        self.fast_insns
                            .get(first + len)
                            .is_none_or(|ins| ins.op != FastOp::EndRun),
                    );
                    let run = Run {
                        first: first as u32,
                        bytes: ctx.run_bytes as u16,
                        len: len as u8,
                        ends_with_transfer,
                    };
                    if let Err(stop) = rust::finish_run_from(
                        self,
                        run,
                        done,
                        len,
                        ctx.run_start_eip,
                        cpu,
                        mem,
                        &mut retired,
                    ) {
                        return (retired, Err(stop));
                    }
                }
                _ => unreachable!("invalid run_fast_block status {status}"),
            }
        }
        (retired, Ok(()))
    }
}

/// Conservative intra-block dead-flag promotion pass (#283).
///
/// Scans one flat32 run backwards and promotes register-only 32-bit ALU/shift
/// ops to their `*Nf` variants ONLY when all six arithmetic flags (`CF`, `PF`,
/// `AF`, `ZF`, `SF`, `OF`) are unconditionally overwritten by a later
/// non-faulting, non-bailing, non-flag-reading op in the same run, with ZERO
/// intervening flag reads, branches, `Slow`/`X87` ops, or potentially faulting
/// memory ops.
fn promote_dead_flags(run: &mut [FastInsn]) {
    let mut flags_dead = false;
    for insn in run.iter_mut().rev() {
        if flags_dead {
            if let Some(nf) = promote_no_flags(insn.op) {
                insn.op = nf;
                continue;
            }
        }
        if overwrites_all_flags_unconditionally(insn.op) {
            flags_dead = true;
        } else if !is_pure_non_flag_op(insn.op) {
            flags_dead = false;
        }
    }
}

#[inline(always)]
fn promote_no_flags(op: FastOp) -> Option<FastOp> {
    Some(match op {
        FastOp::AddRegReg => FastOp::AddRegRegNf,
        FastOp::AddRegImm => FastOp::AddRegImmNf,
        FastOp::SubRegReg => FastOp::SubRegRegNf,
        FastOp::SubRegImm => FastOp::SubRegImmNf,
        FastOp::XorRegReg => FastOp::XorRegRegNf,
        FastOp::XorRegImm => FastOp::XorRegImmNf,
        FastOp::AndRegReg => FastOp::AndRegRegNf,
        FastOp::AndRegImm => FastOp::AndRegImmNf,
        FastOp::OrRegReg => FastOp::OrRegRegNf,
        FastOp::OrRegImm => FastOp::OrRegImmNf,
        FastOp::ShlRegImm => FastOp::ShlRegImmNf,
        FastOp::ShrRegImm => FastOp::ShrRegImmNf,
        FastOp::SarRegImm => FastOp::SarRegImmNf,
        _ => return None,
    })
}

/// Ops guaranteed to overwrite ALL 6 arithmetic flags (`CF, PF, AF, ZF, SF, OF`)
/// without reading any flag, accessing memory, transferring control, or ever
/// bailing to `STATUS_FALLBACK`.
#[inline(always)]
fn overwrites_all_flags_unconditionally(op: FastOp) -> bool {
    matches!(
        op,
        FastOp::AddRegReg
            | FastOp::AddRegImm
            | FastOp::SubRegReg
            | FastOp::SubRegImm
            | FastOp::CmpRegReg
            | FastOp::CmpRegImm
            | FastOp::AndRegReg
            | FastOp::AndRegImm
            | FastOp::OrRegReg
            | FastOp::OrRegImm
            | FastOp::XorRegReg
            | FastOp::XorRegImm
            | FastOp::TestRegReg
            | FastOp::TestRegImm
            | FastOp::ShlRegImm
            | FastOp::ShrRegImm
            | FastOp::SarRegImm
            | FastOp::Add8RegReg
            | FastOp::Add8RegImm
            | FastOp::Sub8RegReg
            | FastOp::Sub8RegImm
            | FastOp::Cmp8RegReg
            | FastOp::Cmp8RegImm
            | FastOp::And8RegReg
            | FastOp::And8RegImm
            | FastOp::Or8RegReg
            | FastOp::Or8RegImm
            | FastOp::Xor8RegReg
            | FastOp::Xor8RegImm
            | FastOp::Test8RegReg
            | FastOp::Test8RegImm
            | FastOp::Alu16RegReg
            | FastOp::Alu16RegImm
            | FastOp::Neg32Reg
            | FastOp::Neg8Reg
    )
}

/// Pure register/immediate ops that NEVER read or write flags, NEVER access
/// memory, NEVER transfer control, and NEVER bail to `STATUS_FALLBACK`.
#[inline(always)]
fn is_pure_non_flag_op(op: FastOp) -> bool {
    matches!(
        op,
        FastOp::Nop
            | FastOp::MovRegReg
            | FastOp::MovRegImm
            | FastOp::Lea
            | FastOp::ExtendReg
            | FastOp::Mov8RegReg
            | FastOp::Mov8RegImm
            | FastOp::Mov16RegReg
            | FastOp::Mov16RegImm
            | FastOp::Cdq
            | FastOp::Cwde
            | FastOp::Not32Reg
            | FastOp::Not8Reg
            | FastOp::Xchg32RegReg
            | FastOp::Xchg8RegReg
            | FastOp::Extend16Reg
    )
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
pub(crate) fn sum_lengths(insns: &[FastInsn]) -> u32 {
    insns.iter().map(|ins| u32::from(ins.len)).sum()
}

#[cold]
#[inline(never)]
pub(crate) fn clamp_to_outside(
    insns: &[FastInsn],
    start_eip: u32,
    outside: &std::ops::Range<u32>,
) -> usize {
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
        cpu.eip = at.wrapping_add(u32::from(insn.len));
        match exec::execute_one_fast(insn, cpu, mem, &self.slow_insns, self.mode) {
            Ok(()) => Ok(()),
            Err(stop @ (Stop::Halted | Stop::Interrupt(_))) => Err(stop),
            Err(stop) => {
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
        if self.mode.real || crate::arch::use_rust() {
            return rust::run(self, cpu, mem, count, outside);
        }
        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        {
            self.run_native(cpu, mem, count, outside)
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        {
            rust::run(self, cpu, mem, count, outside)
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::flags::{Flags, AF, CF, DF, OF, PF, SF, ZF};
    use crate::cpu::{EAX, EBP, EBX, ECX, EDX, ESI, ESP};
    use crate::guest::DOS_MEMORY;
    use crate::host::headless::Headless;
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn run_and_fast_ctx_repr_c_layouts_match_asm_offsets() {
        assert_eq!(size_of::<Run>(), 8);
        assert_eq!(align_of::<Run>(), 4);
        assert_eq!(offset_of!(Run, first), 0);
        assert_eq!(offset_of!(Run, bytes), 4);
        assert_eq!(offset_of!(Run, len), 6);
        assert_eq!(offset_of!(Run, ends_with_transfer), 7);

        assert_eq!(size_of::<L1Slot>(), 16);
        assert_eq!(align_of::<L1Slot>(), 16);
        assert_eq!(offset_of!(L1Slot, eip), 0);
        assert_eq!(offset_of!(L1Slot, run_idx), 4);
        assert_eq!(offset_of!(L1Slot, run), 8);

        #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
        {
            assert_eq!(size_of::<crate::cpu::x87::X87>(), 72);
            assert_eq!(align_of::<crate::cpu::x87::X87>(), 8);
            assert_eq!(size_of::<FastCtx>(), 184);
            assert_eq!(align_of::<FastCtx>(), 8);
            assert_eq!(offset_of!(FastCtx, regs), 0);
            assert_eq!(offset_of!(FastCtx, arena_base), 8);
            assert_eq!(offset_of!(FastCtx, code_bitmap), 16);
            assert_eq!(offset_of!(FastCtx, arena_limit4), 24);
            assert_eq!(offset_of!(FastCtx, remaining), 32);
            assert_eq!(offset_of!(FastCtx, retired), 40);
            assert_eq!(offset_of!(FastCtx, fast_insns), 48);
            assert_eq!(offset_of!(FastCtx, runs), 56);
            assert_eq!(offset_of!(FastCtx, directory), 64);
            assert_eq!(offset_of!(FastCtx, region_ptrs), 72);
            assert_eq!(offset_of!(FastCtx, page_ptrs), 80);
            assert_eq!(offset_of!(FastCtx, fault_pc), 88);
            assert_eq!(offset_of!(FastCtx, run_first_pc), 96);
            assert_eq!(offset_of!(FastCtx, eip), 104);
            assert_eq!(offset_of!(FastCtx, flags), 108);
            assert_eq!(offset_of!(FastCtx, fs_base), 112);
            assert_eq!(offset_of!(FastCtx, arena_floor), 116);
            assert_eq!(offset_of!(FastCtx, outside_start), 120);
            assert_eq!(offset_of!(FastCtx, outside_end), 124);
            assert_eq!(offset_of!(FastCtx, outside_len), 128);
            assert_eq!(offset_of!(FastCtx, run_start_eip), 132);
            assert_eq!(offset_of!(FastCtx, run_bytes), 136);
            assert_eq!(offset_of!(FastCtx, run_len), 140);
            assert_eq!(offset_of!(FastCtx, l1_cache), 144);
            assert_eq!(offset_of!(FastCtx, outside_danger_len), 152);
            assert_eq!(offset_of!(FastCtx, outside_danger_start), 160);
            assert_eq!(offset_of!(FastCtx, fpu), 168);
            assert_eq!(offset_of!(FastCtx, code_bbox_start), 176);
            assert_eq!(offset_of!(FastCtx, code_bbox_len), 180);
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn flag_pack_and_unpack_roundtrip_all_64_combinations_and_preserve_rest() {
        let base_rest = 0x0024_0202u32 | DF;
        for bits in 0u32..64 {
            let mut eflags = base_rest;
            if bits & 1 != 0 {
                eflags |= CF;
            }
            if bits & 2 != 0 {
                eflags |= PF;
            }
            if bits & 4 != 0 {
                eflags |= AF;
            }
            if bits & 8 != 0 {
                eflags |= ZF;
            }
            if bits & 16 != 0 {
                eflags |= SF;
            }
            if bits & 32 != 0 {
                eflags |= OF;
            }
            let orig = Flags::from_eflags(eflags);
            let packed = native::pack_flags(&orig);
            let mut out = Flags::from_eflags(base_rest);
            native::unpack_flags(packed, &mut out);
            assert_eq!(out, orig, "roundtrip failed for bits={bits:#04x}");
            assert_eq!(out.to_eflags(), eflags);
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[allow(clippy::too_many_arguments)]
    fn assert_native_matches_rust(
        label: &str,
        code: &[u8],
        init_cpu: Cpu,
        init_writes: &[(u32, &[u8])],
        check_addrs: &[(u32, usize)],
        budget: u64,
        outside: std::ops::Range<u32>,
        floor: u32,
    ) {
        let arena_len = 1 << 22; // 4 MiB
        let mut h_rust = Headless::new();
        let mut h_nat = Headless::new();
        let mut m_rust = Arena::new(&mut h_rust, arena_len)
            .unwrap()
            .with_floor(floor);
        let mut m_nat = Arena::new(&mut h_nat, arena_len).unwrap().with_floor(floor);

        let entry = init_cpu.eip;
        m_rust.write(entry, code).unwrap();
        m_nat.write(entry, code).unwrap();
        for &(addr, bytes) in init_writes {
            m_rust.write(addr, bytes).unwrap();
            m_nat.write(addr, bytes).unwrap();
        }

        let mut c_rust = init_cpu;
        let mut c_nat = init_cpu;
        let mut i_rust = Interp::new(Mode::FLAT);
        let mut i_nat = Interp::new(Mode::FLAT);

        let (ret_r, res_r) = rust::run(
            &mut i_rust,
            &mut c_rust,
            &mut m_rust,
            budget,
            outside.clone(),
        );
        let (ret_n, res_n) = i_nat.run_native(&mut c_nat, &mut m_nat, budget, outside);

        assert_eq!(ret_n, ret_r, "{label}: retired mismatch");
        assert_eq!(res_n, res_r, "{label}: stop mismatch");
        assert_eq!(c_nat.r, c_rust.r, "{label}: GPRs mismatch");
        assert_eq!(c_nat.eip, c_rust.eip, "{label}: EIP mismatch");
        assert_eq!(
            c_nat.flags.to_eflags(),
            c_rust.flags.to_eflags(),
            "{label}: EFLAGS mismatch (nat={:#x} rust={:#x})",
            c_nat.flags.to_eflags(),
            c_rust.flags.to_eflags()
        );
        assert_eq!(c_nat.tsc, c_rust.tsc, "{label}: TSC mismatch");
        assert!(
            c_nat.fpu.same_bits(&c_rust.fpu),
            "{label}: FPU bit mismatch\n  nat ={:?}\n  rust={:?}",
            c_nat.fpu,
            c_rust.fpu
        );
        assert_eq!(
            i_nat.cached(),
            i_rust.cached(),
            "{label}: cached() mismatch"
        );
        for &(addr, len) in check_addrs {
            assert_eq!(
                m_nat.slice(addr, len),
                m_rust.slice(addr, len),
                "{label}: memory mismatch at {addr:#x}+{len}"
            );
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_native_matches_rust_on_data_movement_addressing_and_stack() {
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;
        let stack = 0x0030_0000u32;
        let fs_base = 0x0022_0000u32;

        // Exercise: nop, fwait, mov32/16/8 (rr, ri, rm, mr, mi), lea (with SIB),
        // FS: override, movzx/movsx (r8 low/high, r16, m8, m16), push/pop (reg,
        // imm, mem, push esp, pop esp, pop [esp+4]).
        let mut prog = Vec::new();
        prog.extend_from_slice(&[0x90, 0x9b]); // nop; fwait
        prog.extend_from_slice(&[0xb8, 0x78, 0x56, 0x34, 0x12]); // mov eax, 0x12345678
        prog.extend_from_slice(&[0x89, 0xc3]); // mov ebx, eax
        prog.extend_from_slice(&[0x89, 0x06]); // mov [esi], eax
        prog.extend_from_slice(&[0xc7, 0x46, 0x04, 0xef, 0xbe, 0xad, 0xde]); // mov dword [esi+4], 0xdeadbeef
        prog.extend_from_slice(&[0x8b, 0x4e, 0x04]); // mov ecx, [esi+4]
        prog.extend_from_slice(&[0x8d, 0x54, 0x8e, 0x10]); // lea edx, [esi + ecx*4 + 0x10]
                                                           // FS: load from fs:[0x08]
        prog.extend_from_slice(&[0x64, 0x8b, 0x3d, 0x08, 0x00, 0x00, 0x00]); // mov edi, fs:[8]
                                                                             // Mov8 low & high regs + mem
        prog.extend_from_slice(&[0xb0, 0xab]); // mov al, 0xab
        prog.extend_from_slice(&[0xb4, 0xcd]); // mov ah, 0xcd
        prog.extend_from_slice(&[0x88, 0xe7]); // mov bh, ah
        prog.extend_from_slice(&[0x88, 0x66, 0x08]); // mov [esi+8], ah
        prog.extend_from_slice(&[0xc6, 0x46, 0x09, 0x81]); // mov byte [esi+9], 0x81
        prog.extend_from_slice(&[0x8a, 0x6e, 0x09]); // mov ch, [esi+9]
                                                     // Mov16 reg & mem
        prog.extend_from_slice(&[0x66, 0xba, 0xfe, 0xca]); // mov dx, 0xcafe
        prog.extend_from_slice(&[0x66, 0x89, 0xd3]); // mov bx, dx
        prog.extend_from_slice(&[0x66, 0x89, 0x56, 0x0a]); // mov [esi+10], dx
        prog.extend_from_slice(&[0x66, 0xc7, 0x46, 0x0c, 0x01, 0x80]); // mov word [esi+12], 0x8001
        prog.extend_from_slice(&[0x66, 0x8b, 0x46, 0x0c]); // mov ax, [esi+12]
                                                           // ExtendReg & ExtendMem: ZX8/SX8 (low & high regs), ZX16/SX16
        prog.extend_from_slice(&[0x0f, 0xb6, 0xd4]); // movzx edx, ah
        prog.extend_from_slice(&[0x0f, 0xbe, 0xdc]); // movsx ebx, ah
        prog.extend_from_slice(&[0x0f, 0xb7, 0xc8]); // movzx ecx, ax
        prog.extend_from_slice(&[0x0f, 0xbf, 0xf8]); // movsx edi, ax
        prog.extend_from_slice(&[0x0f, 0xb6, 0x46, 0x09]); // movzx eax, byte [esi+9]
        prog.extend_from_slice(&[0x0f, 0xbe, 0x4e, 0x09]); // movsx ecx, byte [esi+9]
        prog.extend_from_slice(&[0x0f, 0xb7, 0x56, 0x0c]); // movzx edx, word [esi+12]
        prog.extend_from_slice(&[0x0f, 0xbf, 0x5e, 0x0c]); // movsx ebx, word [esi+12]
                                                           // Stack: push reg, push esp, push imm, push mem, pop reg, pop mem ([esp+4]), pop esp
        prog.extend_from_slice(&[0x50, 0x54]); // push eax; push esp
        prog.extend_from_slice(&[0x68, 0x11, 0x22, 0x33, 0x44]); // push 0x44332211
        prog.extend_from_slice(&[0xff, 0x76, 0x04]); // push dword [esi+4]
        prog.extend_from_slice(&[0x5a]); // pop edx
        prog.extend_from_slice(&[0x8f, 0x44, 0x24, 0x04]); // pop dword [esp+4]
        prog.extend_from_slice(&[0x59, 0x58]); // pop ecx; pop eax
        prog.extend_from_slice(&[0x54, 0x5c]); // push esp; pop esp
        prog.push(0xf4); // hlt

        let mut cpu = Cpu {
            eip: code_at,
            tsc: 77,
            ..Cpu::default()
        };
        cpu.r[ESI] = data;
        cpu.r[ESP] = stack;
        cpu.r[EBP] = 0x5555_5555;
        cpu.seg_base[FS] = fs_base;

        assert_native_matches_rust(
            "data_mov_addr_stack",
            &prog,
            cpu,
            &[(fs_base + 8, &0x9988_7766u32.to_le_bytes())],
            &[(data, 32), (stack - 32, 48)],
            1000,
            0..0,
            DOS_MEMORY,
        );
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_native_matches_rust_on_all_alu_widths_inc_dec_shifts_and_imul2() {
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;

        const EDGE32: &[u32] = &[
            0,
            1,
            0x0f,
            0x10,
            0x7f,
            0x80,
            0xff,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_ffff,
        ];
        // 32-bit add/sub/cmp/and/or/xor/test (reg-reg, reg-imm, reg-mem, mem-reg, mem-imm)
        for &a in EDGE32 {
            for &b in EDGE32 {
                let mut prog = Vec::new();
                // add eax, ebx; sub ecx, edx; cmp eax, [esi]; and ebx, imm; or [esi+4], ecx;
                // xor edx, [esi+8]; test eax, ebx; test [esi+4], imm; cmp [esi+8], edx; hlt
                prog.extend_from_slice(&[0x01, 0xd8]); // add eax, ebx
                prog.extend_from_slice(&[0x29, 0xd1]); // sub ecx, edx
                prog.extend_from_slice(&[0x3b, 0x06]); // cmp eax, [esi]
                prog.extend_from_slice(&[0x81, 0xe3]);
                prog.extend_from_slice(&b.to_le_bytes()); // and ebx, b
                prog.extend_from_slice(&[0x09, 0x4e, 0x04]); // or [esi+4], ecx
                prog.extend_from_slice(&[0x33, 0x56, 0x08]); // xor edx, [esi+8]
                prog.extend_from_slice(&[0x85, 0xd8]); // test eax, ebx
                prog.extend_from_slice(&[0xf7, 0x46, 0x04]);
                prog.extend_from_slice(&a.to_le_bytes()); // test dword [esi+4], a
                prog.extend_from_slice(&[0x39, 0x56, 0x08]); // cmp [esi+8], edx
                prog.push(0xf4);

                let mut cpu = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu.r[EAX] = a;
                cpu.r[EBX] = b;
                cpu.r[ECX] = a ^ 0x1234_5678;
                cpu.r[EDX] = b.wrapping_add(0x0f);
                cpu.r[ESI] = data;
                let mut mem_init = [0u8; 16];
                mem_init[0..4].copy_from_slice(&b.to_le_bytes());
                mem_init[4..8].copy_from_slice(&a.to_le_bytes());
                mem_init[8..12].copy_from_slice(&a.wrapping_sub(b).to_le_bytes());

                assert_native_matches_rust(
                    "alu32",
                    &prog,
                    cpu,
                    &[(data, &mem_init)],
                    &[(data, 16)],
                    200,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }

        // Inc/Dec reg & mem preserving CF=0 and CF=1
        for &a in EDGE32 {
            for cf in [false, true] {
                for &(name, opcodes) in &[
                    ("inc_reg", &[0x40u8, 0xf4][..]),
                    ("dec_reg", &[0x48u8, 0xf4][..]),
                    ("inc_mem", &[0xffu8, 0x06, 0xf4][..]),
                    ("dec_mem", &[0xffu8, 0x0e, 0xf4][..]),
                ] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = a;
                    cpu.r[ESI] = data;
                    cpu.flags.cf = cf;
                    assert_native_matches_rust(
                        name,
                        opcodes,
                        cpu,
                        &[(data, &a.to_le_bytes())],
                        &[(data, 4)],
                        10,
                        0..0,
                        DOS_MEMORY,
                    );
                }
            }
        }

        // Shifts: SHL/SHR/SAR (imm, CL, mem-imm, mem-CL) incl. CL=0 and CL=32
        for &a in &[0u32, 1, 0x7fff_ffff, 0x8000_0000, 0x8000_0001, 0xffff_ffff] {
            for &c in &[0u32, 1, 2, 7, 15, 31, 32] {
                for &(name, bytes) in &[
                    ("shl_cl", &[0xd3u8, 0xe0, 0xf4][..]),
                    ("shr_cl", &[0xd3u8, 0xe8, 0xf4][..]),
                    ("sar_cl", &[0xd3u8, 0xf8, 0xf4][..]),
                    ("shl_mem_cl", &[0xd3u8, 0x26, 0xf4][..]),
                    ("shr_mem_cl", &[0xd3u8, 0x2e, 0xf4][..]),
                    ("sar_mem_cl", &[0xd3u8, 0x3e, 0xf4][..]),
                ] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(CF | PF | AF | ZF | SF | OF | DF),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = a;
                    cpu.r[ECX] = c;
                    // When c & 31 == 0, ShiftMem must NOT even fault on unmapped ESI!
                    cpu.r[ESI] = if (c & 31) == 0 { 0x0000_0010 } else { data };
                    assert_native_matches_rust(
                        name,
                        bytes,
                        cpu,
                        &[(data, &a.to_le_bytes())],
                        &[(data, 4)],
                        10,
                        0..0,
                        DOS_MEMORY,
                    );
                }
                if c > 0 && c < 32 {
                    let cb = c as u8;
                    for (name, bytes) in [
                        ("shl_imm", [0xc1u8, 0xe0, cb, 0xf4]),
                        ("shr_imm", [0xc1u8, 0xe8, cb, 0xf4]),
                        ("sar_imm", [0xc1u8, 0xf8, cb, 0xf4]),
                        ("shl_m_imm", [0xc1u8, 0x26, cb, 0xf4]),
                        ("shr_m_imm", [0xc1u8, 0x2e, cb, 0xf4]),
                        ("sar_m_imm", [0xc1u8, 0x3e, cb, 0xf4]),
                    ] {
                        let mut cpu = Cpu {
                            eip: code_at,
                            ..Cpu::default()
                        };
                        cpu.r[EAX] = a;
                        cpu.r[ESI] = data;
                        assert_native_matches_rust(
                            name,
                            &bytes,
                            cpu,
                            &[(data, &a.to_le_bytes())],
                            &[(data, 4)],
                            10,
                            0..0,
                            DOS_MEMORY,
                        );
                    }
                }
            }
        }

        // 8-bit and 16-bit ALU across reg/mem/imm (including high 8-bit regs AH/CH!)
        for &a in &[
            0u32, 1, 0x0f, 0x10, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff,
        ] {
            for &b in &[0u32, 1, 0x0f, 0x7f, 0x80, 0xff, 0x8000, 0xffff] {
                let mut prog8 = Vec::new();
                prog8.extend_from_slice(&[0x00, 0xec]); // add ah, ch
                prog8.extend_from_slice(&[0x28, 0xd8]); // sub al, bl
                prog8.extend_from_slice(&[0x3a, 0x26]); // cmp ah, [esi]
                prog8.extend_from_slice(&[0x80, 0x7e, 0x01, b as u8]); // cmp byte [esi+1], b
                prog8.extend_from_slice(&[0x08, 0x4e, 0x02]); // or [esi+2], cl
                prog8.extend_from_slice(&[0xf6, 0x46, 0x03, b as u8]); // test byte [esi+3], b
                prog8.extend_from_slice(&[0x34, b as u8]); // xor al, b
                prog8.push(0xf4);

                let mut cpu = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0xaabb_0000 | ((a & 0xff) << 8) | (b & 0xff);
                cpu.r[EBX] = 0xccdd_0000 | (a & 0xff);
                cpu.r[ECX] = 0xeeff_0000 | ((b & 0xff) << 8) | (a & 0xff);
                cpu.r[ESI] = data;
                assert_native_matches_rust(
                    "alu8",
                    &prog8,
                    cpu,
                    &[(data, &[b as u8, a as u8, b as u8, a as u8])],
                    &[(data, 4)],
                    50,
                    0..0,
                    DOS_MEMORY,
                );

                let mut prog16 = Vec::new();
                prog16.extend_from_slice(&[0x66, 0x01, 0xd8]); // add ax, bx
                prog16.extend_from_slice(&[0x66, 0x29, 0xd1]); // sub cx, dx
                prog16.extend_from_slice(&[0x66, 0x0b, 0x1e]); // or bx, [esi] (Hot Wheels hot op!)
                prog16.extend_from_slice(&[0x66, 0x81, 0x7e, 0x02]);
                prog16.extend_from_slice(&(b as u16).to_le_bytes()); // cmp word [esi+2], b
                prog16.extend_from_slice(&[0x66, 0x31, 0x4e, 0x04]); // xor [esi+4], cx
                prog16.extend_from_slice(&[0x66, 0x85, 0xc0]); // test ax, ax
                prog16.push(0xf4);

                let mut cpu16 = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu16.r[EAX] = 0x1234_0000 | (a & 0xffff);
                cpu16.r[EBX] = 0x5678_0000 | (b & 0xffff);
                cpu16.r[ECX] = 0x9abc_0000 | (a & 0xffff);
                cpu16.r[EDX] = 0xdef0_0000 | (b & 0xffff);
                cpu16.r[ESI] = data;
                let mut m16 = [0u8; 8];
                m16[0..2].copy_from_slice(&(b as u16).to_le_bytes());
                m16[2..4].copy_from_slice(&(a as u16).to_le_bytes());
                m16[4..6].copy_from_slice(&(b as u16).to_le_bytes());
                assert_native_matches_rust(
                    "alu16",
                    &prog16,
                    cpu16,
                    &[(data, &m16)],
                    &[(data, 8)],
                    50,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }

        // Imul2Reg & Imul2Mem (both 2-op and 3-op imm), preserving lazy & materialized flags
        for &(a, b) in &[
            (3i32, 7i32),
            (-5, 20),
            (0x4000_0000, 4),
            (i32::MIN, -1),
            (0, -1),
        ] {
            let mut prog = Vec::new();
            prog.extend_from_slice(&[0x01, 0xd8]); // add eax, ebx (sets lazy flags!)
            prog.extend_from_slice(&[0x0f, 0xaf, 0xc3]); // imul eax, ebx
            prog.extend_from_slice(&[0x6b, 0xca, 20]); // imul ecx, edx, 20
            prog.extend_from_slice(&[0x0f, 0xaf, 0x16]); // imul edx, [esi]
            prog.extend_from_slice(&[0x69, 0x3e]);
            prog.extend_from_slice(&b.to_le_bytes()); // imul edi, [esi], b
            prog.push(0xf4);

            let mut cpu = Cpu {
                eip: code_at,
                ..Cpu::default()
            };
            cpu.r[EAX] = a as u32;
            cpu.r[EBX] = b as u32;
            cpu.r[ECX] = 0x1111_1111;
            cpu.r[EDX] = a as u32;
            cpu.r[ESI] = data;
            assert_native_matches_rust(
                "imul2",
                &prog,
                cpu,
                &[(data, &(b as u32).to_le_bytes())],
                &[],
                50,
                0..0,
                DOS_MEMORY,
            );
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_native_matches_rust_on_all_16_jcc_and_setcc_across_all_64_flag_states() {
        let code_at = 0x0018_0000u32;
        for cc in 0u8..16 {
            // setcc al; setcc ah; jcc +2; inc ebx; hlt; inc ecx; hlt
            let prog = [
                0x0f,
                0x90 + cc,
                0xc0, // setcc al
                0x0f,
                0x90 + cc,
                0xc4, // setcc ah
                0x70 + cc,
                0x02, // jcc +2 (skips inc ebx; hlt)
                0x43, // inc ebx
                0xf4, // hlt
                0x41, // inc ecx
                0xf4, // hlt
            ];
            for bits in 0u32..64 {
                let mut eflags = 0x0002u32;
                if bits & 1 != 0 {
                    eflags |= CF;
                }
                if bits & 2 != 0 {
                    eflags |= PF;
                }
                if bits & 4 != 0 {
                    eflags |= AF;
                }
                if bits & 8 != 0 {
                    eflags |= ZF;
                }
                if bits & 16 != 0 {
                    eflags |= SF;
                }
                if bits & 32 != 0 {
                    eflags |= OF;
                }
                let mut cpu = Cpu {
                    eip: code_at,
                    flags: Flags::from_eflags(eflags),
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0xdead_beef;
                assert_native_matches_rust(
                    "jcc_setcc_materialized",
                    &prog,
                    cpu,
                    &[],
                    &[],
                    20,
                    0..0,
                    DOS_MEMORY,
                );
            }
            // Also test JP/JNP and all 16 Jcc/Setcc after lazy ALU flags!
            let mut lazy_prog = vec![0x01, 0xd8]; // add eax, ebx (lazy PF/AF on aarch64!)
            lazy_prog.extend_from_slice(&prog);
            for &(a, b) in &[
                (0u32, 0u32),
                (0, 1),
                (0, 3),
                (0x7fff_ffff, 1),
                (0xffff_ffff, 1),
            ] {
                let mut cpu = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu.r[EAX] = a;
                cpu.r[EBX] = b;
                assert_native_matches_rust(
                    "jcc_setcc_lazy",
                    &lazy_prog,
                    cpu,
                    &[],
                    &[],
                    20,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_native_matches_rust_on_transfers_endrun_faults_code_writes_and_watch() {
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;
        let stack = 0x0030_0000u32;

        // 1. Long straight-line run (> MOST_IN_A_RUN = 64) exercising EndRun sentinel + Call/Ret/JmpReg/JmpMem/CallReg/CallMem
        let mut prog = vec![0x40u8; 70]; // 70 x `inc eax` (hits EndRun after 64!)
                                         // call +5 (to helper), add eax, 100, hlt
                                         // helper: mov ebx, [esp]; inc eax; ret 4
        let helper_off = (prog.len() + 5 + 3 + 1) as u32;
        prog.push(0xe8);
        prog.extend_from_slice(&4i32.to_le_bytes()); // call helper (skip 3+1 bytes)
        prog.extend_from_slice(&[0x83, 0xc0, 100]); // add eax, 100 (3 bytes)
        prog.push(0xf4); // hlt (1 byte)
        assert_eq!(prog.len() as u32, helper_off);
        prog.extend_from_slice(&[0x40, 0xc2, 0x04, 0x00]); // inc eax; ret 4

        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[ESP] = stack;
        for budget in [0u64, 1, 63, 64, 65, 70, 71, 72, 73, 74, 200] {
            assert_native_matches_rust(
                "endrun_call_ret_budgets",
                &prog,
                cpu,
                &[],
                &[(stack - 16, 24)],
                budget,
                0..0,
                DOS_MEMORY,
            );
        }

        // 2. Indirect transfers: JmpReg, JmpMem, CallReg, CallMem
        let sub1 = code_at + 0x40;
        let sub2 = code_at + 0x50;
        let jmp_target = code_at + 0x60;
        let final_target = code_at + 0x70;
        let mut ind = vec![0x90u8; 0x80];
        // @0: call eax (sub1); call dword [esi] (sub2); jmp ebx (jmp_target)
        ind[0..2].copy_from_slice(&[0xff, 0xd0]); // call eax
        ind[2..4].copy_from_slice(&[0xff, 0x16]); // call dword [esi]
        ind[4..6].copy_from_slice(&[0xff, 0xe3]); // jmp ebx
                                                  // @0x40: inc ecx; ret
        ind[0x40..0x42].copy_from_slice(&[0x41, 0xc3]);
        // @0x50: add ecx, 10; ret
        ind[0x50..0x54].copy_from_slice(&[0x83, 0xc1, 10, 0xc3]);
        // @0x60: jmp dword [esi+4] (final_target)
        ind[0x60..0x63].copy_from_slice(&[0xff, 0x66, 0x04]);
        // @0x70: inc ecx; hlt
        ind[0x70..0x72].copy_from_slice(&[0x41, 0xf4]);

        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[EAX] = sub1;
        cpu.r[EBX] = jmp_target;
        cpu.r[ESI] = data;
        cpu.r[ESP] = stack;
        let mut ptrs = [0u8; 8];
        ptrs[0..4].copy_from_slice(&sub2.to_le_bytes());
        ptrs[4..8].copy_from_slice(&final_target.to_le_bytes());
        assert_native_matches_rust(
            "indirect_transfers",
            &ind,
            cpu,
            &[(data, &ptrs)],
            &[(stack - 16, 20)],
            100,
            0..0,
            DOS_MEMORY,
        );

        // 3. Floor & arena-end faults (including PopMem rollback!)
        let end = 1u32 << 22;
        for &(name, bytes, addr) in &[
            (
                "fault_read32_floor",
                &[0x40u8, 0x8b, 0x06, 0xf4][..],
                DOS_MEMORY - 2,
            ),
            (
                "fault_write32_floor",
                &[0x40u8, 0x89, 0x06, 0xf4][..],
                DOS_MEMORY - 4,
            ),
            ("fault_read32_end", &[0x40u8, 0x8b, 0x06, 0xf4][..], end - 2),
            ("fault_write8_end", &[0x40u8, 0x88, 0x06, 0xf4][..], end),
            (
                "fault_read16_end",
                &[0x40u8, 0x66, 0x8b, 0x06, 0xf4][..],
                end - 1,
            ),
            (
                "fault_pop_mem_rollback",
                &[0x40u8, 0x8f, 0x06, 0xf4][..],
                DOS_MEMORY - 4,
            ),
        ] {
            let mut cpu = Cpu {
                eip: code_at,
                ..Cpu::default()
            };
            cpu.r[ESI] = addr;
            cpu.r[ESP] = stack;
            assert_native_matches_rust(name, bytes, cpu, &[], &[], 50, 0..0, DOS_MEMORY);
        }

        // 4. Page-crossing write into marked code page + self-modifying code
        let target_page = 0x0018_2000u32;
        let mut smc = Vec::new();
        // call target_page; mov dword [target_page - 2], 0x002a_b800 (crosses 0x1ffe..0x2002!); call target_page; hlt
        let rel1 = target_page.wrapping_sub(code_at + 5).to_le_bytes();
        smc.push(0xe8);
        smc.extend_from_slice(&rel1);
        smc.extend_from_slice(&[0xc7, 0x05]);
        smc.extend_from_slice(&(target_page - 2).to_le_bytes());
        smc.extend_from_slice(&[0x00, 0x00, 0xb8, 42]); // writes bytes [0xb8, 42] into target_page[0..2]!
        let rel2 = target_page
            .wrapping_sub(code_at + smc.len() as u32 + 5)
            .to_le_bytes();
        smc.push(0xe8);
        smc.extend_from_slice(&rel2);
        smc.push(0xf4);
        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[ESP] = stack;
        assert_native_matches_rust(
            "page_crossing_smc",
            &smc,
            cpu,
            &[(target_page, &[0xb8, 1, 0, 0, 0, 0xc3])], // mov eax, 1; ret
            &[(target_page - 2, 8)],
            100,
            0..0,
            DOS_MEMORY,
        );

        // 5. Mid-run X87 + Slow (`rdtsc`) + `outside` overlap clamping
        let mix = [
            0x40, // inc eax
            0xd9, 0xe8, // fld1 (X87Reg)
            0x40, // inc eax
            0x0f, 0x31, // rdtsc (Slow)
            0x43, // inc ebx
            0xf4, // hlt
        ];
        let cpu = Cpu {
            eip: code_at,
            tsc: 500,
            ..Cpu::default()
        };
        assert_native_matches_rust(
            "mid_run_x87_rdtsc",
            &mix,
            cpu,
            &[],
            &[],
            50,
            0..0,
            DOS_MEMORY,
        );
        assert_native_matches_rust(
            "outside_overlap_clamp",
            &mix,
            cpu,
            &[],
            &[],
            50,
            (code_at + 4)..(code_at + 6),
            DOS_MEMORY,
        );

        // 6. Active `mem.watch()` preserves `unchanged()` under native run
        let mut h = Headless::new();
        let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
        mem.write(code_at, &[0x89, 0x06, 0x89, 0x1e, 0xf4]).unwrap(); // mov [esi], eax; mov [esi], ebx; hlt
        mem.write_u32(data, 0x1234_5678).unwrap();
        mem.watch();
        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[EAX] = 0xaaaa_bbbb;
        cpu.r[EBX] = 0x1234_5678;
        cpu.r[ESI] = data;
        let mut interp = Interp::new(Mode::FLAT);
        let (ret, res) = interp.run_native(&mut cpu, &mut mem, 20, 0..0);
        assert_eq!((ret, res), (2, Err(Stop::Halted)));
        assert!(mem.unchanged(), "native path must honor Arena::watch()");
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_smc_chained_targets_l1_collisions_and_custom_outside_ranges() {
        let stack = 0x0030_0000u32;

        // 1. SMC over chained JmpRel, JccRel (both taken & fallthrough), CallRel, and EndRun.
        // Place the dispatcher on page 0x0018_0000 and four mutable targets on separate pages
        // so patching one target page invalidates only its runs while keeping callers' chains live.
        let code_at = 0x0018_0000u32;
        let call_tgt = 0x0018_2000u32;
        let jmp_tgt = 0x0018_3000u32;
        let jcc_taken_tgt = 0x0018_4000u32;
        let endrun_page = 0x0018_5000u32;
        let endrun_cont = endrun_page + 64; // 64 x `inc eax` hits EndRun right at +64!

        let emit_rel32 = |buf: &mut Vec<u8>, opcode: &[u8], base: u32, target: u32| {
            buf.extend_from_slice(opcode);
            let next = base + buf.len() as u32 + 4;
            buf.extend_from_slice(&target.wrapping_sub(next).to_le_bytes());
        };

        // Main loop (runs 4 iterations, ecx = 4 -> 0):
        // - Iter 4, 3: warm & hit direct chains (CallRel, JccRel taken & fallthrough, JmpRel, EndRun)
        // - Iter 2: rewrite call_tgt, jmp_tgt, jcc_taken_tgt, and endrun_cont in guest memory!
        // - Iter 1: re-execute every chained static transfer -> must trap cbz run.len and run new code!
        let mut main = Vec::new();
        // Loop top:
        let loop_top = code_at;
        // CallRel -> call_tgt (adds 1 initially, then 100 after SMC)
        emit_rel32(&mut main, &[0xe8], code_at, call_tgt);
        // JccRel: test cl, 1; jnz jcc_taken_tgt (alternates taken/fallthrough every iter!)
        main.extend_from_slice(&[0xf6, 0xc1, 0x01]); // test cl, 1
        emit_rel32(&mut main, &[0x0f, 0x85], code_at, jcc_taken_tgt);
        // Fallthrough of JccRel: add eax, 2; jmp jmp_tgt
        main.extend_from_slice(&[0x83, 0xc0, 0x02]);
        emit_rel32(&mut main, &[0xe9], code_at, jmp_tgt);
        let after_jcc = code_at + main.len() as u32;
        // Call endrun_page (64 x inc eax -> EndRun -> endrun_cont: add eax, 3; ret)
        emit_rel32(&mut main, &[0xe8], code_at, endrun_page);
        // If ecx == 3, patch all four targets!
        main.extend_from_slice(&[0x83, 0xf9, 0x03]); // cmp ecx, 3
        let jnz_skip_pos = main.len();
        main.extend_from_slice(&[0x75, 0x00]); // jnz skip_smc
                                               // Rewrite call_tgt+2 imm8: 1 -> 100
        main.extend_from_slice(&[0xc6, 0x05]);
        main.extend_from_slice(&(call_tgt + 2).to_le_bytes());
        main.push(100);
        // Rewrite jmp_tgt+2 imm8: 5 -> 50
        main.extend_from_slice(&[0xc6, 0x05]);
        main.extend_from_slice(&(jmp_tgt + 2).to_le_bytes());
        main.push(50);
        // Rewrite jcc_taken_tgt+2 imm8: 7 -> 70
        main.extend_from_slice(&[0xc6, 0x05]);
        main.extend_from_slice(&(jcc_taken_tgt + 2).to_le_bytes());
        main.push(70);
        // Rewrite endrun_cont+2 imm8: 3 -> 30
        main.extend_from_slice(&[0xc6, 0x05]);
        main.extend_from_slice(&(endrun_cont + 2).to_le_bytes());
        main.push(30);
        let skip_smc_off = main.len() - (jnz_skip_pos + 2);
        main[jnz_skip_pos + 1] = skip_smc_off as u8;
        // dec ecx; jnz loop_top; hlt
        main.push(0x49); // dec ecx
        emit_rel32(&mut main, &[0x0f, 0x85], code_at, loop_top);
        main.push(0xf4); // hlt

        // Targets:
        let call_code = [0x83, 0xc0, 0x01, 0xc3]; // add eax, 1; ret
        let mut jmp_code = vec![0x83, 0xc0, 0x05]; // add eax, 5
        emit_rel32(&mut jmp_code, &[0xe9], jmp_tgt, after_jcc); // jmp after_jcc
        let mut jcc_taken_code = vec![0x83, 0xc0, 0x07]; // add eax, 7
        emit_rel32(&mut jcc_taken_code, &[0xe9], jcc_taken_tgt, after_jcc);
        let mut endrun_code = vec![0x40u8; 64]; // 64 x `inc eax` -> EndRun!
        endrun_code.extend_from_slice(&[0x83, 0xc0, 0x03, 0xc3]); // add eax, 3; ret

        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[ECX] = 5;
        cpu.r[ESP] = stack;
        assert_native_matches_rust(
            "smc_chained_static_transfers",
            &main,
            cpu,
            &[
                (call_tgt, &call_code),
                (jmp_tgt, &jmp_code),
                (jcc_taken_tgt, &jcc_taken_code),
                (endrun_page, &endrun_code),
            ],
            &[
                (call_tgt, 4),
                (jmp_tgt, 8),
                (jcc_taken_tgt, 8),
                (endrun_cont, 4),
            ],
            10_000,
            0..0,
            DOS_MEMORY,
        );

        // Also assert directly that native execution actually patches static chain slots
        // and re-chains them cleanly after SMC invalidation.
        {
            let mut h = Headless::new();
            let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
            mem.write(code_at, &main).unwrap();
            mem.write(call_tgt, &call_code).unwrap();
            mem.write(jmp_tgt, &jmp_code).unwrap();
            mem.write(jcc_taken_tgt, &jcc_taken_code).unwrap();
            mem.write(endrun_page, &endrun_code).unwrap();
            let mut c = cpu;
            let mut interp = Interp::new(Mode::FLAT);
            let (_, res) = interp.run_native(&mut c, &mut mem, 10_000, 0..0);
            assert_eq!(res, Err(Stop::Halted));
            let chained_static = interp
                .fast_insns
                .iter()
                .filter(|ins| {
                    matches!(
                        ins.op,
                        FastOp::JmpRel | FastOp::JccRel | FastOp::CallRel | FastOp::EndRun
                    ) && ins.imm != 0
                })
                .count();
            assert!(
                chained_static >= 4,
                "expected JmpRel/JccRel/CallRel/EndRun chain slots to be patched, got {chained_static}"
            );
            let jcc_fallthrough_patched = interp.fast_insns.iter().any(|ins| {
                ins.op == FastOp::JccRel
                    && u32::from_le_bytes([ins.base, ins.index, ins.scale, ins.aux]) != 0
            });
            assert!(
                jcc_fallthrough_patched,
                "expected JccRel fallthrough chain slot to be patched"
            );
        }

        // 2. L1 block-cache collision / eviction ping-pong via indirect transfers.
        // Pick pairs of addresses that hash to the exact same L1 slot (including slot 0!).
        let slot0_a = 0x0018_0180u32; // (0x180 ^ 0x180) & 0xfff == 0
        let slot0_b = 0x0018_1181u32; // (0x181 ^ 0x181) & 0xfff == 0
        let slotx_a = 0x0018_2001u32; // (0x001 ^ 0x182) & 0xfff == 0x183
        let slotx_b = 0x0018_3000u32; // (0x000 ^ 0x183) & 0xfff == 0x183
        assert_eq!(l1_slot_index(slot0_a), 0);
        assert_eq!(l1_slot_index(slot0_b), 0);
        assert_eq!(l1_slot_index(slotx_a), l1_slot_index(slotx_b));

        // Caller calls slot0_a, slot0_b, slotx_a, slotx_b indirectly (CallReg / JmpReg / Ret) 20 times!
        let caller = 0x0018_6000u32;
        let data = 0x0020_0000u32;
        let mut coll_prog = Vec::new();
        // Loop: call eax (slot0_a); call ebx (slot0_b); call [esi] (slotx_a); call [esi+4] (slotx_b); dec ecx; jnz loop; hlt
        coll_prog.extend_from_slice(&[0xff, 0xd0]); // call eax
        coll_prog.extend_from_slice(&[0xff, 0xd3]); // call ebx
        coll_prog.extend_from_slice(&[0xff, 0x16]); // call dword [esi]
        coll_prog.extend_from_slice(&[0xff, 0x56, 0x04]); // call dword [esi+4]
        coll_prog.push(0x49); // dec ecx
        let back = -(coll_prog.len() as i8 + 2);
        coll_prog.extend_from_slice(&[0x75, back as u8]); // jnz loop
        coll_prog.push(0xf4); // hlt

        let mut ptrs = [0u8; 8];
        ptrs[0..4].copy_from_slice(&slotx_a.to_le_bytes());
        ptrs[4..8].copy_from_slice(&slotx_b.to_le_bytes());
        let mut cpu = Cpu {
            eip: caller,
            ..Cpu::default()
        };
        cpu.r[EAX] = slot0_a;
        cpu.r[EBX] = slot0_b;
        cpu.r[ECX] = 20;
        cpu.r[ESI] = data;
        cpu.r[ESP] = stack;
        assert_native_matches_rust(
            "l1_slot_collision_eviction",
            &coll_prog,
            cpu,
            &[
                (slot0_a, &[0x83, 0xc2, 0x01, 0xc3]), // add edx, 1; ret
                (slot0_b, &[0x83, 0xc2, 0x03, 0xc3]), // add edx, 3; ret
                (slotx_a, &[0x83, 0xc2, 0x07, 0xc3]), // add edx, 7; ret
                (slotx_b, &[0x83, 0xc2, 0x0b, 0xc3]), // add edx, 11; ret
                (data, &ptrs),
            ],
            &[(stack - 16, 20)],
            5_000,
            0..0,
            DOS_MEMORY,
        );

        // 3. Custom `outside` ranges (below & above outside, near-outside non-overlapping,
        // wrapping `outside_danger_start`, huge `outside`, and `outside` hitting an already-chained target).
        let below = 0x0018_8000u32;
        let above = 0x0024_0000u32;
        let mut split_prog = Vec::new();
        // `inc eax; call above; inc eax; hlt`
        split_prog.push(0x40);
        emit_rel32(&mut split_prog, &[0xe8], below, above);
        split_prog.extend_from_slice(&[0x40, 0xf4]);
        let above_code = [0x40, 0x40, 0x40, 0xc3]; // 3 x inc eax; ret

        for (name, outside) in [
            (
                "outside_between_below_and_above",
                0x0020_0000u32..0x0021_0000,
            ),
            ("outside_near_below_no_overlap", (below + 64)..(below + 128)),
            ("outside_mid_above_overlap", (above + 1)..(above + 2)),
            ("outside_entry_of_above", above..(above + 1)),
            ("outside_wraps_danger_start", 0..1),
            ("outside_huge_gt_u32_max_danger", 0..u32::MAX),
            ("outside_boundary_range", Boundary::RANGE),
        ] {
            let mut c = Cpu {
                eip: below,
                ..Cpu::default()
            };
            c.r[ESP] = stack;
            assert_native_matches_rust(
                name,
                &split_prog,
                c,
                &[(above, &above_code)],
                &[],
                500,
                outside,
                DOS_MEMORY,
            );
        }

        // Verify an ALREADY-CHAINED `Interp` still stops/clamps when re-invoked with a new `outside`!
        let mut h_r = Headless::new();
        let mut h_n = Headless::new();
        let mut m_r = Arena::new(&mut h_r, 1 << 22)
            .unwrap()
            .with_floor(DOS_MEMORY);
        let mut m_n = Arena::new(&mut h_n, 1 << 22)
            .unwrap()
            .with_floor(DOS_MEMORY);
        m_r.write(below, &split_prog).unwrap();
        m_n.write(below, &split_prog).unwrap();
        m_r.write(above, &above_code).unwrap();
        m_n.write(above, &above_code).unwrap();
        let mut i_r = Interp::new(Mode::FLAT);
        let mut i_n = Interp::new(Mode::FLAT);

        // Pass 1: run with empty `outside` so `below -> above` gets direct-chained!
        let mut warm_r = Cpu {
            eip: below,
            ..Cpu::default()
        };
        warm_r.r[ESP] = stack;
        let mut warm_n = warm_r;
        let _ = rust::run(&mut i_r, &mut warm_r, &mut m_r, 500, 0..0);
        let _ = i_n.run_native(&mut warm_n, &mut m_n, 500, 0..0);
        assert_eq!(warm_n.r, warm_r.r);

        // Pass 2: re-run on the SAME chained `Interp` with `outside` at entry & mid-block of `above`!
        for outside in [above..(above + 1), (above + 2)..(above + 3)] {
            let mut cr = Cpu {
                eip: below,
                ..Cpu::default()
            };
            cr.r[ESP] = stack;
            let mut cn = cr;
            let (rr, sr) = rust::run(&mut i_r, &mut cr, &mut m_r, 500, outside.clone());
            let (rn, sn) = i_n.run_native(&mut cn, &mut m_n, 500, outside.clone());
            assert_eq!(
                (rn, sn, cn.eip, cn.r, cn.tsc),
                (rr, sr, cr.eip, cr.r, cr.tsc)
            );
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_x87_inline_all_ops_edges_nan_pc0_rc_ftol_stack_and_faults() {
        use crate::cpu::x87::{INDEFINITE, X87};
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;
        let fs_base = 0x0022_0000u32;

        let make_fpu = |top_pushes: usize, vals: &[f64], control: u16, status: u16| -> X87 {
            let mut fpu = X87::new();
            for _ in 0..top_pushes {
                fpu.decrement_top();
            }
            for &v in vals {
                fpu.push(v);
            }
            fpu.set_control(control);
            if status != 0 {
                // Compare against NaN or equal to seed non-zero status bits if needed.
                fpu.compare(f64::NAN, 1.0);
            }
            fpu
        };

        let edge_f64: &[f64] = &[
            0.0,
            -0.0,
            1.0,
            -1.0,
            0.5,
            -0.5,
            2.25,
            -3.75,
            1e-100,
            -1e100,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::from_bits(1),                     // smallest +subnormal
            f64::from_bits(0x8000_0000_0000_0001), // smallest -subnormal
            f64::MAX,
            -f64::MAX,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            INDEFINITE,
            f64::from_bits(0x7ff0_0000_0000_0001), // +sNaN
            f64::from_bits(0xfff0_0000_0000_0007), // -sNaN
            f64::from_bits(0x7ff8_0000_0000_0005), // +qNaN with payload
        ];

        let edge_f32_bits: &[u32] = &[
            0.0f32.to_bits(),
            (-0.0f32).to_bits(),
            1.0f32.to_bits(),
            (-2.5f32).to_bits(),
            0.1f32.to_bits(),
            1,           // +subnormal f32
            0x8000_0001, // -subnormal f32
            f32::MAX.to_bits(),
            f32::INFINITY.to_bits(),
            f32::NEG_INFINITY.to_bits(),
            0x7fc0_0000, // +qNaN
            0xffc0_0003, // -qNaN payload
            0x7f80_0001, // +sNaN
            0xff80_0005, // -sNaN
        ];

        // 1. Register unary/load/store/fxch/compare ops across stack wraps (top 0..8),
        //    control words (PC=0 Single, PC=2 Double, PC=3 Extended), and edge values.
        let reg_ops_1arg: &[(&str, &[u8])] = &[
            ("fnop", &[0xd9, 0xd0, 0xf4]),
            ("fchs", &[0xd9, 0xe0, 0xf4]),
            ("fabs", &[0xd9, 0xe1, 0xf4]),
            ("ftst", &[0xd9, 0xe4, 0xf4]),
            ("fld1", &[0xd9, 0xe8, 0xf4]),
            ("fldz", &[0xd9, 0xee, 0xf4]),
            ("fsqrt", &[0xd9, 0xfa, 0xf4]),
            ("fnstsw_ax", &[0xdf, 0xe0, 0xf4]),
            ("fld_st0", &[0xd9, 0xc0, 0xf4]),
            ("fld_st2", &[0xd9, 0xc2, 0xf4]),
            ("fxch_0", &[0xd9, 0xc8, 0xf4]),
            ("fxch_1", &[0xd9, 0xc9, 0xf4]),
            ("fxch_3", &[0xd9, 0xcb, 0xf4]),
            ("fst_st0", &[0xdd, 0xd0, 0xf4]),
            ("fst_st2", &[0xdd, 0xd2, 0xf4]),
            ("fst_st5_unocc", &[0xdd, 0xd5, 0xf4]),
            ("fstp_st0", &[0xdd, 0xd8, 0xf4]),
            ("fstp_st2", &[0xdd, 0xda, 0xf4]),
            ("fstp_st5_unocc", &[0xdd, 0xdd, 0xf4]),
            ("fcom_st1", &[0xd8, 0xd1, 0xf4]),
            ("fucom_st2", &[0xdd, 0xe2, 0xf4]),
            ("fcomp_st1", &[0xd8, 0xd9, 0xf4]),
            ("fucomp_st2", &[0xdd, 0xea, 0xf4]),
            ("fcompp", &[0xde, 0xd9, 0xf4]),
            ("fucompp", &[0xda, 0xe9, 0xf4]),
        ];

        for &(name, code) in reg_ops_1arg {
            for &ctrl in &[0x007fu16, 0x027f, 0x037f] {
                for shift in 0..8usize {
                    for &v0 in &[
                        0.0,
                        -0.0,
                        -4.0,
                        9.0,
                        f64::INFINITY,
                        f64::NEG_INFINITY,
                        f64::from_bits(1),
                        f64::from_bits(0x7ff0_0000_0000_0001),
                        INDEFINITE,
                    ] {
                        let mut cpu = Cpu {
                            eip: code_at,
                            fpu: make_fpu(shift, &[1.5, -2.5, 0.25, v0], ctrl, 1),
                            ..Cpu::default()
                        };
                        cpu.r[EAX] = 0xdead_beef;
                        assert_native_matches_rust(name, code, cpu, &[], &[], 20, 0..0, DOS_MEMORY);
                    }
                }
            }
        }

        // 2. Fcomi / Fucomi / Fcomip / Fucomip preserving lazy & materialized AF!
        for &(name, op_bytes) in &[
            ("fcomi", [0xdbu8, 0xf1]),
            ("fucomi", [0xdb, 0xe9]),
            ("fcomip", [0xdf, 0xf1]),
            ("fucomip", [0xdf, 0xe9]),
        ] {
            for &(a, b) in &[
                (2.0, 1.0),
                (1.0, 2.0),
                (1.5, 1.5),
                (-0.0, 0.0),
                (f64::INFINITY, 1.0),
                (f64::NEG_INFINITY, f64::INFINITY),
                (f64::NAN, 1.0),
                (1.0, f64::from_bits(0x7ff0_0000_0000_0001)),
            ] {
                // Test both materialized AF (0/1) and lazy AF (from `add eax, ebx`: 0x0f+1 sets AF, 1+1 clears AF)
                for &(eax_init, ebx_init, prefix_add) in &[
                    (0u32, 0u32, false),
                    (0x0f, 1, true), // lazy AF = 1
                    (0x01, 1, true), // lazy AF = 0
                ] {
                    for af_mat in [false, true] {
                        let mut prog = Vec::new();
                        if prefix_add {
                            prog.extend_from_slice(&[0x01, 0xd8]); // add eax, ebx (lazy flags on aarch64!)
                        }
                        prog.extend_from_slice(&op_bytes);
                        prog.extend_from_slice(&[0x0f, 0x92, 0xc1]); // setb cl
                        prog.extend_from_slice(&[0x0f, 0x94, 0xc5]); // sete ch
                        prog.extend_from_slice(&[0x0f, 0x9a, 0xc2]); // setp dl
                        prog.push(0xf4);

                        let mut cpu = Cpu {
                            eip: code_at,
                            fpu: make_fpu(7, &[b, a], 0x037f, 0),
                            flags: Flags::from_eflags(
                                CF | PF | ZF | SF | OF | if af_mat { AF } else { 0 },
                            ),
                            ..Cpu::default()
                        };
                        cpu.r[EAX] = eax_init;
                        cpu.r[EBX] = ebx_init;
                        assert_native_matches_rust(
                            name,
                            &prog,
                            cpu,
                            &[],
                            &[],
                            30,
                            0..0,
                            DOS_MEMORY,
                        );
                    }
                }
            }
        }

        // 3. All 18 register binary arithmetic ops (St0 / Sti / Pop) across edge pairs
        //    (including NaN-generating 0/0, Inf-Inf, 0*Inf, Inf/Inf, sNaNs, i==0, i==1, PC=0).
        let binop_pairs: &[(f64, f64)] = &[
            (3.5, 1.25),
            (-3.5, 1.25),
            (0.0, 0.0),
            (-0.0, 0.0),
            (0.0, -0.0),
            (1.0, 0.0),
            (-1.0, -0.0),
            (f64::INFINITY, f64::INFINITY),
            (f64::INFINITY, f64::NEG_INFINITY),
            (0.0, f64::INFINITY),
            (f64::from_bits(1), f64::from_bits(0x8000_0000_0000_0001)),
            (f64::from_bits(0x7ff0_0000_0000_0001), 2.0),
            (2.0, f64::from_bits(0xfff0_0000_0000_0007)),
            (
                f64::from_bits(0x7ff0_0000_0000_0001),
                f64::from_bits(0xfff8_0000_0000_0009),
            ),
        ];
        for &esc in &[0xd8u8, 0xdc, 0xde] {
            for &reg_base in &[0xc0u8, 0xc8, 0xe0, 0xe8, 0xf0, 0xf8] {
                for i in [0u8, 1, 2] {
                    let prog = [esc, reg_base + i, 0xf4];
                    for &(a, b) in binop_pairs {
                        for &ctrl in &[0x007fu16, 0x037f] {
                            let cpu = Cpu {
                                eip: code_at,
                                fpu: make_fpu(5, &[7.0, b, a], ctrl, 0),
                                ..Cpu::default()
                            };
                            assert_native_matches_rust(
                                "reg_binop",
                                &prog,
                                cpu,
                                &[],
                                &[],
                                10,
                                0..0,
                                DOS_MEMORY,
                            );
                        }
                    }
                }
            }
        }

        // 4. Memory loads (FldF32, FldF64, FildI32, FildI16), stores (Fst/Fstp F32/F64),
        //    arithmetic & compares (F32 & F64), plus FS: override & unaligned addresses.
        for &bits in edge_f32_bits {
            let mem_bytes = bits.to_le_bytes();
            for &(name, prog) in &[
                ("fld_f32", &[0xd9u8, 0x06, 0xf4][..]),
                ("fld_f32_fs", &[0x64u8, 0xd9, 0x06, 0xf4][..]),
                ("fadd_f32", &[0xd8, 0x06, 0xf4][..]),
                ("fmul_f32", &[0xd8, 0x0e, 0xf4][..]),
                ("fcom_f32", &[0xd8, 0x16, 0xf4][..]),
                ("fcomp_f32", &[0xd8, 0x1e, 0xf4][..]),
                ("fsub_f32", &[0xd8, 0x26, 0xf4][..]),
                ("fsubr_f32", &[0xd8, 0x2e, 0xf4][..]),
                ("fdiv_f32", &[0xd8, 0x36, 0xf4][..]),
                ("fdivr_f32", &[0xd8, 0x3e, 0xf4][..]),
            ] {
                for &st0 in &[
                    0.0,
                    -0.0,
                    2.5,
                    f64::INFINITY,
                    f64::from_bits(0x7ff0_0000_0000_0001),
                ] {
                    for &ctrl in &[0x007fu16, 0x037f] {
                        let mut cpu = Cpu {
                            eip: code_at,
                            fpu: make_fpu(6, &[st0], ctrl, 1),
                            ..Cpu::default()
                        };
                        cpu.r[ESI] = if name == "fld_f32_fs" { 3 } else { data + 1 }; // odd unaligned offset!
                        cpu.seg_base[FS] = fs_base;
                        assert_native_matches_rust(
                            name,
                            prog,
                            cpu,
                            &[(data + 1, &mem_bytes), (fs_base + 3, &mem_bytes)],
                            &[],
                            10,
                            0..0,
                            DOS_MEMORY,
                        );
                    }
                }
            }
        }

        for &val in edge_f64 {
            let mem_bytes = val.to_le_bytes();
            for &(name, prog) in &[
                ("fld_f64", &[0xddu8, 0x06, 0xf4][..]),
                ("fadd_f64", &[0xdc, 0x06, 0xf4][..]),
                ("fmul_f64", &[0xdc, 0x0e, 0xf4][..]),
                ("fcom_f64", &[0xdc, 0x16, 0xf4][..]),
                ("fcomp_f64", &[0xdc, 0x1e, 0xf4][..]),
                ("fsub_f64", &[0xdc, 0x26, 0xf4][..]),
                ("fsubr_f64", &[0xdc, 0x2e, 0xf4][..]),
                ("fdiv_f64", &[0xdc, 0x36, 0xf4][..]),
                ("fdivr_f64", &[0xdc, 0x3e, 0xf4][..]),
                ("fst_f32", &[0xd9, 0x16, 0xf4][..]),
                ("fstp_f32", &[0xd9, 0x1e, 0xf4][..]),
                ("fst_f64", &[0xdd, 0x16, 0xf4][..]),
                ("fstp_f64", &[0xdd, 0x1e, 0xf4][..]),
            ] {
                for &ctrl in &[0x007fu16, 0x037f] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        fpu: make_fpu(0, &[val], ctrl, 1),
                        ..Cpu::default()
                    };
                    cpu.r[ESI] = data + 3; // unaligned 8-byte offset
                    assert_native_matches_rust(
                        name,
                        prog,
                        cpu,
                        &[(data + 3, &mem_bytes)],
                        &[(data + 3, 8)],
                        10,
                        0..0,
                        DOS_MEMORY,
                    );
                }
            }
        }

        // FildI16 & FildI32 across extremes & PC=0/PC=3
        for &i16v in &[0i16, 1, -1, i16::MIN, i16::MAX, 12345, -23456] {
            for &ctrl in &[0x007fu16, 0x037f] {
                let mut cpu = Cpu {
                    eip: code_at,
                    fpu: make_fpu(3, &[], ctrl, 0),
                    ..Cpu::default()
                };
                cpu.r[ESI] = data + 1;
                assert_native_matches_rust(
                    "fild_i16",
                    &[0xdf, 0x06, 0xf4],
                    cpu,
                    &[(data + 1, &i16v.to_le_bytes())],
                    &[],
                    10,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }
        for &i32v in &[0i32, 1, -1, i32::MIN, i32::MAX, 0x0100_0001, -0x0100_0001] {
            for &ctrl in &[0x007fu16, 0x037f] {
                let mut cpu = Cpu {
                    eip: code_at,
                    fpu: make_fpu(4, &[], ctrl, 0),
                    ..Cpu::default()
                };
                cpu.r[ESI] = data + 1;
                assert_native_matches_rust(
                    "fild_i32",
                    &[0xdb, 0x06, 0xf4],
                    cpu,
                    &[(data + 1, &i32v.to_le_bytes())],
                    &[],
                    10,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }

        // 5. FistpI32 & FistpI64 across RC=0..3, PC=0..3, and ALL _ftol boundaries + full MSVC _ftol idiom!
        let ftol_vals: &[f64] = &[
            0.0,
            -0.0,
            0.25,
            -0.25,
            0.5,
            -0.5,
            0.999_999_999,
            -0.999_999_999,
            1.5,
            -1.5,
            2.5,
            -2.5,
            -1_073_741_824.5,          // < -2^30 (RC=0 bail, RC=3 inline)
            -1_073_741_824.0,          // exact -2^30 (RC=0 bail)
            -1_073_741_823.5,          // > -2^30 (RC=0 inline!)
            1_073_741_823.5,           // < +2^30 (RC=0 inline!)
            1_073_741_824.0,           // exact +2^30 (RC=0 bail)
            -2_147_483_648.0,          // exact -2^31 (in range for Fistp32!)
            -2_147_483_648.5,          // < -2^31 (bail!)
            -2_147_483_649.0,          // -2^31 - 1 (bail!)
            -2_147_483_647.999_999_5,  // > -2^31 (in range!)
            2_147_483_647.0,           // 2^31 - 1 (in range!)
            2_147_483_647.999_999_5,   // < 2^31 (in range!)
            2_147_483_648.0,           // exact +2^31 (bail for Fistp32, in range for Fistp64!)
            2_147_483_649.0,           // +2^31 + 1
            ((1u64 << 52) - 1) as f64, // 2^52 - 1 (in range for Fistp64!)
            -(((1u64 << 52) - 1) as f64),
            (1u64 << 52) as f64 - 0.5, // < 2^52 (in range for Fistp64!)
            -((1u64 << 52) as f64 - 0.5),
            (1u64 << 52) as f64,    // exact 2^52 (bail for Fistp64!)
            -((1u64 << 52) as f64), // exact -2^52 (bail for Fistp64!)
            (1u64 << 53) as f64,
            f64::from_bits(1),
            f64::from_bits(0x8000_0000_0000_0001),
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            INDEFINITE,
            f64::from_bits(0x7ff0_0000_0000_0001),
        ];
        for &val in ftol_vals {
            for rc in 0u16..4 {
                for pc in [0u16, 2, 3] {
                    let ctrl = 0x007f | (pc << 8) | (rc << 10);
                    for &(name, prog) in &[
                        ("fistp_i32", &[0xdbu8, 0x1e, 0xf4][..]),
                        ("fistp_i64", &[0xdfu8, 0x3e, 0xf4][..]),
                    ] {
                        let mut cpu = Cpu {
                            eip: code_at,
                            fpu: make_fpu(2, &[val], ctrl, 0),
                            ..Cpu::default()
                        };
                        cpu.r[ESI] = data + 3;
                        assert_native_matches_rust(
                            name,
                            prog,
                            cpu,
                            &[],
                            &[(data + 3, 8)],
                            10,
                            0..0,
                            DOS_MEMORY,
                        );
                    }
                }
            }
        }

        // Full MSVC `_ftol` sequence (`fnstcw`, `or ah, 0xc`, `fldcw`, `fistp qword`, `fldcw`, `mov eax`, `mov edx`)
        let msvc_ftol: &[u8] = &[
            0xd9, 0x3e, // fnstcw [esi]
            0x66, 0x8b, 0x06, // mov ax, [esi]
            0x80, 0xcc, 0x0c, // or ah, 0x0c (RC = 3)
            0x66, 0x89, 0x46, 0x02, // mov [esi+2], ax
            0xd9, 0x6e, 0x02, // fldcw [esi+2]
            0xdf, 0x7e, 0x08, // fistp qword [esi+8]
            0xd9, 0x2e, // fldcw [esi]
            0x8b, 0x46, 0x08, // mov eax, [esi+8]
            0x8b, 0x56, 0x0c, // mov edx, [esi+12]
            0xf4, // hlt
        ];
        for &val in &[
            0.0,
            -0.0,
            42.9,
            -42.9,
            1e12,
            -1e12,
            -2_147_483_648.0,
            2_147_483_648.0,
            f64::NAN,
        ] {
            let mut cpu = Cpu {
                eip: code_at,
                fpu: make_fpu(0, &[val], 0x037f, 0),
                ..Cpu::default()
            };
            cpu.r[ESI] = data;
            assert_native_matches_rust(
                "msvc_ftol",
                msvc_ftol,
                cpu,
                &[],
                &[(data, 16)],
                50,
                0..0,
                DOS_MEMORY,
            );
        }

        // 6. Empty-stack underflow & full-stack overflow faults across ops!
        let mut full_fpu = X87::new();
        for k in 1..=8 {
            full_fpu.push(f64::from(k));
        }
        let empty_fpu = X87::new();
        for &(name, prog) in &[
            ("empty_fchs", &[0xd9u8, 0xe0, 0xf4][..]),
            ("empty_fabs", &[0xd9, 0xe1, 0xf4][..]),
            ("empty_ftst", &[0xd9, 0xe4, 0xf4][..]),
            ("empty_fsqrt", &[0xd9, 0xfa, 0xf4][..]),
            ("empty_fcompp", &[0xde, 0xd9, 0xf4][..]),
            ("empty_fld_st0", &[0xd9, 0xc0, 0xf4][..]),
            ("empty_fxch", &[0xd9, 0xc9, 0xf4][..]),
            ("empty_fst_st1", &[0xdd, 0xd1, 0xf4][..]),
            ("empty_fstp_st0", &[0xdd, 0xd8, 0xf4][..]),
            ("empty_fcom_st1", &[0xd8, 0xd1, 0xf4][..]),
            ("empty_fcomi", &[0xdb, 0xf1, 0xf4][..]),
            ("empty_fadd_st0", &[0xd8, 0xc1, 0xf4][..]),
            ("empty_faddp", &[0xde, 0xc1, 0xf4][..]),
            ("empty_fadd_f32", &[0xd8, 0x06, 0xf4][..]),
            ("empty_fadd_f64", &[0xdc, 0x06, 0xf4][..]),
            ("empty_fstp_f32", &[0xd9, 0x1e, 0xf4][..]),
            ("empty_fstp_f64", &[0xdd, 0x1e, 0xf4][..]),
            ("empty_fistp_i32", &[0xdb, 0x1e, 0xf4][..]),
            ("empty_fistp_i64", &[0xdf, 0x3e, 0xf4][..]),
        ] {
            let mut cpu = Cpu {
                eip: code_at,
                fpu: empty_fpu,
                ..Cpu::default()
            };
            cpu.fpu.set_control(0x0f7f); // RC=3 so Fistp tests empty check too
            cpu.r[ESI] = data;
            assert_native_matches_rust(
                name,
                prog,
                cpu,
                &[(data, &1.5f64.to_le_bytes())],
                &[(data, 8)],
                10,
                0..0,
                DOS_MEMORY,
            );
        }
        for &(name, prog) in &[
            ("full_fld1", &[0xd9u8, 0xe8, 0xf4][..]),
            ("full_fldz", &[0xd9, 0xee, 0xf4][..]),
            ("full_fld_st0", &[0xd9, 0xc0, 0xf4][..]),
            ("full_fld_f32", &[0xd9, 0x06, 0xf4][..]),
            ("full_fld_f64", &[0xdd, 0x06, 0xf4][..]),
            ("full_fild_i32", &[0xdb, 0x06, 0xf4][..]),
            ("full_fild_i16", &[0xdf, 0x06, 0xf4][..]),
        ] {
            let mut cpu = Cpu {
                eip: code_at,
                fpu: full_fpu,
                ..Cpu::default()
            };
            cpu.r[ESI] = data;
            assert_native_matches_rust(
                name,
                prog,
                cpu,
                &[(data, &1.5f64.to_le_bytes())],
                &[],
                10,
                0..0,
                DOS_MEMORY,
            );
        }

        // 7. Memory faults (floor & arena end for 2B/4B/8B reads & writes), page-crossing writes,
        //    and code-page write hits (code_bitmap) for Fst/Fstp/Fistp/Fnstcw!
        let end = 1u32 << 22;
        for &(name, prog, addr) in &[
            (
                "x87_fault_fld_f32_floor",
                &[0x40u8, 0xd9, 0x06, 0xf4][..],
                DOS_MEMORY - 2,
            ),
            (
                "x87_fault_fld_f64_end4",
                &[0x40, 0xdd, 0x06, 0xf4][..],
                end - 4,
            ),
            (
                "x87_fault_fild_i16_end1",
                &[0x40, 0xdf, 0x06, 0xf4][..],
                end - 1,
            ),
            (
                "x87_fault_fldcw_end1",
                &[0x40, 0xd9, 0x2e, 0xf4][..],
                end - 1,
            ),
            (
                "x87_fault_fadd_f64_end4",
                &[0x40, 0xdc, 0x06, 0xf4][..],
                end - 4,
            ),
            (
                "x87_fault_fstp_f32_end2",
                &[0x40, 0xd9, 0x1e, 0xf4][..],
                end - 2,
            ),
            (
                "x87_fault_fstp_f64_end4",
                &[0x40, 0xdd, 0x1e, 0xf4][..],
                end - 4,
            ),
            (
                "x87_fault_fistp_i32_floor",
                &[0x40, 0xdb, 0x1e, 0xf4][..],
                DOS_MEMORY - 4,
            ),
            (
                "x87_fault_fistp_i64_end4",
                &[0x40, 0xdf, 0x3e, 0xf4][..],
                end - 4,
            ),
            (
                "x87_fault_fnstcw_end1",
                &[0x40, 0xd9, 0x3e, 0xf4][..],
                end - 1,
            ),
            // Page-crossing writes (valid memory, crosses 4 KiB page boundary!)
            ("x87_cross_fstp_f32", &[0xd9, 0x1e, 0xf4][..], data + 0xffe),
            ("x87_cross_fstp_f64", &[0xdd, 0x1e, 0xf4][..], data + 0xffc),
            ("x87_cross_fistp_i32", &[0xdb, 0x1e, 0xf4][..], data + 0xffe),
            ("x87_cross_fistp_i64", &[0xdf, 0x3e, 0xf4][..], data + 0xffc),
            ("x87_cross_fnstcw", &[0xd9, 0x3e, 0xf4][..], data + 0xfff),
        ] {
            let mut cpu = Cpu {
                eip: code_at,
                fpu: make_fpu(0, &[12.5], 0x0f7f, 0),
                ..Cpu::default()
            };
            cpu.r[ESI] = addr;
            let checks: &[(u32, usize)] = if addr < end - 8 && addr >= DOS_MEMORY {
                &[(addr, 8)]
            } else {
                &[]
            };
            assert_native_matches_rust(name, prog, cpu, &[], checks, 20, 0..0, DOS_MEMORY);
        }

        // Code-page write hit via FstpF64 / FistpI64 / Fnstcw into a decoded target page (SMC!)
        let target_page = 0x0018_2000u32;
        let stack = 0x0030_0000u32;
        let mut smc = Vec::new();
        let rel1 = target_page.wrapping_sub(code_at + 5).to_le_bytes();
        smc.push(0xe8);
        smc.extend_from_slice(&rel1); // call target_page (marks page as code)
        smc.extend_from_slice(&[0xdf, 0x3e]); // fistp qword [esi] -> overwrites target_page with `mov eax, 99; ret`!
        let rel2 = target_page
            .wrapping_sub(code_at + smc.len() as u32 + 5)
            .to_le_bytes();
        smc.push(0xe8);
        smc.extend_from_slice(&rel2); // call target_page again!
        smc.push(0xf4);
        // Encode `mov eax, 99; ret; nop; nop` as a finite integer value (< 2^52) in f64!
        let new_code_u64 = u64::from_le_bytes([0xb8, 99, 0, 0, 0, 0xc3, 0x00, 0x00]);
        assert!(new_code_u64 < (1u64 << 52));
        let mut cpu = Cpu {
            eip: code_at,
            fpu: make_fpu(0, &[new_code_u64 as f64], 0x0f7f, 0),
            ..Cpu::default()
        };
        cpu.r[ESI] = target_page;
        cpu.r[ESP] = stack;
        assert_native_matches_rust(
            "x87_smc_fistp64_code_bitmap",
            &smc,
            cpu,
            &[(target_page, &[0xb8, 1, 0, 0, 0, 0xc3, 0x90, 0x90])],
            &[(target_page, 8)],
            50,
            0..0,
            DOS_MEMORY,
        );

        // 8. Prove hot inlined x87 loop executes inside run_fast_block with ZERO STATUS_FALLBACK!
        {
            let mut h = Headless::new();
            let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
            // Straight-line + loop using FldF32, FmulF32, FaddF64, Fsqrt, Fchs, Fabs, Fxch,
            // FcomSt, FnstswAx, FstF32, FstpF64, FildI32, FildI16, Fnstcw, Fldcw, FistpI64:
            let mut loop_prog = Vec::new();
            loop_prog.extend_from_slice(&[0xd9, 0x06]); // fld dword [esi]
            loop_prog.extend_from_slice(&[0xd8, 0x4e, 0x04]); // fmul dword [esi+4]
            loop_prog.extend_from_slice(&[0xdc, 0x46, 0x08]); // fadd qword [esi+8]
            loop_prog.extend_from_slice(&[0xd9, 0xfa]); // fsqrt
            loop_prog.extend_from_slice(&[0xd9, 0xe0]); // fchs
            loop_prog.extend_from_slice(&[0xd9, 0xe1]); // fabs
            loop_prog.extend_from_slice(&[0xd9, 0xe8]); // fld1
            loop_prog.extend_from_slice(&[0xd9, 0xc9]); // fxch st(1)
            loop_prog.extend_from_slice(&[0xde, 0xc1]); // faddp st(1)
            loop_prog.extend_from_slice(&[0xd9, 0x56, 0x10]); // fst dword [esi+16]
            loop_prog.extend_from_slice(&[0xdf, 0x7e, 0x18]); // fistp qword [esi+24]
            loop_prog.extend_from_slice(&[0xd9, 0xee]); // fldz
            loop_prog.extend_from_slice(&[0xd9, 0xe4]); // ftst
            loop_prog.extend_from_slice(&[0xdf, 0xe0]); // fnstsw ax
            loop_prog.extend_from_slice(&[0xdd, 0xd8]); // fstp st(0)
            loop_prog.push(0x49); // dec ecx
            let back = -(loop_prog.len() as i8 + 2);
            loop_prog.extend_from_slice(&[0x75, back as u8]); // jnz loop_prog
            mem.write(code_at, &loop_prog).unwrap();
            mem.write(data, &2.0f32.to_le_bytes()).unwrap();
            mem.write(data + 4, &4.5f32.to_le_bytes()).unwrap();
            mem.write(data + 8, &7.0f64.to_le_bytes()).unwrap();

            let mut interp = Interp::new(Mode::FLAT);
            // Pre-decode the loop block
            let _ = interp.run_at(code_at, &mut mem).unwrap();
            // Verify ZERO STATUS_FALLBACK across Extended+RC=3, Single(PC=0)+RC=3, and Double+RC=0!
            for &ctrl in &[0x0f7fu16, 0x0c3f, 0x003f, 0x127f] {
                let mut cpu = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu.fpu.set_control(ctrl);
                cpu.r[ESI] = data;
                cpu.r[ECX] = 50;

                let (arena_base, arena_floor, arena_len, code_bitmap) = mem.raw_parts_mut();
                let (code_bbox_start, code_bbox_len) = encode_code_bbox(&mem).unwrap();
                let mut fast_regs = [0u32; 9];
                fast_regs[..8].copy_from_slice(&cpu.r);
                let mut ctx = FastCtx {
                    regs: fast_regs.as_mut_ptr(),
                    arena_base,
                    code_bitmap,
                    arena_limit4: (arena_len - arena_floor as usize - 4) as u64,
                    remaining: 50 * 17,
                    retired: 0,
                    fast_insns: interp.fast_insns.as_mut_ptr(),
                    runs: interp.runs.as_ptr(),
                    directory: interp.directory.as_ptr(),
                    region_ptrs: interp.region_ptrs.as_ptr(),
                    page_ptrs: interp.page_ptrs.as_ptr(),
                    fault_pc: core::ptr::null(),
                    run_first_pc: core::ptr::null(),
                    eip: cpu.eip,
                    flags: native::pack_flags(&cpu.flags),
                    fs_base: 0,
                    arena_floor,
                    outside_start: 0,
                    outside_end: 0,
                    outside_len: 0,
                    run_start_eip: 0,
                    run_bytes: 0,
                    run_len: 0,
                    l1_cache: interp.l1.as_mut_ptr(),
                    outside_danger_len: 0,
                    outside_danger_start: 0,
                    _pad: 0,
                    fpu: &raw mut cpu.fpu,
                    code_bbox_start,
                    code_bbox_len,
                };
                // All 50 iterations (850 instructions!) must finish in ONE call with STATUS_DONE!
                let status = unsafe { native::run_fast_block(&mut ctx) };
                assert_eq!(fast_regs[ZERO_REG], 0);
                cpu.r.copy_from_slice(&fast_regs[..8]);
                assert_eq!(
                    status, STATUS_DONE,
                    "hot scalar x87 loop fell back for ctrl={ctrl:#06x} (status={status})"
                );
                assert_eq!(ctx.retired, 50 * 17);
                assert_eq!(cpu.r[ECX], 0);
                assert_eq!(mem.read_u32(data + 16).unwrap(), 5.0f32.to_bits());
                assert_eq!(mem.read_u64(data + 24).unwrap(), 5u64);
            }
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn differential_promoted_integer_ops_and_split_alu8_match_rust_bit_for_bit() {
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;
        let stack = 0x0030_0000u32;

        // 1. All 14 split Alu8RegReg/RegImm + Sbb8RegReg/RegImm across low & high regs (AL..BH).
        const EDGE8: &[u8] = &[0x00, 0x01, 0x0f, 0x10, 0x7f, 0x80, 0xfe, 0xff];
        for &a in EDGE8 {
            for &b in EDGE8 {
                for cf in [false, true] {
                    // Exercise all 8 byte registers AL(0), CL(1), DL(2), BL(3), AH(4), CH(5), DH(6), BH(7)
                    // and every split Alu8 + Sbb8 opcode without disturbing neighboring bytes.
                    let prog = [
                        0x00, 0xe0, // add al, ah
                        0x80, 0xc1, b, // add cl, b
                        0x28, 0xfa, // sub dl, bh
                        0x80, 0xec, b, // sub ah, b
                        0x38, 0xfb, // cmp bl, bh
                        0x80, 0xfd, b, // cmp ch, b
                        0x20, 0xf0, // and al, dh
                        0x80, 0xe7, b, // and bh, b
                        0x08, 0xea, // or dl, ch
                        0x80, 0xc9, b, // or cl, b
                        0x30, 0xdc, // xor ah, bl
                        0x80, 0xf6, b, // xor dh, b
                        0x84, 0xc3, // test bl, al
                        0xf6, 0xc5, b, // test ch, b
                        0x18, 0xe0, // sbb al, ah
                        0x80, 0xdc, b,    // sbb ah, b
                        0xf4, // hlt
                    ];
                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(if cf { CF | OF } else { OF }),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = 0xa1b2_0000 | (u32::from(b) << 8) | u32::from(a);
                    cpu.r[ECX] = 0xc3d4_0000 | (u32::from(a) << 8) | u32::from(b);
                    cpu.r[EDX] = 0xe5f6_0000 | (u32::from(b) << 8) | u32::from(a);
                    cpu.r[EBX] = 0x1234_0000 | (u32::from(a) << 8) | u32::from(b);
                    assert_native_matches_rust(
                        "split_alu8_sbb8",
                        &prog,
                        cpu,
                        &[],
                        &[],
                        100,
                        0..0,
                        DOS_MEMORY,
                    );
                }
            }
        }

        // Dedicated exhaustive edge check for Sbb8 (all 16x16x2 boundary pairs incl. 0-0xff-1, 0x80-0xff-1, 0-0x7f-1).
        for &a in &[0u8, 1, 0x0f, 0x10, 0x7e, 0x7f, 0x80, 0x81, 0xfe, 0xff] {
            for &b in &[0u8, 1, 0x0f, 0x10, 0x7e, 0x7f, 0x80, 0x81, 0xfe, 0xff] {
                for cf in [false, true] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(if cf { CF } else { 0 }),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = 0x55aa_0000 | (u32::from(b) << 8) | u32::from(a);
                    assert_native_matches_rust(
                        "sbb8_rr_edges",
                        &[0x18, 0xe0, 0xf4], // sbb al, ah; hlt
                        cpu,
                        &[],
                        &[],
                        10,
                        0..0,
                        DOS_MEMORY,
                    );
                    assert_native_matches_rust(
                        "sbb8_ri_edges",
                        &[0x80, 0xd8, b, 0xf4], // sbb al, b; hlt
                        cpu,
                        &[],
                        &[],
                        10,
                        0..0,
                        DOS_MEMORY,
                    );
                }
            }
        }

        // 2. Adc32 & Sbb32 across all 5 operand shapes (RegReg, RegImm, RegMem, MemReg, MemImm), CF=0/1.
        const EDGE32: &[u32] = &[
            0,
            1,
            0x0f,
            0x10,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_fffe,
            0xffff_ffff,
        ];
        for &a in EDGE32 {
            for &b in EDGE32 {
                for cf in [false, true] {
                    let mut prog = Vec::new();
                    prog.extend_from_slice(&[0x11, 0xd8]); // adc eax, ebx
                    prog.extend_from_slice(&[0x81, 0xd1]);
                    prog.extend_from_slice(&b.to_le_bytes()); // adc ecx, b
                    prog.extend_from_slice(&[0x13, 0x16]); // adc edx, [esi]
                    prog.extend_from_slice(&[0x11, 0x46, 0x04]); // adc [esi+4], eax
                    prog.extend_from_slice(&[0x81, 0x56, 0x08]);
                    prog.extend_from_slice(&a.to_le_bytes()); // adc dword [esi+8], a
                    prog.extend_from_slice(&[0x19, 0xd8]); // sbb eax, ebx
                    prog.extend_from_slice(&[0x81, 0xd9]);
                    prog.extend_from_slice(&b.to_le_bytes()); // sbb ecx, b
                    prog.extend_from_slice(&[0x1b, 0x16]); // sbb edx, [esi]
                    prog.extend_from_slice(&[0x19, 0x46, 0x0c]); // sbb [esi+12], eax
                    prog.extend_from_slice(&[0x81, 0x5e, 0x10]);
                    prog.extend_from_slice(&b.to_le_bytes()); // sbb dword [esi+16], b
                    prog.push(0xf4);

                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(if cf { CF } else { 0 }),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = a;
                    cpu.r[EBX] = b;
                    cpu.r[ECX] = a;
                    cpu.r[EDX] = b;
                    cpu.r[ESI] = data;
                    let mut mem_init = [0u8; 20];
                    mem_init[0..4].copy_from_slice(&a.to_le_bytes());
                    mem_init[4..8].copy_from_slice(&b.to_le_bytes());
                    mem_init[8..12].copy_from_slice(&b.to_le_bytes());
                    mem_init[12..16].copy_from_slice(&a.to_le_bytes());
                    mem_init[16..20].copy_from_slice(&a.to_le_bytes());
                    assert_native_matches_rust(
                        "adc32_sbb32",
                        &prog,
                        cpu,
                        &[(data, &mem_init)],
                        &[(data, 20)],
                        100,
                        0..0,
                        DOS_MEMORY,
                    );
                }
            }
        }

        // 3. Sahf & Lahf across all 64 flag combinations + lazy aarch64 ALU flags.
        for bits in 0u32..64 {
            let mut eflags = 0x0002u32;
            if bits & 1 != 0 {
                eflags |= CF;
            }
            if bits & 2 != 0 {
                eflags |= PF;
            }
            if bits & 4 != 0 {
                eflags |= AF;
            }
            if bits & 8 != 0 {
                eflags |= ZF;
            }
            if bits & 16 != 0 {
                eflags |= SF;
            }
            if bits & 32 != 0 {
                eflags |= OF;
            }
            let mut cpu = Cpu {
                eip: code_at,
                flags: Flags::from_eflags(eflags),
                ..Cpu::default()
            };
            cpu.r[EAX] = 0xdead_beef;
            // lahf; mov bl, ah; mov ah, cl; sahf; lahf; hlt
            cpu.r[ECX] = (bits << 1) ^ 0xff;
            assert_native_matches_rust(
                "sahf_lahf_all64",
                &[0x9f, 0x88, 0xe3, 0x88, 0xcc, 0x9e, 0x9f, 0xf4],
                cpu,
                &[],
                &[],
                20,
                0..0,
                DOS_MEMORY,
            );
        }
        // Lazy flags -> lahf, and sahf -> jcc/setcc
        for &(a, b) in &[(0x0fu32, 1u32), (0x01, 2), (0x8000_0000, 0x8000_0000)] {
            let mut cpu = Cpu {
                eip: code_at,
                ..Cpu::default()
            };
            cpu.r[EAX] = a;
            cpu.r[EBX] = b;
            assert_native_matches_rust(
                "lazy_add_lahf_sahf",
                &[0x01, 0xd8, 0x9f, 0x80, 0xf4, 0xd5, 0x9e, 0x9f, 0xf4],
                cpu,
                &[],
                &[],
                20,
                0..0,
                DOS_MEMORY,
            );
        }

        // 4. Leave (normal + fault pre-mutation), Cdq, Cwde, Neg32/8 (reg/mem), Not32/8 (reg/mem), Xchg32/8, Extend16Reg.
        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[EBP] = stack - 8;
        cpu.r[ESP] = stack - 32;
        assert_native_matches_rust(
            "leave_normal",
            &[0xc9, 0xf4], // leave; hlt
            cpu,
            &[(stack - 8, &0x1122_3344u32.to_le_bytes())],
            &[],
            10,
            0..0,
            DOS_MEMORY,
        );
        let mut cpu_fault = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu_fault.r[EBP] = DOS_MEMORY - 4;
        cpu_fault.r[ESP] = stack;
        assert_native_matches_rust(
            "leave_fault",
            &[0x40, 0xc9, 0xf4], // inc eax; leave; hlt
            cpu_fault,
            &[],
            &[],
            10,
            0..0,
            DOS_MEMORY,
        );

        for &v in &[
            0u32,
            1,
            0x7fff,
            0x8000,
            0xffff,
            0x1234_7fff,
            0x1234_8000,
            0x7fff_ffff,
            0x8000_0000,
            0xffff_ffff,
        ] {
            let mut cpu = Cpu {
                eip: code_at,
                flags: Flags::from_eflags(CF | PF | AF | ZF | SF | OF),
                ..Cpu::default()
            };
            cpu.r[EAX] = v;
            cpu.r[EBX] = !v;
            cpu.r[ECX] = v.rotate_left(7);
            cpu.r[EDX] = 0x5555_5555;
            cpu.r[ESI] = data;
            let prog = [
                0x98, // cwde
                0x99, // cdq
                0xf7, 0xd0, // not eax
                0xf7, 0x16, // not dword [esi]
                0xf6, 0xd4, // not ah
                0x91, // xchg eax, ecx
                0x86, 0xe3, // xchg bl, ah
                0x86, 0xe4, // xchg ah, ah (self)
                0x66, 0x0f, 0xb6, 0xc5, // movzx ax, ch
                0x66, 0x0f, 0xbe, 0xd7, // movsx dx, bh
                0xf7, 0xd8, // neg eax
                0xf7, 0x1e, // neg dword [esi]
                0xf6, 0xdc, // neg ah
                0xf4, // hlt
            ];
            assert_native_matches_rust(
                "cdq_cwde_neg_not_xchg_ext16",
                &prog,
                cpu,
                &[(data, &v.to_le_bytes())],
                &[(data, 4)],
                100,
                0..0,
                DOS_MEMORY,
            );
        }

        // 5. Inc8/Dec8/Inc16/Dec16 (reg & mem) preserving CF=0 and CF=1 across edges + page-crossing 16-bit write.
        for &v in &[
            0u32, 1, 0x0f, 0x10, 0x7f, 0x80, 0xff, 0x7fff, 0x8000, 0xffff,
        ] {
            for cf in [false, true] {
                for &(name, prog) in &[
                    ("inc8_ah", &[0xfeu8, 0xc4, 0xf4][..]),
                    ("dec8_bl", &[0xfe, 0xcb, 0xf4][..]),
                    ("inc8_mem", &[0xfe, 0x06, 0xf4][..]),
                    ("dec8_mem", &[0xfe, 0x0e, 0xf4][..]),
                    ("inc16_ax", &[0x66, 0x40, 0xf4][..]),
                    ("dec16_dx", &[0x66, 0x4a, 0xf4][..]),
                    ("inc16_mem", &[0x66, 0xff, 0x06, 0xf4][..]),
                    ("dec16_mem", &[0x66, 0xff, 0x0e, 0xf4][..]),
                ] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(if cf { CF } else { 0 }),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = 0xabcd_0000 | ((v & 0xff) << 8) | (v & 0xffff);
                    cpu.r[EBX] = 0x1234_5600 | (v & 0xff);
                    cpu.r[EDX] = 0x9876_0000 | (v & 0xffff);
                    cpu.r[ESI] = data;
                    assert_native_matches_rust(
                        name,
                        prog,
                        cpu,
                        &[(data, &v.to_le_bytes())],
                        &[(data, 4)],
                        10,
                        0..0,
                        DOS_MEMORY,
                    );
                }
            }
        }
        // Page-crossing 16-bit inc mem (at offset 0xfff)
        let mut cpu_cross = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu_cross.r[ESI] = data + 0xfff;
        assert_native_matches_rust(
            "inc16_mem_page_cross",
            &[0x66, 0xff, 0x06, 0xf4],
            cpu_cross,
            &[(data + 0xfff, &0x7fffu16.to_le_bytes())],
            &[(data + 0xfff, 2)],
            10,
            0..0,
            DOS_MEMORY,
        );

        // 6. Shift8Reg (SHL/SHR/SAR/ROL/ROR), Shift16Reg (SHL/SHR/SAR), Rot32Reg (ROL/ROR)
        //    covering count=0 (preserves ALL flags!), 1, 7, 8, 9, 15, 16, 17, 24, 31, 32!
        for &a in &[
            0u32,
            1,
            0x7f,
            0x80,
            0x81,
            0xff,
            0x7fff,
            0x8000,
            0xffff,
            0x8000_0001,
            0x7fff_ffff,
            0xffff_ffff,
        ] {
            for &c in &[0u32, 1, 2, 7, 8, 9, 15, 16, 17, 24, 31, 32] {
                let cb = c as u8;
                for &(name, prog) in &[
                    ("shl8_cl", &[0xd2u8, 0xe4, 0xf4][..]),      // shl ah, cl
                    ("shr8_cl", &[0xd2, 0xec, 0xf4][..]),        // shr ah, cl
                    ("sar8_cl", &[0xd2, 0xfc, 0xf4][..]),        // sar ah, cl
                    ("rol8_cl", &[0xd2, 0xc4, 0xf4][..]),        // rol ah, cl
                    ("ror8_cl", &[0xd2, 0xcc, 0xf4][..]),        // ror ah, cl
                    ("shl16_cl", &[0x66, 0xd3, 0xe0, 0xf4][..]), // shl ax, cl
                    ("shr16_cl", &[0x66, 0xd3, 0xe8, 0xf4][..]), // shr ax, cl
                    ("sar16_cl", &[0x66, 0xd3, 0xf8, 0xf4][..]), // sar ax, cl
                    ("rol32_cl", &[0xd3, 0xc0, 0xf4][..]),       // rol eax, cl
                    ("ror32_cl", &[0xd3, 0xc8, 0xf4][..]),       // ror eax, cl
                    // Prefix with lazy `add edx, ebx` to prove Rol/Ror & count==0 preserve lazy PF/AF!
                    ("lazy_rol32_cl", &[0x01, 0xda, 0xd3, 0xc0, 0xf4][..]),
                    ("lazy_ror8_cl", &[0x01, 0xda, 0xd2, 0xcc, 0xf4][..]),
                ] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(CF | PF | AF | ZF | SF | OF),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = a;
                    cpu.r[ECX] = c;
                    cpu.r[EDX] = 0x0f;
                    cpu.r[EBX] = 1;
                    assert_native_matches_rust(name, prog, cpu, &[], &[], 20, 0..0, DOS_MEMORY);
                }
                for (name, prog) in [
                    ("shl8_imm", vec![0xc0u8, 0xe4, cb, 0xf4]),
                    ("shr8_imm", vec![0xc0, 0xec, cb, 0xf4]),
                    ("sar8_imm", vec![0xc0, 0xfc, cb, 0xf4]),
                    ("rol8_imm", vec![0xc0, 0xc4, cb, 0xf4]),
                    ("ror8_imm", vec![0xc0, 0xcc, cb, 0xf4]),
                    ("shl16_imm", vec![0x66, 0xc1, 0xe0, cb, 0xf4]),
                    ("shr16_imm", vec![0x66, 0xc1, 0xe8, cb, 0xf4]),
                    ("sar16_imm", vec![0x66, 0xc1, 0xf8, cb, 0xf4]),
                    ("rol32_imm", vec![0xc1, 0xc0, cb, 0xf4]),
                    ("ror32_imm", vec![0xc1, 0xc8, cb, 0xf4]),
                ] {
                    let mut cpu = Cpu {
                        eip: code_at,
                        flags: Flags::from_eflags(CF | PF | AF | ZF | SF | OF),
                        ..Cpu::default()
                    };
                    cpu.r[EAX] = a;
                    assert_native_matches_rust(name, &prog, cpu, &[], &[], 20, 0..0, DOS_MEMORY);
                }
            }
        }

        // 7. Mul32 / Imul32 (reg & mem), preserving lazy & materialized SF/ZF/PF/AF.
        for &(a, b) in &[
            (0u32, 0u32),
            (1, 0xffff_ffff),
            (0x1_0000, 0x1_0000),
            (0x7fff_ffff, 2),
            (0x8000_0000, 0xffff_ffff),
            (0xffff_ffff, 0xffff_ffff),
            (0xffff_fffe, 3),
        ] {
            for &(name, prog) in &[
                ("mul32_reg", &[0x01u8, 0xd9, 0xf7, 0xe3, 0xf4][..]), // add ecx, ebx; mul ebx; hlt
                ("imul32_reg", &[0x01, 0xd9, 0xf7, 0xeb, 0xf4][..]),  // add ecx, ebx; imul ebx; hlt
                ("mul32_mem", &[0xf7, 0x26, 0xf4][..]),               // mul dword [esi]; hlt
                ("imul32_mem", &[0xf7, 0x2e, 0xf4][..]),              // imul dword [esi]; hlt
            ] {
                let mut cpu = Cpu {
                    eip: code_at,
                    flags: Flags::from_eflags(PF | AF | ZF | SF),
                    ..Cpu::default()
                };
                cpu.r[EAX] = a;
                cpu.r[EBX] = b;
                cpu.r[ECX] = 0x0f;
                cpu.r[EDX] = 0xdead_beef;
                cpu.r[ESI] = data;
                assert_native_matches_rust(
                    name,
                    prog,
                    cpu,
                    &[(data, &b.to_le_bytes())],
                    &[],
                    20,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }

        // 8. Div32 / Idiv32 (reg & mem) including normal, 64-bit dividends, AND #DE div0 / overflow pre-mutation bail!
        let div_cases: &[(u32, u32, u32)] = &[
            // (edx, eax, divisor)
            (0, 100, 7),
            (0, 0xffff_ffff, 1),
            (1, 0, 2),                     // 2^32 / 2 = 2^31 (fits u32, overflows i32!)
            (0, 0x8000_0000, 1),           // +2^31 / 1 (fits u32, overflows i32!)
            (0xffff_ffff, 0x8000_0000, 1), // i32::MIN / 1 (fits i32!)
            (0xffff_ffff, 0x8000_0000, 0xffff_ffff), // i32::MIN / -1 (#DE signed overflow!)
            (0xffff_ffff, 0xffff_fff9, 2), // -7 / 2 = -3 rem -1
            (0x8000_0000, 0, 0xffff_ffff), // i64::MIN / -1 (#DE!)
            (0x8000_0000, 0, 0x8000_0000), // i64::MIN / i32::MIN (#DE!)
            (5, 0, 5),                     // EDX == divisor (#DE unsigned!)
            (6, 0, 5),                     // EDX > divisor (#DE unsigned!)
            (0, 42, 0),                    // div-by-0 (#DE!)
        ];
        for &(hi, lo, div) in div_cases {
            for &(name, prog) in &[
                ("div32_reg", &[0x41u8, 0xf7, 0xf3, 0xf4][..]), // inc ecx; div ebx; hlt
                ("idiv32_reg", &[0x41, 0xf7, 0xfb, 0xf4][..]),  // inc ecx; idiv ebx; hlt
                ("div32_mem", &[0x41, 0xf7, 0x36, 0xf4][..]),   // inc ecx; div dword [esi]; hlt
                ("idiv32_mem", &[0x41, 0xf7, 0x3e, 0xf4][..]),  // inc ecx; idiv dword [esi]; hlt
            ] {
                let mut cpu = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu.r[EAX] = lo;
                cpu.r[EDX] = hi;
                cpu.r[EBX] = div;
                cpu.r[ESI] = data;
                assert_native_matches_rust(
                    name,
                    prog,
                    cpu,
                    &[(data, &div.to_le_bytes())],
                    &[],
                    20,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }

        // 9. All 16 Cmovcc32Reg & Cmovcc32Mem across all 64 flag combinations + lazy ALU flags + unmapped mem when false!
        for cc in 0u8..16 {
            let prog_reg = [0x0f, 0x40 + cc, 0xc3, 0xf4]; // cmovcc eax, ebx; hlt
            let prog_mem = [0x0f, 0x40 + cc, 0x06, 0xf4]; // cmovcc eax, [esi]; hlt
            for bits in 0u32..64 {
                let mut eflags = 0x0002u32;
                if bits & 1 != 0 {
                    eflags |= CF;
                }
                if bits & 2 != 0 {
                    eflags |= PF;
                }
                if bits & 4 != 0 {
                    eflags |= AF;
                }
                if bits & 8 != 0 {
                    eflags |= ZF;
                }
                if bits & 16 != 0 {
                    eflags |= SF;
                }
                if bits & 32 != 0 {
                    eflags |= OF;
                }
                let mut cpu = Cpu {
                    eip: code_at,
                    flags: Flags::from_eflags(eflags),
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0x1111_1111;
                cpu.r[EBX] = 0x2222_2222;
                // Deliberately test unmapped ESI too: must NOT fault when condition is false, MUST fault identically when true!
                cpu.r[ESI] = DOS_MEMORY - 4;
                assert_native_matches_rust(
                    "cmovcc32_reg",
                    &prog_reg,
                    cpu,
                    &[],
                    &[],
                    10,
                    0..0,
                    DOS_MEMORY,
                );
                assert_native_matches_rust(
                    "cmovcc32_mem_unmapped",
                    &prog_mem,
                    cpu,
                    &[],
                    &[],
                    10,
                    0..0,
                    DOS_MEMORY,
                );
                cpu.r[ESI] = data;
                assert_native_matches_rust(
                    "cmovcc32_mem_mapped",
                    &prog_mem,
                    cpu,
                    &[(data, &0x3333_3333u32.to_le_bytes())],
                    &[],
                    10,
                    0..0,
                    DOS_MEMORY,
                );
            }
            // Lazy flags -> cmovcc (especially JP/JNP!)
            let lazy_prog = [0x01, 0xda, 0x0f, 0x40 + cc, 0xc3, 0xf4]; // add edx, ebx; cmovcc eax, ebx; hlt
            for &(d, b) in &[(0u32, 0u32), (0, 1), (0, 3), (0x7fff_ffff, 1)] {
                let mut cpu = Cpu {
                    eip: code_at,
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0xaaaa_aaaa;
                cpu.r[EBX] = b;
                cpu.r[EDX] = d;
                assert_native_matches_rust(
                    "cmovcc32_lazy",
                    &lazy_prog,
                    cpu,
                    &[],
                    &[],
                    20,
                    0..0,
                    DOS_MEMORY,
                );
            }
        }

        // 10. Prove a loop containing EVERY promoted integer op runs inside run_fast_block with ZERO STATUS_FALLBACK!
        {
            let mut h = Headless::new();
            let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
            let mut loop_prog = Vec::new();
            loop_prog.extend_from_slice(&[0x00, 0xe0]); // add al, ah
            loop_prog.extend_from_slice(&[0x18, 0xe0]); // sbb al, ah
            loop_prog.extend_from_slice(&[0x80, 0xdf, 0x01]); // sbb bh, 1
            loop_prog.extend_from_slice(&[0x86, 0xe0]); // xchg al, ah
            loop_prog.extend_from_slice(&[0x93]); // xchg eax, ebx
            loop_prog.extend_from_slice(&[0x66, 0x0f, 0xb6, 0xc4]); // movzx ax, ah
            loop_prog.extend_from_slice(&[0x98]); // cwde
            loop_prog.extend_from_slice(&[0x99]); // cdq
            loop_prog.extend_from_slice(&[0xf7, 0xd0]); // not eax
            loop_prog.extend_from_slice(&[0xf7, 0xd8]); // neg eax
            loop_prog.extend_from_slice(&[0x11, 0xd8]); // adc eax, ebx
            loop_prog.extend_from_slice(&[0x19, 0x1e]); // sbb ebx, [esi]
            loop_prog.extend_from_slice(&[0x9f]); // lahf
            loop_prog.extend_from_slice(&[0x9e]); // sahf
            loop_prog.extend_from_slice(&[0x66, 0xff, 0x06]); // inc word [esi]
            loop_prog.extend_from_slice(&[0xfe, 0xc4]); // inc ah
            loop_prog.extend_from_slice(&[0xd2, 0xec]); // shr ah, cl
            loop_prog.extend_from_slice(&[0x66, 0xd3, 0xf8]); // sar ax, cl
            loop_prog.extend_from_slice(&[0xc1, 0xc0, 0x03]); // rol eax, 3
            loop_prog.extend_from_slice(&[0xf7, 0xe3]); // mul ebx
            loop_prog.extend_from_slice(&[0x31, 0xd2]); // xor edx, edx
            loop_prog.extend_from_slice(&[0xf7, 0x76, 0x04]); // div dword [esi+4]
            loop_prog.extend_from_slice(&[0x0f, 0x45, 0xc3]); // cmovne eax, ebx
            loop_prog.push(0x49); // dec ecx
            let back = -(loop_prog.len() as i8 + 2);
            loop_prog.extend_from_slice(&[0x75, back as u8]); // jnz loop_prog
            mem.write(code_at, &loop_prog).unwrap();
            mem.write_u32(data, 7).unwrap();
            mem.write_u32(data + 4, 3).unwrap();

            let mut interp = Interp::new(Mode::FLAT);
            let _ = interp.run_at(code_at, &mut mem).unwrap();

            let mut cpu = Cpu {
                eip: code_at,
                ..Cpu::default()
            };
            cpu.r[EAX] = 0x1234_5678;
            cpu.r[EBX] = 0x0000_0011;
            cpu.r[ECX] = 20;
            cpu.r[ESI] = data;

            let (arena_base, arena_floor, arena_len, code_bitmap) = mem.raw_parts_mut();
            let (code_bbox_start, code_bbox_len) = encode_code_bbox(&mem).unwrap();
            let mut fast_regs = [0u32; 9];
            fast_regs[..8].copy_from_slice(&cpu.r);
            let mut ctx = FastCtx {
                regs: fast_regs.as_mut_ptr(),
                arena_base,
                code_bitmap,
                arena_limit4: (arena_len - arena_floor as usize - 4) as u64,
                remaining: 20 * 25,
                retired: 0,
                fast_insns: interp.fast_insns.as_mut_ptr(),
                runs: interp.runs.as_ptr(),
                directory: interp.directory.as_ptr(),
                region_ptrs: interp.region_ptrs.as_ptr(),
                page_ptrs: interp.page_ptrs.as_ptr(),
                fault_pc: core::ptr::null(),
                run_first_pc: core::ptr::null(),
                eip: cpu.eip,
                flags: native::pack_flags(&cpu.flags),
                fs_base: 0,
                arena_floor,
                outside_start: 0,
                outside_end: 0,
                outside_len: 0,
                run_start_eip: 0,
                run_bytes: 0,
                run_len: 0,
                l1_cache: interp.l1.as_mut_ptr(),
                outside_danger_len: 0,
                outside_danger_start: 0,
                _pad: 0,
                fpu: &raw mut cpu.fpu,
                code_bbox_start,
                code_bbox_len,
            };
            let status = unsafe { native::run_fast_block(&mut ctx) };
            assert_eq!(fast_regs[ZERO_REG], 0);
            cpu.r.copy_from_slice(&fast_regs[..8]);
            assert_eq!(
                status, STATUS_DONE,
                "promoted integer ops loop fell back (status={status})"
            );
            assert_eq!(ctx.retired, 20 * 25);
            assert_eq!(cpu.r[ECX], 0);
        }
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn branchless_ea_no_reg_8_sib_fs_and_base_specialized_movs() {
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;
        let fs_base = 0x0024_0000u32;

        // Exercise every EA form:
        //  - [disp32] (base=NO_REG, index=NO_REG) -> MovRegMemBase / MovMemRegBase
        //  - [base] & [base + disp8] (index=NO_REG) -> MovRegMemBase / MovMemRegBase
        //  - [index*scale + disp32] (base=NO_REG, scale=0/1/2/3) -> MovRegMem / MovMemReg
        //  - [base + index*scale + disp] (scale=0/1/2/3)
        //  - FS:[disp32], FS:[base + disp], FS:[base + index*4 + disp]
        //  - FS: LEA (x86 ignores segment override on LEA!)
        let mut prog = Vec::new();
        // mov eax, [data + 0x00] (disp-only -> MovRegMemBase)
        prog.extend_from_slice(&[0x8b, 0x05]);
        prog.extend_from_slice(&data.to_le_bytes());
        // mov [data + 0x40], eax (disp-only -> MovMemRegBase)
        prog.extend_from_slice(&[0x89, 0x05]);
        prog.extend_from_slice(&(data + 0x40).to_le_bytes());
        // mov ebx, [esi] (base-only -> MovRegMemBase)
        prog.extend_from_slice(&[0x8b, 0x1e]);
        // mov [esi + 0x44], ebx (base+disp -> MovMemRegBase)
        prog.extend_from_slice(&[0x89, 0x5e, 0x44]);
        // mov ecx, [edx*1 + data] (index-only, scale=0)
        prog.extend_from_slice(&[0x8b, 0x0c, 0x15]);
        prog.extend_from_slice(&data.to_le_bytes());
        // mov [edx*2 + data + 0x48], ecx (index-only, scale=1)
        prog.extend_from_slice(&[0x89, 0x0c, 0x55]);
        prog.extend_from_slice(&(data + 0x48).to_le_bytes());
        // mov ecx, [edx*4 + data] (index-only, scale=2)
        prog.extend_from_slice(&[0x8b, 0x0c, 0x95]);
        prog.extend_from_slice(&data.to_le_bytes());
        // mov [edx*8 + data + 0x50], ecx (index-only, scale=3)
        prog.extend_from_slice(&[0x89, 0x0c, 0xd5]);
        prog.extend_from_slice(&(data + 0x50).to_le_bytes());
        // mov eax, [esi + edx*4 + 4] (base + index*4 + disp8)
        prog.extend_from_slice(&[0x8b, 0x44, 0x96, 0x04]);
        // mov [esi + edx*8 + 0x54], eax (base + index*8 + disp8)
        prog.extend_from_slice(&[0x89, 0x44, 0xd6, 0x54]);
        // FS:[0x08] -> MovRegMemBase with FLAG_FS
        prog.extend_from_slice(&[0x64, 0x8b, 0x1d, 0x08, 0x00, 0x00, 0x00]);
        // mov fs:[0x10], ebx -> MovMemRegBase with FLAG_FS
        prog.extend_from_slice(&[0x64, 0x89, 0x1d, 0x10, 0x00, 0x00, 0x00]);
        // mov eax, fs:[ebp + edx*4 + 4] -> MovRegMem with FLAG_FS + SIB
        prog.extend_from_slice(&[0x64, 0x8b, 0x44, 0x95, 0x04]);
        // mov fs:[ebp + edx*4 + 0x14], eax -> MovMemReg with FLAG_FS + SIB
        prog.extend_from_slice(&[0x64, 0x89, 0x44, 0x95, 0x14]);
        // fs: lea edi, [esi + edx*8 + 0x10] (must ignore FS:)
        prog.extend_from_slice(&[0x64, 0x8d, 0x7c, 0xd6, 0x10]);
        // lea ebx, [edx*4 + 0x20] (base=NO_REG=8)
        prog.extend_from_slice(&[0x8d, 0x1c, 0x95, 0x20, 0x00, 0x00, 0x00]);
        let body_insns = 16u64;
        prog.push(0xf4); // hlt

        let mut data_init = [0u8; 0x80];
        for i in 0..16u32 {
            let v = 0x1000_0001u32.wrapping_mul(i + 1);
            data_init[(i * 4) as usize..(i * 4 + 4) as usize].copy_from_slice(&v.to_le_bytes());
        }
        let mut fs_init = [0u8; 0x40];
        for i in 0..8u32 {
            let v = 0xf000_0010u32.wrapping_add(i * 0x1111);
            fs_init[(i * 4) as usize..(i * 4 + 4) as usize].copy_from_slice(&v.to_le_bytes());
        }

        let mut cpu = Cpu {
            eip: code_at,
            ..Cpu::default()
        };
        cpu.r[ESI] = data;
        cpu.r[EDX] = 2;
        cpu.r[EBP] = 4;
        cpu.seg_base[FS] = fs_base;

        assert_native_matches_rust(
            "branchless_ea_all_modes",
            &prog,
            cpu,
            &[(data, &data_init), (fs_base, &fs_init)],
            &[(data, 0x80), (fs_base, 0x40)],
            100,
            0..0,
            DOS_MEMORY,
        );

        // Verify compile_fast specialized base-only forms AND run_fast_block runs all 16 ops with zero fallback + r[8]==0.
        let mut h = Headless::new();
        let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
        mem.write(code_at, &prog).unwrap();
        mem.write(data, &data_init).unwrap();
        mem.write(fs_base, &fs_init).unwrap();
        let mut interp = Interp::new(Mode::FLAT);
        let run = interp.run_at(code_at, &mut mem).unwrap();
        let ops: Vec<FastOp> = interp.fast_insns
            [run.first as usize..run.first as usize + usize::from(run.len)]
            .iter()
            .map(|i| i.op)
            .collect();
        assert_eq!(ops[0], FastOp::MovRegMemBase);
        assert_eq!(ops[1], FastOp::MovMemRegBase);
        assert_eq!(ops[2], FastOp::MovRegMemBase);
        assert_eq!(ops[3], FastOp::MovMemRegBase);
        assert_eq!(ops[4], FastOp::MovRegMem);
        assert_eq!(ops[5], FastOp::MovMemReg);
        // FS:-overridden loads/stores stay on MovRegMem/MovMemReg so Mov*Base skips FS: check!
        assert_eq!(ops[10], FastOp::MovRegMem);
        assert_eq!(ops[11], FastOp::MovMemReg);
        assert_eq!(ops[12], FastOp::MovRegMem);
        assert_eq!(ops[13], FastOp::MovMemReg);

        let mut no_hlt = prog[..prog.len() - 1].to_vec();
        no_hlt.extend_from_slice(&[0xeb, 0xfe]); // jmp $
        let code2 = 0x0018_2000u32;
        mem.write(code2, &no_hlt).unwrap();
        let _ = interp.run_at(code2, &mut mem).unwrap();
        let (arena_base, arena_floor, arena_len, code_bitmap) = mem.raw_parts_mut();
        let (code_bbox_start, code_bbox_len) = encode_code_bbox(&mem).unwrap();
        let mut fast_regs = [0u32; 9];
        fast_regs[..8].copy_from_slice(&cpu.r);
        let mut ctx = FastCtx {
            regs: fast_regs.as_mut_ptr(),
            arena_base,
            code_bitmap,
            arena_limit4: (arena_len - arena_floor as usize - 4) as u64,
            remaining: body_insns + 1,
            retired: 0,
            fast_insns: interp.fast_insns.as_mut_ptr(),
            runs: interp.runs.as_ptr(),
            directory: interp.directory.as_ptr(),
            region_ptrs: interp.region_ptrs.as_ptr(),
            page_ptrs: interp.page_ptrs.as_ptr(),
            fault_pc: core::ptr::null(),
            run_first_pc: core::ptr::null(),
            eip: code2,
            flags: native::pack_flags(&cpu.flags),
            fs_base,
            arena_floor,
            outside_start: 0,
            outside_end: 0,
            outside_len: 0,
            run_start_eip: 0,
            run_bytes: 0,
            run_len: 0,
            l1_cache: interp.l1.as_mut_ptr(),
            outside_danger_len: 0,
            outside_danger_start: 0,
            _pad: 0,
            fpu: &raw mut cpu.fpu,
            code_bbox_start,
            code_bbox_len,
        };
        let status = unsafe { native::run_fast_block(&mut ctx) };
        assert_eq!(status, STATUS_DONE, "EA modes fell back (status={status})");
        assert_eq!(ctx.retired, body_insns + 1);
        assert_eq!(fast_regs[ZERO_REG], 0, "NO_REG=8 slot must remain zero");
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn code_bbox_fast_path_and_straddling_smc_boundaries() {
        // Two code pages with a non-code hole between them:
        //   page A: [0x0018_0000, 0x0018_1000)
        //   hole:   [0x0018_1000, 0x0019_0000) (non-code inside bbox!)
        //   page B: [0x0019_0000, 0x0019_1000)
        // Bbox: [0x0018_0000, 0x0019_1000); padded start = 0x0017_fff9.
        let code_a = 0x0018_0000u32;
        let code_b = 0x0019_0000u32;

        // 1. Direct run_fast_block checks: stores outside bbox (even page-crossing!)
        //    and stores in the non-code hole stay in-block (STATUS_DONE), whereas
        //    every store overlapping code_a or code_b (incl. straddling code_min - 1
        //    and code_min - 7!) MUST return STATUS_FALLBACK!
        // Store instructions to test:
        //   - 1B: `mov byte [esi], al; jmp $`
        //   - 2B: `mov word [esi], ax; jmp $`
        //   - 4B: `mov dword [esi], eax; jmp $` (MovMemRegBase)
        //   - 4B SIB: `mov dword [esi + edx*1], eax; jmp $` (MovMemReg)
        //   - 8B: `fldz; fstp qword [esi]; jmp $` (X87Mem FstpM64)
        let store_cases: &[(&str, &[u8], u32, u64)] = &[
            ("store1", &[0x88, 0x06, 0xeb, 0xfe], 1, 2),
            ("store2", &[0x66, 0x89, 0x06, 0xeb, 0xfe], 2, 2),
            ("store4_base", &[0x89, 0x06, 0xeb, 0xfe], 4, 2),
            ("store4_sib", &[0x89, 0x04, 0x16, 0xeb, 0xfe], 4, 2),
            ("store8_f64", &[0xd9, 0xee, 0xdd, 0x1e, 0xeb, 0xfe], 8, 3),
        ];
        let test_addrs: &[u32] = &[
            code_a - 0x1001, // page-crossing outside bbox below
            code_a - 8,      // 8B ends at code_a (strictly outside!)
            code_a - 7,      // 8B straddles into code_a; 1/2/4B strictly outside!
            code_a - 4,      // 4B ends at code_a (outside); 8B straddles!
            code_a - 3,      // 4B/8B straddle into code_a; 1/2B outside!
            code_a - 1,      // 2B/4B/8B straddle into code_a; 1B outside!
            code_a + 0x100,  // directly inside code page A!
            code_a + 0x1000, // inside non-code hole within bbox
            code_a + 0x1fff, // page-crossing inside non-code hole (4B/8B/2B cross page -> cold check!)
            code_b - 1,      // 2B/4B/8B straddle into code page B!
            code_b + 0x100,  // directly inside code page B!
            code_b + 0x0fff, // 1B/2B/4B/8B all touch code page B's last byte!
            code_b + 0x1000, // strictly above code_max!
            code_b + 0x1fff, // page-crossing strictly above code_max!
        ];

        for &(name, prog, width, insns) in store_cases {
            for &addr in test_addrs {
                let mut h = Headless::new();
                let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
                mem.write(code_a, prog).unwrap();
                mem.write(code_b, &[0x90, 0xf4]).unwrap(); // nop; hlt
                let mut interp = Interp::new(Mode::FLAT);
                let _ = interp.run_at(code_a, &mut mem).unwrap();
                let _ = interp.run_at(code_b, &mut mem).unwrap();
                assert_eq!(mem.code_bbox(), (code_a, code_b + 0x1000));

                let mut cpu = Cpu {
                    eip: code_a,
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0x1122_3344;
                cpu.r[ESI] = addr;
                cpu.r[EDX] = 0;

                let (arena_base, arena_floor, arena_len, code_bitmap) = mem.raw_parts_mut();
                let (code_bbox_start, code_bbox_len) = encode_code_bbox(&mem).unwrap();
                let mut fast_regs = [0u32; 9];
                fast_regs[..8].copy_from_slice(&cpu.r);
                let mut ctx = FastCtx {
                    regs: fast_regs.as_mut_ptr(),
                    arena_base,
                    code_bitmap,
                    arena_limit4: (arena_len - arena_floor as usize - 4) as u64,
                    remaining: insns,
                    retired: 0,
                    fast_insns: interp.fast_insns.as_mut_ptr(),
                    runs: interp.runs.as_ptr(),
                    directory: interp.directory.as_ptr(),
                    region_ptrs: interp.region_ptrs.as_ptr(),
                    page_ptrs: interp.page_ptrs.as_ptr(),
                    fault_pc: core::ptr::null(),
                    run_first_pc: core::ptr::null(),
                    eip: code_a,
                    flags: native::pack_flags(&cpu.flags),
                    fs_base: 0,
                    arena_floor,
                    outside_start: 0,
                    outside_end: 0,
                    outside_len: 0,
                    run_start_eip: 0,
                    run_bytes: 0,
                    run_len: 0,
                    l1_cache: interp.l1.as_mut_ptr(),
                    outside_danger_len: 0,
                    outside_danger_start: 0,
                    _pad: 0,
                    fpu: &raw mut cpu.fpu,
                    code_bbox_start,
                    code_bbox_len,
                };
                let status = unsafe { native::run_fast_block(&mut ctx) };
                let end = addr + width;
                let touches_code_a = addr < code_a + 0x1000 && end > code_a;
                let touches_code_b = addr < code_b + 0x1000 && end > code_b;
                let crosses_page_in_bbox = (addr.wrapping_sub(code_bbox_start) < code_bbox_len)
                    && ((addr & 0xfff) + width > 0x1000);
                if touches_code_a || touches_code_b {
                    assert_eq!(
                        status, STATUS_FALLBACK,
                        "{name} at {addr:#x} (+{width}) overlaps code and MUST bail to STATUS_FALLBACK!"
                    );
                } else if !crosses_page_in_bbox {
                    assert_eq!(
                        status, STATUS_DONE,
                        "{name} at {addr:#x} (+{width}) does not touch code and should stay in run_fast_block"
                    );
                }
            }
        }

        // 2. Differential parity against rust::run when a 4-byte store straddling
        //    `code_min - 3` rewrites the first byte of a cached function!
        let caller = 0x0019_0000u32;
        let target = 0x0018_0000u32; // target IS code_min!
                                     // Target initially: `mov eax, 1; ret` (b8 01 00 00 00 c3)
                                     // Caller: `call target; mov dword [target - 3], 0x90000000` (changes b8->90? Let's write 0x05_00_00_00? Wait: byte at target+0 is MSB of u32 at target-3!)
                                     // At target - 3: bytes [target-3, target-2, target-1, target+0].
                                     // Writing 0x40_00_00_00 to [target - 3] overwrites target[0] (0x90 nop -> 0x40 inc eax)!
        let target_code: &[u8] = &[0x90, 0xc3]; // nop; ret -> becomes `inc eax; ret`!
        let mut caller_code = Vec::new();
        caller_code.push(0xe8);
        caller_code.extend_from_slice(&target.wrapping_sub(caller + 5).to_le_bytes());
        // mov dword [target - 3], 0x40000000
        caller_code.extend_from_slice(&[0xc7, 0x05]);
        caller_code.extend_from_slice(&(target - 3).to_le_bytes());
        caller_code.extend_from_slice(&0x4000_0000u32.to_le_bytes());
        caller_code.push(0xe8);
        caller_code.extend_from_slice(&target.wrapping_sub(caller + 20).to_le_bytes());
        caller_code.push(0xf4);

        let mut cpu = Cpu {
            eip: caller,
            ..Cpu::default()
        };
        cpu.r[ESP] = 0x0030_0000;
        assert_native_matches_rust(
            "smc_straddling_code_min_minus_3",
            &caller_code,
            cpu,
            &[(target, target_code)],
            &[(target - 4, 8)],
            200,
            0..0,
            DOS_MEMORY,
        );
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn dead_flag_nf_promotion_is_conservative_and_exact() {
        let code_at = 0x0018_0000u32;
        let data = 0x0020_0000u32;

        // 1. Verify ALL 13 ALU/shift ops promote to *Nf before an unconditional overwriter,
        //    even across pure non-flag ops (`mov`, `lea`, `not`, `xchg`, `cdq`, `nop`).
        let mut nf_prog = Vec::new();
        nf_prog.extend_from_slice(&[0x01, 0xd8]); // add eax, ebx       -> AddRegRegNf
        nf_prog.extend_from_slice(&[0x83, 0xc0, 0x05]); // add eax, 5   -> AddRegImmNf
        nf_prog.extend_from_slice(&[0x29, 0xd8]); // sub eax, ebx       -> SubRegRegNf
        nf_prog.extend_from_slice(&[0x83, 0xe8, 0x03]); // sub eax, 3   -> SubRegImmNf
        nf_prog.extend_from_slice(&[0x31, 0xd8]); // xor eax, ebx       -> XorRegRegNf
        nf_prog.extend_from_slice(&[0x83, 0xf0, 0x07]); // xor eax, 7   -> XorRegImmNf
        nf_prog.extend_from_slice(&[0x21, 0xd8]); // and eax, ebx       -> AndRegRegNf
        nf_prog.extend_from_slice(&[0x83, 0xe0, 0x7f]); // and eax, 0x7f-> AndRegImmNf
        nf_prog.extend_from_slice(&[0x09, 0xd8]); // or eax, ebx        -> OrRegRegNf
        nf_prog.extend_from_slice(&[0x83, 0xc8, 0x11]); // or eax, 0x11 -> OrRegImmNf
        nf_prog.extend_from_slice(&[0xc1, 0xe0, 0x02]); // shl eax, 2   -> ShlRegImmNf
        nf_prog.extend_from_slice(&[0xc1, 0xe8, 0x01]); // shr eax, 1   -> ShrRegImmNf
        nf_prog.extend_from_slice(&[0xc1, 0xf8, 0x03]); // sar eax, 3   -> SarRegImmNf
                                                        // Pure non-flag ops between SarRegImm and the final overwriter:
        nf_prog.extend_from_slice(&[0x89, 0xc2]); // mov edx, eax
        nf_prog.extend_from_slice(&[0x8d, 0x54, 0x9a, 0x04]); // lea edx, [edx + ebx*4 + 4]
        nf_prog.extend_from_slice(&[0xf7, 0xd2]); // not edx
        nf_prog.extend_from_slice(&[0x99]); // cdq
        nf_prog.extend_from_slice(&[0x90]); // nop
                                            // Unconditional all-6-flag overwriter:
        nf_prog.extend_from_slice(&[0x39, 0xda]); // cmp edx, ebx (NOT promoted!)
        nf_prog.push(0xf4); // hlt

        let mut h = Headless::new();
        let mut mem = Arena::new(&mut h, 1 << 22).unwrap().with_floor(DOS_MEMORY);
        mem.write(code_at, &nf_prog).unwrap();
        let mut interp = Interp::new(Mode::FLAT);
        let run = interp.run_at(code_at, &mut mem).unwrap();
        let ops: Vec<FastOp> = interp.fast_insns
            [run.first as usize..run.first as usize + usize::from(run.len)]
            .iter()
            .map(|i| i.op)
            .collect();
        assert_eq!(
            &ops[..13],
            &[
                FastOp::AddRegRegNf,
                FastOp::AddRegImmNf,
                FastOp::SubRegRegNf,
                FastOp::SubRegImmNf,
                FastOp::XorRegRegNf,
                FastOp::XorRegImmNf,
                FastOp::AndRegRegNf,
                FastOp::AndRegImmNf,
                FastOp::OrRegRegNf,
                FastOp::OrRegImmNf,
                FastOp::ShlRegImmNf,
                FastOp::ShrRegImmNf,
                FastOp::SarRegImmNf,
            ]
        );
        assert_eq!(ops[18], FastOp::CmpRegReg);

        // 2. Verify barrier ops BLOCK promotion (never promote across memory/faults,
        //    flag readers, partial-flag writers, shift-by-CL, X87, or end-of-run).
        let barrier_progs: &[(&str, &[u8])] = &[
            ("mem_load", &[0x01, 0xd8, 0x8b, 0x0e, 0x39, 0xd8, 0xf4]), // add eax,ebx; mov ecx,[esi]; cmp eax,ebx
            ("mem_store", &[0x01, 0xd8, 0x89, 0x06, 0x39, 0xd8, 0xf4]), // add eax,ebx; mov [esi],eax; cmp eax,ebx
            ("adc", &[0x01, 0xd8, 0x11, 0xd8, 0x39, 0xd8, 0xf4]),       // add; adc; cmp
            ("inc_partial", &[0x01, 0xd8, 0x41, 0x39, 0xd8, 0xf4]),     // add; inc ecx; cmp
            ("setcc", &[0x01, 0xd8, 0x0f, 0x94, 0xc1, 0x39, 0xd8, 0xf4]), // add; sete cl; cmp
            ("cmovcc", &[0x01, 0xd8, 0x0f, 0x44, 0xc1, 0x39, 0xd8, 0xf4]), // add; cmove eax,ecx; cmp
            ("lahf", &[0x01, 0xd8, 0x9f, 0x39, 0xd8, 0xf4]),               // add; lahf; cmp
            ("shift_cl", &[0x01, 0xd8, 0xd3, 0xe0, 0x39, 0xd8, 0xf4]),     // add; shl eax,cl; cmp
            ("x87", &[0x01, 0xd8, 0xd9, 0xee, 0x39, 0xd8, 0xf4]),          // add; fldz; cmp
            ("run_end", &[0x01, 0xd8, 0xf4]),                              // add; hlt
        ];
        for (i, &(label, bytes)) in barrier_progs.iter().enumerate() {
            let at = code_at + 0x1000 + (i as u32) * 0x40;
            mem.write(at, bytes).unwrap();
            let r = interp.run_at(at, &mut mem).unwrap();
            assert_eq!(
                interp.fast_insns[r.first as usize].op,
                FastOp::AddRegReg,
                "barrier '{label}' MUST prevent *Nf promotion"
            );
        }

        // 3. Differential exactness across EVERY budget 0..=20 (mid-run stops MUST
        //    still observe exact intermediate flags via rust::finish_run_from!).
        let mut init = Cpu {
            eip: code_at,
            flags: Flags::from_eflags(CF | PF | AF | ZF | SF | OF | DF),
            ..Cpu::default()
        };
        init.r[EAX] = 0x8000_0001;
        init.r[EBX] = 0x7fff_ffff;
        for budget in 0..=20 {
            assert_native_matches_rust(
                "nf_mid_run_budget",
                &nf_prog,
                init,
                &[],
                &[],
                budget,
                0..0,
                DOS_MEMORY,
            );
        }

        // 4. Tight loop containing ALL 13 *Nf ops + MovRegMemBase + MovMemRegBase
        //    must execute inside run_fast_block with ZERO STATUS_FALLBACK!
        let mut loop_prog = Vec::new();
        loop_prog.extend_from_slice(&[0x8b, 0x06]); // mov eax, [esi] (MovRegMemBase)
        loop_prog.extend_from_slice(&nf_prog[..nf_prog.len() - 1]); // 13 *Nf + 5 pure + cmp
        loop_prog.extend_from_slice(&[0x89, 0x06]); // mov [esi], eax (MovMemRegBase)
        loop_prog.push(0x49); // dec ecx
        let back = -(loop_prog.len() as i8 + 2);
        loop_prog.extend_from_slice(&[0x75, back as u8]); // jnz loop_prog
        let loop_at = code_at + 0x2000;
        mem.write(loop_at, &loop_prog).unwrap();
        mem.write_u32(data, 0x1234_5678).unwrap();
        let r_loop = interp.run_at(loop_at, &mut mem).unwrap();
        let insns_per_iter = u64::from(r_loop.len);
        let mut cpu = Cpu {
            eip: loop_at,
            ..Cpu::default()
        };
        cpu.r[EBX] = 0x0102_0304;
        cpu.r[ECX] = 16;
        cpu.r[ESI] = data;
        let (arena_base, arena_floor, arena_len, code_bitmap) = mem.raw_parts_mut();
        let (code_bbox_start, code_bbox_len) = encode_code_bbox(&mem).unwrap();
        let mut fast_regs = [0u32; 9];
        fast_regs[..8].copy_from_slice(&cpu.r);
        let mut ctx = FastCtx {
            regs: fast_regs.as_mut_ptr(),
            arena_base,
            code_bitmap,
            arena_limit4: (arena_len - arena_floor as usize - 4) as u64,
            remaining: 16 * insns_per_iter,
            retired: 0,
            fast_insns: interp.fast_insns.as_mut_ptr(),
            runs: interp.runs.as_ptr(),
            directory: interp.directory.as_ptr(),
            region_ptrs: interp.region_ptrs.as_ptr(),
            page_ptrs: interp.page_ptrs.as_ptr(),
            fault_pc: core::ptr::null(),
            run_first_pc: core::ptr::null(),
            eip: loop_at,
            flags: native::pack_flags(&cpu.flags),
            fs_base: 0,
            arena_floor,
            outside_start: 0,
            outside_end: 0,
            outside_len: 0,
            run_start_eip: 0,
            run_bytes: 0,
            run_len: 0,
            l1_cache: interp.l1.as_mut_ptr(),
            outside_danger_len: 0,
            outside_danger_start: 0,
            _pad: 0,
            fpu: &raw mut cpu.fpu,
            code_bbox_start,
            code_bbox_len,
        };
        let status = unsafe { native::run_fast_block(&mut ctx) };
        assert_eq!(status, STATUS_DONE, "*Nf loop fell back (status={status})");
        assert_eq!(ctx.retired, 16 * insns_per_iter);
        assert_eq!(fast_regs[ZERO_REG], 0);
        // Also check the entire loop matches rust::run bit-for-bit!
        loop_prog.push(0xf4);
        assert_native_matches_rust(
            "nf_loop_parity",
            &loop_prog,
            cpu,
            &[(data, &0x1234_5678u32.to_le_bytes())],
            &[(data, 4)],
            1000,
            0..0,
            DOS_MEMORY,
        );
    }

    #[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
    #[test]
    fn flag_capture_matches_all_64_states_and_x86_64_has_zero_pushfq() {
        // Verify x86_64.rs has zero `pushfq` or `popfq` instructions in assembly strings.
        let x86_src = include_str!("x86_64.rs");
        for (idx, line) in x86_src.lines().enumerate() {
            let code_part = line.split("//").next().unwrap_or("");
            assert!(
                !code_part.contains("pushfq") && !code_part.contains("popfq"),
                "x86_64.rs line {} contains forbidden pushfq/popfq: {}",
                idx + 1,
                code_part.trim()
            );
        }

        // Sweep ALL 64 arithmetic flag states through flag-preserving, flag-reading,
        // partial-flag-updating, and full-flag-overwriting instructions.
        let code_at = 0x0018_0000u32;
        let progs: &[(&str, &[u8])] = &[
            ("nop_preserve", &[0x90, 0xf4]),
            ("inc_preserve_cf", &[0x40, 0xf4]),
            ("dec_preserve_cf", &[0x48, 0xf4]),
            ("adc_reads_cf", &[0x11, 0xd8, 0xf4]),
            ("sbb_reads_cf", &[0x19, 0xd8, 0xf4]),
            ("sbb8_reads_cf", &[0x18, 0xd8, 0xf4]),
            ("lahf_reads_5", &[0x9f, 0xf4]),
            ("sahf_writes_5_preserves_of", &[0x9e, 0xf4]),
            ("rol_count_0", &[0xd3, 0xc0, 0xf4]),
            ("rol_count_1", &[0xc1, 0xc0, 0x01, 0xf4]),
            ("mul_preserves_szpa", &[0xf7, 0xe3, 0xf4]),
            ("and_clears_cf_of", &[0x21, 0xd8, 0xf4]),
            ("or_clears_cf_of", &[0x09, 0xd8, 0xf4]),
            ("xor_clears_cf_of", &[0x31, 0xd8, 0xf4]),
            ("test_clears_cf_of", &[0x85, 0xd8, 0xf4]),
            ("neg_sets_all_6", &[0xf7, 0xd8, 0xf4]),
            (
                "fcomi_preserves_of_af_sf",
                &[0xd9, 0xe8, 0xd9, 0xee, 0xdb, 0xf1, 0xf4],
            ),
        ];

        for bits in 0u32..64 {
            let mut eflags = 0x0002u32 | DF;
            if bits & 1 != 0 {
                eflags |= CF;
            }
            if bits & 2 != 0 {
                eflags |= PF;
            }
            if bits & 4 != 0 {
                eflags |= AF;
            }
            if bits & 8 != 0 {
                eflags |= ZF;
            }
            if bits & 16 != 0 {
                eflags |= SF;
            }
            if bits & 32 != 0 {
                eflags |= OF;
            }
            for &(label, prog) in progs {
                let mut cpu = Cpu {
                    eip: code_at,
                    flags: Flags::from_eflags(eflags),
                    ..Cpu::default()
                };
                cpu.r[EAX] = 0x7fff_ff00 | ((bits << 2) & 0xff);
                cpu.r[EBX] = 0x8000_000f;
                cpu.r[ECX] = 0; // count == 0 for rol_count_0
                assert_native_matches_rust(label, prog, cpu, &[], &[], 20, 0..0, DOS_MEMORY);
            }
        }
    }
}
