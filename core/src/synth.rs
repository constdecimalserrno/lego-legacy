//! The machine's MIDI out: a General MIDI synthesiser playing a DLS Level 1
//! bank into the mix on the declared clock (ADR-0008), as the Microsoft GS
//! Wavetable SW Synth plays the player's own copy of its bank.
//!
//! A voice is a region's wave played at the pitch its key, tuning, pitch
//! bend, LFO and pitch envelope add up to, through an amplifier whose gain
//! its volume envelope, velocity, volume, expression, LFO and pan add up to,
//! as the DLS Level 1 device architecture (MMA, v1.1b, read with its errata)
//! connects them. What that leaves to the device is declared here, each with
//! its reason:
//!
//! - **32 voices**, what the GS Wavetable synth had on Windows 98 Second
//!   Edition ([`crate::title::WINDOWS_98_SE`]), the Windows the roster
//!   declares. Microsoft documents no count; the one account found (gearnews,
//!   Robin Vincent, 26 March 2017) has it "moving from 32 note polyphony to
//!   64 note polyphony with the arrival of Windows XP in 2001". DLS Level 1
//!   and General MIDI Level 1 ask for at least 24. LEGO Chess's four tunes
//!   reach 40 to 55 at once with every release heard to its end. At 32, played
//!   with the player's bank, they steal 620 times, 611 of them a release
//!   tail, and 66 notes are never played, 62 of them on channel 16, which may
//!   take a voice from no other channel: DLS Level 1's rule on a 32-voice
//!   device. A voice that has finished frees its place before a note looks
//!   for one.
//! - **A note past the last voice steals one by DLS Level 1's static channel
//!   priority:** channel 10, then 1 to 9, then 11 to 16, and never from a
//!   channel above its own. The specification leaves the choice within a
//!   channel to the device: a released voice goes before a held one, and the
//!   oldest first, which loses the least that is still heard. A stolen voice
//!   stops on the frame the new note starts.
//! - **A key struck again ends the notes on it that have had no note off,**
//!   within 15 ms: the specification ends only those, and its errata prefer a
//!   note ended so to shut down in 15 ms. A note the pedal keeps has had its
//!   note off, and rings on. A region marked self-nonexclusive ends none.
//! - **The oscillator draws straight between the two samples either side.**
//!   Microsoft's own interpolation is not documented; linear is what a
//!   software synthesiser of 1999 spent on every voice, and what the mix
//!   already does for a buffer at another rate ([`crate::sound::Pcm`]), so
//!   it is close without claiming to be the same.
//! - **Envelopes and the LFO move every frame**, as the specification draws
//!   them: the volume envelope's attack is linear in amplitude, its decay and
//!   release linear in decibels at 96 dB per their time, and the pitch
//!   envelope linear in cents throughout. A segment of no time takes no frame.
//!   A voice whose volume envelope has reached -96 dB is done.
//! - **The LFO is a sine,** the shape Level 1 asks for, from zero once its
//!   delay is over, and swings gain as it swings pitch, both ways. The
//!   specification disagrees with itself here: its prose has the LFO move
//!   attenuation one way only, and its worked example, like the connection
//!   block's own arithmetic, swings it both. The example is taken.
//! - **Gain is added up in centibels and stops at 0 dB,** as the
//!   specification's summing node does, and 0 dB there is full scale: nothing
//!   further scales the synth in the mix. Pan is the specification's equal
//!   power.
//! - **A drum's key group cuts its others off in 15 ms,** the errata's
//!   preference.
//! - **A variation the bank lacks** plays its program's capital tone, in bank
//!   0, as a GS Sound Canvas does; a drum kit it lacks plays the standard kit,
//!   program 0.
//! - **The controllers General MIDI Level 1 names are answered:** modulation,
//!   volume, pan, expression, sustain, data entry for the pitch bend range and
//!   fine and coarse tuning, reset all controllers and all notes off, with
//!   bank select for the GS kits. MIDI 1.0's other channel mode messages are
//!   answered as it says: all sound off silences the channel at once, and
//!   omni off, omni on, mono on and poly on end its notes as all notes off
//!   does, the synth staying polyphonic. Key and channel pressure reach no
//!   connection in DLS Level 1, and nothing else is answered. There is no
//!   reverb or chorus: Level 1 has none, and Microsoft's own are not
//!   documented.
//!
//! **Deterministic by construction.** Every sample is integer arithmetic
//! from the messages and the frames they come at, so the same messages at the
//! same frames give the same samples on every host, headless or not. Three
//! things need a transcendental: a ratio from cents, an amplitude from
//! centibels and the LFO's sine. They read tables built once with
//! [`crate::cpu::x87::maths`], whose only arithmetic is IEEE-754 addition,
//! multiplication, division and rounding, which every host's processor rounds
//! alike and which Rust never fuses, and a test pins the tables' hash. The one
//! float a note meets is when it is let go in its attack: it finds its
//! decibels with the same maths.

use crate::dls::{Bank, Envelope, Instrument, Region, NO_TIME};
use std::collections::VecDeque;
use std::sync::OnceLock;

/// The mix's rate, which is DLS Level 1's least output rate too.
const RATE: u64 = crate::sound::RATE as u64;

/// How many notes sound at once.
pub const VOICES: usize = 32;

/// MIDI's tenth channel, the drums'.
const DRUMS: usize = 9;

/// The top of an envelope or a level, in fixed point.
const ONE: u32 = 1 << 30;

/// -96 dB, in 1/65,536 centibels: where DLS takes sound to have ended.
const FLOOR: i64 = 960 << 16;

