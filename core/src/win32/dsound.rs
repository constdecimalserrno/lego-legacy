//! DirectSound.
//!
//! Mixing is real and runs whether or not a device is listening, because a
//! test reads what was mixed rather than what was heard. What reaches a
//! host device is `--audio`'s business and the host's.
//!
//! A buffer the title plays becomes a voice in the machine's mixer at the
//! declared time it was played: its samples read from the buffer's own bytes
//! as the mix reaches them, taken to the mix's rate, and weighed by its volume,
//! its pan, and where it is from the listener. Where it has got to, and whether
//! it is still playing, follow from the declared clock (ADR-0003).

use super::{stdcall, ComState, Ctx, Shim, State};
use crate::com::{method, ok, wall, Interface, ADD_REF, QUERY_INTERFACE, S_OK};
use crate::cpu::x87::maths;
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::sound::{frame_at, Format, Pcm};
use std::collections::BTreeMap;

pub const SHIMS: &[Shim] = &[
    // DirectSound exports its creator by ordinal as well as by name, and Rock
    // Raiders imports the ordinal.
    stdcall("DSOUND", "#1", 3, create),
    stdcall("DSOUND", "DirectSoundCreate", 3, create),
];

/// Every buffer, by the object the guest holds, and the listener.
#[derive(Default)]
pub struct Sounds {
    buffers: BTreeMap<u32, Buffer>,
    listener: Listener,
}

/// A buffer: its bytes, and what it plays and how.
#[derive(Clone, Debug)]
struct Buffer {
    /// Where its bytes are in guest memory, and how many there are. The
    /// primary buffer has none of its own.
    store: u32,
    bytes: u32,
    /// The format its bytes are in; `None` for the primary buffer, which
    /// plays the mix.
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
    /// The thousandths of a frame it had played past `offset` when a change
    /// while it plays brought it up to date, which it goes on from rather
    /// than rounds away (#99).
    part: u64,
    /// Where it is, when the title made it with 3D.
    place: Option<Place>,
}

/// `DS3DMODE_*`: how a 3D buffer's position is taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// In the world, and heard from where the listener is.
    Normal,
    /// Relative to the listener, wherever the listener is.
    HeadRelative,
    /// Not in 3D at all.
    Disabled,
}

impl Mode {
    fn from_ds(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Normal,
            1 => Self::HeadRelative,
            2 => Self::Disabled,
            _ => return None,
        })
    }

    fn to_ds(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::HeadRelative => 1,
            Self::Disabled => 2,
        }
    }
}

/// A 3D buffer's position, and how its loudness falls off.
#[derive(Clone, Copy, Debug)]
struct Place {
    position: [f32; 3],
    min_distance: f32,
    max_distance: f32,
    mode: Mode,
}

impl Default for Place {
    /// `DS3D_DEFAULTMINDISTANCE` and `_MAXDISTANCE`, in normal mode, at the
    /// origin.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            min_distance: 1.0,
            max_distance: 1_000_000_000.0,
            mode: Mode::Normal,
        }
    }
}

/// Where the one who hears is, and which way they face.
#[derive(Clone, Copy, Debug)]
struct Listener {
    position: [f32; 3],
    front: [f32; 3],
    top: [f32; 3],
    /// Metres to a unit, which only a Doppler shift would use.
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

/// `DSBSTATUS_PLAYING` and `DSBSTATUS_LOOPING`.
const DSBSTATUS_PLAYING: u32 = 0x1;
const DSBSTATUS_LOOPING: u32 = 0x4;

/// `DSERR_INVALIDPARAM` and `DSERR_INVALIDCALL`.
const DSERR_INVALIDPARAM: u32 = 0x8007_0057;
const DSERR_INVALIDCALL: u32 = 0x8878_0032;

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DirectSoundCreate(guid, out, outer).
    let object = make(ctx)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// `IClassFactory` over DirectSound, which is how a title that goes through
/// COM rather than through `DirectSoundCreate` asks for one (#60).
///
/// Alpha Team's sound server does exactly that: it makes DirectSound by class
/// and then asks the factory for the object.
pub static CLASS_FACTORY: Interface = Interface {
    name: "IClassFactory(DirectSound)",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        crate::com::RELEASE,
        method("CreateInstance", 3, factory_create),
        method("LockServer", 1, lock_server),
    ],
};

