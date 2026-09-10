//! DirectSound.
//!
//! Mixing is real and runs whether or not a device is listening, because a
//! golden hashes what was mixed rather than what was heard. What reaches a
//! host device is `--audio`'s business and the host's.
//!
//! A buffer the title plays becomes a voice in the machine's mixer at the
//! declared time it was played: its samples decoded from the buffer's own
//! format, taken to the mix's rate, and weighed by its volume, its pan, and
//! where it is from the listener. Where it has got to, and whether it is still
//! playing, follow from the declared clock (ADR-0003).

use super::{ComState, Convention, Ctx, Shim, State};
use crate::com::{wall, Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::sound::{Format, RATE};
use std::collections::BTreeMap;

pub const SHIMS: &[Shim] = &[
    // DirectSound exports its creator by ordinal as well as by name, and this
    // title imports the ordinal.
    Shim {
        dll: "DSOUND",
        name: "#1",
        args: 3,
        convention: Convention::Stdcall,
        run: create,
    },
    Shim {
        dll: "DSOUND",
        name: "DirectSoundCreate",
        args: 3,
        convention: Convention::Stdcall,
        run: create,
    },
];

/// Every buffer's playing state, and the listener.
#[derive(Default)]
pub struct Sounds {
    buffers: BTreeMap<u32, Buffer>,
    listener: Listener,
}

/// What a buffer plays and how, beyond its bytes.
#[derive(Clone, Debug)]
struct Buffer {
    /// The format its bytes are in; a primary buffer's is what the title set.
    format: Option<Format>,
    /// Hundredths of a decibel below full, from 0 to -10000.
    volume: i32,
    /// Hundredths of a decibel taken off one side: negative is the right.
    pan: i32,
    /// The rate it plays at, or `None` for its format's own.
    frequency: Option<u32>,
    looping: bool,
    /// The declared millisecond it started playing at, while it plays.
    started: Option<u64>,
    /// Where in its bytes it starts, or stopped.
    offset: u32,
    /// Where it is, when the title asked for it in 3D.
    place: Option<Place>,
}

impl Buffer {
    fn new(format: Option<Format>, three_d: bool) -> Self {
        Self {
            format,
            volume: 0,
            pan: 0,
            frequency: None,
            looping: false,
            started: None,
            offset: 0,
            place: three_d.then(Place::default),
        }
    }
}

/// A 3D buffer's position, and how its loudness falls off.
#[derive(Clone, Copy, Debug)]
struct Place {
    position: [f32; 3],
    min_distance: f32,
    max_distance: f32,
    /// `DS3DMODE_NORMAL`, `_HEADRELATIVE` or `_DISABLE`.
    mode: u32,
}

impl Default for Place {
    /// `DS3D_DEFAULTMINDISTANCE` and `_MAXDISTANCE`, in normal mode, at the
    /// origin.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            min_distance: 1.0,
            max_distance: 1_000_000_000.0,
            mode: 0,
        }
    }
}

/// Where the one who hears is, and which way they face.
#[derive(Clone, Copy, Debug)]
struct Listener {
    position: [f32; 3],
    front: [f32; 3],
    top: [f32; 3],
    distance_factor: f32,
    rolloff_factor: f32,
    doppler_factor: f32,
}

impl Default for Listener {
    /// DirectSound's own: at the origin, looking down z with y up, and every
    /// factor one.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            front: [0.0, 0.0, 1.0],
            top: [0.0, 1.0, 0.0],
            distance_factor: 1.0,
            rolloff_factor: 1.0,
            doppler_factor: 1.0,
        }
    }
}

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DirectSoundCreate(guid, out, outer).
    let object = State::make_com(ctx, &DIRECT_SOUND, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("dsound created at {object:#010x}"));
    Ok(S_OK)
}

/// `IDirectSound`, in slot order.
pub static DIRECT_SOUND: Interface = Interface {
    name: "IDirectSound",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "CreateSoundBuffer",
            args: 3,
            run: create_buffer,
        },
        Method {
            name: "GetCaps",
            args: 1,
            run: caps,
        },
        Method {
            name: "DuplicateSoundBuffer",
            args: 2,
            run: duplicate,
        },
        Method {
            name: "SetCooperativeLevel",
            args: 2,
            run: ok,
        },
        Method {
            name: "Compact",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetSpeakerConfig",
            args: 1,
            run: speaker_config,
        },
        Method {
            name: "SetSpeakerConfig",
            args: 1,
            run: ok,
        },
        Method {
            name: "Initialize",
            args: 1,
            run: ok,
        },
    ],
};

