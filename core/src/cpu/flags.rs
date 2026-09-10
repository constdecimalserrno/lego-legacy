//! The flags register.
//!
//! The engine keeps the six arithmetic flags it computes plus the direction
//! flag, and carries every other bit of `EFLAGS` untouched. Carrying them is
//! what makes a round trip exact: a guest that pushes its flags, changes one,
//! and pops them back must get its own bits returned, not ours.

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
    fn set_pzs(&mut self, result: u32, w: Width) {
        self.pf = (result as u8).count_ones().is_multiple_of(2);
        self.zf = result & w.mask() == 0;
        self.sf = result & w.sign_bit() != 0;
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

    /// Set parity, zero and sign from a result, leaving carry and overflow to
    /// the caller. What a shift needs: it has already worked out its own
    /// carry from the bit that left the value.
    pub fn logic_keep_carry(&mut self, result: u32, w: Width) {
        self.set_pzs(result, w);
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
}