/// Make the class factory itself, which `CoCreateInstance` hands back.
///
/// # Errors
/// When the object will not fit in guest memory.
pub fn make_factory(ctx: &mut Ctx<'_>) -> Result<u32, Stop> {
    State::make_com(ctx, &CLASS_FACTORY, ComState::None).map_err(Stop::Fault)
}

/// Make a DirectSound object, however it was asked for.
///
/// # Errors
/// When the object will not fit in guest memory.
pub fn make(ctx: &mut Ctx<'_>) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &DIRECT_SOUND, ComState::None).map_err(Stop::Fault)?;
    ctx.log.line(&format!("dsound created at {object:#010x}"));
    Ok(object)
}

fn factory_create(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateInstance(outer, interface, out). The object is the same one
    // `DirectSoundCreate` makes; what the caller asked for it as is checked
    // the way every other face is.
    let interface = crate::com::iid_text(&crate::com::read_iid(ctx, args[1])?);
    // IID_IDirectSound, and IUnknown, which everything answers to.
    if interface != "279afa83-4981-11ce-a521-0020af0be560"
        && interface != "00000000-0000-0000-c000-000000000046"
    {
        return Err(Stop::NotWritten {
            what: format!(
                "IClassFactory(DirectSound)::CreateInstance: asked for {interface}, and this \
                 factory makes IDirectSound"
            ),
            caller: 0,
        });
    }
    let object = State::make_com(ctx, &DIRECT_SOUND, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "dsound created at {object:#010x} by its class factory"
    ));
    Ok(S_OK)
}

fn lock_server(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Nothing is ever unloaded here, so a server that is asked to stay is
    // already staying.
    Ok(S_OK)
}

/// `IDirectSound`, in slot order.
pub static DIRECT_SOUND: Interface = Interface {
    name: "IDirectSound",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        crate::com::RELEASE,
        method("CreateSoundBuffer", 3, create_buffer),
        method("GetCaps", 1, caps),
        method("DuplicateSoundBuffer", 2, duplicate),
        method("SetCooperativeLevel", 2, ok),
        method("Compact", 0, ok),
        method("GetSpeakerConfig", 1, speaker_config),
        method("SetSpeakerConfig", 1, ok),
        method("Initialize", 1, ok),
    ],
};

/// `IDirectSoundBuffer`, in slot order.
pub static DIRECT_SOUND_BUFFER: Interface = Interface {
    name: "IDirectSoundBuffer",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        method("Release", 0, release_buffer),
        method("GetCaps", 1, caps),
        method("GetCurrentPosition", 2, position),
        wall("GetFormat", 3),
        method("GetVolume", 1, |ctx, this, args| {
            let volume = buffer(ctx, this).map(|b| b.volume as u32);
            write_word(ctx, args[0], volume)
        }),
        method("GetPan", 1, |ctx, this, args| {
            let pan = buffer(ctx, this).map(|b| b.pan as u32);
            write_word(ctx, args[0], pan)
        }),
        method("GetFrequency", 1, |ctx, this, args| {
            let rate = buffer(ctx, this).and_then(|b| b.rate());
            write_word(ctx, args[0], rate)
        }),
        method("GetStatus", 1, status),
        method("Initialize", 2, ok),
        method("Lock", 7, lock),
        method("Play", 3, play),
        // It moves where it plays from, so a voice starts again from there.
        method("SetCurrentPosition", 1, |ctx, this, args| {
            change(ctx, this, false, |b| {
                b.offset = args[0].min(b.bytes);
                b.part = 0;
            })
        }),
        method("SetFormat", 1, set_format),
        method("SetVolume", 1, |ctx, this, args| {
            change(ctx, this, true, |b| {
                b.volume = (args[0] as i32).clamp(-10_000, 0)
            })
        }),
        method("SetPan", 1, |ctx, this, args| {
            change(ctx, this, true, |b| {
                b.pan = (args[0] as i32).clamp(-10_000, 10_000)
            })
        }),
        // DSBFREQUENCY_ORIGINAL, zero, is the format's own rate.
        method("SetFrequency", 1, |ctx, this, args| {
            change(ctx, this, true, |b| {
                b.frequency = (args[0] != 0).then_some(args[0])
            })
        }),
        method("Stop", 0, stop_buffer),
        method("Unlock", 4, ok),
        method("Restore", 0, ok),
    ],
};

