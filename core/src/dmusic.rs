//! DirectMusic RIFF parser and deterministic declared-clock sequencer.
//!
//! Parses standard DirectMusic 6.1/7 binary assets (`RIFF:DMBD` bands,
//! `RIFF:DMST` styles, and `RIFF:DMSG` segments with chord, command, tempo,
//! style, band, and sequence tracks) and turns them into timestamped short MIDI
//! messages on the declared clock for [`crate::synth::Synth`] via
//! [`crate::sound::Mixer`].
//!
//! All timing and pitch calculations use pure integer arithmetic so identical
//! assets and control inputs produce identical MIDI streams and PCM mixes on
//! every host (Golden Rule 1, ADR-0001, ADR-0008).

use crate::pe::{u16_at, u32_at};
use crate::snap::{snap_vec, Snap, Visitor};

/// DirectMusic ticks per quarter note (`DMUS_PPQ`).
pub const DMUS_PPQ: u64 = 768;

/// Default DirectMusic tempo in millibpm (120.0 BPM).
pub const DEFAULT_MILLIBPM: u32 = 120_000;

/// DirectMusic segment notification options (`DMUS_NOTIFICATION_SEG*`).
pub const DMUS_NOTIFICATION_SEGSTART: u32 = 0;
pub const DMUS_NOTIFICATION_SEGEND: u32 = 1;
pub const DMUS_NOTIFICATION_SEGALMOSTEND: u32 = 2;
pub const DMUS_NOTIFICATION_SEGLOOP: u32 = 3;
pub const DMUS_NOTIFICATION_SEGABORT: u32 = 4;

/// `DMUS_PLAYMODE_*` flags from `dmusici.h`.
pub const DMUS_PLAYMODE_FIXED: u8 = 0x00;
pub const DMUS_PLAYMODE_KEY_ROOT: u8 = 0x01;
pub const DMUS_PLAYMODE_CHORD_ROOT: u8 = 0x02;
pub const DMUS_PLAYMODE_SCALE_INTERVALS: u8 = 0x04;
pub const DMUS_PLAYMODE_CHORD_INTERVALS: u8 = 0x08;
pub const DMUS_PLAYMODE_NONE: u8 = 0x10;

/// `DMUS_IO_INST_*` flags from `dmusicf.h`.
pub const DMUS_IO_INST_PATCH: u32 = 1 << 0;
pub const DMUS_IO_INST_BANKSELECT: u32 = 1 << 1;
pub const DMUS_IO_INST_PAN: u32 = 1 << 5;
pub const DMUS_IO_INST_VOLUME: u32 = 1 << 6;
pub const DMUS_IO_INST_TRANSPOSE: u32 = 1 << 7;

/// `DMUS_CURVET_*` curve event types and `DMUS_CURVE_RESET` flag from `dmusici.h`.
pub const DMUS_CURVET_PBCURVE: u8 = 0x03;
pub const DMUS_CURVET_CCCURVE: u8 = 0x04;
pub const DMUS_CURVE_RESET: u8 = 0x01;

/// Track class GUIDs (`CLSID_DirectMusic*Track`) in little-endian binary layout.
const CLSID_CHORD_TRACK: [u8; 16] = [
    0x8b, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];
const CLSID_COMMAND_TRACK: [u8; 16] = [
    0x8c, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];
const CLSID_TEMPO_TRACK: [u8; 16] = [
    0x85, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];
const CLSID_STYLE_TRACK: [u8; 16] = [
    0x8d, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];
const CLSID_BAND_TRACK: [u8; 16] = [
    0x94, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];
const CLSID_SEQ_TRACK: [u8; 16] = [
    0x83, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];

/// C major scale mask (bits 0, 2, 4, 5, 7, 9, 11 repeated across two octaves).
const DEFAULT_SCALE_MASK: u32 = 0x00ab_5ab5;
/// Major triad chord mask (root=0, major third=4, fifth=7).
const DEFAULT_CHORD_MASK: u32 = 0x0000_0091;
/// MIDI drum channel (channel 10, 0-based index 9).
const DRUM_CHANNEL: usize = 9;

/// One instrument assignment inside a DirectMusic band (`bins`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BandInstrument {
    pub patch: u32,
    pub pchannel: u32,
    pub flags: u32,
    pub pan: u8,
    pub volume: u8,
    pub transpose: i16,
}

impl BandInstrument {
    /// Emit MIDI bank-select, program-change, volume, and pan short messages
    /// for this instrument onto its MIDI channel (`0..16`) at `tick`.
    pub fn emit_midi(&self, tick: u32, out: &mut Vec<(u32, u32)>) {
        if self.pchannel >= 16 {
            return;
        }
        let ch = self.pchannel & 0x0f;
        if (self.flags & DMUS_IO_INST_PATCH) != 0 {
            if (self.flags & DMUS_IO_INST_BANKSELECT) != 0 {
                let msb = (self.patch >> 16) & 0x7f;
                let lsb = (self.patch >> 8) & 0x7f;
                out.push((tick, 0xb0 | ch | (msb << 16)));
                out.push((tick, 0xb0 | ch | (32 << 8) | (lsb << 16)));
            }
            let program = self.patch & 0x7f;
            out.push((tick, 0xc0 | ch | (program << 8)));
        }
        if (self.flags & DMUS_IO_INST_VOLUME) != 0 {
            let vol = u32::from(self.volume & 0x7f);
            out.push((tick, 0xb0 | ch | (7 << 8) | (vol << 16)));
        }
        if (self.flags & DMUS_IO_INST_PAN) != 0 {
            let pan = u32::from(self.pan & 0x7f);
            out.push((tick, 0xb0 | ch | (10 << 8) | (pan << 16)));
        }
    }
}

/// A DirectMusic band (`RIFF:DMBD`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Band {
    pub guid: [u8; 16],
    pub name: String,
    pub instruments: Vec<BandInstrument>,
}

/// DirectMusic time signature (`DMUS_IO_TIMESIG`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSig {
    pub beats_per_measure: u8,
    pub beat: u8,
    pub grids_per_beat: u16,
}

impl Default for TimeSig {
    fn default() -> Self {
        Self {
            beats_per_measure: 4,
            beat: 4,
            grids_per_beat: 4,
        }
    }
}

impl TimeSig {
    /// Music ticks in one beat (`DMUS_PPQ * 4 / beat`, 768 for quarter notes).
    #[must_use]
    pub fn beat_ticks(self) -> u64 {
        let denom = if self.beat == 0 {
            256
        } else {
            u64::from(self.beat)
        };
        (DMUS_PPQ * 4 / denom).max(1)
    }

    /// Music ticks in one measure.
    #[must_use]
    pub fn measure_ticks(self) -> u64 {
        u64::from(self.beats_per_measure.max(1)) * self.beat_ticks()
    }

    /// Music ticks per grid step.
    #[must_use]
    pub fn grid_ticks(self) -> u64 {
        (self.beat_ticks() / u64::from(self.grids_per_beat.max(1))).max(1)
    }
}