/// The oscillator's reach, in 1/65,536 cents: up four octaves and down
/// eight, past the two and four DLS Level 1 asks for.
const PITCH: (i64, i64) = (-9600 << 16, 4800 << 16);

/// What the transcendentals give, built once.
struct Tables {
    /// Two to the power of each cent of an octave over 1200, in Q30.
    octave: [u32; 1201],
    /// One turn of a sine in 256 steps, in Q15.
    sine: [i16; 256],
    /// The concave transform: the attenuation each value of a velocity,
    /// volume or expression gives, `20·log10(127²/v²)` dB, in 1/65,536
    /// centibels, nothing at all being the floor.
    concave: [i64; 128],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        use crate::cpu::x87::maths;
        Tables {
            octave: std::array::from_fn(|i| {
                (maths::pow2(i as f64 / 1200.0) * f64::from(ONE)).round() as u32
            }),
            sine: std::array::from_fn(|i| {
                (maths::sin(i as f64 * maths::PI / 128.0) * 32767.0).round() as i16
            }),
            concave: std::array::from_fn(|v| match v {
                0 => FLOOR,
                _ => (400.0 * maths::log2(127.0 / v as f64) / std::f64::consts::LOG2_10 * 65536.0)
                    .round() as i64,
            }),
        }
    })
}

/// Two to the power of a number of cents over 1200, the cents in 1/65,536ths,
/// in Q32: what a pitch, a time and a gain all become.
fn pow2(cents: i64) -> u64 {
    const OCTAVE: i64 = 1200 << 16;
    let (octaves, within) = (cents.div_euclid(OCTAVE), cents.rem_euclid(OCTAVE));
    let table = &tables().octave;
    let (cent, part) = ((within >> 16) as usize, within & 0xffff);
    let (a, b) = (i64::from(table[cent]), i64::from(table[cent + 1]));
    let q30 = (a + (((b - a) * part) >> 16)) as u64;
    // From Q30 to Q32 is two places more; past 2^31 is more than anything
    // here asks for.
    match octaves + 2 {
        33.. => u64::MAX,
        shift @ 0.. => q30 << shift,
        shift @ -63.. => q30 >> -shift,
        _ => 0,
    }
}

/// The amplitude, in Q16, an attenuation in 1/65,536 centibels leaves: a
/// centibel is a two-hundredth of a decade, `6·log2(10)` cents of pitch.
fn amplitude(attenuation: i64) -> u64 {
    const CENTS_PER_CENTIBEL: i64 = 1_306_235;
    if attenuation >= FLOOR {
        return 0;
    }
    pow2(-((attenuation.max(0) * CENTS_PER_CENTIBEL) >> 16)) >> 16
}

/// How many frames of the mix a time in time cents lasts.
fn frames(timecents: i64) -> u64 {
    // Seven octaves of seconds, 128 s, is past every time Level 1 has.
    (RATE * pow2(timecents.min((7 * 1200) << 16))) >> 32
}

/// A segment's time, with what the velocity or the key adds to it: no time
/// at all stays so.
fn scaled(time: i32, by: i32, value: u8) -> u64 {
    match time {
        NO_TIME => 0,
        _ => frames(i64::from(time) + i64::from(by) * i64::from(value) / 128),
    }
}

/// Where an envelope is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
    Done,
}

/// An envelope as it runs: its stage, its level from nothing to [`ONE`], and
/// how far each frame of a segment moves it.
#[derive(Clone, Copy, Debug)]
struct Generator {
    stage: Stage,
    level: u32,
    attack: u32,
    decay: u32,
    sustain: u32,
    release: u32,
}

/// How far a frame moves a level that goes all the way in so many frames:
/// none for a segment of no time, which is over before a frame is.
fn step(frames: u64) -> u32 {
    match frames {
        0 => 0,
        _ => u64::from(ONE).div_ceil(frames) as u32,
    }
}

impl Generator {
    fn new(shape: &Envelope, velocity: u8, key: u8) -> Self {
        let attack = scaled(shape.attack, shape.velocity_attack, velocity);
        let decay = scaled(shape.decay, shape.key_decay, key);
        let sustain = i64::from(shape.sustain).clamp(0, 1000 << 16) * i64::from(ONE) / (1000 << 16);
        let mut eg = Self {
            stage: Stage::Attack,
            level: 0,
            attack: step(attack),
            decay: step(decay),
            sustain: sustain as u32,
            release: step(scaled(shape.release, 0, 0)),
        };
        eg.settle();
        eg
    }

    /// Past each segment of no time.
    fn settle(&mut self) {
        if self.stage == Stage::Attack && self.attack == 0 {
            (self.stage, self.level) = (Stage::Decay, ONE);
        }
        if self.stage == Stage::Decay && self.decay == 0 {
            (self.stage, self.level) = (Stage::Sustain, self.sustain);
        }
    }

