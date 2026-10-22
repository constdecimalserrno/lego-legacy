//! DirectShow's multimedia streams: how a title of this era plays a movie
//! into a DirectDraw surface.
//!
//! Rock Raiders makes an `AMMultiMediaStream`, opens its movie, and asks for
//! each frame in turn by updating a sample that wraps a surface of its own
//! (#9). The filter graph a real system would build underneath is not here:
//! the file is read by [`crate::avi`], its pictures by [`crate::iv50`], and
//! each one lands in the guest's surface when the declared clock says it is
//! due.

use super::{ComState, Ctx, State};
use crate::com::{
    iid_text, method, read_iid, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK,
};
use crate::cpu::Stop;
use crate::guest::GuestMem;

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
    pub movie: Option<Movie>,
}

/// One movie a title has opened.
pub struct Movie {
    pub name: String,
    bytes: Vec<u8>,
    avi: crate::avi::Avi,
    video: usize,
    decoder: crate::iv50::Decoder,
    picture: crate::iv50::Picture,
    /// The next video chunk a sample update will show.
    next: usize,
    /// The declared millisecond the stream was set running, while it runs.
    running_since: Option<u64>,
}

impl Movie {
    /// How long one frame is on screen, in declared milliseconds, as a ratio.
    fn frame_millis(&self) -> (u64, u64) {
        let stream = &self.avi.streams[self.video];
        (
            u64::from(stream.scale) * 1000,
            u64::from(stream.rate.max(1)),
        )
    }

    /// When frame `n` is due, in declared milliseconds after the start.
    fn due(&self, n: usize) -> u64 {
        let (num, den) = self.frame_millis();
        n as u64 * num / den
    }

    fn frames(&self) -> usize {
        self.avi.streams[self.video].chunks.len()
    }

    /// The movie's sound track, decoded for the mix, if it has one.
    fn sound(&self) -> Result<Option<Vec<i16>>, Stop> {
        let Some(stream) = self.avi.streams.iter().find(|s| s.is_audio()) else {
            return Ok(None);
        };
        let refuse = |e: crate::sound::Error| Stop::NotWritten {
            what: format!("IAMMultiMediaStream::SetState: {}: {e}", self.name),
            caller: 0,
        };
        let format = crate::sound::Format::parse(&stream.format).map_err(refuse)?;
        let data: Vec<u8> = stream
            .chunks
            .iter()
            .flat_map(|(at, len)| self.bytes[*at..at + len].iter().copied())
            .collect();
        format.decode(&data).map(Some).map_err(refuse)
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

fn movie<'a>(ctx: &'a Ctx<'_>, player: u32) -> Option<&'a Movie> {
    ctx.state.players.get(&player)?.movie.as_ref()
}

fn movie_mut(
    players: &mut std::collections::BTreeMap<u32, Player>,
    player: u32,
) -> Option<&mut Movie> {
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
    let Some(bytes) = ctx.open_guest_file(&name) else {
        ctx.log.line(&format!("movie {name} missing"));
        // VFW_E_NOT_FOUND.
        return Ok(0x8004_0216);
    };
    let avi = crate::avi::parse(&bytes).map_err(|e| Stop::NotWritten {
        what: format!("IAMMultiMediaStream::OpenFile: {name}: {e}"),
        caller: 0,
    })?;
    let video = avi
        .streams
        .iter()
        .position(crate::avi::Stream::is_video)
        .ok_or_else(|| Stop::NotWritten {
            what: format!("IAMMultiMediaStream::OpenFile: {name} has no video"),
            caller: 0,
        })?;
    let codec = avi.streams[video].compression();
    if !codec.eq_ignore_ascii_case(b"IV50") {
        return Err(Stop::NotWritten {
            what: format!(
                "IAMMultiMediaStream::OpenFile: {name} is {}, which nothing here decodes",
                String::from_utf8_lossy(&codec)
            ),
            caller: 0,
        });
    }
    let (width, height) = avi.streams[video].picture_size();
    let movie = Movie {
        name: name.clone(),
        bytes,
        avi,
        video,
        decoder: crate::iv50::Decoder::new(),
        picture: crate::iv50::Picture::default(),
        next: 0,
        running_since: None,
    };
    ctx.log.line(&format!(
        "movie {name} opened: {width}x{height} IV50, {} frames",
        movie.frames()
    ));
    if let Some(player) = ctx.state.players.get_mut(&this) {
        player.movie = Some(movie);
    }
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
            ctx.log.line(&format!("movie {} let go of", movie.name));
        }
    }
    Ok(left)
}

