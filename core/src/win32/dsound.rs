//! DirectSound.
//!
//! Mixing is real and runs whether or not a device is listening, because a
//! golden hashes what was mixed rather than what was heard. What reaches a
//! host device is `--audio`'s business and the host's.

use super::{ComState, Convention, Ctx, Shim, State};
use crate::com::{Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

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
        Method {
            name: "GetFormat",
            args: 3,
            run: ok,
        },
        Method {
            name: "GetVolume",
            args: 1,
            run: zero_out,
        },
        Method {
            name: "GetPan",
            args: 1,
            run: zero_out,
        },
        Method {
            name: "GetFrequency",
            args: 1,
            run: frequency,
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
            args: 6,
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
            run: ok,
        },
        Method {
            name: "SetFormat",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetVolume",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetPan",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetFrequency",
            args: 1,
            run: ok,
        },
        Method {
            name: "Stop",
            args: 0,
            run: stop_buffer,
        },
        Method {
            name: "Unlock",
            args: 4,
            run: unlock,
        },
        Method {
            name: "Restore",
            args: 0,
            run: ok,
        },
    ],
};

pub static INTERFACES: &[&Interface] = &[&DIRECT_SOUND, &DIRECT_SOUND_BUFFER];

fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn zero_out(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], 0).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
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

fn create_buffer(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateSoundBuffer(descriptor, out, outer). The descriptor's third field
    // is how many bytes the guest wants to write into.
    let bytes = ctx.mem.read_u32(args[0] + 12).map_err(Stop::Fault)?;
    let store = ctx.state.heap.alloc(bytes.max(4));
    let object = State::make_com(
        ctx,
        &DIRECT_SOUND_BUFFER,
        ComState::Sound {
            store,
            bytes,
            playing: false,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn duplicate(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(
        ctx,
        &DIRECT_SOUND_BUFFER,
        ComState::Sound {
            store: 0,
            bytes: 0,
            playing: false,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn position(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Both cursors derive from the declared clock, never from a device: a
    // title that waits for one to move must see it move by the same amount on
    // every run (ADR-0003).
    let bytes_per_second: u64 = 44_100 * 4;
    let at = (ctx.clock_millis * bytes_per_second / 1000) as u32;
    for (i, cursor) in [at, at].iter().enumerate() {
        let out = args[i];
        if out != 0 {
            ctx.mem.write_u32(out, *cursor).map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn frequency(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], 44_100).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn status(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let playing = matches!(
        ctx.state.com(this).map(|o| &o.state),
        Some(ComState::Sound { playing: true, .. })
    );
    if args[0] != 0 {
        // DSBSTATUS_PLAYING is bit zero.
        ctx.mem
            .write_u32(args[0], u32::from(playing))
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn lock(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Lock(offset, bytes, out1, len1, out2, len2). The guest is handed real
    // guest memory to write its samples into, which is what makes the mixer
    // able to read them back.
    let (offset, wanted) = (args[0], args[1]);
    let Some(ComState::Sound { store, bytes, .. }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
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

fn unlock(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn play(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Sound { playing, .. } = &mut object.state {
            *playing = true;
        }
    }
    Ok(S_OK)
}

fn stop_buffer(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Sound { playing, .. } = &mut object.state {
            *playing = false;
        }
    }
    Ok(S_OK)
}
