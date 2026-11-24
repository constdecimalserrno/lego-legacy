//! The floating-point unit.
//!
//! Eight registers in a stack, the control and status words the guest reads,
//! and the arithmetic a title's geometry is made of. Every transcendental is
//! written here rather than taken from the host's maths library: a run must
//! give the same answer on every machine, and a library that is free to
//! improve is a run that is free to change.

use super::Stop;
use crate::guest::{Arena, GuestMem};

/// How many registers the stack has.
pub const DEPTH: usize = 8;

/// What the control word's precision field asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// 24 bits of significand: a single.
    Single,
    /// 53 bits: a double.
    Double,
    /// 64 bits: the extended format the hardware works in.
    Extended,
}

/// The floating-point unit's whole state.
///
// ponytail: registers are held as f64 rather than the hardware's 80-bit
// extended format. Every value a title stores back to memory is a single or a
// double, so the difference only shows in a long chain of arithmetic that never
// touches memory. Upgrade path: a soft extended-precision float behind this
// same interface, if a title is ever measured drifting because of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct X87 {
    /// The registers, indexed by their physical number rather than by their
    /// position in the stack.
    regs: [f64; DEPTH],
    /// Which physical register `st(0)` is.
    top: usize,
    /// Which registers hold a value.
    used: [bool; DEPTH],
    /// The control word the guest wrote.
    control: u16,
    /// The condition codes the last comparison left.
    status: u16,
}

impl Default for X87 {
    fn default() -> Self {
        Self::new()
    }
}

impl X87 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            regs: [0.0; DEPTH],
            top: 0,
            used: [false; DEPTH],
            // What `fninit` leaves behind: extended precision, round to
            // nearest, every exception masked.
            control: 0x037f,
            status: 0,
        }
    }

    /// The whole unit the same bit for bit, which `==` on its registers is
    /// not: it calls 0.0 and -0.0 one, and a NaN no number.
    #[must_use]
    pub fn same_bits(&self, other: &Self) -> bool {
        let rest = |unit: &Self| Self {
            regs: [0.0; DEPTH],
            ..*unit
        };
        self.regs
            .iter()
            .zip(&other.regs)
            .all(|(a, b)| a.to_bits() == b.to_bits())
            && rest(self) == rest(other)
    }

    /// Put the unit back the way `fninit` leaves it.
    pub fn init(&mut self) {
        *self = Self::new();
    }

    #[must_use]
    pub fn control(&self) -> u16 {
        self.control
    }

    pub fn set_control(&mut self, value: u16) {
        self.control = value;
    }

    /// The status word, with the stack's top written into it where the guest
    /// expects to read it.
    #[must_use]
    pub fn status(&self) -> u16 {
        (self.status & !0x3800) | ((self.top as u16 & 7) << 11)
    }

    #[must_use]
    #[inline(always)]
    pub fn precision(&self) -> Precision {
        match (self.control >> 8) & 3 {
            0 => Precision::Single,
            2 => Precision::Double,
            _ => Precision::Extended,
        }
    }

    /// Round to a whole number the way the control word's rounding field
    /// says: to nearest with ties to even, down, up, or toward zero. The C
    /// library's `_ftol` sets toward zero for every `(int)` cast it makes.
    #[must_use]
    #[inline(always)]
    pub fn to_whole(&self, x: f64) -> f64 {
        if x.is_nan() {
            return answer(x, x, x);
        }
        match (self.control >> 10) & 3 {
            0 => round_half_to_even(x),
            1 => x.floor(),
            2 => x.ceil(),
            _ => x.trunc(),
        }
    }

    /// Round a result to the precision the guest asked for.
    #[inline(always)]
    fn round(&self, value: f64) -> f64 {
        if (self.control >> 8) & 3 == 0 {
            f64::from(value as f32)
        } else {
            value
        }
    }

    #[inline(always)]
    fn physical(&self, i: usize) -> usize {
        (self.top + i) % DEPTH
    }

    /// Read `st(i)`.
    #[must_use]
    #[inline(always)]
    pub fn st(&self, i: usize) -> f64 {
        self.regs[self.physical(i)]
    }

    /// Write `st(i)`.
    #[inline(always)]
    pub fn set_st(&mut self, i: usize, value: f64) {
        let at = self.physical(i);
        self.regs[at] = self.round(value);
        self.used[at] = true;
    }

    /// Push a value, which becomes `st(0)`.
    #[inline(always)]
    pub fn push(&mut self, value: f64) {
        self.top = (self.top + DEPTH - 1) % DEPTH;
        self.regs[self.top] = self.round(value);
        self.used[self.top] = true;
    }

    /// Take `st(0)` off the stack.
    #[inline(always)]
    pub fn pop(&mut self) -> f64 {
        let value = self.regs[self.top];
        self.used[self.top] = false;
        self.top = (self.top + 1) % DEPTH;
        value
    }

    /// Whether `st(i)` holds anything.
    #[must_use]
    pub fn occupied(&self, i: usize) -> bool {
        self.used[self.physical(i)]
    }

    /// Give up a register without reading it.
    pub fn free(&mut self, i: usize) {
        let at = self.physical(i);
        self.used[at] = false;
    }

    /// Move the stack's top down one, as `fdecstp` does.
    pub fn decrement_top(&mut self) {
        self.top = (self.top + DEPTH - 1) % DEPTH;
    }

    /// Move the stack's top up one, as `fincstp` does.
    pub fn increment_top(&mut self) {
        self.top = (self.top + 1) % DEPTH;
    }

    /// Clear the exception flags, as `fnclex` does.
    pub fn clear_exceptions(&mut self) {
        self.status &= !0x00ff;
    }

    /// Set the condition codes from a comparison.
    ///
    /// C3, C2 and C0 spell the answer: equal sets C3, less sets C0, and
    /// unordered sets all three.
    pub fn compare(&mut self, a: f64, b: f64) {
        let (c3, c2, c0) = if a.is_nan() || b.is_nan() {
            (true, true, true)
        } else if a > b {
            (false, false, false)
        } else if a < b {
            (false, false, true)
        } else {
            (true, false, false)
        };
        self.status &= !0x4500;
        if c0 {
            self.status |= 0x0100;
        }
        if c2 {
            self.status |= 0x0400;
        }
        if c3 {
            self.status |= 0x4000;
        }
    }

    /// Classify `st(0)`, as `fxam` does: C1 is its sign, and C3, C2 and C0
    /// say what kind of value it is.
    ///
    /// The registers hold doubles, and every double that is not zero, infinite
    /// or a NaN is a normal number in the unit's extended format, so a
    /// denormal is never reported.
    pub fn examine(&mut self) {
        let value = self.st(0);
        let (c3, c2, c0) = if !self.occupied(0) {
            (true, false, true)
        } else if value.is_nan() {
            (false, false, true)
        } else if value.is_infinite() {
            (false, true, true)
        } else if value == 0.0 {
            (true, false, false)
        } else {
            (false, true, false)
        };
        self.status &= !0x4700;
        for (set, bit) in [
            (c0, 0x0100),
            (value.is_sign_negative(), 0x0200),
            (c2, 0x0400),
            (c3, 0x4000),
        ] {
            if set {
                self.status |= bit;
            }
        }
    }

    /// `fprem` and `fprem1`: `st(0)` less a whole number of `st(1)`s, the
    /// quotient rounded toward zero or to nearest.
    ///
    /// The unit reduces a remainder a step at a time and sets C2 while there
    /// is more to do; a guest loops on it. Here the reduction is exact in one
    /// step, so C2 is always clear, and C0, C3 and C1 are the quotient's three
    /// low bits, as the unit reports them.
    // ponytail: the unit sets C2 and stops part way when the operands' exponents
    // are more than 63 apart; a guest's loop copes with either, so the whole
    // reduction happens at once here.
    pub fn remainder(&mut self, nearest: bool) {
        let (value, divisor) = (self.st(0), self.st(1));
        let d = divisor.abs();
        let mut rest = answer(value % divisor, value, divisor);
        // What a reduction by eight divisors leaves, less what a reduction by
        // one does, is the quotient's low three bits in divisors.
        let mut low = if rest.is_finite() && (8.0 * d).is_finite() && d > 0.0 {
            ((((value % (8.0 * d)).abs() - rest.abs()) / d).round() as u32) % 8
        } else {
            0
        };
        if nearest && rest.is_finite() {
            let half = d / 2.0;
            if rest.abs() > half || (rest.abs() == half && low % 2 == 1) {
                rest -= d.copysign(rest);
                low = (low + 1) % 8;
            }
        }
        self.set_st(0, rest);
        self.status &= !0x4700;
        for (set, bit) in [
            (low & 4 != 0, 0x0100),
            (low & 2 != 0, 0x0200),
            (low & 1 != 0, 0x4000),
        ] {
            if set {
                self.status |= bit;
            }
        }
    }

    /// Set the flags a comparison against the integer unit writes.
    ///
    /// `fcomi` puts the answer in the processor's own flags instead of the
    /// unit's condition codes.
    #[must_use]
    pub fn compare_flags(a: f64, b: f64) -> (bool, bool, bool) {
        // (zero, parity, carry)
        if a.is_nan() || b.is_nan() {
            (true, true, true)
        } else if a > b {
            (false, false, false)
        } else if a < b {
            (false, false, true)
        } else {
            (true, false, false)
        }
    }
}

