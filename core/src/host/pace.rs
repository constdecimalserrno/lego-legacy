//! Holding presentation to real time, the same on every host with a window
//! (ADR-0003).
//!
//! A game-second takes a wall-second: a machine ahead of its own clock waits,
//! and one that has fallen behind carries on from where it got to. The pace
//! it settles on is what the mix is stretched by (#75). Written for macOS and
//! moved here unchanged, with no `cfg`, so every window backend paces the
//! same way (#107).

use crate::sound::{BLOCK_FRAMES, RATE};
use std::time::Instant;

/// How much wall time the pace is worked out over: long enough that one slow
/// flip is not a pace change, short enough that the mix follows the picture
/// (#75). Half a second is about fifteen flips on a host that is struggling.
const PACE_NANOS: u64 = 500_000_000;

/// A window's hold on presentation, and the pace it came to.
pub struct Pace {
    started: Instant,
    /// A wall clock of the tests' own, in nanoseconds, which a hold moves on
    /// rather than sleeps: how the feed's simulated runs drive the very holds
    /// a window makes.
    #[cfg(test)]
    pub(crate) simulated: Option<u64>,
    /// Frames of the mix handed over so far: where in declared time the next
    /// block of it ends, since the mixer hands over every block from the
    /// run's start, silent ones too.
    // ponytail: counted from the run's start. A run resumed from a snapshot
    // at a window would need the mix's place handed over with it.
    mixed_frames: u64,
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
            #[cfg(test)]
            simulated: None,
            mixed_frames: 0,
            lag_nanos: 0,
            paced_from: (0, 0),
            pace: 1.0,
        }
    }

    /// What [`super::Host::pace`] does on a host with a window: wait until
    /// the declared clock is due, and answer with the pace.
    pub fn hold(&mut self, declared_nanos: u64) -> f64 {
        let wait = self.wait_for(declared_nanos, self.wall_nanos());
        if wait > 0 {
            self.sleep(wait);
        }
        self.settle(declared_nanos, self.wall_nanos(), wait > 0)
    }

    /// Hold the next block of the mix to its own time before it is handed
    /// over, and answer with the pace it is stretched by (#217). A machine
    /// ahead of real time, as one passing clock spins can be between two
    /// flips, otherwise hands over a whole wait's mix at once, more than a
    /// device keeps waiting.
    pub fn hold_mixed(&mut self) -> f64 {
        let at = self.next_block();
        self.hold(at)
    }

    /// Where in declared time the next block of the mix ends: blocks are
    /// mixed one after another from the run's start.
    fn next_block(&mut self) -> u64 {
        self.mixed_frames += BLOCK_FRAMES as u64;
        let nanos = u128::from(self.mixed_frames) * 1_000_000_000 / u128::from(RATE);
        u64::try_from(nanos).unwrap_or(u64::MAX)
    }

    /// How long a hold at `wall` waits for the declared clock to be due. A
    /// game-second takes a wall-second (ADR-0003): if the machine is ahead of
    /// its own clock, it waits.
    fn wait_for(&mut self, declared_nanos: u64, wall: u64) -> u64 {
        self.lag_nanos = self.lag_nanos.max(wall.saturating_sub(declared_nanos));
        (declared_nanos + self.lag_nanos).saturating_sub(wall)
    }

    /// The pace once a hold is over at `now`. A hold that had to wait found
    /// the machine ahead of real time, so the pace is real time there, and
    /// it is worked out afresh from there (#217): a slow stretch before it,
    /// as a title that waits while it polls makes one, no longer stretches
    /// the sound after it. Otherwise it is how much declared time the last
    /// window of wall time was worth: over a window rather than a flip,
    /// because one flip's worth is noise, and the mix is stretched by this
    /// (#75).
    fn settle(&mut self, declared_nanos: u64, now: u64, waited: bool) -> f64 {
        let (was_wall, was_declared) = self.paced_from;
        let (gone, made) = (
            now.saturating_sub(was_wall),
            declared_nanos.saturating_sub(was_declared),
        );
        if waited {
            self.pace = 1.0;
            self.paced_from = (now, declared_nanos);
        } else if gone >= PACE_NANOS {
            self.pace = (made as f64 / gone as f64).clamp(0.0, 1.0);
            self.paced_from = (now, declared_nanos);
        }
        self.pace
    }

    /// The host's own clock, in nanoseconds since the window opened. The guest
    /// cannot reach this: it paces presentation.
    pub(crate) fn wall_nanos(&self) -> u64 {
        #[cfg(test)]
        if let Some(wall) = self.simulated {
            return wall;
        }
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn sleep(&mut self, nanos: u64) {
        #[cfg(test)]
        if let Some(wall) = &mut self.simulated {
            *wall += nanos;
            return;
        }
        std::thread::sleep(std::time::Duration::from_nanos(nanos));
    }
}

impl Default for Pace {
    fn default() -> Self {
        Self::new()
    }
}
