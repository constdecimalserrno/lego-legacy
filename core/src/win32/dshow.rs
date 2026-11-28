//! DirectShow's filter graph (`CLSID_FilterGraph`): how a title plays an AVI
//! movie directly into the window via `IGraphBuilder`, `IMediaControl`,
//! `IMediaEvent`, `IVideoWindow`, and `IMediaPosition`.
//!
//! Playback, Indeo 5 decoding, declared-clock timing, and `Mixer` PCM sync
//! live in [`crate::movie::MoviePlayer`]; this module is the COM adapter and
//! `IVideoWindow` / `IMediaEvent` state machine.

use super::{ComState, Ctx, Object, Present, State};
use crate::clock::FlipCost;
use crate::com::{method, wall, Interface, ADD_REF, QUERY_INTERFACE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::movie::{MoviePlayer, OpenError, StartOutcome, StepError};
use std::collections::VecDeque;

const E_POINTER: u32 = 0x8000_4003;
const E_ABORT: u32 = 0x8000_4004;
const VFW_E_NOT_FOUND: u32 = 0x8004_0216;
const HRESULT_ERROR_NOT_READY: u32 = 0x8007_0015;

/// `EC_COMPLETE`: all data from a stream has been rendered.
pub const EC_COMPLETE: u32 = 0x0001;

/// Automation booleans (`OATRUE` / `OAFALSE`) used by `IVideoWindow`.
const OATRUE: u32 = (-1i32) as u32;
const OAFALSE: u32 = 0;

/// Default DirectShow video window style (`WS_OVERLAPPEDWINDOW | WS_CLIPSIBLINGS | WS_CLIPCHILDREN`).
const DEFAULT_VIDEO_STYLE: u32 = 0x06cf_0000;

/// One DirectShow filter graph instance keyed by its primary COM identity.
pub struct FilterGraph {
    /// Manual-reset Win32 event handle returned by `IMediaEvent::GetEventHandle`.
    pub event_handle: u32,
    /// Queued `(event_code, param1, param2)` notifications for `IMediaEvent::GetEvent`.
    pub events: VecDeque<(u32, u32, u32)>,
    /// `IVideoWindow` properties.
    pub window_style: u32,
    pub owner: u32,
    pub message_drain: u32,
    pub visible: bool,
    pub fullscreen: bool,
    pub cursor_hidden: bool,
    pub position: [i32; 4],
    /// Guest-heap RGB565 frame buffer owned by this graph (`width * height * 2` bytes).
    pub frame_buffer: u32,
    /// Opened AVI media, once `IGraphBuilder::RenderFile` succeeds.
    pub movie: Option<MoviePlayer>,
}

impl Default for FilterGraph {
    fn default() -> Self {
        Self {
            event_handle: 0,
            events: VecDeque::new(),
            window_style: DEFAULT_VIDEO_STYLE,
            owner: 0,
            message_drain: 0,
            visible: true,
            fullscreen: false,
            cursor_hidden: false,
            position: [0, 0, 0, 0],
            frame_buffer: 0,
            movie: None,
        }
    }
}

impl FilterGraph {
    /// Whether this filter graph is actively playing a movie.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.movie.as_ref().is_some_and(MoviePlayer::is_running)
    }
}

impl crate::snap::Snap for FilterGraph {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            event_handle,
            events,
            window_style,
            owner,
            message_drain,
            visible,
            fullscreen,
            cursor_hidden,
            position,
            frame_buffer,
            movie,
        } = self;
        v.u32(event_handle);
        crate::snap::snap_vec_deque(v, events, |(c, p1, p2), v| {
            v.u32(c);
            v.u32(p1);
            v.u32(p2);
        });
        v.u32(window_style);
        v.u32(owner);
        v.u32(message_drain);
        v.bool(visible);
        v.bool(fullscreen);
        v.bool(cursor_hidden);
        for coord in position {
            v.i32(coord);
        }
        v.u32(frame_buffer);
        crate::snap::snap_option_with(v, movie, MoviePlayer::empty_for_snap, |m, v| m.snap(v));
    }
}

/// `Release` shared across all five `CLSID_FilterGraph` faces: cleans up the
/// graph, its event handle, mixer voice, and frame buffer once the last live
/// reference across every face drops to zero.
const RELEASE_GRAPH: crate::com::Method = method("Release", 0, release_graph);

/// `IGraphBuilder` (inherits `IUnknown` [0..2], `IFilterGraph` [3..10], `IGraphBuilder` [11..17]).
pub static GRAPH_BUILDER: Interface = Interface {
    name: "IGraphBuilder",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE_GRAPH,
        wall("AddFilter", 2),
        wall("RemoveFilter", 1),
        wall("EnumFilters", 1),
        wall("FindFilterByName", 2),
        wall("ConnectDirect", 3),
        wall("Reconnect", 1),
        wall("Disconnect", 1),
        wall("SetDefaultSyncSource", 0),
        wall("Connect", 2),
        wall("Render", 1),
        method("RenderFile", 2, render_file),
        wall("AddSourceFilter", 3),
        wall("SetLogFile", 1),
        wall("Abort", 0),
        wall("ShouldOperationContinue", 0),
    ],
};

