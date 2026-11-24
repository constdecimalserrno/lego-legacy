//! The flags register.
//!
//! The engine keeps the six arithmetic flags it computes plus the direction
//! flag, and carries every other bit of `EFLAGS` untouched. Carrying them is
//! what makes a round trip exact: a guest that pushes its flags, changes one,
//! and pops them back must get its own bits returned, not ours.
//!
//! **They are computed eagerly, and that was measured rather than assumed
//! (#69).** Computing a flag only when something reads it is the well-known
//! way round this, and the question is what it is worth here. Measured by
//! building the engine so that it pays for every flag twice — the same run,
//! the same results, the extra wall time being exactly what one flag
//! computation costs — over Alpha Team's menu route, launch to flip 2247:
//!
//! | | wall, median of three | instructions a second |
//! |---|---|---|
//! | as it is | 47.8 s | 106 M |
//! | paying twice for all six | 50.2 s | 101 M |
//! | paying twice for parity and auxiliary alone | 48.5 s | 104 M |
//!
//! So **every flag on every arithmetic instruction costs about 5% of the
//! run**, and parity and auxiliary — the two almost nothing ever reads — are
//! about 2.3% of it. That 5% is the whole ceiling for a lazy scheme, and a
//! lazy scheme does not reach it: it has to store the kind, the width and the
//! operands on every flag-writing instruction, and compute on demand for the
//! ones that are read. Against that, 74 places outside this file touch
//! `flags` — `adc`, `sbb`, `inc`, `dec`, every shift and rotate, `bt`, the
//! decimal adjusts, x87's `fnstsw` — and each has to compose correctly with
//! whatever was left lazy before it.
//!
//! So the flags stay eager: five percent at most is not worth that, and it is
//! not where the time at that menu goes (#65).

use super::insn::Width;

pub const CF: u32 = 1 << 0;
pub const PF: u32 = 1 << 2;
pub const AF: u32 = 1 << 4;
pub const ZF: u32 = 1 << 6;
pub const SF: u32 = 1 << 7;
pub const DF: u32 = 1 << 10;
pub const OF: u32 = 1 << 11;
/// Bit 1 reads as one on every x86 that has ever shipped.
pub const ALWAYS_ONE: u32 = 1 << 1;

const OURS: u32 = CF | PF | AF | ZF | SF | DF | OF;

/// The flags this engine computes, and everything else the guest set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    pub cf: bool,
    pub pf: bool,
    pub af: bool,
    pub zf: bool,
    pub sf: bool,
    pub df: bool,
    pub of: bool,
    /// Every bit of `EFLAGS` this engine does not model, exactly as the guest
    /// last left it.
    rest: u32,
}

impl Default for Flags {
    fn default() -> Self {
        Self::from_eflags(ALWAYS_ONE)
    }
}

impl Flags {
    #[must_use]
    pub fn from_eflags(e: u32) -> Self {
        Self {
            cf: e & CF != 0,
            pf: e & PF != 0,
            af: e & AF != 0,
            zf: e & ZF != 0,
            sf: e & SF != 0,
            df: e & DF != 0,
            of: e & OF != 0,
            rest: (e & !OURS) | ALWAYS_ONE,
        }
    }

    #[must_use]
    pub fn to_eflags(self) -> u32 {
        let mut e = self.rest | ALWAYS_ONE;
        for (bit, set) in [
            (CF, self.cf),
            (PF, self.pf),
            (AF, self.af),
            (ZF, self.zf),
            (SF, self.sf),
            (DF, self.df),
            (OF, self.of),
        ] {
            if set {
                e |= bit;
            }
        }
        e
    }

    /// Parity is of the low byte only, on every width.
    #[inline(always)]
    fn set_pzs(&mut self, result: u32, w: Width) {
        self.pf = parity(result as u8);
        self.zf = result & w.mask() == 0;
        self.sf = result & w.sign_bit() != 0;
    }

    #[inline(always)]
    fn set_pzs32(&mut self, result: u32) {
        self.pf = parity(result as u8);
        self.zf = result == 0;
        self.sf = (result as i32) < 0;
    }

    /// Flags after a logical operation: carry and overflow are cleared, and
    /// the auxiliary flag is left undefined by the manual, so it is cleared
    /// too rather than left to chance.
    pub fn logic(&mut self, result: u32, w: Width) {
        self.set_pzs(result, w);
        self.cf = false;
        self.of = false;
        self.af = false;
    }

    /// Flags after a 32-bit logical operation (`and`, `or`, `xor`, `test`).
    #[inline(always)]
    pub fn logic32(&mut self, result: u32) {
        self.set_pzs32(result);
        self.cf = false;
        self.of = false;
        self.af = false;
    }

