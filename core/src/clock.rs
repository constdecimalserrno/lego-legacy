//! The cursor and the declared clock (ADR-0003).
//!
//! Time on the declared machine is a function of what the machine did, never
//! of what the host's wall clock says. A run's timing therefore depends on the
//! route alone, which is what makes a route replayable.

/// Declared processor frequency: Pentium II 450, one instruction per cycle.
///
/// The processor is a declared value like any other. 450 MHz is the baseline
/// the roster's titles shipped against; a title that measures otherwise
/// re-declares it in its own crate.
pub const DECLARED_HZ: u64 = 450_000_000;

/// What one presented frame costs the declared clock.
///
/// A title declares this once its own drawing has been measured. Until then it
/// is [`FlipCost::Free`], and a run that presents costs nothing extra: honest,
/// because nothing has been measured yet, and visible, because the value is
/// named rather than defaulted silently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipCost {
    /// Drawing is instantaneous on this declared machine. Unmeasured, or
    /// deliberately uncapped for a headless run.
    Free,
    /// Drawing costs this many declared cycles per presented frame.
    Cycles(u64),
    /// A presented frame waits for the declared display's next refresh, at
    /// this many per second: what a flip does by default on real hardware.
    Refresh(u64),
}

/// Instructions retired, and the declared time that implies.
///
/// The cursor is the count of guest instructions the machine has retired. It
/// names one moment in a run: every log line, every route event and every
/// snapshot is keyed to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clock {
    cursor: u64,
    flip_cycles: u64,
}

impl Clock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a clock back where a snapshot left it.
    #[must_use]
    pub fn resumed(cursor: u64, cycles: u64) -> Self {
        Self {
            cursor,
            flip_cycles: cycles.saturating_sub(cursor),
        }
    }

    /// Instructions retired so far.
    #[must_use]
    pub fn cursor(self) -> u64 {
        self.cursor
    }

    /// Charge one retired instruction.
    pub fn retire(&mut self) {
        self.cursor += 1;
    }

    /// Charge a presented frame at the title's declared flip cost.
    pub fn present(&mut self, cost: FlipCost) {
        match cost {
            FlipCost::Free => {}
            FlipCost::Cycles(c) => self.flip_cycles += c,
            FlipCost::Refresh(hz) => {
                let period = DECLARED_HZ / hz.max(1);
                let into = self.cycles() % period;
                if into != 0 {
                    self.flip_cycles += period - into;
                }
            }
        }
    }

    /// Charge declared time the guest spent waiting: a sleep, or a blocking
    /// wait for something due later on the declared clock.
    pub fn wait(&mut self, millis: u64) {
        self.flip_cycles += millis * DECLARED_HZ / 1000;
    }

    /// Declared cycles spent: one per retired instruction, plus what drawing
    /// and waiting were charged for.
    #[must_use]
    pub fn cycles(self) -> u64 {
        self.cursor + self.flip_cycles
    }

    /// Declared nanoseconds elapsed. Computed in `u128` so a long run does not
    /// drift: the division happens once, at the end, not per instruction.
    #[must_use]
    pub fn nanos(self) -> u64 {
        let n = u128::from(self.cycles()) * 1_000_000_000 / u128::from(DECLARED_HZ);
        u64::try_from(n).unwrap_or(u64::MAX)
    }

    /// Declared milliseconds elapsed. What a guest's `timeGetTime` will read.
    #[must_use]
    pub fn millis(self) -> u64 {
        self.nanos() / 1_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, FlipCost, DECLARED_HZ};

    #[test]
    fn a_second_of_declared_time_is_one_frequency_of_instructions() {
        let mut c = Clock::new();
        for _ in 0..1000 {
            c.retire();
        }
        assert_eq!(c.cursor(), 1000);
        assert_eq!(c.nanos(), 1000 * 1_000_000_000 / DECLARED_HZ);
    }

    #[test]
    fn a_free_flip_costs_the_clock_nothing_and_a_priced_one_costs_its_cycles() {
        let mut free = Clock::new();
        free.present(FlipCost::Free);
        assert_eq!(free.cycles(), 0);

        let mut priced = Clock::new();
        priced.present(FlipCost::Cycles(4_500_000));
        assert_eq!(priced.millis(), 10);
    }

    #[test]
    fn a_frame_waiting_for_the_refresh_lands_on_the_next_one() {
        let mut clock = Clock::new();
        for _ in 0..1000 {
            clock.retire();
        }
        clock.present(FlipCost::Refresh(60));
        assert_eq!(clock.cycles(), DECLARED_HZ / 60);
        // Already on a refresh: nothing more to wait for.
        clock.present(FlipCost::Refresh(60));
        assert_eq!(clock.cycles(), DECLARED_HZ / 60);
    }
}
