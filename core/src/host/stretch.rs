//! Stretching the mix so a host that cannot keep pace still fills the device
//! (#75).
//!
//! The mix is made on the declared clock and the device eats it in real time,
//! so a host running at four tenths of real time hands over 16.7 ms of sound
//! every 36 ms and the device runs dry for the rest. That is the crackle.
//! ADR-0003 already names the honest symptom of a throughput problem: slow
//! motion with stretched audio.
//!
//! **Stretched in time, and not in pitch.** Resampling — reading the mix more
//! slowly — fills the device too, but drops every pitch by the same ratio it
//! stretches by. At four tenths of real time that is more than an octave, a
//! voice heard as *"very deep, like its slowed down"*. A pitch that low is the
//! resampler's own artefact, not the guest's sound, which makes it the less
//! faithful of the two as well as the uglier.
//!
//! So instead: overlap-add. Take a window of the mix, lay it over the tail of
//! the last one, and advance through the input more slowly than through the
//! output. What comes out is longer and the same pitch, because every sample
//! is played at the rate it was made at. Where the next window is taken from
//! is searched a little either side for the offset that best continues what
//! is already there, which is what keeps a periodic sound — a voice, a note —
//! from cancelling itself at the seam.

/// Frames the output advances per window, about 12 ms at the mix's rate.
///
/// Long enough that the search below has a period of speech to lock on to,
/// short enough that a sound starting between two windows is not late.
const HOP: usize = 256;

/// Frames in one window. Two hops, so that the cross-fades of consecutive
/// steps sum to one and a steady sound comes out steady.
const WINDOW: usize = HOP * 2;

/// How far either side of where the input is due the next window may be
/// taken from, in frames, to find the offset that best continues the tail.
/// A quarter of a hop is enough for the periods that matter and keeps the
/// search at a few operations a frame.
const SEARCH: usize = HOP / 4;

/// The slowest pace the mix is stretched for. Below this a run is not slow
/// motion, it is stopped, and stretching it would smear one window of sound
/// over a second.
const SLOWEST: f64 = 0.1;

/// A time stretch that leaves pitch alone.
#[derive(Debug, Default)]
pub struct Stretch {
    /// Mix frames taken in and not yet passed under a window, interleaved
    /// stereo as the mixer makes them.
    input: Vec<i16>,
    /// Where the next window is due in `input`, in frames. Fractional,
    /// because the input advances by `HOP * pace`.
    due: f64,
    /// The part of the last window that has still to be overlapped with the
    /// next one: `WINDOW - HOP` frames, interleaved.
    tail: Vec<f32>,
}

impl Stretch {
    /// One block of interleaved stereo, stretched to the length the device
    /// wants at this pace.
    ///
    /// `pace` is declared seconds a wall second. At 1.0 or above the block is
    /// handed back as it is, sample for sample, which is what every host that
    /// keeps up does.
    pub fn block(&mut self, samples: &[i16], pace: f64) -> Vec<i16> {
        if !(pace.is_finite() && pace < 1.0) || samples.is_empty() {
            self.input.clear();
            self.tail.clear();
            self.due = 0.0;
            return samples.to_vec();
        }
        let pace = pace.max(SLOWEST);
        self.input.extend_from_slice(samples);
        if self.tail.is_empty() {
            self.tail = vec![0.0; (WINDOW - HOP) * 2];
        }

        let mut out = Vec::new();
        // A window needs `WINDOW` frames from where it is due, and the search
        // may take it up to `SEARCH` frames later.
        while self.input.len() / 2 >= self.due as usize + WINDOW + SEARCH {
            let from = self.best_offset(self.due as usize);
            let window = &self.input[from * 2..(from + WINDOW) * 2];
            // The first hop of the window lands on the tail and is finished;
            // the rest becomes the new tail.
            let mut next = vec![0.0f32; (WINDOW - HOP) * 2];
            for frame in 0..WINDOW {
                let w = fade(frame);
                for channel in 0..2 {
                    let value = f32::from(window[frame * 2 + channel]) * w;
                    let at = frame * 2 + channel;
                    if frame < HOP {
                        out.push(clamp(self.tail[at] + value));
                    } else {
                        next[at - HOP * 2] = value;
                    }
                }
            }
            self.tail = next;
            self.due += HOP as f64 * pace;
        }

        // Everything before the earliest a window could still be taken from
        // is finished with.
        let keep = (self.due as usize).saturating_sub(SEARCH);
        if keep > 0 {
            self.input.drain(..keep * 2);
            self.due -= keep as f64;
        }
        out
    }

    /// Where to take the next window from: the offset within `SEARCH` of
    /// where it is due whose first hop best continues the tail.
    ///
    /// "Best" is the largest sum of products with the tail, which is the
    /// correlation without its normalising terms — the terms do not change
    /// which offset wins often enough to be worth the divisions.
    fn best_offset(&self, due: usize) -> usize {
        let mut best = (f32::NEG_INFINITY, due);
        for offset in due..=due + SEARCH {
            let mut score = 0.0;
            // Every fourth frame, left channel: a period of speech at this
            // rate is hundreds of frames long, so a quarter of the samples
            // picks the same offset for a quarter of the work.
            for frame in (0..HOP).step_by(4) {
                score += self.tail[frame * 2] * f32::from(self.input[(offset + frame) * 2]);
            }
            if score > best.0 {
                best = (score, offset);
            }
        }
        best.1
    }
}