    /// Flags after an addition, with an optional carry in.
    pub fn add(&mut self, a: u32, b: u32, carry_in: bool, w: Width) -> u32 {
        let c = u32::from(carry_in);
        let full = u64::from(a & w.mask()) + u64::from(b & w.mask()) + u64::from(c);
        let result = (full as u32) & w.mask();
        self.set_pzs(result, w);
        self.cf = full >> w.bits() != 0;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        let sign = w.sign_bit();
        self.of = ((a ^ result) & (b ^ result) & sign) != 0;
        result
    }

    /// Flags after an 8-bit addition with no carry in (`add`).
    #[inline(always)]
    pub fn add8(&mut self, a: u8, b: u8) -> u8 {
        let (result, cf) = a.overflowing_add(b);
        let (_, of) = (a as i8).overflowing_add(b as i8);
        self.pf = parity(result);
        self.zf = result == 0;
        self.sf = (result as i8) < 0;
        self.cf = cf;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = of;
        result
    }

    /// Flags after a 16-bit addition with no carry in (`add`).
    #[inline(always)]
    pub fn add16(&mut self, a: u16, b: u16) -> u16 {
        let (result, cf) = a.overflowing_add(b);
        let (_, of) = (a as i16).overflowing_add(b as i16);
        self.pf = parity(result as u8);
        self.zf = result == 0;
        self.sf = (result as i16) < 0;
        self.cf = cf;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = of;
        result
    }

    /// Flags after a 32-bit addition with no carry in (`add`).
    #[inline(always)]
    pub fn add32(&mut self, a: u32, b: u32) -> u32 {
        let (result, cf) = a.overflowing_add(b);
        let (_, of) = (a as i32).overflowing_add(b as i32);
        self.set_pzs32(result);
        self.cf = cf;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = of;
        result
    }

    /// Flags after a 32-bit addition with carry in (`adc`).
    #[inline(always)]
    pub fn adc32(&mut self, a: u32, b: u32, carry_in: bool) -> u32 {
        let (ab, c1) = a.overflowing_add(b);
        let (result, c2) = ab.overflowing_add(u32::from(carry_in));
        self.set_pzs32(result);
        self.cf = c1 | c2;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = ((a ^ result) & (b ^ result) & 0x8000_0000) != 0;
        result
    }

    /// Flags after a subtraction, with an optional borrow in.
    pub fn sub(&mut self, a: u32, b: u32, borrow_in: bool, w: Width) -> u32 {
        let c = u32::from(borrow_in);
        let a_m = a & w.mask();
        let b_m = b & w.mask();
        let full = u64::from(a_m)
            .wrapping_sub(u64::from(b_m))
            .wrapping_sub(u64::from(c));
        let result = (full as u32) & w.mask();
        self.set_pzs(result, w);
        self.cf = u64::from(a_m) < u64::from(b_m) + u64::from(c);
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        let sign = w.sign_bit();
        self.of = ((a ^ b) & (a ^ result) & sign) != 0;
        result
    }

    /// Flags after an 8-bit subtraction with no borrow in (`sub`, `cmp`, `scasb`, `cmpsb`).
    #[inline(always)]
    pub fn sub8(&mut self, a: u8, b: u8) -> u8 {
        let (result, cf) = a.overflowing_sub(b);
        let (_, of) = (a as i8).overflowing_sub(b as i8);
        self.pf = parity(result);
        self.zf = result == 0;
        self.sf = (result as i8) < 0;
        self.cf = cf;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = of;
        result
    }

    /// Flags after a 16-bit subtraction with no borrow in (`sub`, `cmp`, `scasw`, `cmpsw`).
    #[inline(always)]
    pub fn sub16(&mut self, a: u16, b: u16) -> u16 {
        let (result, cf) = a.overflowing_sub(b);
        let (_, of) = (a as i16).overflowing_sub(b as i16);
        self.pf = parity(result as u8);
        self.zf = result == 0;
        self.sf = (result as i16) < 0;
        self.cf = cf;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = of;
        result
    }

    /// Flags after a 32-bit subtraction with no borrow in (`sub`, `cmp`, `neg`).
    #[inline(always)]
    pub fn sub32(&mut self, a: u32, b: u32) -> u32 {
        let (result, cf) = a.overflowing_sub(b);
        let (_, of) = (a as i32).overflowing_sub(b as i32);
        self.set_pzs32(result);
        self.cf = cf;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = of;
        result
    }

    /// Flags after a 32-bit subtraction with borrow in (`sbb`).
    #[inline(always)]
    pub fn sbb32(&mut self, a: u32, b: u32, borrow_in: bool) -> u32 {
        let (ab, c1) = a.overflowing_sub(b);
        let (result, c2) = ab.overflowing_sub(u32::from(borrow_in));
        self.set_pzs32(result);
        self.cf = c1 | c2;
        self.af = ((a ^ b ^ result) & 0x10) != 0;
        self.of = ((a ^ b) & (a ^ result) & 0x8000_0000) != 0;
        result
    }