/// The NaN the unit makes when an operation has no answer, as `0/0` has
/// none: the real indefinite, negative and quiet.
pub const INDEFINITE: f64 = f64::from_bits(0xfff8_0000_0000_0000);

/// An operation's answer as the unit gives it, when the answer is not a
/// number.
///
/// The host's own arithmetic makes a NaN of its own choosing, and hosts do
/// not agree: an x86 host makes the unit's indefinite, an Arm host a positive
/// NaN, so a guest that stored one ran differently on each (#108). The unit's
/// rule is the same everywhere: a NaN operand comes through quieted, the one
/// with the larger significand when both are, and an answer made from two
/// numbers is the indefinite.
#[must_use]
pub fn answer(result: f64, a: f64, b: f64) -> f64 {
    if !result.is_nan() {
        return result;
    }
    const QUIET: u64 = 0x0008_0000_0000_0000;
    const SIGNIFICAND: u64 = 0x000f_ffff_ffff_ffff;
    // A quiet NaN's significand is always the larger, its top bit being the
    // one that makes it quiet.
    let nan = match (a.is_nan(), b.is_nan()) {
        (false, false) => return INDEFINITE,
        (true, true) if b.to_bits() & SIGNIFICAND > a.to_bits() & SIGNIFICAND => b,
        (true, _) => a,
        (false, true) => b,
    };
    f64::from_bits(nan.to_bits() | QUIET)
}

/// [`answer`] for a single-precision result a shim's own arithmetic made
/// from any number of operands, as retained mode's maths makes them (#109):
/// the NaN a real x86 machine gives, never the one this host made.
///
/// The widths change as the unit's own loads and stores change them, which
/// keep a NaN's sign and significand on every host.
#[must_use]
pub fn answer_single(result: f32, operands: impl IntoIterator<Item = f32>) -> f32 {
    if !result.is_nan() {
        return result;
    }
    let nan = operands
        .into_iter()
        .map(f64::from)
        .filter(|x| x.is_nan())
        .reduce(|a, b| answer(f64::NAN, a, b))
        .unwrap_or(0.0);
    answer(f64::NAN, nan, nan) as f32
}

/// How wide a floating-point value is in memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Four bytes: what a title stores its vertices in.
    Single,
    /// Eight bytes.
    Double,
}

/// Read a floating-point value out of memory.
///
/// # Errors
/// When the address is outside the arena.
#[inline(always)]
pub fn read(mem: &Arena, addr: u32, format: Format) -> Result<f64, Stop> {
    Ok(match format {
        Format::Single => {
            let bits = mem.read_u32(addr).map_err(Stop::Fault)?;
            f64::from(f32::from_bits(bits))
        }
        Format::Double => {
            let bits = mem.read_u64(addr).map_err(Stop::Fault)?;
            f64::from_bits(bits)
        }
    })
}

/// Write a floating-point value in one of the memory formats.
///
/// # Errors
/// When the address is outside the arena.
#[inline(always)]
pub fn write(mem: &mut Arena, addr: u32, format: Format, value: f64) -> Result<(), Stop> {
    match format {
        Format::Single => mem
            .write_u32(addr, (value as f32).to_bits())
            .map_err(Stop::Fault),
        Format::Double => mem.write_u64(addr, value.to_bits()).map_err(Stop::Fault),
    }
}

/// Read the ten-byte extended format the hardware keeps its registers in.
///
/// # Errors
/// When the address is outside the arena.
pub fn read_extended(mem: &Arena, addr: u32) -> Result<f64, Stop> {
    let significand = mem.read_u64(addr).map_err(Stop::Fault)?;
    let tail = mem.read_u16(addr + 8).map_err(Stop::Fault)?;
    let sign = if tail & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from(tail & 0x7fff);
    if exponent == 0 && significand == 0 {
        return Ok(0.0 * sign);
    }
    // The extended format writes its leading one down; the value is the
    // significand scaled by two to the exponent, biased by 16383.
    let mantissa = significand as f64 / (1u64 << 63) as f64;
    Ok(sign * mantissa * exp2(exponent - 16383))
}

