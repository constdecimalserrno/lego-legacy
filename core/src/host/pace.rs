//! Holding presentation to a wall-clock pace, on any host with a window
//! (ADR-0003).
//!
//! The one place a backend reads the host's own clock. The guest cannot reach
//! it: it only decides how long a flip is held back for. Every windowed backend
//! shares this, so a game-second takes a wall-second the same way on each, and
//! the tests reach it on every host (#108).

use std::time::{Duration, Instant};

/// How much wall time the pace is worked out over: long enough that one slow
/// flip is not a pace change, short enough that the mix follows the picture
/// (#75). Half a second is about fifteen flips on a host that is struggling.
const PACE_NANOS: u64 = 500_000_000;

/// The host's clock against the declared one.
#[derive(Debug)]
pub struct Pace {
    started: Instant,
    /// How far behind the declared clock the host has fallen and will not
    /// make up: a run the host could not keep pace with carries on in real
    /// time from where it got to, rather than racing to catch up.
    lag_nanos: u64,
    /// Where the wall and the declared clock were when the pace was last
    /// worked out, and the pace it came to: declared seconds a wall second
    /// (#75).
    paced_from: (u64, u64),
    pace: f64,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            lag_nanos: 0,
            paced_from: (0, 0),
            pace: 1.0,
        }
    }
}

impl Pace {
    /// The pace last settled on, which the mix is stretched by (#75).
    #[must_use]
    pub fn now(&self) -> f64 {
        self.pace
    }

    /// Hold presentation back so that a game-second takes a wall-second, and
    /// answer with the pace settled on ([`super::Host::pace`]).
    pub fn hold(&mut self, declared_nanos: u64) -> f64 {
        let wait = self.wait(self.wall_nanos(), declared_nanos);
        if wait > 0 {
            std::thread::sleep(Duration::from_nanos(wait));
        }
        self.settle(self.wall_nanos(), declared_nanos)
    }

    /// The host's own clock, in nanoseconds since the window opened.
    fn wall_nanos(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// How long to wait, at `wall`, for the declared clock to be due. A
    /// machine ahead of its own clock waits; one behind it has the lag taken
    /// as lost, and waits from there on.
    fn wait(&mut self, wall: u64, declared_nanos: u64) -> u64 {
        self.lag_nanos = self.lag_nanos.max(wall.saturating_sub(declared_nanos));
        (declared_nanos + self.lag_nanos).saturating_sub(wall)
    }

    /// How much declared time the last window of wall time was worth. Over a
    /// window rather than a flip, because one flip's worth is noise, and the
    /// mix is stretched by this (#75).
    fn settle(&mut self, now: u64, declared_nanos: u64) -> f64 {
        let (was_wall, was_declared) = self.paced_from;
        let (gone, made) = (
            now.saturating_sub(was_wall),
            declared_nanos.saturating_sub(was_declared),
        );
        if gone >= PACE_NANOS {
            self.pace = (made as f64 / gone as f64).clamp(0.0, 1.0);
            self.paced_from = (now, declared_nanos);
        }
        self.pace
    }
}

#[cfg(test)]
mod tests {
    use super::{Pace, PACE_NANOS};

    const MS: u64 = 1_000_000;

    /// A machine ahead of its clock waits for it; one that fell behind
    /// carries on from where it got to rather than racing to catch up.
    #[test]
    fn a_machine_ahead_waits_and_one_behind_carries_on_from_where_it_got_to() {
        let mut pace = Pace::default();
        assert_eq!(pace.wait(10 * MS, 40 * MS), 30 * MS, "ahead: it waits");
        assert_eq!(pace.wait(100 * MS, 50 * MS), 0, "behind: no wait");
        // Fifty milliseconds were lost, and the clock is due from there.
        assert_eq!(pace.wait(110 * MS, 70 * MS), 10 * MS);
    }

    /// The pace is declared time over wall time, worked out over half a
    /// second of wall and never above real time.
    #[test]
    fn the_pace_is_settled_over_half_a_second_of_wall() {
        let mut pace = Pace::default();
        assert_eq!(pace.settle(PACE_NANOS / 2, 100 * MS), 1.0, "too soon");
        assert!((pace.settle(PACE_NANOS, PACE_NANOS / 2) - 0.5).abs() < 1e-9);
        assert_eq!(pace.now(), pace.settle(PACE_NANOS + MS, PACE_NANOS));
        // A machine racing ahead of the wall is still real time.
        assert_eq!(pace.settle(2 * PACE_NANOS, 10 * PACE_NANOS), 1.0);
    }
}
