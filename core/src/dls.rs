//! Downloadable Sounds Level 1 banks, read as the MIDI Manufacturers
//! Association's specification (DLS Level 1, v1.1b) lays out the RIFF `DLS `
//! form: a pool of waves, and instruments whose regions each play one of them.
//!
//! Byte in, structure out; [`crate::synth`] plays it. Each region comes out
//! whole, with the wave it plays, the sample values that play it and the
//! articulation that shapes it, wherever in the bank each was written.

use crate::pe::{u16_at, u32_at};
use crate::sound::{Error, Format};
use crate::win32::winmm::{find_chunk, Chunk, Find};

/// A bank: its instruments, and the waves their regions play, in the order
/// its pool table lists them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bank {
    pub instruments: Vec<Instrument>,
    pub waves: Vec<Wave>,
}

/// One instrument, and where MIDI finds it: the bank controllers 0 and 32
/// select (`msb << 7 | lsb`), and a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instrument {
    pub bank: u16,
    pub program: u8,
    /// A drum kit: played on MIDI's tenth channel, each region its own drum.
    pub drums: bool,
    pub regions: Vec<Region>,
}

/// The keys of an instrument one wave plays, and how it plays them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// The lowest key and the highest.
    pub keys: (u16, u16),
    /// Whether a note on a key leaves the key's last note sounding, where by
    /// default it ends it (`F_RGN_OPTION_SELFNONEXCLUSIVE`).
    pub overlaps: bool,
    /// The drums of one group cut each other off; 0 is no group.
    pub group: u16,
    /// Which of the bank's waves it plays.
    pub wave: usize,
    pub sample: Sample,
    pub articulation: Articulation,
}

/// How a wave is played (`wsmp`): the key that sounds it at its own pitch,
/// its tuning in cents, its gain in 1/65,536 centibels (negative is quieter,
/// as the specification's errata reads it), and its loop as its first sample
/// and its length. A region's own stands in for its wave's, and for neither
/// a one-shot at middle C, as the specification says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub unity: u16,
    pub fine_tune: i16,
    pub gain: i32,
    pub looped: Option<(u32, u32)>,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            unity: 60,
            fine_tune: 0,
            gain: 0,
            looped: None,
        }
    }
}

/// A wave of the pool: its rate, and its samples, eight-bit ones widened to
/// sixteen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wave {
    pub rate: u32,
    pub samples: Vec<i16>,
}

/// The time cents DLS writes for no time at all, which nothing added to it
/// moves.
pub const NO_TIME: i32 = i32::MIN;

/// What DLS Level 1's connection blocks set (its Table 1), each in
/// 1/65,536ths of the specification's unit; what a bank does not set is its
/// Table 2's default. The connections the architecture infers, from velocity,
/// volume, expression, pan, pitch bend and the key, are the synth's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Articulation {
    /// The LFO's frequency, in absolute pitch cents, and its delay, in time
    /// cents.
    pub lfo_frequency: i32,
    pub lfo_delay: i32,
    /// What the LFO adds at its peak to gain, in centibels, and to pitch, in
    /// cents, and what the mod wheel at its top adds to each.
    pub lfo_gain: i32,
    pub lfo_pitch: i32,
    pub wheel_gain: i32,
    pub wheel_pitch: i32,
    /// The volume envelope (EG1) and the pitch envelope (EG2), and how far
    /// the pitch envelope moves the pitch at its peak, in cents.
    pub volume: Envelope,
    pub pitch: Envelope,
    pub pitch_depth: i32,
    /// Where it sits from left to right, in 0.1% either side of the middle.
    pub pan: i32,
}

/// An envelope: its segments' times in time cents, its sustain level in 0.1%
/// of its peak, and the time cents its attack gains from the velocity and
/// its decay from the key, each at its top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub attack: i32,
    pub decay: i32,
    pub sustain: i32,
    pub release: i32,
    pub velocity_attack: i32,
    pub key_decay: i32,
}

