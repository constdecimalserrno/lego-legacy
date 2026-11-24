//! The cursor and the declared clock (ADR-0003).
//!
//! Time on the declared machine is a function of what the machine did, never
//! of what the host's wall clock says. A run's timing therefore depends on the
//! route alone, which is what makes a route replayable.

/// The roster's baseline declared processor: Pentium II 450, one instruction
/// per cycle.
///
/// The processor is a declared value like any other, and this is the value a
/// title takes unless its own crate declares another. It decides how much
/// work a declared second holds, which is what a title that waits by reading
/// the clock rather than by sleeping spends its time on.
pub const DECLARED_HZ: u64 = 450_000_000;

/// What one presented frame costs the declared clock.
///
/// Every title declares [`FlipCost::Refresh`], as a flip on real hardware waits
/// for the display; [`FlipCost::Free`] is the stub's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipCost {
    /// A flip costs nothing: the stub's, which has no display to wait for.
    Free,
    /// A presented frame waits for the declared display's next refresh, at
    /// this many per second: what a flip does by default on real hardware.
    Refresh(u64),
}

/// Greatest common divisor, used to reduce `1_000_000_000 / hz` to lowest
/// terms once when a [`Clock`] is built.
const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

/// Instructions retired, and the declared time that implies.
///
/// The cursor is the count of guest instructions the machine has retired. It
/// names one moment in a run: every log line, every route event and every
/// snapshot is keyed to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    cursor: u64,
    /// Declared cycles charged on top of instructions: presented frames, and
    /// time the guest spent waiting.
    charged_cycles: u64,
    /// The declared processor this clock runs at, in cycles a second.
    hz: u64,
    /// `1_000_000_000 / gcd(1_000_000_000, hz)`.
    nanos_num: u64,
    /// `hz / gcd(1_000_000_000, hz)`.
    nanos_den: u64,
    /// Cached [`Clock::millis`], kept current whenever `cycles` reaches
    /// `next_millis_cycle`.
    cached_millis: u64,
    /// First declared cycle at which [`Clock::millis`] exceeds `cached_millis`.
    next_millis_cycle: u64,
}

impl Default for Clock {
    fn default() -> Self {
        Self::at(DECLARED_HZ)
    }
}

impl Clock {
    /// A clock on a declared processor of this frequency.
    #[must_use]
    pub fn at(hz: u64) -> Self {
        let hz = hz.max(1);
        let g = gcd(1_000_000_000, hz);
        let mut clock = Self {
            cursor: 0,
            charged_cycles: 0,
            hz,
            nanos_num: 1_000_000_000 / g,
            nanos_den: hz / g,
            cached_millis: 0,
            next_millis_cycle: 0,
        };
        clock.refresh_millis();
        clock
    }

    /// What the declared processor runs at.
    #[inline(always)]
    #[must_use]
    pub fn hz(self) -> u64 {
        self.hz
    }

    /// Put a clock back where a snapshot left it, on the processor the title
    /// declares.
    ///
    /// The frequency comes from the caller rather than from the roster's
    /// baseline: a snapshot does not carry it, and a title that declares
    /// another one — Hot Wheels declares a Pentium 166 — would otherwise have
    /// its clock quietly swapped for a faster machine's the first time a
    /// snapshot was written.
    #[must_use]
    pub fn resumed(cursor: u64, cycles: u64, hz: u64) -> Self {
        let hz = hz.max(1);
        let g = gcd(1_000_000_000, hz);
        let mut clock = Self {
            cursor,
            charged_cycles: cycles.saturating_sub(cursor),
            hz,
            nanos_num: 1_000_000_000 / g,
            nanos_den: hz / g,
            cached_millis: 0,
            next_millis_cycle: 0,
        };
        clock.refresh_millis();
        clock
    }

    /// Instructions retired so far.
    #[inline(always)]
    #[must_use]
    pub fn cursor(self) -> u64 {
        self.cursor
    }

    /// Charge one retired instruction.
    #[inline(always)]
    pub fn retire(&mut self) {
        self.cursor += 1;
        if self.cycles() >= self.next_millis_cycle {
            self.refresh_millis();
        }
    }

    /// Charge instructions an engine retired in one run.
    #[inline(always)]
    pub fn retire_many(&mut self, count: u64) {
        self.cursor += count;
        if self.cycles() >= self.next_millis_cycle {
            self.refresh_millis();
        }
    }

    /// Charge a presented frame at the title's declared flip cost.
    pub fn present(&mut self, cost: FlipCost) {
        match cost {
            FlipCost::Free => {}
            // The next refresh strictly after now, so no two flips share one.
            FlipCost::Refresh(hz) => {
                let period = self.hz / hz.max(1);
                self.charged_cycles += period - self.cycles() % period;
                if self.cycles() >= self.next_millis_cycle {
                    self.refresh_millis();
                }
            }
        }
    }

    /// Charge declared time the guest spent waiting: a sleep, or a blocking
    /// wait for something due later on the declared clock.
    pub fn wait(&mut self, millis: u64) {
        self.charged_cycles += millis * self.hz / 1000;
        if self.cycles() >= self.next_millis_cycle {
            self.refresh_millis();
        }
    }