    fn advance(&mut self) {
        match self.stage {
            Stage::Attack => {
                self.level = self.level.saturating_add(self.attack).min(ONE);
                if self.level == ONE {
                    self.stage = Stage::Decay;
                    self.settle();
                }
            }
            Stage::Decay => {
                self.level = self.level.saturating_sub(self.decay).max(self.sustain);
                if self.level == self.sustain {
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Release => {
                self.level = self.level.saturating_sub(self.release);
                if self.level == 0 {
                    self.stage = Stage::Done;
                }
            }
            Stage::Sustain | Stage::Done => {}
        }
    }

    fn release(&mut self) {
        if self.release == 0 {
            (self.stage, self.level) = (Stage::Done, 0);
        } else if self.stage != Stage::Done {
            self.stage = Stage::Release;
        }
    }
}

/// The LFO as it runs: the frames of its delay left, and where it is in its
/// turn and how far a frame takes it, as fractions of a turn in Q32.
#[derive(Clone, Copy, Debug)]
struct Lfo {
    delay: u64,
    phase: u32,
    step: u32,
}

impl Lfo {
    /// Where it is, in Q15.
    fn value(&self) -> i64 {
        if self.delay > 0 {
            0
        } else {
            i64::from(tables().sine[(self.phase >> 24) as usize])
        }
    }

    fn advance(&mut self) {
        if self.delay > 0 {
            self.delay -= 1;
        } else {
            self.phase = self.phase.wrapping_add(self.step);
        }
    }
}

/// One MIDI channel: the instrument it plays, and its controllers.
#[derive(Clone, Copy, Debug)]
struct Channel {
    instrument: Option<usize>,
    /// The bank controllers 0 and 32 have chosen for the next program change.
    bank: u16,
    volume: u8,
    expression: u8,
    pan: u8,
    modulation: u8,
    sustain: bool,
    /// The pitch wheel, from -8192 to 8191.
    bend: i16,
    /// The registered parameters: the pitch bend range in semitones and
    /// cents, fine tuning as fourteen bits about 8192, and coarse tuning in
    /// semitones about 64.
    range: (u8, u8),
    fine: u16,
    coarse: u8,
    /// The registered parameter number controllers 101 and 100 last chose,
    /// 127 being none, and whether a non-registered one was chosen since,
    /// which this synth has none of: MIDI 1.0 keeps each number's controllers
    /// apart, and data entry goes to whichever kind was chosen last.
    rpn: (u8, u8),
    nrpn: bool,
}

impl Channel {
    fn new() -> Self {
        // General MIDI's and DLS's power-on values.
        Self {
            instrument: None,
            bank: 0,
            volume: 100,
            expression: 127,
            pan: 64,
            modulation: 0,
            sustain: false,
            bend: 0,
            range: (2, 0),
            fine: 8192,
            coarse: 64,
            rpn: (127, 127),
            nrpn: false,
        }
    }

    /// What the pitch wheel and fine tuning add to every note's pitch, in
    /// 1/65,536 cents.
    fn pitch(&self) -> i64 {
        let range = i64::from(self.range.0) * 100 + i64::from(self.range.1);
        let bend = i64::from(self.bend) * range * 65536 / 8192;
        bend + (i64::from(self.fine) - 8192) * 100 * 65536 / 8192
    }

    /// What volume and expression take from every note, in 1/65,536 cB.
    fn attenuation(&self) -> i64 {
        let concave = &tables().concave;
        concave[usize::from(self.volume)] + concave[usize::from(self.expression)]
    }
}

/// How loud each side is, in Q16, from the pan controller and the
/// articulation's own pan: the specification's equal power over the sum.
fn pan(controller: u8, articulation: i32) -> [u64; 2] {
    let at =
        (i64::from(controller) * 1000 / 127 - 500 + i64::from(articulation >> 16)).clamp(-500, 500);
    let right = (at + 500) as u64;
    [1000 - right, right].map(|share| ((share << 32) / 1000).isqrt())
}

/// One note sounding.
#[derive(Clone, Debug)]
struct Voice {
    channel: usize,
    /// The note as it came, which its note off names.
    note: u8,
    /// The instrument and region it plays.
    instrument: usize,
    region: usize,
    /// Which note this is of all the synth has started: the oldest is the
    /// least.
    serial: u64,
    /// Whether its note off has yet to come, and whether it came while the
    /// sustain pedal was down.
    held: bool,
    sustained: bool,
    /// Where it is in its wave and how far a frame takes it at its own pitch,
    /// in samples in Q32.
    position: u64,
    rate: u64,
    ended: bool,
    /// Its own pitch, from its key, the sample's unity key and fine tune, in
    /// 1/65,536 cents, and its own attenuation, from its velocity and the
    /// sample's gain, in 1/65,536 centibels.
    pitch: i64,
    attenuation: i64,
    pan: [u64; 2],
    lfo: Lfo,
    volume: Generator,
    pitch_eg: Generator,
}

impl Voice {
    fn done(&self) -> bool {
        self.ended
            || self.volume.stage == Stage::Done
            || (self.volume.stage != Stage::Attack && self.volume.level == 0)
    }

    /// Into its release, from wherever it is.
    fn release(&mut self) {
        self.held = false;
        self.sustained = false;
        if self.volume.stage == Stage::Attack {
            // An attack is an amplitude, and the rest of the envelope is in
            // decibels: the same loudness, there.
            use crate::cpu::x87::maths;
            let amplitude = f64::from(self.volume.level) / f64::from(ONE);
            let decibels = 20.0 * maths::log2(amplitude) / std::f64::consts::LOG2_10;
            let level = (1.0 + decibels / 96.0).clamp(0.0, 1.0) * f64::from(ONE);
            self.volume.level = level.round() as u32;
        }
        self.volume.release();
        self.pitch_eg.release();
    }

    /// Off within 15 ms, as a key group cuts it.
    fn cut(&mut self) {
        self.release();
        self.volume.release = self.volume.release.max(step(RATE * 15 / 1000));
    }

    /// The frame it sounds now, on both sides, and on to the next.
    fn frame(&mut self, region: &Region, wave: &[i16], channel: &Channel) -> [i64; 2] {
        let art = &region.articulation;
        let (lfo, wheel) = (self.lfo.value(), i64::from(channel.modulation));
        let swing = |at_peak: i32, by_wheel: i32| {
            ((i64::from(at_peak) + i64::from(by_wheel) * wheel / 128) * lfo) >> 15
        };
        let envelope = (i64::from(art.pitch_depth) * i64::from(self.pitch_eg.level)) >> 30;
        let cents =
            (self.pitch + channel.pitch() + swing(art.lfo_pitch, art.wheel_pitch) + envelope)
                .clamp(PITCH.0, PITCH.1);
        let step = ((u128::from(self.rate) * u128::from(pow2(cents))) >> 32) as u64;

        // The gains DLS writes are negative to be quieter: here they are
        // taken away.
        let mut attenuation =
            self.attenuation + channel.attenuation() - swing(art.lfo_gain, art.wheel_gain);
        let attack = self.volume.stage == Stage::Attack;
        if !attack {
            attenuation += (FLOOR * i64::from(ONE - self.volume.level)) >> 30;
        }
        let mut gain = amplitude(attenuation);
        if attack {
            gain = (gain * u64::from(self.volume.level)) >> 30;
        }

        let whole = (self.position >> 32) as usize;
        let part = (self.position & 0xffff_ffff) as i64;
        let next = match region.sample.looped {
            Some((start, length)) if whole + 1 == (start + length) as usize => start as usize,
            _ => whole + 1,
        };
        let a = i64::from(wave[whole]);
        let b = wave.get(next).map_or(0, |s| i64::from(*s));
        let sample = a + (((b - a) * part) >> 32);
        let out = self.pan.map(|side| (sample * (gain * side) as i64) >> 32);

        self.position += step;
        let at = self.position >> 32;
        match region.sample.looped {
            Some((start, length)) if at >= u64::from(start) + u64::from(length) => {
                let back = u64::from(start) + (at - u64::from(start)) % u64::from(length);
                self.position = (back << 32) | (self.position & 0xffff_ffff);
            }
            None if at >= wave.len() as u64 => self.ended = true,
            _ => {}
        }
        self.lfo.advance();
        self.volume.advance();
        self.pitch_eg.advance();
        out
    }
}

/// A channel's place in DLS Level 1's static priority, the highest first:
/// channel 10, then 1 to 9, then 11 to 16.
fn priority(channel: usize) -> usize {
    match channel {
        DRUMS => 0,
        0..DRUMS => channel + 1,
        _ => channel,
    }
}

/// A General MIDI synthesiser over a bank.
pub struct Synth {
    bank: Bank,
    channels: [Channel; 16],
    voices: Vec<Voice>,
    /// The messages still to come, each at the frame of the mix it is heard
    /// from, in order.
    queue: VecDeque<(u64, u32)>,
    /// The last status a message carried, which a message without one runs
    /// on with.
    running: u8,
    started: u64,
}

impl Synth {
    /// A synth playing a bank, every channel on program 0, as General MIDI
    /// powers on.
    #[must_use]
    pub fn new(bank: Bank) -> Self {
        let mut synth = Self {
            bank,
            channels: [Channel::new(); 16],
            voices: Vec::with_capacity(VOICES),
            queue: VecDeque::new(),
            running: 0,
            started: 0,
        };
        for channel in 0..16 {
            synth.program(channel, 0);
        }
        synth
    }

    /// A short message, packed as `midiOutShortMsg` takes one, its status in
    /// the low byte, heard from a frame of the mix on: from the next frame
    /// rendered, if that is past it.
    pub fn message(&mut self, frame: u64, message: u32) {
        let at = self.queue.partition_point(|(f, _)| *f <= frame);
        self.queue.insert(at, (frame, message));
    }

    /// One frame of the mix, both sides, after every message heard from it.
    pub fn frame(&mut self, frame: u64) -> [i32; 2] {
        while let Some(&(at, message)) = self.queue.front() {
            if at > frame {
                break;
            }
            self.queue.pop_front();
            self.apply(message);
        }
        let mut out = [0i64; 2];
        for voice in &mut self.voices {
            let region = &self.bank.instruments[voice.instrument].regions[voice.region];
            let wave = &self.bank.waves[region.wave].samples;
            let [left, right] = voice.frame(region, wave, &self.channels[voice.channel]);
            out[0] += left;
            out[1] += right;
        }
        self.voices.retain(|v| !v.done());
        out.map(|s| s.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
    }

    fn apply(&mut self, message: u32) {
        let [mut status, mut first, mut second, _] = message.to_le_bytes();
        match status {
            // Running status: the data bytes of the last status's kind.
            0..0x80 => (status, first, second) = (self.running, status, first),
            0x80..0xf0 => self.running = status,
            // System messages mean nothing to a General MIDI voice, and a
            // system common one ends running status, as MIDI 1.0 says.
            0xf0..0xf8 => {
                self.running = 0;
                return;
            }
            _ => return,
        }
        let (channel, first, second) = (usize::from(status & 0x0f), first & 0x7f, second & 0x7f);
        match status & 0xf0 {
            0x80 => self.note_off(channel, first),
            0x90 if second == 0 => self.note_off(channel, first),
            0x90 => self.note_on(channel, first, second),
            0xb0 => self.control(channel, first, second),
            0xc0 => self.program(channel, first),
            0xe0 => {
                self.channels[channel].bend = ((i16::from(second) << 7) | i16::from(first)) - 8192
            }
            // Key and channel pressure, which no Level 1 connection takes,
            // and nothing before a first status.
            _ => {}
        }
    }

    fn program(&mut self, channel: usize, program: u8) {
        let drums = channel == DRUMS;
        let bank = self.channels[channel].bank;
        let find = |bank: u16, program: u8| {
            self.bank.instruments.iter().position(|i: &Instrument| {
                i.drums == drums && i.bank == bank && i.program == program
            })
        };
        self.channels[channel].instrument = find(bank, program)
            .or_else(|| find(0, program))
            .or_else(|| if drums { find(0, 0) } else { None });
    }

    fn note_on(&mut self, channel: usize, note: u8, velocity: u8) {
        let ch = self.channels[channel];
        let Some(index) = ch.instrument else {
            return;
        };
        let instrument = &self.bank.instruments[index];
        // Coarse tuning moves the key before a region is chosen; Level 1 has
        // none on the drums.
        let key = match instrument.drums {
            true => note,
            false => (i16::from(note) + i16::from(ch.coarse) - 64).clamp(0, 127) as u8,
        };
        let Some(r) = instrument
            .regions
            .iter()
            .position(|r| (r.keys.0..=r.keys.1).contains(&u16::from(key)))
        else {
            return;
        };
        let region = &instrument.regions[r];
        let wave = &self.bank.waves[region.wave];
        if wave.samples.is_empty() {
            return;
        }
        // A drum ends the others of its key group, and a note those on its
        // key that have had no note off, unless their own region lets them
        // overlap.
        for voice in self.voices.iter_mut().filter(|v| v.channel == channel) {
            let theirs = &self.bank.instruments[voice.instrument].regions[voice.region];
            let grouped = instrument.drums && region.group != 0 && theirs.group == region.group;
            if grouped || (voice.note == note && voice.held && !theirs.overlaps) {
                voice.cut();
            }
        }
        let art = &region.articulation;
        let sample = &region.sample;
        let voice = Voice {
            channel,
            note,
            instrument: index,
            region: r,
            serial: self.started,
            held: true,
            sustained: false,
            position: 0,
            rate: (u64::from(wave.rate) << 32) / RATE,
            ended: false,
            pitch: ((i64::from(key) - i64::from(sample.unity)) * 100 + i64::from(sample.fine_tune))
                << 16,
            attenuation: tables().concave[usize::from(velocity)] - i64::from(sample.gain),
            pan: pan(ch.pan, art.pan),
            lfo: Lfo {
                delay: scaled(art.lfo_delay, 0, 0),
                phase: 0,
                // An absolute pitch is cents above 8.176 Hz, 6900 below 440.
                step: (u128::from(pow2(i64::from(art.lfo_frequency) - (6900 << 16))) * 440
                    / u128::from(RATE))
                .min(u128::from(u32::MAX)) as u32,
            },
            volume: Generator::new(&art.volume, velocity, key),
            pitch_eg: Generator::new(&art.pitch, velocity, key),
        };
        self.started += 1;
        self.voices.retain(|v| !v.done());
        if self.voices.len() < VOICES {
            self.voices.push(voice);
        } else if let Some(stolen) = self.victim(channel) {
            self.voices[stolen] = voice;
        }
    }

    /// The voice a note on a channel takes when every voice is sounding: on
    /// the lowest channel no higher than its own, released before held, the
    /// oldest first.
    fn victim(&self, channel: usize) -> Option<usize> {
        let rank = priority(channel);
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, v)| priority(v.channel) >= rank)
            .max_by_key(|(_, v)| {
                (
                    priority(v.channel),
                    !v.held && !v.sustained,
                    std::cmp::Reverse(v.serial),
                )
            })
            .map(|(i, _)| i)
    }

    fn note_off(&mut self, channel: usize, note: u8) {
        let sustain = self.channels[channel].sustain;
        for voice in self
            .voices
            .iter_mut()
            .filter(|v| v.channel == channel && v.note == note && v.held)
        {
            if sustain {
                (voice.held, voice.sustained) = (false, true);
            } else {
                voice.release();
            }
        }
    }

    fn control(&mut self, channel: usize, controller: u8, value: u8) {
        let ch = &mut self.channels[channel];
        let was = ch.pan;
        match controller {
            // A controller's high part sets its low part to nothing, as MIDI
            // 1.0 says.
            0 => ch.bank = u16::from(value) << 7,
            32 => ch.bank = (ch.bank & !0x7f) | u16::from(value),
            1 => ch.modulation = value,
            7 => ch.volume = value,
            10 => ch.pan = value,
            11 => ch.expression = value,
            6 if !ch.nrpn => match ch.rpn {
                (0, 0) => ch.range = (value, 0),
                (0, 1) => ch.fine = u16::from(value) << 7,
                (0, 2) => ch.coarse = value,
                _ => {}
            },
            38 if !ch.nrpn => match ch.rpn {
                (0, 0) => ch.range.1 = value,
                (0, 1) => ch.fine = (ch.fine & !0x7f) | u16::from(value),
                _ => {}
            },
            101 => (ch.rpn.0, ch.nrpn) = (value, false),
            100 => (ch.rpn.1, ch.nrpn) = (value, false),
            98 | 99 => ch.nrpn = true,
            64 => {
                ch.sustain = value >= 64;
                if !ch.sustain {
                    self.pedal_up(channel);
                }
            }
            121 => {
                // All but volume and pan (DLS Level 1, read with its errata on
                // expression), and bank select and the registered parameters'
                // own values (General MIDI's RP-015), unless 127 asks for
                // every power-on value (DLS Level 1).
                let fresh = Channel::new();
                (ch.modulation, ch.expression, ch.bend) = (0, 127, 0);
                (ch.rpn, ch.nrpn) = (fresh.rpn, fresh.nrpn);
                if value == 127 {
                    (ch.volume, ch.pan, ch.bank) = (fresh.volume, fresh.pan, fresh.bank);
                    (ch.range, ch.fine, ch.coarse) = (fresh.range, fresh.fine, fresh.coarse);
                }
                if ch.sustain {
                    ch.sustain = false;
                    self.pedal_up(channel);
                }
            }
            // All sound off: the channel silent at once.
            120 => self.voices.retain(|v| v.channel != channel),
            // All notes off, and the modes that end every note as it does:
            // every key let go, which the pedal may keep.
            123..=127 => (0..128).for_each(|note| self.note_off(channel, note)),
            _ => {}
        }
        // Pan moves the notes already sounding.
        let now = self.channels[channel].pan;
        if now != was {
            for voice in self.voices.iter_mut().filter(|v| v.channel == channel) {
                let region = &self.bank.instruments[voice.instrument].regions[voice.region];
                voice.pan = pan(now, region.articulation.pan);
            }
        }
    }

    fn pedal_up(&mut self, channel: usize) {
        for voice in self
            .voices
            .iter_mut()
            .filter(|v| v.channel == channel && v.sustained)
        {
            voice.release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{tables, Synth, VOICES};
    use crate::dls::{tiny, Bank};

    fn synth() -> Synth {
        Synth::new(Bank::parse(&tiny::bytes()).expect("the tiny bank"))
    }

    fn message(status: u8, first: u8, second: u8) -> u32 {
        u32::from_le_bytes([status, first, second, 0])
    }

    /// Messages to a channel, counted from 1 as MIDI counts them: a note on
    /// and off, a controller and a program.
    fn on(channel: u8, key: u8, velocity: u8) -> u32 {
        message(0x90 | (channel - 1), key, velocity)
    }

    fn off(channel: u8, key: u8) -> u32 {
        message(0x80 | (channel - 1), key, 64)
    }

    fn cc(channel: u8, controller: u8, value: u8) -> u32 {
        message(0xb0 | (channel - 1), controller, value)
    }

    fn program(channel: u8, program: u8) -> u32 {
        message(0xc0 | (channel - 1), program, 0)
    }

    /// Every channel at full volume and panned hard left, so the left side
    /// is what a voice plays at the gain its envelope gives it.
    fn full_left(synth: &mut Synth) {
        for channel in 1..=16 {
            synth.message(0, cc(channel, 7, 127));
            synth.message(0, cc(channel, 10, 0));
        }
    }

    /// The left side of the frames rendered from one frame to another.
    fn left(synth: &mut Synth, frames: std::ops::Range<u64>) -> Vec<i32> {
        frames.map(|f| synth.frame(f)[0]).collect()
    }

    fn near(heard: i32, want: f64, within: f64) -> bool {
        (f64::from(heard) - want).abs() <= want.abs() * within + 1.0
    }

    #[test]
    fn a_note_rises_through_its_attack_decays_to_its_sustain_and_releases_to_silence() {
        // Program 0: a steady 16,384, a 1/64 s attack, 96 dB a second of
        // decay to half the envelope (-48 dB), and 96 dB in half a second of
        // release.
        let mut synth = synth();
        full_left(&mut synth);
        synth.message(0, on(1, 60, 127));
        let heard = left(&mut synth, 0..20_000);
        let attack = 22_050 / 64;
        assert_eq!(heard[0], 0, "the attack starts from nothing");
        assert!(
            near(heard[attack / 2], 8192.0, 0.01),
            "half way, half as loud"
        );
        assert!(near(heard[attack + 1], 16_384.0, 0.01), "at its peak");
        // With the engine's own maths, as everything under src/ is.
        let db = |down: f64| {
            16_384.0 * crate::cpu::x87::maths::pow2(-down / 20.0 * std::f64::consts::LOG2_10)
        };
        assert!(
            near(heard[attack + 5512], db(24.0), 0.02),
            "a quarter second down"
        );
        assert!(
            near(heard[attack + 11_025 + 100], db(48.0), 0.03),
            "held at half"
        );
        assert_eq!(heard[19_000], heard[19_999], "and steady there");
        synth.message(20_000, off(1, 60));
        let release = left(&mut synth, 20_000..26_000);
        assert!(
            near(release[2756], db(72.0), 0.05),
            "an eighth of a second on"
        );
        assert!(release[5600..].iter().all(|s| *s == 0), "and then nothing");
        assert!(synth.voices.is_empty(), "and the voice is free");
    }

    #[test]
    fn a_held_note_goes_round_its_loop_and_a_note_played_once_ends() {
        // Program 1: the ramp, looping its last four steps, from key 60; the
        // eight-bit wave at half the mix's rate below, once.
        let mut synth = synth();
        full_left(&mut synth);
        synth.message(0, program(1, 1));
        synth.message(0, on(1, 60, 127));
        let looped = left(&mut synth, 0..14);
        assert_eq!(
            looped,
            [0, 100, 200, 300, 400, 500, 600, 700, 400, 500, 600, 700, 400, 500]
        );
        synth.message(14, off(1, 60));
        synth.message(14, on(1, 48, 127));
        let once = left(&mut synth, 14..24);
        assert_eq!(
            once,
            [0, 2048, 4096, 6144, 8192, 10_240, 12_288, 6144, 0, 0]
        );
        assert!(synth.voices.is_empty(), "both are over");
    }

    #[test]
    fn channel_ten_plays_its_kit_past_general_midis_keys_and_a_key_group_cuts_its_own() {
        let mut synth = synth();
        full_left(&mut synth);
        // Key 82, beyond General MIDI's 35 to 81: the kit's eight-bit wave
        // at its own pitch, once.
        synth.message(0, on(10, 82, 127));
        assert_eq!(left(&mut synth, 0..4), [0, 2048, 4096, 6144]);
        assert!(synth.voices.iter().all(|v| v.instrument == 2));
        // 42 and 46 share a group: the second cuts the first off within
        // 15 ms, and only it sounds on.
        synth.message(100, on(10, 42, 127));
        synth.message(200, on(10, 46, 127));
        let heard = left(&mut synth, 4..1000);
        assert_eq!(heard[150], 16_384, "one steady drum");
        assert!(heard[197] > 30_000, "both, as the second starts");
        assert!(
            heard[196 + 331..].iter().all(|s| *s == 16_384),
            "then the second alone"
        );
        assert_eq!(synth.voices.len(), 1);
    }

    #[test]
    fn a_note_past_thirty_two_voices_steals_from_the_lowest_channel_never_from_a_higher_one() {
        // Windows 98 SE's count, which the module doc declares: a test of its
        // own, so the count cannot move unseen.
        assert_eq!(VOICES, 32);
        let mut synth = synth();
        let notes = |synth: &Synth, channel: usize| -> Vec<u8> {
            let on_it = synth.voices.iter().filter(|v| v.channel == channel);
            on_it.map(|v| v.note).collect()
        };
        // Every voice held on channel 2, then one released.
        for key in 0..VOICES as u8 {
            synth.message(0, on(2, key, 100));
        }
        synth.message(1, off(2, 5));
        left(&mut synth, 0..2);
        assert_eq!(synth.voices.len(), VOICES);
        // The drums, the highest channel, take the released voice first and
        // then the oldest held.
        synth.message(2, on(10, 82, 100));
        synth.message(2, on(10, 42, 100));
        left(&mut synth, 2..3);
        let held = notes(&synth, 1);
        assert!(!held.contains(&5) && !held.contains(&0) && held.contains(&1));
        // Each in the voice it took: the oldest held note's was the first.
        assert_eq!(notes(&synth, 9), [42, 82]);
        // Channel 1 is above channel 2 and takes from it; channel 11 is below
        // both, and channel 16 below everything: neither is played.
        synth.message(3, on(1, 64, 100));
        synth.message(3, on(11, 64, 100));
        synth.message(3, on(16, 64, 100));
        left(&mut synth, 3..4);
        assert_eq!(notes(&synth, 0), [64]);
        assert!(notes(&synth, 10).is_empty() && notes(&synth, 15).is_empty());
        assert_eq!(synth.voices.len(), VOICES);
    }

    #[test]
    fn a_voice_let_go_with_no_release_frees_its_place_for_a_note_in_the_same_frame() {
        // Every voice on channel 2's ramps, which loop until let go and have
        // no release; channel 16, below them all, steals from none.
        let mut synth = synth();
        synth.message(0, program(2, 1));
        for key in 60..60 + VOICES as u8 {
            synth.message(0, on(2, key, 100));
        }
        left(&mut synth, 0..1);
        synth.message(1, off(2, 60));
        synth.message(1, on(16, 64, 100));
        left(&mut synth, 1..2);
        assert!(synth.voices.iter().any(|v| v.channel == 15));
    }

    #[test]
    fn a_key_struck_again_cuts_its_note_with_no_note_off_and_one_the_pedal_keeps_rings_on() {
        // Program 0 releases over half a second; 15 ms is 331 frames.
        let mut synth = synth();
        synth.message(0, on(1, 60, 100));
        synth.message(1000, on(1, 60, 100));
        left(&mut synth, 0..1400);
        assert_eq!(synth.voices.len(), 1, "the first cut, the second on");
        // Under the pedal, the first note has had its note off.
        synth.message(1400, cc(2, 64, 127));
        synth.message(1400, on(2, 60, 100));
        synth.message(1401, off(2, 60));
        synth.message(1402, on(2, 60, 100));
        left(&mut synth, 1400..2000);
        let pedalled = synth.voices.iter().filter(|v| v.channel == 1);
        let kept: Vec<_> = pedalled.map(|v| (v.held, v.sustained)).collect();
        assert_eq!(kept, [(false, true), (true, false)]);
    }

    #[test]
    fn a_key_struck_again_asks_the_old_notes_own_region_whether_it_may_overlap() {
        // Program 0 lets its notes overlap and program 1 does not, over one
        // steady loop.
        let looped = [tiny::wsmp(60, 0, 0, Some((0, 64)))];
        let steady = tiny::wave(22_050, 16, &tiny::samples(&[1000; 64]), &looped);
        let program_of = |options| {
            let region = tiny::region((0, 127), options, 0, 0, &[]);
            tiny::instrument(0, u32::from(options == 0), &[region], &[])
        };
        let instruments = [program_of(1), program_of(0)];
        let bank = Bank::parse(&tiny::bank(&instruments, &[steady], &[0])).expect("a bank");
        let mut synth = Synth::new(bank);
        // Channel 1 holds an overlapping note and strikes it again on the
        // other program; channel 2 the other way round.
        for (channel, first, then) in [(1, 0, 1), (2, 1, 0)] {
            synth.message(0, program(channel, first));
            synth.message(0, on(channel, 60, 100));
            synth.message(1, program(channel, then));
            synth.message(1, on(channel, 60, 100));
        }
        left(&mut synth, 0..1000);
        let on_it = |c: usize| synth.voices.iter().filter(|v| v.channel == c).count();
        assert_eq!((on_it(0), on_it(1)), (2, 1));
    }

    #[test]
    fn a_system_message_ends_running_status() {
        let mut synth = synth();
        synth.message(0, on(1, 60, 100));
        // Song position, a system common message, and then data bytes with
        // no status of their own.
        synth.message(0, message(0xf2, 0, 0));
        synth.message(0, message(62, 100, 0));
        left(&mut synth, 0..1);
        assert!(synth.voices.iter().all(|v| v.note == 60));
    }

    #[test]
    fn all_sound_off_silences_a_channel_at_once_and_the_mode_messages_end_its_notes() {
        let mut synth = synth();
        for channel in 1..=7 {
            synth.message(0, on(channel, 60, 100));
        }
        synth.message(1, cc(1, 120, 0));
        for (channel, controller) in [(2, 123), (3, 124), (4, 125), (5, 126), (6, 127)] {
            synth.message(1, cc(channel, controller, 0));
        }
        left(&mut synth, 0..2);
        let held = |c: usize| -> Vec<bool> {
            let on_it = synth.voices.iter().filter(|v| v.channel == c);
            on_it.map(|v| v.held).collect()
        };
        assert!(held(0).is_empty(), "channel 1 silent");
        for channel in 1..6 {
            assert_eq!(held(channel), [false], "{channel} let go");
        }
        assert_eq!(held(6), [true], "channel 7 untouched");
    }

    #[test]
    fn data_entry_goes_to_the_registered_parameter_chosen_last_and_its_high_part_clears_its_low() {
        let mut synth = synth();
        let send = |synth: &mut Synth, messages: &[u32]| {
            for m in messages {
                synth.message(0, *m);
            }
            synth.frame(0);
            (synth.channels[0].range, synth.channels[0].bank)
        };
        let rpn0 = [cc(1, 101, 0), cc(1, 100, 0)];
        assert_eq!(
            send(&mut synth, &[rpn0[0], rpn0[1], cc(1, 6, 2), cc(1, 38, 50)]).0,
            (2, 50)
        );
        assert_eq!(send(&mut synth, &[cc(1, 6, 12)]).0, (12, 0));
        // A non-registered parameter takes data entry, and keeps the
        // registered number chosen before it.
        assert_eq!(
            send(&mut synth, &[cc(1, 99, 0), cc(1, 98, 0), cc(1, 6, 7)]).0,
            (12, 0)
        );
        assert_eq!(send(&mut synth, &[cc(1, 101, 0), cc(1, 6, 7)]).0, (7, 0));
        // Bank select's high part clears its low part too.
        assert_eq!(send(&mut synth, &[cc(1, 32, 2), cc(1, 0, 3)]).1, 3 << 7);
        // Reset all controllers keeps bank select, unless it is 127.
        assert_eq!(send(&mut synth, &[cc(1, 0, 1), cc(1, 121, 0)]).1, 1 << 7);
        assert_eq!(send(&mut synth, &[cc(1, 121, 127)]), ((2, 0), 0));
    }

    #[test]
    fn volume_follows_the_concave_curve_and_the_pitch_wheel_its_range() {
        let mut synth = synth();
        full_left(&mut synth);
        // Volume 64 is 40·log10(127/64) dB down: a quarter, near enough.
        synth.message(0, cc(10, 7, 64));
        synth.message(0, on(10, 42, 127));
        let quarter = 16_384.0 * (64.0 / 127.0) * (64.0 / 127.0);
        assert!(near(left(&mut synth, 0..10)[9], quarter, 0.005));
        // A range of twelve semitones by RPN 0, and the wheel at the bottom:
        // the ramp an octave down, half a step a frame.
        let range = [cc(1, 101, 0), cc(1, 100, 0), cc(1, 6, 12), cc(1, 38, 0)];
        for m in [program(1, 1)].into_iter().chain(range) {
            synth.message(10, m);
        }
        synth.message(10, message(0xe0, 0, 0));
        synth.message(10, on(1, 60, 127));
        synth.message(10, cc(10, 7, 0));
        assert_eq!(left(&mut synth, 10..17), [0, 50, 100, 150, 200, 250, 300]);
    }

    #[test]
    fn the_same_messages_at_the_same_frames_give_the_same_samples() {
        let render = || {
            let mut synth = synth();
            let messages = [
                (0, on(1, 60, 90)),
                (0, on(10, 42, 120)),
                (300, cc(1, 1, 100)),
                (500, message(0xe0, 0x10, 0x50)),
                (700, cc(1, 10, 20)),
                (900, on(10, 46, 70)),
                (1200, off(1, 60)),
                (1500, program(2, 1)),
                (1500, on(2, 70, 127)),
                (4000, cc(2, 123, 0)),
            ];
            for (at, m) in messages {
                synth.message(at, m);
            }
            let frames: Vec<u8> = (0..30_000u64)
                .flat_map(|f| synth.frame(f))
                .flat_map(i32::to_le_bytes)
                .collect();
            crate::hash::sha256_hex(&frames)
        };
        assert_eq!(render(), render());
        // The same on every host: the tables the transcendentals come from,
        // and what is rendered from them.
        let tables = tables();
        let mut bytes: Vec<u8> = tables.octave.iter().flat_map(|v| v.to_le_bytes()).collect();
        bytes.extend(tables.sine.iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend(tables.concave.iter().flat_map(|v| v.to_le_bytes()));
        assert_eq!(
            crate::hash::sha256_hex(&bytes),
            "c42f41d9d20ee954084baa5402f2a131334ef0209957fc43c516e537a3c9fe17"
        );
        assert_eq!(
            render(),
            "5185c0f510fd1d0691088e710176fa784cc7ee8331717a22ca7417e8713ec813"
        );
    }
}