/// `IDirectSoundBuffer`, in slot order.
pub static DIRECT_SOUND_BUFFER: Interface = Interface {
    name: "IDirectSoundBuffer",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "GetCaps",
            args: 1,
            run: caps,
        },
        Method {
            name: "GetCurrentPosition",
            args: 2,
            run: position,
        },
        wall("GetFormat", 3),
        Method {
            name: "GetVolume",
            args: 1,
            run: get_volume,
        },
        Method {
            name: "GetPan",
            args: 1,
            run: get_pan,
        },
        Method {
            name: "GetFrequency",
            args: 1,
            run: get_frequency,
        },
        Method {
            name: "GetStatus",
            args: 1,
            run: status,
        },
        Method {
            name: "Initialize",
            args: 2,
            run: ok,
        },
        Method {
            name: "Lock",
            args: 7,
            run: lock,
        },
        Method {
            name: "Play",
            args: 3,
            run: play,
        },
        Method {
            name: "SetCurrentPosition",
            args: 1,
            run: set_position,
        },
        Method {
            name: "SetFormat",
            args: 1,
            run: set_format,
        },
        Method {
            name: "SetVolume",
            args: 1,
            run: set_volume,
        },
        Method {
            name: "SetPan",
            args: 1,
            run: set_pan,
        },
        Method {
            name: "SetFrequency",
            args: 1,
            run: set_frequency,
        },
        Method {
            name: "Stop",
            args: 0,
            run: stop_buffer,
        },
        Method {
            name: "Unlock",
            args: 4,
            run: ok,
        },
        Method {
            name: "Restore",
            args: 0,
            run: ok,
        },
    ],
};

/// `IDirectSound3DListener`: where the one who hears is, on the primary buffer.
pub static DIRECT_SOUND_3D_LISTENER: Interface = Interface {
    name: "IDirectSound3DListener",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetAllParameters", 1),
        Method {
            name: "GetDistanceFactor",
            args: 1,
            run: |ctx, _, args| {
                let value = ctx.state.sound.listener.distance_factor;
                write_float(ctx, args[0], value)
            },
        },
        Method {
            name: "GetDopplerFactor",
            args: 1,
            run: |ctx, _, args| {
                let value = ctx.state.sound.listener.doppler_factor;
                write_float(ctx, args[0], value)
            },
        },
        Method {
            name: "GetOrientation",
            args: 2,
            run: |ctx, _, args| {
                let Listener { front, top, .. } = ctx.state.sound.listener;
                write_vector(ctx, args[0], front)?;
                write_vector(ctx, args[1], top)
            },
        },
        Method {
            name: "GetPosition",
            args: 1,
            run: |ctx, _, args| {
                let value = ctx.state.sound.listener.position;
                write_vector(ctx, args[0], value)
            },
        },
        Method {
            name: "GetRolloffFactor",
            args: 1,
            run: |ctx, _, args| {
                let value = ctx.state.sound.listener.rolloff_factor;
                write_float(ctx, args[0], value)
            },
        },
        wall("GetVelocity", 1),
        wall("SetAllParameters", 2),
        Method {
            name: "SetDistanceFactor",
            args: 2,
            run: |ctx, _, args| {
                ctx.state.sound.listener.distance_factor = float(args[0]);
                Ok(S_OK)
            },
        },
        Method {
            name: "SetDopplerFactor",
            args: 2,
            run: |ctx, _, args| {
                ctx.state.sound.listener.doppler_factor = float(args[0]);
                Ok(S_OK)
            },
        },
        Method {
            name: "SetOrientation",
            args: 7,
            run: |ctx, _, args| {
                let listener = &mut ctx.state.sound.listener;
                listener.front = [float(args[0]), float(args[1]), float(args[2])];
                listener.top = [float(args[3]), float(args[4]), float(args[5])];
                Ok(S_OK)
            },
        },
        Method {
            name: "SetPosition",
            args: 4,
            run: |ctx, _, args| {
                ctx.state.sound.listener.position =
                    [float(args[0]), float(args[1]), float(args[2])];
                Ok(S_OK)
            },
        },
        Method {
            name: "SetRolloffFactor",
            args: 2,
            run: |ctx, _, args| {
                ctx.state.sound.listener.rolloff_factor = float(args[0]);
                Ok(S_OK)
            },
        },
        // How fast the listener moves is for the Doppler shift, which the mix
        // does not make.
        Method {
            name: "SetVelocity",
            args: 4,
            run: ok,
        },
        // Every setting here takes effect when it is made, deferred or not.
        Method {
            name: "CommitDeferredSettings",
            args: 0,
            run: ok,
        },
    ],
};