/// `IDirectSound3DListener`: where the one who hears is, on the primary buffer.
pub static DIRECT_SOUND_3D_LISTENER: Interface = Interface {
    name: "IDirectSound3DListener",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        crate::com::RELEASE,
        wall("GetAllParameters", 1),
        method("GetDistanceFactor", 1, |ctx, _, args| {
            let value = ctx.state.sound.listener.distance_factor;
            write_float(ctx, args[0], value)
        }),
        method("GetDopplerFactor", 1, |ctx, _, args| {
            let value = ctx.state.sound.listener.doppler_factor;
            write_float(ctx, args[0], value)
        }),
        method("GetOrientation", 2, |ctx, _, args| {
            let Listener { front, top, .. } = ctx.state.sound.listener;
            write_vector(ctx, args[0], front)?;
            write_vector(ctx, args[1], top)
        }),
        method("GetPosition", 1, |ctx, _, args| {
            let value = ctx.state.sound.listener.position;
            write_vector(ctx, args[0], value)
        }),
        method("GetRolloffFactor", 1, |ctx, _, args| {
            let value = ctx.state.sound.listener.rolloff_factor;
            write_float(ctx, args[0], value)
        }),
        wall("GetVelocity", 1),
        // SetAllParameters(listener, apply): every value at once, from a
        // `DS3DLISTENER` as dsound.h lays it out: its size, then position,
        // velocity, front and top, then the distance, rolloff and Doppler
        // factors. The velocity is dropped, as `SetVelocity` drops it: the
        // mix makes no Doppler shift. Refused with `DSERR_INVALIDPARAM`, and
        // nothing kept, as DirectSound documents it: another size, an apply
        // that is neither `DS3D_IMMEDIATE` nor `DS3D_DEFERRED`, a distance
        // factor that is not above zero, or a rolloff or Doppler factor
        // outside 0 to 10 (`DS3D_MINROLLOFFFACTOR` to `DS3D_MAXDOPPLERFACTOR`).
        // LEGO Chess sets its listener this way as its sound comes up, and so
        // would Alpha Team's A3D on a renderer no run of it takes (`a3dapi.dll`
        // 0x1001FD34, #149).
        method("SetAllParameters", 2, |ctx, _, args| {
            let (at, apply) = (args[0], args[1]);
            let read = |o: u32| ctx.mem.read_u32(at + o).map_err(Stop::Fault);
            if at == 0 || apply > 1 || read(0)? != 64 {
                return Ok(DSERR_INVALIDPARAM);
            }
            let f = |o: u32| read(o).map(float);
            let vector = |o: u32| -> Result<[f32; 3], Stop> { Ok([f(o)?, f(o + 4)?, f(o + 8)?]) };
            let listener = Listener {
                position: vector(4)?,
                front: vector(28)?,
                top: vector(40)?,
                distance_factor: f(52)?,
                rolloff_factor: f(56)?,
                doppler_factor: f(60)?,
            };
            let factor = 0.0..=10.0;
            if !(listener.distance_factor > 0.0
                && factor.contains(&listener.rolloff_factor)
                && factor.contains(&listener.doppler_factor))
            {
                return Ok(DSERR_INVALIDPARAM);
            }
            ctx.state.sound.listener = listener;
            Ok(S_OK)
        }),
        method("SetDistanceFactor", 2, |ctx, _, args| {
            ctx.state.sound.listener.distance_factor = float(args[0]);
            Ok(S_OK)
        }),
        method("SetDopplerFactor", 2, |ctx, _, args| {
            ctx.state.sound.listener.doppler_factor = float(args[0]);
            Ok(S_OK)
        }),
        method("SetOrientation", 7, |ctx, _, args| {
            let listener = &mut ctx.state.sound.listener;
            listener.front = [float(args[0]), float(args[1]), float(args[2])];
            listener.top = [float(args[3]), float(args[4]), float(args[5])];
            Ok(S_OK)
        }),
        method("SetPosition", 4, |ctx, _, args| {
            ctx.state.sound.listener.position = [float(args[0]), float(args[1]), float(args[2])];
            Ok(S_OK)
        }),
        method("SetRolloffFactor", 2, |ctx, _, args| {
            ctx.state.sound.listener.rolloff_factor = float(args[0]);
            Ok(S_OK)
        }),
        // How fast the listener moves is for the Doppler shift, which the mix
        // does not make.
        method("SetVelocity", 4, ok),
        // Deferred or not, a listener's settings are kept when they are made;
        // a sound takes them when it starts.
        method("CommitDeferredSettings", 0, ok),
    ],
};

