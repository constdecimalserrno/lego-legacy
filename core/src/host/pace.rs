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
    /// When the host was paused, if paused right now (#296).
    paused_at: Option<Instant>,
    /// Cumulative wall nanoseconds spent paused, excluded from pacing so a
    /// long pause never drags pace or audio stretch down on resume (#296).
    paused_total_nanos: u128,
    /// A wall clock of the tests' own, in nanoseconds, which a hold moves on
    /// rather than sleeps: how the feed's simulated runs drive the very holds
    /// a window makes.
    #[cfg(test)]
    pub(crate) simulated: Option<u64>,
    #[cfg(test)]
    simulated_paused_at: Option<u64>,
    /// Frames of the mix handed over so far: where in declared time the next
    /// block of it ends, since the mixer hands over every block from the
    /// run's start, silent ones too.
    // ponytail: counted from the run's start. A run resumed from a snapshot
    // at a window would need the mix's place handed over with it.
    mixed_frames: u64,
    /// Signed alignment offset (`unpaused_wall - declared`) so a run that fell
    /// behind carries on in real time rather than racing to catch up, and a
    /// run resynced after an uncapped burst never sleeps off the burst (#296).
    offset_nanos: i128,
    /// Where the wall and the declared clock were when the pace was last
    /// worked out, and the pace it came to: declared seconds a wall second
    /// (#75).
    paced_from: (u64, u64),
    pace: f64,
    /// When true, `hold` skips sleeping and keeps alignment synced to the
    /// declared clock for fast-burst execution (#296).
    uncapped: bool,
    /// Cumulative wall nanoseconds spent inside intentional pacing sleeps (#349).
    slept_total_nanos: u64,
}

