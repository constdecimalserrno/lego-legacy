//! What one decoded instruction is.
//!
//! Decoding an instruction and executing it are separate acts (ADR-0004): the
//! first is expensive and repeats, the second is cheap and does not, which is
//! the whole reason there is a cache between them.

/// How wide an operand is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    B8,
    B16,
    B32,
}

impl Width {
    #[must_use]
    pub fn bytes(self) -> u32 {
        match self {
            Self::B8 => 1,
            Self::B16 => 2,
            Self::B32 => 4,
        }
    }

    #[must_use]
    pub fn bits(self) -> u32 {
        self.bytes() * 8
    }

    /// The mask that keeps only this width's bits.
    #[must_use]
    pub fn mask(self) -> u32 {
        match self {
            Self::B8 => 0xff,
            Self::B16 => 0xffff,
            Self::B32 => u32::MAX,
        }
    }

    /// The bit that is the sign for this width.
    #[must_use]
    pub fn sign_bit(self) -> u32 {
        1 << (self.bits() - 1)
    }
}

/// Which segment an address is taken through.
///
/// The declared machine is flat but for `fs`, whose base is the running
/// thread's information block; every other segment's base is zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Seg {
    Es = 0,
    Cs = 1,
    Ss = 2,
    #[default]
    Ds = 3,
    Fs = 4,
    Gs = 5,
}

impl Seg {
    /// Which segment register this is, as the encoding numbers them.
    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How a memory operand's address is formed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemRef {
    pub seg: Seg,
    pub base: Option<u8>,
    /// Index register and its scale, as `(register, shift)`.
    pub index: Option<(u8, u8)>,
    /// What the base register is shifted by before it is added.
    ///
    /// Always zero except on the 386 in real mode, where a scaled address with
    /// no index register scales the base instead. Measured against the
    /// conformance corpus, which is what a 386 actually did; later processors
    /// ignore the scale in that encoding, so the flat model never sets this.
    pub base_shift: u8,
    pub disp: i32,
    /// The address is formed in sixteen bits and wraps there. Real mode's
    /// default, and what a `67` prefix asks for in the flat model.
    pub addr16: bool,
}

/// One operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// A general-purpose register, numbered as the encoding numbers it. At
    /// `Width::B8` the numbers 4 to 7 are the high halves of the first four.
    Reg(u8),
    Mem(MemRef),
    Imm(u32),
}

/// What an instruction does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Mov,
    Lea,
    Push,
    Pop,
    Xchg,
    Add,
    Or,
    Adc,
    Sbb,
    And,
    Sub,
    Xor,
    Cmp,
    Test,
    Inc,
    Dec,
    Neg,
    Not,
    Jmp,
    /// Conditional jump on a condition code.
    Jcc(u8),
    /// Set a byte to one or zero on a condition code.
    Setcc(u8),
    /// Move only if a condition code holds.
    Cmovcc(u8),
    Call,
    Ret,
    /// `loop`, `loope`, `loopne`: the low two bits of the opcode say which.
    Loop {
        /// `Some(true)` waits for the zero flag set, `Some(false)` for clear.
        zf: Option<bool>,
    },
    Jecxz,
    Nop,
    /// A shift or rotate. The count is the source operand.
    Shift(ShiftOp),
    /// A double-precision shift: `shld` and `shrd`.
    Dshift {
        left: bool,
    },
    Mul,
    Imul,
    /// `imul` with an explicit destination, and optionally an immediate.
    Imul2,
    Div,
    Idiv,
    /// A string operation, one element at a time or repeated.
    String(StringOp),
    /// A bit test, and what it does to the bit it tested.
    Bit(BitOp),
    /// Scan for the first or last set bit.
    BitScan {
        forward: bool,
    },
    /// Widen a value, with or without its sign.
    Extend {
        signed: bool,
        from: Width,
    },
    /// Widen the accumulator into itself: `cbw`, `cwde`.
    Cbw,
    /// Widen the accumulator into `dx` or `edx`: `cwd`, `cdq`.
    Cwd,
    Pushf,
    Popf,
    Sahf,
    Lahf,
    /// Set, clear or complement one flag.
    SetFlag {
        flag: FlagBit,
        to: Option<bool>,
    },
    Enter,
    Leave,
    Xlat,
    Pusha,
    Popa,
    /// Read a segment register into an operand, or write one from it.
    MovSegTo,
    MovSegFrom,
    Push0f,
    Pop0f,
    /// A decimal adjustment: the four that exist, by name.
    Bcd(BcdOp),
    /// `wait`: nothing to wait for on a machine with no coprocessor pin.
    Wait,
    /// One of the eight floating-point escapes, pre-decoded into its exact
    /// unit operation once at decode time so execution dispatches directly.
    X87(super::x87::X87Op),
    /// `hlt`: the processor stops until something wakes it. On this machine
    /// nothing does, so it ends the run.
    Hlt,
    /// `int n`.
    Int(u8),
    /// `cpuid`: what the processor says it is. The declared machine's answer,
    /// like every other number a title reads about the machine (#56).
    Cpuid,
    /// `rdtsc`: the declared machine's cycle counter, which is what a title
    /// times itself against.
    Rdtsc,
}

