//! The machine's one audio stream, and the sample formats that feed it.
//!
//! Every sound a title makes is mixed here on the declared clock, whether or
//! not a host device is listening (ADR-0003): a block is due when the declared
//! clock passes its end, so what is mixed is a function of the route alone.

/// The mix's sample rate: what the roster's first title records its own
/// sound at, movies and effects alike, so nothing it plays is resampled.
pub const RATE: u32 = 22_050;

/// Frames in one mixed block: about 46 milliseconds, short enough that a
/// host device fed block by block stays close behind the picture.
pub const BLOCK_FRAMES: usize = 1024;

/// Why sound data did not decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sound: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// A `WAVEFORMATEX`, and the MS-ADPCM extension when there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Format {
    pub tag: u16,
    pub channels: u16,
    pub rate: u32,
    pub block_align: u16,
    pub bits: u16,
    /// MS-ADPCM's predictor coefficient pairs.
    pub coefficients: Vec<(i32, i32)>,
}

impl Format {
    /// Read a `WAVEFORMATEX` as a stream or a file stores it.
    ///
    /// # Errors
    /// When the block is too short for what it says it is.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let u16_at = |at: usize| {
            bytes
                .get(at..at + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .ok_or_else(|| Error("a short wave format".to_owned()))
        };
        let mut format = Self {
            tag: u16_at(0)?,
            channels: u16_at(2)?,
            rate: u32::from(u16_at(4)?) | (u32::from(u16_at(6)?) << 16),
            block_align: u16_at(12)?,
            bits: u16_at(14)?,
            coefficients: Vec::new(),
        };
        if format.tag == 2 {
            let count = usize::from(u16_at(20)?);
            for i in 0..count {
                let a = i32::from(u16_at(22 + i * 4)? as i16);
                let b = i32::from(u16_at(24 + i * 4)? as i16);
                format.coefficients.push((a, b));
            }
        }
        Ok(format)
    }

    /// Decode samples in this format into stereo at the mix's rate.
    ///
    /// # Errors
    /// When the format is one nothing here decodes, or the data is corrupt.
    pub fn decode(&self, data: &[u8]) -> Result<Vec<i16>, Error> {
        if self.rate != RATE {
            return Err(Error(format!("{} Hz, and the mix is at {RATE}", self.rate)));
        }
        self.stereo(data)
    }

    /// Decode samples in this format into stereo at the format's own rate.
    ///
    /// # Errors
    /// When the format is one nothing here decodes, or the data is corrupt.
    pub fn stereo(&self, data: &[u8]) -> Result<Vec<i16>, Error> {
        if !(1..=2).contains(&self.channels) {
            return Err(Error(format!(
                "{} channels, and the mix is stereo",
                self.channels
            )));
        }
        let samples = match (self.tag, self.bits) {
            (1, 16) => data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| i16::from_le_bytes(*b))
                .collect(),
            (1, 8) => data.iter().map(|b| (i16::from(*b) - 128) << 8).collect(),
            (2, 4) => self.ms_adpcm(data)?,
            (tag, bits) => {
                return Err(Error(format!(
                    "format tag {tag} at {bits} bits, which nothing here decodes"
                )))
            }
        };
        Ok(if self.channels == 1 {
            samples.iter().flat_map(|s| [*s, *s]).collect()
        } else {
            samples
        })
    }

    /// Microsoft's ADPCM: each block starts with its predictors and first two
    /// samples, and every sample after is a nibble.
    fn ms_adpcm(&self, data: &[u8]) -> Result<Vec<i16>, Error> {
        const ADAPTATION: [i32; 16] = [
            230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
        ];
        let channels = usize::from(self.channels);
        let align = usize::from(self.block_align);
        if align < 7 * channels {
            return Err(Error(
                "an MS-ADPCM block too short for its header".to_owned(),
            ));
        }
        let mut out = Vec::new();
        for block in data.chunks(align).filter(|b| b.len() == align) {
            let mut at = 0;
            let mut byte = || {
                at += 1;
                block[at - 1]
            };
            let mut coef = [(0, 0); 2];
            for c in coef.iter_mut().take(channels) {
                let index = usize::from(byte());
                *c = *self
                    .coefficients
                    .get(index)
                    .ok_or_else(|| Error(format!("predictor {index} past the table")))?;
            }
            let mut word = || i32::from(i16::from_le_bytes([byte(), byte()]));
            let mut delta = [0i32; 2];
            let mut s1 = [0i32; 2];
            let mut s2 = [0i32; 2];
            for d in delta.iter_mut().take(channels) {
                *d = word();
            }
            for s in s1.iter_mut().take(channels) {
                *s = word();
            }
            for s in s2.iter_mut().take(channels) {
                *s = word();
            }
            out.extend(s2.iter().take(channels).map(|s| *s as i16));
            out.extend(s1.iter().take(channels).map(|s| *s as i16));
            let header = 7 * channels;
            for &packed in &block[header..] {
                for (n, nibble) in [packed >> 4, packed & 0x0f].into_iter().enumerate() {
                    let c = if channels == 2 { n } else { 0 };
                    let signed = i32::from(nibble) - if nibble & 8 != 0 { 16 } else { 0 };
                    let predicted =
                        (s1[c] * coef[c].0 + s2[c] * coef[c].1) / 256 + signed * delta[c];
                    let sample = predicted.clamp(-32768, 32767);
                    s2[c] = s1[c];
                    s1[c] = sample;
                    delta[c] = ((ADAPTATION[usize::from(nibble)] * delta[c]) >> 8).max(16);
                    out.push(sample as i16);
                }
            }
        }
        Ok(out)
    }
}