impl Default for Articulation {
    fn default() -> Self {
        let envelope = Envelope {
            attack: NO_TIME,
            decay: NO_TIME,
            sustain: 1000 << 16,
            release: NO_TIME,
            velocity_attack: 0,
            key_decay: 0,
        };
        Self {
            // Five hertz, as the specification's own example writes it.
            lfo_frequency: 0xfcac_ae9c_u32 as i32,
            // Ten milliseconds.
            lfo_delay: -522_494_111,
            lfo_gain: 0,
            lfo_pitch: 0,
            wheel_gain: 0,
            wheel_pitch: 0,
            volume: envelope,
            pitch: envelope,
            pitch_depth: 0,
            pan: 0,
        }
    }
}

impl Articulation {
    /// Where a connection block's value goes, when it is one Level 1 sets:
    /// its source, control and destination, none of them transformed.
    fn slot(&mut self, block: [u16; 4]) -> Option<&mut i32> {
        // Sources and controls: none, the LFO, velocity, the key, the pitch
        // envelope and the mod wheel (CC1). Destinations: gain (by the
        // errata; "attenuation" in the header), pitch and pan.
        const NONE: u16 = 0;
        const LFO: u16 = 1;
        const VELOCITY: u16 = 2;
        const KEY: u16 = 3;
        const EG2: u16 = 5;
        const WHEEL: u16 = 0x81;
        const GAIN: u16 = 1;
        const PITCH: u16 = 3;
        const PAN: u16 = 4;
        Some(match block {
            [NONE, NONE, 0x0104, 0] => &mut self.lfo_frequency,
            [NONE, NONE, 0x0105, 0] => &mut self.lfo_delay,
            [LFO, NONE, GAIN, 0] => &mut self.lfo_gain,
            [LFO, NONE, PITCH, 0] => &mut self.lfo_pitch,
            [LFO, WHEEL, GAIN, 0] => &mut self.wheel_gain,
            [LFO, WHEEL, PITCH, 0] => &mut self.wheel_pitch,
            [NONE, NONE, 0x0206, 0] => &mut self.volume.attack,
            [NONE, NONE, 0x0207, 0] => &mut self.volume.decay,
            [NONE, NONE, 0x020a, 0] => &mut self.volume.sustain,
            [NONE, NONE, 0x0209, 0] => &mut self.volume.release,
            [VELOCITY, NONE, 0x0206, 0] => &mut self.volume.velocity_attack,
            [KEY, NONE, 0x0207, 0] => &mut self.volume.key_decay,
            [NONE, NONE, 0x030a, 0] => &mut self.pitch.attack,
            [NONE, NONE, 0x030b, 0] => &mut self.pitch.decay,
            [NONE, NONE, 0x030e, 0] => &mut self.pitch.sustain,
            [NONE, NONE, 0x030d, 0] => &mut self.pitch.release,
            [VELOCITY, NONE, 0x030a, 0] => &mut self.pitch.velocity_attack,
            [KEY, NONE, 0x030b, 0] => &mut self.pitch.key_decay,
            [EG2, NONE, PITCH, 0] => &mut self.pitch_depth,
            [NONE, NONE, PAN, 0] => &mut self.pan,
            _ => return None,
        })
    }
}

impl Bank {
    /// Read a bank.
    ///
    /// # Errors
    /// When the bytes are not a DLS bank, a chunk runs past what holds it, or
    /// a wave or loop is one Level 1 does not have.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let file = find_chunk(bytes, 0, len, Find::Next)
            .filter(|c| names(c) == (*b"RIFF", *b"DLS "))
            .ok_or_else(|| Error("not a DLS bank".to_owned()))?;
        let (mut lins, mut ptbl, mut wvpl) = (None, None, None);
        for chunk in children(bytes, &file)? {
            let (id, form) = names(&chunk);
            match (&id, &form) {
                (b"LIST", b"lins") => lins = Some(chunk),
                (b"ptbl", _) => ptbl = Some(chunk),
                (b"LIST", b"wvpl") => wvpl = Some(chunk),
                _ => {}
            }
        }
        let missing = |what: &str| Error(format!("a DLS bank with no {what}"));
        let lins = lins.ok_or_else(|| missing("instruments"))?;
        let ptbl = ptbl.ok_or_else(|| missing("pool table"))?;
        let wvpl = wvpl.ok_or_else(|| missing("wave pool"))?;