impl Pace {
    /// Pacing from now, at real time until a window of wall time says
    /// otherwise.
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            paused_at: None,
            paused_total_nanos: 0,
            #[cfg(test)]
            simulated: None,
            #[cfg(test)]
            simulated_paused_at: None,
            mixed_frames: 0,
            offset_nanos: 0,
            paced_from: (0, 0),
            pace: 1.0,
            uncapped: false,
            slept_total_nanos: 0,
        }
    }

    /// Pause or resume the host wall clock tracked by this pacer (#296).
    ///
    /// Idempotent: pausing while already paused or resuming while running is a
    /// no-op. Time spent paused is subtracted from [`Self::wall_nanos`] so
    /// resuming never treats pause time as host lag or warps audio stretch.
    pub fn set_paused(&mut self, paused: bool) {
        #[cfg(test)]
        if let Some(wall) = self.simulated {
            if paused {
                if self.simulated_paused_at.is_none() {
                    self.simulated_paused_at = Some(wall);
                }
            } else if let Some(at) = self.simulated_paused_at.take() {
                self.paused_total_nanos += u128::from(wall.saturating_sub(at));
            }
            return;
        }
        if paused {
            if self.paused_at.is_none() {
                self.paused_at = Some(Instant::now());
            }
        } else if let Some(at) = self.paused_at.take() {
            self.paused_total_nanos += at.elapsed().as_nanos();
        }
    }

    /// Whether pacing is currently paused (#296).
    #[must_use]
    pub fn is_paused(&self) -> bool {
        #[cfg(test)]
        if self.simulated.is_some() {
            return self.simulated_paused_at.is_some();
        }
        self.paused_at.is_some()
    }

    /// Cumulative wall nanoseconds spent paused so far (#296).
    #[must_use]
    pub fn paused_total_nanos(&self) -> u128 {
        #[cfg(test)]
        if let Some(wall) = self.simulated {
            let active = self
                .simulated_paused_at
                .map_or(0, |at| u128::from(wall.saturating_sub(at)));
            return self.paused_total_nanos + active;
        }
        let active = self.paused_at.map_or(0, |at| at.elapsed().as_nanos());
        self.paused_total_nanos + active
    }

    /// Enable or disable uncapped fast-burst mode (#296).
    ///
    /// While uncapped, holds do not sleep and continuously resync alignment so
    /// switching back to real-time pacing resumes cleanly at 1.0×.
    pub fn set_uncapped(&mut self, uncapped: bool) {
        self.uncapped = uncapped;
    }

    /// Whether uncapped fast-burst mode is active (#296).
    #[must_use]
    pub fn is_uncapped(&self) -> bool {
        self.uncapped
    }

    /// Re-anchor wall-to-declared alignment to `declared_nanos` at the current
    /// unpaused wall time, resetting settled pace to 1.0× (#296).
    pub fn resync(&mut self, declared_nanos: u64) {
        let wall = self.wall_nanos();
        self.offset_nanos = i128::from(wall) - i128::from(declared_nanos);
        self.paced_from = (wall, declared_nanos);
        self.pace = 1.0;
    }

    /// What [`super::Host::pace`] does on a host with a window: wait until
    /// the declared clock is due, and answer with the pace.
    pub fn hold(&mut self, declared_nanos: u64) -> f64 {
        if self.uncapped {
            self.resync(declared_nanos);
            return 1.0;
        }
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
        let wall_i = i128::from(wall);
        let declared_i = i128::from(declared_nanos);
        self.offset_nanos = self.offset_nanos.max(wall_i - declared_i);
        let target = declared_i + self.offset_nanos;
        u64::try_from((target - wall_i).max(0)).unwrap_or(u64::MAX)
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

    /// The host's own unpaused clock, in nanoseconds since the window opened.
    /// The guest cannot reach this: it paces presentation.
    pub(crate) fn wall_nanos(&self) -> u64 {
        #[cfg(test)]
        if let Some(wall) = self.simulated {
            let effective = self.simulated_paused_at.unwrap_or(wall);
            let unpaused = u128::from(effective).saturating_sub(self.paused_total_nanos);
            return u64::try_from(unpaused).unwrap_or(u64::MAX);
        }
        let elapsed = self.paused_at.map_or_else(
            || self.started.elapsed().as_nanos(),
            |at| at.saturating_duration_since(self.started).as_nanos(),
        );
        let unpaused = elapsed.saturating_sub(self.paused_total_nanos);
        u64::try_from(unpaused).unwrap_or(u64::MAX)
    }

    /// Cumulative wall nanoseconds spent in intentional pacing sleeps (#349).
    #[must_use]
    pub fn slept_total_nanos(&self) -> u64 {
        self.slept_total_nanos
    }

    fn sleep(&mut self, nanos: u64) {
        #[cfg(test)]
        if let Some(wall) = &mut self.simulated {
            *wall += nanos;
            self.slept_total_nanos = self.slept_total_nanos.saturating_add(nanos);
            return;
        }
        let start = Instant::now();
        std::thread::sleep(std::time::Duration::from_nanos(nanos));
        let slept = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.slept_total_nanos = self.slept_total_nanos.saturating_add(slept);
    }
}

impl Default for Pace {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Pace;

    const MS: u64 = 1_000_000;
    const SEC: u64 = 1_000_000_000;

    #[test]
    fn pausing_freezes_wall_clock_and_prevents_audio_warp_on_resume() {
        let mut pace = Pace::new();
        pace.simulated = Some(0);

        // Run a few blocks smoothly at real time.
        for _ in 0..5 {
            *pace.simulated.as_mut().unwrap() += 5 * MS;
            assert_eq!(pace.hold_mixed(), 1.0);
        }
        let before_pause = pace.wall_nanos();

        // Pause for 15 wall seconds (e.g. LLM think time).
        pace.set_paused(true);
        assert!(pace.is_paused());
        pace.set_paused(true); // idempotent
        *pace.simulated.as_mut().unwrap() += 15 * SEC;
        assert_eq!(
            pace.wall_nanos(),
            before_pause,
            "wall clock stays frozen while paused"
        );
        assert_eq!(pace.paused_total_nanos(), u128::from(15 * SEC));

        // Resume: wall_nanos continues seamlessly, hold_mixed stays at 1.0x.
        pace.set_paused(false);
        assert!(!pace.is_paused());
        pace.set_paused(false); // idempotent
        assert_eq!(pace.wall_nanos(), before_pause);
        assert_eq!(pace.paused_total_nanos(), u128::from(15 * SEC));

        for _ in 0..10 {
            *pace.simulated.as_mut().unwrap() += 5 * MS;
            assert_eq!(
                pace.hold_mixed(),
                1.0,
                "resuming after a long pause must not warp pace"
            );
        }
    }