/// One note inside a style part (`DMUS_IO_STYLENOTE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleNote {
    pub grid_start: u32,
    pub variation: u32,
    pub duration: u32,
    pub time_offset: i16,
    pub music_value: u16,
    pub velocity: u8,
    pub play_mode: u8,
}

/// One controller/pitch-bend curve inside a style part (`DMUS_IO_STYLECURVE`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StyleCurve {
    pub grid_start: u32,
    pub variation: u32,
    pub duration: u32,
    pub reset_duration: u32,
    pub time_offset: i16,
    pub start_value: i16,
    pub end_value: i16,
    pub reset_value: i16,
    pub event_type: u8,
    pub cc_data: u8,
    pub flags: u8,
}

/// One reusable musical part inside a style (`LIST:part`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StylePart {
    pub guid: [u8; 16],
    pub time_sig: TimeSig,
    pub measures: u16,
    pub play_mode: u8,
    pub notes: Vec<StyleNote>,
    pub curves: Vec<StyleCurve>,
}

/// Reference from a pattern to a part on a logical PChannel (`LIST:pref`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartRef {
    pub part_guid: [u8; 16],
    pub pchannel: u32,
    pub subchord_level: u8,
}

/// One pattern inside a style (`LIST:pttn`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pattern {
    pub groove_bottom: u8,
    pub groove_top: u8,
    pub embellishment: u16,
    pub measures: u16,
    pub part_refs: Vec<PartRef>,
}

/// A DirectMusic style (`RIFF:DMST`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub guid: [u8; 16],
    pub name: String,
    pub time_sig: TimeSig,
    pub millibpm: u32,
    pub bands: Vec<Band>,
    pub parts: Vec<StylePart>,
    pub patterns: Vec<Pattern>,
}

/// One subchord definition inside a chord track item (`DMUS_IO_SUBCHORD`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubChord {
    pub chord_pattern: u32,
    pub scale_pattern: u32,
    pub chord_root: u8,
    pub scale_root: u8,
    pub levels: u32,
}

impl Default for SubChord {
    fn default() -> Self {
        Self {
            chord_pattern: DEFAULT_CHORD_MASK,
            scale_pattern: DEFAULT_SCALE_MASK,
            chord_root: 0,
            scale_root: 0,
            levels: 1,
        }
    }
}

/// One chord event in a segment's chord track (`crdb`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChordEvent {
    pub time: u32,
    pub subchords: Vec<SubChord>,
}

/// One command event in a segment's command track (`cmnd`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandEvent {
    pub time: u32,
    pub groove_level: u8,
}

/// One tempo event in a segment's tempo track (`tetr`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TempoEvent {
    pub time: u32,
    pub millibpm: u32,
}

/// One style reference in a segment's style track (`LIST:strf`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyleRef {
    pub time: u32,
    pub guid: [u8; 16],
    pub name: String,
    pub file: String,
}

/// One timed band change in a segment's band track (`LIST:lbnd`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BandEvent {
    pub time: u32,
    pub band: Band,
}

/// One direct MIDI sequence note in a sequence track (`seqt`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SeqItem {
    pub time: u32,
    pub duration: u32,
    pub pchannel: u32,
    pub status: u8,
    pub byte1: u8,
    pub byte2: u8,
}

/// A DirectMusic segment (`RIFF:DMSG`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Segment {
    pub guid: [u8; 16],
    pub name: String,
    pub repeats: u32,
    pub length: u32,
    pub play_start: u32,
    pub loop_start: u32,
    pub loop_end: u32,
    pub default_scale_root: u8,
    pub default_scale_mask: u32,
    pub chords: Vec<ChordEvent>,
    pub commands: Vec<CommandEvent>,
    pub tempos: Vec<TempoEvent>,
    pub styles: Vec<StyleRef>,
    pub bands: Vec<BandEvent>,
    pub seq_items: Vec<SeqItem>,
}

/// Convert a DirectMusic `wMusicValue` and effective playmode into a MIDI note
/// number (`0..=127`) against the active chord and scale.
#[must_use]
pub fn music_value_to_midi(music_value: u16, play_mode: u8, subchord: SubChord) -> u8 {
    if play_mode == DMUS_PLAYMODE_FIXED {
        return (music_value & 0x7f) as u8;
    }

    let octave = i32::from((music_value >> 12) & 0x0f);
    let chord_tone = usize::from((music_value >> 8) & 0x0f);
    let scale_tone = usize::from((music_value >> 4) & 0x0f);
    let raw_acc = i32::from(music_value & 0x0f);
    let accidental = if raw_acc >= 8 { raw_acc - 16 } else { raw_acc };

    let root = if (play_mode & DMUS_PLAYMODE_CHORD_ROOT) != 0 {
        i32::from(subchord.chord_root)
    } else if (play_mode & DMUS_PLAYMODE_KEY_ROOT) != 0 {
        i32::from(subchord.scale_root)
    } else {
        0
    };

    let chord_offset = if (play_mode & DMUS_PLAYMODE_CHORD_INTERVALS) != 0 {
        nth_set_bit(subchord.chord_pattern, chord_tone)
    } else {
        0
    };

    let mut pitch = root + chord_offset;
    if (play_mode & DMUS_PLAYMODE_SCALE_INTERVALS) != 0
        && scale_tone > 0
        && (subchord.scale_pattern & 0x0fff) != 0
    {
        let scale_root = i32::from(subchord.scale_root);
        let mut steps = 0usize;
        for _ in 0..128 {
            pitch += 1;
            let rel = (pitch - scale_root).rem_euclid(12);
            if ((subchord.scale_pattern >> rel) & 1) != 0 {
                steps += 1;
                if steps == scale_tone {
                    break;
                }
            }
        }
    }

    (octave * 12 + pitch + accidental).clamp(0, 127) as u8
}

/// Return the bit position of the `n`-th set bit in a 24-bit DirectMusic chord
/// mask, extending by octaves past the mask's span.
fn nth_set_bit(mask: u32, n: usize) -> i32 {
    if mask == 0 {
        return 0;
    }
    let mut bits = [0i32; 24];
    let mut count = 0usize;
    let mut oct_count = 0usize;
    for i in 0..24 {
        if ((mask >> i) & 1) != 0 {
            bits[count] = i;
            count += 1;
            if i < 12 {
                oct_count += 1;
            }
        }
    }
    if count == 0 {
        return 0;
    }
    if n < count {
        return bits[n];
    }
    if oct_count == 0 {
        return bits[n % count];
    }
    let extra = n - oct_count;
    bits[extra % oct_count] + 12 * (1 + (extra / oct_count) as i32)
}

