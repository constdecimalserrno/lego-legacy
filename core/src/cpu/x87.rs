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
    pub fn to_whole(&self, x: f64) -> f64 {
        match (self.control >> 10) & 3 {
            0 => round_half_to_even(x),
            1 => x.floor(),
            2 => x.ceil(),
            _ => x.trunc(),
        }
    }

    /// Round a result to the precision the guest asked for.
    fn round(&self, value: f64) -> f64 {
        match self.precision() {
            Precision::Single => f64::from(value as f32),
            _ => value,
        }
    }

    fn physical(&self, i: usize) -> usize {
        (self.top + i) % DEPTH
    }

    /// Read `st(i)`.
    #[must_use]
    pub fn st(&self, i: usize) -> f64 {
        self.regs[self.physical(i)]
    }

    /// Write `st(i)`.
    pub fn set_st(&mut self, i: usize, value: f64) {
        let at = self.physical(i);
        self.regs[at] = self.round(value);
        self.used[at] = true;
    }

    /// Push a value, which becomes `st(0)`.
    pub fn push(&mut self, value: f64) {
        self.top = (self.top + DEPTH - 1) % DEPTH;
        self.regs[self.top] = self.round(value);
        self.used[self.top] = true;
    }

    /// Take `st(0)` off the stack.
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
pub fn read(mem: &Arena, addr: u32, format: Format) -> Result<f64, Stop> {
    Ok(match format {
        Format::Single => {
            let bits = mem.read_u32(addr).map_err(Stop::Fault)?;
            f64::from(f32::from_bits(bits))
        }
        Format::Double => {
            let lo = mem.read_u32(addr).map_err(Stop::Fault)?;
            let hi = mem.read_u32(addr + 4).map_err(Stop::Fault)?;
            f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
        }
    })
}

/// Write a floating-point value in one of the three memory formats.
///
/// # Errors
/// When the address is outside the arena.
pub fn write(mem: &mut Arena, addr: u32, format: Format, value: f64) -> Result<(), Stop> {
    match format {
        Format::Single => mem
            .write_u32(addr, (value as f32).to_bits())
            .map_err(Stop::Fault),
        Format::Double => {
            let bits = value.to_bits();
            mem.write_u32(addr, bits as u32).map_err(Stop::Fault)?;
            mem.write_u32(addr + 4, (bits >> 32) as u32)
                .map_err(Stop::Fault)
        }
    }
}

