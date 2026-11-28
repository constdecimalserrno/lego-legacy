//! DirectShow's multimedia streams: how a title of this era plays a movie
//! into a DirectDraw surface.
//!
//! Rock Raiders makes an `AMMultiMediaStream`, opens its movie, and asks for
//! each frame in turn by updating a sample that wraps a surface of its own
//! (#9). Playback, Indeo 5 decoding, declared-clock timing, and `Mixer` PCM
//! sync live in [`crate::movie::MoviePlayer`]; this module is the COM adapter.

use super::{ComState, Ctx, State};
use crate::com::{
    iid_text, method, read_iid, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK,
};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::movie::{MoviePlayer, OpenError, StepError};

const MSPID_PRIMARY_VIDEO: &str = "a35ff56a-9fda-11d0-8fdf-00c04fd9189d";
const MSPID_PRIMARY_AUDIO: &str = "a35ff56b-9fda-11d0-8fdf-00c04fd9189d";

/// The movie is over.
const MS_S_ENDOFSTREAM: u32 = 0x0004_0003;
/// A stream the multimedia stream does not have.
const MS_E_NOSTREAM: u32 = 0x8004_0403;
/// A sample updated before the stream was set running.
const MS_E_NOTRUNNING: u32 = 0x8004_040a;
const E_POINTER: u32 = 0x8000_4003;

/// One multimedia stream a title has made: the streams it added, and the
/// movie once one is open.
#[derive(Default)]
pub struct Player {
    video_stream: u32,
    audio_stream: u32,
    pub movie: Option<MoviePlayer>,
}

impl Player {
    /// Whether this player currently has a running movie.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.movie
            .as_ref()
            .is_some_and(|m| m.running_since().is_some())
    }
}

impl crate::snap::Snap for Player {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            video_stream,
            audio_stream,
            movie,
        } = self;
        v.u32(video_stream);
        v.u32(audio_stream);
        crate::snap::snap_option_with(v, movie, MoviePlayer::empty_for_snap, |m, v| m.snap(v));
    }
}

pub static MULTI_MEDIA_STREAM: Interface = Interface {
    name: "IAMMultiMediaStream",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        method("Release", 0, release_stream),
        wall("GetInformation", 2),
        method("GetMediaStream", 2, get_media_stream),
        wall("EnumMediaStreams", 2),
        wall("GetState", 1),
        method("SetState", 1, set_state),
        method("GetTime", 1, get_time),
        method("GetDuration", 1, get_duration),
        wall("Seek", 2),
        wall("GetEndOfStreamEventHandle", 1),
        method("Initialize", 3, initialize),
        wall("GetFilterGraph", 1),
        wall("GetFilter", 1),
        method("AddMediaStream", 4, add_media_stream),
        method("OpenFile", 2, open_file),
        wall("OpenMoniker", 3),
        wall("Render", 1),
    ],
};

pub static MEDIA_STREAM: Interface = Interface {
    name: "IMediaStream",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetMultiMediaStream", 1),
        wall("GetInformation", 2),
        wall("SetSameFormat", 2),
        wall("AllocateSample", 2),
        wall("CreateSharedSample", 3),
        wall("SendEndOfStream", 1),
    ],
};

pub static DIRECT_DRAW_MEDIA_STREAM: Interface = Interface {
    name: "IDirectDrawMediaStream",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetMultiMediaStream", 1),
        wall("GetInformation", 2),
        wall("SetSameFormat", 2),
        wall("AllocateSample", 2),
        wall("CreateSharedSample", 3),
        wall("SendEndOfStream", 1),
        method("GetFormat", 4, get_format),
        wall("SetFormat", 2),
        wall("GetDirectDraw", 1),
        wall("SetDirectDraw", 1),
        method("CreateSample", 4, create_sample),
        wall("GetTimePerFrame", 1),
    ],
};

pub static DIRECT_DRAW_STREAM_SAMPLE: Interface = Interface {
    name: "IDirectDrawStreamSample",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetMediaStream", 1),
        wall("GetSampleTimes", 3),
        wall("SetSampleTimes", 2),
        method("Update", 4, update),
        wall("CompletionStatus", 2),
        method("GetSurface", 2, get_surface),
        wall("SetRect", 1),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &MULTI_MEDIA_STREAM,
    &MEDIA_STREAM,
    &DIRECT_DRAW_MEDIA_STREAM,
    &DIRECT_DRAW_STREAM_SAMPLE,
];

