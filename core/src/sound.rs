//! The machine's one audio stream, and the sample formats that feed it.
//!
//! Every sound a title makes is mixed here on the declared clock, whether or
//! not a host device is listening (ADR-0003): a block is due when the declared
//! clock passes its end, so what is mixed is a function of the route alone.

use crate::guest::GuestMem;

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
    fn stereo(&self, data: &[u8]) -> Result<Vec<i16>, Error> {
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
    pub(crate) fn ms_adpcm(&self, data: &[u8]) -> Result<Vec<i16>, Error> {
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
                    // Wrapping where a step can run past 32 bits, as the
                    // codec's own `int` arithmetic does, on corrupt data only.
                    let predicted = ((s1[c] * coef[c].0 + s2[c] * coef[c].1) / 256)
                        .wrapping_add(signed.wrapping_mul(delta[c]));
                    let sample = predicted.clamp(-32768, 32767);
                    s2[c] = s1[c];
                    s1[c] = sample;
                    delta[c] =
                        (ADAPTATION[usize::from(nibble)].wrapping_mul(delta[c]) >> 8).max(16);
                    out.push(sample as i16);
                }
            }
        }
        Ok(out)
    }
}

/// Sixteen- or eight-bit PCM a title keeps in guest memory, read as the mix
/// reaches it: a title may go on writing into a buffer while it plays, as a
/// streamed sound does, and what it wrote is what is heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pcm {
    /// Where its bytes are in guest memory, and how many there are.
    store: u32,
    bytes: u32,
    channels: u16,
    bits: u16,
    /// The rate it plays at.
    rate: u32,
    /// How loud it is on each side.
    left: f64,
    right: f64,
}

impl Pcm {
    /// A buffer's bytes in their format, played at a rate and weighed on each
    /// side.
    ///
    /// # Errors
    /// When the format is not PCM a buffer holds: eight or sixteen bits, in
    /// one channel or two, each frame as wide as its samples.
    pub fn new(
        store: u32,
        bytes: u32,
        format: &Format,
        rate: u32,
        (left, right): (f64, f64),
    ) -> Result<Self, Error> {
        let pcm = Self {
            store,
            bytes,
            channels: format.channels,
            bits: format.bits,
            rate: rate.max(1),
            left,
            right,
        };
        if format.tag != 1
            || ![8, 16].contains(&format.bits)
            || !(1..=2).contains(&format.channels)
            || u32::from(format.block_align) != pcm.align()
        {
            return Err(Error(format!(
                "format tag {} at {} bits in {} channels and {} bytes a frame, which is not PCM",
                format.tag, format.bits, format.channels, format.block_align
            )));
        }
        Ok(pcm)
    }

    fn align(&self) -> u32 {
        u32::from(self.channels) * u32::from(self.bits / 8)
    }

    /// Both sides of one of its frames, read out of a pre-borrowed slice of
    /// the buffer's bytes.
    #[inline(always)]
    fn frame_from(&self, buf: &[u8], n: u64) -> [i16; 2] {
        let align = self.align() as usize;
        let off = n as usize * align;
        let bytes = &buf[off..off + align];
        let sample = |i: usize| match self.bits {
            16 => i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]),
            _ => (i16::from(bytes[i]) - 128) << 8,
        };
        [sample(0), sample(usize::from(self.channels > 1))]
    }

    /// Both sides of one of its frames, as guest memory holds it now.
    fn frame(&self, mem: &dyn GuestMem, n: u64) -> Option<[i16; 2]> {
        let align = self.align();
        let bytes = mem
            .slice(self.store + n as u32 * align, align as usize)
            .ok()?;
        let sample = |i: usize| match self.bits {
            16 => i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]),
            // Eight bits, the only other width a buffer is made with.
            _ => (i16::from(bytes[i]) - 128) << 8,
        };
        Some([sample(0), sample(usize::from(self.channels > 1))])
    }
}

/// What a voice plays.
enum Source {
    /// Stereo samples at the mix's rate, decoded before it started.
    Decoded(Vec<i16>),
    /// A title's own buffer, read as it plays.
    Guest(Pcm),
}

