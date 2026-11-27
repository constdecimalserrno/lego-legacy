//! Live Twitch-Plays Spectator routes (`/watch`, `/api/stream`, `/api/events`)
//! (#310).
//!
//! Implements [`ExtraRoutes`] strictly outside guest state (Golden Rules 1 & 2,
//! ADR-0001): reads immutable [`crate::control::FrameSnapshot`],
//! [`crate::control::ControlStatus`], [`crate::control::AgentState`], and
//! `#296` telemetry snapshots from [`crate::control::ControlBus`] without ever
//! mutating `Machine` or stalling `ControlHost`.

use super::bus::{AgentState, ControlStatus, FrameSnapshot};
use super::http::{ExtraRoutes, HttpContext, HttpRequest, HttpResponse, StreamIo};
use super::json::{write_agent_state, write_json_str, write_status, write_window_metrics};
use super::observe::stuck_loop_warning;
use super::ws::{
    encode_binary_frame_with_pointer_state, encode_xor_rle, is_websocket_upgrade,
    read_client_frame, write_handshake_response, write_ws_frame, WsClientFrame,
    FRAME_FLAG_PNG_KEYFRAME, FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING,
    OPCODE_PONG,
};
use std::io;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Embedded zero-dependency single-file HTML/CSS/JS Twitch-Plays spectator UI.
pub const WATCH_HTML: &str = include_str!("watch.html");

/// Force a fresh PNG keyframe after this many consecutive XOR+RLE deltas.
const KEYFRAME_INTERVAL: u32 = 60;
/// Switch to a PNG keyframe if XOR+RLE payload exceeds this byte budget (256 KiB).
const MAX_DELTA_BYTES: usize = 256 * 1024;
/// Switch to a PNG keyframe on scene cuts affecting more than this % of pixels.
const KEYFRAME_DIFF_PCT: f64 = 45.0;
/// Default WebSocket frame push rate cap (Hz).
const DEFAULT_WS_FPS: u32 = 30;
/// Default multipart PNG fallback push rate cap (Hz).
const DEFAULT_MJPEG_FPS: u32 = 15;
/// SSE normal telemetry/flip push cadence (~12.5 Hz).
const SSE_MIN_INTERVAL: Duration = Duration::from_millis(80);
/// Keepalive heartbeat cadence for idle SSE and WebSocket streams.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Idle poll slice when waiting for new frames or state changes.
const IDLE_POLL_SLICE: Duration = Duration::from_millis(16);
/// Write timeout for streaming sockets so stalled clients disconnect cleanly.
const STREAM_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// Default [`ExtraRoutes`] implementation serving the live spectator UI and
/// real-time WebSocket / multipart PNG / SSE streams (#310).
#[derive(Clone, Copy, Debug, Default)]
pub struct SpectatorRoutes;

impl ExtraRoutes for SpectatorRoutes {
    fn handle(
        &self,
        req: &HttpRequest,
        _ctx: &HttpContext,
        stream: &mut dyn io::Write,
    ) -> io::Result<bool> {
        if matches!(req.method.as_str(), "GET" | "HEAD")
            && matches!(req.path.as_str(), "/" | "/watch" | "/watch/")
        {
            serve_watch_html(stream, req.method == "HEAD")?;
            return Ok(true);
        }
        Ok(false)
    }