/// `IMediaControl` (inherits `IUnknown` [0..2], `IDispatch` [3..6], `IMediaControl` [7..15]).
pub static MEDIA_CONTROL: Interface = Interface {
    name: "IMediaControl",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE_GRAPH,
        wall("GetTypeInfoCount", 1),
        wall("GetTypeInfo", 3),
        wall("GetIDsOfNames", 5),
        wall("Invoke", 8),
        method("Run", 0, run),
        wall("Pause", 0),
        method("Stop", 0, stop),
        wall("GetState", 2),
        wall("RenderFile", 1),
        wall("AddSourceFilter", 2),
        wall("get_FilterCollection", 1),
        wall("get_RegFilterCollection", 1),
        wall("StopWhenReady", 0),
    ],
};

/// `IMediaEvent` (inherits `IUnknown` [0..2], `IDispatch` [3..6], `IMediaEvent` [7..12]).
pub static MEDIA_EVENT: Interface = Interface {
    name: "IMediaEvent",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE_GRAPH,
        wall("GetTypeInfoCount", 1),
        wall("GetTypeInfo", 3),
        wall("GetIDsOfNames", 5),
        wall("Invoke", 8),
        method("GetEventHandle", 1, get_event_handle),
        method("GetEvent", 4, get_event),
        wall("WaitForCompletion", 2),
        wall("CancelDefaultHandling", 1),
        wall("RestoreDefaultHandling", 1),
        method("FreeEventParams", 3, crate::com::ok),
    ],
};

/// `IMediaPosition` (inherits `IUnknown` [0..2], `IDispatch` [3..6], `IMediaPosition` [7..17]).
pub static MEDIA_POSITION: Interface = Interface {
    name: "IMediaPosition",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE_GRAPH,
        wall("GetTypeInfoCount", 1),
        wall("GetTypeInfo", 3),
        wall("GetIDsOfNames", 5),
        wall("Invoke", 8),
        method("get_Duration", 1, get_duration),
        method("put_CurrentPosition", 2, put_current_position),
        method("get_CurrentPosition", 1, get_current_position),
        wall("get_StopTime", 1),
        wall("put_StopTime", 2),
        wall("get_PrerollTime", 1),
        wall("put_PrerollTime", 2),
        wall("put_Rate", 2),
        wall("get_Rate", 1),
        wall("CanSeekForward", 1),
        wall("CanSeekBackward", 1),
    ],
};

/// `IVideoWindow` (inherits `IUnknown` [0..2], `IDispatch` [3..6], `IVideoWindow` [7..45]).
pub static VIDEO_WINDOW: Interface = Interface {
    name: "IVideoWindow",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE_GRAPH,
        wall("GetTypeInfoCount", 1),
        wall("GetTypeInfo", 3),
        wall("GetIDsOfNames", 5),
        wall("Invoke", 8),
        wall("put_Caption", 1),
        wall("get_Caption", 1),
        method("put_WindowStyle", 1, put_window_style),
        method("get_WindowStyle", 1, get_window_style),
        wall("put_WindowStyleEx", 1),
        wall("get_WindowStyleEx", 1),
        wall("put_AutoShow", 1),
        wall("get_AutoShow", 1),
        wall("put_WindowState", 1),
        wall("get_WindowState", 1),
        wall("put_BackgroundPalette", 1),
        wall("get_BackgroundPalette", 1),
        method("put_Visible", 1, put_visible),
        method("get_Visible", 1, get_visible),
        wall("put_Left", 1),
        wall("get_Left", 1),
        wall("put_Width", 1),
        wall("get_Width", 1),
        wall("put_Top", 1),
        wall("get_Top", 1),
        wall("put_Height", 1),
        wall("get_Height", 1),
        method("put_Owner", 1, put_owner),
        method("get_Owner", 1, get_owner),
        method("put_MessageDrain", 1, put_message_drain),
        method("get_MessageDrain", 1, get_message_drain),
        wall("get_BorderColor", 1),
        wall("put_BorderColor", 1),
        method("get_FullScreenMode", 1, get_fullscreen_mode),
        method("put_FullScreenMode", 1, put_fullscreen_mode),
        wall("SetWindowForeground", 1),
        wall("NotifyOwnerMessage", 4),
        method("SetWindowPosition", 4, set_window_position),
        method("GetWindowPosition", 4, get_window_position),
        wall("GetMinIdealImageSize", 2),
        wall("GetMaxIdealImageSize", 2),
        wall("GetRestorePosition", 4),
        method("HideCursor", 1, hide_cursor),
        method("IsCursorHidden", 1, is_cursor_hidden),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &GRAPH_BUILDER,
    &MEDIA_CONTROL,
    &MEDIA_EVENT,
    &MEDIA_POSITION,
    &VIDEO_WINDOW,
];