/// `IDirectSound3DBuffer`: where a sound is, on its own buffer.
pub static DIRECT_SOUND_3D_BUFFER: Interface = Interface {
    name: "IDirectSound3DBuffer",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetAllParameters", 1),
        wall("GetConeAngles", 2),
        wall("GetConeOrientation", 1),
        wall("GetConeOutsideVolume", 1),
        Method {
            name: "GetMaxDistance",
            args: 1,
            run: |ctx, this, args| {
                let place = place(ctx, this)?;
                write_float(ctx, args[0], place.max_distance)
            },
        },
        Method {
            name: "GetMinDistance",
            args: 1,
            run: |ctx, this, args| {
                let place = place(ctx, this)?;
                write_float(ctx, args[0], place.min_distance)
            },
        },
        Method {
            name: "GetMode",
            args: 1,
            run: |ctx, this, args| {
                let place = place(ctx, this)?;
                ctx.mem
                    .write_u32(args[0], place.mode)
                    .map_err(Stop::Fault)?;
                Ok(S_OK)
            },
        },
        Method {
            name: "GetPosition",
            args: 1,
            run: |ctx, this, args| {
                let place = place(ctx, this)?;
                write_vector(ctx, args[0], place.position)
            },
        },
        wall("GetVelocity", 1),
        wall("SetAllParameters", 2),
        wall("SetConeAngles", 3),
        wall("SetConeOrientation", 4),
        wall("SetConeOutsideVolume", 2),
        Method {
            name: "SetMaxDistance",
            args: 2,
            run: |ctx, this, args| with_place(ctx, this, |p| p.max_distance = float(args[0])),
        },
        Method {
            name: "SetMinDistance",
            args: 2,
            run: |ctx, this, args| with_place(ctx, this, |p| p.min_distance = float(args[0])),
        },
        Method {
            name: "SetMode",
            args: 2,
            run: |ctx, this, args| {
                if args[0] > 2 {
                    return Err(Stop::NotWritten {
                        what: format!("IDirectSound3DBuffer::SetMode: mode {}", args[0]),
                        caller: 0,
                    });
                }
                with_place(ctx, this, |p| p.mode = args[0])
            },
        },
        Method {
            name: "SetPosition",
            args: 4,
            run: |ctx, this, args| {
                with_place(ctx, this, |p| {
                    p.position = [float(args[0]), float(args[1]), float(args[2])];
                })
            },
        },
        // For the Doppler shift, which the mix does not make.
        Method {
            name: "SetVelocity",
            args: 4,
            run: ok,
        },
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &DIRECT_SOUND,
    &DIRECT_SOUND_BUFFER,
    &DIRECT_SOUND_3D_LISTENER,
    &DIRECT_SOUND_3D_BUFFER,
];

fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

/// A `D3DVALUE` passed on the stack.
fn float(word: u32) -> f32 {
    f32::from_bits(word)
}

