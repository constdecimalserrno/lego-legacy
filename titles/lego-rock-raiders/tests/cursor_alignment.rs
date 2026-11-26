//! Real-disc pointer-to-cursor alignment proof for LEGO Rock Raiders (#331).
//!
//! Runs retail `LegoRR.exe` under [`ControlHost<Headless>`] with a live
//! loopback [`HttpServer`] serving `/api/meta`, `/api/act`, and `/api/stream`
//! WebSocket v2 (`FRAME_FLAG_HAS_POINTER`), proving across Main Menu, Level
//! Select, and Training Mission 1 (`1. Moving Rock Raiders`) that:
//! - Initial pre-move state reports `pointer_pos == (320, 240)` and
//!   `pointer_shown == false`, with `/api/meta` declaring
//!   `mouse_mode.kind == "software_absolute"` and
//!   `requires_poll_release == false`.
//! - Moving the pointer to safe interior targets renders the DirectDraw
//!   software cursor whose detected tip matches `ControlStatus::pointer_pos`,
//!   WS v2 frame-locked `pointer_pos`, and `--cursor` crosshair overlay within
//!   `<= 2px`.
//! - Live `/watch`-style `/api/act` clicks transition Main Menu -> Level Select
//!   -> Training Mission 1 and accurately open the Priorities HUD panel.
//!
//! Needs the player's own Rock Raiders rip under `game-data/`. Without it the
//! test prints a skip line and passes unless `LEGO_LEGACY_REQUIRE_DISC=1`.

use lego_legacy_core::cli::Cli;
use lego_legacy_core::control::json::parse_json;
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::ws::{
    decode_binary_frame, decode_xor_rle, websocket_accept_key, FRAME_FLAG_PNG_KEYFRAME,
    FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG,
};
use lego_legacy_core::control::{
    Action, Command, ControlBus, ControlConfig, ControlHost, ExecMode, HttpContext, HttpServer,
    InputOwner,
};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::{inflate, run, testkit};
use lego_rock_raiders::{GUIDE, ROCK_RAIDERS};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const W: u32 = 640;
const H: u32 = 480;
const MAX_TIP_ERROR_PX: i32 = 2;
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);

fn scratch_dir(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("rr-cursor-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test scratch dir");
    dir
}