        // The pool table: each wave by where it starts, counted from the
        // pool's first chunk.
        let table = body(bytes, &ptbl)?;
        let (size, count) = (word(table, 0)? as usize, word(table, 4)? as usize);
        let pool_end = end(&wvpl)?;
        let mut waves = Vec::new();
        let mut their_samples = Vec::new();
        for i in 0..count {
            let at = wvpl
                .data
                .saturating_add(4)
                .saturating_add(word(table, size + 4 * i)?);
            let list = find_chunk(bytes, at, pool_end, Find::Next)
                .filter(|c| names(c) == (*b"LIST", *b"wave") && end(c).is_ok_and(|e| e <= pool_end))
                .ok_or_else(|| Error(format!("wave {i} of the pool table is not a wave")))?;
            let (wave, sample) = wave(bytes, &list)?;
            waves.push(wave);
            their_samples.push(sample);
        }

        let mut instruments = Vec::new();
        for list in children(bytes, &lins)? {
            if names(&list) == (*b"LIST", *b"ins ") {
                instruments.push(instrument(bytes, &list, &waves, &their_samples)?);
            }
        }
        Ok(Self { instruments, waves })
    }
}

/// A wave of the pool, and the sample values it carries for itself.
fn wave(bytes: &[u8], list: &Chunk) -> Result<(Wave, Option<Sample>), Error> {
    let (mut format, mut data, mut sample) = (None, None, None);
    for chunk in children(bytes, list)? {
        match &names(&chunk).0 {
            b"fmt " => format = Some(Format::parse(body(bytes, &chunk)?)?),
            b"data" => data = Some(body(bytes, &chunk)?),
            b"wsmp" => sample = Some(wsmp(body(bytes, &chunk)?)?),
            _ => {}
        }
    }
    let (Some(format), Some(data)) = (format, data) else {
        return Err(Error("a wave with no format or no data".to_owned()));
    };
    let samples = match (format.tag, format.channels, format.bits) {
        (1, 1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect(),
        (1, 1, 8) => data.iter().map(|b| (i16::from(*b) - 128) << 8).collect(),
        (tag, channels, bits) => {
            return Err(Error(format!(
                "a wave of format tag {tag} in {channels} channels at {bits} bits, where DLS \
                 Level 1 has mono PCM at eight or sixteen"
            )))
        }
    };
    Ok((
        Wave {
            rate: format.rate,
            samples,
        },
        sample,
    ))
}

/// A wave sample chunk: Level 1 has one loop at most, played forward.
fn wsmp(b: &[u8]) -> Result<Sample, Error> {
    let size = word(b, 0)? as usize;
    let looped = match word(b, 16)? {
        0 => None,
        _ if word(b, size + 4)? != 0 => {
            return Err(Error(
                "a loop of a type DLS Level 1 does not have".to_owned(),
            ))
        }
        _ => Some((word(b, size + 8)?, word(b, size + 12)?)),
    };
    Ok(Sample {
        unity: half(b, 4)?,
        fine_tune: half(b, 6)? as i16,
        gain: word(b, 8)? as i32,
        looped,
    })
}

fn instrument(
    bytes: &[u8],
    list: &Chunk,
    waves: &[Wave],
    their_samples: &[Option<Sample>],
) -> Result<Instrument, Error> {
    let (mut header, mut regions, mut articulation) = (None, Vec::new(), None);
    for chunk in children(bytes, list)? {
        let (id, form) = names(&chunk);
        match (&id, &form) {
            (b"insh", _) => header = Some(body(bytes, &chunk)?),
            (b"LIST", b"lrgn") => regions = children(bytes, &chunk)?,
            (b"LIST", b"lart") => articulation = Some(lart(bytes, &chunk)?),
            _ => {}
        }
    }
    let header = header.ok_or_else(|| Error("an instrument with no header".to_owned()))?;
    let (bank, program) = (word(header, 4)?, word(header, 8)?);
    let regions = regions
        .iter()
        .filter(|c| names(c) == (*b"LIST", *b"rgn "))
        .map(|r| region(bytes, r, articulation, waves, their_samples))
        .collect::<Result<_, _>>()?;
    Ok(Instrument {
        // Controller 0 is in bits 8 to 14, controller 32 in 0 to 6, and bit
        // 31 says it is a drum kit.
        bank: ((((bank >> 8) & 0x7f) << 7) | (bank & 0x7f)) as u16,
        program: (program & 0x7f) as u8,
        drums: bank & 0x8000_0000 != 0,
        regions,
    })
}

/// A region, with its own sample values and articulation where it has them,
/// and otherwise its wave's and its instrument's.
fn region(
    bytes: &[u8],
    list: &Chunk,
    theirs: Option<Articulation>,
    waves: &[Wave],
    their_samples: &[Option<Sample>],
) -> Result<Region, Error> {
    let (mut header, mut link, mut sample, mut articulation) = (None, None, None, None);
    for chunk in children(bytes, list)? {
        let (id, form) = names(&chunk);
        match (&id, &form) {
            (b"rgnh", _) => header = Some(body(bytes, &chunk)?),
            (b"wlnk", _) => link = Some(body(bytes, &chunk)?),
            (b"wsmp", _) => sample = Some(wsmp(body(bytes, &chunk)?)?),
            (b"LIST", b"lart") => articulation = Some(lart(bytes, &chunk)?),
            _ => {}
        }
    }
    let (Some(header), Some(link)) = (header, link) else {
        return Err(Error("a region with no header or no wave link".to_owned()));
    };
    let wave = word(link, 8)? as usize;
    let Some(pcm) = waves.get(wave) else {
        return Err(Error(format!(
            "a region playing wave {wave} of a pool of {}",
            waves.len()
        )));
    };
    let sample = sample.or(their_samples[wave]).unwrap_or_default();
    if let Some((start, length)) = sample.looped {
        if length == 0 || u64::from(start) + u64::from(length) > pcm.samples.len() as u64 {
            return Err(Error(format!(
                "a loop of {length} from {start} in a wave of {}",
                pcm.samples.len()
            )));
        }
    }
    Ok(Region {
        keys: (half(header, 0)?, half(header, 2)?),
        overlaps: half(header, 8)? & 1 != 0,
        group: half(header, 10)?,
        wave,
        sample,
        articulation: articulation.or(theirs).unwrap_or_default(),
    })
}

/// A list of articulators: Level 1's own connection blocks, over the
/// defaults. Another level's or another maker's articulators are passed by.
fn lart(bytes: &[u8], list: &Chunk) -> Result<Articulation, Error> {
    let mut articulation = Articulation::default();
    for chunk in children(bytes, list)? {
        if &names(&chunk).0 != b"art1" {
            continue;
        }
        let b = body(bytes, &chunk)?;
        let (size, count) = (word(b, 0)? as usize, word(b, 4)? as usize);
        for at in (0..count).map(|i| size + 12 * i) {
            let block = [
                half(b, at)?,
                half(b, at + 2)?,
                half(b, at + 4)?,
                half(b, at + 6)?,
            ];
            if let Some(slot) = articulation.slot(block) {
                *slot = word(b, at + 8)? as i32;
            }
        }
    }
    Ok(articulation)
}

/// A chunk's id and its form type, as their four letters.
fn names(chunk: &Chunk) -> ([u8; 4], [u8; 4]) {
    (chunk.id.to_le_bytes(), chunk.form.to_le_bytes())
}

/// Where a chunk's data ends.
fn end(chunk: &Chunk) -> Result<u32, Error> {
    chunk.data.checked_add(chunk.size).ok_or_else(damaged)
}

fn damaged() -> Error {
    Error("a DLS bank that records more than it holds".to_owned())
}

/// A chunk's data.
fn body<'a>(bytes: &'a [u8], chunk: &Chunk) -> Result<&'a [u8], Error> {
    bytes
        .get(chunk.data as usize..end(chunk)? as usize)
        .ok_or_else(damaged)
}

