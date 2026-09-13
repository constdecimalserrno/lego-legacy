/// Stretching the mix so a host that cannot keep pace still fills the device
/// (#75).
///
/// The mix is made on the declared clock and the device eats it in real time,
/// so a host running at four tenths of real time hands over 16.7 ms of sound
/// every 36 ms and the device runs dry for the rest. That is the crackle.
/// ADR-0003 already names the honest symptom of a throughput problem: slow
/// motion with stretched audio. This is the stretching half.
///
/// Linear between the two input frames either side of each output frame, with
/// the last frame of one block and the fraction left over carried into the
/// next, so a block boundary is not a click.
#[derive(Debug, Default)]
pub struct Stretch {
    /// The last frame of the block before, which the first output frame of
    /// this one may sit between.
    last: (i16, i16),
    /// Where the next output frame sits, in input frames from the start of
    /// the next block. Always less than one.
    phase: f64,
}

/// The slowest pace the mix is stretched for. Below this a run is not slow
/// motion, it is stopped, and stretching it would smear one frame of sound
/// over a second.
const SLOWEST: f64 = 0.1;

impl Stretch {
    /// One block of interleaved stereo, stretched to the length the device
    /// wants at this pace.
    ///
    /// `pace` is declared seconds a wall second. At 1.0 or above the block is
    /// handed back as it is, byte for byte, which is what every host that
    /// keeps up does.
    pub fn block(&mut self, samples: &[i16], pace: f64) -> Vec<i16> {
        let frames = samples.len() / 2;
        if !(pace.is_finite() && pace < 1.0) || frames == 0 {
            self.reset(samples);
            return samples.to_vec();
        }
        // Input frames a device frame: below one, so more come out than went
        // in.
        let step = pace.max(SLOWEST);
        let frame = |i: isize| -> (f64, f64) {
            let i = i.clamp(-1, frames as isize - 1);
            if i < 0 {
                (f64::from(self.last.0), f64::from(self.last.1))
            } else {
                let at = i as usize * 2;
                (f64::from(samples[at]), f64::from(samples[at + 1]))
            }
        };
        let mut out = Vec::with_capacity(((frames as f64 / step) as usize + 2) * 2);
        let mut at = self.phase;
        while at < frames as f64 {
            let whole = at.floor();
            let fraction = at - whole;
            let (a, b) = (frame(whole as isize - 1), frame(whole as isize));
            out.push(mix(a.0, b.0, fraction));
            out.push(mix(a.1, b.1, fraction));
            at += step;
        }
        self.phase = at - frames as f64;
        self.last = (samples[(frames - 1) * 2], samples[(frames - 1) * 2 + 1]);
        out
    }

    /// Start again from the end of a block that was not stretched, so that
    /// the first stretched block after it carries on from real sound.
    fn reset(&mut self, samples: &[i16]) {
        self.phase = 0.0;
        if samples.len() >= 2 {
            self.last = (samples[samples.len() - 2], samples[samples.len() - 1]);
        }
    }
}

/// One sample between two, rounded and kept inside the range.
fn mix(a: f64, b: f64, fraction: f64) -> i16 {
    let v = a + (b - a) * fraction;
    v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

#[cfg(test)]
mod tests {
    use super::Stretch;

    #[test]
    fn a_host_that_keeps_pace_hands_the_device_exactly_what_it_made() {
        let mut stretch = Stretch::default();
        let block: Vec<i16> = (0..64).collect();
        for pace in [1.0, 1.5, f64::NAN] {
            assert_eq!(stretch.block(&block, pace), block, "pace {pace}");
        }
    }

    #[test]
    fn a_slow_host_gets_as_many_frames_as_the_device_wants() {
        // Over a run of blocks, because the fraction left over at the end of
        // one block is what makes the next one come out right.
        let block: Vec<i16> = (0..2048).collect();
        for pace in [0.8, 0.45, 0.25] {
            let mut stretch = Stretch::default();
            let mut got = 0.0;
            for _ in 0..8 {
                got += (stretch.block(&block, pace).len() / 2) as f64;
            }
            let want = 8.0 * (block.len() / 2) as f64 / pace;
            assert!(
                (got - want).abs() <= 1.0,
                "pace {pace}: {got} frames where the device wants {want}"
            );
        }
    }

    #[test]
    fn the_stretched_mix_is_continuous_across_blocks() {
        // A ramp broken into blocks comes out a ramp: no step at a block
        // boundary, which is what a click would be.
        let mut stretch = Stretch::default();
        let mut out = Vec::new();
        for block in 0..4u16 {
            let samples: Vec<i16> = (0..32)
                .map(|i| i16::try_from(block * 32 + i).expect("small"))
                .collect();
            out.extend(stretch.block(&samples, 0.5));
        }
        let left: Vec<i16> = out.iter().step_by(2).copied().collect();
        for pair in left.windows(2) {
            let step = i32::from(pair[1]) - i32::from(pair[0]);
            assert!(
                (0..=2).contains(&step),
                "a ramp stretched is a ramp: {pair:?} steps by {step}"
            );
        }
        assert!(left.len() > 100, "the ramp was stretched, not dropped");
    }

    #[test]
    fn a_pace_below_the_slowest_is_stretched_no_further() {
        let mut stretch = Stretch::default();
        let block: Vec<i16> = (0..64).collect();
        let slowest = stretch.block(&block, 0.1).len();
        let mut other = Stretch::default();
        assert_eq!(other.block(&block, 0.001).len(), slowest);
    }
}