/// Read the ten-byte extended format the hardware keeps its registers in.
///
/// # Errors
/// When the address is outside the arena.
pub fn read_extended(mem: &Arena, addr: u32) -> Result<f64, Stop> {
    let significand = {
        let lo = mem.read_u32(addr).map_err(Stop::Fault)?;
        let hi = mem.read_u32(addr + 4).map_err(Stop::Fault)?;
        (u64::from(hi) << 32) | u64::from(lo)
    };
    let tail = mem.read_u32(addr + 8).map_err(Stop::Fault)? as u16;
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
        mem.write_u32(addr, 0).map_err(Stop::Fault)?;
        mem.write_u32(addr + 4, 0).map_err(Stop::Fault)?;
        mem.write(addr + 8, &sign.to_le_bytes())
            .map_err(Stop::Fault)?;
        return Ok(());
    }
    // The exponent is already written down in the value's own bits, so it is
    // read rather than computed: no logarithm, and no rounding on the way.
    let exponent = ((magnitude.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let mantissa = magnitude / exp2(exponent);
    let significand = (mantissa * (1u64 << 63) as f64) as u64;
    mem.write_u32(addr, significand as u32)
        .map_err(Stop::Fault)?;
    mem.write_u32(addr + 4, (significand >> 32) as u32)
        .map_err(Stop::Fault)?;
    let tail = sign | u16::try_from(exponent + 16383).unwrap_or(0x7fff);
    mem.write(addr + 8, &tail.to_le_bytes())
        .map_err(Stop::Fault)?;
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

/// Run one floating-point escape.
///
/// The escape byte and the ModRM byte together say what the unit does. A form
/// this unit has not been taught stops the run and names both, rather than
/// leaving the stack in a state the guest will misread later.
///
/// # Errors
/// When the form is not implemented, or an address is outside the arena.
pub fn execute(
    cpu: &mut super::Cpu,
    mem: &mut Arena,
    escape: u8,
    modrm: u8,
    address: Option<u32>,
) -> Result<(), Stop> {
    let reg = (modrm >> 3) & 7;
    let index = (modrm & 7) as usize;
    let register_form = modrm >> 6 == 3;

    let wall = |why: &'static str| {
        Err(Stop::Undecoded {
            eip: 0,
            byte: escape,
            group: Some(reg),
            why,
        })
    };

    if !register_form {
        let Some(at) = address else {
            return wall("a floating-point escape with no address");
        };
        return memory_form(cpu, mem, escape, reg, at);
    }

    match (escape, modrm) {
        // The constants, and the unit's own housekeeping.
        (0xd9, 0xd0) => {}                                      // fnop
        (0xd9, 0xe0) => cpu.fpu.set_st(0, -cpu.fpu.st(0)),      // fchs
        (0xd9, 0xe1) => cpu.fpu.set_st(0, cpu.fpu.st(0).abs()), // fabs
        (0xd9, 0xe4) => {
            let value = cpu.fpu.st(0);
            cpu.fpu.compare(value, 0.0);
        }
        (0xd9, 0xe5) => cpu.fpu.examine(),
        (0xd9, 0xe8) => cpu.fpu.push(1.0),
        (0xd9, 0xe9) => cpu.fpu.push(std::f64::consts::LOG2_10),
        (0xd9, 0xea) => cpu.fpu.push(std::f64::consts::LOG2_E),
        (0xd9, 0xeb) => cpu.fpu.push(maths::PI),
        (0xd9, 0xec) => cpu.fpu.push(std::f64::consts::LOG10_2),
        (0xd9, 0xed) => cpu.fpu.push(maths::LN_2),
        (0xd9, 0xee) => cpu.fpu.push(0.0),
        // The transcendentals.
        (0xd9, 0xf0) => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::pow2(x) - 1.0, x, x));
        }
        (0xd9, 0xf1) => {
            // y * log2(x), with x on top.
            let x = cpu.fpu.pop();
            let y = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(y * maths::log2(x), x, y));
        }
        (0xd9, 0xf2) => {
            // The tangent, and a one pushed after it.
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::tan(x), x, x));
            cpu.fpu.push(1.0);
        }
        (0xd9, 0xf3) => {
            let x = cpu.fpu.pop();
            let y = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::atan2(y, x), x, y));
        }
        (0xd9, 0xf4) => {
            // Split into exponent and significand.
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
        (0xd9, 0xf5) => cpu.fpu.remainder(true),
        (0xd9, 0xf8) => cpu.fpu.remainder(false),
        (0xd9, 0xf6) => cpu.fpu.decrement_top(),
        (0xd9, 0xf7) => cpu.fpu.increment_top(),
        (0xd9, 0xf9) => {
            // y * log2(x + 1).
            let x = cpu.fpu.pop();
            let y = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(y * maths::log2(x + 1.0), x, y));
        }
        (0xd9, 0xfa) => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(x.sqrt(), x, x));
        }
        (0xd9, 0xfb) => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::sin(x), x, x));
            cpu.fpu.push(answer(maths::cos(x), x, x));
        }
        (0xd9, 0xfc) => {
            // Round to a whole number, the way the control word says.
            let x = cpu.fpu.st(0);
            let rounded = cpu.fpu.to_whole(x);
            cpu.fpu.set_st(0, rounded);
        }
        (0xd9, 0xfd) => {
            // Scale by two to the power in st(1).
            let scale = cpu.fpu.st(1).trunc();
            let value = cpu.fpu.st(0);
            cpu.fpu
                .set_st(0, answer(value * exp2(scale as i32), value, scale));
        }
        (0xd9, 0xfe) => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::sin(x), x, x));
        }
        (0xd9, 0xff) => {
            let x = cpu.fpu.st(0);
            cpu.fpu.set_st(0, answer(maths::cos(x), x, x));
        }
        (0xdb, 0xe2) => cpu.fpu.clear_exceptions(),
        (0xdb, 0xe3) => cpu.fpu.init(),
        (0xde, 0xd9) => {
            // Compare and pop twice.
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(1));
            cpu.fpu.compare(a, b);
            cpu.fpu.pop();
            cpu.fpu.pop();
        }
        (0xda, 0xe9) => {
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(1));
            cpu.fpu.compare(a, b);
            cpu.fpu.pop();
            cpu.fpu.pop();
        }
        (0xdf, 0xe0) => {
            // The status word goes to ax, which is how a guest reads a
            // comparison it just made.
            let status = u32::from(cpu.fpu.status());
            cpu.r[super::EAX] = (cpu.r[super::EAX] & 0xffff_0000) | status;
        }
        _ => return register_form_op(cpu, escape, reg, index),
    }
    Ok(())
}

