//! Canonical pure-Rust flat32 / real-mode interpreter loop (ADR-0009).
//!
//! Always compiled and kept forever as the bit-exact oracle for differential
//! tests, `LEGO_LEGACY_ARCH=rust`, `Mode::REAL` conformance, active clock-spin
//! watches, and non-`aarch64`/non-`x86_64` hosts.

use super::{clamp_to_outside, sum_lengths, Interp, Run};
use crate::cpu::{exec, Cpu, Stop, CS};
use crate::guest::Arena;

/// Execute a single already-decoded run in pure Rust starting at instruction
/// `from_idx`, updating `cpu.eip`, `cpu.tsc`, and `retired`.
///
/// Used by the native `naked_asm!` seam to finish a run after a mid-run
/// fallback (`Slow`, `X87`, fault, code-page hit) without duplicating cached runs.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_run_from(
    interp: &Interp,
    run: Run,
    from_idx: usize,
    steps: usize,
    start_eip: u32,
    cpu: &mut Cpu,
    mem: &mut Arena,
    retired: &mut u64,
) -> Result<(), Stop> {
    let start = run.first as usize;
    let len = usize::from(run.len);
    let run_insns = &interp.fast_insns[start..start + len];
    let has_terminal = run.ends_with_transfer != 0 && steps == len;
    let straight_len = if has_terminal { steps - 1 } else { steps };
    let base_tsc = cpu.tsc - from_idx as u64;

    if from_idx < straight_len {
        for (offset, insn) in run_insns[from_idx..straight_len].iter().enumerate() {
            let i = from_idx + offset;
            if let Err(stop) = exec::execute_straight(
                insn,
                cpu,
                mem,
                &interp.slow_insns,
                interp.mode,
                base_tsc + i as u64,
            ) {
                cpu.eip = start_eip.wrapping_add(sum_lengths(&run_insns[..i]));
                cpu.tsc = base_tsc + i as u64;
                *retired += offset as u64;
                return Err(stop);
            }
        }
        *retired += (straight_len - from_idx) as u64;
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
            &interp.slow_insns,
            interp.mode,
            base_tsc + done,
        ) {
            Ok(()) => {
                *retired += 1;
                cpu.tsc = base_tsc + done + 1;
                Ok(())
            }
            Err(stop @ Stop::Interrupt(_)) => {
                cpu.tsc = base_tsc + done;
                *retired += 1;
                Err(stop)
            }
            Err(stop @ Stop::Halted) => {
                cpu.tsc = base_tsc + done;
                Err(stop)
            }
            Err(stop) => {
                cpu.eip = at;
                cpu.tsc = base_tsc + done;
                Err(stop)
            }
        }
    } else {
        let advance = if steps == len {
            u32::from(run.bytes)
        } else {
            sum_lengths(&run_insns[..straight_len])
        };
        cpu.eip = start_eip.wrapping_add(advance);
        cpu.tsc = base_tsc + done;
        Ok(())
    }
}

/// Execute one run at `cpu.eip` in pure Rust, clamping to `count` and `outside`.
#[inline(always)]
pub(crate) fn run_one_block(
    interp: &mut Interp,
    cpu: &mut Cpu,
    mem: &mut Arena,
    count: u64,
    outside: &std::ops::Range<u32>,
    retired: &mut u64,
) -> Result<bool, Stop> {
    if *retired >= count || outside.contains(&cpu.eip) {
        return Ok(false);
    }
    let lin = if interp.mode.real {
        cpu.seg_base[CS].wrapping_add(cpu.eip)
    } else {
        cpu.eip
    };
    let run = interp.run_at(lin, mem)?;
    let start = run.first as usize;
    let len = usize::from(run.len);
    let mut steps = (len as u64).min(count - *retired) as usize;
    let start_eip = cpu.eip;

    if !outside.is_empty()
        && start_eip
            .checked_add(u32::from(run.bytes))
            .is_none_or(|end| start_eip < outside.end && end > outside.start)
    {
        let run_insns = &interp.fast_insns[start..start + len];
        steps = clamp_to_outside(&run_insns[..steps], start_eip, outside);
        if steps == 0 {
            return Ok(false);
        }
    }

    finish_run_from(interp, run, 0, steps, start_eip, cpu, mem, retired)?;
    Ok(true)
}

/// Canonical pure-Rust execution loop (tight single-function oracle).
pub(crate) fn run(
    interp: &mut Interp,
    cpu: &mut Cpu,
    mem: &mut Arena,
    count: u64,
    outside: std::ops::Range<u32>,
) -> (u64, Result<(), Stop>) {
    let mut retired = 0u64;
    while retired < count && !outside.contains(&cpu.eip) {
        let lin = if interp.mode.real {
            cpu.seg_base[CS].wrapping_add(cpu.eip)
        } else {
            cpu.eip
        };
        let run = match interp.run_at(lin, mem) {
            Ok(run) => run,
            Err(stop) => return (retired, Err(stop)),
        };
        let start = run.first as usize;
        let len = usize::from(run.len);
        let run_insns = &interp.fast_insns[start..start + len];
        let mut steps = (len as u64).min(count - retired) as usize;
        let start_eip = cpu.eip;

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

        let has_terminal = run.ends_with_transfer != 0 && steps == len;
        let straight_len = if has_terminal { steps - 1 } else { steps };
        let base_tsc = cpu.tsc;

        for (i, insn) in run_insns[..straight_len].iter().enumerate() {
            if let Err(stop) = exec::execute_straight(
                insn,
                cpu,
                mem,
                &interp.slow_insns,
                interp.mode,
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
                &interp.slow_insns,
                interp.mode,
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