/// Make the multimedia stream `CoCreateInstance` hands out.
///
/// # Errors
/// When guest memory faults.
pub fn create(ctx: &mut Ctx<'_>) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &MULTI_MEDIA_STREAM, ComState::MultiMediaStream)
        .map_err(Stop::Fault)?;
    ctx.state.players.insert(object, Player::default());
    Ok(object)
}

fn movie<'a>(ctx: &'a Ctx<'_>, player: u32) -> Option<&'a MoviePlayer> {
    ctx.state.players.get(&player)?.movie.as_ref()
}

fn movie_mut(
    players: &mut std::collections::BTreeMap<u32, Player>,
    player: u32,
) -> Option<&mut MoviePlayer> {
    players.get_mut(&player)?.movie.as_mut()
}

fn owner_of(ctx: &Ctx<'_>, this: u32) -> u32 {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::MediaStream { owner, .. } | ComState::StreamSample { owner, .. }) => *owner,
        _ => this,
    }
}

fn initialize(_ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Initialize(type, flags, graph). Reading is the only kind this machine
    // plays, and a graph handed in would be one nothing here builds.
    if args[0] != 0 || args[2] != 0 {
        return Err(Stop::NotWritten {
            what: format!(
                "IAMMultiMediaStream::Initialize: stream type {} with graph {:#x}",
                args[0], args[2]
            ),
            caller: 0,
        });
    }
    Ok(S_OK)
}

fn add_media_stream(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // AddMediaStream(stream object, purpose, flags, out).
    let purpose = iid_text(&read_iid(ctx, args[1])?);
    let video = match purpose.as_str() {
        MSPID_PRIMARY_VIDEO => true,
        MSPID_PRIMARY_AUDIO => false,
        _ => {
            return Err(Stop::NotWritten {
                what: format!("IAMMultiMediaStream::AddMediaStream: purpose {purpose}"),
                caller: 0,
            })
        }
    };
    let interface = if video {
        &DIRECT_DRAW_MEDIA_STREAM
    } else {
        &MEDIA_STREAM
    };
    let object = State::make_com(ctx, interface, ComState::MediaStream { owner: this, video })
        .map_err(Stop::Fault)?;
    if args[3] != 0 {
        ctx.mem.write_u32(args[3], object).map_err(Stop::Fault)?;
    }
    if let Some(player) = ctx.state.players.get_mut(&this) {
        if video {
            player.video_stream = object;
        } else {
            player.audio_stream = object;
        }
    }
    ctx.log.line(&format!(
        "movie stream {} added, flags {:#x}",
        if video { "video" } else { "audio" },
        args[2]
    ));
    Ok(S_OK)
}

fn open_file(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let name = ctx.wide_string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&name)? {
        // HRESULT_FROM_WIN32(ERROR_NOT_READY), the drive's own error as COM
        // carries a Win32 one.
        return Ok(0x8007_0015);
    }
    let abs = ctx.absolute(&name);
    let movie = match MoviePlayer::open(name.clone(), &abs, &mut ctx.state.movies, ctx.tree) {
        Ok(movie) => movie,
        Err(OpenError::Missing) => {
            ctx.log.line(&format!("movie {name} missing"));
            // VFW_E_NOT_FOUND.
            return Ok(0x8004_0216);
        }
        Err(OpenError::Parse(e)) => {
            return Err(Stop::NotWritten {
                what: format!("IAMMultiMediaStream::OpenFile: {name}: {e}"),
                caller: 0,
            });
        }
        Err(OpenError::NoVideo) => {
            return Err(Stop::NotWritten {
                what: format!("IAMMultiMediaStream::OpenFile: {name} has no video"),
                caller: 0,
            });
        }
        Err(OpenError::UnsupportedCodec(codec)) => {
            return Err(Stop::NotWritten {
                what: format!(
                    "IAMMultiMediaStream::OpenFile: {name} is {}, which nothing here decodes",
                    String::from_utf8_lossy(&codec)
                ),
                caller: 0,
            });
        }
    };
    let (width, height) = movie.picture_size();
    ctx.log.line(&format!(
        "movie {name} opened: {width}x{height} IV50, {} frames",
        movie.frames()
    ));
    if let Some(player) = ctx.state.players.get_mut(&this) {
        player.movie = Some(movie);
    } else {
        drop(movie);
    }
    ctx.state.movies.evict_unused();
    Ok(S_OK)
}