/// Create a `CLSID_FilterGraph` COM object for `CoCreateInstance`, returning
/// the requested initial interface face.
///
/// # Errors
/// When guest memory faults.
pub fn create(ctx: &mut Ctx<'_>, interface: &'static Interface) -> Result<u32, Stop> {
    let identity =
        State::make_com(ctx, &GRAPH_BUILDER, ComState::FilterGraph).map_err(Stop::Fault)?;
    let event_handle = ctx.state.give(Object::Named {
        kind: "event",
        name: String::new(),
        signalled: false,
        manual: true,
        held: None,
    });
    ctx.state.filter_graphs.insert(
        identity,
        FilterGraph {
            event_handle,
            ..FilterGraph::default()
        },
    );
    if interface.name == GRAPH_BUILDER.name {
        return Ok(identity);
    }
    let face = State::face(ctx, identity, interface).map_err(Stop::Fault)?;
    // Drop the bootstrap reference on `IGraphBuilder` so only the requested face holds a ref.
    ctx.state.com_ref(identity, -1);
    Ok(face)
}

fn release_graph(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let left = ctx.state.com_ref(this, -1);
    if left == 0 && !ctx.state.has_live_refs(identity) {
        let now = ctx.declared_millis();
        ctx.state.mixer.stop(identity, now * 1_000_000);
        if let Some(graph) = ctx.state.filter_graphs.remove(&identity) {
            if graph.event_handle != 0 {
                ctx.state.close(graph.event_handle);
            }
            if graph.frame_buffer != 0 {
                ctx.state.heap.free(graph.frame_buffer);
            }
            if let Some(movie) = graph.movie {
                ctx.log.line(&format!("dshow {} released", movie.name()));
                drop(movie);
                ctx.state.movies.evict_unused();
            }
        }
    }
    Ok(left)
}

fn set_event_signalled(state: &mut State, handle: u32, value: bool) {
    if handle == 0 {
        return;
    }
    if let Some(Object::Named { signalled, .. }) = state.object_mut(handle) {
        *signalled = value;
    }
}

fn push_event(state: &mut State, identity: u32, code: u32, param1: u32, param2: u32) {
    let handle = if let Some(graph) = state.filter_graphs.get_mut(&identity) {
        graph.events.push_back((code, param1, param2));
        graph.event_handle
    } else {
        0
    };
    set_event_signalled(state, handle, true);
}

fn render_file(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // IGraphBuilder::RenderFile(lpcwstrFile, lpcwstrPlayList).
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    if args[1] != 0 {
        return Err(Stop::NotWritten {
            what: format!("IGraphBuilder::RenderFile: playlist {:#x}", args[1]),
            caller: 0,
        });
    }
    let name = ctx.wide_string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(HRESULT_ERROR_NOT_READY);
    }
    let abs = ctx.absolute(&name);
    let movie = match MoviePlayer::open(name.clone(), &abs, &mut ctx.state.movies, ctx.tree) {
        Ok(movie) => movie,
        Err(OpenError::Missing) => {
            ctx.log.line(&format!("dshow {name} missing"));
            return Ok(VFW_E_NOT_FOUND);
        }
        Err(OpenError::Parse(e)) => {
            return Err(Stop::NotWritten {
                what: format!("IGraphBuilder::RenderFile: {name}: {e}"),
                caller: 0,
            });
        }
        Err(OpenError::NoVideo) => {
            return Err(Stop::NotWritten {
                what: format!("IGraphBuilder::RenderFile: {name} has no video stream"),
                caller: 0,
            });
        }
        Err(OpenError::UnsupportedCodec(codec)) => {
            return Err(Stop::NotWritten {
                what: format!(
                    "IGraphBuilder::RenderFile: {name} uses {}, which nothing here decodes",
                    String::from_utf8_lossy(&codec)
                ),
                caller: 0,
            });
        }
    };
    let (width, height) = movie.picture_size();
    if width == 0 || height == 0 {
        drop(movie);
        ctx.state.movies.evict_unused();
        return Err(Stop::NotWritten {
            what: format!("IGraphBuilder::RenderFile: {name} has empty size {width}x{height}"),
            caller: 0,
        });
    }
    let identity = ctx.state.identity(this);
    let now = ctx.declared_millis();
    ctx.state.mixer.stop(identity, now * 1_000_000);
    ctx.log.line(&format!(
        "dshow {name} rendered: {width}x{height} IV50, {} frames",
        movie.frames()
    ));
    let event_handle = if let Some(graph) = ctx.state.filter_graphs.get_mut(&identity) {
        if graph.frame_buffer != 0 {
            ctx.state.heap.free(graph.frame_buffer);
            graph.frame_buffer = 0;
        }
        graph.events.clear();
        graph.position = [0, 0, width as i32, height as i32];
        graph.movie = Some(movie);
        graph.event_handle
    } else {
        drop(movie);
        0
    };
    ctx.state.movies.evict_unused();
    set_event_signalled(ctx.state, event_handle, false);
    Ok(S_OK)
}