fn set_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let now = ctx.declared_millis();
    let running = args[0] == 1;
    let Some(movie) = movie_mut(&mut ctx.state.players, this) else {
        return Ok(S_OK);
    };
    let starting = running && movie.running_since.is_none();
    // A stream set running again carries on from the frame it stopped at, so
    // its start is put back by however far it had got.
    let since = now.saturating_sub(movie.due(movie.next));
    movie.running_since = running.then_some(movie.running_since.unwrap_or(since));
    let line = format!(
        "movie {} {}",
        movie.name,
        if running { "running" } else { "stopped" }
    );
    // The movie's sound starts with it, through the machine's one mix, and
    // stops when it is stopped.
    let sound = if starting { movie.sound()? } else { None };
    ctx.log.line(&line);
    if let Some(samples) = sound {
        ctx.state.mixer.play(this, samples, since * 1_000_000);
    } else if !running {
        ctx.state.mixer.stop(this, now * 1_000_000);
    }
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
    let (width, height) = movie.avi.streams[movie.video].picture_size();
    let depth = ctx.state.display_mode.2;
    for at in [args[0], args[2]] {
        if at != 0 {
            super::ddraw::write_surface_desc(
                ctx,
                at,
                width,
                height,
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
        let size = movie(ctx, owner).map(|m| m.avi.streams[m.video].picture_size());
        let (width, height) = size.unwrap_or((0, 0));
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
        let (width, height) = movie.avi.streams[movie.video].picture_size();
        let depth = ctx.state.display_mode.2;
        super::ddraw::make_surface(ctx, width, height, depth, false)?.0
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
    let Some(movie) = movie_mut(&mut ctx.state.players, owner) else {
        return Ok(MS_E_NOSTREAM);
    };
    let Some(since) = movie.running_since else {
        return Ok(MS_E_NOTRUNNING);
    };
    if movie.next >= movie.frames() {
        return Ok(MS_S_ENDOFSTREAM);
    }
    // A blocking update waits for its frame, and on the declared machine
    // waiting is declared time passing.
    let due = since + movie.due(movie.next);
    if due > now {
        ctx.state.waited_millis += due - now;
    }
    let (at, len) = movie.avi.streams[movie.video].chunks[movie.next];
    let index = movie.next;
    movie.next += 1;
    if len > 0 {
        movie
            .decoder
            .decode(&movie.bytes[at..at + len], &mut movie.picture)
            .map_err(|e| Stop::NotWritten {
                what: format!(
                    "IDirectDrawStreamSample::Update: {} frame {index}: {e}",
                    movie.name
                ),
                caller: 0,
            })?;
    }
    let picture = std::mem::take(&mut movie.picture);
    let result = put_picture(ctx, &picture, surface);
    if let Some(movie) = movie_mut(&mut ctx.state.players, owner) {
        movie.picture = picture;
    }
    result?;
    Ok(S_OK)
}

/// Write a decoded picture into a guest surface, in the surface's format.
fn put_picture(
    ctx: &mut Ctx<'_>,
    picture: &crate::iv50::Picture,
    surface: u32,
) -> Result<(), Stop> {
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
    let w = (width as usize).min(picture.width);
    let h = (height as usize).min(picture.height);
    let mut row = vec![0u8; w * 2];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = picture.rgb(x, y);
            let pixel = (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
            row[x * 2..x * 2 + 2].copy_from_slice(&pixel.to_le_bytes());
        }
        ctx.mem
            .write(pixels + y as u32 * pitch, &row)
            .map_err(Stop::Fault)?;
    }
    Ok(())
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
    let millis = movie.due(movie.frames());
    write_stream_time(ctx, args[0], millis)?;
    Ok(S_OK)
}

impl State {
    /// Whether one of this title's own movies is playing right now.
    #[must_use]
    pub fn movie_running(&self) -> bool {
        self.players
            .values()
            .any(|p| p.movie.as_ref().is_some_and(|m| m.running_since.is_some()))
    }
}

fn get_time(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // How far the running stream has got, on the declared clock.
    let now = ctx.declared_millis();
    let Some(movie) = movie(ctx, this) else {
        return Ok(MS_E_NOSTREAM);
    };
    let millis = movie
        .running_since
        .map_or(0, |since| now.saturating_sub(since));
    write_stream_time(ctx, args[0], millis)?;
    Ok(S_OK)
}
