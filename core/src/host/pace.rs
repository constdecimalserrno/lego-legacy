//! Holding presentation to real time, the same on every host with a window
//! (ADR-0003).
//!
//! A game-second takes a wall-second: a machine ahead of its own clock waits,
//! and one that has fallen behind carries on from where it got to. The pace
//! it settles on is what the mix is stretched by (#75). Written for macOS and
//! moved here unchanged, with no `cfg`, so every window backend paces the
//! same way (#107).

use std::time::Instant;

/// How much wall time the pace is worked out over: long enough that one slow
/// flip is not a pace change, short enough that the mix follows the picture
/// (#75). Half a second is about fifteen flips on a host that is struggling.
const PACE_NANOS: u64 = 500_000_000;

/// A window's hold on presentation, and the pace it came to.
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

impl Pace {
    /// Pacing from now, at real time until a window of wall time says
    /// otherwise.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            lag_nanos: 0,
            paced_from: (0, 0),
            pace: 1.0,
        }
    }

    /// The pace last settled on: declared seconds a wall second.
    #[must_use]
    pub fn current(&self) -> f64 {
        self.pace
    }

    /// What [`super::Host::pace`] does on a host with a window: wait until
    /// the declared clock is due, and answer with the pace.
    pub fn hold(&mut self, declared_nanos: u64) -> f64 {
        // A game-second takes a wall-second (ADR-0003): if the machine is
        // ahead of its own clock, it waits.
        let wall = self.wall_nanos();
        self.lag_nanos = self.lag_nanos.max(wall.saturating_sub(declared_nanos));
        let due = declared_nanos + self.lag_nanos;
        if due > wall {
            std::thread::sleep(std::time::Duration::from_nanos(due - wall));
        }
        // How much declared time the last window of wall time was worth. Over
        // a window rather than a flip, because one flip's worth is noise, and
        // the mix is stretched by this (#75).
        let now = self.wall_nanos();
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

    /// The host's own clock, in nanoseconds since the window opened. The guest
    /// cannot reach this: it paces presentation.
    fn wall_nanos(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

impl Default for Pace {
    fn default() -> Self {
        Self::new()
    }
}