/// One sound playing.
struct Voice {
    id: u32,
    /// Whether it goes round again at its end, until it is stopped.
    looping: bool,
    source: Source,
    /// The frame of the mix it starts at, and where in its own frames it
    /// starts from, in the mix's frames: a voice that goes on from another
    /// starts part of the way through one of them (#99).
    start: u64,
    first: u64,
    /// The frame of the mix it stops at, once something has stopped it.
    end: Option<u64>,
}

impl Voice {
    fn frames(&self) -> u64 {
        match &self.source {
            Source::Decoded(samples) => (samples.len() / 2) as u64,
            Source::Guest(pcm) => u64::from(pcm.bytes / pcm.align()),
        }
    }

    fn rate(&self) -> u64 {
        match &self.source {
            Source::Decoded(_) => u64::from(RATE),
            Source::Guest(pcm) => u64::from(pcm.rate),
        }
    }

    /// Where a frame of the mix falls in the sound: its own frame, and the
    /// part of a frame past it, in the mix's frames.
    #[inline(always)]
    fn along(&self, frame: u64) -> (u64, u64) {
        let along = self.first + frame.saturating_sub(self.start) * self.rate();
        (along / u64::from(RATE), along % u64::from(RATE))
    }

    /// The pair of samples a frame of the mix hears from this voice, if any.
    fn at(&self, frame: u64, mem: &dyn GuestMem) -> Option<[i16; 2]> {
        if frame < self.start || self.end.is_some_and(|end| frame >= end) {
            return None;
        }
        // Never zero: a voice with nothing to play is never added.
        let frames = self.frames();
        let (whole, part) = self.along(frame);
        let (this, next) = if self.looping {
            (whole % frames, (whole + 1) % frames)
        } else if whole < frames {
            (whole, (whole + 1).min(frames - 1))
        } else {
            return None;
        };
        match &self.source {
            Source::Decoded(samples) => {
                let i = this as usize * 2;
                Some([samples[i], samples[i + 1]])
            }
            // Drawn straight between the two frames either side, and weighed.
            Source::Guest(pcm) => {
                let (a, b) = (pcm.frame(mem, this)?, pcm.frame(mem, next)?);
                let part = part as f64 / f64::from(RATE);
                let side = |i: usize, weight: f64| {
                    ((f64::from(a[i]) * (1.0 - part) + f64::from(b[i]) * part) * weight)
                        .round()
                        .clamp(-32768.0, 32767.0) as i16
                };
                Some([side(0, pcm.left), side(1, pcm.right)])
            }
        }
    }

    /// Mix `start_frame..start_frame + accum.len()` of this voice into `accum`,
    /// borrowing a guest PCM slice once for the whole chunk when in bounds and
    /// skipping the unused `next` read whenever `part == 0`.
    fn mix_into(&self, start_frame: u64, accum: &mut [[i32; 2]], mem: &dyn GuestMem) {
        let end_frame = start_frame + accum.len() as u64;
        let v_start = start_frame.max(self.start);
        let v_end = self.end.map_or(end_frame, |end| end_frame.min(end));
        if v_start >= v_end {
            return;
        }
        // Never zero: a voice with nothing to play is never added.
        let frames = self.frames();
        let offset = (v_start - start_frame) as usize;
        let count = (v_end - v_start) as usize;
        match &self.source {
            Source::Decoded(samples) => {
                for (i, dst) in accum[offset..offset + count].iter_mut().enumerate() {
                    let frame = v_start + i as u64;
                    let (whole, _) = self.along(frame);
                    let this = if self.looping {
                        whole % frames
                    } else if whole < frames {
                        whole
                    } else {
                        break;
                    };
                    let idx = this as usize * 2;
                    dst[0] += i32::from(samples[idx]);
                    dst[1] += i32::from(samples[idx + 1]);
                }
            }
            Source::Guest(pcm) => {
                let valid_bytes = frames as usize * pcm.align() as usize;
                if let Ok(bytes) = mem.slice(pcm.store, valid_bytes) {
                    let unit_weight = pcm.left == 1.0 && pcm.right == 1.0;
                    for (i, dst) in accum[offset..offset + count].iter_mut().enumerate() {
                        let frame = v_start + i as u64;
                        let (whole, part) = self.along(frame);
                        let this = if self.looping {
                            whole % frames
                        } else if whole < frames {
                            whole
                        } else {
                            break;
                        };
                        let a = pcm.frame_from(bytes, this);
                        let [l, r] = if part == 0 {
                            if unit_weight {
                                a
                            } else {
                                let side = |s: i16, weight: f64| {
                                    (f64::from(s) * weight).round().clamp(-32768.0, 32767.0) as i16
                                };
                                [side(a[0], pcm.left), side(a[1], pcm.right)]
                            }
                        } else {
                            let next = if self.looping {
                                (this + 1) % frames
                            } else {
                                (this + 1).min(frames - 1)
                            };
                            let b = pcm.frame_from(bytes, next);
                            let part = part as f64 / f64::from(RATE);
                            let inv = 1.0 - part;
                            let side = |c: usize, weight: f64| {
                                ((f64::from(a[c]) * inv + f64::from(b[c]) * part) * weight)
                                    .round()
                                    .clamp(-32768.0, 32767.0) as i16
                            };
                            [side(0, pcm.left), side(1, pcm.right)]
                        };
                        dst[0] += i32::from(l);
                        dst[1] += i32::from(r);
                    }
                } else {
                    for (i, dst) in accum[offset..offset + count].iter_mut().enumerate() {
                        if let Some(s) = self.at(v_start + i as u64, mem) {
                            dst[0] += i32::from(s[0]);
                            dst[1] += i32::from(s[1]);
                        }
                    }
                }
            }
        }
    }