/// Ensure the graph's RGB565 guest-heap frame buffer exists and is zeroed.
fn ensure_frame_buffer(
    ctx: &mut Ctx<'_>,
    identity: u32,
    width: u32,
    height: u32,
) -> Result<u32, Stop> {
    let existing = ctx
        .state
        .filter_graphs
        .get(&identity)
        .map_or(0, |g| g.frame_buffer);
    if existing != 0 {
        return Ok(existing);
    }
    let byte_len = width
        .checked_mul(height)
        .and_then(|px| px.checked_mul(2))
        .ok_or_else(|| Stop::NotWritten {
            what: format!("dshow frame buffer overflow for {width}x{height}"),
            caller: 0,
        })?;
    let buf = ctx.state.heap.alloc(byte_len);
    if buf == 0 {
        return Err(Stop::NotWritten {
            what: format!("dshow frame buffer allocation failed ({byte_len} bytes)"),
            caller: 0,
        });
    }
    ctx.mem.zero_range_raw(buf, buf.saturating_add(byte_len));
    if let Some(graph) = ctx.state.filter_graphs.get_mut(&identity) {
        graph.frame_buffer = buf;
    }
    Ok(buf)
}

/// Decode the next frame of `identity`'s movie and stage it for presentation.
fn decode_and_stage_frame(ctx: &mut Ctx<'_>, identity: u32) -> Result<(), Stop> {
    let Some((width, height)) = ctx
        .state
        .filter_graphs
        .get(&identity)
        .and_then(|g| g.movie.as_ref())
        .filter(|m| m.has_next())
        .map(MoviePlayer::picture_size)
    else {
        return Ok(());
    };
    let buf = ensure_frame_buffer(ctx, identity, width, height)?;
    let visible = {
        let Some(graph) = ctx.state.filter_graphs.get_mut(&identity) else {
            return Ok(());
        };
        let Some(movie) = graph.movie.as_mut() else {
            return Ok(());
        };
        movie
            .step_rgb565(ctx.mem, buf, width, height, width * 2)
            .map_err(|e| match e {
                StepError::Decode { index, error } => Stop::NotWritten {
                    what: format!("dshow {} frame {index}: {error}", movie.name()),
                    caller: 0,
                },
                StepError::Fault(fault) => Stop::Fault(fault),
            })?;
        graph.visible
    };
    if visible {
        ctx.state.present = Some(Present {
            pixels: buf,
            width,
            height,
            pitch: width * 2,
            depth: 16,
        });
    }
    Ok(())
}

fn run(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let now = ctx.declared_millis();
    let (outcome, name) = {
        let state = &mut *ctx.state;
        let Some(movie) = state
            .filter_graphs
            .get_mut(&identity)
            .and_then(|g| g.movie.as_mut())
        else {
            return Ok(S_OK);
        };
        let outcome =
            movie
                .start(identity, now, &mut state.mixer)
                .map_err(|e| Stop::NotWritten {
                    what: format!("IMediaControl::Run: {}: {e}", movie.name()),
                    caller: 0,
                })?;
        (outcome, movie.name().to_owned())
    };
    match outcome {
        StartOutcome::AlreadyRunning => return Ok(S_OK),
        StartOutcome::AtEnd { first_eos } => {
            ctx.log.line(&format!("dshow {name} running"));
            if first_eos {
                push_event(ctx.state, identity, EC_COMPLETE, 0, 0);
            }
        }
        StartOutcome::Started => {
            ctx.log.line(&format!("dshow {name} running"));
            decode_and_stage_frame(ctx, identity)?;
        }
    }
    Ok(S_OK)
}

fn stop(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let now = ctx.declared_millis();
    let stopped_name = {
        let state = &mut *ctx.state;
        if let Some(movie) = state
            .filter_graphs
            .get_mut(&identity)
            .and_then(|g| g.movie.as_mut())
        {
            movie.stop(identity, now, &mut state.mixer);
            Some(movie.name().to_owned())
        } else {
            state.mixer.stop(identity, now * 1_000_000);
            None
        }
    };
    if let Some(name) = stopped_name {
        ctx.log.line(&format!("dshow {name} stopped"));
    }
    Ok(S_OK)
}

fn oa_bool(v: bool) -> u32 {
    if v {
        OATRUE
    } else {
        OAFALSE
    }
}