/// `IDirectSound3DBuffer`: where a sound is, on its own buffer.
pub static DIRECT_SOUND_3D_BUFFER: Interface = Interface {
    name: "IDirectSound3DBuffer",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        crate::com::RELEASE,
        wall("GetAllParameters", 1),
        wall("GetConeAngles", 2),
        wall("GetConeOrientation", 1),
        wall("GetConeOutsideVolume", 1),
        method("GetMaxDistance", 1, |ctx, this, args| {
            let value = place(ctx, this).map(|p| p.max_distance.to_bits());
            write_word(ctx, args[0], value)
        }),
        method("GetMinDistance", 1, |ctx, this, args| {
            let value = place(ctx, this).map(|p| p.min_distance.to_bits());
            write_word(ctx, args[0], value)
        }),
        method("GetMode", 1, |ctx, this, args| {
            let value = place(ctx, this).map(|p| p.mode.to_ds());
            write_word(ctx, args[0], value)
        }),
        method("GetPosition", 1, |ctx, this, args| match place(ctx, this) {
            Some(p) => write_vector(ctx, args[0], p.position),
            None => Ok(DSERR_INVALIDCALL),
        }),
        wall("GetVelocity", 1),
        wall("SetAllParameters", 2),
        wall("SetConeAngles", 3),
        wall("SetConeOrientation", 4),
        // How loud the sound is outside its cone. A cone starts all the way
        // round (DS3D_DEFAULTCONEANGLE, 360 degrees) and nothing here narrows
        // one, as SetConeAngles is a wall, so every listener is inside it and
        // this is never heard. LEGO Island sets it to DSBVOLUME_MIN (#119).
        method("SetConeOutsideVolume", 2, |ctx, this, _args| {
            with_place(ctx, this, |_| {})
        }),
        method("SetMaxDistance", 2, |ctx, this, args| {
            with_place(ctx, this, |p| p.max_distance = float(args[0]))
        }),
        method("SetMinDistance", 2, |ctx, this, args| {
            with_place(ctx, this, |p| p.min_distance = float(args[0]))
        }),
        method("SetMode", 2, |ctx, this, args| {
            let Some(mode) = Mode::from_ds(args[0]) else {
                return Ok(DSERR_INVALIDPARAM);
            };
            with_place(ctx, this, |p| p.mode = mode)
        }),
        method("SetPosition", 4, |ctx, this, args| {
            with_place(ctx, this, |p| {
                p.position = [float(args[0]), float(args[1]), float(args[2])];
            })
        }),
        // For the Doppler shift, which the mix does not make.
        method("SetVelocity", 4, ok),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &CLASS_FACTORY,
    &DIRECT_SOUND,
    &DIRECT_SOUND_BUFFER,
    &DIRECT_SOUND_3D_LISTENER,
    &DIRECT_SOUND_3D_BUFFER,
];

/// A `D3DVALUE` passed on the stack.
fn float(word: u32) -> f32 {
    f32::from_bits(word)
}

fn write_float(ctx: &mut Ctx<'_>, at: u32, value: f32) -> Result<u32, Stop> {
    write_word(ctx, at, Some(value.to_bits()))
}