/// The chunks a RIFF or LIST chunk holds, past its form type, each ending
/// inside it.
fn children(bytes: &[u8], list: &Chunk) -> Result<Vec<Chunk>, Error> {
    let end = end(list)
        .ok()
        .filter(|e| *e as usize <= bytes.len())
        .ok_or_else(damaged)?;
    let mut at = list.data.saturating_add(4);
    let mut out = Vec::new();
    while let Some(chunk) = find_chunk(bytes, at, end, Find::Next) {
        let last = self::end(&chunk)
            .ok()
            .filter(|e| *e <= end)
            .ok_or_else(damaged)?;
        out.push(chunk);
        at = last.saturating_add(chunk.size % 2);
    }
    Ok(out)
}

fn word(b: &[u8], at: usize) -> Result<u32, Error> {
    u32_at(b, at).ok_or_else(|| Error("a short DLS chunk".to_owned()))
}

fn half(b: &[u8], at: usize) -> Result<u16, Error> {
    u16_at(b, at).ok_or_else(|| Error("a short DLS chunk".to_owned()))
}

/// A bank written here, small enough to hear every part of: what the synth's
/// tests play.
#[cfg(test)]
pub(crate) mod tiny {
    pub fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    pub fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        chunk(b"LIST", &[kind.to_vec(), parts.concat()].concat())
    }

    fn words(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A `wsmp`: the unity key, the fine tune and the gain, and the loop.
    pub fn wsmp(unity: u16, fine: i16, gain: i32, looped: Option<(u32, u32)>) -> Vec<u8> {
        let mut b = words(&[20]);
        b.extend_from_slice(&unity.to_le_bytes());
        b.extend_from_slice(&fine.to_le_bytes());
        b.extend_from_slice(&gain.to_le_bytes());
        b.extend_from_slice(&words(&[0, u32::from(looped.is_some())]));
        if let Some((start, length)) = looped {
            b.extend_from_slice(&words(&[16, 0, start, length]));
        }
        chunk(b"wsmp", &b)
    }

    /// Connection blocks: source, control, destination, and the value.
    pub fn art1(blocks: &[(u16, u16, u16, i32)]) -> Vec<u8> {
        let mut b = words(&[8, blocks.len() as u32]);
        for (source, control, destination, scale) in blocks {
            for half in [*source, *control, *destination, 0] {
                b.extend_from_slice(&half.to_le_bytes());
            }
            b.extend_from_slice(&scale.to_le_bytes());
        }
        list(b"lart", &[chunk(b"art1", &b)])
    }

    /// A region over keys, playing a wave of the pool table, with what else
    /// it carries.
    pub fn region(
        keys: (u16, u16),
        options: u16,
        group: u16,
        wave: u32,
        more: &[Vec<u8>],
    ) -> Vec<u8> {
        let mut rgnh = Vec::new();
        for half in [keys.0, keys.1, 0, 127, options, group] {
            rgnh.extend_from_slice(&half.to_le_bytes());
        }
        let wlnk = [vec![0, 0, 0, 0], words(&[1, wave])].concat();
        let parts = [
            vec![chunk(b"rgnh", &rgnh)],
            more.to_vec(),
            vec![chunk(b"wlnk", &wlnk)],
        ];
        list(b"rgn ", &parts.concat())
    }

    pub fn instrument(bank: u32, program: u32, regions: &[Vec<u8>], more: &[Vec<u8>]) -> Vec<u8> {
        let insh = words(&[regions.len() as u32, bank, program]);
        let parts = [
            vec![chunk(b"insh", &insh), list(b"lrgn", regions)],
            more.to_vec(),
        ];
        list(b"ins ", &parts.concat())
    }

    /// A wave: mono PCM at a rate and a width, and what else it carries.
    pub fn wave(rate: u32, bits: u16, data: &[u8], more: &[Vec<u8>]) -> Vec<u8> {
        let mut fmt = vec![1, 0, 1, 0];
        fmt.extend_from_slice(&words(&[rate, rate * u32::from(bits / 8)]));
        fmt.extend_from_slice(&(bits / 8).to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let parts = [
            vec![chunk(b"fmt ", &fmt)],
            more.to_vec(),
            vec![chunk(b"data", data)],
        ];
        list(b"wave", &parts.concat())
    }

    /// A bank of instruments over waves, its pool table listing the waves in
    /// the order `order` gives.
    pub fn bank(instruments: &[Vec<u8>], waves: &[Vec<u8>], order: &[usize]) -> Vec<u8> {
        let mut offsets = Vec::new();
        let mut at = 0;
        for w in waves {
            offsets.push(at);
            at += w.len() as u32;
        }
        let cues: Vec<u32> = order.iter().map(|i| offsets[*i]).collect();
        let ptbl = [words(&[8, cues.len() as u32]), words(&cues)].concat();
        let body = [
            b"DLS ".to_vec(),
            chunk(b"colh", &words(&[instruments.len() as u32])),
            list(b"lins", instruments),
            chunk(b"ptbl", &ptbl),
            list(b"wvpl", waves),
        ]
        .concat();
        chunk(b"RIFF", &body)
    }

    pub fn samples(values: &[i16]) -> Vec<u8> {
        values.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    /// Time cents for a time in seconds that is a power of two.
    pub fn seconds_log2(power: i32) -> i32 {
        power * 1200 * 65536
    }

    /// Two melodic instruments and a drum kit over three waves:
    ///
    /// - wave 0, a steady level of 16,384 at 22,050 Hz, looped over its
    ///   second half;
    /// - wave 1, a ramp of eight steps of 100 at 22,050 Hz, its own sample
    ///   values looping its last four;
    /// - wave 2, four eight-bit steps of 4,096 at 11,025 Hz, played once.
    ///
    /// Program 0 plays wave 0 over every key with an envelope of a 1/64 s
    /// attack, a 1 s decay to half, and a 1/2 s release. Program 1 plays
    /// wave 1 from key 60 up and wave 2 below, with no envelope at all. The
    /// kit plays wave 2 on key 82, and wave 0 on keys 42 and 46, a group of
    /// their own, each released over a second.
    pub fn bytes() -> Vec<u8> {
        let steady = wave(22_050, 16, &samples(&[16_384; 64]), &[]);
        let ramp: Vec<i16> = (0..8).map(|i| i * 100).collect();
        let ramp = wave(22_050, 16, &samples(&ramp), &[wsmp(60, 0, 0, Some((4, 4)))]);
        let eight = wave(11_025, 8, &[128, 144, 160, 176], &[]);
        // Instant, held at its peak, and over at once: no envelope at all.
        let flat = (0, 0, 0x0209, super::NO_TIME);
        let shaped = art1(&[
            (0, 0, 0x0206, seconds_log2(-6)),
            (0, 0, 0x0207, seconds_log2(0)),
            (0, 0, 0x020a, 500 << 16),
            (0, 0, 0x0209, seconds_log2(-1)),
        ]);
        let piano = instrument(
            0,
            0,
            &[region((0, 127), 0, 0, 1, &[wsmp(60, 0, 0, Some((32, 32)))])],
            &[shaped],
        );
        let ramps = instrument(
            0,
            1,
            &[
                region((60, 127), 0, 0, 2, &[]),
                region((0, 59), 0, 0, 0, &[wsmp(48, 0, 0, None)]),
            ],
            &[art1(&[flat])],
        );
        // A drum sounds at once, at its peak, and dies away over a second.
        let struck = art1(&[(0, 0, 0x0209, seconds_log2(0))]);
        let drum = |key, group, wave, looped| {
            region(
                (key, key),
                0,
                group,
                wave,
                &[wsmp(key, 0, 0, looped), struck.clone()],
            )
        };
        let kit = instrument(
            0x8000_0000,
            0,
            &[
                drum(82, 0, 0, None),
                drum(42, 1, 1, Some((32, 32))),
                drum(46, 1, 1, Some((32, 32))),
            ],
            &[],
        );
        // The pool table lists them out of the pool's own order: the
        // eight-bit wave first, then the steady one, then the ramp.
        bank(&[piano, ramps, kit], &[steady, ramp, eight], &[2, 0, 1])
    }
}

#[cfg(test)]
mod tests {
    use super::{tiny, Articulation, Bank, Sample, NO_TIME};

    #[test]
    fn a_bank_reads_into_instruments_whose_regions_carry_their_wave_sample_and_articulation() {
        let bank = Bank::parse(&tiny::bytes()).expect("a bank");
        // The pool table's order, not the pool's: the eight-bit wave first,
        // widened to sixteen bits.
        assert_eq!(bank.waves.len(), 3);
        assert_eq!(bank.waves[0].rate, 11_025);
        assert_eq!(bank.waves[0].samples, [0, 4096, 8192, 12_288]);
        assert_eq!(bank.waves[1].samples, [16_384; 64]);

        let [piano, ramps, kit] = &bank.instruments[..] else {
            panic!("three instruments")
        };
        assert_eq!((piano.bank, piano.program, piano.drums), (0, 0, false));
        assert_eq!((kit.program, kit.drums), (0, true));
        // A melodic region has its instrument's articulation; what that sets
        // is set, and the rest is the specification's default.
        let shaped = piano.regions[0].articulation;
        assert_eq!(shaped.volume.attack, tiny::seconds_log2(-6));
        assert_eq!(shaped.volume.sustain, 500 << 16);
        assert_eq!(shaped.pitch, Articulation::default().pitch);
        assert_eq!(shaped.lfo_frequency, Articulation::default().lfo_frequency);
        // A region's own sample values; else its wave's; else middle C once.
        assert_eq!(piano.regions[0].sample.looped, Some((32, 32)));
        assert_eq!(ramps.regions[0].wave, 2);
        assert_eq!(ramps.regions[0].sample.looped, Some((4, 4)));
        assert_eq!(ramps.regions[1].sample.unity, 48);
        assert_eq!(
            kit.regions[0].sample,
            Sample {
                unity: 82,
                ..Sample::default()
            }
        );
        // A drum's articulation is its own.
        assert_eq!(kit.regions[1].group, 1);
        assert_eq!(kit.regions[1].articulation.volume.release, 0);
        assert_eq!(kit.regions[1].articulation.volume.decay, NO_TIME);
        assert_eq!(ramps.regions[0].articulation.volume.release, NO_TIME);
    }

    #[test]
    fn what_is_not_a_level_1_bank_is_refused() {
        assert!(Bank::parse(b"RIFF\x04\x00\x00\x00WAVE").is_err());
        let bytes = tiny::bytes();
        assert!(
            Bank::parse(&bytes[..bytes.len() - 10]).is_err(),
            "cut short"
        );

        let wave = |data: &[u8], more: &[Vec<u8>]| tiny::wave(22_050, 16, data, more);
        let one = |looped| {
            let region = tiny::region((0, 127), 0, 0, 0, &[tiny::wsmp(60, 0, 0, looped)]);
            tiny::instrument(0, 0, &[region], &[])
        };
        let four = tiny::samples(&[0; 4]);
        assert!(Bank::parse(&tiny::bank(&[one(Some((2, 2)))], &[wave(&four, &[])], &[0])).is_ok());
        assert!(
            Bank::parse(&tiny::bank(&[one(Some((2, 3)))], &[wave(&four, &[])], &[0])).is_err(),
            "a loop past its wave's end"
        );
        let stereo = {
            let mut w = wave(&four, &[]);
            // The format's channel count, inside the wave's `fmt ` chunk.
            w[22] = 2;
            w
        };
        assert!(Bank::parse(&tiny::bank(&[one(None)], &[stereo], &[0])).is_err());
        let past = tiny::region((0, 127), 0, 0, 1, &[]);
        let instrument = tiny::instrument(0, 0, &[past], &[]);
        assert!(
            Bank::parse(&tiny::bank(&[instrument], &[wave(&four, &[])], &[0])).is_err(),
            "a region playing a wave the pool table does not list"
        );
    }
}
