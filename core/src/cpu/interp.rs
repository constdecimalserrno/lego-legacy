//! The interpreter.
//!
//! It decodes the opcodes the roster has been measured to need, and stops on
//! any other byte rather than inventing behaviour for it. The set grows one
//! measurement at a time.

use super::{CpuEngine, Regs, Stop};
use crate::guest::{Arena, GuestMem};

/// A straightforward decode-and-execute interpreter.
#[derive(Debug, Default)]
pub struct Interp;

impl Interp {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// The longest instruction x86 encodes. One bounds check over this much
    /// memory licenses every fetch the instruction then makes.
    const MAX_INSN: usize = 15;

    fn fetch_u8(regs: &mut Regs, mem: &Arena, fast: bool) -> Result<u8, Stop> {
        let b = if fast {
            // Safety: `fast` says the whole instruction window is in bounds.
            unsafe { mem.read_u8_unchecked(regs.eip) }
        } else {
            mem.slice(regs.eip, 1).map_err(Stop::Fault)?[0]
        };
        regs.eip = regs.eip.wrapping_add(1);
        Ok(b)
    }

    fn fetch_u32(regs: &mut Regs, mem: &Arena, fast: bool) -> Result<u32, Stop> {
        let v = if fast {
            // Safety: `fast` says the whole instruction window is in bounds.
            unsafe { mem.read_u32_unchecked(regs.eip) }
        } else {
            let s = mem.slice(regs.eip, 4).map_err(Stop::Fault)?;
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        };
        regs.eip = regs.eip.wrapping_add(4);
        Ok(v)
    }

    /// Set the flags an increment or decrement leaves behind. Neither touches
    /// the carry flag: that is the whole reason both exist beside `add` and
    /// `sub`.
    fn inc_dec_flags(regs: &mut Regs, result: u32, before: u32, up: bool) {
        regs.flags.zf = result == 0;
        regs.flags.sf = result & 0x8000_0000 != 0;
        regs.flags.pf = (result as u8).count_ones().is_multiple_of(2);
        regs.flags.af = if up {
            before & 0xf == 0xf
        } else {
            before & 0xf == 0
        };
        regs.flags.of = if up {
            before == 0x7fff_ffff
        } else {
            before == 0x8000_0000
        };
    }
}

impl CpuEngine for Interp {
    fn step(&mut self, regs: &mut Regs, mem: &mut Arena) -> Result<(), Stop> {
        let start = regs.eip;
        // One check for the whole instruction; every fetch below rides on it.
        // An instruction within a maximum length of the arena's end takes the
        // checked path instead, so the tail of the address space is still
        // correct rather than merely rare.
        let fast = mem.has_window(start, Self::MAX_INSN);
        let op = Self::fetch_u8(regs, mem, fast)?;
        match op {
            // nop
            0x90 => {}
            // inc r32
            0x40..=0x47 => {
                let r = (op - 0x40) as usize;
                let before = regs.r[r];
                let after = before.wrapping_add(1);
                regs.r[r] = after;
                Self::inc_dec_flags(regs, after, before, true);
            }
            // dec r32
            0x48..=0x4f => {
                let r = (op - 0x48) as usize;
                let before = regs.r[r];
                let after = before.wrapping_sub(1);
                regs.r[r] = after;
                Self::inc_dec_flags(regs, after, before, false);
            }
            // mov r32, imm32
            0xb8..=0xbf => {
                let r = (op - 0xb8) as usize;
                regs.r[r] = Self::fetch_u32(regs, mem, fast)?;
            }
            // mov eax, moffs32
            0xa1 => {
                let addr = Self::fetch_u32(regs, mem, fast)?;
                let s = mem.slice(addr, 4).map_err(Stop::Fault)?;
                regs.r[super::EAX] = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            }
            // mov moffs32, eax
            0xa3 => {
                let addr = Self::fetch_u32(regs, mem, fast)?;
                let value = regs.r[super::EAX];
                mem.write_u32(addr, value).map_err(Stop::Fault)?;
            }
            // jmp rel8
            0xeb => {
                let rel = Self::fetch_u8(regs, mem, fast)? as i8;
                regs.eip = regs.eip.wrapping_add_signed(i32::from(rel));
            }
            // jz rel8 / jnz rel8
            0x74 | 0x75 => {
                let rel = Self::fetch_u8(regs, mem, fast)? as i8;
                let take = if op == 0x74 {
                    regs.flags.zf
                } else {
                    !regs.flags.zf
                };
                if take {
                    regs.eip = regs.eip.wrapping_add_signed(i32::from(rel));
                }
            }
            // int imm8
            0xcd => {
                let n = Self::fetch_u8(regs, mem, fast)?;
                return Err(Stop::Interrupt(n));
            }
            _ => {
                regs.eip = start;
                return Err(Stop::Undecoded {
                    eip: start,
                    byte: op,
                });
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "interp"
    }
}