    /// Set parity, zero and sign from a result, leaving carry and overflow to
    /// the caller. What a shift needs: it has already worked out its own
    /// carry from the bit that left the value.
    pub fn logic_keep_carry(&mut self, result: u32, w: Width) {
        self.set_pzs(result, w);
        self.af = false;
    }

    /// Set 32-bit parity, zero and sign from a result, leaving carry and
    /// overflow to the caller.
    #[inline(always)]
    pub fn logic_keep_carry32(&mut self, result: u32) {
        self.set_pzs32(result);
        self.af = false;
    }

    /// Flags after an increment or a decrement, which leave carry alone.
    pub fn inc_dec(&mut self, before: u32, up: bool, w: Width) -> u32 {
        let carry = self.cf;
        let result = if up {
            self.add(before, 1, false, w)
        } else {
            self.sub(before, 1, false, w)
        };
        self.cf = carry;
        result
    }

    /// Flags after a 32-bit increment (`inc`), leaving carry untouched.
    #[inline(always)]
    pub fn inc32(&mut self, before: u32) -> u32 {
        let result = before.wrapping_add(1);
        self.set_pzs32(result);
        self.af = (result & 0x0f) == 0;
        self.of = result == 0x8000_0000;
        result
    }

    /// Flags after a 32-bit decrement (`dec`), leaving carry untouched.
    #[inline(always)]
    pub fn dec32(&mut self, before: u32) -> u32 {
        let result = before.wrapping_sub(1);
        self.set_pzs32(result);
        self.af = (before & 0x0f) == 0;
        self.of = before == 0x8000_0000;
        result
    }
}

/// Even parity of the low byte via a 16-bit nibble lookup table.
///
/// Bit `i` of `0x9669` is set iff the 4-bit nibble `i` has an even popcount,
/// giving branchless 3-op parity on both `aarch64` and baseline `x86_64` SSE2
/// without `popcnt`.
#[inline(always)]
fn parity(byte: u8) -> bool {
    let folded = (byte ^ (byte >> 4)) & 0x0f;
    ((0x9669u32 >> folded) & 1) != 0
}

#[cfg(test)]
mod tests {
    use super::{parity, Flags, Width};

    #[test]
    fn nibble_parity_matches_popcount_for_every_byte() {
        for b in 0u8..=255 {
            assert_eq!(
                parity(b),
                b.count_ones().is_multiple_of(2),
                "parity mismatch for byte {b:#04x}"
            );
        }
    }

    #[test]
    fn specialized_32bit_flag_ops_match_generic_width_b32() {
        const EDGE: [u32; 16] = [
            0,
            1,
            0x0f,
            0x10,
            0x7f,
            0x80,
            0xff,
            0x0000_ffff,
            0x7fff_ffef,
            0x7fff_fff0,
            0x7fff_ffff,
            0x8000_0000,
            0x8000_0001,
            0xffff_fff0,
            0xffff_fffe,
            0xffff_ffff,
        ];
        // Include non-arithmetic bits (`DF` and reserved/unmodeled `rest`) to
        // prove specialized paths leave them untouched.
        let init = Flags::from_eflags(0x0024_0602);

        for &a in &EDGE {
            let mut gen_l = init;
            let mut fast_l = init;
            gen_l.logic(a, Width::B32);
            fast_l.logic32(a);
            assert_eq!(fast_l.to_eflags(), gen_l.to_eflags(), "logic32({a:#x})");

            let mut gen_lkc = init;
            let mut fast_lkc = init;
            gen_lkc.cf = true;
            fast_lkc.cf = true;
            gen_lkc.of = true;
            fast_lkc.of = true;
            gen_lkc.logic_keep_carry(a, Width::B32);
            fast_lkc.logic_keep_carry32(a);
            assert_eq!(
                fast_lkc.to_eflags(),
                gen_lkc.to_eflags(),
                "logic_keep_carry32({a:#x})"
            );

            for cf in [false, true] {
                let mut gen_inc = init;
                let mut fast_inc = init;
                gen_inc.cf = cf;
                fast_inc.cf = cf;
                let r_gen = gen_inc.inc_dec(a, true, Width::B32);
                let r_fast = fast_inc.inc32(a);
                assert_eq!(
                    (r_fast, fast_inc.to_eflags()),
                    (r_gen, gen_inc.to_eflags()),
                    "inc32({a:#x}, cf={cf})"
                );

                let mut gen_dec = init;
                let mut fast_dec = init;
                gen_dec.cf = cf;
                fast_dec.cf = cf;
                let r_gen = gen_dec.inc_dec(a, false, Width::B32);
                let r_fast = fast_dec.dec32(a);
                assert_eq!(
                    (r_fast, fast_dec.to_eflags()),
                    (r_gen, gen_dec.to_eflags()),
                    "dec32({a:#x}, cf={cf})"
                );
            }

            for &b in &EDGE {
                let mut gen_add = init;
                let mut fast_add = init;
                let r_gen = gen_add.add(a, b, false, Width::B32);
                let r_fast = fast_add.add32(a, b);
                assert_eq!(
                    (r_fast, fast_add.to_eflags()),
                    (r_gen, gen_add.to_eflags()),
                    "add32({a:#x}, {b:#x})"
                );

                let mut gen_sub = init;
                let mut fast_sub = init;
                let r_gen = gen_sub.sub(a, b, false, Width::B32);
                let r_fast = fast_sub.sub32(a, b);
                assert_eq!(
                    (r_fast, fast_sub.to_eflags()),
                    (r_gen, gen_sub.to_eflags()),
                    "sub32({a:#x}, {b:#x})"
                );

                for carry in [false, true] {
                    let mut gen_adc = init;
                    let mut fast_adc = init;
                    let r_gen = gen_adc.add(a, b, carry, Width::B32);
                    let r_fast = fast_adc.adc32(a, b, carry);
                    assert_eq!(
                        (r_fast, fast_adc.to_eflags()),
                        (r_gen, gen_adc.to_eflags()),
                        "adc32({a:#x}, {b:#x}, {carry})"
                    );

                    let mut gen_sbb = init;
                    let mut fast_sbb = init;
                    let r_gen = gen_sbb.sub(a, b, carry, Width::B32);
                    let r_fast = fast_sbb.sbb32(a, b, carry);
                    assert_eq!(
                        (r_fast, fast_sbb.to_eflags()),
                        (r_gen, gen_sbb.to_eflags()),
                        "sbb32({a:#x}, {b:#x}, {carry})"
                    );
                }
            }
        }
    }