    /// Declared cycles spent: one per retired instruction, plus what drawing
    /// and waiting were charged for.
    #[inline(always)]
    #[must_use]
    pub fn cycles(self) -> u64 {
        self.cursor + self.charged_cycles
    }

    /// Declared nanoseconds elapsed. Computed with GCD-reduced 64-bit
    /// arithmetic and a `u128` overflow fallback so a long run does not drift
    /// and matches `u128` bit-for-bit.
    #[inline]
    #[must_use]
    pub fn nanos(self) -> u64 {
        let cycles = self.cycles();
        if let Some(prod) = cycles.checked_mul(self.nanos_num) {
            prod / self.nanos_den
        } else {
            self.nanos_wide(cycles)
        }
    }

    #[cold]
    #[inline(never)]
    fn nanos_wide(self, cycles: u64) -> u64 {
        let n = u128::from(cycles) * 1_000_000_000 / u128::from(self.hz);
        u64::try_from(n).unwrap_or(u64::MAX)
    }

    /// Declared milliseconds elapsed. What a guest's `timeGetTime` will read.
    #[inline(always)]
    #[must_use]
    pub fn millis(self) -> u64 {
        self.cached_millis
    }

    #[cold]
    #[inline(never)]
    fn refresh_millis(&mut self) {
        let millis = self.nanos() / 1_000_000;
        self.cached_millis = millis;
        self.next_millis_cycle = if millis >= u64::MAX / 1_000_000 {
            u64::MAX
        } else {
            self.first_cycle_of(millis + 1)
        };
    }

    /// The first declared cycle that [`Clock::millis`] reads as `millis`:
    /// since a millisecond is `cycles × 1000 ÷ hz` rounded down, its first
    /// cycle is `millis × hz ÷ 1000` rounded up. Where a clock spin is
    /// passed to (#217).
    #[must_use]
    pub fn first_cycle_of(self, millis: u64) -> u64 {
        if let Some(prod) = millis.checked_mul(self.hz) {
            return prod.div_ceil(1000);
        }
        let first = (u128::from(millis) * u128::from(self.hz)).div_ceil(1000);
        u64::try_from(first).unwrap_or(u64::MAX)
    }

    /// The first declared cycle whose `sound::frame_at(self.nanos())` is at
    /// least `frame`, used to gate `Machine::mix` between audio frames.
    #[must_use]
    pub fn first_cycle_of_audio_frame(self, frame: u64) -> u64 {
        const FRAME_NUM: u64 = 441;
        const FRAME_DEN: u64 = 20_000_000;
        if let Some(need_nanos) = frame
            .checked_mul(FRAME_DEN)
            .map(|prod| prod.div_ceil(FRAME_NUM))
        {
            if let Some(prod) = need_nanos.checked_mul(self.nanos_den) {
                return prod.div_ceil(self.nanos_num);
            }
        }
        let need_nanos =
            (u128::from(frame) * u128::from(FRAME_DEN)).div_ceil(u128::from(FRAME_NUM));
        if need_nanos > u128::from(u64::MAX) {
            return u64::MAX;
        }
        let Some(prod) = need_nanos.checked_mul(u128::from(self.hz)) else {
            return u64::MAX;
        };
        u64::try_from(prod.div_ceil(1_000_000_000)).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, FlipCost, DECLARED_HZ};

    #[test]
    fn a_second_of_declared_time_is_one_frequency_of_instructions() {
        let mut c = Clock::default();
        for _ in 0..1000 {
            c.retire();
        }
        assert_eq!(c.cursor(), 1000);
        assert_eq!(c.nanos(), 1000 * 1_000_000_000 / DECLARED_HZ);
    }

    #[test]
    fn a_clock_written_down_keeps_the_processor_it_was_declared_on() {
        let mut clock = Clock::at(166_000_000);
        for _ in 0..1000 {
            clock.retire();
        }
        let (cursor, cycles) = (clock.cursor(), clock.cycles());
        // What `Snap` does on the way out as well as on the way back.
        let put_back = Clock::resumed(cursor, cycles, clock.hz());
        assert_eq!(put_back.hz(), 166_000_000);
        assert_eq!(put_back.nanos(), clock.nanos());
    }

    #[test]
    fn a_millisecond_s_first_cycle_reads_as_it_and_the_cycle_before_does_not() {
        // A rate that is no whole number of kilohertz, as a real Pentium 166
        // runs at, so the rounding up is needed; and the roster's own.
        for hz in [166_666_667, 166_000_000, DECLARED_HZ] {
            for millis in [1, 2, 3, 999, 1_000, 123_457] {
                let first = Clock::at(hz).first_cycle_of(millis);
                let at = |cycles: u64| Clock::resumed(cycles, cycles, hz).millis();
                assert_eq!(at(first), millis, "{hz} Hz: {first} is in {millis} ms");
                assert_eq!(
                    at(first - 1),
                    millis - 1,
                    "{hz} Hz: the cycle before is not"
                );
            }
        }
    }