/// The forms that name a register: `st(0)` against `st(i)`, either way round.
fn register_form_op(cpu: &mut super::Cpu, escape: u8, reg: u8, index: usize) -> Result<(), Stop> {
    // Whether the answer goes to st(0) or to st(i), and whether the stack is
    // popped afterwards, is what separates the four arithmetic escapes.
    let (to_top, pop_after) = match escape {
        0xd8 => (true, false),
        0xdc => (false, false),
        0xde => (false, true),
        _ => (true, false),
    };

    match (escape, reg) {
        (0xd9, 0) => {
            // fld st(i)
            let value = cpu.fpu.st(index);
            cpu.fpu.push(value);
        }
        (0xd9, 1) => {
            // fxch
            let a = cpu.fpu.st(0);
            let b = cpu.fpu.st(index);
            cpu.fpu.set_st(0, b);
            cpu.fpu.set_st(index, a);
        }
        (0xdd, 0) => cpu.fpu.free(index),
        (0xdd, 2 | 3) => {
            let value = cpu.fpu.st(0);
            cpu.fpu.set_st(index, value);
            if reg == 3 {
                cpu.fpu.pop();
            }
        }
        (0xdd, 4 | 5) | (0xda | 0xdb | 0xdf, 5 | 6) => {
            // The unordered comparisons, and the ones that write the
            // processor's own flags.
            let (a, b) = (cpu.fpu.st(0), cpu.fpu.st(index));
            if escape == 0xdd {
                cpu.fpu.compare(a, b);
                if reg == 5 {
                    cpu.fpu.pop();
                }
            } else {
                let (zero, parity, carry) = X87::compare_flags(a, b);
                cpu.flags.zf = zero;
                cpu.flags.pf = parity;
                cpu.flags.cf = carry;
                cpu.flags.of = false;
                cpu.flags.sf = false;
                if escape == 0xdf {
                    cpu.fpu.pop();
                }
            }
        }
        (0xd8 | 0xdc | 0xde, 0..=7) => {
            let top = cpu.fpu.st(0);
            let other = cpu.fpu.st(index);
            // The comparisons in this block do not write a result back.
            if escape == 0xd8 && (reg == 2 || reg == 3) {
                cpu.fpu.compare(top, other);
                if reg == 3 {
                    cpu.fpu.pop();
                }
                return Ok(());
            }
            // Where the answer goes differs by escape; the operation does not:
            // Intel's tables read the same in st(0) and st(i) for all three,
            // the reverse of what the mnemonics on dc and de suggest.
            let result = match reg {
                0 => top + other,
                1 => top * other,
                4 => top - other,
                5 => other - top,
                6 => top / other,
                7 => other / top,
                _ => {
                    return Err(Stop::Undecoded {
                        eip: 0,
                        byte: escape,
                        group: Some(reg),
                        why: "a floating-point register form this unit does not have",
                    })
                }
            };
            let result = answer(result, top, other);
            if to_top {
                cpu.fpu.set_st(0, result);
            } else {
                cpu.fpu.set_st(index, result);
            }
            if pop_after {
                cpu.fpu.pop();
            }
        }
        _ => {
            return Err(Stop::Undecoded {
                eip: 0,
                byte: escape,
                group: Some(reg),
                why: "a floating-point register form this unit does not have",
            })
        }
    }
    Ok(())
}