/// Minimal pure-std PNG decoder for 8-bit RGB PNGs produced by `core::png::encode`.
fn decode_png_to_bgra(png: &[u8]) -> (u32, u32, Vec<u8>) {
    assert!(png.len() >= 33, "truncated PNG ({} bytes)", png.len());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(&png[24..29], &[8, 2, 0, 0, 0]);

    let mut pos = 8usize;
    let mut zlib = Vec::new();
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        if &png[pos + 4..pos + 8] == b"IDAT" {
            zlib.extend_from_slice(&png[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    assert!(zlib.len() >= 6, "truncated zlib stream");
    let raw = inflate::inflate(&zlib[2..zlib.len() - 4]).expect("inflate PNG IDAT");
    let row_bytes = (w as usize) * 3;
    assert_eq!(raw.len(), (row_bytes + 1) * (h as usize));

    let mut rgb = Vec::with_capacity(row_bytes * (h as usize));
    for y in 0..h as usize {
        let line = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let filter = line[0];
        let data = &line[1..];
        match filter {
            0 => rgb.extend_from_slice(data),
            1 => {
                let row_start = rgb.len();
                for i in 0..row_bytes {
                    let left = if i >= 3 { rgb[row_start + i - 3] } else { 0 };
                    rgb.push(data[i].wrapping_add(left));
                }
            }
            other => panic!("unexpected PNG filter {other}"),
        }
    }

    let mut bgra = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for px in rgb.as_chunks::<3>().0 {
        bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    (w, h, bgra)
}

/// Locate the unique center `(cx, cy)` of `render_frame_png(..., cursor=true)`'s
/// bright red dot + cyan crosshair arms.
fn find_crosshair_center(png: &[u8]) -> (i32, i32) {
    let (w, h, bgra) = decode_png_to_bgra(png);
    let wi = w as i32;
    let hi = h as i32;
    let is_px = |x: i32, y: i32, bgr: [u8; 3]| -> bool {
        if x < 0 || y < 0 || x >= wi || y >= hi {
            return false;
        }
        let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
        bgra[i] == bgr[0] && bgra[i + 1] == bgr[1] && bgra[i + 2] == bgr[2]
    };
    let red = [0u8, 0, 255];
    let cyan = [255u8, 255, 0];
    let mut matches = Vec::new();
    for y in 0..hi {
        for x in 0..wi {
            if !is_px(x, y, red) {
                continue;
            }
            // Verify cyan crosshair arms at offsets ±5 and ±9 along both axes.
            let arm_hits = [
                (x + 5, y),
                (x - 5, y),
                (x, y + 5),
                (x, y - 5),
                (x + 9, y),
                (x - 9, y),
                (x, y + 9),
                (x, y - 9),
            ]
            .into_iter()
            .filter(|&(ax, ay)| is_px(ax, ay, cyan))
            .count();
            if arm_hits >= 6 {
                matches.push((x, y));
            }
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one --cursor crosshair center, found {matches:?}"
    );
    matches[0]
}

fn http_request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("http connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let payload = body.unwrap_or("");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\n\
         Host: localhost\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n\
         {payload}",
        payload.len()
    )
    .expect("http write");
    stream.flush().unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("http read");
    let (head, resp_body) = resp.split_once("\r\n\r\n").expect("http response split");
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("http status code");
    (status, resp_body.to_owned())
}

fn write_masked_ws_frame(sock: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    assert!(payload.len() <= 125, "control frame payload exceeds 125B");
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let mut buf = Vec::with_capacity(6 + payload.len());
    buf.push(0x80 | (opcode & 0x0f));
    buf.push(0x80 | (payload.len() as u8));
    buf.extend_from_slice(&mask);
    for (i, &b) in payload.iter().enumerate() {
        buf.push(b ^ mask[i & 3]);
    }
    sock.write_all(&buf)?;
    sock.flush()
}

#[derive(Default)]
struct WsState {
    last_bgra: Option<Vec<u8>>,
    last_flip: Option<u64>,
    last_pointer: Option<((i32, i32), bool)>,
    keyframes_seen: u32,
    deltas_seen: u32,
    closed: bool,
    error: Option<String>,
}

/// Background-draining RFC 6455 client for `/api/stream?fps=60` so TCP
/// backpressure never triggers the server's 2 s write timeout during multi-flip
/// transitions.
struct WsClient {
    shared: Arc<(Mutex<WsState>, Condvar)>,
    stopping: Arc<AtomicBool>,
    writer: TcpStream,
    reader_thread: Option<JoinHandle<()>>,
}

impl WsClient {
    fn connect(addr: SocketAddr) -> Self {
        let client_key = "dGhlIHNhbXBsZSBub25jZQ==";
        let expected_accept = websocket_accept_key(client_key);

        let mut sock = TcpStream::connect(addr).expect("ws connect");
        sock.set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        write!(
            sock,
            "GET /api/stream?fps=60 HTTP/1.1\r\n\
             Host: localhost\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {client_key}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .expect("ws upgrade write");
        sock.flush().unwrap();

        let mut reader = BufReader::new(sock);
        let mut status_line = String::new();
        reader.read_line(&mut status_line).expect("ws status line");
        assert!(
            status_line.contains("101"),
            "expected HTTP 101 upgrade, got {status_line:?}"
        );
        let mut got_accept = None;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("ws header line");
            if line == "\r\n" || line == "\n" || line.is_empty() {
                break;
            }
            if let Some((name, val)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                    got_accept = Some(val.trim().to_owned());
                }
            }
        }
        assert_eq!(
            got_accept.as_deref(),
            Some(expected_accept.as_str()),
            "Sec-WebSocket-Accept mismatch"
        );

        let writer = reader.get_ref().try_clone().expect("clone ws tcp stream");
        let mut pong_writer = reader.get_ref().try_clone().expect("clone ws pong stream");
        let shared = Arc::new((Mutex::new(WsState::default()), Condvar::new()));
        let shared_bg = Arc::clone(&shared);
        let stopping = Arc::new(AtomicBool::new(false));
        let stopping_bg = Arc::clone(&stopping);

        let reader_thread = thread::spawn(move || {
            let mut prev_bgra: Option<Vec<u8>> = None;
            loop {
                let mut hdr = [0u8; 2];
                match reader.read_exact(&mut hdr) {
                    Ok(()) => {}
                    Err(e) => {
                        let (lock, cv) = &*shared_bg;
                        let mut st = lock.lock().unwrap();
                        st.closed = true;
                        if !stopping_bg.load(Ordering::Relaxed) {
                            st.error = Some(format!("ws read header: {e}"));
                        }
                        cv.notify_all();
                        return;
                    }
                }
                let opcode = hdr[0] & 0x0f;
                let masked = (hdr[1] & 0x80) != 0;
                if masked {
                    let (lock, cv) = &*shared_bg;
                    let mut st = lock.lock().unwrap();
                    st.closed = true;
                    st.error = Some("server sent masked WS frame (RFC 6455 violation)".to_owned());
                    cv.notify_all();
                    return;
                }
                let len7 = hdr[1] & 0x7f;
                let len = match len7 {
                    0..=125 => usize::from(len7),
                    126 => {
                        let mut ext = [0u8; 2];
                        if let Err(e) = reader.read_exact(&mut ext) {
                            let (lock, cv) = &*shared_bg;
                            let mut st = lock.lock().unwrap();
                            st.closed = true;
                            if !stopping_bg.load(Ordering::Relaxed) {
                                st.error = Some(format!("ws read len16: {e}"));
                            }
                            cv.notify_all();
                            return;
                        }
                        usize::from(u16::from_be_bytes(ext))
                    }
                    _ => {
                        let mut ext = [0u8; 8];
                        if let Err(e) = reader.read_exact(&mut ext) {
                            let (lock, cv) = &*shared_bg;
                            let mut st = lock.lock().unwrap();
                            st.closed = true;
                            if !stopping_bg.load(Ordering::Relaxed) {
                                st.error = Some(format!("ws read len64: {e}"));
                            }
                            cv.notify_all();
                            return;
                        }
                        u64::from_be_bytes(ext) as usize
                    }
                };
                let mut payload = vec![0u8; len];
                if len > 0 {
                    if let Err(e) = reader.read_exact(&mut payload) {
                        let (lock, cv) = &*shared_bg;
                        let mut st = lock.lock().unwrap();
                        st.closed = true;
                        if !stopping_bg.load(Ordering::Relaxed) {
                            st.error = Some(format!("ws read payload: {e}"));
                        }
                        cv.notify_all();
                        return;
                    }
                }

                match opcode {
                    OPCODE_BINARY => {
                        let hdr = decode_binary_frame(&payload).expect("valid v2 binary frame");
                        assert_eq!((u32::from(hdr.width), u32::from(hdr.height)), (W, H));
                        let (ptr, shown) = hdr.pointer.expect("v2 pointer extension present");
                        let (bgra, is_keyframe) = match hdr.kind {
                            FRAME_FLAG_PNG_KEYFRAME => {
                                let (pw, ph, buf) = decode_png_to_bgra(hdr.payload);
                                assert_eq!((pw, ph), (W, H));
                                (buf, true)
                            }
                            FRAME_FLAG_XOR_RLE_DELTA => {
                                let prev = prev_bgra.as_ref().expect("delta preceded by keyframe");
                                (
                                    decode_xor_rle(prev, hdr.payload)
                                        .expect("decode XOR+RLE delta"),
                                    false,
                                )
                            }
                            other => panic!("unexpected WS binary frame kind {other}"),
                        };
                        prev_bgra = Some(bgra.clone());
                        let (lock, cv) = &*shared_bg;
                        let mut st = lock.lock().unwrap();
                        if is_keyframe {
                            st.keyframes_seen += 1;
                        } else {
                            st.deltas_seen += 1;
                        }
                        st.last_bgra = Some(bgra);
                        st.last_flip = Some(u64::from(hdr.flip));
                        st.last_pointer = Some((ptr, shown));
                        cv.notify_all();
                    }
                    OPCODE_PING => {
                        // RFC 6455 §5.5.3: echo ping application data in masked Pong.
                        let _ = write_masked_ws_frame(&mut pong_writer, OPCODE_PONG, &payload);
                    }
                    OPCODE_PONG => {}
                    OPCODE_CLOSE => {
                        let (lock, cv) = &*shared_bg;
                        let mut st = lock.lock().unwrap();
                        st.closed = true;
                        cv.notify_all();
                        return;
                    }
                    other => {
                        let (lock, cv) = &*shared_bg;
                        let mut st = lock.lock().unwrap();
                        st.closed = true;
                        st.error = Some(format!("unexpected WS opcode {other}"));
                        cv.notify_all();
                        return;
                    }
                }
            }
        });

        Self {
            shared,
            stopping,
            writer,
            reader_thread: Some(reader_thread),
        }
    }

    /// Wait until the reconstructed WS stream matches `expected_bgra`,
    /// `expected_ptr`, and `expected_shown` at a flip in `min_flip..=max_flip`
    /// (accounting for `serve_websocket_stream`'s deduplication of identical frames).
    fn wait_for_matching_frame(
        &self,
        expected_bgra: &[u8],
        expected_ptr: (i32, i32),
        expected_shown: bool,
        min_flip: u64,
        max_flip: u64,
    ) -> (u64, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let (lock, cv) = &*self.shared;
        let mut st = lock.lock().unwrap();
        loop {
            if let Some(err) = &st.error {
                panic!("WS reader failed: {err}");
            }
            if let (Some(f), Some(bgra), Some((ptr, shown))) =
                (st.last_flip, st.last_bgra.as_ref(), st.last_pointer)
            {
                if f >= min_flip
                    && f <= max_flip
                    && ptr == expected_ptr
                    && shown == expected_shown
                    && bgra.as_slice() == expected_bgra
                {
                    return (f, bgra.clone());
                }
            }
            assert!(
                !st.closed,
                "WS closed before matching frame in {min_flip}..={max_flip} (last_flip={:?}, last_ptr={:?})",
                st.last_flip, st.last_pointer
            );
            let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                panic!(
                    "timed out waiting for WS frame in {min_flip}..={max_flip} ptr={expected_ptr:?} \
                     (last_flip={:?}, last_ptr={:?})",
                    st.last_flip, st.last_pointer
                );
            };
            let (guard, _) = cv.wait_timeout(st, rem).unwrap();
            st = guard;
        }
    }

    fn counts(&self) -> (u32, u32) {
        let st = self.shared.0.lock().unwrap();
        (st.keyframes_seen, st.deltas_seen)
    }
}

impl Drop for WsClient {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let _ = write_masked_ws_frame(&mut self.writer, OPCODE_CLOSE, &[]);
        let _ = self.writer.shutdown(Shutdown::Both);
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

/// Ensures background emulator, HTTP server, and scratch directory clean up even on panic.
struct TestRigGuard {
    bus: ControlBus,
    runner: Option<JoinHandle<()>>,
    server: Option<HttpServer>,
    scratch: PathBuf,
}

impl Drop for TestRigGuard {
    fn drop(&mut self) {
        let _ = self.bus.dispatch(Command::Close { force: true });
        if let Some(runner) = self.runner.take() {
            let _ = runner.join();
        }
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

struct CursorCluster {
    tip: (i32, i32),
    pixels: usize,
}

/// Detect the rendered Rock Raiders software cursor tip `(min_x, min_y)` from
/// the largest 8-connected cluster of newly changed pixels in `curr` vs `prev`
/// inside a wide 120x120 region around `approx_target` (excluding `prev_pos`'s
/// erase footprint).
fn detect_cursor_tip(
    prev: &[u8],
    curr: &[u8],
    prev_pos: (i32, i32),
    approx_target: (i32, i32),
) -> CursorCluster {
    let x0 = (approx_target.0 - 40).clamp(0, W as i32 - 1);
    let x1 = (approx_target.0 + 79).clamp(0, W as i32 - 1);
    let y0 = (approx_target.1 - 40).clamp(0, H as i32 - 1);
    let y1 = (approx_target.1 + 79).clamp(0, H as i32 - 1);
    let bw = (x1 - x0 + 1) as usize;
    let bh = (y1 - y0 + 1) as usize;

    let mut mask = vec![false; bw * bh];
    for y in y0..=y1 {
        for x in x0..=x1 {
            // Skip the previous cursor's erase footprint (~36x36 down-right from prev_pos).
            if x >= prev_pos.0 - 2
                && x <= prev_pos.0 + 36
                && y >= prev_pos.1 - 2
                && y <= prev_pos.1 + 36
            {
                continue;
            }
            let idx = ((y as usize) * (W as usize) + (x as usize)) * 4;
            if prev[idx..idx + 3] != curr[idx..idx + 3] {
                mask[(y - y0) as usize * bw + (x - x0) as usize] = true;
            }
        }
    }

    let mut visited = vec![false; bw * bh];
    let mut best: Option<CursorCluster> = None;
    let mut stack = Vec::new();
    for sy in 0..bh {
        for sx in 0..bw {
            let start = sy * bw + sx;
            if !mask[start] || visited[start] {
                continue;
            }
            visited[start] = true;
            stack.push((sx, sy));
            let mut min_x = i32::MAX;
            let mut min_y = i32::MAX;
            let mut count = 0usize;
            while let Some((cx, cy)) = stack.pop() {
                let gx = x0 + cx as i32;
                let gy = y0 + cy as i32;
                min_x = min_x.min(gx);
                min_y = min_y.min(gy);
                count += 1;
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = cx as isize + dx;
                        let ny = cy as isize + dy;
                        if nx >= 0 && nx < bw as isize && ny >= 0 && ny < bh as isize {
                            let ni = (ny as usize) * bw + (nx as usize);
                            if mask[ni] && !visited[ni] {
                                visited[ni] = true;
                                stack.push((nx as usize, ny as usize));
                            }
                        }
                    }
                }
            }
            if best.as_ref().is_none_or(|b| count > b.pixels) {
                best = Some(CursorCluster {
                    tip: (min_x, min_y),
                    pixels: count,
                });
            }
        }
    }

    let cluster = best.unwrap_or(CursorCluster {
        tip: (0, 0),
        pixels: 0,
    });
    assert!(
        cluster.pixels >= 32,
        "expected >= 32 newly rendered cursor pixels near {approx_target:?} (prev={prev_pos:?}), \
         found {} (whole_diff={:.4})",
        cluster.pixels,
        fraction_diff(prev, curr)
    );
    cluster
}

fn step_flips(bus: &ControlBus, flips: u64) {
    let resp = bus.dispatch_sync(Command::Step { flips }, Some(SYNC_TIMEOUT));
    assert!(resp.ok, "step {flips} failed: {:?}", resp.error);
}

fn press_key(bus: &ControlBus, key: &str, hold_flips: u64) {
    let resp = bus.dispatch_sync(
        Command::Act {
            action: Action::KeyHold {
                key: key.to_owned(),
                hold_flips,
                hold_ms: None,
            },
            auto_step: true,
        },
        Some(SYNC_TIMEOUT),
    );
    assert!(resp.ok, "key_hold {key} failed: {:?}", resp.error);
}

fn move_to(bus: &ControlBus, x: i32, y: i32) {
    let resp = bus.dispatch_sync(
        Command::Act {
            action: Action::Move {
                x,
                y,
                glide_flips: 1,
            },
            auto_step: true,
        },
        Some(SYNC_TIMEOUT),
    );
    assert!(resp.ok, "move ({x}, {y}) failed: {:?}", resp.error);
    // Let LegoRR.exe render at least 2 frames at the new pointer position.
    step_flips(bus, 2);
}

fn fraction_diff(a: &[u8], b: &[u8]) -> f64 {
    let total = (W as usize) * (H as usize);
    let mut diff = 0usize;
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0.iter()) {
        if pa[0..3] != pb[0..3] {
            diff += 1;
        }
    }
    diff as f64 / total as f64
}

fn verify_targets(
    bus: &ControlBus,
    ws: &WsClient,
    phase: &str,
    prev_bgra: &mut Vec<u8>,
    prev_pos: &mut (i32, i32),
    targets: &[(i32, i32)],
) {
    for &(tx, ty) in targets {
        let min_flip = bus.status().flip.unwrap_or(0);
        move_to(bus, tx, ty);
        let st = bus.status();
        let snap = bus.latest_frame().expect("target snap");
        assert_eq!(
            st.pointer_pos,
            (tx, ty),
            "{phase}: status.pointer_pos mismatch for target ({tx}, {ty})"
        );
        assert_eq!(
            snap.pointer_pos,
            (tx, ty),
            "{phase}: snap.pointer_pos mismatch for target ({tx}, {ty})"
        );
        assert!(!st.pointer_shown, "{phase}: pointer_shown must stay false");
        assert!(
            !snap.pointer_shown,
            "{phase}: snap.pointer_shown must stay false"
        );

        // Wait for live WS v2 frame matching snap.bgra, snap.pointer_pos, and pointer_shown=false.
        let (ws_flip, ws_bgra) =
            ws.wait_for_matching_frame(&snap.bgra, snap.pointer_pos, false, min_flip, snap.flip);

        // Since ws_bgra == snap.bgra bit-for-bit and ws_ptr == status.pointer_pos,
        // detecting the cursor tip from ws_bgra simultaneously proves headless and WS alignment.
        let cursor = detect_cursor_tip(prev_bgra, &ws_bgra, *prev_pos, (tx, ty));
        let tip_err = (cursor.tip.0 - st.pointer_pos.0)
            .abs()
            .max((cursor.tip.1 - st.pointer_pos.1).abs());

        // `--cursor` PNG crosshair center vs reported pointer_pos.
        let crosshair_png = render_frame_png(
            &snap,
            ObserveOptions {
                cursor: true,
                ..ObserveOptions::default()
            },
            snap.pointer_pos,
        )
        .expect("render --cursor PNG");
        let cross_center = find_crosshair_center(&crosshair_png);
        assert_eq!(
            cross_center, st.pointer_pos,
            "{phase}: --cursor crosshair center {cross_center:?} != pointer_pos {:?}",
            st.pointer_pos
        );

        eprintln!(
            "[#331 {phase}] target=({tx},{ty}) flip={} ws_flip={ws_flip} pos={:?} \
             tip={:?} (err={}px, px={}) crosshair={:?}",
            snap.flip, st.pointer_pos, cursor.tip, tip_err, cursor.pixels, cross_center
        );

        assert!(
            tip_err <= MAX_TIP_ERROR_PX,
            "{phase}: rendered cursor tip {:?} diverged from pointer_pos {:?} by {tip_err}px (> {MAX_TIP_ERROR_PX}px)",
            cursor.tip,
            st.pointer_pos
        );

        *prev_bgra = ws_bgra;
        *prev_pos = (tx, ty);
    }
}

#[test]
fn real_disc_software_absolute_cursor_aligns_headlessly_and_over_ws() {
    if !testkit::rips(&ROCK_RAIDERS.image) {
        return;
    }
    let scratch = scratch_dir("ws");
    std::env::set_var("LEGO_LEGACY_RUNS", &scratch);

    let mut control = ControlHost::new(
        Headless::new(),
        ControlConfig {
            title: ROCK_RAIDERS.key,
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            step_uncapped: true,
            pointer_speed: ROCK_RAIDERS.pointer_speed,
            flip_cost: ROCK_RAIDERS.flip_cost,
            guide: &GUIDE,
            ..ControlConfig::default()
        },
    );
    let bus = control.bus();

    // 1. Pre-boot initial state from GUIDE.mouse_mode = SoftwareAbsolute { requires_poll_release: false }.
    let init_st = bus.status();
    assert_eq!(init_st.pointer_pos, (320, 240));
    assert!(!init_st.pointer_shown);

    let ctx = Arc::new(HttpContext::new(bus.clone(), &ROCK_RAIDERS, None).with_guide(&GUIDE));
    let server = HttpServer::start("127.0.0.1:0", ctx).expect("loopback http start");
    let addr = server.addr();

    // Verify /api/meta advertises software_absolute with requires_poll_release=false.
    let (code, meta_body) = http_request(addr, "GET", "/api/meta", None);
    assert_eq!(code, 200);
    let meta_json = parse_json(&meta_body).expect("valid /api/meta JSON");
    let mm = meta_json.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    assert_eq!(mm["kind"].as_str(), Some("software_absolute"));
    assert_eq!(mm["requires_poll_release"].as_bool(), Some(false));

    // Launch retail LegoRR.exe on a background thread with a fresh --data-root.
    let data_root = scratch.join("data-root");
    let runner = thread::spawn(move || {
        let cli = Cli {
            headless: true,
            data_root: Some(data_root),
            ..Cli::default()
        };
        let (finish, _log, digest) =
            run::run_digested_with_guide(&ROCK_RAIDERS, &GUIDE, &cli, &mut control, 12_000_000_000)
                .expect("rock raiders headless control run");
        control.mark_finished(&finish, &digest);
    });
    let _rig = TestRigGuard {
        bus: bus.clone(),
        runner: Some(runner),
        server: Some(server),
        scratch,
    };

    // Wait for first presented flip pause.
    let boot_st = bus.wait_until_paused(Some(Duration::from_secs(30)));
    assert!(
        boot_st.paused && boot_st.flip.is_some(),
        "failed to reach initial flip pause: finished={} outcome={:?}",
        boot_st.finished,
        boot_st.outcome
    );

    // 2. Let `lmi640.avi` finish (~285 flips), skip `intro.avi` (3821 frames)
    //    with `Escape` once it starts (~flip 350), then step through loading
    //    (~468 flips) until the Main Menu settles.
    step_flips(&bus, 350);
    press_key(&bus, "Escape", 3);
    step_flips(&bus, 40);

    let loading_frame = bus.latest_frame().expect("loading frame").bgra.clone();
    let mut reached_menu = false;
    let mut static_runs = 0u32;
    for _ in 0..80 {
        step_flips(&bus, 15);
        let st = bus.status();
        let snap = bus.latest_frame().expect("frame");
        let fd = fraction_diff(&loading_frame, &snap.bgra);
        if !st.pointer_shown && fd > 0.35 && snap.diff.diff_pct == 0.0 {
            static_runs += 1;
            if static_runs >= 2 {
                reached_menu = true;
                break;
            }
        } else {
            static_runs = 0;
        }
    }
    assert!(
        reached_menu,
        "failed to reach Main Menu after skipping intros"
    );

    // Settle on Main Menu, connect the background-draining WebSocket client,
    // and capture the initial v2 keyframe before our first explicit move.
    step_flips(&bus, 4);
    let menu_base_snap = bus.latest_frame().expect("menu baseline snap");
    assert!(
        !bus.status().pointer_shown,
        "Rock Raiders hides OS cursor on Main Menu"
    );
    let ws = WsClient::connect(addr);
    let (_, mut prev_bgra) = ws.wait_for_matching_frame(
        &menu_base_snap.bgra,
        menu_base_snap.pointer_pos,
        false,
        menu_base_snap.flip,
        menu_base_snap.flip,
    );
    let mut prev_pos = menu_base_snap.pointer_pos;

    // 3. Main Menu alignment across safe interior targets.
    //    First verify non-hover background targets, then hover `Start Game` at
    //    (365, 222) -> (320, 222) so the button highlight stays lit on both
    //    frames and the diff isolates the cursor sprite cleanly at (320, 222).
    verify_targets(
        &bus,
        &ws,
        "main-menu",
        &mut prev_bgra,
        &mut prev_pos,
        &[(200, 150), (460, 320), (160, 380)],
    );
    let hover_min_flip = bus.status().flip.unwrap_or(0);
    move_to(&bus, 365, 222);
    let hover_snap = bus.latest_frame().expect("hover snap");
    assert_eq!(hover_snap.pointer_pos, (365, 222));
    (_, prev_bgra) = ws.wait_for_matching_frame(
        &hover_snap.bgra,
        hover_snap.pointer_pos,
        false,
        hover_min_flip,
        hover_snap.flip,
    );
    prev_pos = hover_snap.pointer_pos;
    verify_targets(
        &bus,
        &ws,
        "main-menu-start-game",
        &mut prev_bgra,
        &mut prev_pos,
        &[(320, 222)],
    );

    // 4. `/watch`-style HTTP POST /api/act click on `Start Game` (320, 222) -> Level Select.
    let menu_before_click = prev_bgra.clone();
    let ls_start_flip = bus.status().flip.unwrap_or(0);
    let (act_code, act_body) = http_request(
        addr,
        "POST",
        "/api/act",
        Some(
            r#"{"action":"click","x":320,"y":222,"button":"left","hold_flips":2,"source":"human"}"#,
        ),
    );
    assert_eq!(act_code, 200, "POST /api/act click failed: {act_body}");
    step_flips(&bus, 95);
    let ls_snap = bus.latest_frame().expect("level select snap");
    assert!(
        fraction_diff(&menu_before_click, &ls_snap.bgra) > 0.35,
        "Start Game click did not transition from Main Menu to Level Select"
    );
    (_, prev_bgra) = ws.wait_for_matching_frame(
        &ls_snap.bgra,
        ls_snap.pointer_pos,
        false,
        ls_start_flip,
        ls_snap.flip,
    );
    prev_pos = ls_snap.pointer_pos;

    verify_targets(
        &bus,
        &ws,
        "level-select",
        &mut prev_bgra,
        &mut prev_pos,
        &[(240, 200), (420, 300)],
    );

    // 5. Click Training Mission 1 (`1. Moving Rock Raiders`) at (155, 160),
    //    wait past loading, dismiss Mission Brief with Space, pause gameplay
    //    (`P`) to freeze 3D cavern animation, and prove cursor alignment across
    //    the 3D map & HUD.
    let ls_before_click = prev_bgra.clone();
    let m1_start_flip = bus.status().flip.unwrap_or(0);
    let (act_code, act_body) = http_request(
        addr,
        "POST",
        "/api/act",
        Some(
            r#"{"action":"click","x":155,"y":160,"button":"left","hold_flips":2,"source":"human"}"#,
        ),
    );
    assert_eq!(
        act_code, 200,
        "POST /api/act mission click failed: {act_body}"
    );
    step_flips(&bus, 110);
    press_key(&bus, "Space", 2);
    step_flips(&bus, 25);
    press_key(&bus, "Space", 2);
    step_flips(&bus, 25);
    press_key(&bus, "P", 2);
    step_flips(&bus, 10);

    let m1_snap = bus.latest_frame().expect("training mission 1 snap");
    // Mission load calls SetCursorPos(320, 240); SoftwareAbsolute reconciles it.
    assert_eq!(m1_snap.pointer_pos, (320, 240));
    assert!(
        fraction_diff(&ls_before_click, &m1_snap.bgra) > 0.35,
        "Mission click did not leave Level Select"
    );
    (_, prev_bgra) = ws.wait_for_matching_frame(
        &m1_snap.bgra,
        m1_snap.pointer_pos,
        false,
        m1_start_flip,
        m1_snap.flip,
    );
    prev_pos = m1_snap.pointer_pos;

    verify_targets(
        &bus,
        &ws,
        "training-mission-1",
        &mut prev_bgra,
        &mut prev_pos,
        &[(260, 240), (450, 180), (495, 12)],
    );

    // Unpause (`P`) and click Top Panel Priorities icon `(495, 12)` via HTTP `/api/act`
    // to prove `/watch` Take Over HUD targeting opens the Priorities panel (`398..540, 50..260`).
    press_key(&bus, "P", 2);
    step_flips(&bus, 10);
    let before_prio = bus.latest_frame().expect("before prio").bgra.clone();
    let (act_code, act_body) = http_request(
        addr,
        "POST",
        "/api/act",
        Some(
            r#"{"action":"click","x":495,"y":12,"button":"left","hold_flips":2,"source":"human"}"#,
        ),
    );
    assert_eq!(
        act_code, 200,
        "POST /api/act priorities click failed: {act_body}"
    );
    step_flips(&bus, 25);
    let after_prio = bus.latest_frame().expect("after prio").bgra.clone();
    let mut prio_diff = 0usize;
    for y in 60usize..240 {
        for x in 400usize..530 {
            let i = (y * (W as usize) + x) * 4;
            if before_prio[i..i + 3] != after_prio[i..i + 3] {
                prio_diff += 1;
            }
        }
    }
    eprintln!("[#331 training-mission-1 priorities-click] changed_px_in_panel={prio_diff}");
    assert!(
        prio_diff > 2000,
        "clicking Priorities HUD icon (495, 12) should open Priorities panel (only {prio_diff} px changed)"
    );

    let (keyframes, deltas) = ws.counts();
    assert!(
        keyframes >= 1 && deltas >= 1,
        "expected both WS PNG keyframes and XOR+RLE deltas (keyframes={keyframes}, deltas={deltas})"
    );
}