/// One sound playing.
struct Voice {
    id: u32,
    /// Whether it goes round again at its end, until it is stopped.
    looping: bool,
    /// Stereo samples at the mix's rate.
    samples: Vec<i16>,
    /// The frame of the mix its first sample lands on.
    start: u64,
}

/// Everything playing, and how much of the mix has been handed out.
#[derive(Default)]
pub struct Mixer {
    voices: Vec<Voice>,
    blocks: u64,
}

/// The mix frame a declared time falls in.
#[must_use]
pub fn frame_at(nanos: u64) -> u64 {
    u64::try_from(u128::from(nanos) * u128::from(RATE) / 1_000_000_000).unwrap_or(u64::MAX)
}

impl Mixer {
    /// Start a sound at a declared time, replacing one with the same id.
    pub fn play(&mut self, id: u32, samples: Vec<i16>, nanos: u64) {
        self.play_from(id, samples, nanos, false);
    }

    /// Start a sound at a declared time, once or round and round.
    pub fn play_from(&mut self, id: u32, samples: Vec<i16>, nanos: u64, looping: bool) {
        self.stop(id);
        if samples.len() < 2 {
            return;
        }
        self.voices.push(Voice {
            id,
            looping,
            samples,
            start: frame_at(nanos),
        });
    }

    pub fn stop(&mut self, id: u32) {
        self.voices.retain(|v| v.id != id);
    }

    /// Every block whose last frame the declared clock has passed.
    pub fn due(&mut self, nanos: u64) -> Vec<Vec<i16>> {
        let mut out = Vec::new();
        let now = frame_at(nanos);
        while (self.blocks + 1) * BLOCK_FRAMES as u64 <= now {
            let first = self.blocks * BLOCK_FRAMES as u64;
            let mut block = vec![0i32; BLOCK_FRAMES * 2];
            for voice in &self.voices {
                for (frame, pair) in block.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                    let Some(offset) = (first + frame as u64).checked_sub(voice.start) else {
                        continue;
                    };
                    let frames = (voice.samples.len() / 2) as u64;
                    let offset = if voice.looping {
                        offset % frames
                    } else {
                        offset
                    };
                    let at = offset as usize * 2;
                    if let Some(s) = voice.samples.get(at..at + 2) {
                        pair[0] += i32::from(s[0]);
                        pair[1] += i32::from(s[1]);
                    }
                }
            }
            out.push(
                block
                    .iter()
                    .map(|s| (*s).clamp(-32768, 32767) as i16)
                    .collect(),
            );
            self.blocks += 1;
            let end = self.blocks * BLOCK_FRAMES as u64;
            self.voices
                .retain(|v| v.looping || v.start + (v.samples.len() / 2) as u64 > end);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{Format, Mixer, BLOCK_FRAMES, RATE};

    fn pcm(channels: u16) -> Format {
        Format {
            tag: 1,
            channels,
            rate: RATE,
            block_align: channels * 2,
            bits: 16,
            coefficients: Vec::new(),
        }
    }

    #[test]
    fn mono_is_played_on_both_sides() {
        let data = [1u8, 0, 2, 0];
        assert_eq!(pcm(1).decode(&data), Ok(vec![1, 1, 2, 2]));
    }

    #[test]
    fn a_rate_the_mix_is_not_is_refused_rather_than_played_at_the_wrong_speed() {
        let format = Format {
            rate: 44_100,
            ..pcm(2)
        };
        assert!(format.decode(&[0, 0, 0, 0]).is_err());
    }

    #[test]
    fn a_block_is_due_once_the_clock_passes_its_end_and_sounds_add() {
        let mut mixer = Mixer::default();
        mixer.play(1, vec![100; BLOCK_FRAMES * 4], 0);
        mixer.play(2, vec![-30; BLOCK_FRAMES * 4], 0);
        let block_nanos = BLOCK_FRAMES as u64 * 1_000_000_000 / u64::from(RATE);
        assert!(mixer.due(block_nanos - 1_000_000).is_empty());
        let blocks = mixer.due(block_nanos + 1_000_000);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].iter().all(|s| *s == 70));
        mixer.stop(1);
        let blocks = mixer.due(3 * block_nanos);
        assert!(blocks
            .last()
            .expect("more blocks")
            .iter()
            .all(|s| *s == -30));
    }
}
