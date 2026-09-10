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
/// The declared machine is flat: every segment's base is zero, so a prefix
/// changes nothing today. It is decoded rather than dropped because `fs` stops
/// being flat the moment a title reads its thread information block.
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
    /// `hlt`: the processor stops until something wakes it. On this machine
    /// nothing does, so it ends the run.
    Hlt,
    /// `int n`.
    Int(u8),
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