    #[test]
    fn specialized_8bit_flag_ops_match_generic_width_b8_exhaustively() {
        let init = Flags::from_eflags(0x0024_0602);
        for a in 0u8..=255 {
            for b in 0u8..=255 {
                let mut gen_add = init;
                let mut fast_add = init;
                let r_gen = gen_add.add(u32::from(a), u32::from(b), false, Width::B8);
                let r_fast = u32::from(fast_add.add8(a, b));
                assert_eq!(
                    (r_fast, fast_add.to_eflags()),
                    (r_gen, gen_add.to_eflags()),
                    "add8({a:#04x}, {b:#04x})"
                );

                let mut gen_sub = init;
                let mut fast_sub = init;
                let r_gen = gen_sub.sub(u32::from(a), u32::from(b), false, Width::B8);
                let r_fast = u32::from(fast_sub.sub8(a, b));
                assert_eq!(
                    (r_fast, fast_sub.to_eflags()),
                    (r_gen, gen_sub.to_eflags()),
                    "sub8({a:#04x}, {b:#04x})"
                );
            }
        }
    }

    #[test]
    fn specialized_16bit_flag_ops_match_generic_width_b16() {
        const EDGE16: [u16; 18] = [
            0x0000, 0x0001, 0x000f, 0x0010, 0x007f, 0x0080, 0x00ff, 0x0100, 0x7fef, 0x7ff0, 0x7fff,
            0x8000, 0x8001, 0x80ff, 0xfff0, 0xfffe, 0xffff, 0x5a3c,
        ];
        let init = Flags::from_eflags(0x0024_0602);

        let check = |a: u16, b: u16| {
            let mut gen_add = init;
            let mut fast_add = init;
            let r_gen = gen_add.add(u32::from(a), u32::from(b), false, Width::B16);
            let r_fast = u32::from(fast_add.add16(a, b));
            assert_eq!(
                (r_fast, fast_add.to_eflags()),
                (r_gen, gen_add.to_eflags()),
                "add16({a:#06x}, {b:#06x})"
            );

            let mut gen_sub = init;
            let mut fast_sub = init;
            let r_gen = gen_sub.sub(u32::from(a), u32::from(b), false, Width::B16);
            let r_fast = u32::from(fast_sub.sub16(a, b));
            assert_eq!(
                (r_fast, fast_sub.to_eflags()),
                (r_gen, gen_sub.to_eflags()),
                "sub16({a:#06x}, {b:#06x})"
            );
        };

        for &a in &EDGE16 {
            for &b in &EDGE16 {
                check(a, b);
            }
        }
        // Deterministic sweep across 16-bit space.
        let mut state = 0x1234_5678u32;
        for _ in 0..4096 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            check(state as u16, (state >> 16) as u16);
        }
    }
}