/// Convert a DirectMusic tick span into declared nanoseconds at `millibpm`.
#[must_use]
pub fn ticks_to_nanos(ticks: u64, millibpm: u32) -> u64 {
    let denom = u128::from(millibpm.max(1_000)) * u128::from(DMUS_PPQ);
    let nanos = (u128::from(ticks) * 60_000_000_000_000) / denom;
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

/// Convert a declared nanosecond span into DirectMusic ticks at `millibpm`.
#[must_use]
pub fn nanos_to_ticks(nanos: u64, millibpm: u32) -> u64 {
    let numer = u128::from(nanos) * u128::from(millibpm.max(1_000)) * u128::from(DMUS_PPQ);
    u64::try_from(numer / 60_000_000_000_000).unwrap_or(u64::MAX)
}

/// Convert a positive IEEE-754 BPM value from a RIFF header into integer
/// millibpm, clamping out-of-range or NaN values to [`DEFAULT_MILLIBPM`].
fn bpm_to_millibpm(bpm: f64) -> u32 {
    if !bpm.is_finite() || bpm <= 1.0 || bpm > 1_000.0 {
        return DEFAULT_MILLIBPM;
    }
    let scaled = (bpm * 1000.0 + 0.5) as u64;
    u32::try_from(scaled).unwrap_or(DEFAULT_MILLIBPM).max(1_000)
}

/// Read a signed `MUSIC_TIME` (`LONG`), clamping negative pre-roll timestamps
/// (such as `-1` on initial band/chord events) to `0`.
fn music_time_at(b: &[u8], off: usize) -> u32 {
    (u32_at(b, off).unwrap_or(0) as i32).max(0) as u32
}

fn f64_at(b: &[u8], off: usize) -> Option<f64> {
    let bytes: [u8; 8] = b.get(off..off + 8)?.try_into().ok()?;
    Some(f64::from_le_bytes(bytes))
}

fn guid_at(b: &[u8], off: usize) -> Option<[u8; 16]> {
    b.get(off..off + 16)?.try_into().ok()
}

fn utf16le_z(b: &[u8]) -> String {
    let mut units = Vec::with_capacity(b.len() / 2);
    for pair in b.as_chunks::<2>().0 {
        let code = u16::from_le_bytes(*pair);
        if code == 0 {
            break;
        }
        units.push(code);
    }
    String::from_utf16_lossy(&units)
}

/// Iterator over sibling RIFF chunks in a slice.
struct Chunks<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Chunks<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = ([u8; 4], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let hdr = self.data.get(self.at..self.at + 8)?;
        let id = [hdr[0], hdr[1], hdr[2], hdr[3]];
        let size = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
        let start = self.at + 8;
        let end = start.saturating_add(size).min(self.data.len());
        let body = &self.data[start..end];
        self.at = end.saturating_add(size & 1);
        Some((id, body))
    }
}

/// Validate a top-level `RIFF:<form>` container and return its inner body slice.
fn riff_body<'a>(bytes: &'a [u8], expected_form: &[u8; 4]) -> Result<&'a [u8], &'static str> {
    let (id, body) = Chunks::new(bytes).next().ok_or("empty RIFF")?;
    if &id != b"RIFF" {
        return Err("missing RIFF header");
    }
    let (form, inner) = body.split_first_chunk::<4>().ok_or("truncated RIFF")?;
    if form != expected_form {
        return Err("unexpected RIFF form");
    }
    Ok(inner)
}

/// Iterate fixed-size records inside a DirectMusic table chunk (`u32 struct_size`
/// followed by packed records of `struct_size >= min_size` bytes).
fn records(payload: &[u8], min_size: usize) -> impl Iterator<Item = &[u8]> {
    let ssz = u32_at(payload, 0).unwrap_or(0) as usize;
    let body = if ssz >= min_size && payload.len() >= 4 {
        &payload[4..]
    } else {
        &[]
    };
    body.chunks_exact(ssz.max(1))
}

fn unfo_name(body: &[u8]) -> Option<String> {
    Chunks::new(body).find_map(|(id, p)| (&id == b"UNAM").then(|| utf16le_z(p)))
}

/// Parse a `RIFF:DMBD` band container.
///
/// # Errors
/// When the bytes are not a `RIFF:DMBD` chunk.
pub fn parse_band(bytes: &[u8]) -> Result<Band, &'static str> {
    Ok(parse_band_body(riff_body(bytes, b"DMBD")?))
}