/// The forms that name an address.
fn memory_form(
    cpu: &mut super::Cpu,
    mem: &mut Arena,
    escape: u8,
    reg: u8,
    at: u32,
) -> Result<(), Stop> {
    let integer = matches!(escape, 0xda | 0xde | 0xdb | 0xdf);

    // What the escape reads or writes, once the format is known.
    let load = |mem: &Arena| -> Result<f64, Stop> {
        if integer {
            let raw = match escape {
                0xde | 0xdf => i32::from(mem.read_u32(at).map_err(Stop::Fault)? as u16 as i16),
                _ => mem.read_u32(at).map_err(Stop::Fault)? as i32,
            };
            Ok(f64::from(raw))
        } else if matches!(escape, 0xdc | 0xdd) {
            read(mem, at, Format::Double)
        } else {
            read(mem, at, Format::Single)
        }
    };

    match (escape, reg) {
        // The arithmetic escapes, which all take one value from memory.
        (0xd8 | 0xdc | 0xda | 0xde, 0..=7) => {
            let value = load(mem)?;
            let top = cpu.fpu.st(0);
            let result = match reg {
                0 => top + value,
                1 => top * value,
                2 | 3 => {
                    cpu.fpu.compare(top, value);
                    if reg == 3 {
                        cpu.fpu.pop();
                    }
                    return Ok(());
                }
                4 => top - value,
                5 => value - top,
                6 => top / value,
                _ => value / top,
            };
            cpu.fpu.set_st(0, answer(result, top, value));
        }
        // Load and store, in every format.
        (0xd9, 0) => {
            let value = load(mem)?;
            cpu.fpu.push(value);
        }
        (0xdd, 0) => {
            let value = load(mem)?;
            cpu.fpu.push(value);
        }
        (0xdb, 0) | (0xdf, 0) => {
            let value = load(mem)?;
            cpu.fpu.push(value);
        }
        (0xdb, 5) => {
            let value = read_extended(mem, at)?;
            cpu.fpu.push(value);
        }
        (0xdf, 5) => {
            let lo = mem.read_u32(at).map_err(Stop::Fault)?;
            let hi = mem.read_u32(at + 4).map_err(Stop::Fault)?;
            let raw = ((u64::from(hi) << 32) | u64::from(lo)) as i64;
            cpu.fpu.push(raw as f64);
        }
        (0xd9, 2 | 3) => {
            let value = cpu.fpu.st(0);
            write(mem, at, Format::Single, value)?;
            if reg == 3 {
                cpu.fpu.pop();
            }
        }
        (0xdd, 2 | 3) => {
            let value = cpu.fpu.st(0);
            write(mem, at, Format::Double, value)?;
            if reg == 3 {
                cpu.fpu.pop();
            }
        }
        (0xdb, 7) => {
            let value = cpu.fpu.st(0);
            write_extended(mem, at, value)?;
            cpu.fpu.pop();
        }
        // Store as an integer, rounding the way the control word says.
        (0xdb | 0xdf, 2 | 3) => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0));
            if escape == 0xdf {
                let short = value as i32 as i16;
                mem.write(at, &short.to_le_bytes()).map_err(Stop::Fault)?;
            } else {
                mem.write_u32(at, value as i32 as u32)
                    .map_err(Stop::Fault)?;
            }
            if reg == 3 {
                cpu.fpu.pop();
            }
        }
        (0xdf, 7) => {
            let value = cpu.fpu.to_whole(cpu.fpu.st(0)) as i64;
            mem.write_u32(at, value as u32).map_err(Stop::Fault)?;
            mem.write_u32(at + 4, (value >> 32) as u32)
                .map_err(Stop::Fault)?;
            cpu.fpu.pop();
        }
        // The control and status words.
        (0xd9, 5) => {
            let word = mem.read_u32(at).map_err(Stop::Fault)? as u16;
            cpu.fpu.set_control(word);
        }
        (0xd9, 7) => {
            let word = cpu.fpu.control();
            mem.write(at, &word.to_le_bytes()).map_err(Stop::Fault)?;
        }
        (0xdd, 7) => {
            let word = cpu.fpu.status();
            mem.write(at, &word.to_le_bytes()).map_err(Stop::Fault)?;
        }
        // The environment, which this unit writes as its own two words and
        // zeroes for the pointers it does not keep.
        (0xd9, 6) => {
            let control = cpu.fpu.control();
            let status = cpu.fpu.status();
            mem.write(at, &control.to_le_bytes()).map_err(Stop::Fault)?;
            mem.write(at + 4, &status.to_le_bytes())
                .map_err(Stop::Fault)?;
            mem.write(at + 8, &[0u8; 20]).map_err(Stop::Fault)?;
        }
        (0xd9, 4) => {
            let control = mem.read_u32(at).map_err(Stop::Fault)? as u16;
            cpu.fpu.set_control(control);
        }
        _ => {
            return Err(Stop::Undecoded {
                eip: at,
                byte: escape,
                group: Some(reg),
                why: "a floating-point memory form this unit does not have",
            });
        }
    }
    Ok(())
}

/// Round to a whole number, ties going to the even one: the control word's
/// default rounding.
#[must_use]
pub fn round_half_to_even(x: f64) -> f64 {
    let rounded = x.round();
    if (x - x.trunc()).abs() == 0.5 && rounded % 2.0 != 0.0 {
        rounded - x.signum()
    } else {
        rounded
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
}