fn get_media_stream(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let purpose = iid_text(&read_iid(ctx, args[0])?);
    let object = ctx
        .state
        .players
        .get(&this)
        .map_or(0, |p| match purpose.as_str() {
            MSPID_PRIMARY_VIDEO => p.video_stream,
            MSPID_PRIMARY_AUDIO => p.audio_stream,
            _ => 0,
        });
    if args[1] == 0 {
        return Ok(E_POINTER);
    }
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    if object == 0 {
        return Ok(MS_E_NOSTREAM);
    }
    ctx.state.com_ref(object, 1);
    Ok(S_OK)
}

fn release_stream(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The last reference let go of ends the movie, played out or not: a title
    // that skips a movie lets go of it without stopping it first, and what is
    // left of its sound stops there.
    let left = ctx.state.com_ref(this, -1);
    if left == 0 {
        let now = ctx.declared_millis();
        ctx.state.mixer.stop(this, now * 1_000_000);
        if let Some(movie) = ctx.state.players.remove(&this).and_then(|p| p.movie) {
            ctx.log.line(&format!("movie {} let go of", movie.name()));
            drop(movie);
            ctx.state.movies.evict_unused();
        }
    }
    Ok(left)
}

fn set_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let now = ctx.declared_millis();
    let running = args[0] == 1;
    let state = &mut *ctx.state;
    let Some(movie) = movie_mut(&mut state.players, this) else {
        return Ok(S_OK);
    };
    if running {
        movie
            .start(this, now, &mut state.mixer)
            .map_err(|e| Stop::NotWritten {
                what: format!("IAMMultiMediaStream::SetState: {}: {e}", movie.name()),
                caller: 0,
            })?;
    } else {
        movie.stop(this, now, &mut state.mixer);
    }
    ctx.log.line(&format!(
        "movie {} {}",
        movie.name(),
        if running { "running" } else { "stopped" }
    ));
    Ok(S_OK)
}

fn get_format(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetFormat(current, palette, desired, flags). The movie's own size, in
    // the screen's own format: what DirectShow would have negotiated with a
    // display in that mode.
    let owner = owner_of(ctx, this);
    let Some(movie) = movie(ctx, owner) else {
        return Ok(MS_E_NOSTREAM);
    };
    let (width, height) = movie.picture_size();
    let depth = ctx.state.display_mode.2;
    for at in [args[0], args[2]] {
        if at != 0 {
            let pitch = super::ddraw::pitch_for(width, depth);
            super::ddraw::write_surface_desc(
                ctx,
                at,
                (width, height, pitch),
                super::ddraw::screen_format(depth),
                0,
            )?;
            // DDSCAPS_SYSTEMMEMORY | DDSCAPS_OFFSCREENPLAIN.
            ctx.mem.write_u32(at + 0x68, 0x840).map_err(Stop::Fault)?;
        }
    }
    if args[1] != 0 {
        ctx.mem.write_u32(args[1], 0).map_err(Stop::Fault)?;
    }
    if args[3] != 0 {
        ctx.mem.write_u32(args[3], 0).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn create_sample(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateSample(surface, rect, flags, out). With no surface the stream
    // makes one of its own, the movie's size in the screen's format, which
    // the guest then asks the sample for.
    let owner = owner_of(ctx, this);
    // A rectangle names the part of the surface the frame goes into. One that
    // starts at the surface's own corner and is at least as big as the movie
    // is the whole of it, which is what the frame is written as; anything else
    // would have to be placed, and nothing has asked (#57).
    if args[1] != 0 {
        let read = |o: u32| ctx.mem.read_u32(args[1] + o).map_err(Stop::Fault);
        let (left, top, right, bottom) = (read(0)?, read(4)?, read(8)?, read(12)?);
        let (width, height) = movie(ctx, owner).map_or((0, 0), MoviePlayer::picture_size);
        if left != 0 || top != 0 || right < width || bottom < height {
            return Err(Stop::NotWritten {
                what: format!(
                    "IDirectDrawMediaStream::CreateSample: a {}x{} movie into {left},{top} to \
                     {right},{bottom}",
                    width, height
                ),
                caller: 0,
            });
        }
    }
    let surface = if args[0] == 0 {
        let Some(movie) = movie(ctx, owner) else {
            return Ok(MS_E_NOSTREAM);
        };
        let (width, height) = movie.picture_size();
        let depth = ctx.state.display_mode.2;
        let surface = super::ddraw::make_surface(ctx, width, height, depth)?.0;
        // The stream sample owns this implicitly created surface; `GetSurface`
        // adds an external reference, and releasing the sample reclaims it once
        // external references are gone (#338).
        if let Some(obj) = ctx.state.com_mut(surface) {
            obj.refs = 0;
        }
        surface
    } else {
        args[0]
    };
    let object = State::make_com(
        ctx,
        &DIRECT_DRAW_STREAM_SAMPLE,
        ComState::StreamSample { owner, surface },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[3], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn surface_rgb565_target(ctx: &Ctx<'_>, surface: u32) -> Result<(u32, u32, u32, u32), Stop> {
    let Some(ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawStreamSample::Update: {surface:#x} is not a surface"),
            caller: 0,
        });
    };
    let depth = format.bits;
    if depth != 16 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawStreamSample::Update: a {depth}-bit surface"),
            caller: 0,
        });
    }
    Ok((pixels, width, height, pitch))
}