fn get_u32(
    ctx: &mut Ctx<'_>,
    this: u32,
    out: u32,
    default: u32,
    read: impl FnOnce(&FilterGraph) -> u32,
) -> Result<u32, Stop> {
    if out == 0 {
        return Ok(E_POINTER);
    }
    let identity = ctx.state.identity(this);
    let val = ctx.state.filter_graphs.get(&identity).map_or(default, read);
    ctx.mem.write_u32(out, val).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_graph(
    ctx: &mut Ctx<'_>,
    this: u32,
    write: impl FnOnce(&mut FilterGraph),
) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    if let Some(graph) = ctx.state.filter_graphs.get_mut(&identity) {
        write(graph);
    }
    Ok(S_OK)
}

fn get_event_handle(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], 0, |g| g.event_handle)
}

fn get_event(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // IMediaEvent::GetEvent(lEventCode, lParam1, lParam2, msTimeout).
    let (ev_ptr, p1_ptr, p2_ptr, timeout) = (args[0], args[1], args[2], args[3]);
    if ev_ptr == 0 || p1_ptr == 0 || p2_ptr == 0 {
        return Ok(E_POINTER);
    }
    let identity = ctx.state.identity(this);
    let popped = {
        let Some(graph) = ctx.state.filter_graphs.get_mut(&identity) else {
            return Ok(E_ABORT);
        };
        let ev = graph.events.pop_front();
        let empty = graph.events.is_empty();
        let handle = graph.event_handle;
        (ev, empty, handle)
    };
    if let (Some((code, p1, p2)), empty, handle) = popped {
        if empty {
            set_event_signalled(ctx.state, handle, false);
        }
        ctx.mem.write_u32(ev_ptr, code).map_err(Stop::Fault)?;
        ctx.mem.write_u32(p1_ptr, p1).map_err(Stop::Fault)?;
        ctx.mem.write_u32(p2_ptr, p2).map_err(Stop::Fault)?;
        return Ok(S_OK);
    }
    if timeout > 0 && timeout != u32::MAX {
        ctx.state.waited_millis += u64::from(timeout);
    }
    Ok(E_ABORT)
}

fn get_duration(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let identity = ctx.state.identity(this);
    let secs = ctx
        .state
        .filter_graphs
        .get(&identity)
        .and_then(|g| g.movie.as_ref())
        .map_or(0.0, MoviePlayer::duration_secs);
    ctx.mem
        .write_u64(args[0], secs.to_bits())
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn get_current_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let identity = ctx.state.identity(this);
    let now = ctx.declared_millis();
    let secs = ctx
        .state
        .filter_graphs
        .get(&identity)
        .and_then(|g| g.movie.as_ref())
        .map_or(0.0, |m| m.position_secs(now));
    ctx.mem
        .write_u64(args[0], secs.to_bits())
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn put_current_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // REFTIME (64-bit IEEE double) passed by value across two x86 stack words.
    let bits = u64::from(args[0]) | (u64::from(args[1]) << 32);
    let secs = f64::from_bits(bits).max(0.0);
    let identity = ctx.state.identity(this);
    let now = ctx.declared_millis();
    let (event_handle, stage_frame) = {
        let state = &mut *ctx.state;
        let Some(graph) = state.filter_graphs.get_mut(&identity) else {
            return Ok(S_OK);
        };
        let handle = graph.event_handle;
        let Some(movie) = graph.movie.as_mut() else {
            return Ok(S_OK);
        };
        if movie.frame_for_secs(secs) > 0 {
            return Err(Stop::NotWritten {
                what: format!("IMediaPosition::put_CurrentPosition({secs:.3}s) non-zero seek"),
                caller: 0,
            });
        }
        let stage =
            movie
                .rewind(identity, now, &mut state.mixer)
                .map_err(|e| Stop::NotWritten {
                    what: format!("IMediaControl::Run: {}: {e}", movie.name()),
                    caller: 0,
                })?;
        graph.events.retain(|&(code, _, _)| code != EC_COMPLETE);
        let clear_event = graph.events.is_empty();
        (clear_event.then_some(handle), stage)
    };
    if let Some(handle) = event_handle {
        set_event_signalled(ctx.state, handle, false);
    }
    if stage_frame {
        decode_and_stage_frame(ctx, identity)?;
    }
    Ok(S_OK)
}

fn put_window_style(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| g.window_style = args[0])
}

fn get_window_style(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], DEFAULT_VIDEO_STYLE, |g| g.window_style)
}

fn put_visible(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| g.visible = args[0] != OAFALSE)
}

fn get_visible(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], OAFALSE, |g| oa_bool(g.visible))
}

fn put_owner(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| g.owner = args[0])
}

fn get_owner(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], 0, |g| g.owner)
}

fn put_message_drain(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| g.message_drain = args[0])
}

fn get_message_drain(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], 0, |g| g.message_drain)
}