fn write_float(ctx: &mut Ctx<'_>, at: u32, value: f32) -> Result<u32, Stop> {
    ctx.mem
        .write_u32(at, value.to_bits())
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn write_vector(ctx: &mut Ctx<'_>, at: u32, v: [f32; 3]) -> Result<u32, Stop> {
    for (i, c) in v.iter().enumerate() {
        ctx.mem
            .write_u32(at + 4 * i as u32, c.to_bits())
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// `DSERR_INVALIDCALL`: a 3D face on a buffer made without 3D.
const DSERR_INVALIDCALL: u32 = 0x8878_0032;

fn buffer<'a>(ctx: &'a mut Ctx<'_>, this: u32) -> Option<&'a mut Buffer> {
    let identity = ctx.state.identity(this);
    ctx.state.sound.buffers.get_mut(&identity)
}

fn place(ctx: &mut Ctx<'_>, this: u32) -> Result<Place, Stop> {
    buffer(ctx, this)
        .and_then(|b| b.place)
        .ok_or_else(|| Stop::NotWritten {
            what: "IDirectSound3DBuffer: a buffer made without 3D".to_owned(),
            caller: 0,
        })
}

fn with_place(ctx: &mut Ctx<'_>, this: u32, change: impl FnOnce(&mut Place)) -> Result<u32, Stop> {
    match buffer(ctx, this).and_then(|b| b.place.as_mut()) {
        Some(place) => {
            change(place);
            Ok(S_OK)
        }
        None => Ok(DSERR_INVALIDCALL),
    }
}

fn caps(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's audio hardware: enough voices for anything on the
    // roster, and no hardware mixing, because the mixer is written here.
    let at = args[0];
    if at != 0 {
        let size = ctx.mem.read_u32(at).map_err(Stop::Fault)?.max(4);
        for offset in (4..size.min(0x100)).step_by(4) {
            ctx.mem.write_u32(at + offset, 0).map_err(Stop::Fault)?;
        }
        // DSCAPS_PRIMARY16BIT | DSCAPS_PRIMARYSTEREO.
        ctx.mem
            .write_u32(at + 4, 0x0000_000c)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn speaker_config(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DSSPEAKER_STEREO.
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], 1).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// A `WAVEFORMATEX` in guest memory: its eighteen bytes and what follows.
fn read_format(ctx: &Ctx<'_>, at: u32) -> Result<Format, Stop> {
    let extra = ctx.mem.slice(at + 16, 2).map_err(Stop::Fault)?;
    let len = 18 + usize::from(u16::from_le_bytes([extra[0], extra[1]]));
    let bytes = ctx.mem.slice(at, len).map_err(Stop::Fault)?;
    Format::parse(bytes).map_err(|e| Stop::NotWritten {
        what: format!("IDirectSound: {e}"),
        caller: 0,
    })
}

fn create_buffer(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateSoundBuffer(descriptor, out, outer). A DSBUFFERDESC: size, flags,
    // how many bytes the guest wants to write into, and its format.
    const DSBCAPS_PRIMARYBUFFER: u32 = 0x1;
    const DSBCAPS_CTRL3D: u32 = 0x10;
    let at = args[0];
    let flags = ctx.mem.read_u32(at + 4).map_err(Stop::Fault)?;
    let bytes = ctx.mem.read_u32(at + 8).map_err(Stop::Fault)?;
    let format_at = ctx.mem.read_u32(at + 16).map_err(Stop::Fault)?;
    let format = if flags & DSBCAPS_PRIMARYBUFFER == 0 && format_at != 0 {
        Some(read_format(ctx, format_at)?)
    } else {
        None
    };
    let store = ctx.state.heap.alloc(bytes.max(4));
    let object = State::make_com(ctx, &DIRECT_SOUND_BUFFER, ComState::Sound { store, bytes })
        .map_err(Stop::Fault)?;
    ctx.state
        .sound
        .buffers
        .insert(object, Buffer::new(format, flags & DSBCAPS_CTRL3D != 0));
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn duplicate(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DuplicateSoundBuffer(original, out): the same bytes, played on their own.
    let original = args[0];
    let Some(ComState::Sound { store, bytes }) = ctx.state.com(original).map(|o| o.state.clone())
    else {
        return Ok(DSERR_INVALIDCALL);
    };
    let Some(template) = buffer(ctx, original).cloned() else {
        return Ok(DSERR_INVALIDCALL);
    };
    let object = State::make_com(ctx, &DIRECT_SOUND_BUFFER, ComState::Sound { store, bytes })
        .map_err(Stop::Fault)?;
    let copy = Buffer {
        started: None,
        offset: 0,
        looping: false,
        ..template
    };
    ctx.state.sound.buffers.insert(object, copy);
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// The rate a buffer plays at, and how many bytes one of its frames takes.
fn pace(buffer: &Buffer) -> Option<(u32, u32)> {
    let format = buffer.format.as_ref()?;
    let rate = buffer.frequency.unwrap_or(format.rate).max(1);
    Some((rate, u32::from(format.block_align.max(1))))
}

/// Where a buffer is in its bytes at a declared millisecond, and whether it
/// has played to its end.
fn progress(buffer: &Buffer, bytes: u32, millis: u64) -> (u32, bool) {
    let (Some(started), Some((rate, align))) = (buffer.started, pace(buffer)) else {
        return (buffer.offset, buffer.started.is_none());
    };
    let played = millis.saturating_sub(started) * u64::from(rate) / 1000 * u64::from(align);
    let at = u64::from(buffer.offset) + played;
    let len = u64::from(bytes.max(1));
    if buffer.looping {
        ((at % len) as u32, false)
    } else if at >= len {
        (bytes, true)
    } else {
        (at as u32, false)
    }
}

fn position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetCurrentPosition(play, write). A buffer's cursors derive from the
    // declared clock, never from a device: a title that waits for one to move
    // must see it move by the same amount on every run (ADR-0003). The
    // primary buffer, which has no bytes of its own, runs at the mix's rate.
    let millis = ctx.clock_millis;
    let bytes = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Sound { bytes, .. }) => *bytes,
        _ => 0,
    };
    let at = match buffer(ctx, this) {
        Some(b) if b.format.is_some() => progress(b, bytes, millis).0,
        _ => (millis * u64::from(RATE) * 4 / 1000) as u32,
    };
    for out in [args[0], args[1]] {
        if out != 0 {
            ctx.mem.write_u32(out, at).map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn status(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetStatus(out): DSBSTATUS_PLAYING, and DSBSTATUS_LOOPING with it.
    let millis = ctx.clock_millis;
    let bytes = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Sound { bytes, .. }) => *bytes,
        _ => 0,
    };
    let word = match buffer(ctx, this) {
        Some(b) if b.started.is_some() && !progress(b, bytes, millis).1 => {
            1 | if b.looping { 4 } else { 0 }
        }
        _ => 0,
    };
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], word).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn lock(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Lock(offset, bytes, out1, len1, out2, len2, flags). The guest is handed
    // real guest memory to write its samples into, which is what makes the
    // mixer able to read them back.
    // ponytail: what is written into a buffer already playing is not heard
    // until it is played again; a title that streams into a looping buffer
    // needs the mixer to read the buffer as it goes.
    const DSBLOCK_FROMWRITECURSOR: u32 = 0x1;
    const DSBLOCK_ENTIREBUFFER: u32 = 0x2;
    let Some(ComState::Sound { store, bytes }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };
    if args[6] & DSBLOCK_FROMWRITECURSOR != 0 {
        return Err(Stop::NotWritten {
            what: "IDirectSoundBuffer::Lock: from the write cursor".to_owned(),
            caller: 0,
        });
    }
    let (offset, wanted) = if args[6] & DSBLOCK_ENTIREBUFFER != 0 {
        (0, bytes)
    } else {
        (args[0], args[1])
    };
    let room = bytes.saturating_sub(offset);
    let give = wanted.min(room);
    ctx.mem
        .write_u32(args[2], store + offset)
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[3], give).map_err(Stop::Fault)?;
    // The wrap-around half is empty: this machine hands out one run.
    if args[4] != 0 {
        ctx.mem.write_u32(args[4], 0).map_err(Stop::Fault)?;
    }
    if args[5] != 0 {
        ctx.mem.write_u32(args[5], 0).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// Decibels, in hundredths, as a gain.
fn gain(hundredths: i32) -> f64 {
    if hundredths <= -10_000 {
        return 0.0;
    }
    crate::cpu::x87::maths::pow2(f64::from(hundredths) / 2000.0 * std::f64::consts::LOG2_10)
}

/// How loud a buffer is on each side: its volume and pan, and for a 3D
/// buffer how far it is and on which side of the listener.
fn weights(buffer: &Buffer, listener: &Listener) -> (f64, f64) {
    let volume = gain(buffer.volume);
    let (mut left, mut right) = (
        volume * gain(-buffer.pan.max(0)),
        volume * gain(buffer.pan.min(0)),
    );
    let Some(place) = buffer.place.filter(|p| p.mode != 2) else {
        return (left, right);
    };
    // Where the sound is from the listener, in the listener's own axes.
    let relative = if place.mode == 1 {
        place.position
    } else {
        std::array::from_fn(|i| place.position[i] - listener.position[i])
    };
    let dot = |a: [f32; 3], b: [f32; 3]| f64::from(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]);
    let distance = dot(relative, relative).sqrt() * f64::from(listener.distance_factor);
    let min = f64::from(place.min_distance.max(f32::MIN_POSITIVE));
    let far = distance.clamp(min, f64::from(place.max_distance).max(min));
    let falloff = 1.0 / (1.0 + f64::from(listener.rolloff_factor) * (far / min - 1.0));
    let across = if place.mode == 1 {
        [1.0, 0.0, 0.0]
    } else {
        super::scene::cross(listener.top, listener.front)
    };
    let side = if distance > 0.0 {
        (dot(
            relative,
            super::scene::normalise(across).unwrap_or([1.0, 0.0, 0.0]),
        ) / dot(relative, relative).sqrt())
        .clamp(-1.0, 1.0)
    } else {
        0.0
    };
    left *= falloff * (1.0 - side).min(1.0);
    right *= falloff * (1.0 + side).min(1.0);
    (left, right)
}

fn play(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Play(reserved, priority, flags): from where the buffer is, looping when
    // DSBPLAY_LOOPING asks.
    // ponytail: volume, pan and position are taken when a buffer starts; a
    // title that moves a sound while it plays is heard where it started.
    const DSBPLAY_LOOPING: u32 = 0x1;
    let identity = ctx.state.identity(this);
    let millis = ctx.clock_millis;
    let Some(ComState::Sound { store, bytes }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(DSERR_INVALIDCALL);
    };
    let listener = ctx.state.sound.listener;
    let Some(sound) = ctx.state.sound.buffers.get_mut(&identity) else {
        return Ok(DSERR_INVALIDCALL);
    };
    // The primary buffer plays whatever is mixed; it has nothing of its own.
    let Some(format) = sound.format.clone() else {
        return Ok(S_OK);
    };
    if sound.started.is_some() {
        let (at, _) = progress(sound, bytes, millis);
        sound.offset = at;
    }
    sound.looping = args[2] & DSBPLAY_LOOPING != 0;
    sound.started = Some(millis);
    let (rate, align) = pace(sound).unwrap_or((format.rate, 1));
    let (left, right) = weights(sound, &listener);
    let offset = sound.offset.min(bytes) / align * align;
    let looping = sound.looping;
    let data = ctx
        .mem
        .slice(store + offset, (bytes - offset) as usize)
        .map_err(Stop::Fault)?;
    let decoded = Format {
        rate: RATE,
        ..format
    }
    .decode(data)
    .map_err(|e| Stop::NotWritten {
        what: format!("IDirectSoundBuffer::Play: {e}"),
        caller: 0,
    })?;
    let samples = resample(&decoded, rate, left, right);
    ctx.state
        .mixer
        .play_from(identity, samples, millis * 1_000_000, looping);
    Ok(S_OK)
}

/// Stereo samples at one rate taken to the mix's, nearest sample by nearest
/// sample, and weighed on each side.
fn resample(stereo: &[i16], rate: u32, left: f64, right: f64) -> Vec<i16> {
    let frames = stereo.len() / 2;
    let out_frames = (frames as u64 * u64::from(RATE) / u64::from(rate.max(1))) as usize;
    let weigh = |s: i16, w: f64| (f64::from(s) * w).round().clamp(-32768.0, 32767.0) as i16;
    (0..out_frames)
        .flat_map(|n| {
            let from =
                ((n as u64 * u64::from(rate) / u64::from(RATE)) as usize).min(frames - 1) * 2;
            [weigh(stereo[from], left), weigh(stereo[from + 1], right)]
        })
        .collect()
}

fn stop_buffer(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let millis = ctx.clock_millis;
    let bytes = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Sound { bytes, .. }) => *bytes,
        _ => 0,
    };
    if let Some(sound) = ctx.state.sound.buffers.get_mut(&identity) {
        if sound.started.is_some() {
            sound.offset = progress(sound, bytes, millis).0 % bytes.max(1);
            sound.started = None;
        }
    }
    ctx.state.mixer.stop(identity);
    Ok(S_OK)
}

fn set_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetCurrentPosition(offset). A buffer that is playing goes on from there.
    let Some(sound) = buffer(ctx, this) else {
        return Ok(DSERR_INVALIDCALL);
    };
    let was_playing = sound.started.take().is_some();
    sound.offset = args[0];
    let looping = sound.looping;
    if was_playing {
        return play(ctx, this, &[0, 0, u32::from(looping)]);
    }
    Ok(S_OK)
}