    #[test]
    fn a_free_flip_costs_the_clock_nothing() {
        let mut free = Clock::default();
        free.present(FlipCost::Free);
        assert_eq!(free.cycles(), 0);
    }

    #[test]
    fn a_frame_waiting_for_the_refresh_lands_on_the_next_one() {
        let mut clock = Clock::default();
        for _ in 0..1000 {
            clock.retire();
        }
        clock.present(FlipCost::Refresh(60));
        assert_eq!(clock.cycles(), DECLARED_HZ / 60);
        // A second flip straight after waits for the refresh after that.
        clock.present(FlipCost::Refresh(60));
        assert_eq!(clock.cycles(), 2 * DECLARED_HZ / 60);
    }

    fn ref_nanos(cycles: u64, hz: u64) -> u64 {
        let n = u128::from(cycles) * 1_000_000_000 / u128::from(hz.max(1));
        u64::try_from(n).unwrap_or(u64::MAX)
    }

    fn ref_millis(cycles: u64, hz: u64) -> u64 {
        ref_nanos(cycles, hz) / 1_000_000
    }

    #[test]
    fn nanos_and_millis_match_u128_reference_across_sweeps_and_overflow() {
        let rates = [
            1,
            500,
            1_000,
            166_000_000,
            166_666_667,
            DECLARED_HZ,
            1_000_000_000,
            3_000_000_000,
            u64::MAX,
        ];
        for hz in rates {
            let base = Clock::at(hz);
            let mut samples: Vec<u64> = (0..=64).collect();
            let mut c = 1u64;
            while c <= 100_000_000_000_000 {
                samples.push(c.saturating_sub(1));
                samples.push(c);
                samples.push(c.saturating_add(1));
                c = c.saturating_mul(7).saturating_add(13);
            }
            for m in [1, 2, 3, 17, 999, 1_000, 120_966, 605_367, 1_000_000] {
                let first = base.first_cycle_of(m);
                samples.push(first.saturating_sub(1));
                samples.push(first);
                samples.push(first.saturating_add(1));
            }
            let mul_edge = u64::MAX / base.nanos_num;
            for delta in -3i64..=3 {
                samples.push(mul_edge.saturating_add_signed(delta));
            }
            let sat_edge = u64::try_from(u128::from(u64::MAX) * u128::from(hz) / 1_000_000_000)
                .unwrap_or(u64::MAX);
            for delta in -3i64..=3 {
                samples.push(sat_edge.saturating_add_signed(delta));
            }
            for edge in [u64::MAX - 2, u64::MAX - 1, u64::MAX] {
                samples.push(edge);
            }

            for cycles in samples {
                let clock = Clock::resumed(cycles, cycles, hz);
                assert_eq!(
                    clock.nanos(),
                    ref_nanos(cycles, hz),
                    "nanos mismatch at hz={hz} cycles={cycles}"
                );
                assert_eq!(
                    clock.millis(),
                    ref_millis(cycles, hz),
                    "millis mismatch at hz={hz} cycles={cycles}"
                );
            }
        }
    }

    #[test]
    fn millis_cache_stays_exact_across_retire_wait_and_present() {
        for hz in [1, 500, 166_000_000, 166_666_667, DECLARED_HZ] {
            let mut clock = Clock::at(hz);
            // Step across millisecond boundaries with retire, retire_many, wait, and present.
            for _ in 0..20 {
                let target = clock.first_cycle_of(clock.millis() + 1);
                if target > clock.cycles() + 2 {
                    clock.retire_many(target - clock.cycles() - 2);
                    assert_eq!(clock.millis(), ref_millis(clock.cycles(), hz));
                }
                for _ in 0..4 {
                    clock.retire();
                    assert_eq!(clock.millis(), ref_millis(clock.cycles(), hz));
                }
                clock.wait(3);
                assert_eq!(clock.millis(), ref_millis(clock.cycles(), hz));
                if hz >= 60 {
                    clock.present(FlipCost::Refresh(60));
                    assert_eq!(clock.millis(), ref_millis(clock.cycles(), hz));
                }
            }
        }
    }

    #[test]
    fn first_cycle_of_audio_frame_matches_sound_frame_at_boundary() {
        use crate::sound::frame_at;
        for hz in [1, 166_000_000, 166_666_667, DECLARED_HZ] {
            let base = Clock::at(hz);
            assert_eq!(base.first_cycle_of_audio_frame(0), 0);
            for frame in [
                1,
                2,
                3,
                10,
                1024,
                22_050,
                1_000_000,
                13_348_000,
                100_000_000,
            ] {
                let first = base.first_cycle_of_audio_frame(frame);
                assert!(
                    frame_at(Clock::resumed(first, first, hz).nanos()) >= frame,
                    "hz={hz} frame={frame} first={first}"
                );
                assert!(
                    frame_at(Clock::resumed(first - 1, first - 1, hz).nanos()) < frame,
                    "hz={hz} frame={frame} first-1={}",
                    first - 1
                );
            }
            let max_frame = frame_at(u64::MAX);
            assert_eq!(base.first_cycle_of_audio_frame(max_frame + 1), u64::MAX);
        }
    }
}