/// Write an answer through a pointer the guest passed: a null pointer, or no
/// buffer to answer about, is the call's own refusal.
fn write_word(ctx: &mut Ctx<'_>, at: u32, value: Option<u32>) -> Result<u32, Stop> {
    let Some(value) = value else {
        return Ok(DSERR_INVALIDCALL);
    };
    if at == 0 {
        return Ok(DSERR_INVALIDPARAM);
    }
    ctx.mem.write_u32(at, value).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn write_vector(ctx: &mut Ctx<'_>, at: u32, v: [f32; 3]) -> Result<u32, Stop> {
    if at == 0 {
        return Ok(DSERR_INVALIDPARAM);
    }
    for (i, c) in v.iter().enumerate() {
        ctx.mem
            .write_u32(at + 4 * i as u32, c.to_bits())
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

impl Buffer {
    fn new(store: u32, bytes: u32, format: Option<Format>, three_d: bool) -> Self {
        Self {
            store,
            bytes,
            format,
            volume: 0,
            pan: 0,
            frequency: None,
            looping: false,
            started: None,
            offset: 0,
            part: 0,
            place: three_d.then(Place::default),
        }
    }

    /// The rate it plays at.
    fn rate(&self) -> Option<u32> {
        let format = self.format.as_ref()?;
        Some(self.frequency.unwrap_or(format.rate).max(1))
    }

    /// How many bytes one of its frames takes.
    fn align(&self) -> u32 {
        self.format
            .as_ref()
            .map_or(1, |f| u32::from(f.block_align.max(1)))
    }

    /// How far past `offset` it has played by a declared millisecond, in
    /// thousandths of a frame, while it plays.
    fn played(&self, millis: u64) -> Option<u64> {
        let (started, rate) = (self.started?, self.rate()?);
        Some(millis.saturating_sub(started) * u64::from(rate) + self.part)
    }

    /// Where it is in its bytes at a declared millisecond, and whether it is
    /// still playing. A sound that has played to its end is stopped, back at
    /// its beginning, as DirectSound leaves it.
    fn progress(&self, millis: u64) -> (u32, bool) {
        let Some(played) = self.played(millis) else {
            return (self.offset, self.started.is_some());
        };
        let at = u64::from(self.offset) + played / 1000 * u64::from(self.align());
        let len = u64::from(self.bytes.max(1));
        if self.looping {
            ((at % len) as u32, true)
        } else if at >= len {
            (0, false)
        } else {
            (at as u32, true)
        }
    }

    /// Bring its state up to a declared millisecond: where it is, to the part
    /// of a frame, and whether it has stopped by playing to its end. Only a
    /// change to how it plays does this. Until #99 it rounded the part away,
    /// and a buffer changed while it played went on a frame behind.
    fn settle(&mut self, millis: u64) {
        if let Some(played) = self.played(millis) {
            let (at, playing) = self.progress(millis);
            self.part = if playing { played % 1000 } else { 0 };
            self.offset = at;
            self.started = playing.then_some(millis);
        }
    }
}

fn buffer<'a>(ctx: &'a mut Ctx<'_>, this: u32) -> Option<&'a mut Buffer> {
    let identity = ctx.state.identity(this);
    ctx.state.sound.buffers.get_mut(&identity)
}

fn place(ctx: &mut Ctx<'_>, this: u32) -> Option<Place> {
    buffer(ctx, this).and_then(|b| b.place)
}

fn with_place(ctx: &mut Ctx<'_>, this: u32, change: impl FnOnce(&mut Place)) -> Result<u32, Stop> {
    match buffer(ctx, this).and_then(|b| b.place.as_mut()) {
        Some(place) => {
            change(place);
            Ok(S_OK)
        }
        // A 3D face on a buffer made without 3D.
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
        // DSCAPS_PRIMARY16BIT | DSCAPS_PRIMARYSTEREO (#93).
        ctx.mem
            .write_u32(at + 4, 0x0000_000a)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn speaker_config(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DSSPEAKER_STEREO (#93).
    write_word(ctx, args[0], Some(4))
}

/// A `WAVEFORMATEX` in guest memory. PCM's is sixteen bytes and says nothing
/// of its extra bytes; another format's is eighteen and says how many follow.
fn read_format(ctx: &Ctx<'_>, at: u32) -> Result<Format, Stop> {
    let tag = ctx.mem.slice(at, 2).map_err(Stop::Fault)?;
    let len = if u16::from_le_bytes([tag[0], tag[1]]) == 1 {
        16
    } else {
        let extra = ctx.mem.slice(at + 16, 2).map_err(Stop::Fault)?;
        18 + usize::from(u16::from_le_bytes([extra[0], extra[1]]))
    };
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
    let primary = flags & DSBCAPS_PRIMARYBUFFER != 0;
    let format = match (primary, format_at) {
        (true, _) => None,
        // A secondary buffer with no format is the call's own refusal.
        (false, 0) => return Ok(DSERR_INVALIDPARAM),
        (false, at) => Some(read_format(ctx, at)?),
    };
    let store = ctx.state.heap.alloc(bytes.max(4));
    let object =
        State::make_com(ctx, &DIRECT_SOUND_BUFFER, ComState::Sound).map_err(Stop::Fault)?;
    ctx.state.sound.buffers.insert(
        object,
        Buffer::new(store, bytes, format, flags & DSBCAPS_CTRL3D != 0),
    );
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn duplicate(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DuplicateSoundBuffer(original, out): the same bytes, played on their own
    // from the beginning.
    let Some(template) = buffer(ctx, args[0]).cloned() else {
        return Ok(DSERR_INVALIDCALL);
    };
    let object =
        State::make_com(ctx, &DIRECT_SOUND_BUFFER, ComState::Sound).map_err(Stop::Fault)?;
    let copy = Buffer {
        started: None,
        offset: 0,
        part: 0,
        looping: false,
        ..template
    };
    ctx.state.sound.buffers.insert(object, copy);
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn release_buffer(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The last reference let go stops the sound and forgets the buffer.
    let left = ctx.state.com_ref(this, -1);
    if left == 0 {
        let identity = ctx.state.identity(this);
        let nanos = ctx.clock_millis * 1_000_000;
        ctx.state.mixer.stop(identity, nanos);
        ctx.state.sound.buffers.remove(&identity);
    }
    Ok(left)
}

fn position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetCurrentPosition(play, write). A buffer's cursors derive from the
    // declared clock, never from a device: a title that waits for one to move
    // must see it move by the same amount on every run (ADR-0003). The
    // primary buffer, which has no bytes of its own, runs at the mix's rate.
    let millis = ctx.clock_millis;
    let Some(b) = buffer(ctx, this) else {
        return Ok(DSERR_INVALIDCALL);
    };
    let at = if b.format.is_some() {
        b.progress(millis).0
    } else {
        (frame_at(millis * 1_000_000) * 4) as u32
    };
    for out in [args[0], args[1]] {
        if out != 0 {
            ctx.mem.write_u32(out, at).map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn status(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetStatus(out): whether it plays, and whether round and round.
    let millis = ctx.clock_millis;
    let word = buffer(ctx, this).map(|b| match (b.progress(millis).1, b.looping) {
        (false, _) => 0,
        (true, false) => DSBSTATUS_PLAYING,
        (true, true) => DSBSTATUS_PLAYING | DSBSTATUS_LOOPING,
    });
    write_word(ctx, args[0], word)
}

fn lock(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Lock(offset, bytes, out1, len1, out2, len2, flags). The guest is handed
    // real guest memory to write its samples into, which is what the mixer
    // reads as it plays: what a title streams into a buffer already playing
    // is heard when the mix gets to it.
    const DSBLOCK_FROMWRITECURSOR: u32 = 0x1;
    const DSBLOCK_ENTIREBUFFER: u32 = 0x2;
    ctx.state
        .mixer
        .catch_up(ctx.clock_millis * 1_000_000, &*ctx.mem);
    let Some((store, bytes)) = buffer(ctx, this).map(|b| (b.store, b.bytes)) else {
        return Ok(DSERR_INVALIDCALL);
    };
    if args[6] & DSBLOCK_FROMWRITECURSOR != 0 {
        return Err(Stop::NotWritten {
            what: "IDirectSoundBuffer::Lock: from the write cursor".to_owned(),
            caller: 0,
        });
    }
    // The whole buffer is every byte of it, from the offset given.
    let wanted = if args[6] & DSBLOCK_ENTIREBUFFER != 0 {
        bytes
    } else {
        args[1]
    };
    let offset = args[0].min(bytes);
    let (first, second) = runs(offset, wanted, bytes, args[4] != 0);
    ctx.mem
        .write_u32(args[2], store + offset)
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[3], first).map_err(Stop::Fault)?;
    if args[4] != 0 {
        let at = if second > 0 { store } else { 0 };
        ctx.mem.write_u32(args[4], at).map_err(Stop::Fault)?;
    }
    if args[5] != 0 {
        ctx.mem.write_u32(args[5], second).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// The two runs a lock hands out: from the offset to as far as was asked or
/// the buffer's end, and then what was asked past the end, from the buffer's
/// start. A caller that gave nowhere to put a second run gets a lock that
/// stops at the end. A title streaming through a looping buffer writes the
/// rest of its chunk into the second run and goes on from past it, so a lock
/// that left the second run empty would lose a second of every lap of a
/// one-second buffer (#92).
fn runs(offset: u32, wanted: u32, bytes: u32, second: bool) -> (u32, u32) {
    let first = wanted.min(bytes - offset);
    let wrapped = if second {
        (wanted - first).min(offset)
    } else {
        0
    };
    (first, wrapped)
}

/// Decibels, in hundredths, as a gain.
fn gain(hundredths: i32) -> f64 {
    if hundredths <= -10_000 {
        return 0.0;
    }
    maths::pow2(f64::from(hundredths) / 2000.0 * std::f64::consts::LOG2_10)
}

/// How loud a buffer is on each side: its volume and pan, and for a 3D
/// buffer how far it is and on which side of the listener.
fn weights(buffer: &Buffer, listener: &Listener) -> (f64, f64) {
    let volume = gain(buffer.volume);
    let (mut left, mut right) = (
        volume * gain(-buffer.pan.max(0)),
        volume * gain(buffer.pan.min(0)),
    );
    let Some(place) = buffer.place.filter(|p| p.mode != Mode::Disabled) else {
        return (left, right);
    };
    // Where the sound is from the listener, in the listener's own axes.
    let (relative, across) = match place.mode {
        Mode::HeadRelative => (place.position, [1.0, 0.0, 0.0]),
        _ => (
            std::array::from_fn(|i| place.position[i] - listener.position[i]),
            super::scene::cross(listener.top, listener.front),
        ),
    };
    let dot = |a: [f32; 3], b: [f32; 3]| f64::from(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]);
    let distance = dot(relative, relative).sqrt();
    // Past the minimum distance a sound falls off as the minimum over the
    // distance, to the power of the rolloff: twice the rolloff, twice as
    // fast; and past the maximum distance it falls off no further.
    let min = f64::from(place.min_distance.max(f32::MIN_POSITIVE));
    let far = distance.clamp(min, f64::from(place.max_distance).max(min));
    let falloff = maths::pow2(f64::from(listener.rolloff_factor) * maths::log2(min / far));
    let side = if distance > 0.0 {
        let across = super::scene::normalise(across).unwrap_or([1.0, 0.0, 0.0]);
        (dot(relative, across) / distance).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    left *= falloff * (1.0 - side).min(1.0);
    right *= falloff * (1.0 + side).min(1.0);
    (left, right)
}

fn play(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Play(reserved, priority, flags): from where the buffer is, round and
    // round when DSBPLAY_LOOPING asks. A buffer already playing only takes
    // the flag, and goes on from where it is.
    const DSBPLAY_LOOPING: u32 = 0x1;
    let looping = args[2] & DSBPLAY_LOOPING != 0;
    let millis = ctx.clock_millis;
    let Some(b) = buffer(ctx, this) else {
        return Ok(DSERR_INVALIDCALL);
    };
    let playing = b.progress(millis).1;
    if playing && b.looping == looping {
        return Ok(S_OK);
    }
    b.settle(millis);
    b.looping = looping;
    b.started = Some(millis);
    start_voice(ctx, this, playing)
}

/// Put a playing buffer's sound in the mixer, from where it is now: from
/// where the buffer is, or, when it goes on, from just where its voice has
/// got to.
///
/// ponytail: its volume, pan and position are taken when it starts, so a
/// title moving a 3D sound while it plays is heard where it last started; a
/// sound moved while it plays wants the mixer to weigh it as it goes.
fn start_voice(ctx: &mut Ctx<'_>, this: u32, go_on: bool) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let millis = ctx.clock_millis;
    let listener = ctx.state.sound.listener;
    let Some(b) = ctx.state.sound.buffers.get(&identity).cloned() else {
        return Ok(DSERR_INVALIDCALL);
    };
    // The primary buffer plays the mix; it has nothing of its own.
    let (Some(format), Some(rate), Some(_)) = (b.format.as_ref(), b.rate(), b.started) else {
        return Ok(S_OK);
    };
    // A DirectSound buffer holds PCM, and nothing else.
    let pcm = Pcm::new(b.store, b.bytes, format, rate, weights(&b, &listener)).map_err(|e| {
        Stop::NotWritten {
            what: format!("IDirectSoundBuffer: {e}"),
            caller: 0,
        }
    })?;
    let nanos = millis * 1_000_000;
    if go_on {
        ctx.state.mixer.go_on(identity, pcm, nanos, b.looping);
    } else {
        let first = u64::from(b.offset / b.align());
        ctx.state
            .mixer
            .play_buffer(identity, pcm, first, nanos, b.looping);
    }
    Ok(S_OK)
}

fn stop_buffer(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let millis = ctx.clock_millis;
    if let Some(b) = buffer(ctx, this) {
        b.settle(millis);
        b.started = None;
        // Stopped, it is at a byte, and plays again from that byte.
        b.part = 0;
    }
    ctx.state.mixer.stop(identity, millis * 1_000_000);
    Ok(S_OK)
}

/// Change something about a buffer; one that is playing goes on from where
/// it is, changed, unless the change moves where it plays from.
fn change(
    ctx: &mut Ctx<'_>,
    this: u32,
    go_on: bool,
    edit: impl FnOnce(&mut Buffer),
) -> Result<u32, Stop> {
    let millis = ctx.clock_millis;
    let Some(b) = buffer(ctx, this) else {
        return Ok(DSERR_INVALIDCALL);
    };
    b.settle(millis);
    edit(b);
    if b.started.is_some() {
        b.started = Some(millis);
        return start_voice(ctx, this, go_on);
    }
    Ok(S_OK)
}

fn set_format(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetFormat(format): the primary buffer's, which is what a card would
    // play the mix at. The mix is the machine's own rate whatever it is set
    // to, so it is logged rather than kept.
    let format = read_format(ctx, args[0])?;
    ctx.log.line(&format!(
        "dsound primary format {} Hz, {} channels, {} bits",
        format.rate, format.channels, format.bits
    ));
    Ok(S_OK)
}

#[cfg(test)]
mod tests {
    use super::{gain, runs, weights, Buffer, Listener, Place};
    use crate::sound::Format;

    fn mono_16bit(rate: u32) -> Format {
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
        // Two seconds of mono sixteen-bit is 88200 bytes.
        let mut b = Buffer::new(0, 88_200, Some(mono_16bit(22_050)), false);
        b.started = Some(1_000);
        assert_eq!(b.progress(1_500), (22_050, true), "half a second in");
        // Played to its end, it is stopped and back at its beginning.
        b.settle(4_000);
        assert_eq!((b.offset, b.started), (0, None));
        b.started = Some(1_000);
        b.looping = true;
        assert_eq!(b.progress(4_000), (44_100, true), "a loop goes round");
    }

    #[test]
    fn a_change_while_it_plays_leaves_it_on_the_same_sample() {
        let mut b = Buffer::new(0, 88_200, Some(mono_16bit(22_050)), false);
        b.started = Some(1_000);
        let unchanged = b.clone();
        // Nineteen milliseconds is 418.95 frames and one more is 22.05: the
        // parts each would round away make a frame between them.
        b.settle(1_019);
        b.volume = -1_000;
        assert_eq!(b.progress(1_020), unchanged.progress(1_020));
        assert_eq!(b.progress(1_020), (882, true), "441 frames in");
    }

    #[test]
    fn a_lock_past_the_end_goes_on_from_the_start() {
        // Alpha Team's one-second stream buffer, locked as it was on #92.
        assert_eq!(runs(82_288, 11_728, 88_200, true), (5_912, 5_816));
        assert_eq!(runs(88_196, 11_732, 88_200, true), (4, 11_728));
        // With nowhere to put a second run, the lock stops at the end.
        assert_eq!(runs(82_288, 11_728, 88_200, false), (5_912, 0));
        // And one that does not reach the end is one run.
        assert_eq!(runs(1_000, 11_728, 88_200, true), (11_728, 0));
    }

    #[test]
    fn volume_pan_and_distance_weigh_each_side() {
        assert_eq!(gain(0), 1.0);
        assert!(
            (gain(-2000) - 0.1).abs() < 1e-9,
            "twenty decibels is a tenth"
        );
        assert_eq!(gain(-10_000), 0.0);

        let mut listener = Listener::default();
        let mut b = Buffer::new(0, 0, Some(mono_16bit(22_050)), true);
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
        listener.rolloff_factor = 2.0;
        let (_, right) = weights(&b, &listener);
        assert!(
            (right - 0.25).abs() < 1e-9,
            "twice the rolloff, twice as fast"
        );
    }
}