    fn handle_stream(
        &self,
        req: &HttpRequest,
        ctx: &HttpContext,
        stream: &mut dyn StreamIo,
    ) -> io::Result<bool> {
        if req.method == "HEAD" && matches!(req.path.as_str(), "/" | "/watch" | "/watch/") {
            serve_watch_html(stream, true)?;
            return Ok(true);
        }
        if req.method != "GET" {
            return Ok(false);
        }
        match req.path.as_str() {
            "/" | "/watch" | "/watch/" => {
                serve_watch_html(stream, false)?;
                Ok(true)
            }
            "/api/stream" => {
                if is_websocket_upgrade(req) {
                    serve_websocket_stream(req, ctx, stream)?;
                } else {
                    serve_multipart_png_stream(req, ctx, stream)?;
                }
                Ok(true)
            }
            "/api/stream.mjpeg" => {
                serve_multipart_png_stream(req, ctx, stream)?;
                Ok(true)
            }
            "/api/events" => {
                serve_sse_events(ctx, stream)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

fn serve_watch_html(stream: &mut dyn io::Write, head_only: bool) -> io::Result<()> {
    if head_only {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             Connection: close\r\n\r\n",
            WATCH_HTML.len()
        )?;
        stream.flush()
    } else {
        HttpResponse::raw(
            200,
            "text/html; charset=utf-8",
            WATCH_HTML.as_bytes().to_vec(),
        )
        .write_to(stream)
    }
}

fn parse_fps_param(req: &HttpRequest, default_fps: u32) -> Duration {
    let fps = req
        .query
        .get("fps")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(default_fps)
        .clamp(1, 60);
    Duration::from_millis((1000 / u64::from(fps)).max(16))
}

/// Encode the next spectator binary frame payload, choosing a PNG keyframe on
/// first frame, resize, major scene cut, periodic interval, or oversized delta.
#[must_use]
pub fn encode_stream_frame(
    prev: Option<&FrameSnapshot>,
    curr: &FrameSnapshot,
    deltas_since_keyframe: u32,
) -> (Vec<u8>, bool) {
    let expected = (curr.width as usize)
        .saturating_mul(curr.height as usize)
        .saturating_mul(4);
    let curr_slice = &curr.bgra[..expected.min(curr.bgra.len())];

    let can_delta = prev.is_some_and(|p| {
        p.width == curr.width
            && p.height == curr.height
            && p.bgra.len() >= expected
            && curr.bgra.len() >= expected
            && deltas_since_keyframe < KEYFRAME_INTERVAL
            && curr.diff.diff_pct <= KEYFRAME_DIFF_PCT
    });

    if can_delta {
        if let Some(p) = prev {
            if let Ok(rle) = encode_xor_rle(&p.bgra[..expected], curr_slice) {
                if rle.len() <= MAX_DELTA_BYTES {
                    return (
                        encode_binary_frame_with_pointer_state(
                            FRAME_FLAG_XOR_RLE_DELTA,
                            curr.flip,
                            curr.width,
                            curr.height,
                            curr.pointer_pos,
                            curr.pointer_shown,
                            curr.cursor_blank,
                            &rle,
                        ),
                        false,
                    );
                }
            }
        }
    }

    let png = crate::png::encode(curr.width, curr.height, curr_slice);
    (
        encode_binary_frame_with_pointer_state(
            FRAME_FLAG_PNG_KEYFRAME,
            curr.flip,
            curr.width,
            curr.height,
            curr.pointer_pos,
            curr.pointer_shown,
            curr.cursor_blank,
            &png,
        ),
        true,
    )
}

fn is_new_snapshot(prev: Option<&FrameSnapshot>, curr: &FrameSnapshot) -> bool {
    match prev {
        None => true,
        Some(p) => {
            !Arc::ptr_eq(&p.bgra, &curr.bgra)
                && (p.width != curr.width
                    || p.height != curr.height
                    || p.frame_hash != curr.frame_hash
                    || p.pointer_pos != curr.pointer_pos
                    || p.pointer_shown != curr.pointer_shown
                    || p.cursor_blank != curr.cursor_blank)
        }
    }
}

fn poll_ws_control(stream: &mut dyn StreamIo) -> io::Result<bool> {
    if !stream.supports_read_timeout() {
        return Ok(true);
    }
    match read_client_frame(stream) {
        Ok(Some(WsClientFrame::Close(_))) => {
            let _ = write_ws_frame(stream, OPCODE_CLOSE, &[]);
            Ok(false)
        }
        Ok(Some(WsClientFrame::Ping(data))) => {
            write_ws_frame(stream, OPCODE_PONG, &data)?;
            Ok(true)
        }
        Ok(Some(WsClientFrame::Pong(_) | WsClientFrame::Data { .. }) | None) => Ok(true),
        Err(e) => Err(e),
    }
}

fn serve_websocket_stream(
    req: &HttpRequest,
    ctx: &HttpContext,
    stream: &mut dyn StreamIo,
) -> io::Result<()> {
    let Some(client_key) = req.headers.get("sec-websocket-key") else {
        return HttpResponse::error_json(400, "missing Sec-WebSocket-Key").write_to(stream);
    };
    write_handshake_response(stream, client_key)?;

    let _ = stream.set_stream_nodelay(true);
    let _ = stream.set_stream_write_timeout(Some(STREAM_WRITE_TIMEOUT));
    let _ = stream.set_stream_read_timeout(Some(IDLE_POLL_SLICE));

    let min_interval = parse_fps_param(req, DEFAULT_WS_FPS);
    let mut last_snap: Option<FrameSnapshot> = None;
    let mut deltas_since_keyframe: u32 = 0;
    let mut last_send = Instant::now()
        .checked_sub(min_interval)
        .unwrap_or_else(Instant::now);
    let mut last_ping = Instant::now();
    let mut last_ctrl_poll = Instant::now();

    loop {
        let now = Instant::now();
        let finished = ctx.bus.status().finished;

        if now.duration_since(last_send) >= min_interval || last_snap.is_none() || finished {
            if let Some(snap) = ctx.bus.latest_frame() {
                if snap.width > 0 && snap.height > 0 && is_new_snapshot(last_snap.as_ref(), &snap) {
                    let (frame_bytes, is_keyframe) =
                        encode_stream_frame(last_snap.as_ref(), &snap, deltas_since_keyframe);
                    write_ws_frame(stream, OPCODE_BINARY, &frame_bytes)?;
                    deltas_since_keyframe = if is_keyframe {
                        0
                    } else {
                        deltas_since_keyframe.saturating_add(1)
                    };
                    last_snap = Some(snap);
                    last_send = Instant::now();
                    last_ping = last_send;
                    if !finished {
                        // Periodically service client Close/Ping even under continuous 30 FPS load.
                        if last_send.duration_since(last_ctrl_poll) >= Duration::from_millis(120) {
                            let _ = stream.set_stream_read_timeout(Some(Duration::from_millis(1)));
                            let keep_open = poll_ws_control(stream)?;
                            let _ = stream.set_stream_read_timeout(Some(IDLE_POLL_SLICE));
                            last_ctrl_poll = Instant::now();
                            if !keep_open {
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
        }

        if finished {
            let _ = write_ws_frame(stream, OPCODE_CLOSE, &[]);
            break;
        }

        if Instant::now().duration_since(last_ping) >= KEEPALIVE_INTERVAL {
            write_ws_frame(stream, OPCODE_PING, b"ll")?;
            last_ping = Instant::now();
        }

        // Wait up to `IDLE_POLL_SLICE` while immediately waking on client Close/Ping/EOF.
        if stream.supports_read_timeout() {
            if !poll_ws_control(stream)? {
                break;
            }
            last_ctrl_poll = Instant::now();
        } else {
            thread::sleep(IDLE_POLL_SLICE);
        }
    }
    Ok(())
}

fn serve_multipart_png_stream(
    req: &HttpRequest,
    ctx: &HttpContext,
    stream: &mut dyn StreamIo,
) -> io::Result<()> {
    let _ = stream.set_stream_nodelay(true);
    let _ = stream.set_stream_write_timeout(Some(STREAM_WRITE_TIMEOUT));
    let _ = stream.set_stream_read_timeout(Some(IDLE_POLL_SLICE));

    write!(
        stream,
        "HTTP/1.1 200 OK\r\n\
         Content-Type: multipart/x-mixed-replace; boundary=frame\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n"
    )?;
    stream.flush()?;

    let min_interval = parse_fps_param(req, DEFAULT_MJPEG_FPS);
    let mut last_snap: Option<FrameSnapshot> = None;
    let mut last_send = Instant::now()
        .checked_sub(min_interval)
        .unwrap_or_else(Instant::now);

    loop {
        let now = Instant::now();
        let finished = ctx.bus.status().finished;

        if now.duration_since(last_send) >= min_interval || last_snap.is_none() || finished {
            if let Some(snap) = ctx.bus.latest_frame() {
                if snap.width > 0 && snap.height > 0 && is_new_snapshot(last_snap.as_ref(), &snap) {
                    let expected = (snap.width as usize)
                        .saturating_mul(snap.height as usize)
                        .saturating_mul(4);
                    let png = crate::png::encode(
                        snap.width,
                        snap.height,
                        &snap.bgra[..expected.min(snap.bgra.len())],
                    );
                    write!(
                        stream,
                        "--frame\r\n\
                         Content-Type: image/png\r\n\
                         Content-Length: {}\r\n\
                         X-Flip: {}\r\n\r\n",
                        png.len(),
                        snap.flip
                    )?;
                    stream.write_all(&png)?;
                    stream.write_all(b"\r\n")?;
                    stream.flush()?;
                    last_snap = Some(snap);
                    last_send = Instant::now();
                }
            }
        }

        if finished {
            break;
        }

        if poll_peer_alive(stream) {
            // Peer still connected after idle slice.
        } else {
            break;
        }
    }
    Ok(())
}

/// Wait up to `IDLE_POLL_SLICE` while detecting an immediate TCP peer close.
fn poll_peer_alive(stream: &mut dyn StreamIo) -> bool {
    if !stream.supports_read_timeout() {
        thread::sleep(IDLE_POLL_SLICE);
        return true;
    }
    let mut probe = [0u8; 16];
    match stream.read(&mut probe) {
        Ok(0) => false,
        Ok(_) => true,
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
            ) =>
        {
            true
        }
        Err(_) => false,
    }
}

#[derive(Clone, PartialEq)]
struct SseFingerprint {
    flip: Option<u64>,
    paused: bool,
    finished: bool,
    mode: &'static str,
    owner: &'static str,
    pace_bits: u64,
    step_uncapped: bool,
    route_active: bool,
    pointer_pos: (i32, i32),
    pointer_shown: bool,
    cursor_blank: bool,
    held_keys: Vec<u32>,
    pending_actions: usize,
    unchanged_streak: u64,
    uniform: bool,
    active_driving: bool,
    agent_state: AgentState,
}

impl SseFingerprint {
    fn from_snapshot(
        status: &ControlStatus,
        agent_state: &AgentState,
        uniform: bool,
        active_driving: bool,
    ) -> Self {
        Self {
            flip: status.flip,
            paused: status.paused,
            finished: status.finished,
            mode: status.mode.as_str(),
            owner: status.owner.as_str(),
            pace_bits: status.pace.to_bits(),
            step_uncapped: status.step_uncapped,
            route_active: status.route_active,
            pointer_pos: status.pointer_pos,
            pointer_shown: status.pointer_shown,
            cursor_blank: status.cursor_blank,
            held_keys: status.held_keys.clone(),
            pending_actions: status.pending_actions,
            unchanged_streak: status.diff.unchanged_streak,
            uniform,
            active_driving,
            agent_state: agent_state.clone(),
        }
    }

    fn urgent_change(&self, prev: &Self) -> bool {
        self.paused != prev.paused
            || self.finished != prev.finished
            || self.mode != prev.mode
            || self.owner != prev.owner
            || self.pace_bits != prev.pace_bits
            || self.step_uncapped != prev.step_uncapped
            || self.uniform != prev.uniform
            || self.active_driving != prev.active_driving
            || self.agent_state != prev.agent_state
    }
}

fn format_sse_event_with_fp(
    ctx: &HttpContext,
) -> (String, ControlStatus, AgentState, SseFingerprint) {
    let (snap, report) = ctx.bus.snapshot_for_sse_with_telemetry();
    let status = snap.status;
    let agent = snap.agent_state;
    let uniform = snap.uniform;
    let active_driving = snap.active_driving;
    let fp = SseFingerprint::from_snapshot(&status, &agent, uniform, active_driving);

    let mut warnings: Vec<String> = Vec::new();
    if uniform {
        warnings.push("uniform_frame: every pixel has the same color".to_owned());
    }
    if let Some(w) = stuck_loop_warning(
        status.diff.unchanged_streak,
        status.paused,
        status.finished,
        active_driving,
    ) {
        warnings.push(w);
    }

    let mut out = String::with_capacity(1024);
    out.push_str("{\"status\":");
    write_status(&mut out, &status);
    out.push_str(",\"telemetry\":");
    write_window_metrics(&mut out, &report.full);
    out.push_str(",\"agent_state\":");
    write_agent_state(&mut out, &agent);
    out.push_str(",\"warnings\":[");
    for (i, w) in warnings.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_str(&mut out, w);
    }
    out.push_str("]}");
    (out, status, agent, fp)
}

/// Format one SSE `data:` JSON payload combining live status, `#296` telemetry,
/// `AgentState`, and stuck/uniform critic warnings.
#[must_use]
pub fn format_sse_event_json(ctx: &HttpContext) -> (String, ControlStatus, AgentState) {
    let (out, status, agent, _) = format_sse_event_with_fp(ctx);
    (out, status, agent)
}

fn serve_sse_events(ctx: &HttpContext, stream: &mut dyn StreamIo) -> io::Result<()> {
    let _ = stream.set_stream_nodelay(true);
    let _ = stream.set_stream_write_timeout(Some(STREAM_WRITE_TIMEOUT));
    let _ = stream.set_stream_read_timeout(Some(IDLE_POLL_SLICE));

    write!(
        stream,
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream; charset=utf-8\r\n\
         Cache-Control: no-store\r\n\
         Connection: keep-alive\r\n\
         X-Accel-Buffering: no\r\n\r\n"
    )?;
    stream.flush()?;

    let mut prev_fp: Option<SseFingerprint> = None;
    let mut last_send = Instant::now()
        .checked_sub(SSE_MIN_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut last_heartbeat = Instant::now();

    loop {
        let (fp, mut finished) = {
            let snap = ctx.bus.snapshot_for_sse();
            let fp = SseFingerprint::from_snapshot(
                &snap.status,
                &snap.agent_state,
                snap.uniform,
                snap.active_driving,
            );
            (fp, snap.status.finished)
        };
        let now = Instant::now();

        let should_push = match &prev_fp {
            None => true,
            Some(prev) if fp.urgent_change(prev) => true,
            Some(prev) if fp != *prev && now.duration_since(last_send) >= SSE_MIN_INTERVAL => true,
            _ => false,
        };

        if should_push {
            let (payload, sent_status, _, sent_fp) = format_sse_event_with_fp(ctx);
            write!(stream, "event: state\ndata: {payload}\n\n")?;
            stream.flush()?;
            prev_fp = Some(sent_fp);
            finished = sent_status.finished;
            last_send = Instant::now();
            last_heartbeat = last_send;
        } else if now.duration_since(last_heartbeat) >= KEEPALIVE_INTERVAL {
            stream.write_all(b": keepalive\n\n")?;
            stream.flush()?;
            last_heartbeat = Instant::now();
        }

        if finished {
            break;
        }

        if !poll_peer_alive(stream) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::bus::{
        AgentStatePatch, Command, ControlBus, ControlConfig, ExecMode, InputOwner,
    };
    use crate::control::host::{service_flip_boundary, ControlHost};
    use crate::control::http::HttpServer;
    use crate::control::json::parse_json;
    use crate::control::ws::{decode_binary_frame_header, decode_xor_rle, websocket_accept_key};
    use crate::guest::GuestMem;
    use crate::host::headless::Headless;
    use crate::machine::Machine;
    use crate::route::Route;
    use crate::runlog::RunLog;
    use crate::stub;
    use crate::Title;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;

    /// Tiny multi-flip stub guest (presents `count` frames, mutating pixel 0 each flip).
    fn multi_flip_stub(count: u32) -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0xb9);
        code.extend_from_slice(&count.to_le_bytes()); // mov ecx, count
        code.extend_from_slice(&[0xb8, 0, 0, 0, 0]); // mov eax, 0
                                                     // loop:
        code.extend_from_slice(&[0xa3, 0x00, 0x00, 0x10, 0x00]); // mov [0x100000], eax
        code.extend_from_slice(&[0xcd, stub::INT_PRESENT]); // int 0x20
        code.push(0x40); // inc eax
        code.push(0x49); // dec ecx
        code.extend_from_slice(&[0x75, (-11i8) as u8]); // jnz loop
        code.extend_from_slice(&[0xcd, stub::INT_EXIT]); // int 0x21
        code
    }

    struct CloseOnDrop(ControlBus);
    impl Drop for CloseOnDrop {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let _ = self.0.dispatch(Command::Close { force: true });
            }
        }
    }

    /// Read one unmasked server-to-client WebSocket frame from a `BufReader`.
    fn read_server_ws_frame<R: Read>(r: &mut BufReader<R>) -> (u8, Vec<u8>) {
        let mut h = [0u8; 2];
        r.read_exact(&mut h).expect("ws header");
        let opcode = h[0] & 0x0f;
        assert_eq!(h[1] & 0x80, 0, "server frames must be unmasked");
        let len7 = h[1] & 0x7f;
        let len: usize = match len7 {
            0..=125 => usize::from(len7),
            126 => {
                let mut b = [0u8; 2];
                r.read_exact(&mut b).unwrap();
                usize::from(u16::from_be_bytes(b))
            }
            _ => {
                let mut b = [0u8; 8];
                r.read_exact(&mut b).unwrap();
                usize::try_from(u64::from_be_bytes(b)).unwrap()
            }
        };
        let mut payload = vec![0u8; len];
        r.read_exact(&mut payload).unwrap();
        (opcode, payload)
    }

    #[test]
    fn live_watch_html_sse_events_ws_stream_and_mjpeg_preserve_guest_determinism() {
        // 1. Baseline unattached run on pure Headless.
        let prog = multi_flip_stub(5);
        let mut base_host = Headless::new();
        let mut base_machine = Machine::new(&Title::STUB, &mut base_host).unwrap();
        base_machine.load_stub().unwrap();
        base_machine.arena.write(stub::ENTRY, &prog).unwrap();
        let mut base_log = RunLog::create("spectator-det-base", crate::cli::Trace::Off).unwrap();
        let base_finish = base_machine.run(&mut base_host, &mut base_log, 200_000);
        let base_digest = base_machine.state_digest();

        // 2. Live control run with default SpectatorRoutes attached by HttpContext::new.
        let mut control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                title: Title::STUB.key,
                mode: ExecMode::Step,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        )
        .with_deferred_pause(true);
        let bus = control.bus();
        bus.set_machine_attached(true);
        let ctx = Arc::new(HttpContext::new(bus.clone(), &Title::STUB, None));
        let mut server = HttpServer::start("127.0.0.1:0", ctx).expect("http start");
        let addr = server.addr();

        let client_bus = bus.clone();
        let client = thread::spawn(move || {
            let _guard = CloseOnDrop(client_bus.clone());
            let st = client_bus.wait_until_paused(Some(Duration::from_secs(5)));
            assert!(st.paused);
            assert_eq!(st.flip, Some(0));

            // A. GET /watch and GET / return 200 text/html with embedded canvas & controls.
            for path in ["/", "/watch"] {
                let mut s = TcpStream::connect(addr).unwrap();
                write!(
                    s,
                    "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                let mut resp = String::new();
                s.read_to_string(&mut resp).unwrap();
                assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"), "{path}");
                assert!(resp.contains("Content-Type: text/html; charset=utf-8"));
                assert!(resp.contains("<canvas"));
                assert!(resp.contains("/api/stream"));
                assert!(resp.contains("/api/events"));
                assert!(resp.contains("pointerBlank"));
            }

            // B. GET /api/events streams valid SSE JSON and pushes agent_state updates immediately.
            let _ = client_bus.dispatch(Command::SetAgentState(AgentStatePatch {
                goal: Some("Beat the stub".to_owned()),
                thought: Some("Checking spectator SSE".to_owned()),
                ..AgentStatePatch::default()
            }));
            let mut sse_sock = TcpStream::connect(addr).unwrap();
            sse_sock
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                sse_sock,
                "GET /api/events HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n"
            )
            .unwrap();
            let mut sse_reader = BufReader::new(sse_sock);
            let mut sse_head = String::new();
            loop {
                let mut line = String::new();
                sse_reader.read_line(&mut line).unwrap();
                sse_head.push_str(&line);
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            assert!(sse_head.starts_with("HTTP/1.1 200 OK\r\n"));
            assert!(sse_head.contains("text/event-stream"));

            let mut event_line = String::new();
            let data_line;
            loop {
                let mut line = String::new();
                sse_reader.read_line(&mut line).unwrap();
                if line.starts_with("event:") {
                    event_line = line;
                } else if line.starts_with("data:") {
                    data_line = line;
                    break;
                }
            }
            assert_eq!(event_line.trim(), "event: state");
            let json_str = data_line.trim().strip_prefix("data:").unwrap().trim();
            let parsed = parse_json(json_str).expect("valid SSE JSON");
            let obj = parsed.as_object().unwrap();
            assert_eq!(obj["status"].as_object().unwrap()["flip"].as_u64(), Some(0));
            assert_eq!(
                obj["agent_state"].as_object().unwrap()["goal"].as_str(),
                Some("Beat the stub")
            );
            assert!(obj["telemetry"]
                .as_object()
                .unwrap()
                .contains_key("throughput_mips"));
            drop(sse_reader);

            // C. GET /api/stream WebSocket upgrade delivers keyframe PNG (flip 0)
            //    followed by XOR+RLE delta frame (flip 1) that reconstructs bit-identically!
            let mut ws_sock = TcpStream::connect(addr).unwrap();
            ws_sock
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let key = "dGhlIHNhbXBsZSBub25jZQ==";
            write!(
                ws_sock,
                "GET /api/stream?fps=60 HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: {key}\r\n\
                 Sec-WebSocket-Version: 13\r\n\r\n"
            )
            .unwrap();
            let mut ws_reader = BufReader::new(ws_sock);
            let mut ws_head = String::new();
            loop {
                let mut line = String::new();
                ws_reader.read_line(&mut line).unwrap();
                ws_head.push_str(&line);
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            assert!(ws_head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
            let expected_accept = websocket_accept_key(key);
            assert!(ws_head.contains(&format!("Sec-WebSocket-Accept: {expected_accept}\r\n")));

            let snap0 = client_bus.latest_frame().expect("flip 0 frame");
            let (op0, frame0) = read_server_ws_frame(&mut ws_reader);
            assert_eq!(op0, OPCODE_BINARY);
            let (flags0, flip0, w0, h0, payload0) =
                decode_binary_frame_header(&frame0).expect("header 0");
            assert_eq!(flags0, FRAME_FLAG_PNG_KEYFRAME);
            assert_eq!(flip0, 0);
            assert_eq!((u32::from(w0), u32::from(h0)), (snap0.width, snap0.height));
            assert_eq!(&payload0[..8], b"\x89PNG\r\n\x1a\n");

            // Step 1 flip -> pixel 0 mutates -> expect XOR+RLE delta frame for flip 1.
            let step_resp =
                client_bus.dispatch_sync(Command::Step { flips: 1 }, Some(Duration::from_secs(5)));
            assert!(step_resp.ok);
            assert_eq!(step_resp.status.flip, Some(1));
            let snap1 = client_bus.latest_frame().expect("flip 1 frame");

            let (op1, frame1) = read_server_ws_frame(&mut ws_reader);
            assert_eq!(op1, OPCODE_BINARY);
            let (flags1, flip1, w1, h1, payload1) =
                decode_binary_frame_header(&frame1).expect("header 1");
            assert_eq!(flags1, FRAME_FLAG_XOR_RLE_DELTA);
            assert_eq!(flip1, 1);
            assert_eq!((u32::from(w1), u32::from(h1)), (snap1.width, snap1.height));
            let reconstructed = decode_xor_rle(&snap0.bgra, payload1).expect("decode delta");
            assert_eq!(reconstructed, *snap1.bgra);

            // Verify WebSocket Ping -> Pong round-trip while connected.
            let ws_inner = ws_reader.get_mut();
            let ping_mask = [0x12u8, 0x34, 0x56, 0x78];
            let mut ping_frame = vec![0x80 | OPCODE_PING, 0x80 | 2];
            ping_frame.extend_from_slice(&ping_mask);
            ping_frame.push(b'o' ^ ping_mask[0]);
            ping_frame.push(b'k' ^ ping_mask[1]);
            ws_inner.write_all(&ping_frame).unwrap();
            ws_inner.flush().unwrap();
            let (pong_op, pong_payload) = read_server_ws_frame(&mut ws_reader);
            assert_eq!(pong_op, OPCODE_PONG);
            assert_eq!(pong_payload, b"ok");

            // Send masked Close frame so server tears down the WS loop cleanly.
            let ws_inner = ws_reader.get_mut();
            ws_inner
                .write_all(&[0x80 | OPCODE_CLOSE, 0x80, 0x11, 0x22, 0x33, 0x44])
                .unwrap();
            let _ = ws_inner.flush();
            drop(ws_reader);

            // D. GET /api/stream without Upgrade returns multipart/x-mixed-replace PNG fallback.
            let mut mj_sock = TcpStream::connect(addr).unwrap();
            mj_sock
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(
                mj_sock,
                "GET /api/stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut mj_reader = BufReader::new(mj_sock);
            let mut mj_head = String::new();
            loop {
                let mut line = String::new();
                mj_reader.read_line(&mut line).unwrap();
                mj_head.push_str(&line);
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            assert!(mj_head.starts_with("HTTP/1.1 200 OK\r\n"));
            assert!(mj_head.contains("multipart/x-mixed-replace; boundary=frame"));
            let mut boundary = String::new();
            mj_reader.read_line(&mut boundary).unwrap();
            assert_eq!(boundary.trim(), "--frame");
            let mut part_len = 0usize;
            loop {
                let mut line = String::new();
                mj_reader.read_line(&mut line).unwrap();
                if let Some(v) = line.strip_prefix("Content-Length:") {
                    part_len = v.trim().parse().unwrap();
                }
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }
            assert!(part_len > 8);
            let mut png_part = vec![0u8; part_len];
            mj_reader.read_exact(&mut png_part).unwrap();
            assert_eq!(&png_part[..8], b"\x89PNG\r\n\x1a\n");
            drop(mj_reader);

            // Switch to Live mode so the stub finishes cleanly.
            let _ = client_bus.dispatch(Command::SetMode {
                mode: ExecMode::Live,
                step_uncapped: None,
            });
        });

        let mut machine = Machine::new(&Title::STUB, &mut control).unwrap();
        machine.load_stub().unwrap();
        machine.arena.write(stub::ENTRY, &prog).unwrap();
        machine.recording = Some(Route::new());
        let obs_bus = bus.clone();
        machine.set_flip_observer(Box::new(move |m, h| {
            service_flip_boundary(&obs_bus, m, h);
        }));
        let mut log = RunLog::create("spectator-det-live", crate::cli::Trace::Off).unwrap();
        let finish = machine.run(&mut control, &mut log, 200_000);
        let digest = machine.state_digest();
        control.mark_finished(&finish, &digest);

        client.join().unwrap();
        server.stop();

        // Bit-exact guest determinism: watching over /watch, /api/events, /api/stream (WS + MJPEG)
        // must not change a single cycle, flip, or byte of guest state!
        assert_eq!(finish, base_finish);
        assert_eq!(digest, base_digest);
    }

    #[test]
    fn encode_stream_frame_selects_keyframe_on_first_resize_cut_and_interval() {
        use crate::control::bus::FrameDiff;
        use crate::control::ws::{decode_binary_frame, FRAME_FLAG_HAS_POINTER};
        let mk_snap = |flip: u64, w: u32, h: u32, fill: u8, diff_pct: f64| FrameSnapshot {
            flip,
            width: w,
            height: h,
            bgra: Arc::new(vec![fill; (w * h * 4) as usize]),
            frame_hash: format!("{flip:016x}"),
            diff: FrameDiff {
                diff_pixels: 1,
                total_pixels: u64::from(w) * u64::from(h),
                diff_pct,
                diff_bbox: None,
                unchanged_streak: 0,
            },
            uniform: true,
            pointer_pos: (10 + flip as i32, 20 + flip as i32),
            pointer_shown: flip.is_multiple_of(2),
            cursor_blank: flip == 0,
        };

        let f0 = mk_snap(0, 16, 12, 0x10, 100.0);
        let (bytes0, kf0) = encode_stream_frame(None, &f0, 0);
        assert!(kf0);
        let h0 = decode_binary_frame(&bytes0).expect("v2 keyframe header");
        assert_eq!(h0.flags, FRAME_FLAG_PNG_KEYFRAME | FRAME_FLAG_HAS_POINTER);
        assert_eq!(h0.kind, FRAME_FLAG_PNG_KEYFRAME);
        assert_eq!(h0.pointer, Some(((10, 20), true)));
        assert!(h0.pointer_blank);
        assert!(!h0.cursor_visible());

        let f1 = mk_snap(1, 16, 12, 0x12, 5.0);
        let (bytes1, kf1) = encode_stream_frame(Some(&f0), &f1, 0);
        assert!(!kf1);
        let h1 = decode_binary_frame(&bytes1).expect("v2 delta header");
        assert_eq!(h1.flags, FRAME_FLAG_XOR_RLE_DELTA | FRAME_FLAG_HAS_POINTER);
        assert_eq!(h1.kind, FRAME_FLAG_XOR_RLE_DELTA);
        assert_eq!(h1.pointer, Some(((11, 21), false)));
        assert!(!h1.pointer_blank);
        assert!(h1.cursor_visible());

        // Resize forces keyframe.
        let f_resize = mk_snap(2, 32, 24, 0x12, 5.0);
        let (bytes_r, kf_r) = encode_stream_frame(Some(&f1), &f_resize, 1);
        assert!(kf_r);
        assert_eq!(
            decode_binary_frame(&bytes_r).unwrap().kind,
            FRAME_FLAG_PNG_KEYFRAME
        );

        // Scene cut (> KEYFRAME_DIFF_PCT) forces keyframe.
        let f_cut = mk_snap(3, 16, 12, 0x80, 60.0);
        let (bytes_c, kf_c) = encode_stream_frame(Some(&f1), &f_cut, 1);
        assert!(kf_c);
        assert_eq!(
            decode_binary_frame(&bytes_c).unwrap().kind,
            FRAME_FLAG_PNG_KEYFRAME
        );

        // Periodic interval forces keyframe.
        let (bytes_i, kf_i) = encode_stream_frame(Some(&f0), &f1, KEYFRAME_INTERVAL);
        assert!(kf_i);
        assert_eq!(
            decode_binary_frame(&bytes_i).unwrap().kind,
            FRAME_FLAG_PNG_KEYFRAME
        );
    }

    #[test]
    fn head_watch_routes_return_200_with_content_length_and_empty_body() {
        let bus = ControlBus::new(ControlConfig::default());
        let ctx = Arc::new(HttpContext::new(bus, &Title::STUB, None));
        let mut server = HttpServer::start("127.0.0.1:0", ctx).expect("http start");
        let addr = server.addr();
        let expected_cl = format!("Content-Length: {}\r\n", WATCH_HTML.len());

        for path in ["/", "/watch", "/watch/"] {
            let mut s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            write!(
                s,
                "HEAD {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut resp = Vec::new();
            s.read_to_end(&mut resp).unwrap();
            let text = String::from_utf8(resp).expect("utf-8 head response");
            let (head, body) = text.split_once("\r\n\r\n").expect("header terminator");
            assert!(
                head.starts_with("HTTP/1.1 200 OK\r\n"),
                "HEAD {path}: {head}"
            );
            assert!(
                head.contains("Content-Type: text/html; charset=utf-8\r\n"),
                "HEAD {path}: {head}"
            );
            assert!(
                format!("{head}\r\n").contains(&expected_cl),
                "HEAD {path}: expected `{expected_cl}` in {head}"
            );
            assert!(
                body.is_empty(),
                "HEAD {path} must return an empty body, got {} bytes",
                body.len()
            );
        }

        server.stop();
    }

    #[test]
    fn sse_event_json_suppresses_stuck_loop_while_paused_finished_or_below_180_flips() {
        use crate::control::observe::STUCK_LOOP_THRESHOLD;
        use crate::host::{FlipMeta, Frame, Host};
        use crate::machine::{Finish, Outcome};

        let mut host = ControlHost::new(
            Headless::new(),
            ControlConfig {
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();
        let ctx = HttpContext::new(bus.clone(), &Title::STUB, None);

        let mut pixels = vec![0u8; 64];
        pixels[0] = 0x55;
        let present = |h: &mut ControlHost<Headless>, flip: u64| {
            h.present(&Frame {
                width: 4,
                height: 4,
                pitch: 16,
                pixels: &pixels,
            });
            h.observe_flip(&FlipMeta {
                flip,
                clock_nanos: flip * 16_000_000,
                cursor: flip * 100,
                instructions_passed: 0,
                pointer_shown: true,
                pointer_pos: (320, 240),
                route_active: false,
                route_remaining: 0,
            });
        };

        // 88-flip idle loop in active play: no stuck_loop warning.
        for flip in 0..=88 {
            present(&mut host, flip);
        }
        let (json, _, _) = format_sse_event_json(&ctx);
        let parsed = parse_json(&json).unwrap();
        assert!(parsed.as_object().unwrap()["warnings"]
            .as_array()
            .unwrap()
            .is_empty());

        // Advance past STUCK_LOOP_THRESHOLD and schedule an agent action so
        // active play WOULD emit stuck_loop.
        for flip in 89..=STUCK_LOOP_THRESHOLD {
            present(&mut host, flip);
        }
        assert!(
            bus.dispatch(Command::Act {
                action: crate::control::timeline::Action::KeyHold {
                    key: "Space".to_owned(),
                    hold_flips: 1,
                    hold_ms: None,
                },
                auto_step: false,
            })
            .ok
        );

        // 180+ flips in active play while agent is driving: warning emitted.
        let (json, _, _) = format_sse_event_json(&ctx);
        let parsed = parse_json(&json).unwrap();
        let warnings = parsed.as_object().unwrap()["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].as_str().unwrap().starts_with("stuck_loop:"));

        // 180+ flips while paused: suppressed (pause genuinely via ControlHost::observe_flip).
        assert!(bus.dispatch(Command::Pause).ok);
        let pause_pixels = pixels.clone();
        let pause_flip = STUCK_LOOP_THRESHOLD + 1;
        let paused_thread = thread::spawn(move || {
            host.present(&Frame {
                width: 4,
                height: 4,
                pitch: 16,
                pixels: &pause_pixels,
            });
            host.observe_flip(&FlipMeta {
                flip: pause_flip,
                clock_nanos: pause_flip * 16_000_000,
                cursor: pause_flip * 100,
                instructions_passed: 0,
                pointer_shown: true,
                pointer_pos: (320, 240),
                route_active: false,
                route_remaining: 0,
            });
            host
        });
        let st = bus.wait_until_paused(Some(Duration::from_secs(2)));
        assert!(st.paused);
        let (json, _, _) = format_sse_event_json(&ctx);
        let parsed = parse_json(&json).unwrap();
        assert!(parsed.as_object().unwrap()["warnings"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(bus.dispatch(Command::Resume).ok);
        let host = paused_thread.join().unwrap();

        // 180+ flips once finished: suppressed.
        host.mark_finished(
            &Finish {
                outcome: Outcome::Exited { code: 0 },
                cursor: 0,
                flips: pause_flip,
                declared_millis: pause_flip * 16,
            },
            "0000000000000000",
        );
        let (json, _, _) = format_sse_event_json(&ctx);
        let parsed = parse_json(&json).unwrap();
        assert!(parsed.as_object().unwrap()["warnings"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn ws_stream_frame_locked_pointer_matches_rendered_centroid_for_relative_modes() {
        use crate::control::host::{
            find_green_cursor_centroid, png_crosshair_red_center, render_3x3_cursor_bgra,
        };
        use crate::control::observe::{render_frame_png, ObserveOptions};
        use crate::control::timeline::Action;
        use crate::control::ws::decode_binary_frame;
        use crate::host::{FlipMeta, Frame, Host, Input};
        use crate::title::{MouseMode, TitleGuide};

        const W: u32 = 320;
        const H: u32 = 240;

        fn wire_png_to_bgra(png: &[u8]) -> Vec<u8> {
            let (pw, ph, rgb) = crate::png::decode_png_to_rgb(png);
            assert_eq!((pw, ph), (W, H));
            let mut bgra = Vec::with_capacity((W * H * 4) as usize);
            for px in rgb.as_chunks::<3>().0 {
                bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
            bgra
        }

        const SOFT_REL: TitleGuide = TitleGuide {
            display_name: "WS SoftwareRelative",
            resolution: (W, H),
            mouse_mode: MouseMode::SoftwareRelative {
                virtual_span: (W, H),
                start_pos: (160, 120),
            },
            ..TitleGuide::EMPTY
        };
        const REL_STEER: TitleGuide = TitleGuide {
            display_name: "WS RelativeSteered",
            resolution: (W, H),
            mouse_mode: MouseMode::RelativeSteered {
                virtual_span: (640, 640),
                recenter: (160, 120),
                start_corner: (0, 240),
            },
            ..TitleGuide::EMPTY
        };

        for (guide, recenter_every_flip, scale) in [
            (&SOFT_REL, false, (1.0f64, 1.0f64)),
            (&REL_STEER, true, (2.0f64, 640.0 / 240.0)),
        ] {
            let mut host = ControlHost::new(
                Headless::new(),
                ControlConfig {
                    title: Title::STUB.key,
                    mode: ExecMode::Live,
                    owner: InputOwner::Agent,
                    guide,
                    ..ControlConfig::default()
                },
            );
            let bus = host.bus();
            let ctx = Arc::new(HttpContext::new(bus.clone(), &Title::STUB, None));
            let mut server = HttpServer::start("127.0.0.1:0", ctx).expect("http start");
            let addr = server.addr();

            let mut hw = (160i32, 120i32);
            let mut cur = match guide.mouse_mode {
                MouseMode::RelativeSteered { start_corner, .. } => (
                    f64::from(start_corner.0),
                    f64::from((start_corner.1).min(H as i32 - 1)),
                ),
                MouseMode::SoftwareRelative { start_pos, .. } => {
                    (f64::from(start_pos.0), f64::from(start_pos.1))
                }
                _ => (160.0, 120.0),
            };
            let present =
                |host: &mut ControlHost<Headless>, flip: u64, cur: (f64, f64), hw: (i32, i32)| {
                    let bgra =
                        render_3x3_cursor_bgra(W, H, cur.0.round() as i32, cur.1.round() as i32);
                    host.present(&Frame {
                        width: W,
                        height: H,
                        pitch: W * 4,
                        pixels: &bgra,
                    });
                    host.observe_flip(&FlipMeta {
                        flip,
                        clock_nanos: flip * 16_000_000,
                        cursor: flip * 100,
                        instructions_passed: 0,
                        pointer_shown: false,
                        pointer_pos: hw,
                        route_active: false,
                        route_remaining: 0,
                    });
                };

            // Flip 0: initial keyframe.
            present(&mut host, 0, cur, hw);

            // Connect live WS client to /api/stream?fps=60.
            let mut ws_sock = TcpStream::connect(addr).unwrap();
            ws_sock
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let key = "dGhlIHNhbXBsZSBub25jZQ==";
            write!(
                ws_sock,
                "GET /api/stream?fps=60 HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: {key}\r\n\
                 Sec-WebSocket-Version: 13\r\n\r\n"
            )
            .unwrap();
            let mut ws_reader = BufReader::new(ws_sock);
            loop {
                let mut line = String::new();
                ws_reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line == "\n" {
                    break;
                }
            }

            let (op0, raw0) = read_server_ws_frame(&mut ws_reader);
            assert_eq!(op0, OPCODE_BINARY);
            let h0 = decode_binary_frame(&raw0).expect("v2 keyframe");
            assert_eq!(h0.kind, FRAME_FLAG_PNG_KEYFRAME);
            let (ws_ptr0, ws_shown0) = h0.pointer.expect("pointer extension");
            assert!(!ws_shown0);
            let snap0 = bus.latest_frame().unwrap();
            assert_eq!(u64::from(h0.flip), snap0.flip);
            assert_eq!(ws_ptr0, snap0.pointer_pos);
            let mut prev_bgra = wire_png_to_bgra(h0.payload);
            let c0 = find_green_cursor_centroid(&prev_bgra, W, H);
            assert!(
                (c0.0 - ws_ptr0.0).abs() <= 1 && (c0.1 - ws_ptr0.1).abs() <= 1,
                "centroid {c0:?} vs ws_ptr0 {ws_ptr0:?}"
            );

            // Drive target move (240, 60) + transition SetCursorPos(160, 120) + move (70, 190).
            let mut saw_delta = false;
            let mut flip = 1u64;
            for target in [(240, 60), (70, 190)] {
                assert!(
                    bus.dispatch(Command::Act {
                        action: Action::Move {
                            x: target.0,
                            y: target.1,
                            glide_flips: 1,
                        },
                        auto_step: false,
                    })
                    .ok
                );
                for _ in 0..4 {
                    if recenter_every_flip {
                        hw = (160, 120);
                        host.put_pointer(160, 120);
                    }
                    for ev in host.poll_input(flip - 1) {
                        if let Input::Move { x, y } = ev {
                            let dx = x - hw.0;
                            let dy = y - hw.1;
                            hw = (x, y);
                            cur.0 = (cur.0 + f64::from(dx) / scale.0).clamp(0.0, f64::from(W - 1));
                            cur.1 = (cur.1 + f64::from(dy) / scale.1).clamp(0.0, f64::from(H - 1));
                        }
                    }
                    if !recenter_every_flip && target == (240, 60) {
                        // Simulate post-race SetCursorPos(160, 120) transition!
                        hw = (160, 120);
                        host.put_pointer(160, 120);
                    }
                    present(&mut host, flip, cur, hw);
                    flip += 1;
                    if bus.latest_frame().unwrap().bgra.as_slice() != prev_bgra.as_slice() {
                        let (op, frame) = read_server_ws_frame(&mut ws_reader);
                        assert_eq!(op, OPCODE_BINARY);
                        let hdr = decode_binary_frame(&frame).expect("v2 delta");
                        let (ws_ptr, ws_shown) = hdr.pointer.expect("pointer ext");
                        assert!(!ws_shown);
                        let reconstructed = if hdr.kind == FRAME_FLAG_XOR_RLE_DELTA {
                            saw_delta = true;
                            decode_xor_rle(&prev_bgra, hdr.payload).expect("decode xor")
                        } else {
                            wire_png_to_bgra(hdr.payload)
                        };
                        let centroid = find_green_cursor_centroid(&reconstructed, W, H);
                        let snap = bus.latest_frame().unwrap();
                        assert_eq!(u64::from(hdr.flip), snap.flip);
                        let png = render_frame_png(
                            &snap,
                            ObserveOptions {
                                cursor: true,
                                ..ObserveOptions::default()
                            },
                            snap.pointer_pos,
                        )
                        .unwrap();
                        let cross = png_crosshair_red_center(&png);
                        assert!(
                            (centroid.0 - ws_ptr.0).abs() <= 1
                                && (centroid.1 - ws_ptr.1).abs() <= 1
                        );
                        assert_eq!(ws_ptr, snap.pointer_pos);
                        assert_eq!(ws_ptr, bus.status().pointer_pos);
                        assert_eq!(cross, ws_ptr);
                        prev_bgra = reconstructed;
                    }
                    if bus.status().pending_actions == 0 {
                        break;
                    }
                }
                assert!(
                    (bus.status().pointer_pos.0 - target.0).abs() <= 1
                        && (bus.status().pointer_pos.1 - target.1).abs() <= 1,
                    "guide={} target={target:?} got={:?}",
                    guide.display_name,
                    bus.status().pointer_pos
                );
            }
            assert!(
                saw_delta,
                "expected at least one XOR+RLE delta frame for {}",
                guide.display_name
            );

            let ws_inner = ws_reader.get_mut();
            let _ = ws_inner.write_all(&[0x80 | OPCODE_CLOSE, 0x80, 1, 2, 3, 4]);
            let _ = ws_inner.flush();
            drop(ws_reader);
            server.stop();
        }
    }

    #[test]
    fn sse_event_json_suppresses_stuck_loop_on_static_menu_idle_and_fires_when_agent_stuck() {
        use crate::control::observe::STUCK_LOOP_THRESHOLD;
        use crate::control::timeline::Action;
        use crate::host::{Button, FlipMeta, Frame, Host};

        let mut host = ControlHost::new(
            Headless::new(),
            ControlConfig {
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();
        let ctx = HttpContext::new(bus.clone(), &Title::STUB, None);

        let present_only = |h: &mut ControlHost<Headless>, flip: u64, tag: u8| {
            let mut pixels = vec![0u8; 64];
            pixels[0] = tag;
            pixels[4] = 0xff;
            h.present(&Frame {
                width: 4,
                height: 4,
                pitch: 16,
                pixels: &pixels,
            });
            h.observe_flip(&FlipMeta {
                flip,
                clock_nanos: flip * 16_000_000,
                cursor: flip * 100,
                instructions_passed: 0,
                pointer_shown: true,
                pointer_pos: (320, 240),
                route_active: false,
                route_remaining: 0,
            });
        };
        let step_flip = |h: &mut ControlHost<Headless>, flip: u64, tag: u8| {
            present_only(h, flip, tag);
            let _ = h.poll_input(flip);
        };
        let warnings = || -> Vec<String> {
            let (json, _, _) = format_sse_event_json(&ctx);
            parse_json(&json).unwrap().as_object().unwrap()["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect()
        };
        let click = Action::Click {
            x: Some(100),
            y: Some(100),
            button: Button::Left,
            hold_flips: 1,
            hold_ms: None,
            settle_flips: 1,
        };

        // 1. Static menu idle in live 1.0x (empty queue, no agent actions): suppressed even at 300 flips.
        let mut flip = 0u64;
        while flip <= 300 {
            step_flip(&mut host, flip, 1);
            flip += 1;
        }
        assert!(warnings().is_empty(), "idle static menu must not warn");

        // 2. Human Take Over (owner=shared, source="human") queued on static screen: suppressed!
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Shared)).ok);
        assert!(
            bus.dispatch_forced_with_source(
                Command::Act {
                    action: click.clone(),
                    auto_step: false,
                },
                false,
                Some("human"),
            )
            .ok
        );
        assert!(
            warnings().is_empty(),
            "Human Take Over queued action on static menu must not flash stuck_loop"
        );
        assert!(bus.dispatch(Command::ClearQueue).ok);
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Agent)).ok);

        // 3. Agent acted BEFORE frame changed to static menu:
        //    action drains, frame changes (streak resets to 0), then 200 static flips -> suppressed.
        assert!(
            bus.dispatch(Command::Act {
                action: click.clone(),
                auto_step: false,
            })
            .ok
        );
        for _ in 0..5 {
            step_flip(&mut host, flip, 1);
            flip += 1;
        }
        assert_eq!(bus.status().pending_actions, 0);
        // Transition to a new static screen (tag 2) and sit idle for 200 flips.
        for _ in 0..=200 {
            step_flip(&mut host, flip, 2);
            flip += 1;
        }
        assert!(
            warnings().is_empty(),
            "post-transition idle menu must not emit stuck_loop"
        );

        // 4. Agent acts right on transition flip K (act_flip == streak_start) and stays stuck 180 flips:
        //    must fire stuck_loop (no off-by-one miss)!
        present_only(&mut host, flip, 3);
        assert!(
            bus.dispatch(Command::Act {
                action: click.clone(),
                auto_step: false,
            })
            .ok
        );
        let _ = host.poll_input(flip);
        flip += 1;
        for _ in 0..STUCK_LOOP_THRESHOLD {
            step_flip(&mut host, flip, 3);
            flip += 1;
        }
        assert_eq!(bus.status().pending_actions, 0);
        let w = warnings();
        assert_eq!(w.len(), 1);
        assert!(w[0].starts_with("stuck_loop:"));

        // 5. ClearQueue clears driving state and suppresses stuck_loop immediately.
        assert!(bus.dispatch(Command::ClearQueue).ok);
        assert!(warnings().is_empty());

        // 6. Agent acts ON the unchanged static frame, drains its action, emits stuck_loop,
        //    then stops past STUCK_LOOP_THRESHOLD flips -> clears automatically.
        assert!(
            bus.dispatch(Command::Act {
                action: click.clone(),
                auto_step: false,
            })
            .ok
        );
        for _ in 0..5 {
            step_flip(&mut host, flip, 3);
            flip += 1;
        }
        assert_eq!(bus.status().pending_actions, 0);
        assert_eq!(warnings().len(), 1);
        for _ in 0..=STUCK_LOOP_THRESHOLD {
            step_flip(&mut host, flip, 3);
            flip += 1;
        }
        assert!(warnings().is_empty());

        // 7. While owner == Human, stuck_loop is suppressed even with queued actions.
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Human)).ok);
        assert!(
            bus.dispatch(Command::Act {
                action: click,
                auto_step: false,
            })
            .ok
        );
        assert!(warnings().is_empty());
    }

    #[test]
    fn watch_html_includes_collapsible_controls_goals_panel_human_source_and_preserves_viewport() {
        // Collapsible Controls & Goals reference drawer backed by /api/controls & /api/goals.
        assert!(WATCH_HTML.contains("id=\"refDrawer\""));
        assert!(WATCH_HTML.contains("Controls &amp; Goals"));
        assert!(WATCH_HTML.contains("/api/controls"));
        assert!(WATCH_HTML.contains("/api/goals"));

        // Human Take Over action feed styling & source: 'human' tagging.
        assert!(WATCH_HTML.contains(".src-human"));
        assert!(WATCH_HTML.contains("source: 'human'"));

        // Preserves #327 viewport sizing and ResizeObserver intact.
        assert!(WATCH_HTML.contains("box-sizing:content-box"));
        assert!(WATCH_HTML.contains("function fitViewport()"));
        assert!(WATCH_HTML.contains("ResizeObserver"));

        assert!(WATCH_HTML.contains(".viewport.takeover.hide-cursor #stage{cursor:none}"));
        assert!(WATCH_HTML.contains("function syncTakeoverCursor()"));
        assert!(WATCH_HTML.contains("function enqueueAct(body)"));
        assert!(!WATCH_HTML.contains("relCursor"));
    }
}