    /// Whether nothing from this frame of the mix on hears it.
    fn over(&self, frame: u64) -> bool {
        self.end.is_some_and(|end| frame >= end)
            || (!self.looping && self.along(frame).0 >= self.frames())
    }
}

/// Everything playing, and how much of the mix has been mixed.
#[derive(Default)]
pub struct Mixer {
    voices: Vec<Voice>,
    /// The machine's MIDI out, once it has one (ADR-0008).
    synth: Option<crate::synth::Synth>,
    /// Blocks mixed whole, the block being mixed, and whole blocks not yet
    /// handed out.
    blocks: u64,
    block: Vec<i16>,
    ready: Vec<Vec<i16>>,
}

/// Reduced fraction `RATE / 1_000_000_000 = 22_050 / 1_000_000_000 = 441 / 20_000_000`.
const FRAME_NUM: u64 = 441;
const FRAME_DEN: u64 = 20_000_000;
const _: () = assert!(FRAME_NUM * 50 == RATE as u64 && FRAME_DEN * 50 == 1_000_000_000);

/// The mix frame a declared time falls in.
#[inline]
#[must_use]
pub fn frame_at(nanos: u64) -> u64 {
    if let Some(prod) = nanos.checked_mul(FRAME_NUM) {
        prod / FRAME_DEN
    } else {
        frame_at_wide(nanos)
    }
}

#[cold]
#[inline(never)]
fn frame_at_wide(nanos: u64) -> u64 {
    u64::try_from(u128::from(nanos) * u128::from(RATE) / 1_000_000_000).unwrap_or(u64::MAX)
}

impl Mixer {
    /// Start a sound at a declared time, from its beginning, once.
    pub fn play(&mut self, id: u32, samples: Vec<i16>, nanos: u64) {
        self.add(id, Source::Decoded(samples), 0, nanos, false);
    }

    /// Start a title's buffer at a declared time from one of its own frames,
    /// once or round and round. A sound already playing under the same id
    /// stops where this one starts.
    pub fn play_buffer(&mut self, id: u32, pcm: Pcm, first: u64, nanos: u64, looping: bool) {
        let first = first * u64::from(RATE);
        self.add(id, Source::Guest(pcm), first, nanos, looping);
    }

    /// Go on playing a title's buffer at a declared time, changed, from just
    /// where its voice has got to, to the part of a frame: a change to how it
    /// sounds moves nothing else (#99). A voice that is over stays over.
    pub fn go_on(&mut self, id: u32, pcm: Pcm, nanos: u64, looping: bool) {
        let at = frame_at(nanos);
        let Some(voice) = self.voices.iter().find(|v| v.id == id && !v.over(at)) else {
            return;
        };
        let (mut whole, part) = voice.along(at);
        if voice.looping {
            whole %= voice.frames();
        }
        let first = whole * u64::from(RATE) + part;
        self.add(id, Source::Guest(pcm), first, nanos, looping);
    }