fn set_format(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetFormat(format): the primary buffer's, which is what the mix is heard
    // at. The mix is the machine's own rate whatever it is set to.
    let format = read_format(ctx, args[0])?;
    ctx.log.line(&format!(
        "dsound primary format {} Hz, {} channels, {} bits",
        format.rate, format.channels, format.bits
    ));
    Ok(S_OK)
}

fn get_volume(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let volume = buffer(ctx, this).map_or(0, |b| b.volume);
    ctx.mem
        .write_u32(args[0], volume as u32)
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_volume(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if let Some(b) = buffer(ctx, this) {
        b.volume = (args[0] as i32).clamp(-10_000, 0);
    }
    Ok(S_OK)
}

fn get_pan(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let pan = buffer(ctx, this).map_or(0, |b| b.pan);
    ctx.mem
        .write_u32(args[0], pan as u32)
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_pan(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if let Some(b) = buffer(ctx, this) {
        b.pan = (args[0] as i32).clamp(-10_000, 10_000);
    }
    Ok(S_OK)
}

fn get_frequency(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let rate = buffer(ctx, this)
        .and_then(|b| pace(b).map(|(rate, _)| rate))
        .unwrap_or(RATE);
    ctx.mem.write_u32(args[0], rate).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_frequency(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DSBFREQUENCY_ORIGINAL, zero, is the format's own rate.
    if let Some(b) = buffer(ctx, this) {
        b.frequency = (args[0] != 0).then_some(args[0]);
    }
    Ok(S_OK)
}

#[cfg(test)]
mod tests {
    use super::{gain, progress, resample, weights, Buffer, Listener, Place};
    use crate::sound::Format;

    fn pcm(rate: u32) -> Format {
        Format {
            tag: 1,
            channels: 1,
            rate,
            block_align: 2,
            bits: 16,
            coefficients: Vec::new(),
        }
    }

    #[test]
    fn a_buffer_is_where_the_declared_clock_has_played_it_to() {
        let mut b = Buffer::new(Some(pcm(22_050)), false);
        b.started = Some(1_000);
        // Two seconds of mono sixteen-bit is 88200 bytes; half a second in.
        assert_eq!(progress(&b, 88_200, 1_500), (22_050, false));
        assert_eq!(progress(&b, 88_200, 4_000), (88_200, true), "played out");
        b.looping = true;
        assert!(!progress(&b, 88_200, 4_000).1, "a loop goes round");
    }

    #[test]
    fn volume_pan_and_distance_weigh_each_side() {
        assert_eq!(gain(0), 1.0);
        assert!(
            (gain(-2000) - 0.1).abs() < 1e-9,
            "twenty decibels is a tenth"
        );
        assert_eq!(gain(-10_000), 0.0);

        let listener = Listener::default();
        let mut b = Buffer::new(Some(pcm(22_050)), true);
        // To the right of a listener looking down z: only the right side.
        b.place = Some(Place {
            position: [2.0, 0.0, 0.0],
            ..Place::default()
        });
        let (left, right) = weights(&b, &listener);
        assert_eq!(left, 0.0);
        assert!(
            (right - 0.5).abs() < 1e-9,
            "twice the minimum distance is half as loud"
        );
    }

    #[test]
    fn a_sound_at_half_the_rate_takes_twice_as_long() {
        let stereo = [100, -100, 200, -200];
        assert_eq!(
            resample(&stereo, 11_025, 1.0, 0.5),
            [100, -50, 100, -50, 200, -100, 200, -100]
        );
    }
}
