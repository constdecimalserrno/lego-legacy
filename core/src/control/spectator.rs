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
use super::observe::STUCK_LOOP_THRESHOLD;
use super::ws::{
    encode_binary_frame, encode_xor_rle, is_websocket_upgrade, read_client_frame,
    write_handshake_response, write_ws_frame, WsClientFrame, FRAME_FLAG_PNG_KEYFRAME,
    FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG,
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
        if req.method == "GET" && matches!(req.path.as_str(), "/" | "/watch" | "/watch/") {
            serve_watch_html(stream)?;
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
        if req.method != "GET" {
            return Ok(false);
        }
        match req.path.as_str() {
            "/" | "/watch" | "/watch/" => {
                serve_watch_html(stream)?;
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

fn serve_watch_html(stream: &mut dyn io::Write) -> io::Result<()> {
    HttpResponse::raw(
        200,
        "text/html; charset=utf-8",
        WATCH_HTML.as_bytes().to_vec(),
    )
    .write_to(stream)
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
                        encode_binary_frame(
                            FRAME_FLAG_XOR_RLE_DELTA,
                            curr.flip,
                            curr.width,
                            curr.height,
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
        encode_binary_frame(
            FRAME_FLAG_PNG_KEYFRAME,
            curr.flip,
            curr.width,
            curr.height,
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
                    || p.frame_hash != curr.frame_hash)
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
    held_keys: Vec<u32>,
    pending_actions: usize,
    unchanged_streak: u64,
    agent_state: AgentState,
}

impl SseFingerprint {
    fn from_snapshot(status: &ControlStatus, agent_state: &AgentState) -> Self {
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
            held_keys: status.held_keys.clone(),
            pending_actions: status.pending_actions,
            unchanged_streak: status.diff.unchanged_streak,
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
            || self.agent_state != prev.agent_state
    }
}

/// Format one SSE `data:` JSON payload combining live status, `#296` telemetry,
/// `AgentState`, and stuck/uniform critic warnings.
#[must_use]
pub fn format_sse_event_json(ctx: &HttpContext) -> (String, ControlStatus, AgentState) {
    let status = ctx.bus.status();
    let agent = ctx.bus.agent_state();
    let report = ctx.bus.telemetry_report(None);
    let uniform = ctx.bus.latest_frame().is_some_and(|f| f.uniform);

    let mut warnings: Vec<String> = Vec::new();
    if uniform {
        warnings.push("uniform_frame: every pixel has the same color".to_owned());
    }
    if status.diff.unchanged_streak >= STUCK_LOOP_THRESHOLD {
        warnings.push(format!(
            "stuck_loop: frame unchanged for {} consecutive flips",
            status.diff.unchanged_streak
        ));
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
        let status = ctx.bus.status();
        let agent = ctx.bus.agent_state();
        let fp = SseFingerprint::from_snapshot(&status, &agent);
        let now = Instant::now();

        let should_push = match &prev_fp {
            None => true,
            Some(prev) if fp.urgent_change(prev) => true,
            Some(prev) if fp != *prev && now.duration_since(last_send) >= SSE_MIN_INTERVAL => true,
            _ => false,
        };

        if should_push {
            let (payload, _, _) = format_sse_event_json(ctx);
            write!(stream, "event: state\ndata: {payload}\n\n")?;
            stream.flush()?;
            prev_fp = Some(fp);
            last_send = Instant::now();
            last_heartbeat = last_send;
        } else if now.duration_since(last_heartbeat) >= KEEPALIVE_INTERVAL {
            stream.write_all(b": keepalive\n\n")?;
            stream.flush()?;
            last_heartbeat = Instant::now();
        }

        if status.finished {
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
        };

        let f0 = mk_snap(0, 16, 12, 0x10, 100.0);
        let (bytes0, kf0) = encode_stream_frame(None, &f0, 0);
        assert!(kf0);
        assert_eq!(bytes0[0], FRAME_FLAG_PNG_KEYFRAME);

        let f1 = mk_snap(1, 16, 12, 0x12, 5.0);
        let (bytes1, kf1) = encode_stream_frame(Some(&f0), &f1, 0);
        assert!(!kf1);
        assert_eq!(bytes1[0], FRAME_FLAG_XOR_RLE_DELTA);

        // Resize forces keyframe.
        let f_resize = mk_snap(2, 32, 24, 0x12, 5.0);
        let (bytes_r, kf_r) = encode_stream_frame(Some(&f1), &f_resize, 1);
        assert!(kf_r);
        assert_eq!(bytes_r[0], FRAME_FLAG_PNG_KEYFRAME);

        // Scene cut (> KEYFRAME_DIFF_PCT) forces keyframe.
        let f_cut = mk_snap(3, 16, 12, 0x80, 60.0);
        let (bytes_c, kf_c) = encode_stream_frame(Some(&f1), &f_cut, 1);
        assert!(kf_c);
        assert_eq!(bytes_c[0], FRAME_FLAG_PNG_KEYFRAME);

        // Periodic interval forces keyframe.
        let (bytes_i, kf_i) = encode_stream_frame(Some(&f0), &f1, KEYFRAME_INTERVAL);
        assert!(kf_i);
        assert_eq!(bytes_i[0], FRAME_FLAG_PNG_KEYFRAME);
    }
}