fn parse_band_body(inner: &[u8]) -> Band {
    let mut band = Band::default();
    for (id, body) in Chunks::new(inner) {
        match &id {
            b"guid" => {
                if let Some(g) = guid_at(body, 0) {
                    band.guid = g;
                }
            }
            b"LIST" => {
                if let Some((form, sub)) = body.split_first_chunk::<4>() {
                    match form {
                        b"UNFO" => {
                            if let Some(name) = unfo_name(sub) {
                                band.name = name;
                            }
                        }
                        b"lbil" => {
                            for (lid, lbody) in Chunks::new(sub) {
                                if &lid == b"LIST" {
                                    if let Some((b"lbin", ibody)) = lbody.split_first_chunk::<4>() {
                                        parse_band_instrument(ibody, &mut band.instruments);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    band
}

fn parse_band_instrument(body: &[u8], out: &mut Vec<BandInstrument>) {
    for (id, payload) in Chunks::new(body) {
        if &id == b"bins" && payload.len() >= 34 {
            let patch = u32_at(payload, 0).unwrap_or(0);
            let pchannel = u32_at(payload, 24).unwrap_or(0);
            let flags = u32_at(payload, 28).unwrap_or(0);
            let pan = payload[32];
            let volume = payload[33];
            let transpose = if (flags & DMUS_IO_INST_TRANSPOSE) != 0 {
                u16_at(payload, 34).unwrap_or(0) as i16
            } else {
                0
            };
            out.push(BandInstrument {
                patch,
                pchannel,
                flags,
                pan,
                volume,
                transpose,
            });
        }
    }
}

/// Parse a `RIFF:DMST` DirectMusic style file.
///
/// # Errors
/// When the bytes are not a valid `RIFF:DMST` container.
pub fn parse_style(bytes: &[u8]) -> Result<Style, &'static str> {
    let inner = riff_body(bytes, b"DMST")?;
    let mut style = Style {
        millibpm: DEFAULT_MILLIBPM,
        ..Style::default()
    };

    for (cid, payload) in Chunks::new(inner) {
        match &cid {
            b"styh" if payload.len() >= 12 => {
                style.time_sig = TimeSig {
                    beats_per_measure: payload[0],
                    beat: payload[1],
                    grids_per_beat: u16_at(payload, 2).unwrap_or(4),
                };
                if let Some(tempo) = f64_at(payload, 4) {
                    style.millibpm = bpm_to_millibpm(tempo);
                }
            }
            b"guid" => {
                if let Some(g) = guid_at(payload, 0) {
                    style.guid = g;
                }
            }
            b"RIFF" => {
                if let Some((b"DMBD", sub)) = payload.split_first_chunk::<4>() {
                    style.bands.push(parse_band_body(sub));
                }
            }
            b"LIST" => {
                if let Some((form, sub)) = payload.split_first_chunk::<4>() {
                    match form {
                        b"UNFO" => {
                            if let Some(name) = unfo_name(sub) {
                                style.name = name;
                            }
                        }
                        b"part" => style.parts.push(parse_style_part(sub)),
                        b"pttn" => style.patterns.push(parse_style_pattern(sub)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(style)
}

fn parse_style_part(body: &[u8]) -> StylePart {
    let mut part = StylePart::default();
    for (id, payload) in Chunks::new(body) {
        match &id {
            b"prth" if payload.len() >= 151 => {
                part.time_sig = TimeSig {
                    beats_per_measure: payload[0],
                    beat: payload[1],
                    grids_per_beat: u16_at(payload, 2).unwrap_or(4),
                };
                if let Some(g) = guid_at(payload, 132) {
                    part.guid = g;
                }
                part.measures = u16_at(payload, 148).unwrap_or(1).max(1);
                part.play_mode = payload[150];
            }
            b"note" => {
                for rec in records(payload, 22) {
                    part.notes.push(StyleNote {
                        grid_start: music_time_at(rec, 0),
                        variation: u32_at(rec, 4).unwrap_or(0),
                        duration: music_time_at(rec, 8),
                        time_offset: u16_at(rec, 12).unwrap_or(0) as i16,
                        music_value: u16_at(rec, 14).unwrap_or(0),
                        velocity: rec[16],
                        play_mode: rec[21],
                    });
                }
            }
            b"crve" => {
                for rec in records(payload, 28) {
                    part.curves.push(StyleCurve {
                        grid_start: music_time_at(rec, 0),
                        variation: u32_at(rec, 4).unwrap_or(0),
                        duration: music_time_at(rec, 8),
                        reset_duration: music_time_at(rec, 12),
                        time_offset: u16_at(rec, 16).unwrap_or(0) as i16,
                        start_value: u16_at(rec, 18).unwrap_or(0) as i16,
                        end_value: u16_at(rec, 20).unwrap_or(0) as i16,
                        reset_value: u16_at(rec, 22).unwrap_or(0) as i16,
                        event_type: rec[24],
                        cc_data: rec[26],
                        flags: rec[27],
                    });
                }
            }
            _ => {}
        }
    }
    part
}

fn parse_style_pattern(body: &[u8]) -> Pattern {
    let mut pat = Pattern {
        measures: 1,
        ..Pattern::default()
    };
    for (id, payload) in Chunks::new(body) {
        match &id {
            b"ptnh" if payload.len() >= 10 => {
                pat.groove_bottom = payload[4];
                pat.groove_top = payload[5];
                pat.embellishment = u16_at(payload, 6).unwrap_or(0);
                pat.measures = u16_at(payload, 8).unwrap_or(1).max(1);
            }
            b"LIST" => {
                if let Some((b"pref", sub)) = payload.split_first_chunk::<4>() {
                    for (pid, pbody) in Chunks::new(sub) {
                        if &pid == b"prfc" && pbody.len() >= 20 {
                            let part_guid = guid_at(pbody, 0).unwrap_or([0; 16]);
                            // DX7+ 28-byte DMUS_IO_PARTREF puts 32-bit dwPChannel at +24;
                            // DX6 22-byte layout uses 16-bit wLogicalPartID at +16.
                            let pchannel = if pbody.len() >= 28 {
                                u32_at(pbody, 24).unwrap_or(0)
                            } else {
                                u32::from(u16_at(pbody, 16).unwrap_or(0))
                            };
                            pat.part_refs.push(PartRef {
                                part_guid,
                                pchannel,
                                subchord_level: pbody[19],
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    pat
}

/// Parse a `RIFF:DMSG` DirectMusic segment file.
///
/// # Errors
/// When the bytes are not a valid `RIFF:DMSG` container.
pub fn parse_segment(bytes: &[u8]) -> Result<Segment, &'static str> {
    let inner = riff_body(bytes, b"DMSG")?;
    let mut seg = Segment {
        default_scale_root: 0,
        default_scale_mask: DEFAULT_SCALE_MASK,
        ..Segment::default()
    };

    for (cid, payload) in Chunks::new(inner) {
        match &cid {
            b"segh" if payload.len() >= 20 => {
                seg.repeats = u32_at(payload, 0).unwrap_or(0);
                seg.length = music_time_at(payload, 4);
                seg.play_start = music_time_at(payload, 8);
                seg.loop_start = music_time_at(payload, 12);
                seg.loop_end = music_time_at(payload, 16);
            }
            b"guid" => {
                if let Some(g) = guid_at(payload, 0) {
                    seg.guid = g;
                }
            }
            b"LIST" => {
                if let Some((form, sub)) = payload.split_first_chunk::<4>() {
                    match form {
                        b"UNFO" => {
                            if let Some(name) = unfo_name(sub) {
                                seg.name = name;
                            }
                        }
                        b"trkl" => {
                            for (tid, tbody) in Chunks::new(sub) {
                                if &tid == b"RIFF" {
                                    if let Some((b"DMTK", track_inner)) =
                                        tbody.split_first_chunk::<4>()
                                    {
                                        parse_track(track_inner, &mut seg);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    seg.chords.sort_by_key(|c| c.time);
    seg.commands.sort_by_key(|c| c.time);
    seg.tempos.sort_by_key(|t| t.time);
    seg.styles.sort_by_key(|s| s.time);
    seg.bands.sort_by_key(|b| b.time);
    seg.seq_items.sort_by_key(|s| s.time);

    Ok(seg)
}

fn parse_track(inner: &[u8], seg: &mut Segment) {
    let mut track_class = [0u8; 16];
    let mut found_class = false;
    for (id, payload) in Chunks::new(inner) {
        if &id == b"trkh" {
            if let Some(g) = guid_at(payload, 0) {
                track_class = g;
                found_class = true;
            }
            continue;
        }
        if !found_class {
            continue;
        }
        if track_class == CLSID_CHORD_TRACK {
            if &id == b"LIST" {
                if let Some((b"cord", sub)) = payload.split_first_chunk::<4>() {
                    parse_chord_track(sub, seg);
                }
            }
        } else if track_class == CLSID_COMMAND_TRACK {
            if &id == b"cmnd" {
                for rec in records(payload, 10) {
                    seg.commands.push(CommandEvent {
                        time: music_time_at(rec, 0),
                        groove_level: rec[8],
                    });
                }
            }
        } else if track_class == CLSID_TEMPO_TRACK {
            if &id == b"tetr" {
                for rec in records(payload, 16) {
                    seg.tempos.push(TempoEvent {
                        time: music_time_at(rec, 0),
                        millibpm: bpm_to_millibpm(f64_at(rec, 8).unwrap_or(120.0)),
                    });
                }
            }
        } else if track_class == CLSID_STYLE_TRACK {
            if &id == b"LIST" {
                if let Some((b"sttr", sub)) = payload.split_first_chunk::<4>() {
                    parse_style_track(sub, &mut seg.styles);
                }
            }
        } else if track_class == CLSID_BAND_TRACK {
            if &id == b"RIFF" {
                if let Some((b"DMBT", sub)) = payload.split_first_chunk::<4>() {
                    parse_band_track(sub, &mut seg.bands);
                }
            }
        } else if track_class == CLSID_SEQ_TRACK && &id == b"seqt" {
            parse_seq_track(payload, &mut seg.seq_items);
        }
    }
}

fn parse_chord_track(body: &[u8], seg: &mut Segment) {
    for (id, payload) in Chunks::new(body) {
        match &id {
            b"crdh" if payload.len() >= 4 => {
                let raw = u32_at(payload, 0).unwrap_or(DEFAULT_SCALE_MASK);
                seg.default_scale_root = ((raw >> 24) & 0xff) as u8 % 12;
                let mask = raw & 0x00ff_ffff;
                seg.default_scale_mask = if mask == 0 { DEFAULT_SCALE_MASK } else { mask };
            }
            b"crdb" if payload.len() >= 4 => {
                let csz = u32_at(payload, 0).unwrap_or(0) as usize;
                if csz < 40 || 4 + csz + 8 > payload.len() {
                    continue;
                }
                let time = music_time_at(payload, 4 + 32);
                let count = u32_at(payload, 4 + csz).unwrap_or(0) as usize;
                let sub_sz = u32_at(payload, 4 + csz + 4).unwrap_or(0) as usize;
                if sub_sz < 18 {
                    continue;
                }
                let mut subchords = Vec::new();
                let mut off = 4 + csz + 8;
                for _ in 0..count {
                    if off + sub_sz > payload.len() {
                        break;
                    }
                    // DMUS_IO_SUBCHORD (dmusicf.h):
                    // +0 dwChordPattern, +4 dwScalePattern, +8 dwInversionPoints,
                    // +12 dwLevels, +16 bChordRoot, +17 bScaleRoot.
                    subchords.push(SubChord {
                        chord_pattern: u32_at(payload, off).unwrap_or(DEFAULT_CHORD_MASK),
                        scale_pattern: u32_at(payload, off + 4).unwrap_or(DEFAULT_SCALE_MASK),
                        levels: u32_at(payload, off + 12).unwrap_or(1),
                        chord_root: payload[off + 16] % 12,
                        scale_root: payload[off + 17] % 12,
                    });
                    off += sub_sz;
                }
                seg.chords.push(ChordEvent { time, subchords });
            }
            _ => {}
        }
    }
}

fn parse_style_track(body: &[u8], out: &mut Vec<StyleRef>) {
    for (id, payload) in Chunks::new(body) {
        if &id != b"LIST" {
            continue;
        }
        let Some((b"strf", sub)) = payload.split_first_chunk::<4>() else {
            continue;
        };
        let mut sref = StyleRef::default();
        for (sid, spayload) in Chunks::new(sub) {
            match &sid {
                b"stmp" if spayload.len() >= 4 => sref.time = music_time_at(spayload, 0),
                b"LIST" => {
                    if let Some((b"DMRF", rsub)) = spayload.split_first_chunk::<4>() {
                        for (rid, rbody) in Chunks::new(rsub) {
                            match &rid {
                                b"guid" => {
                                    if let Some(g) = guid_at(rbody, 0) {
                                        sref.guid = g;
                                    }
                                }
                                b"name" => sref.name = utf16le_z(rbody),
                                b"file" => sref.file = utf16le_z(rbody),
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        out.push(sref);
    }
}

fn parse_band_track(body: &[u8], out: &mut Vec<BandEvent>) {
    for (id, payload) in Chunks::new(body) {
        if &id != b"LIST" {
            continue;
        }
        let Some((b"lbdl", lsub)) = payload.split_first_chunk::<4>() else {
            continue;
        };
        for (bid, bpayload) in Chunks::new(lsub) {
            if &bid != b"LIST" {
                continue;
            }
            let Some((b"lbnd", item_sub)) = bpayload.split_first_chunk::<4>() else {
                continue;
            };
            let mut time = 0u32;
            let mut band = None;
            for (iid, ipayload) in Chunks::new(item_sub) {
                match &iid {
                    b"bdih" if ipayload.len() >= 4 => time = music_time_at(ipayload, 0),
                    b"RIFF" => {
                        if let Some((b"DMBD", dmbd)) = ipayload.split_first_chunk::<4>() {
                            band = Some(parse_band_body(dmbd));
                        }
                    }
                    _ => {}
                }
            }
            if let Some(band) = band {
                out.push(BandEvent { time, band });
            }
        }
    }
}

fn parse_seq_track(body: &[u8], out: &mut Vec<SeqItem>) {
    for (id, payload) in Chunks::new(body) {
        if &id == b"evtl" {
            for rec in records(payload, 18) {
                // DMUS_IO_SEQ_ITEM (pack(2)): mtTime(4), mtDuration(4), dwPChannel(4),
                // nOffset(2), bStatus(1), bByte1(1), bByte2(1).
                let base = music_time_at(rec, 0);
                let n_offset = u16_at(rec, 12).unwrap_or(0) as i16;
                let time =
                    (i64::from(base) + i64::from(n_offset)).clamp(0, i64::from(u32::MAX)) as u32;
                out.push(SeqItem {
                    time,
                    duration: music_time_at(rec, 4),
                    pchannel: u32_at(rec, 8).unwrap_or(0),
                    status: rec[14],
                    byte1: rec[15],
                    byte2: rec[16],
                });
            }
        }
    }
}

/// Active subchord for a segment at `tick` and `subchord_level`.
#[must_use]
pub fn active_subchord(seg: &Segment, tick: u32, level: u8) -> SubChord {
    let default = SubChord {
        chord_pattern: DEFAULT_CHORD_MASK,
        scale_pattern: seg.default_scale_mask,
        chord_root: seg.default_scale_root,
        scale_root: seg.default_scale_root,
        levels: 1,
    };
    let Some(chord) = seg.chords.iter().rfind(|c| c.time <= tick) else {
        return default;
    };
    let level_bit = 1u32.checked_shl(u32::from(level)).unwrap_or(1);
    chord
        .subchords
        .iter()
        .find(|s| (s.levels & level_bit) != 0)
        .or_else(|| chord.subchords.first())
        .copied()
        .unwrap_or(default)
}

/// Active tempo in millibpm for a segment (falling back to style tempo, then 120 BPM).
#[must_use]
pub fn active_millibpm(seg: &Segment, tick: u32, style: Option<&Style>) -> u32 {
    seg.tempos
        .iter()
        .rfind(|t| t.time <= tick)
        .map(|t| t.millibpm)
        .or_else(|| style.map(|s| s.millibpm))
        .unwrap_or(DEFAULT_MILLIBPM)
}

/// Active effective groove level (`1..=100`) for a segment at `tick`, given
/// the performance's master groove offset (`GUID_PerfMasterGrooveLevel`, signed `i8`).
#[must_use]
pub fn active_groove(seg: &Segment, tick: u32, master_groove_offset: i8) -> u8 {
    let base = seg
        .commands
        .iter()
        .rfind(|c| c.time <= tick && c.groove_level > 0)
        .map_or(1u8, |c| c.groove_level);
    (i16::from(base) + i16::from(master_groove_offset)).clamp(1, 100) as u8
}

/// Choose a pattern deterministically from `style` for `groove` (`1..=100`).
#[must_use]
pub fn select_pattern(style: &Style, groove: u8) -> Option<&Pattern> {
    style
        .patterns
        .iter()
        .filter(|p| p.embellishment == 0)
        .min_by_key(|p| {
            groove
                .saturating_sub(p.groove_top)
                .max(p.groove_bottom.saturating_sub(groove))
        })
        .or_else(|| style.patterns.first())
}

fn emit_curve_value(ch: u32, curve: &StyleCurve, value: i16, tick: u32, out: &mut Vec<(u32, u32)>) {
    match curve.event_type {
        DMUS_CURVET_CCCURVE => {
            let val = (value.clamp(0, 127) as u32) & 0x7f;
            out.push((
                tick,
                0xb0 | ch | (u32::from(curve.cc_data & 0x7f) << 8) | (val << 16),
            ));
        }
        DMUS_CURVET_PBCURVE => {
            let bend = value.clamp(0, 16383) as u32;
            out.push((
                tick,
                0xe0 | ch | ((bend & 0x7f) << 8) | (((bend >> 7) & 0x7f) << 16),
            ));
        }
        _ => {}
    }
}

/// Expand one measure of a segment starting at `measure_tick` into timestamped
/// `(tick, short_midi_msg)` events (with ticks relative to segment start),
/// updating `channel_transpose` from any band events applied in this measure.
pub fn render_measure<'a, F>(
    seg: &Segment,
    measure_tick: u32,
    measure_ticks: u32,
    master_groove_offset: i8,
    channel_transpose: &mut [i16; 16],
    mut resolve_style: F,
    out: &mut Vec<(u32, u32)>,
) where
    F: FnMut(&StyleRef) -> Option<&'a Style>,
{
    let measure_end = measure_tick.saturating_add(measure_ticks);

    // 1. Direct band track events in this measure.
    for ev in &seg.bands {
        if ev.time >= measure_tick && ev.time < measure_end {
            for inst in &ev.band.instruments {
                inst.emit_midi(ev.time, out);
                if (inst.flags & DMUS_IO_INST_TRANSPOSE) != 0 && inst.pchannel < 16 {
                    channel_transpose[inst.pchannel as usize] = inst.transpose;
                }
            }
        }
    }

    // 2. Direct sequence track events in this measure.
    for item in &seg.seq_items {
        if item.time >= measure_tick && item.time < measure_end && item.pchannel < 16 {
            let status = (item.status & 0xf0) | (item.pchannel as u8 & 0x0f);
            let msg = u32::from(status)
                | (u32::from(item.byte1 & 0x7f) << 8)
                | (u32::from(item.byte2 & 0x7f) << 16);
            out.push((item.time, msg));
            if (status & 0xf0) == 0x90 && item.byte2 > 0 && item.duration > 0 {
                let off_msg = u32::from(0x80 | (item.pchannel as u8 & 0x0f))
                    | (u32::from(item.byte1 & 0x7f) << 8);
                out.push((item.time.saturating_add(item.duration), off_msg));
            }
        }
    }

    // 3. Active style reference for this measure.
    let Some(sref) = seg.styles.iter().rfind(|s| s.time <= measure_tick) else {
        return;
    };
    let Some(style) = resolve_style(sref) else {
        return;
    };

    if seg.bands.is_empty() && sref.time == measure_tick {
        if let Some(band) = style.bands.first() {
            for inst in &band.instruments {
                inst.emit_midi(measure_tick, out);
                if (inst.flags & DMUS_IO_INST_TRANSPOSE) != 0 && inst.pchannel < 16 {
                    channel_transpose[inst.pchannel as usize] = inst.transpose;
                }
            }
        }
    }

    let groove = active_groove(seg, measure_tick, master_groove_offset);
    let Some(pat) = select_pattern(style, groove) else {
        return;
    };

    let pat_measures = u32::from(pat.measures.max(1));
    let elapsed_from_ref = measure_tick.saturating_sub(sref.time);
    let measure_in_pat = (elapsed_from_ref / measure_ticks.max(1)) % pat_measures;

    for pref in &pat.part_refs {
        if pref.pchannel >= 16 {
            continue;
        }
        let ch = pref.pchannel & 0x0f;
        let Some(part) = style.parts.iter().find(|p| p.guid == pref.part_guid) else {
            continue;
        };

        let part_measures = u32::from(part.measures.max(1));
        let part_measure = measure_in_pat % part_measures;
        let grids_per_measure = u32::from(part.time_sig.beats_per_measure.max(1))
            .saturating_mul(u32::from(part.time_sig.grids_per_beat.max(1)));
        let grid_lo = part_measure.saturating_mul(grids_per_measure);
        let grid_hi = grid_lo.saturating_add(grids_per_measure);
        let grid_ticks = part.time_sig.grid_ticks() as u32;
        let active_var_mask = lowest_variation_bit(&part.notes);

        for curve in &part.curves {
            if (curve.variation & active_var_mask) == 0
                || curve.grid_start < grid_lo
                || curve.grid_start >= grid_hi
            {
                continue;
            }
            let rel_tick = curve
                .grid_start
                .saturating_sub(grid_lo)
                .saturating_mul(grid_ticks);
            let fire_tick =
                (i64::from(measure_tick) + i64::from(rel_tick) + i64::from(curve.time_offset))
                    .clamp(i64::from(measure_tick), i64::from(u32::MAX)) as u32;
            emit_curve_value(ch, curve, curve.start_value, fire_tick, out);
            let end_tick = fire_tick.saturating_add(curve.duration);
            if curve.end_value != curve.start_value {
                emit_curve_value(ch, curve, curve.end_value, end_tick, out);
            }
            if (curve.flags & DMUS_CURVE_RESET) != 0 {
                let reset_tick = end_tick.saturating_add(curve.reset_duration);
                emit_curve_value(ch, curve, curve.reset_value, reset_tick, out);
            }
        }

        let transpose = if ch as usize == DRUM_CHANNEL {
            0
        } else {
            i32::from(channel_transpose[ch as usize])
        };

        for note in &part.notes {
            if note.velocity == 0
                || (note.variation & active_var_mask) == 0
                || note.grid_start < grid_lo
                || note.grid_start >= grid_hi
            {
                continue;
            }
            let rel_tick = note
                .grid_start
                .saturating_sub(grid_lo)
                .saturating_mul(grid_ticks);
            let on_tick =
                (i64::from(measure_tick) + i64::from(rel_tick) + i64::from(note.time_offset))
                    .clamp(i64::from(measure_tick), i64::from(u32::MAX)) as u32;
            let eff_mode = if note.play_mode == DMUS_PLAYMODE_NONE {
                part.play_mode
            } else {
                note.play_mode
            };
            let subchord = active_subchord(seg, on_tick, pref.subchord_level);
            let base_key = i32::from(music_value_to_midi(note.music_value, eff_mode, subchord));
            let midi_key = (base_key + transpose).clamp(0, 127) as u32;
            let vel = u32::from(note.velocity.min(127));
            let dur = note.duration.max(1);
            out.push((on_tick, 0x90 | ch | (midi_key << 8) | (vel << 16)));
            out.push((on_tick.saturating_add(dur), 0x80 | ch | (midi_key << 8)));
        }
    }
}

fn lowest_variation_bit(notes: &[StyleNote]) -> u32 {
    let combined = notes.iter().fold(0u32, |acc, n| acc | n.variation);
    if combined == 0 {
        1
    } else {
        1u32 << combined.trailing_zeros()
    }
}

// ---------------------------------------------------------------------------
// LLSNAP v2 Snap implementations (exhaustive field destructuring)
// ---------------------------------------------------------------------------

impl Snap for BandInstrument {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            patch,
            pchannel,
            flags,
            pan,
            volume,
            transpose,
        } = self;
        v.u32(patch);
        v.u32(pchannel);
        v.u32(flags);
        v.u8(pan);
        v.u8(volume);
        v.i16(transpose);
    }
}

impl Snap for Band {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            guid,
            name,
            instruments,
        } = self;
        v.bytes(guid);
        v.string(name);
        snap_vec(v, instruments, |inst, v| inst.snap(v));
    }
}

impl Snap for SubChord {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            chord_pattern,
            scale_pattern,
            chord_root,
            scale_root,
            levels,
        } = self;
        v.u32(chord_pattern);
        v.u32(scale_pattern);
        v.u8(chord_root);
        v.u8(scale_root);
        v.u32(levels);
    }
}

impl Snap for ChordEvent {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { time, subchords } = self;
        v.u32(time);
        snap_vec(v, subchords, |s, v| s.snap(v));
    }
}

impl Snap for CommandEvent {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { time, groove_level } = self;
        v.u32(time);
        v.u8(groove_level);
    }
}

impl Snap for TempoEvent {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { time, millibpm } = self;
        v.u32(time);
        v.u32(millibpm);
    }
}

impl Snap for StyleRef {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            time,
            guid,
            name,
            file,
        } = self;
        v.u32(time);
        v.bytes(guid);
        v.string(name);
        v.string(file);
    }
}

impl Snap for BandEvent {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self { time, band } = self;
        v.u32(time);
        band.snap(v);
    }
}

impl Snap for SeqItem {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            time,
            duration,
            pchannel,
            status,
            byte1,
            byte2,
        } = self;
        v.u32(time);
        v.u32(duration);
        v.u32(pchannel);
        v.u8(status);
        v.u8(byte1);
        v.u8(byte2);
    }
}

impl Snap for Segment {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            guid,
            name,
            repeats,
            length,
            play_start,
            loop_start,
            loop_end,
            default_scale_root,
            default_scale_mask,
            chords,
            commands,
            tempos,
            styles,
            bands,
            seq_items,
        } = self;
        v.bytes(guid);
        v.string(name);
        v.u32(repeats);
        v.u32(length);
        v.u32(play_start);
        v.u32(loop_start);
        v.u32(loop_end);
        v.u8(default_scale_root);
        v.u32(default_scale_mask);
        snap_vec(v, chords, |c, v| c.snap(v));
        snap_vec(v, commands, |c, v| c.snap(v));
        snap_vec(v, tempos, |t, v| t.snap(v));
        snap_vec(v, styles, |s, v| s.snap(v));
        snap_vec(v, bands, |b, v| b.snap(v));
        snap_vec(v, seq_items, |s, v| s.snap(v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn riff(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len() + 1);
        out.extend_from_slice(id);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() & 1 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], form: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(form);
        for c in children {
            body.extend_from_slice(c);
        }
        riff(kind, &body)
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    #[test]
    fn purpleized_and_fixed_playmodes_decode_exact_midi_pitches() {
        let c_maj = SubChord::default();
        assert_eq!(music_value_to_midi(36, DMUS_PLAYMODE_FIXED, c_maj), 36);
        assert_eq!(music_value_to_midi(84, DMUS_PLAYMODE_FIXED, c_maj), 84);

        let purple = DMUS_PLAYMODE_CHORD_ROOT
            | DMUS_PLAYMODE_SCALE_INTERVALS
            | DMUS_PLAYMODE_CHORD_INTERVALS;
        assert_eq!(music_value_to_midi(0x5000, purple, c_maj), 60);
        assert_eq!(music_value_to_midi(0x5100, purple, c_maj), 64);
        assert_eq!(music_value_to_midi(0x5200, purple, c_maj), 67);
        assert_eq!(music_value_to_midi(0x5010, purple, c_maj), 62);
        assert_eq!(music_value_to_midi(0x500f, purple, c_maj), 59);
    }

    #[test]
    fn tick_and_nanosecond_conversions_are_exact_at_standard_tempos() {
        assert_eq!(ticks_to_nanos(768, 60_000), 1_000_000_000);
        assert_eq!(ticks_to_nanos(3072, 60_000), 4_000_000_000);
        assert_eq!(nanos_to_ticks(1_000_000_000, 60_000), 768);
        assert_eq!(ticks_to_nanos(768, 120_000), 500_000_000);
        assert_eq!(nanos_to_ticks(500_000_000, 120_000), 768);
    }

    #[test]
    fn negative_bdih_band_flags_transpose_subchord_offsets_and_pbcurve_reset() {
        let part_guid = [1u8; 16];
        let style_guid = [2u8; 16];

        let mut styh = vec![4u8, 4, 8, 0];
        styh.extend_from_slice(&60.0f64.to_le_bytes());

        let mut prth = vec![0u8; 154];
        prth[0] = 4;
        prth[1] = 4;
        prth[2..4].copy_from_slice(&8u16.to_le_bytes());
        prth[132..148].copy_from_slice(&part_guid);
        prth[148..150].copy_from_slice(&1u16.to_le_bytes());
        prth[150] = 0x0e;

        let mut note = Vec::new();
        note.extend_from_slice(&22u32.to_le_bytes());
        note.extend_from_slice(&0u32.to_le_bytes());
        note.extend_from_slice(&1u32.to_le_bytes());
        note.extend_from_slice(&384u32.to_le_bytes());
        note.extend_from_slice(&0i16.to_le_bytes());
        note.extend_from_slice(&0x5000u16.to_le_bytes()); // C4 before chord/transpose
        note.extend_from_slice(&[100, 0, 0, 0, 0, DMUS_PLAYMODE_NONE]);

        // Pitch-bend curve with DMUS_CURVE_RESET back to center (8192).
        let mut crve = Vec::new();
        crve.extend_from_slice(&28u32.to_le_bytes());
        crve.extend_from_slice(&0u32.to_le_bytes()); // grid 0
        crve.extend_from_slice(&1u32.to_le_bytes()); // var 0
        crve.extend_from_slice(&100u32.to_le_bytes()); // duration 100
        crve.extend_from_slice(&50u32.to_le_bytes()); // reset_duration 50
        crve.extend_from_slice(&0i16.to_le_bytes()); // time_offset
        crve.extend_from_slice(&8192i16.to_le_bytes()); // start_value (center)
        crve.extend_from_slice(&4096i16.to_le_bytes()); // end_value (bend down)
        crve.extend_from_slice(&8192i16.to_le_bytes()); // reset_value (center)
        crve.extend_from_slice(&[DMUS_CURVET_PBCURVE, 0, 0, DMUS_CURVE_RESET]);

        let mut ptnh = vec![4u8, 4, 8, 0, 1, 100];
        ptnh.extend_from_slice(&0u16.to_le_bytes());
        ptnh.extend_from_slice(&1u16.to_le_bytes());

        let mut prfc = vec![0u8; 22];
        prfc[0..16].copy_from_slice(&part_guid);
        prfc[16..18].copy_from_slice(&2u16.to_le_bytes()); // channel 2

        let style_bytes = list(
            b"RIFF",
            b"DMST",
            &[
                riff(b"styh", &styh),
                riff(b"guid", &style_guid),
                list(b"LIST", b"UNFO", &[riff(b"UNAM", &utf16("TestStyle"))]),
                list(
                    b"LIST",
                    b"part",
                    &[
                        riff(b"prth", &prth),
                        riff(b"note", &note),
                        riff(b"crve", &crve),
                    ],
                ),
                list(
                    b"LIST",
                    b"pttn",
                    &[
                        riff(b"ptnh", &ptnh),
                        list(b"LIST", b"pref", &[riff(b"prfc", &prfc)]),
                    ],
                ),
            ],
        );
        let style = parse_style(&style_bytes).expect("parse style");

        // Segment with:
        // - bdih = -1 (0xFFFF_FFFF) initial band carrying PATCH | PAN | VOLUME | TRANSPOSE (+12)
        // - crdb subchord with dwLevels = 1 (+12), bChordRoot = 7 (G, +16), bScaleRoot = 0 (+17)
        let mut segh = Vec::new();
        for w in [0u32, 3072, 0, 0, 3072, 0] {
            segh.extend_from_slice(&w.to_le_bytes());
        }
        let mut chord_trkh = Vec::from(CLSID_CHORD_TRACK);
        chord_trkh.extend_from_slice(&[0u8; 16]);
        let mut crdb = Vec::new();
        crdb.extend_from_slice(&40u32.to_le_bytes()); // sizeof(DMUS_IO_CHORD)
        crdb.extend_from_slice(&[0u8; 32]); // wszName
        crdb.extend_from_slice(&0u32.to_le_bytes()); // mtTime
        crdb.extend_from_slice(&[0u8; 4]); // wMeasure, bBeat, bFlags
        crdb.extend_from_slice(&1u32.to_le_bytes()); // count = 1
        crdb.extend_from_slice(&20u32.to_le_bytes()); // sub_sz = 20
        crdb.extend_from_slice(&DEFAULT_CHORD_MASK.to_le_bytes()); // +0 chord_pattern
        crdb.extend_from_slice(&DEFAULT_SCALE_MASK.to_le_bytes()); // +4 scale_pattern
        crdb.extend_from_slice(&0u32.to_le_bytes()); // +8 inversion_points
        crdb.extend_from_slice(&1u32.to_le_bytes()); // +12 dwLevels = bit 0
        crdb.extend_from_slice(&[7u8, 0u8, 0, 0]); // +16 bChordRoot=G(7), +17 bScaleRoot=C(0)
        let chord_track = list(
            b"RIFF",
            b"DMTK",
            &[
                riff(b"trkh", &chord_trkh),
                list(b"LIST", b"cord", &[riff(b"crdb", &crdb)]),
            ],
        );

        let mut style_trkh = Vec::from(CLSID_STYLE_TRACK);
        style_trkh.extend_from_slice(&[0u8; 16]);
        let style_track = list(
            b"RIFF",
            b"DMTK",
            &[
                riff(b"trkh", &style_trkh),
                list(
                    b"LIST",
                    b"sttr",
                    &[list(
                        b"LIST",
                        b"strf",
                        &[
                            riff(b"stmp", &0u32.to_le_bytes()),
                            list(
                                b"LIST",
                                b"DMRF",
                                &[
                                    riff(b"guid", &style_guid),
                                    riff(b"name", &utf16("TestStyle")),
                                    riff(b"file", &utf16("test.sty")),
                                ],
                            ),
                        ],
                    )],
                ),
            ],
        );

        let mut band_trkh = Vec::from(CLSID_BAND_TRACK);
        band_trkh.extend_from_slice(&[0u8; 16]);
        let mut bins = vec![0u8; 40];
        bins[0..4].copy_from_slice(&24u32.to_le_bytes()); // Acoustic Guitar
        bins[24..28].copy_from_slice(&2u32.to_le_bytes()); // pchannel 2
        let flags =
            DMUS_IO_INST_PATCH | DMUS_IO_INST_PAN | DMUS_IO_INST_VOLUME | DMUS_IO_INST_TRANSPOSE;
        bins[28..32].copy_from_slice(&flags.to_le_bytes());
        bins[32] = 45; // pan
        bins[33] = 96; // volume
        bins[34..36].copy_from_slice(&12i16.to_le_bytes()); // +1 octave transpose
        let band_track = list(
            b"RIFF",
            b"DMTK",
            &[
                riff(b"trkh", &band_trkh),
                list(
                    b"RIFF",
                    b"DMBT",
                    &[list(
                        b"LIST",
                        b"lbdl",
                        &[list(
                            b"LIST",
                            b"lbnd",
                            &[
                                riff(b"bdih", &(-1i32).to_le_bytes()),
                                list(
                                    b"RIFF",
                                    b"DMBD",
                                    &[list(
                                        b"LIST",
                                        b"lbil",
                                        &[list(b"LIST", b"lbin", &[riff(b"bins", &bins)])],
                                    )],
                                ),
                            ],
                        )],
                    )],
                ),
            ],
        );

        let seg_bytes = list(
            b"RIFF",
            b"DMSG",
            &[
                riff(b"segh", &segh),
                list(b"LIST", b"trkl", &[chord_track, style_track, band_track]),
            ],
        );
        let seg = parse_segment(&seg_bytes).expect("parse segment");
        assert_eq!(seg.bands[0].time, 0, "bdih -1 clamps to 0");
        assert_eq!(seg.chords[0].subchords[0].chord_root, 7);
        assert_eq!(seg.chords[0].subchords[0].levels, 1);

        let mut transpose = [0i16; 16];
        let mut events = Vec::new();
        render_measure(
            &seg,
            0,
            3072,
            0,
            &mut transpose,
            |_| Some(&style),
            &mut events,
        );
        assert_eq!(transpose[2], 12);
        // Band emits program 24, CC7 volume 96, CC10 pan 45 on channel 2.
        assert!(events.contains(&(0, 0x00_18_c2)));
        assert!(events.contains(&(0, 0x60_07_b2)));
        assert!(events.contains(&(0, 0x2d_0a_b2)));
        // Note: octave 5 (60) + G chord root (+7) + +12 transpose = 79 (G5).
        assert!(events.contains(&(0, 0x64_4f_92)));
        assert!(events.contains(&(384, 0x00_4f_82)));
        // Pitch bend bends to 4096 (0x2000 -> 0x1000: msb=0x20, lsb=0x00) at tick 100
        // and resets to 8192 (msb=0x40, lsb=0x00) at tick 150.
        assert!(events.contains(&(100, 0x20_00_e2)));
        assert!(events.contains(&(150, 0x40_00_e2)));
    }
}