    fn add(&mut self, id: u32, source: Source, first: u64, nanos: u64, looping: bool) {
        self.stop(id, nanos);
        let voice = Voice {
            id,
            looping,
            source,
            start: frame_at(nanos),
            first,
            end: None,
        };
        if voice.frames() > 0 {
            self.voices.push(voice);
        }
    }

    /// Give the mix a General MIDI synth, which every frame mixed from here
    /// on hears (ADR-0008).
    pub fn open_synth(&mut self, synth: crate::synth::Synth) {
        self.synth = Some(synth);
    }

    /// A short MIDI message, packed as `midiOutShortMsg` takes one, heard
    /// from the frame of the mix a declared time falls in, or from the next
    /// frame mixed if the mix is past it; and whether a synth heard it. A
    /// machine without the player's bank has no MIDI device to send through
    /// (ADR-0008), so a message no synth hears is never dropped as if heard:
    /// the device that sent it stops the run at a wall (#154).
    #[must_use]
    pub fn midi(&mut self, nanos: u64, message: u32) -> bool {
        let Some(synth) = &mut self.synth else {
            return false;
        };
        synth.message(frame_at(nanos), message);
        true
    }

    /// Whether the machine has a MIDI out: a synth, which only the player's
    /// bank gives it (ADR-0008).
    #[must_use]
    pub fn midi_out(&self) -> bool {
        self.synth.is_some()
    }

    /// Set the synth's own volume at a declared time, an attenuation in
    /// 1/65,536 centibels: what the mix has not reached by then is heard at
    /// the new volume, and what it has, was heard at the old. Without a synth
    /// there is nothing to set.
    pub fn midi_volume(&mut self, nanos: u64, mem: &dyn GuestMem, attenuation: i64) {
        self.catch_up(nanos, mem);
        if let Some(synth) = &mut self.synth {
            synth.volume(attenuation);
        }
    }

    /// Stop a sound at a declared time: what the mix has not reached by then
    /// is not heard, and what it has, was.
    pub fn stop(&mut self, id: u32, nanos: u64) {
        let at = frame_at(nanos);
        for voice in self
            .voices
            .iter_mut()
            .filter(|v| v.id == id && v.end.is_none())
        {
            voice.end = Some(at);
        }
    }

    /// The mix frame index that will be mixed next.
    #[inline(always)]
    #[must_use]
    pub fn next_frame(&self) -> u64 {
        self.blocks * BLOCK_FRAMES as u64 + (self.block.len() / 2) as u64
    }

    /// Number of currently active (non-finished) voices in the mixer (#309).
    #[must_use]
    pub fn active_voice_count(&self) -> usize {
        let frame = self.next_frame();
        self.voices.iter().filter(|v| !v.over(frame)).count()
    }

    /// Every block whose last frame the declared clock has passed.
    pub fn due(&mut self, nanos: u64, mem: &dyn GuestMem) -> Vec<Vec<i16>> {
        self.catch_up(nanos, mem);
        std::mem::take(&mut self.ready)
    }

    /// Bulk zero-fill `remaining` frames when neither voices nor a synth can
    /// contribute to the mix.
    fn fill_silence(&mut self, mut remaining: u64) {
        let block = BLOCK_FRAMES as u64;
        let in_block = (self.block.len() / 2) as u64;
        let space = block - in_block;
        if remaining < space {
            self.block
                .resize(self.block.len() + remaining as usize * 2, 0);
            return;
        }
        self.block.resize(BLOCK_FRAMES * 2, 0);
        let whole = std::mem::replace(&mut self.block, Vec::with_capacity(BLOCK_FRAMES * 2));
        self.ready.push(whole);
        self.blocks += 1;
        remaining -= space;

        let full_blocks = remaining / block;
        for _ in 0..full_blocks {
            self.ready.push(vec![0i16; BLOCK_FRAMES * 2]);
        }
        self.blocks += full_blocks;
        remaining %= block;

        if remaining > 0 {
            self.block.resize(remaining as usize * 2, 0);
        }
    }

    fn mix_chunk(&mut self, start_frame: u64, accum: &mut [[i32; 2]], mem: &dyn GuestMem) {
        for voice in &self.voices {
            voice.mix_into(start_frame, accum, mem);
        }
        if let Some(synth) = &mut self.synth {
            for (i, dst) in accum.iter_mut().enumerate() {
                let [left, right] = synth.frame(start_frame + i as u64);
                dst[0] = dst[0].saturating_add(left);
                dst[1] = dst[1].saturating_add(right);
            }
        }
        self.block.reserve(accum.len() * 2);
        for &[l, r] in accum.iter() {
            self.block.push(l.clamp(-32768, 32767) as i16);
            self.block.push(r.clamp(-32768, 32767) as i16);
        }
    }

