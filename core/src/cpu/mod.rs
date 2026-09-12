//! The x86-32 engine (ADR-0004).
//!
//! Decoding is separate from execution and cached between them; execution
//! happens behind [`CpuEngine`] so a later engine can take its place without
//! the machine noticing.

pub mod decode;
pub mod exec;
pub mod flags;
pub mod insn;
pub mod interp;
pub mod x87;

use crate::guest::{Arena, Fault};
use flags::Flags;
use insn::Width;
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

/// Segment registers, in the order the instruction encoding numbers them.
pub const ES: usize = 0;
pub const CS: usize = 1;
pub const SS: usize = 2;
pub const DS: usize = 3;
pub const FS: usize = 4;
pub const GS: usize = 5;

pub const SEG_NAMES: [&str; 6] = ["es", "cs", "ss", "ds", "fs", "gs"];

/// Which machine the processor believes it is inside.
///
/// The roster's titles run flat and 32-bit; the conformance corpus is real
/// mode. One engine serves both, because the difference is entirely in how an
/// address is formed and how wide an operand is by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// Segment registers are shifted four bits and added, rather than naming a
    /// descriptor.
    pub real: bool,
}

impl Mode {
    /// The declared machine the roster runs on: flat, 32-bit, protected.
    pub const FLAT: Self = Self { real: false };
    /// What the conformance corpus was recorded on.
    pub const REAL: Self = Self { real: true };

    /// The operand width an instruction has with no size prefix.
    #[must_use]
    pub fn default_width(self) -> Width {
        if self.real {
            Width::B16
        } else {
            Width::B32
        }
    }

    /// Whether addresses are formed from 32-bit registers with no size prefix.
    #[must_use]
    pub fn default_addr32(self) -> bool {
        !self.real
    }
}

/// Everything the processor is, at one cursor.
///
/// Compared by value rather than by identity, which is what a determinism gate
/// needs. It is not `Eq`, because the floating-point registers are not.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cpu {
    pub r: [u32; 8],
    pub eip: u32,
    /// Segment registers, as the guest last wrote them.
    pub seg: [u16; 6],
    /// What each segment register adds to an address. In real mode this
    /// follows the register; in the flat model it is zero until a title's
    /// thread information block gives `fs` a base of its own.
    pub seg_base: [u32; 6],
    pub flags: Flags,
    /// The floating-point unit.
    pub fpu: x87::X87,
    /// The declared machine's cycle counter, which `rdtsc` reads (#56). The
    /// machine puts the declared clock's own count here before each run and
    /// the engine carries it forward one to the instruction, because a title
    /// timing a few hundred instructions has to see them go by.
    pub tsc: u64,
}

impl Cpu {
    /// Write a segment register, keeping its base in step with the mode.
    pub fn set_seg(&mut self, which: usize, value: u16, mode: Mode) {
        self.seg[which] = value;
        if mode.real {
            self.seg_base[which] = u32::from(value) << 4;
        }
    }
}

/// Why the engine stopped between instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    /// The guest executed `int n`.
    Interrupt(u8),
    /// Bytes this engine does not decode. A wall: the run stops and names what
    /// it found and where it stood, rather than guessing.
    Undecoded {
        eip: u32,
        byte: u8,
        group: Option<u8>,
        why: &'static str,
    },
    /// The guest reached outside its own address space.
    Fault(Fault),
    /// The guest divided by zero, or by a value too small to hold the answer.
    DivideError,
    /// The guest executed `hlt`. Nothing on this machine wakes a halted
    /// processor, so the run is over.
    Halted,
    /// The guest ended the run itself, through the boundary.
    Exited { code: u32 },
    /// The guest called something across the boundary that nothing stands
    /// behind yet.
    NotWritten { what: String, caller: u32 },
    /// The thread that was running ended (ADR-0005). Not the run: the machine
    /// switches to another thread and carries on.
    ThreadEnded,
    /// The thread that was running cannot go on until another one does
    /// something (ADR-0005). The machine switches away and the call it was
    /// making is made again when it comes back.
    Blocked,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interrupt(n) => write!(f, "int {n:#04x}"),
            Self::Undecoded {
                eip,
                byte,
                group,
                why,
            } => {
                write!(f, "undecoded opcode {byte:#04x}")?;
                if let Some(g) = group {
                    write!(f, " /{g}")?;
                }
                write!(f, " at eip {eip:#010x}: {why}")
            }
            Self::Fault(fault) => write!(f, "{fault}"),
            Self::DivideError => write!(f, "divide error"),
            Self::Halted => write!(f, "halted"),
            Self::Exited { code } => write!(f, "exited {code}"),
            Self::ThreadEnded => write!(f, "the running thread ended"),
            Self::Blocked => write!(f, "the running thread is waiting for another"),
            Self::NotWritten { what, caller } => {
                // A name with a colon in it is already a sentence about
                // itself; a bare one needs saying what happened to it.
                if what.contains(':') {
                    write!(f, "{what} (called from {caller:#010x})")
                } else {
                    write!(
                        f,
                        "{what} is imported and not written yet, called from {caller:#010x}"
                    )
                }
            }
        }
    }
}

/// One engine that runs guest instructions (ADR-0004).
pub trait CpuEngine {
    /// Retire exactly one instruction, or stop and say why.
    ///
    /// # Errors
    /// When the instruction cannot be retired: an interrupt, bytes this engine
    /// does not decode, or an address outside the arena.
    fn step(&mut self, cpu: &mut Cpu, mem: &mut Arena) -> Result<(), Stop>;

    /// Retire up to `count` instructions, stopping early before one whose
    /// address is in `outside`, and say how many retired and why it stopped:
    /// exactly what as many calls to [`CpuEngine::step`] would retire. An
    /// interrupt retires the instruction that raised it; any other stop does
    /// not.
    fn run(
        &mut self,
        cpu: &mut Cpu,
        mem: &mut Arena,
        count: u64,
        outside: std::ops::Range<u32>,
    ) -> (u64, Result<(), Stop>);

    /// What this engine is called, for the run log.
    fn name(&self) -> &'static str;
}