/// A window that rises across its first hop and falls across its second, so
/// that consecutive steps sum to exactly one: one window's fall is the next
/// one's rise. A straight cross-fade, which is what an overlap-add wants and
/// what keeps this out of the host's maths library — a raised cosine would
/// need `cos`, and the engine gives the same answer on every machine because
/// it never calls one (the gate in `core/tests/x87.rs` holds that).
fn fade(frame: usize) -> f32 {
    let rising = frame.min(WINDOW - frame);
    rising as f32 / HOP as f32
}

fn clamp(value: f32) -> i16 {
    value
        .round()
        .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::Stretch;
    use crate::sound::RATE;

    /// A tone, as the mixer would hand one over: interleaved stereo. A
    /// triangle rather than a sine, because it crosses zero once a period
    /// just as a sine does and needs no maths library (see the gate in
    /// `core/tests/x87.rs`).
    fn tone(hz: f64, frames: usize, from: usize) -> Vec<i16> {
        let period = f64::from(RATE) / hz;
        let mut out = Vec::with_capacity(frames * 2);
        for frame in from..from + frames {
            let turn = (frame as f64 % period) / period;
            // Up from -1 to 1 over the first half, down again over the second.
            let value = ((if turn < 0.5 {
                turn * 4.0 - 1.0
            } else {
                3.0 - turn * 4.0
            }) * 12000.0) as i16;
            out.push(value);
            out.push(value);
        }
        out
    }

    /// How often the left channel crosses zero going up, per second of the
    /// samples given: a tone's own frequency, measured.
    fn hz_of(samples: &[i16]) -> f64 {
        let left: Vec<i16> = samples.iter().step_by(2).copied().collect();
        let crossings = left
            .windows(2)
            .filter(|pair| pair[0] <= 0 && pair[1] > 0)
            .count();
        crossings as f64 * f64::from(RATE) / left.len() as f64
    }

    #[test]
    fn a_host_that_keeps_pace_hands_the_device_exactly_what_it_made() {
        let mut stretch = Stretch::default();
        let block: Vec<i16> = (0..64).collect();
        for pace in [1.0, 1.5, f64::NAN] {
            assert_eq!(stretch.block(&block, pace), block, "pace {pace}");
        }
    }

    /// The whole point of this over a resampler: the sound comes out longer
    /// and at the pitch it went in at (#75, after a person said the voice was
    /// "very deep, like its slowed down").
    #[test]
    fn a_stretched_tone_keeps_its_own_pitch() {
        for pace in [0.8, 0.45, 0.25] {
            let mut stretch = Stretch::default();
            let mut out = Vec::new();
            let mut sent = 0;
            for block in 0..40 {
                let samples = tone(440.0, 1024, block * 1024);
                sent += samples.len() / 2;
                out.extend(stretch.block(&samples, pace));
            }
            let got = out.len() / 2;
            let want = sent as f64 / pace;
            // A window and its search are always held back, waiting to be
            // laid over what is already out, so the device is behind by that
            // much for as long as the stretch runs: about 64 ms at 0.45x,
            // against the 139 ms the queue waits for before it plays at all.
            let lag = (super::WINDOW + super::SEARCH) as f64 / pace + super::WINDOW as f64;
            assert!(
                want - got as f64 > 0.0 && want - got as f64 <= lag,
                "pace {pace}: {got} frames where the device wants {want:.0}, lag allowed {lag:.0}"
            );
            let heard = hz_of(&out);
            assert!(
                (heard - 440.0).abs() < 8.0,
                "pace {pace}: 440 Hz came out at {heard:.0} Hz"
            );
        }
    }

    /// And a resampler's answer, for contrast: the frequency would fall by
    /// the pace. This is the bug that was heard, written down as a test so
    /// that going back to one fails here.
    #[test]
    fn the_pitch_does_not_follow_the_pace() {
        let mut stretch = Stretch::default();
        let mut out = Vec::new();
        for block in 0..40 {
            out.extend(stretch.block(&tone(440.0, 1024, block * 1024), 0.42));
        }
        let heard = hz_of(&out);
        assert!(
            heard > 300.0,
            "a resampler at 0.42x would put 440 Hz at about 185 Hz; this is {heard:.0} Hz"
        );
    }

    #[test]
    fn a_pace_below_the_slowest_is_stretched_no_further() {
        let block = tone(440.0, 4096, 0);
        let mut slowest = Stretch::default();
        let mut slower = Stretch::default();
        assert_eq!(
            slowest.block(&block, 0.1).len(),
            slower.block(&block, 0.001).len()
        );
    }
}