fn update(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Update(flags, event, apc, apc data). Only the blocking form: the call
    // returns once the next frame is in the surface.
    if args[0] != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawStreamSample::Update: flags {:#x}", args[0]),
            caller: 0,
        });
    }
    let Some(ComState::StreamSample { owner, surface }) =
        ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(MS_E_NOSTREAM);
    };
    let now = ctx.declared_millis();
    let wait_ms = {
        let Some(movie) = movie(ctx, owner) else {
            return Ok(MS_E_NOSTREAM);
        };
        let Some(deadline) = movie.next_deadline_millis(0) else {
            return Ok(MS_E_NOTRUNNING);
        };
        if !movie.has_next() {
            return Ok(MS_S_ENDOFSTREAM);
        }
        deadline.saturating_sub(now)
    };
    // A blocking update waits for its frame, and on the declared machine
    // waiting is declared time passing.
    ctx.state.waited_millis += wait_ms;
    let (pixels, width, height, pitch) = surface_rgb565_target(ctx, surface)?;
    let Some(movie) = movie_mut(&mut ctx.state.players, owner) else {
        return Ok(MS_E_NOSTREAM);
    };
    movie
        .step_rgb565(ctx.mem, pixels, width, height, pitch)
        .map_err(|e| match e {
            StepError::Decode { index, error } => Stop::NotWritten {
                what: format!(
                    "IDirectDrawStreamSample::Update: {} frame {index}: {error}",
                    movie.name()
                ),
                caller: 0,
            },
            StepError::Fault(fault) => Stop::Fault(fault),
        })?;
    Ok(S_OK)
}

fn get_surface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetSurface(surface, rect): the surface the sample fills, and all of it.
    let Some(ComState::StreamSample { surface, .. }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(MS_E_NOSTREAM);
    };
    if args[0] != 0 {
        ctx.state.com_ref(surface, 1);
        ctx.mem.write_u32(args[0], surface).map_err(Stop::Fault)?;
    }
    if args[1] != 0 {
        let (width, height) = match ctx.state.com(surface).map(|o| &o.state) {
            Some(ComState::Surface { width, height, .. }) => (*width, *height),
            _ => (0, 0),
        };
        for (i, v) in [0, 0, width, height].into_iter().enumerate() {
            ctx.mem
                .write_u32(args[1] + 4 * i as u32, v)
                .map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

/// A stream time: hundreds of nanoseconds, in eight bytes.
fn write_stream_time(ctx: &mut Ctx<'_>, at: u32, millis: u64) -> Result<(), Stop> {
    if at == 0 {
        return Ok(());
    }
    ctx.mem
        .write(at, &(millis * 10_000).to_le_bytes())
        .map_err(Stop::Fault)
}

fn get_duration(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(movie) = movie(ctx, this) else {
        return Ok(MS_E_NOSTREAM);
    };
    let millis = movie.duration_millis();
    write_stream_time(ctx, args[0], millis)?;
    Ok(S_OK)
}

fn get_time(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // How far the running stream has got, on the declared clock.
    let now = ctx.declared_millis();
    let Some(movie) = movie(ctx, this) else {
        return Ok(MS_E_NOSTREAM);
    };
    let millis = movie.elapsed_millis(now);
    write_stream_time(ctx, args[0], millis)?;
    Ok(S_OK)
}