/// Write the ten-byte extended format.
///
/// # Errors
/// When the address is outside the arena.
pub fn write_extended(mem: &mut Arena, addr: u32, value: f64) -> Result<(), Stop> {
    let sign = if value.is_sign_negative() {
        0x8000u16
    } else {
        0
    };
    let magnitude = value.abs();
    if magnitude == 0.0 {
        mem.write_u64(addr, 0).map_err(Stop::Fault)?;
        mem.write_u16(addr + 8, sign).map_err(Stop::Fault)?;
        return Ok(());
    }
    // The exponent is already written down in the value's own bits, so it is
    // read rather than computed: no logarithm, and no rounding on the way.
    let exponent = ((magnitude.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let mantissa = magnitude / exp2(exponent);
    let significand = (mantissa * (1u64 << 63) as f64) as u64;
    mem.write_u64(addr, significand).map_err(Stop::Fault)?;
    let tail = sign | u16::try_from(exponent + 16383).unwrap_or(0x7fff);
    mem.write_u16(addr + 8, tail).map_err(Stop::Fault)?;
    Ok(())
}

/// Two to a whole power, without reaching for the host's maths library.
#[must_use]
pub fn exp2(mut power: i32) -> f64 {
    let mut result = 1.0f64;
    let base = if power < 0 { 0.5f64 } else { 2.0f64 };
    power = power.abs();
    let mut factor = base;
    while power > 0 {
        if power & 1 == 1 {
            result *= factor;
        }
        factor *= factor;
        power >>= 1;
    }
    result
}

/// The transcendentals, written here rather than taken from the host.
///
/// Nothing in this module calls the platform's maths library. A run has to
/// give the same answer on every machine and in twenty years' time, and a
/// library that is free to improve its accuracy is a run that is free to
/// change its behaviour.
pub mod maths {
    use super::exp2;

    /// π, to the last bit a double holds.
    pub const PI: f64 = std::f64::consts::PI;
    const HALF_PI: f64 = PI / 2.0;
    /// The natural logarithm of two.
    pub const LN_2: f64 = std::f64::consts::LN_2;

    /// Bring an angle into the range the series converges quickly on, and say
    /// which quadrant it came from.
    fn reduce(x: f64) -> (f64, i32) {
        let turns = (x / HALF_PI).round();
        let remainder = x - turns * HALF_PI;
        let quadrant = ((turns as i64) & 3) as i32;
        (remainder, quadrant)
    }

    /// Sine, by its Taylor series on a reduced argument.
    #[must_use]
    pub fn sin(x: f64) -> f64 {
        if !x.is_finite() {
            return f64::NAN;
        }
        let (r, quadrant) = reduce(x);
        match quadrant {
            0 => taylor_sin(r),
            1 => taylor_cos(r),
            2 => -taylor_sin(r),
            _ => -taylor_cos(r),
        }
    }

    /// Cosine, from the same reduction.
    #[must_use]
    pub fn cos(x: f64) -> f64 {
        if !x.is_finite() {
            return f64::NAN;
        }
        let (r, quadrant) = reduce(x);
        match quadrant {
            0 => taylor_cos(r),
            1 => -taylor_sin(r),
            2 => -taylor_cos(r),
            _ => taylor_sin(r),
        }
    }

    fn taylor_sin(x: f64) -> f64 {
        // Eight terms carry a quarter turn to the last bit of a double.
        let x2 = x * x;
        let mut term = x;
        let mut sum = x;
        for n in 1..9 {
            let k = f64::from(2 * n) * f64::from(2 * n + 1);
            term *= -x2 / k;
            sum += term;
        }
        sum
    }

    fn taylor_cos(x: f64) -> f64 {
        let x2 = x * x;
        let mut term = 1.0;
        let mut sum = 1.0;
        for n in 1..9 {
            let k = f64::from(2 * n - 1) * f64::from(2 * n);
            term *= -x2 / k;
            sum += term;
        }
        sum
    }

    /// Tangent.
    #[must_use]
    pub fn tan(x: f64) -> f64 {
        sin(x) / cos(x)
    }

    /// The angle whose tangent is `x`.
    #[must_use]
    pub fn atan(x: f64) -> f64 {
        if x.is_nan() {
            return x;
        }
        if x.is_infinite() {
            return if x > 0.0 { HALF_PI } else { -HALF_PI };
        }
        let negative = x < 0.0;
        let mut a = x.abs();
        // Two reductions bring the argument under tan(π/12), where the series
        // converges fast: a > 1 is turned over, and what is left is folded by
        // the angle-difference identity.
        let mut offset = 0.0;
        if a > 1.0 {
            a = 1.0 / a;
            offset = HALF_PI;
        }
        let mut sign = if offset == 0.0 { 1.0 } else { -1.0 };
        const SQRT3_MINUS_2: f64 = 0.267_949_192_431_122_7; // tan(π/12)
        if a > SQRT3_MINUS_2 {
            const SQRT3: f64 = 1.732_050_807_568_877_2;
            let folded = (a * SQRT3 - 1.0) / (SQRT3 + a);
            offset += sign * (PI / 6.0);
            a = folded.abs();
            if folded < 0.0 {
                sign = -sign;
            }
        }
        let series = {
            let x2 = a * a;
            let mut term = a;
            let mut sum = a;
            for n in 1..24 {
                term *= -x2;
                sum += term / f64::from(2 * n + 1);
            }
            sum
        };
        let result = offset + sign * series;
        if negative {
            -result
        } else {
            result
        }
    }

    /// The angle to the point `(x, y)`, measured from the positive x axis.
    #[must_use]
    pub fn atan2(y: f64, x: f64) -> f64 {
        if x > 0.0 {
            atan(y / x)
        } else if x < 0.0 {
            if y >= 0.0 {
                atan(y / x) + PI
            } else {
                atan(y / x) - PI
            }
        } else if y > 0.0 {
            HALF_PI
        } else if y < 0.0 {
            -HALF_PI
        } else {
            0.0
        }
    }

    /// The base-two logarithm.
    #[must_use]
    pub fn log2(x: f64) -> f64 {
        if x.is_nan() || x < 0.0 {
            return f64::NAN;
        }
        if x == 0.0 {
            return f64::NEG_INFINITY;
        }
        if x.is_infinite() {
            return x;
        }
        // Split the exponent off, leaving a significand in [1, 2), where the
        // series in (m-1)/(m+1) converges quickly.
        let bits = x.to_bits();
        let mut exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
        let mut mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
        if mantissa > 1.5 {
            mantissa /= 2.0;
            exponent += 1;
        }
        let z = (mantissa - 1.0) / (mantissa + 1.0);
        let z2 = z * z;
        let mut term = z;
        let mut sum = z;
        for n in 1..30 {
            term *= z2;
            sum += term / f64::from(2 * n + 1);
        }
        f64::from(exponent) + 2.0 * sum / LN_2
    }

    /// Two raised to a power that need not be whole.
    #[must_use]
    pub fn pow2(x: f64) -> f64 {
        if x.is_nan() {
            return x;
        }
        let whole = x.floor();
        let fraction = x - whole;
        // The fractional part goes through the exponential's own series.
        let t = fraction * LN_2;
        let mut term = 1.0;
        let mut sum = 1.0;
        for n in 1..20 {
            term *= t / f64::from(n);
            sum += term;
        }
        let clamped = whole.clamp(-1100.0, 1100.0) as i32;
        sum * exp2(clamped)
    }
}

/// One pre-decoded floating-point operation, stored directly in [`super::insn::Kind::X87`].
///
/// Keeping every payload at most one byte holds `size_of::<X87Op>()` to 2 bytes
/// so `Kind` and `Insn` do not grow while `execute` dispatches straight to the
/// operation with no per-execution `(escape, modrm)` decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X87Op {
    // Fixed register / stackless forms.
    Fnop,
    Fchs,
    Fabs,
    Ftst,
    Fxam,
    Fld1,
    Fldl2t,
    Fldl2e,
    Fldpi,
    Fldlg2,
    Fldln2,
    Fldz,
    F2xm1,
    Fyl2x,
    Fptan,
    Fpatan,
    Fxtract,
    Fprem1,
    Fdecstp,
    Fincstp,
    Fprem,
    Fyl2xp1,
    Fsqrt,
    Fsincos,
    Frndint,
    Fscale,
    Fsin,
    Fcos,
    Fnclex,
    Fninit,
    Fcompp,
    FnstswAx,

    // Parameterized register forms (`st(0)` / `st(i)`).
    FldSt(u8),
    Fxch(u8),
    Ffree(u8),
    FstSt(u8),
    FstpSt(u8),
    FcomSt(u8),
    FcompSt(u8),
    Fcomi(u8),
    Fcomip(u8),
    FaddSt0(u8),
    FmulSt0(u8),
    FsubSt0(u8),
    FsubrSt0(u8),
    FdivSt0(u8),
    FdivrSt0(u8),
    FaddSti(u8),
    FmulSti(u8),
    FsubSti(u8),
    FsubrSti(u8),
    FdivSti(u8),
    FdivrSti(u8),
    Faddp(u8),
    Fmulp(u8),
    Fsubp(u8),
    Fsubrp(u8),
    Fdivp(u8),
    Fdivrp(u8),

    // Memory arithmetic and comparisons, specialized by source format.
    FaddF32,
    FmulF32,
    FcomF32,
    FcompF32,
    FsubF32,
    FsubrF32,
    FdivF32,
    FdivrF32,
    FaddF64,
    FmulF64,
    FcomF64,
    FcompF64,
    FsubF64,
    FsubrF64,
    FdivF64,
    FdivrF64,
    FiaddI32,
    FimulI32,
    FicomI32,
    FicompI32,
    FisubI32,
    FisubrI32,
    FidivI32,
    FidivrI32,
    FiaddI16,
    FimulI16,
    FicomI16,
    FicompI16,
    FisubI16,
    FisubrI16,
    FidivI16,
    FidivrI16,

    // Memory loads, stores, and control/status/environment transfers.
    FldF32,
    FldF64,
    FildI32,
    FildI16,
    FldF80,
    FildI64,
    FstF32,
    FstpF32,
    FstF64,
    FstpF64,
    FstpF80,
    FistI32,
    FistpI32,
    FistI16,
    FistpI16,
    FistpI64,
    Fldcw,
    Fnstcw,
    FnstswMem,
    Fnstenv,
    Fldenv,
}

/// Decode a register-form (`modrm >> 6 == 3`) floating-point escape once at
/// instruction decode time.
///
/// # Errors
/// When the `(escape, modrm)` form is not implemented by this unit.
pub fn decode_reg(escape: u8, modrm: u8) -> Result<X87Op, super::decode::Undecoded> {
    let exact = match (escape, modrm) {
        (0xd9, 0xd0) => Some(X87Op::Fnop),
        (0xd9, 0xe0) => Some(X87Op::Fchs),
        (0xd9, 0xe1) => Some(X87Op::Fabs),
        (0xd9, 0xe4) => Some(X87Op::Ftst),
        (0xd9, 0xe5) => Some(X87Op::Fxam),
        (0xd9, 0xe8) => Some(X87Op::Fld1),
        (0xd9, 0xe9) => Some(X87Op::Fldl2t),
        (0xd9, 0xea) => Some(X87Op::Fldl2e),
        (0xd9, 0xeb) => Some(X87Op::Fldpi),
        (0xd9, 0xec) => Some(X87Op::Fldlg2),
        (0xd9, 0xed) => Some(X87Op::Fldln2),
        (0xd9, 0xee) => Some(X87Op::Fldz),
        (0xd9, 0xf0) => Some(X87Op::F2xm1),
        (0xd9, 0xf1) => Some(X87Op::Fyl2x),
        (0xd9, 0xf2) => Some(X87Op::Fptan),
        (0xd9, 0xf3) => Some(X87Op::Fpatan),
        (0xd9, 0xf4) => Some(X87Op::Fxtract),
        (0xd9, 0xf5) => Some(X87Op::Fprem1),
        (0xd9, 0xf6) => Some(X87Op::Fdecstp),
        (0xd9, 0xf7) => Some(X87Op::Fincstp),
        (0xd9, 0xf8) => Some(X87Op::Fprem),
        (0xd9, 0xf9) => Some(X87Op::Fyl2xp1),
        (0xd9, 0xfa) => Some(X87Op::Fsqrt),
        (0xd9, 0xfb) => Some(X87Op::Fsincos),
        (0xd9, 0xfc) => Some(X87Op::Frndint),
        (0xd9, 0xfd) => Some(X87Op::Fscale),
        (0xd9, 0xfe) => Some(X87Op::Fsin),
        (0xd9, 0xff) => Some(X87Op::Fcos),
        (0xdb, 0xe2) => Some(X87Op::Fnclex),
        (0xdb, 0xe3) => Some(X87Op::Fninit),
        (0xde, 0xd9) | (0xda, 0xe9) => Some(X87Op::Fcompp),
        (0xdf, 0xe0) => Some(X87Op::FnstswAx),
        _ => None,
    };
    if let Some(op) = exact {
        return Ok(op);
    }

    let reg = (modrm >> 3) & 7;
    let i = modrm & 7;
    match (escape, reg) {
        (0xd9, 0) => Ok(X87Op::FldSt(i)),
        (0xd9, 1) => Ok(X87Op::Fxch(i)),
        (0xdd, 0) => Ok(X87Op::Ffree(i)),
        (0xdd, 2) => Ok(X87Op::FstSt(i)),
        (0xdd, 3) => Ok(X87Op::FstpSt(i)),
        (0xdd, 4) | (0xd8, 2) => Ok(X87Op::FcomSt(i)),
        (0xdd, 5) | (0xd8, 3) => Ok(X87Op::FcompSt(i)),
        (0xda | 0xdb, 5 | 6) => Ok(X87Op::Fcomi(i)),
        (0xdf, 5 | 6) => Ok(X87Op::Fcomip(i)),
        (0xd8, 0) => Ok(X87Op::FaddSt0(i)),
        (0xd8, 1) => Ok(X87Op::FmulSt0(i)),
        (0xd8, 4) => Ok(X87Op::FsubSt0(i)),
        (0xd8, 5) => Ok(X87Op::FsubrSt0(i)),
        (0xd8, 6) => Ok(X87Op::FdivSt0(i)),
        (0xd8, 7) => Ok(X87Op::FdivrSt0(i)),
        (0xdc, 0) => Ok(X87Op::FaddSti(i)),
        (0xdc, 1) => Ok(X87Op::FmulSti(i)),
        (0xdc, 4) => Ok(X87Op::FsubSti(i)),
        (0xdc, 5) => Ok(X87Op::FsubrSti(i)),
        (0xdc, 6) => Ok(X87Op::FdivSti(i)),
        (0xdc, 7) => Ok(X87Op::FdivrSti(i)),
        (0xde, 0) => Ok(X87Op::Faddp(i)),
        (0xde, 1) => Ok(X87Op::Fmulp(i)),
        (0xde, 4) => Ok(X87Op::Fsubp(i)),
        (0xde, 5) => Ok(X87Op::Fsubrp(i)),
        (0xde, 6) => Ok(X87Op::Fdivp(i)),
        (0xde, 7) => Ok(X87Op::Fdivrp(i)),
        _ => Err(super::decode::Undecoded {
            byte: escape,
            group: Some(reg),
            why: "a floating-point register form this unit does not have",
        }),
    }
}

/// Decode a memory-form (`modrm >> 6 != 3`) floating-point escape once at
/// instruction decode time.
///
/// # Errors
/// When the `(escape, reg)` form is not implemented by this unit.
pub fn decode_mem(escape: u8, reg: u8) -> Result<X87Op, super::decode::Undecoded> {
    match (escape, reg) {
        (0xd8, 0) => Ok(X87Op::FaddF32),
        (0xd8, 1) => Ok(X87Op::FmulF32),
        (0xd8, 2) => Ok(X87Op::FcomF32),
        (0xd8, 3) => Ok(X87Op::FcompF32),
        (0xd8, 4) => Ok(X87Op::FsubF32),
        (0xd8, 5) => Ok(X87Op::FsubrF32),
        (0xd8, 6) => Ok(X87Op::FdivF32),
        (0xd8, 7) => Ok(X87Op::FdivrF32),
        (0xdc, 0) => Ok(X87Op::FaddF64),
        (0xdc, 1) => Ok(X87Op::FmulF64),
        (0xdc, 2) => Ok(X87Op::FcomF64),
        (0xdc, 3) => Ok(X87Op::FcompF64),
        (0xdc, 4) => Ok(X87Op::FsubF64),
        (0xdc, 5) => Ok(X87Op::FsubrF64),
        (0xdc, 6) => Ok(X87Op::FdivF64),
        (0xdc, 7) => Ok(X87Op::FdivrF64),
        (0xda, 0) => Ok(X87Op::FiaddI32),
        (0xda, 1) => Ok(X87Op::FimulI32),
        (0xda, 2) => Ok(X87Op::FicomI32),
        (0xda, 3) => Ok(X87Op::FicompI32),
        (0xda, 4) => Ok(X87Op::FisubI32),
        (0xda, 5) => Ok(X87Op::FisubrI32),
        (0xda, 6) => Ok(X87Op::FidivI32),
        (0xda, 7) => Ok(X87Op::FidivrI32),
        (0xde, 0) => Ok(X87Op::FiaddI16),
        (0xde, 1) => Ok(X87Op::FimulI16),
        (0xde, 2) => Ok(X87Op::FicomI16),
        (0xde, 3) => Ok(X87Op::FicompI16),
        (0xde, 4) => Ok(X87Op::FisubI16),
        (0xde, 5) => Ok(X87Op::FisubrI16),
        (0xde, 6) => Ok(X87Op::FidivI16),
        (0xde, 7) => Ok(X87Op::FidivrI16),
        (0xd9, 0) => Ok(X87Op::FldF32),
        (0xd9, 2) => Ok(X87Op::FstF32),
        (0xd9, 3) => Ok(X87Op::FstpF32),
        (0xd9, 4) => Ok(X87Op::Fldenv),
        (0xd9, 5) => Ok(X87Op::Fldcw),
        (0xd9, 6) => Ok(X87Op::Fnstenv),
        (0xd9, 7) => Ok(X87Op::Fnstcw),
        (0xdd, 0) => Ok(X87Op::FldF64),
        (0xdd, 2) => Ok(X87Op::FstF64),
        (0xdd, 3) => Ok(X87Op::FstpF64),
        (0xdd, 7) => Ok(X87Op::FnstswMem),
        (0xdb, 0) => Ok(X87Op::FildI32),
        (0xdb, 2) => Ok(X87Op::FistI32),
        (0xdb, 3) => Ok(X87Op::FistpI32),
        (0xdb, 5) => Ok(X87Op::FldF80),
        (0xdb, 7) => Ok(X87Op::FstpF80),
        (0xdf, 0) => Ok(X87Op::FildI16),
        (0xdf, 2) => Ok(X87Op::FistI16),
        (0xdf, 3) => Ok(X87Op::FistpI16),
        (0xdf, 5) => Ok(X87Op::FildI64),
        (0xdf, 7) => Ok(X87Op::FistpI64),
        _ => Err(super::decode::Undecoded {
            byte: escape,
            group: Some(reg),
            why: "a floating-point memory form this unit does not have",
        }),
    }
}

#[inline(always)]
fn load_f32(mem: &Arena, at: u32) -> Result<f64, Stop> {
    read(mem, at, Format::Single)
}

#[inline(always)]
fn load_f64(mem: &Arena, at: u32) -> Result<f64, Stop> {
    read(mem, at, Format::Double)
}

#[inline(always)]
fn load_i32(mem: &Arena, at: u32) -> Result<f64, Stop> {
    Ok(f64::from(mem.read_u32(at).map_err(Stop::Fault)? as i32))
}

#[inline(always)]
fn load_i16(mem: &Arena, at: u32) -> Result<f64, Stop> {
    Ok(f64::from(i32::from(
        mem.read_u16(at).map_err(Stop::Fault)? as i16
    )))
}

#[inline(always)]
fn reg_binop(
    fpu: &mut X87,
    index: usize,
    to_top: bool,
    pop_after: bool,
    op: impl FnOnce(f64, f64) -> f64,
) {
    let top = fpu.st(0);
    let other = fpu.st(index);
    let result = answer(op(top, other), top, other);
    if to_top {
        fpu.set_st(0, result);
    } else {
        fpu.set_st(index, result);
    }
    if pop_after {
        fpu.pop();
    }
}

#[inline(always)]
fn mem_binop(fpu: &mut X87, value: f64, op: impl FnOnce(f64, f64) -> f64) {
    let top = fpu.st(0);
    fpu.set_st(0, answer(op(top, value), top, value));
}

/// Run one pre-decoded floating-point operation.
///
/// For memory forms `at` is the effective guest address; register forms ignore it.
///
/// # Errors
/// When a memory operand's address is outside the arena.
pub fn execute(cpu: &mut super::Cpu, mem: &mut Arena, op: X87Op, at: u32) -> Result<(), Stop> {
    match op {
        // Constants and unit housekeeping.
        X87Op::Fnop => {}
        X87Op::Fchs => cpu.fpu.set_st(0, -cpu.fpu.st(0)),
        X87Op::Fabs => cpu.fpu.set_st(0, cpu.fpu.st(0).abs()),
        X87Op::Ftst => {
            let value = cpu.fpu.st(0);
            cpu.fpu.compare(value, 0.0);
        }
        X87Op::Fxam => cpu.fpu.examine(),
        X87Op::Fld1 => cpu.fpu.push(1.0),
        X87Op::Fldl2t => cpu.fpu.push(std::f64::consts::LOG2_10),
        X87Op::Fldl2e => cpu.fpu.push(std::f64::consts::LOG2_E),
        X87Op::Fldpi => cpu.fpu.push(maths::PI),
        X87Op::Fldlg2 => cpu.fpu.push(std::f64::consts::LOG10_2),
        X87Op::Fldln2 => cpu.fpu.push(maths::LN_2),
        X87Op::Fldz => cpu.fpu.push(0.0),

        // Transcendentals and stack maintenance.
        X87Op::F2xm1 => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::pow2(x) - 1.0, x, x));
        }
        X87Op::Fyl2x => {
            let x = cpu.fpu.pop();
            let y = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(y * maths::log2(x), x, y));
        }
        X87Op::Fptan => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::tan(x), x, x));
            cpu.fpu.push(1.0);
        }
        X87Op::Fpatan => {
            let x = cpu.fpu.pop();
            let y = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::atan2(y, x), x, y));
        }
        X87Op::Fxtract => {
            let value = cpu.fpu.st(0);
            let exponent = if value == 0.0 {
                0.0
            } else {
                maths::log2(value.abs()).floor()
            };
            let significand = if value == 0.0 {
                0.0
            } else {
                answer(value / exp2(exponent as i32), value, value)
            };
            cpu.fpu.set_st(0, exponent);
            cpu.fpu.push(significand);
        }
        X87Op::Fprem1 => cpu.fpu.remainder(true),
        X87Op::Fdecstp => cpu.fpu.decrement_top(),
        X87Op::Fincstp => cpu.fpu.increment_top(),
        X87Op::Fprem => cpu.fpu.remainder(false),
        X87Op::Fyl2xp1 => {
            let x = cpu.fpu.pop();
            let y = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(y * maths::log2(x + 1.0), x, y));
        }
        X87Op::Fsqrt => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(x.sqrt(), x, x));
        }
        X87Op::Fsincos => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::sin(x), x, x));
            cpu.fpu.push(answer(maths::cos(x), x, x));
        }
        X87Op::Frndint => {
            let x = cpu.fpu.st(0);
            let rounded = cpu.fpu.to_whole(x);
            cpu.fpu.set_st(0, rounded);
        }
        X87Op::Fscale => {
            let scale = cpu.fpu.st(1).trunc();
            let value = cpu.fpu.st(0);
            cpu.fpu
                .set_st(0, answer(value * exp2(scale as i32), value, scale));
        }
        X87Op::Fsin => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::sin(x), x, x));
        }
        X87Op::Fcos => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::cos(x), x, x));
        }
        X87Op::Fnclex => cpu.fpu.clear_exceptions(),
        X87Op::Fninit => cpu.fpu.init(),
        X87Op::Fcompp => {
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(1));
            cpu.fpu.compare(a, b);
            cpu.fpu.pop();
            cpu.fpu.pop();
        }
        X87Op::FnstswAx => {
            let status = u32::from(cpu.fpu.status());
            cpu.r[super::EAX] = (cpu.r[super::EAX] & 0xffff_0000) | status;
        }

        // Parameterized register forms.
        X87Op::FldSt(i) => {
            let value = cpu.fpu.st(usize::from(i));
            cpu.fpu.push(value);
        }
        X87Op::Fxch(i) => {
            let idx = usize::from(i);
            let a = cpu.fpu.st(0);
            let b = cpu.fpu.st(idx);
            cpu.fpu.set_st(0, b);
            cpu.fpu.set_st(idx, a);
        }
        X87Op::Ffree(i) => cpu.fpu.free(usize::from(i)),
        X87Op::FstSt(i) => {
            let value = cpu.fpu.st(0);
            cpu.fpu.set_st(usize::from(i), value);
        }
        X87Op::FstpSt(i) => {
            let value = cpu.fpu.st(0);
            cpu.fpu.set_st(usize::from(i), value);
            cpu.fpu.pop();
        }
        X87Op::FcomSt(i) => {
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(usize::from(i)));
            cpu.fpu.compare(a, b);
        }
        X87Op::FcompSt(i) => {
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(usize::from(i)));
            cpu.fpu.compare(a, b);
            cpu.fpu.pop();
        }
        X87Op::Fcomi(i) => {
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(usize::from(i)));
            let (zero, parity, carry) = X87::compare_flags(a, b);
            cpu.flags.zf = zero;
            cpu.flags.pf = parity;
            cpu.flags.cf = carry;
            cpu.flags.of = false;
            cpu.flags.sf = false;
        }
        X87Op::Fcomip(i) => {
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(usize::from(i)));
            let (zero, parity, carry) = X87::compare_flags(a, b);
            cpu.flags.zf = zero;
            cpu.flags.pf = parity;
            cpu.flags.cf = carry;
            cpu.flags.of = false;
            cpu.flags.sf = false;
            cpu.fpu.pop();
        }
        X87Op::FaddSt0(i) => reg_binop(&mut cpu.fpu, usize::from(i), true, false, |a, b| a + b),
        X87Op::FmulSt0(i) => reg_binop(&mut cpu.fpu, usize::from(i), true, false, |a, b| a * b),
        X87Op::FsubSt0(i) => reg_binop(&mut cpu.fpu, usize::from(i), true, false, |a, b| a - b),
        X87Op::FsubrSt0(i) => reg_binop(&mut cpu.fpu, usize::from(i), true, false, |a, b| b - a),
        X87Op::FdivSt0(i) => reg_binop(&mut cpu.fpu, usize::from(i), true, false, |a, b| a / b),
        X87Op::FdivrSt0(i) => reg_binop(&mut cpu.fpu, usize::from(i), true, false, |a, b| b / a),
        X87Op::FaddSti(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, false, |a, b| a + b),
        X87Op::FmulSti(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, false, |a, b| a * b),
        X87Op::FsubSti(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, false, |a, b| a - b),
        X87Op::FsubrSti(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, false, |a, b| b - a),
        X87Op::FdivSti(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, false, |a, b| a / b),
        X87Op::FdivrSti(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, false, |a, b| b / a),
        X87Op::Faddp(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, true, |a, b| a + b),
        X87Op::Fmulp(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, true, |a, b| a * b),
        X87Op::Fsubp(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, true, |a, b| a - b),
        X87Op::Fsubrp(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, true, |a, b| b - a),
        X87Op::Fdivp(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, true, |a, b| a / b),
        X87Op::Fdivrp(i) => reg_binop(&mut cpu.fpu, usize::from(i), false, true, |a, b| b / a),

        // Memory arithmetic and comparisons.
        X87Op::FaddF32 => mem_binop(&mut cpu.fpu, load_f32(mem, at)?, |a, b| a + b),
        X87Op::FmulF32 => mem_binop(&mut cpu.fpu, load_f32(mem, at)?, |a, b| a * b),
        X87Op::FcomF32 => {
            let v = load_f32(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
        }
        X87Op::FcompF32 => {
            let v = load_f32(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
            cpu.fpu.pop();
        }
        X87Op::FsubF32 => mem_binop(&mut cpu.fpu, load_f32(mem, at)?, |a, b| a - b),
        X87Op::FsubrF32 => mem_binop(&mut cpu.fpu, load_f32(mem, at)?, |a, b| b - a),
        X87Op::FdivF32 => mem_binop(&mut cpu.fpu, load_f32(mem, at)?, |a, b| a / b),
        X87Op::FdivrF32 => mem_binop(&mut cpu.fpu, load_f32(mem, at)?, |a, b| b / a),

        X87Op::FaddF64 => mem_binop(&mut cpu.fpu, load_f64(mem, at)?, |a, b| a + b),
        X87Op::FmulF64 => mem_binop(&mut cpu.fpu, load_f64(mem, at)?, |a, b| a * b),
        X87Op::FcomF64 => {
            let v = load_f64(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
        }
        X87Op::FcompF64 => {
            let v = load_f64(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
            cpu.fpu.pop();
        }
        X87Op::FsubF64 => mem_binop(&mut cpu.fpu, load_f64(mem, at)?, |a, b| a - b),
        X87Op::FsubrF64 => mem_binop(&mut cpu.fpu, load_f64(mem, at)?, |a, b| b - a),
        X87Op::FdivF64 => mem_binop(&mut cpu.fpu, load_f64(mem, at)?, |a, b| a / b),
        X87Op::FdivrF64 => mem_binop(&mut cpu.fpu, load_f64(mem, at)?, |a, b| b / a),

        X87Op::FiaddI32 => mem_binop(&mut cpu.fpu, load_i32(mem, at)?, |a, b| a + b),
        X87Op::FimulI32 => mem_binop(&mut cpu.fpu, load_i32(mem, at)?, |a, b| a * b),
        X87Op::FicomI32 => {
            let v = load_i32(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
        }
        X87Op::FicompI32 => {
            let v = load_i32(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
            cpu.fpu.pop();
        }
        X87Op::FisubI32 => mem_binop(&mut cpu.fpu, load_i32(mem, at)?, |a, b| a - b),
        X87Op::FisubrI32 => mem_binop(&mut cpu.fpu, load_i32(mem, at)?, |a, b| b - a),
        X87Op::FidivI32 => mem_binop(&mut cpu.fpu, load_i32(mem, at)?, |a, b| a / b),
        X87Op::FidivrI32 => mem_binop(&mut cpu.fpu, load_i32(mem, at)?, |a, b| b / a),

        X87Op::FiaddI16 => mem_binop(&mut cpu.fpu, load_i16(mem, at)?, |a, b| a + b),
        X87Op::FimulI16 => mem_binop(&mut cpu.fpu, load_i16(mem, at)?, |a, b| a * b),
        X87Op::FicomI16 => {
            let v = load_i16(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
        }
        X87Op::FicompI16 => {
            let v = load_i16(mem, at)?;
            let top = cpu.fpu.st(0);
            cpu.fpu.compare(top, v);
            cpu.fpu.pop();
        }
        X87Op::FisubI16 => mem_binop(&mut cpu.fpu, load_i16(mem, at)?, |a, b| a - b),
        X87Op::FisubrI16 => mem_binop(&mut cpu.fpu, load_i16(mem, at)?, |a, b| b - a),
        X87Op::FidivI16 => mem_binop(&mut cpu.fpu, load_i16(mem, at)?, |a, b| a / b),
        X87Op::FidivrI16 => mem_binop(&mut cpu.fpu, load_i16(mem, at)?, |a, b| b / a),

        // Memory loads and stores.
        X87Op::FldF32 => {
            let value = load_f32(mem, at)?;
            cpu.fpu.push(value);
        }
        X87Op::FldF64 => {
            let value = load_f64(mem, at)?;
            cpu.fpu.push(value);
        }
        X87Op::FildI32 => {
            let value = load_i32(mem, at)?;
            cpu.fpu.push(value);
        }
        X87Op::FildI16 => {
            let value = load_i16(mem, at)?;
            cpu.fpu.push(value);
        }
        X87Op::FldF80 => {
            let value = read_extended(mem, at)?;
            cpu.fpu.push(value);
        }
        X87Op::FildI64 => {
            let raw = mem.read_u64(at).map_err(Stop::Fault)? as i64;
            cpu.fpu.push(raw as f64);
        }
        X87Op::FstF32 => {
            let value = cpu.fpu.st(0);
            write(mem, at, Format::Single, value)?;
        }
        X87Op::FstpF32 => {
            let value = cpu.fpu.st(0);
            write(mem, at, Format::Single, value)?;
            cpu.fpu.pop();
        }
        X87Op::FstF64 => {
            let value = cpu.fpu.st(0);
            write(mem, at, Format::Double, value)?;
        }
        X87Op::FstpF64 => {
            let value = cpu.fpu.st(0);
            write(mem, at, Format::Double, value)?;
            cpu.fpu.pop();
        }
        X87Op::FstpF80 => {
            let value = cpu.fpu.st(0);
            write_extended(mem, at, value)?;
            cpu.fpu.pop();
        }
        X87Op::FistI32 => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0));
            mem.write_u32(at, value as i32 as u32)
                .map_err(Stop::Fault)?;
        }
        X87Op::FistpI32 => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0));
            mem.write_u32(at, value as i32 as u32)
                .map_err(Stop::Fault)?;
            cpu.fpu.pop();
        }
        X87Op::FistI16 => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0));
            mem.write_u16(at, value as i32 as i16 as u16)
                .map_err(Stop::Fault)?;
        }
        X87Op::FistpI16 => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0));
            mem.write_u16(at, value as i32 as i16 as u16)
                .map_err(Stop::Fault)?;
            cpu.fpu.pop();
        }
        X87Op::FistpI64 => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0)) as i64;
            mem.write_u64(at, value as u64).map_err(Stop::Fault)?;
            cpu.fpu.pop();
        }

        // Control, status, and environment words.
        X87Op::Fldcw | X87Op::Fldenv => {
            let word = mem.read_u16(at).map_err(Stop::Fault)?;
            cpu.fpu.set_control(word);
        }
        X87Op::Fnstcw => {
            let word = cpu.fpu.control();
            mem.write_u16(at, word).map_err(Stop::Fault)?;
        }
        X87Op::FnstswMem => {
            let word = cpu.fpu.status();
            mem.write_u16(at, word).map_err(Stop::Fault)?;
        }
        X87Op::Fnstenv => {
            let control = cpu.fpu.control();
            let status = cpu.fpu.status();
            mem.write_u16(at, control).map_err(Stop::Fault)?;
            mem.write_u16(at + 4, status).map_err(Stop::Fault)?;
            mem.write(at + 8, &[0u8; 20]).map_err(Stop::Fault)?;
        }
    }
    Ok(())
}