    /// Mix every frame the declared clock has passed. A title about to write
    /// into a buffer has what the buffer already played mixed first, so what
    /// it writes is heard from when it wrote it and never in place of what
    /// was there.
    pub fn catch_up(&mut self, nanos: u64, mem: &dyn GuestMem) {
        let now = frame_at(nanos);
        let block = BLOCK_FRAMES as u64;
        loop {
            let frame = self.next_frame();
            if frame >= now {
                break;
            }
            if self.voices.is_empty() && self.synth.is_none() {
                self.fill_silence(now - frame);
                break;
            }
            let target = now.min((self.blocks + 1) * block);
            let count = (target - frame) as usize;
            if self.voices.is_empty() {
                if let Some(synth) = &mut self.synth {
                    self.block.reserve(count * 2);
                    for f in frame..target {
                        let [left, right] = synth.frame(f);
                        self.block.push(left.clamp(-32768, 32767) as i16);
                        self.block.push(right.clamp(-32768, 32767) as i16);
                    }
                }
            } else if count <= 64 {
                let mut accum = [[0i32; 2]; 64];
                self.mix_chunk(frame, &mut accum[..count], mem);
            } else {
                let mut accum = [[0i32; 2]; BLOCK_FRAMES];
                self.mix_chunk(frame, &mut accum[..count], mem);
            }
            if self.block.len() == BLOCK_FRAMES * 2 {
                let whole =
                    std::mem::replace(&mut self.block, Vec::with_capacity(BLOCK_FRAMES * 2));
                self.ready.push(whole);
                self.blocks += 1;
                let end = self.blocks * block;
                self.voices.retain(|v| !v.over(end));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{frame_at, Format, Mixer, Pcm, BLOCK_FRAMES, RATE};
    use crate::guest::{Arena, GuestMem};
    use crate::host::headless::Headless;

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
        let mem = guest(&[]);
        let mut mixer = Mixer::default();
        mixer.play(1, vec![100; BLOCK_FRAMES * 4], 0);
        mixer.play(2, vec![-30; BLOCK_FRAMES * 4], 0);
        assert!(mixer.due(BLOCK_NANOS - 1_000_000, &mem).is_empty());
        let blocks = mixer.due(BLOCK_NANOS + 1_000_000, &mem);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].iter().all(|s| *s == 70));
        // Stopped where the clock is now: from there on, only the other.
        let now = BLOCK_NANOS + 1_000_000;
        mixer.stop(1, now);
        let blocks = mixer.due(2 * BLOCK_NANOS + 1_000_000, &mem);
        let after = (frame_at(now) as usize - BLOCK_FRAMES) * 2;
        assert!(blocks[0][..after].iter().all(|s| *s == 70));
        assert!(blocks[0][after..].iter().all(|s| *s == -30));
    }

