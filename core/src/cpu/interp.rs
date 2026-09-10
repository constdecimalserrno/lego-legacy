//! The interpreter, and the cache between decoding and executing (ADR-0004).
//!
//! Decoding costs several times what executing costs, and guest code runs the
//! same bytes over and over, so an instruction is decoded once per address and
//! executed from that record every time after.

use super::decode::{self, MAX_INSN};
use super::insn::{Insn, Width};
use super::{exec, Cpu, CpuEngine, Mode, Stop, CS};
use crate::guest::{Arena, GuestMem};
use std::collections::HashMap;

/// A decode-cached interpreter.
pub struct Interp {
    mode: Mode,
    /// Decoded instructions, by the linear address they start at.
    //
    // ponytail: no invalidation. Guest code that rewrites itself would run its
    // old form. Nothing on the roster has been measured doing so; the first
    // title that does gets a write watch over cached ranges.
    cache: HashMap<u32, Insn>,
}

impl Interp {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            cache: HashMap::new(),
        }
    }

    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// How many instructions this engine has decoded and kept.
    #[must_use]
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// Forget every decoded instruction. What a loader calls after it has
    /// placed new bytes where old ones were.
    pub fn forget(&mut self) {
        self.cache.clear();
    }

    /// The instruction at a linear address, decoding it if this is the first
    /// time the engine has stood there.
    fn at(&mut self, lin: u32, mem: &Arena) -> Result<Insn, Stop> {
        if let Some(insn) = self.cache.get(&lin) {
            return Ok(*insn);
        }
        // The window is as much as an instruction can be, or as much as is
        // left before the end of the arena.
        let room = (mem.len() - (lin as usize).min(mem.len())).min(MAX_INSN);
        let bytes = mem.slice(lin, room).map_err(Stop::Fault)?;
        let insn = decode::decode(bytes, self.mode).map_err(|u| Stop::Undecoded {
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
        self.cache.insert(lin, insn);
        Ok(insn)
    }
}

impl CpuEngine for Interp {
    fn step(&mut self, cpu: &mut Cpu, mem: &mut Arena) -> Result<(), Stop> {
        let lin = cpu.seg_base[CS].wrapping_add(cpu.eip);
        let insn = self.at(lin, mem)?;
        // The instruction pointer moves before the instruction runs, so that a
        // relative branch is relative to what comes after it.
        // The instruction pointer is not truncated on the way past an
        // instruction, even in real mode: a 386 lets eip run past 64K and only
        // a branch with a sixteen-bit operand size brings it back.
        cpu.eip = cpu.eip.wrapping_add(u32::from(insn.len));
        match exec::execute(cpu, mem, &insn, self.mode) {
            Ok(()) => Ok(()),
            // A halt or an interrupt retires and then stops the run: the
            // instruction pointer belongs after it, which is where a return
            // address would point.
            Err(stop @ (Stop::Halted | Stop::Interrupt(_))) => Err(stop),
            Err(stop) => {
                // A wall leaves the cursor where the guest stood, so the run
                // log names the instruction that could not be retired rather
                // than the one after it.
                cpu.eip = lin.wrapping_sub(cpu.seg_base[CS]);
                Err(stop)
            }
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

/// The width an instruction pointer is truncated to in a mode.
#[must_use]
pub fn ip_width(mode: Mode) -> Width {
    mode.default_width()
}