/// Which shift or rotate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftOp {
    Rol,
    Ror,
    Rcl,
    Rcr,
    Shl,
    Shr,
    /// `sal` is `shl` under another name, and encodes as group 6.
    Sal,
    Sar,
}

/// Which string operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringOp {
    Movs,
    Cmps,
    Scas,
    Lods,
    Stos,
}

/// What a bit test does to the bit it tested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    Test,
    Set,
    Reset,
    Complement,
}

/// Which decimal adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BcdOp {
    Daa,
    Das,
    Aaa,
    Aas,
    Aam,
    Aad,
}

/// A flag an instruction sets, clears or complements by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagBit {
    Carry,
    Direction,
    Interrupt,
}

/// How a string instruction repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rep {
    /// `rep`, and `repe` on a comparing instruction.
    WhileEqual,
    /// `repne`.
    WhileNotEqual,
}

/// One decoded instruction, and how many bytes it took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insn {
    pub kind: Kind,
    pub width: Width,
    /// Whether the instruction forms addresses, and counts loops, in
    /// thirty-two bits. Separate from the operand width: a prefix can widen
    /// one without the other.
    pub addr32: bool,
    pub dst: Option<Op>,
    pub src: Option<Op>,
    /// A third operand, which only `imul` and the double shifts have.
    pub third: Option<Op>,
    /// How a string instruction repeats, when it does.
    pub rep: Option<Rep>,
    pub len: u8,
}

/// Sentinel register index meaning "no register in this addressing slot".
///
/// Bit 7 is set so presence is a single sign/bit test (`(reg as i8) >= 0`).
pub const NO_REG: u8 = 0xff;

/// Bit in [`FastInsn::scale`] indicating a memory operand adds `seg_base[FS]`.
pub const FLAG_FS: u8 = 0x04;

/// Sub-operation selector in [`FastInsn::aux`] for [`FastOp::AluMemReg`] and
/// [`FastOp::AluMemImm`].
pub const ALU_ADD: u8 = 0;
pub const ALU_SUB: u8 = 1;
pub const ALU_CMP: u8 = 2;
pub const ALU_AND: u8 = 3;
pub const ALU_OR: u8 = 4;
pub const ALU_XOR: u8 = 5;
pub const ALU_TEST: u8 = 6;

/// Sub-operation selector in [`FastInsn::aux`] for [`FastOp::ShiftRegCl`] and
/// [`FastOp::ShiftMem`].
pub const SHIFT_SHL: u8 = 0;
pub const SHIFT_SHR: u8 = 1;
pub const SHIFT_SAR: u8 = 2;
/// Bit in [`FastInsn::aux`] for [`FastOp::ShiftMem`] selecting `CL` count over immediate.
pub const SHIFT_CL: u8 = 0x80;

/// Sub-operation selector in [`FastInsn::aux`] for [`FastOp::ExtendReg`] and
/// [`FastOp::ExtendMem`].
pub const EXTEND_ZX8: u8 = 0;
pub const EXTEND_ZX16: u8 = 1;
pub const EXTEND_SX8: u8 = 2;
pub const EXTEND_SX16: u8 = 3;