    /// Guest memory with sixteen-bit samples written at the bottom of it.
    fn guest(samples: &[i16]) -> Arena {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 20).expect("an arena");
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        mem.write(0, &bytes).expect("the samples fit");
        mem
    }

    fn stereo_16bit(bytes: u32, rate: u32) -> Pcm {
        Pcm::new(0, bytes, &pcm(2), rate, (1.0, 1.0)).expect("PCM")
    }

    const BLOCK_NANOS: u64 = BLOCK_FRAMES as u64 * 1_000_000_000 / RATE as u64;

    #[test]
    fn a_loop_goes_round_its_whole_sound_from_wherever_it_started() {
        let mem = guest(&[1, 1, 2, 2, 3, 3, 4, 4]);
        let mut mixer = Mixer::default();
        // Four frames, started from the third, round and round.
        mixer.play_buffer(1, stereo_16bit(16, RATE), 2, 0, true);
        let block = mixer.due(BLOCK_NANOS + 1_000_000, &mem).remove(0);
        assert_eq!(block[..10], [3, 3, 4, 4, 1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn what_a_title_writes_into_a_buffer_while_it_plays_is_heard_when_the_mix_gets_there() {
        let frames = BLOCK_FRAMES * 4;
        let mut mem = guest(&vec![100; frames * 2]);
        let mut mixer = Mixer::default();
        mixer.play_buffer(1, stereo_16bit(frames as u32 * 4, RATE), 0, 0, true);
        let first = mixer.due(BLOCK_NANOS + 1_000_000, &mem);
        assert!(first[0].iter().all(|s| *s == 100));
        // The title streams the next part of its sound into the part not yet
        // played, as a streamed sound is.
        let next: Vec<u8> = vec![200i16; BLOCK_FRAMES * 2]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        mem.write(BLOCK_FRAMES as u32 * 8, &next).expect("fits");
        let later = mixer.due(3 * BLOCK_NANOS + 1_000_000, &mem);
        assert!(
            later[1].iter().all(|s| *s == 200),
            "the new bytes are heard"
        );
    }

    #[test]
    fn what_a_title_writes_where_the_mix_has_played_is_not_heard_in_place_of_what_played() {
        let mut mem = guest(&vec![100; BLOCK_FRAMES * 8]);
        let mut mixer = Mixer::default();
        mixer.play_buffer(1, stereo_16bit(BLOCK_FRAMES as u32 * 16, RATE), 0, 0, false);
        // Half a block in, the title is about to write over the whole first
        // block, the half it has played included.
        mixer.catch_up(BLOCK_NANOS / 2, &mem);
        let over: Vec<u8> = vec![200i16; BLOCK_FRAMES * 2]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        mem.write(0, &over).expect("fits");
        let block = mixer.due(BLOCK_NANOS + 1_000_000, &mem).remove(0);
        let played = frame_at(BLOCK_NANOS / 2) as usize * 2;
        assert!(
            block[..played].iter().all(|s| *s == 100),
            "what played was heard"
        );
        assert!(
            block[played..].iter().all(|s| *s == 200),
            "and what was written after"
        );
    }

    #[test]
    fn a_buffer_that_is_not_pcm_is_refused_rather_than_played_as_noise() {
        let adpcm = Format {
            tag: 2,
            bits: 4,
            ..pcm(1)
        };
        assert!(Pcm::new(0, 4, &adpcm, RATE, (1.0, 1.0)).is_err());
        let misaligned = Format {
            block_align: 3,
            ..pcm(1)
        };
        assert!(Pcm::new(0, 4, &misaligned, RATE, (1.0, 1.0)).is_err());
    }

    #[test]
    fn a_sound_at_half_the_rate_takes_twice_as_long_and_is_drawn_between() {
        let mem = guest(&[100, -100, 200, -200]);
        let mut mixer = Mixer::default();
        mixer.play_buffer(
            1,
            Pcm::new(0, 8, &pcm(2), RATE / 2, (1.0, 0.5)).expect("PCM"),
            0,
            0,
            false,
        );
        let block = mixer.due(BLOCK_NANOS + 1_000_000, &mem).remove(0);
        assert_eq!(
            block[..10],
            [100, -50, 150, -75, 200, -100, 200, -100, 0, 0]
        );
    }

    #[test]
    fn a_buffer_changed_while_it_plays_goes_on_from_the_same_sample() {
        let ramp: Vec<i16> = (0..BLOCK_FRAMES as i16).flat_map(|s| [s, s]).collect();
        let mem = guest(&ramp);
        let at_third = |weights| {
            let bytes = BLOCK_FRAMES as u32 * 4;
            Pcm::new(0, bytes, &pcm(2), RATE / 3, weights).expect("PCM")
        };
        let (mut changed, mut unchanged) = (Mixer::default(), Mixer::default());
        for mixer in [&mut changed, &mut unchanged] {
            mixer.play_buffer(1, at_third((1.0, 1.0)), 0, 0, false);
        }
        // Panned hard left a third of the way into a block, between two of
        // its own frames.
        let now = BLOCK_NANOS / 3;
        changed.go_on(1, at_third((1.0, 0.0)), now, false);
        let end = 2 * BLOCK_NANOS + 1_000_000;
        let heard = changed.due(end, &mem).concat();
        let would = unchanged.due(end, &mem).concat();
        let after = frame_at(now) as usize;
        for (i, (a, b)) in heard.chunks(2).zip(would.chunks(2)).enumerate() {
            assert_eq!(a[0], b[0], "the left side is on the same sample at {i}");
            assert_eq!(a[1], if i < after { b[1] } else { 0 }, "the right at {i}");
        }
    }

    #[test]
    fn a_midi_message_is_heard_from_the_frame_its_declared_time_falls_in() {
        let mem = guest(&[]);
        let mut mixer = Mixer::default();
        // Note on, key 60, channel 1.
        let note = 0x7f_3c_90;
        let bank = crate::dls::Bank::parse(&crate::dls::tiny::bytes()).expect("a bank");
        mixer.open_synth(crate::synth::Synth::new(bank));
        // Program 1, its ramp, at full volume and hard left, a tenth of a
        // block in.
        for message in [0x01_c0, 0x7f_07_b0, 0x00_0a_b0] {
            assert!(mixer.midi(0, message));
        }
        let at = BLOCK_NANOS / 10;
        assert!(mixer.midi(at, note));
        let block = mixer.due(BLOCK_NANOS + 1_000_000, &mem).remove(0);
        let from = frame_at(at) as usize * 2;
        assert!(block[..from].iter().all(|s| *s == 0), "nothing before it");
        assert_eq!(block[from..from + 8], [0, 0, 100, 0, 200, 0, 300, 0]);
    }

    #[test]
    fn a_midi_message_with_no_synth_open_says_nothing_heard_it() {
        assert!(!Mixer::default().midi(0, 0x7f_3c_90));
    }

    #[test]
    fn a_sound_stopped_is_heard_up_to_the_frame_it_stopped_at() {
        let mem = guest(&[]);
        let mut mixer = Mixer::default();
        mixer.play(1, vec![5; BLOCK_FRAMES * 4], 0);
        // Stopped a tenth of the way through the first block.
        mixer.stop(1, BLOCK_NANOS / 10);
        let block = mixer.due(BLOCK_NANOS + 1_000_000, &mem).remove(0);
        let heard = block.iter().filter(|s| **s == 5).count() / 2;
        assert!((95..=102).contains(&heard), "{heard} frames heard");
        assert!(mixer
            .due(3 * BLOCK_NANOS, &mem)
            .iter()
            .flatten()
            .all(|s| *s == 0));
    }

    fn ref_frame_at(nanos: u64) -> u64 {
        u64::try_from(u128::from(nanos) * u128::from(RATE) / 1_000_000_000).unwrap_or(u64::MAX)
    }

    #[test]
    fn frame_at_matches_u128_reference_across_sweeps_and_overflow() {
        let mut samples: Vec<u64> = (0..=256).collect();
        let mut ns = 1u64;
        while ns <= 100_000_000_000_000 {
            samples.push(ns.saturating_sub(1));
            samples.push(ns);
            samples.push(ns.saturating_add(1));
            ns = ns.saturating_mul(7).saturating_add(19);
        }
        for frame in [1u64, 2, 10, 441, 1024, 22_050, 1_000_000, 13_348_000] {
            let boundary = (frame * 20_000_000).div_ceil(441);
            samples.push(boundary - 1);
            samples.push(boundary);
            samples.push(boundary + 1);
        }
        let mul_edge = u64::MAX / super::FRAME_NUM;
        for delta in -5i64..=5 {
            samples.push(mul_edge.saturating_add_signed(delta));
        }
        for edge in [u64::MAX - 5, u64::MAX - 1, u64::MAX] {
            samples.push(edge);
        }
        for nanos in samples {
            assert_eq!(frame_at(nanos), ref_frame_at(nanos), "nanos={nanos}");
        }
    }

    #[test]
    fn catch_up_silence_bulk_fill_matches_frame_by_frame() {
        let mem = guest(&[]);
        let mut mixer = Mixer::default();
        // Partial block, then finish that block plus 2 full blocks plus a tail.
        let t1 = BLOCK_NANOS / 3;
        assert!(mixer.due(t1, &mem).is_empty());
        assert_eq!(mixer.next_frame(), frame_at(t1));

        let t2 = 3 * BLOCK_NANOS + BLOCK_NANOS / 2;
        let blocks = mixer.due(t2, &mem);
        let expected_blocks = (frame_at(t2) / BLOCK_FRAMES as u64) as usize;
        assert_eq!(blocks.len(), expected_blocks);
        for b in &blocks {
            assert_eq!(b.len(), BLOCK_FRAMES * 2);
            assert!(b.iter().all(|&s| s == 0));
        }
        assert_eq!(mixer.next_frame(), frame_at(t2));

        // Transition from an active voice ending mid-catch_up into silence.
        let mut mixed = Mixer::default();
        mixed.play(1, vec![42; BLOCK_FRAMES], 0);
        let all = mixed.due(4 * BLOCK_NANOS, &mem);
        assert_eq!(
            all.len(),
            (frame_at(4 * BLOCK_NANOS) / BLOCK_FRAMES as u64) as usize
        );
        assert!(all[0][..BLOCK_FRAMES].iter().all(|&s| s == 42));
        assert!(all[0][BLOCK_FRAMES..].iter().all(|&s| s == 0));
        for b in &all[1..] {
            assert!(b.iter().all(|&s| s == 0));
        }
    }

    #[test]
    fn catch_up_guest_pcm_matches_voice_at_reference_across_formats_and_fault_fallback() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 20).expect("an arena");
        // 16-bit stereo ramp at 0x1000.
        let stereo16: Vec<u8> = (0..256i16)
            .flat_map(|i| [i * 100, -i * 80])
            .flat_map(i16::to_le_bytes)
            .collect();
        mem.write(0x1000, &stereo16).expect("stereo16");
        // 8-bit mono ramp at 0x4000.
        let mono8: Vec<u8> = (0..128u8).collect();
        mem.write(0x4000, &mono8).expect("mono8");
        // Out-of-bounds tail at arena end for fallback test.
        let end = mem.len() as u32;
        let tail_base = end - 16;
        let tail_samples: Vec<u8> = [100i16, -100, 200, -200, 300, -300, 400, -400]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect();
        mem.write(tail_base, &tail_samples).expect("tail");

        let fmt8_mono = Format {
            tag: 1,
            channels: 1,
            rate: RATE,
            block_align: 1,
            bits: 8,
            coefficients: Vec::new(),
        };
        let mut mixer = Mixer::default();
        // Voice 1: 16-bit stereo at RATE (part == 0, unit weight), looping.
        mixer.play_buffer(
            1,
            Pcm::new(0x1000, stereo16.len() as u32, &pcm(2), RATE, (1.0, 1.0)).expect("pcm"),
            3,
            0,
            true,
        );
        // Voice 2: 16-bit stereo at RATE/3 (fractional part + non-unit weights), non-looping.
        mixer.play_buffer(
            2,
            Pcm::new(
                0x1000,
                stereo16.len() as u32,
                &pcm(2),
                RATE / 3,
                (0.75, 0.25),
            )
            .expect("pcm"),
            0,
            BLOCK_NANOS / 10,
            false,
        );
        // Voice 3: 8-bit mono at RATE (part == 0, non-unit weights).
        mixer.play_buffer(
            3,
            Pcm::new(0x4000, mono8.len() as u32, &fmt8_mono, RATE, (0.5, 1.0)).expect("pcm"),
            0,
            0,
            true,
        );
        // Voice 4: buffer whose declared byte length extends past arena end (exercises fallback).
        mixer.play_buffer(
            4,
            Pcm::new(tail_base, 64, &pcm(2), RATE, (1.0, 1.0)).expect("pcm"),
            0,
            0,
            false,
        );

        let end_nanos = 2 * BLOCK_NANOS + 1_000_000;
        let target = frame_at(end_nanos);
        let mut expected = Vec::with_capacity(target as usize * 2);
        for frame in 0..target {
            let mut pair = [0i32; 2];
            for s in mixer.voices.iter().filter_map(|v| v.at(frame, &mem)) {
                pair[0] += i32::from(s[0]);
                pair[1] += i32::from(s[1]);
            }
            expected.push(pair[0].clamp(-32768, 32767) as i16);
            expected.push(pair[1].clamp(-32768, 32767) as i16);
        }

        let actual: Vec<i16> = mixer.due(end_nanos, &mem).into_iter().flatten().collect();
        let whole_samples = (target as usize / BLOCK_FRAMES) * BLOCK_FRAMES * 2;
        assert_eq!(actual, expected[..whole_samples]);
        assert_eq!(mixer.block, expected[whole_samples..]);
    }
}