fn get_fullscreen_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], OAFALSE, |g| oa_bool(g.fullscreen))
}

fn put_fullscreen_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| g.fullscreen = args[0] != OAFALSE)
}

fn set_window_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| {
        g.position = [
            args[0] as i32,
            args[1] as i32,
            args[2] as i32,
            args[3] as i32,
        ];
    })
}

fn get_window_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let pos = ctx
        .state
        .filter_graphs
        .get(&identity)
        .map_or([0; 4], |g| g.position);
    for (ptr, val) in args.iter().copied().zip(pos) {
        if ptr != 0 {
            ctx.mem.write_u32(ptr, val as u32).map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn hide_cursor(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_graph(ctx, this, |g| g.cursor_hidden = args[0] != OAFALSE)
}

fn is_cursor_hidden(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    get_u32(ctx, this, args[0], OAFALSE, |g| oa_bool(g.cursor_hidden))
}

/// Outcome of advancing a running `FilterGraph` during `MsgWaitForMultipleObjects`.
pub(super) enum WaitStep {
    /// A video frame was decoded and staged for presentation.
    FrameStaged,
    /// The movie reached end-of-stream and `EC_COMPLETE` signaled handle at index `idx`.
    Completed(usize),
    /// The caller's finite timeout elapsed before the next frame or completion.
    TimedOut,
}

/// Advance a running `FilterGraph` whose notify event appears in `handles` by
/// one video frame (or to `EC_COMPLETE`) on the declared clock.
///
/// Returns `Ok(None)` when none of `handles` belongs to a running graph.
pub(super) fn advance_for_wait(
    ctx: &mut Ctx<'_>,
    handles: &[u32],
    timeout_ms: u32,
) -> Result<Option<WaitStep>, Stop> {
    const INFINITE: u32 = 0xffff_ffff;
    let found = handles.iter().enumerate().find_map(|(idx, &h)| {
        ctx.state
            .filter_graphs
            .iter()
            .find(|(_, g)| g.event_handle == h && g.is_running())
            .map(|(&identity, _)| (idx, identity))
    });
    let Some((idx, identity)) = found else {
        return Ok(None);
    };

    let now = ctx.declared_millis();
    let (has_next, target) = {
        let Some(graph) = ctx.state.filter_graphs.get(&identity) else {
            return Ok(None);
        };
        let Some(movie) = graph.movie.as_ref() else {
            return Ok(None);
        };
        // When visible, compensate for `Clock::present(FlipCost::Refresh(hz))`
        // snapping up to one refresh interval on flip so video stays locked to
        // PCM audio with zero drift.
        let refresh_ms = match ctx.title.flip_cost {
            FlipCost::Refresh(hz) if graph.visible && hz > 0 => 1000 / hz,
            _ => 0,
        };
        let Some(deadline) = movie.next_deadline_millis(refresh_ms) else {
            return Ok(None);
        };
        (movie.has_next(), deadline)
    };
    let wait_ms = target.saturating_sub(now);
    if timeout_ms != INFINITE && u64::from(timeout_ms) < wait_ms {
        ctx.state.waited_millis += u64::from(timeout_ms);
        return Ok(Some(WaitStep::TimedOut));
    }
    ctx.state.waited_millis += wait_ms;

    if has_next {
        decode_and_stage_frame(ctx, identity)?;
        return Ok(Some(WaitStep::FrameStaged));
    }
    if let Some(movie) = ctx
        .state
        .filter_graphs
        .get_mut(&identity)
        .and_then(|g| g.movie.as_mut())
    {
        movie.mark_completed();
    }
    push_event(ctx.state, identity, EC_COMPLETE, 0, 0);
    Ok(Some(WaitStep::Completed(idx)))
}

#[cfg(test)]
mod tests {
    use super::{
        advance_for_wait, create, get_current_position, get_duration, get_event, get_event_handle,
        get_fullscreen_mode, get_message_drain, get_owner, get_visible, get_window_position,
        get_window_style, hide_cursor, is_cursor_hidden, put_current_position, put_fullscreen_mode,
        put_message_drain, put_owner, put_visible, put_window_style, release_graph, render_file,
        run, set_window_position, stop, WaitStep, EC_COMPLETE, E_ABORT, GRAPH_BUILDER, OAFALSE,
        OATRUE, VFW_E_NOT_FOUND,
    };
    use crate::com::S_OK;
    use crate::guest::GuestMem;
    use crate::win32::tests::with_test_ctx;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend_from_slice(p);
        }
        chunk(b"LIST", &body)
    }

    /// Minimal valid RIFF AVI with 15 fps `IV50` video and 22,050 Hz 16-bit stereo PCM audio.
    fn synthetic_iv50_avi(frames: usize) -> Vec<u8> {
        let mut strh_v = vec![0u8; 56];
        strh_v[0..4].copy_from_slice(b"vids");
        strh_v[4..8].copy_from_slice(b"iv50");
        strh_v[20..24].copy_from_slice(&1u32.to_le_bytes()); // scale
        strh_v[24..28].copy_from_slice(&15u32.to_le_bytes()); // rate = 15 fps
        strh_v[32..36].copy_from_slice(&(frames as u32).to_le_bytes());
        let mut strf_v = vec![0u8; 40];
        strf_v[0..4].copy_from_slice(&40u32.to_le_bytes());
        strf_v[4..8].copy_from_slice(&16u32.to_le_bytes()); // width = 16
        strf_v[8..12].copy_from_slice(&16u32.to_le_bytes()); // height = 16
        strf_v[14..16].copy_from_slice(&24u16.to_le_bytes()); // bpp
        strf_v[16..20].copy_from_slice(b"IV50");
        let strl_v = list(b"strl", &[chunk(b"strh", &strh_v), chunk(b"strf", &strf_v)]);

        let mut strh_a = vec![0u8; 56];
        strh_a[0..4].copy_from_slice(b"auds");
        strh_a[20..24].copy_from_slice(&4u32.to_le_bytes());
        strh_a[24..28].copy_from_slice(&(22_050u32 * 4).to_le_bytes());
        strh_a[44..48].copy_from_slice(&4u32.to_le_bytes());
        let mut strf_a = vec![0u8; 16];
        strf_a[0..2].copy_from_slice(&1u16.to_le_bytes()); // PCM
        strf_a[2..4].copy_from_slice(&2u16.to_le_bytes()); // stereo
        strf_a[4..8].copy_from_slice(&22_050u32.to_le_bytes());
        strf_a[8..12].copy_from_slice(&(22_050u32 * 4).to_le_bytes());
        strf_a[12..14].copy_from_slice(&4u16.to_le_bytes()); // block_align
        strf_a[14..16].copy_from_slice(&16u16.to_le_bytes()); // bits
        let strl_a = list(b"strl", &[chunk(b"strh", &strh_a), chunk(b"strf", &strf_a)]);

        let hdrl = list(b"hdrl", &[strl_v, strl_a]);
        let mut movi_parts = Vec::new();
        // Audio chunk with non-silent PCM stereo samples.
        let mut pcm = Vec::with_capacity(400);
        for i in 0..100i16 {
            let s = (i * 100).to_le_bytes();
            pcm.extend_from_slice(&s);
            pcm.extend_from_slice(&s);
        }
        movi_parts.push(chunk(b"01wb", &pcm));
        for _ in 0..frames {
            movi_parts.push(chunk(b"00dc", &[]));
        }
        let movi = list(b"movi", &movi_parts);
        let mut riff = b"AVI ".to_vec();
        riff.extend_from_slice(&hdrl);
        riff.extend_from_slice(&movi);
        chunk(b"RIFF", &riff)
    }

    fn write_wide(ctx: &mut crate::win32::Ctx<'_>, at: u32, s: &str) {
        let mut bytes = Vec::with_capacity((s.len() + 1) * 2);
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
        ctx.mem.write(at, &bytes).unwrap();
    }

    #[test]
    fn filter_graph_renders_plays_steps_seeks_completes_and_releases() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            ctx.state.current_directory = "C:\\Game".to_owned();
            ctx.tree
                .declare("C:\\Game\\intro.avi", synthetic_iv50_avi(3));

            let graph = create(ctx, &GRAPH_BUILDER).expect("create");
            let out = 0x2000u32;
            let name_at = 0x2100u32;

            // Missing file returns VFW_E_NOT_FOUND.
            write_wide(ctx, name_at, "missing.avi");
            assert_eq!(
                render_file(ctx, graph, &[name_at, 0]).unwrap(),
                VFW_E_NOT_FOUND
            );

            // RenderFile loads synthetic IV50+PCM AVI.
            write_wide(ctx, name_at, "intro.avi");
            assert_eq!(render_file(ctx, graph, &[name_at, 0]).unwrap(), S_OK);

            // IMediaEvent::GetEventHandle returns a valid manual-reset event.
            assert_eq!(get_event_handle(ctx, graph, &[out]).unwrap(), S_OK);
            let h_event = ctx.mem.read_u32(out).unwrap();
            assert_ne!(h_event, 0);

            // IVideoWindow properties round-trip.
            assert_eq!(get_window_style(ctx, graph, &[out]).unwrap(), S_OK);
            let style = ctx.mem.read_u32(out).unwrap();
            assert_eq!(
                put_window_style(ctx, graph, &[style & 0xff33_ffff]).unwrap(),
                S_OK
            );
            assert_eq!(get_window_style(ctx, graph, &[out]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out).unwrap(), style & 0xff33_ffff);
            assert_eq!(put_owner(ctx, graph, &[0x10010]).unwrap(), S_OK);
            assert_eq!(get_owner(ctx, graph, &[out]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out).unwrap(), 0x10010);
            assert_eq!(put_message_drain(ctx, graph, &[0x10010]).unwrap(), S_OK);
            assert_eq!(get_message_drain(ctx, graph, &[out]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out).unwrap(), 0x10010);
            assert_eq!(put_fullscreen_mode(ctx, graph, &[OATRUE]).unwrap(), S_OK);
            assert_eq!(get_fullscreen_mode(ctx, graph, &[out]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out).unwrap(), OATRUE);
            assert_eq!(hide_cursor(ctx, graph, &[OATRUE]).unwrap(), S_OK);
            assert_eq!(is_cursor_hidden(ctx, graph, &[out]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out).unwrap(), OATRUE);
            assert_eq!(
                set_window_position(ctx, graph, &[0, 0, 640, 480]).unwrap(),
                S_OK
            );
            assert_eq!(
                get_window_position(ctx, graph, &[out, out + 4, out + 8, out + 12]).unwrap(),
                S_OK
            );
            assert_eq!(ctx.mem.read_u32(out + 8).unwrap(), 640);
            assert_eq!(ctx.mem.read_u32(out + 12).unwrap(), 480);
            assert_eq!(put_visible(ctx, graph, &[OAFALSE]).unwrap(), S_OK);
            assert_eq!(get_visible(ctx, graph, &[out]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out).unwrap(), OAFALSE);
            assert_eq!(put_visible(ctx, graph, &[OATRUE]).unwrap(), S_OK);

            // IMediaPosition::get_Duration (3 frames at 15 fps = 0.2 s).
            assert_eq!(get_duration(ctx, graph, &[out]).unwrap(), S_OK);
            let dur = f64::from_bits(ctx.mem.read_u64(out).unwrap());
            assert!((dur - 0.2).abs() < 1e-9, "duration {dur}");

            // Run stages frame 0 and starts audio + movie_running().
            assert_eq!(run(ctx, graph, &[]).unwrap(), S_OK);
            assert!(ctx.state.movie_running());
            assert!(ctx.state.present.take().is_some());

            // Advance frames 1 and 2 via advance_for_wait, then reach EC_COMPLETE.
            assert!(matches!(
                advance_for_wait(ctx, &[h_event], u32::MAX).unwrap(),
                Some(WaitStep::FrameStaged)
            ));
            ctx.clock_millis += std::mem::take(&mut ctx.state.waited_millis);
            assert!(ctx.state.present.take().is_some());

            assert!(matches!(
                advance_for_wait(ctx, &[h_event], u32::MAX).unwrap(),
                Some(WaitStep::FrameStaged)
            ));
            ctx.clock_millis += std::mem::take(&mut ctx.state.waited_millis);
            assert!(ctx.state.present.take().is_some());

            // Next wait hits end-of-stream and queues EC_COMPLETE.
            assert!(matches!(
                advance_for_wait(ctx, &[h_event], u32::MAX).unwrap(),
                Some(WaitStep::Completed(0))
            ));
            ctx.clock_millis += std::mem::take(&mut ctx.state.waited_millis);

            // GetEvent pops EC_COMPLETE and resets the manual-reset event; a second call returns E_ABORT.
            assert_eq!(
                get_event(ctx, graph, &[out, out + 4, out + 8, 0]).unwrap(),
                S_OK
            );
            assert_eq!(ctx.mem.read_u32(out).unwrap(), EC_COMPLETE);
            assert_eq!(
                get_event(ctx, graph, &[out, out + 4, out + 8, 0]).unwrap(),
                E_ABORT
            );

            // Non-zero seek is refused rather than corrupting inter-frame IV50 state.
            let mid_bits = 0.1f64.to_bits();
            assert!(
                put_current_position(ctx, graph, &[mid_bits as u32, (mid_bits >> 32) as u32])
                    .is_err()
            );

            // Rewind via put_CurrentPosition(0.0) and verify get_CurrentPosition.
            let zero_bits = 0.0f64.to_bits();
            assert_eq!(
                put_current_position(ctx, graph, &[zero_bits as u32, (zero_bits >> 32) as u32])
                    .unwrap(),
                S_OK
            );
            assert_eq!(get_current_position(ctx, graph, &[out]).unwrap(), S_OK);
            let pos = f64::from_bits(ctx.mem.read_u64(out).unwrap());
            assert!(pos.abs() < 1e-6, "rewound pos {pos}");

            // Stop and final Release clean up graph, event handle, and frame buffer.
            assert_eq!(stop(ctx, graph, &[]).unwrap(), S_OK);
            assert!(!ctx.state.movie_running());
            assert_eq!(release_graph(ctx, graph, &[]).unwrap(), 0);
            assert!(ctx.state.filter_graphs.is_empty());
            assert!(ctx.state.object(h_event).is_none());
        });
    }
}