/// Round to a whole number, ties going to the even one: the control word's
/// default rounding.
#[must_use]
#[inline(always)]
pub fn round_half_to_even(x: f64) -> f64 {
    // IEEE-754 `round_ties_even` matches the previous `round`/`trunc`/`% 2.0`
    // formula bit-for-bit on every non-NaN input except `-0.5`, where IEEE
    // produces `-0.0` while `-1.0 - (-1.0)` produced `+0.0`. Preserve `+0.0`
    // for `-0.5` and route any NaN through `answer` so signaling NaNs are
    // quieted identically on every host and target (baseline x86_64 Linux
    // `round_ties_even` otherwise leaves them unquieted).
    if x == -0.5 {
        0.0
    } else if x.is_nan() {
        answer(x, x, x)
    } else {
        x.round_ties_even()
    }
}

#[cfg(test)]
mod tests {
    use super::{answer, maths, INDEFINITE, X87};

    #[test]
    fn a_remainder_is_complete_in_one_step_and_says_its_quotient() {
        let mut fpu = X87::new();
        fpu.push(2.0);
        fpu.push(7.0);
        // A normal number examined leaves C2 set, as a guest's loop may find it.
        fpu.examine();
        fpu.remainder(false);
        assert_eq!(fpu.st(0), 1.0);
        // Quotient three: C3 and C1, and C2 clear so a guest's loop ends.
        assert_eq!(fpu.status() & 0x4700, 0x4200);
        fpu.set_st(0, 7.0);
        fpu.remainder(true);
        // To nearest, seven is four twos less one: quotient four, C0.
        assert_eq!((fpu.st(0), fpu.status() & 0x4700), (-1.0, 0x0100));
        fpu.set_st(0, f64::INFINITY);
        fpu.remainder(false);
        assert!(fpu.st(0).is_nan());
        assert_eq!(fpu.status() & 0x0400, 0, "nothing more to reduce");
    }