    #[test]
    fn pausing_during_a_slow_stretch_preserves_true_unpaused_pace() {
        let mut pace = Pace::new();
        pace.simulated = Some(0);

        // 200 ms wall for 160 ms declared (0.8x pace, still inside 500 ms window).
        *pace.simulated.as_mut().unwrap() = 200 * MS;
        assert_eq!(pace.hold(160 * MS), 1.0);

        // Pause for 10 seconds mid-window.
        pace.set_paused(true);
        *pace.simulated.as_mut().unwrap() += 10 * SEC;
        pace.set_paused(false);

        // Advance another 300 ms wall for 240 ms declared (total 500 ms wall, 400 ms declared).
        *pace.simulated.as_mut().unwrap() += 300 * MS;
        let settled = pace.hold(400 * MS);
        assert!(
            (settled - 0.8).abs() < 1e-9,
            "expected 0.80x unpaused pace, got {settled}"
        );
    }

    #[test]
    fn resync_reanchors_after_ahead_or_behind_drift() {
        let mut pace = Pace::new();
        pace.simulated = Some(0);

        // Jump declared time 30 s ahead of wall time (e.g. after uncapped burst).
        *pace.simulated.as_mut().unwrap() = SEC;
        pace.resync(30 * SEC);

        // Next frame is 16 ms later in declared time, 4 ms later in wall time -> sleeps 12 ms, not 29 s.
        *pace.simulated.as_mut().unwrap() += 4 * MS;
        let p = pace.hold(30 * SEC + 16 * MS);
        assert_eq!(p, 1.0);
        assert_eq!(pace.wall_nanos(), SEC + 16 * MS);

        // Resync when wall is ahead of declared also resets cleanly to 1.0x.
        *pace.simulated.as_mut().unwrap() += 5 * SEC;
        pace.resync(31 * SEC);
        let wall_after = pace.wall_nanos();
        let p = pace.hold(31 * SEC + 20 * MS);
        assert_eq!(p, 1.0);
        assert_eq!(pace.wall_nanos(), wall_after + 20 * MS);
    }

    #[test]
    fn uncapped_mode_skips_sleeps_and_resumes_real_time_cleanly() {
        let mut pace = Pace::new();
        pace.simulated = Some(0);
        pace.set_uncapped(true);
        assert!(pace.is_uncapped());

        // Simulate 100 frames (1.6 s declared) executed in 100 ms wall time (1 ms/frame).
        for i in 1..=100u64 {
            *pace.simulated.as_mut().unwrap() += MS;
            assert_eq!(pace.hold(i * 16 * MS), 1.0);
        }
        assert_eq!(pace.wall_nanos(), 100 * MS, "no sleep while uncapped");
        assert_eq!(
            pace.slept_total_nanos(),
            0,
            "no sleep accumulated while uncapped"
        );

        // Switch back to capped real-time: next frame sleeps only 15 ms, not 1.5 s.
        pace.set_uncapped(false);
        assert!(!pace.is_uncapped());
        *pace.simulated.as_mut().unwrap() += MS;
        assert_eq!(pace.hold(101 * 16 * MS), 1.0);
        assert_eq!(pace.wall_nanos(), 116 * MS);
        assert_eq!(pace.slept_total_nanos(), 15 * MS);
    }
}
