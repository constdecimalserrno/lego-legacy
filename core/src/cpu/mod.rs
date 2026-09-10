//! The x86-32 engine (ADR-0004).
//!
//! Interpretation happens behind [`CpuEngine`] so a later engine can take its
//! place without the machine noticing. This one decodes each instruction where
//! it stands; the decode cache arrives with the conformance corpus.

pub mod interp;

use crate::guest::{Arena, Fault};
use std::fmt;

/// General-purpose registers, in the order the instruction encoding numbers
/// them.
pub const EAX: usize = 0;
pub const ECX: usize = 1;
pub const EDX: usize = 2;
pub const EBX: usize = 3;
pub const ESP: usize = 4;
pub const EBP: usize = 5;
pub const ESI: usize = 6;
pub const EDI: usize = 7;

pub const REG_NAMES: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];

/// The flags this engine keeps. Held as they are computed rather than lazily;
/// the corpus decides whether that stays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub zf: bool,
    pub sf: bool,
    pub cf: bool,
    pub of: bool,
    pub pf: bool,
    pub af: bool,
}

/// Everything the processor is, at one cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Regs {
    pub r: [u32; 8],
    pub eip: u32,
    pub flags: Flags,
}

/// Why the engine stopped between instructions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The guest executed `int n`.
    Interrupt(u8),
    /// An opcode this engine does not decode. A wall: the run stops and names
    /// the byte and where it stood, rather than guessing.
    Undecoded { eip: u32, byte: u8 },
    /// The guest reached outside its own address space.
    Fault(Fault),
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interrupt(n) => write!(f, "int {n:#04x}"),
            Self::Undecoded { eip, byte } => {
                write!(f, "undecoded opcode {byte:#04x} at eip {eip:#010x}")
            }
            Self::Fault(fault) => write!(f, "{fault}"),
        }
    }
}

/// One engine that runs guest instructions (ADR-0004).
pub trait CpuEngine {
    /// Retire exactly one instruction, or stop and say why.
    ///
    /// # Errors
    /// When the instruction cannot be retired: an interrupt, an opcode this
    /// engine does not decode, or an address outside the arena.
    fn step(&mut self, regs: &mut Regs, mem: &mut Arena) -> Result<(), Stop>;

    /// What this engine is called, for the run log.
    fn name(&self) -> &'static str;
}