/// Operation tag for [`FastInsn`].
///
/// Non-terminal operations occupy `0..=47` and terminal control-transfer
/// operations occupy `48..=55`, keeping both straight-line and terminal
/// dispatch tables dense and ready for static `naked_asm!` dispatch (#253, #256).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum FastOp {
    /// Fallback to the cold [`Insn`] table indexed by [`FastInsn::disp`].
    #[default]
    Slow = 0,
    Nop = 1,

    // Data movement and address calculation.
    MovRegReg = 2,
    MovRegImm = 3,
    MovRegMem = 4,
    MovMemReg = 5,
    MovMemImm = 6,
    Lea = 7,
    ExtendReg = 8,
    ExtendMem = 9,

    // Flat32 stack operations.
    PushReg = 10,
    PushImm = 11,
    PushMem = 12,
    PopReg = 13,
    PopMem = 14,

    // Flat32 register-destination ALU.
    AddRegReg = 15,
    AddRegImm = 16,
    AddRegMem = 17,
    SubRegReg = 18,
    SubRegImm = 19,
    SubRegMem = 20,
    CmpRegReg = 21,
    CmpRegImm = 22,
    CmpRegMem = 23,
    AndRegReg = 24,
    AndRegImm = 25,
    AndRegMem = 26,
    OrRegReg = 27,
    OrRegImm = 28,
    OrRegMem = 29,
    XorRegReg = 30,
    XorRegImm = 31,
    XorRegMem = 32,
    TestRegReg = 33,
    TestRegImm = 34,

    // Flat32 memory-destination ALU (`aux` selects `ALU_*`).
    AluMemReg = 35,
    AluMemImm = 36,

    // Flat32 increment / decrement.
    IncReg = 37,
    DecReg = 38,
    IncMem = 39,
    DecMem = 40,

    // Flat32 shifts (`shl`/`sal`, `shr`, `sar`).
    ShlRegImm = 41,
    ShrRegImm = 42,
    SarRegImm = 43,
    ShiftRegCl = 44,
    ShiftMem = 45,

    // Pre-decoded x87 dispatch (`imm` indexes the compact `x87_ops` table).
    X87Reg = 46,
    X87Mem = 47,

    // Terminal control transfers (only ever appear as the final op of a run).
    JmpRel = 48,
    JccRel = 49,
    CallRel = 50,
    Ret = 51,
    JmpReg = 52,
    JmpMem = 53,
    CallReg = 54,
    CallMem = 55,

    // Flat32 8-bit and 16-bit data movement, 8/16-bit ALU/test/cmp, imul2, setcc (#256).
    Mov8RegReg = 56,
    Mov8RegImm = 57,
    Mov8RegMem = 58,
    Mov8MemReg = 59,
    Mov8MemImm = 60,
    Mov16RegReg = 61,
    Mov16RegImm = 62,
    Mov16RegMem = 63,
    Mov16MemReg = 64,
    Mov16MemImm = 65,
    Alu8RegImm = 66,
    Alu8RegReg = 67,
    Alu8RegMem = 68,
    Alu8MemImm = 69,
    Alu8MemReg = 70,
    Alu16RegImm = 71,
    Alu16RegReg = 72,
    Alu16RegMem = 73,
    Alu16MemImm = 74,
    Alu16MemReg = 75,
    Imul2Reg = 76,
    Imul2Mem = 77,
    SetccReg = 78,

    /// Sentinel appended after a non-transfer run so direct-threaded
    /// `naked_asm!` dispatch needs no per-instruction loop counter (#256).
    EndRun = 79,
}

impl FastOp {
    /// Whether this op is one of the terminal control-transfer shapes.
    #[must_use]
    #[inline(always)]
    pub fn is_terminal(self) -> bool {
        let tag = self as u8;
        tag >= Self::JmpRel as u8 && tag <= Self::CallMem as u8
    }
}

/// Compact 16-byte flat32 IR instruction for hot-path execution (#253).
///
/// Four instructions share one 64-byte cache line, and the fixed `#[repr(C)]`
/// field offsets let Wave 2 `naked_asm!` (#256) consume the stream directly.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FastInsn {
    pub op: FastOp,
    pub len: u8,
    pub dst: u8,
    pub src: u8,
    pub base: u8,
    pub index: u8,
    pub scale: u8,
    pub aux: u8,
    pub disp: i32,
    pub imm: u32,
}

impl FastInsn {
    #[must_use]
    pub const fn slow(len: u8, slow_idx: u32) -> Self {
        Self {
            op: FastOp::Slow,
            len,
            dst: NO_REG,
            src: NO_REG,
            base: NO_REG,
            index: NO_REG,
            scale: 0,
            aux: 0,
            disp: slow_idx as i32,
            imm: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FastInsn, FastOp};
    use core::mem::{align_of, offset_of, size_of};

    #[test]
    fn fast_insn_layout_is_compact_16_byte_repr_c() {
        assert_eq!(size_of::<FastOp>(), 1);
        assert_eq!(size_of::<FastInsn>(), 16);
        assert_eq!(align_of::<FastInsn>(), 4);
        assert_eq!(offset_of!(FastInsn, op), 0);
        assert_eq!(offset_of!(FastInsn, len), 1);
        assert_eq!(offset_of!(FastInsn, dst), 2);
        assert_eq!(offset_of!(FastInsn, src), 3);
        assert_eq!(offset_of!(FastInsn, base), 4);
        assert_eq!(offset_of!(FastInsn, index), 5);
        assert_eq!(offset_of!(FastInsn, scale), 6);
        assert_eq!(offset_of!(FastInsn, aux), 7);
        assert_eq!(offset_of!(FastInsn, disp), 8);
        assert_eq!(offset_of!(FastInsn, imm), 12);
    }
}