    /// Close enough that a double cannot tell the difference that matters.
    fn near(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1.0)
    }

    #[test]
    fn the_stack_pushes_and_pops_in_the_order_the_hardware_does() {
        let mut fpu = X87::new();
        fpu.push(1.0);
        fpu.push(2.0);
        assert_eq!(fpu.st(0), 2.0);
        assert_eq!(fpu.st(1), 1.0);
        assert!(fpu.occupied(0) && fpu.occupied(1) && !fpu.occupied(2));
        assert_eq!(fpu.pop(), 2.0);
        assert_eq!(fpu.st(0), 1.0);
    }

    #[test]
    fn the_status_word_carries_the_stack_top_where_the_guest_reads_it() {
        let mut fpu = X87::new();
        assert_eq!(fpu.status() >> 11 & 7, 0);
        fpu.push(1.0);
        assert_eq!(fpu.status() >> 11 & 7, 7, "a push moves the top down one");
    }

    #[test]
    fn a_comparison_spells_its_answer_in_the_condition_codes() {
        let mut fpu = X87::new();
        fpu.compare(2.0, 1.0);
        assert_eq!(fpu.status() & 0x4500, 0, "greater sets nothing");
        fpu.compare(1.0, 2.0);
        assert_eq!(fpu.status() & 0x4500, 0x0100, "less sets C0");
        fpu.compare(1.0, 1.0);
        assert_eq!(fpu.status() & 0x4500, 0x4000, "equal sets C3");
        fpu.compare(f64::NAN, 1.0);
        assert_eq!(fpu.status() & 0x4500, 0x4500, "unordered sets all three");
    }
    #[test]
    fn examining_the_top_names_its_kind_and_sign() {
        let mut fpu = X87::new();
        let kind = |fpu: &X87| fpu.status() & 0x4700;
        fpu.examine();
        assert_eq!(kind(&fpu), 0x4100, "an empty register is C3 and C0");
        fpu.push(-2.5);
        fpu.examine();
        assert_eq!(kind(&fpu), 0x0600, "a negative normal number is C2 and C1");
        fpu.push(0.0);
        fpu.examine();
        assert_eq!(kind(&fpu), 0x4000, "zero is C3");
        fpu.push(f64::INFINITY);
        fpu.examine();
        assert_eq!(kind(&fpu), 0x0500, "infinity is C2 and C0");
        fpu.push(f64::NAN);
        fpu.examine();
        assert_eq!(kind(&fpu) & 0x4500, 0x0100, "a NaN is C0");
    }

    #[test]
    fn a_nan_is_the_units_own_on_every_host() {
        let bits = |x: f64| x.to_bits();
        let zero = std::hint::black_box(0.0f64);
        assert_eq!(bits(answer(zero / zero, zero, zero)), bits(INDEFINITE));
        assert_eq!(
            bits(answer(f64::INFINITY - f64::INFINITY, 1.0, 1.0)),
            bits(INDEFINITE)
        );
        // A NaN operand comes through, quieted, whatever the host made.
        let signalling = f64::from_bits(0x7ff0_0000_0000_0001);
        assert_eq!(
            bits(answer(f64::NAN, signalling, 1.0)),
            0x7ff8_0000_0000_0001
        );
        let larger = f64::from_bits(0xfff8_0000_0000_0002);
        assert_eq!(bits(answer(f64::NAN, signalling, larger)), bits(larger));
        assert_eq!(answer(1.5, 1.0, 0.5), 1.5);
    }

    #[test]
    fn single_precision_rounds_what_it_stores() {
        let mut fpu = X87::new();
        // Precision control 0 is single.
        fpu.set_control(0x037f & !0x0300);
        fpu.push(0.1);
        assert_eq!(
            fpu.st(0),
            f64::from(0.1f32),
            "a single-precision store rounds"
        );
    }

    #[test]
    fn sine_and_cosine_agree_with_their_own_identities() {
        for x in [0.0, 0.25, 1.0, -1.0, 3.0, 7.5, -12.25, 100.0] {
            let (s, c) = (maths::sin(x), maths::cos(x));
            assert!(
                near(s * s + c * c, 1.0, 1e-12),
                "sin^2 + cos^2 at {x} was {}",
                s * s + c * c
            );
        }
        assert!(near(maths::sin(maths::PI / 6.0), 0.5, 1e-14));
        assert!(near(maths::cos(maths::PI / 3.0), 0.5, 1e-14));
        assert!(maths::sin(maths::PI).abs() < 1e-15);
        assert!(near(maths::cos(0.0), 1.0, 1e-15));
    }

    #[test]
    fn the_arc_tangent_turns_the_tangent_back_into_its_angle() {
        // Only where the tangent is well conditioned: near a quarter turn it
        // magnifies any error in the angle, so a round trip there measures the
        // tangent's conditioning rather than the arc tangent's accuracy.
        for x in [0.0, 0.1, 0.5, 1.0, 2.0, 10.0, -0.3, -4.0] {
            let back = maths::tan(maths::atan(x));
            assert!(
                near(back, x, 1e-11),
                "atan then tan of {x} came back {back}"
            );
        }
        // Large arguments are checked by an identity that stays conditioned:
        // the angle and its reciprocal's angle make a quarter turn between
        // them.
        for x in [1e3, 1e6, 1e12] {
            let sum = maths::atan(x) + maths::atan(1.0 / x);
            assert!(
                near(sum, maths::PI / 2.0, 1e-14),
                "atan({x}) + atan(1/{x}) was {sum}"
            );
        }
        assert!(near(maths::atan(1.0), maths::PI / 4.0, 1e-14));
        assert!(near(maths::atan2(1.0, -1.0), 3.0 * maths::PI / 4.0, 1e-14));
    }

    #[test]
    fn the_logarithm_and_the_power_undo_each_other() {
        for x in [1.0, 2.0, 8.0, 0.5, 1e-8, 1234.567, 1e12] {
            assert!(
                near(maths::pow2(maths::log2(x)), x, 1e-12),
                "round trip of {x}"
            );
        }
        assert_eq!(maths::log2(1.0), 0.0);
        assert!(near(maths::log2(1024.0), 10.0, 1e-14));
        assert!(near(maths::pow2(0.5), std::f64::consts::SQRT_2, 1e-14));
        assert!(near(maths::pow2(10.0), 1024.0, 1e-14));
    }

    #[test]
    fn x87_op_and_kind_stay_compact() {
        assert!(
            std::mem::size_of::<super::X87Op>() <= 2,
            "X87Op grew past 2 bytes: {}",
            std::mem::size_of::<super::X87Op>()
        );
        assert!(
            std::mem::size_of::<super::super::insn::Kind>() <= 3,
            "Kind grew past 3 bytes: {}",
            std::mem::size_of::<super::super::insn::Kind>()
        );
    }

    #[test]
    fn round_half_to_even_matches_reference_bit_for_bit() {
        fn legacy(x: f64) -> f64 {
            let rounded = x.round();
            if (x - x.trunc()).abs() == 0.5 && rounded % 2.0 != 0.0 {
                rounded - x.signum()
            } else {
                rounded
            }
        }

        let cases = [
            0.0_f64,
            -0.0,
            0.25,
            -0.25,
            0.49999999999999994,
            -0.49999999999999994,
            0.5,
            -0.5,
            0.5000000000000001,
            -0.5000000000000001,
            1.0,
            -1.0,
            1.5,
            -1.5,
            2.5,
            -2.5,
            3.5,
            -3.5,
            4.5,
            -4.5,
            (1u64 << 51) as f64 + 0.5,
            -((1u64 << 51) as f64 + 0.5),
            (1u64 << 51) as f64 + 1.5,
            -((1u64 << 51) as f64 + 1.5),
            (1u64 << 52) as f64,
            -((1u64 << 52) as f64),
            (1u64 << 52) as f64 + 1.0,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::from_bits(1),
            f64::from_bits(0x8000_0000_0000_0001),
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];

        for x in cases {
            let x = std::hint::black_box(x);
            assert_eq!(
                super::round_half_to_even(x).to_bits(),
                legacy(x).to_bits(),
                "bit mismatch for {x:?} ({:#018x})",
                x.to_bits()
            );
        }

        // NaNs follow x87's own rule (`answer`): quiet NaNs stay bit-exact and
        // signaling NaNs have their quiet bit set with sign and payload kept,
        // independent of host libm (`round`/`round_ties_even`/`floor`/`ceil`/`trunc`).
        let nan_cases = [
            (f64::NAN, f64::NAN.to_bits()),
            (INDEFINITE, INDEFINITE.to_bits()),
            (
                f64::from_bits(0x7ff0_0000_0000_0001),
                0x7ff8_0000_0000_0001u64,
            ),
            (
                f64::from_bits(0xfff0_0000_0000_0007),
                0xfff8_0000_0000_0007u64,
            ),
        ];

        for (input, expected) in nan_cases {
            let x = std::hint::black_box(input);
            assert_eq!(
                super::round_half_to_even(x).to_bits(),
                expected,
                "NaN bit mismatch in round_half_to_even for {:#018x}",
                input.to_bits()
            );
            for rc in 0u16..4 {
                let mut fpu = X87::new();
                fpu.set_control((fpu.control() & !0x0c00) | (rc << 10));
                assert_eq!(
                    fpu.to_whole(x).to_bits(),
                    expected,
                    "NaN bit mismatch in to_whole(rc={rc}) for {:#018x}",
                    input.to_bits()
                );
            }
        }
    }

    #[test]
    fn double_and_extended_io_use_exact_widths_and_fault_at_arena_end() {
        use crate::guest::{Arena, Fault};
        use crate::host::headless::Headless;

        let len = 1 << 14;
        let end = u32::try_from(len).unwrap();
        let mut mem = Arena::new(&mut Headless::new(), len).unwrap();

        // Unaligned double crossing a 4 KiB code page boundary.
        let cross = 4093u32;
        mem.mark_code(4096);
        let val = -123_456.789_012_345_f64;
        super::write(&mut mem, cross, super::Format::Double, val).unwrap();
        assert!(mem.code_written);
        assert_eq!(
            super::read(&mem, cross, super::Format::Double)
                .unwrap()
                .to_bits(),
            val.to_bits()
        );

        // Single 8-byte bounds check faults with len 8 when only 4 bytes remain.
        assert_eq!(
            super::read(&mem, end - 4, super::Format::Double),
            Err(super::Stop::Fault(Fault {
                addr: end - 4,
                len: 8,
                write: false,
            }))
        );
        assert_eq!(
            super::write(&mut mem, end - 4, super::Format::Double, val),
            Err(super::Stop::Fault(Fault {
                addr: end - 4,
                len: 8,
                write: true,
            }))
        );

        // Ten-byte extended round trip at the exact end of the arena.
        super::write_extended(&mut mem, end - 10, 3.25).unwrap();
        assert_eq!(super::read_extended(&mem, end - 10).unwrap(), 3.25);
    }

    #[test]
    fn x87_predecode_covers_exact_register_and_memory_forms() {
        // Exact precedence: (0xda, 0xe9) is fucompp, not fcomi(1).
        assert_eq!(super::decode_reg(0xda, 0xe9), Ok(super::X87Op::Fcompp));
        assert_eq!(super::decode_reg(0xda, 0xe8), Ok(super::X87Op::Fcomi(0)));
        assert_eq!(super::decode_reg(0xde, 0xd9), Ok(super::X87Op::Fcompp));
        assert!(super::decode_reg(0xde, 0xd8).is_err());
        assert!(super::decode_reg(0xdb, 0xe4).is_err());
        assert!(super::decode_mem(0xd9, 1).is_err());
        assert_eq!(super::decode_mem(0xdd, 0), Ok(super::X87Op::FldF64));
        assert_eq!(super::decode_mem(0xdd, 3), Ok(super::X87Op::FstpF64));

        // Exhaustive check over all 8 * 256 (escape, modrm) pairs to verify
        // pre-decoding accepts and rejects the exact same set as the legacy
        // runtime decoder.
        for escape in 0xd8u8..=0xdf {
            for modrm in 0u8..=0xff {
                let reg = (modrm >> 3) & 7;
                if modrm >> 6 == 3 {
                    let expected = matches!(
                        (escape, modrm),
                        (0xd9, 0xd0 | 0xe0 | 0xe1 | 0xe4 | 0xe5 | 0xe8..=0xee | 0xf0..=0xff)
                            | (0xdb, 0xe2 | 0xe3)
                            | (0xde, 0xd9)
                            | (0xda, 0xe9)
                            | (0xdf, 0xe0)
                    ) || matches!(
                        (escape, reg),
                        (0xd9, 0 | 1)
                            | (0xdd, 0 | 2 | 3 | 4 | 5)
                            | (0xda | 0xdb | 0xdf, 5 | 6)
                            | (0xd8, 0..=7)
                            | (0xdc | 0xde, 0 | 1 | 4 | 5 | 6 | 7)
                    );
                    assert_eq!(
                        super::decode_reg(escape, modrm).is_ok(),
                        expected,
                        "decode_reg({escape:#04x}, {modrm:#04x})"
                    );
                } else {
                    let expected = matches!(
                        (escape, reg),
                        (0xd8 | 0xdc | 0xda | 0xde, 0..=7)
                            | (0xd9, 0 | 2 | 3 | 4 | 5 | 6 | 7)
                            | (0xdd, 0 | 2 | 3 | 7)
                            | (0xdb, 0 | 2 | 3 | 5 | 7)
                            | (0xdf, 0 | 2 | 3 | 5 | 7)
                    );
                    assert_eq!(
                        super::decode_mem(escape, reg).is_ok(),
                        expected,
                        "decode_mem({escape:#04x}, /{reg})"
                    );
                }
            }
        }
    }
}
