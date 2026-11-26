//! Real-disc headless + live loopback `/api/stream` WebSocket v2 cursor
//! alignment proof for LEGO Racers (`SoftwareRelative`, #374).
//!
//! Runs retail Masterpiece `LEGORacers.exe` unflagged under
//! [`ControlHost<Headless>`] with a live loopback [`HttpServer`] serving
//! `/api/meta`, `/api/controls`, `/api/goals`, `/api/mode`, `/api/act`, and
//! `/api/stream?fps=60` WebSocket v2 (`FRAME_FLAG_HAS_POINTER`), proving across
//! Main Menu, `/watch` Human Take Over Circuit Select, Build Menu, and
//! post-race return to Main Menu that:
//! - `/api/meta` advertises `mouse_mode.kind == "software_relative"`,
//!   `virtual_span == [640, 480]`, `start_pos == [0, 0]`, and
//!   `recenter_every_flip == false`.
//! - DirectInput exclusive mouse acquisition (`DISCL_EXCLUSIVE`) keeps
//!   `pointer_shown == false` across all menus and after post-race return.
//! - The rendered `MENUDATA/ARROW.BMP` tip matches `status.pointer_pos`,
//!   `--cursor` crosshair PNG, and live WS v2 frame-locked pointer coordinates
//!   within `<= 2 px` on both axes.
//! - `/watch` Human Take Over `POST /api/act` (`source: "human"`) moves and
//!   clicks transition Main Menu -> Circuit Select -> Main Menu -> Build Menu
//!   on the first click with `<= 2 px` cursor alignment on every screen.
//!
//! Needs the player's own Masterpiece rip under `game-data/`. Without it the
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
use lego_racers::{GUIDE, LEGO_RACERS as RACERS};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const W: u32 = 640;
const H: u32 = 480;
const MAX_ERR_PX: i32 = 2;
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const RUN_BUDGET: u64 = 4_000_000_000;

fn scratch_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-cursor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test scratch dir");
    dir
}

/// Minimal pure-std PNG decoder for 8-bit RGB PNGs emitted by `core::png::encode`.
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

/// Locate the center `(cx, cy)` of `render_frame_png(..., cursor=true)`'s red
/// dot `[0, 0, 255]` flanked by bright cyan `[255, 255, 0]` arms (works even
/// when the pointer sits on screen corners like `(0, 0)` where two arms clip).
fn find_crosshair_center(raw_bgra: &[u8], annotated_png: &[u8]) -> (i32, i32) {
    let (w, h, bgra) = decode_png_to_bgra(annotated_png);
    assert_eq!((w, h), (W, H));
    let is_cyan_arm = |x: i32, y: i32| -> bool {
        if !(0..w as i32).contains(&x) || !(0..h as i32).contains(&y) {
            return false;
        }
        let i = ((y as u32 * w + x as u32) * 4) as usize;
        bgra[i..i + 3] != raw_bgra[i..i + 3]
            && bgra[i] >= 220
            && bgra[i + 1] >= 220
            && bgra[i + 2] <= 40
    };
    let mut matches = Vec::new();
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = ((y as u32 * w + x as u32) * 4) as usize;
            if bgra[i..i + 3] != raw_bgra[i..i + 3]
                && bgra[i + 2] >= 220
                && bgra[i + 1] <= 40
                && bgra[i] <= 40
            {
                let arms = [
                    is_cyan_arm(x - 4, y),
                    is_cyan_arm(x + 4, y),
                    is_cyan_arm(x, y - 4),
                    is_cyan_arm(x, y + 4),
                ]
                .into_iter()
                .filter(|&ok| ok)
                .count();
                if arms >= 2 {
                    matches.push((x, y));
                }
            }
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one --cursor crosshair red center, found {matches:?}"
    );
    matches[0]
}

/// Opaque pixel sample `(dx, dy, [b, g, r])` of `MENUDATA/ARROW.BMP` relative
/// to its top-left hotspot `(0, 0)`.
type ArrowTemplate = Vec<(i32, i32, [u8; 3])>;

/// Extract the exact opaque `ARROW.BMP` pixel template from the initial Main
/// Menu frame (arrow at `(0, 0)`) diffed against a frame where the arrow has
/// moved away from `(0..32, 0..32)` (exposing the static blue stud wall).
fn extract_arrow_template(arrow_at_origin: &[u8], origin_clear: &[u8]) -> ArrowTemplate {
    let mut tpl = Vec::new();
    for y in 0..32i32 {
        for x in 0..32i32 {
            let i = ((y as u32 * W + x as u32) * 4) as usize;
            let p = [
                arrow_at_origin[i],
                arrow_at_origin[i + 1],
                arrow_at_origin[i + 2],
            ];
            let bg = [origin_clear[i], origin_clear[i + 1], origin_clear[i + 2]];
            let diff = (i32::from(p[0]) - i32::from(bg[0])).abs()
                + (i32::from(p[1]) - i32::from(bg[1])).abs()
                + (i32::from(p[2]) - i32::from(bg[2])).abs();
            if diff > 24 {
                tpl.push((x, y, p));
            }
        }
    }
    assert!(
        tpl.len() >= 350,
        "expected >= 350 opaque ARROW.BMP pixels at (0,0), got {}",
        tpl.len()
    );
    tpl
}

/// Locate `MENUDATA/ARROW.BMP`'s hotspot `(x, y)` blindly across the entire
/// 640x480 framebuffer (coarse 2 px sweep + 1 px refinement) using the exact
/// extracted sprite template.
fn locate_arrow_tip(bgra: &[u8], tpl: &ArrowTemplate) -> (i32, i32, f64) {
    let stride = W as usize;
    let scan = |x0: i32, x1: i32, y0: i32, y1: i32, step: i32| -> (i32, i32, usize, usize) {
        let mut best = (0i32, 0i32, 0usize, 1usize);
        let mut y = y0.max(0);
        let y_end = y1.min(H as i32 - 1);
        let x_start = x0.max(0);
        let x_end = x1.min(W as i32 - 1);
        while y <= y_end {
            let mut x = x_start;
            while x <= x_end {
                let mut hits = 0usize;
                let mut total = 0usize;
                for &(dx, dy, [tb, tg, tr]) in tpl {
                    let px = x + dx;
                    let py = y + dy;
                    if (0..W as i32).contains(&px) && (0..H as i32).contains(&py) {
                        total += 1;
                        let i = (py as usize * stride + px as usize) * 4;
                        if (i32::from(bgra[i]) - i32::from(tb)).abs() <= 8
                            && (i32::from(bgra[i + 1]) - i32::from(tg)).abs() <= 8
                            && (i32::from(bgra[i + 2]) - i32::from(tr)).abs() <= 8
                        {
                            hits += 1;
                        }
                    }
                }
                if total >= tpl.len() / 2 && hits * best.3 > best.2 * total {
                    best = (x, y, hits, total);
                }
                x += step;
            }
            y += step;
        }
        best
    };

    let coarse = scan(0, W as i32 - 1, 0, H as i32 - 1, 2);
    let fine = scan(coarse.0 - 2, coarse.0 + 2, coarse.1 - 2, coarse.1 + 2, 1);
    (fine.0, fine.1, fine.2 as f64 / fine.3.max(1) as f64)
}

fn count_diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(pa, pb)| pa[..3] != pb[..3])
        .count()
}

fn http_request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("http connect");
    stream
        .set_read_timeout(Some(SYNC_TIMEOUT))
        .expect("http read timeout");
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
    stream.flush().expect("http flush");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("http read");
    let (head, resp_body) = resp.split_once("\r\n\r\n").expect("http split");
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("http status");
    (status, resp_body.to_owned())
}

fn write_masked_ws_frame(sock: &mut TcpStream, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    assert!(payload.len() <= 125);
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
        assert_eq!(got_accept.as_deref(), Some(expected_accept.as_str()));

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
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        if stopping_bg.load(Ordering::Relaxed) {
                            return;
                        }
                        continue;
                    }
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
                assert_eq!(hdr[1] & 0x80, 0, "server WS frames must be unmasked");
                let len7 = hdr[1] & 0x7f;
                let len = match len7 {
                    0..=125 => usize::from(len7),
                    126 => {
                        let mut ext = [0u8; 2];
                        reader.read_exact(&mut ext).expect("ws len16");
                        usize::from(u16::from_be_bytes(ext))
                    }
                    _ => {
                        let mut ext = [0u8; 8];
                        reader.read_exact(&mut ext).expect("ws len64");
                        u64::from_be_bytes(ext) as usize
                    }
                };
                let mut payload = vec![0u8; len];
                if len > 0 {
                    reader.read_exact(&mut payload).expect("ws payload");
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
                    other => panic!("unexpected WS opcode {other}"),
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
                    "timed out waiting for WS frame in {min_flip}..={max_flip} ptr={expected_ptr:?} shown={expected_shown} \
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

fn step_flips(bus: &ControlBus, flips: u64) {
    let resp = bus.dispatch_sync(Command::Step { flips }, Some(SYNC_TIMEOUT));
    assert!(resp.ok, "step {flips} failed: {:?}", resp.error);
}

fn press_key(bus: &ControlBus, key: &str) {
    let resp = bus.dispatch_sync(
        Command::Act {
            action: Action::KeyHold {
                key: key.to_owned(),
                hold_flips: 2,
                hold_ms: None,
            },
            auto_step: true,
        },
        Some(SYNC_TIMEOUT),
    );
    assert!(resp.ok, "key_hold {key} failed: {:?}", resp.error);
}

#[test]
fn real_disc_software_relative_cursor_aligns_headlessly_and_over_ws_across_menus_and_race() {
    if !testkit::rips(&RACERS.image) {
        return;
    }
    let scratch = scratch_dir();
    std::env::set_var("LEGO_LEGACY_RUNS", &scratch);

    // Unflagged boot route skipping skippable intro AVIs (hvscmp.avi via Space,
    // introcmp.avi via click) before DirectInput exclusive mouse acquisition.
    let boot_route = scratch.join("boot.route");
    std::fs::write(
        &boot_route,
        "#lego-legacy-route 1\n\
         flip 240 key Space down\n\
         flip 241 key Space up\n\
         flip 245 move 320 240\n\
         flip 246 down left\n\
         flip 247 up left\n",
    )
    .expect("write boot route");

    let mut control = ControlHost::new(
        Headless::new(),
        ControlConfig {
            title: RACERS.key,
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            start_paused: false,
            step_uncapped: true,
            pointer_speed: RACERS.pointer_speed,
            flip_cost: RACERS.flip_cost,
            guide: &GUIDE,
            ..ControlConfig::default()
        },
    );
    let bus = control.bus();

    // 1. Pre-boot initial state from GUIDE.mouse_mode = SoftwareRelative { (640, 480), (0, 0) }.
    let init_st = bus.status();
    assert_eq!(
        init_st.pointer_pos,
        (0, 0),
        "initial start_pos must be (0, 0)"
    );
    assert_eq!(init_st.hw_pointer_pos, (320, 240));
    assert!(!init_st.pointer_shown);

    let ctx = Arc::new(HttpContext::new(bus.clone(), &RACERS, None).with_guide(&GUIDE));
    let server = HttpServer::start("127.0.0.1:0", ctx).expect("loopback http start");
    let addr = server.addr();

    // 2. Verify /api/meta, /api/controls, /api/goals over HTTP.
    let (meta_code, meta_body) = http_request(addr, "GET", "/api/meta", None);
    assert_eq!(meta_code, 200);
    let meta = parse_json(&meta_body).expect("valid /api/meta JSON");
    let guide_json = meta.as_object().unwrap()["guide"].as_object().unwrap();
    let mm = guide_json["mouse_mode"].as_object().unwrap();
    assert_eq!(mm["kind"].as_str(), Some("software_relative"));
    assert_eq!(mm["recenter_every_flip"].as_bool(), Some(false));
    assert_eq!(
        mm["virtual_span"].as_array().unwrap()[0].as_u64(),
        Some(640)
    );
    assert_eq!(
        mm["virtual_span"].as_array().unwrap()[1].as_u64(),
        Some(480)
    );
    assert_eq!(mm["start_pos"].as_array().unwrap()[0].as_i64(), Some(0));
    assert_eq!(mm["start_pos"].as_array().unwrap()[1].as_i64(), Some(0));

    let (ctrl_code, ctrl_body) = http_request(addr, "GET", "/api/controls", None);
    assert_eq!(ctrl_code, 200);
    let ctrl_json = parse_json(&ctrl_body).expect("valid /api/controls JSON");
    assert_eq!(
        ctrl_json.as_object().unwrap()["escape_quits"].as_bool(),
        Some(false)
    );

    let (goals_code, goals_body) = http_request(addr, "GET", "/api/goals", None);
    assert_eq!(goals_code, 200);
    let goals_json = parse_json(&goals_body).expect("valid /api/goals JSON");
    assert_eq!(
        goals_json.as_object().unwrap()["goals"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    let data_root = scratch.join("data-root");
    let runner = thread::spawn(move || {
        let cli = Cli {
            headless: true,
            route: Some(boot_route),
            data_root: Some(data_root),
            ..Cli::default()
        };
        let (finish, _log, digest) =
            run::run_digested_with_guide(&RACERS, &GUIDE, &cli, &mut control, RUN_BUDGET)
                .expect("lego racers headless control run");
        control.mark_finished(&finish, &digest);
    });
    let _rig = TestRigGuard {
        bus: bus.clone(),
        runner: Some(runner),
        server: Some(server),
        scratch,
    };

    // Wait for boot route to finish, then step to Main Menu (flip ~555).
    let post_boot = bus.wait_until_paused(Some(SYNC_TIMEOUT));
    assert!(
        post_boot.paused && !post_boot.finished,
        "expected paused post-boot state: {post_boot:?}"
    );
    let to_menu = 555u64.saturating_sub(post_boot.flip.unwrap_or(0));
    if to_menu > 0 {
        step_flips(&bus, to_menu);
    }

    // 3. Initial Main Menu state: DISCL_EXCLUSIVE keeps pointer_shown == false, cursor at (0, 0).
    let mm_st = bus.status();
    assert_eq!(mm_st.pointer_pos, (0, 0), "Main Menu starts at (0, 0)");
    assert!(
        !mm_st.pointer_shown,
        "DISCL_EXCLUSIVE GUID_SysMouse must hide OS pointer (pointer_shown == false)"
    );
    let mm_init_snap = bus.latest_frame().expect("Main Menu init frame");
    assert_eq!(mm_init_snap.pointer_pos, (0, 0));
    assert!(!mm_init_snap.pointer_shown);

    let ws = WsClient::connect(addr);
    let (ws_f0, ws_bgra0) = ws.wait_for_matching_frame(
        &mm_init_snap.bgra,
        (0, 0),
        false,
        mm_init_snap.flip,
        mm_init_snap.flip,
    );
    assert_eq!(ws_f0, mm_init_snap.flip);

    // Move away once to (220, 140) so we can extract the exact opaque ARROW.BMP
    // template from the clean (0..32, 0..32) top-left corner.
    let resp = bus.dispatch_sync(
        Command::Act {
            action: Action::Move {
                x: 220,
                y: 140,
                glide_flips: 1,
            },
            auto_step: true,
        },
        Some(SYNC_TIMEOUT),
    );
    assert!(resp.ok, "initial template move failed: {:?}", resp.error);
    step_flips(&bus, 2);
    let clear_snap = bus.latest_frame().expect("post-move frame");
    let tpl = extract_arrow_template(&ws_bgra0, &clear_snap.bgra);

    let assert_aligned_target = |phase: &str, expected: (i32, i32), min_flip: u64| {
        let st = bus.status();
        let snap = bus.latest_frame().expect("target snap");
        assert_eq!(st.pointer_pos, expected, "{phase}: status.pointer_pos");
        assert_eq!(snap.pointer_pos, expected, "{phase}: snap.pointer_pos");
        assert!(
            !st.pointer_shown,
            "{phase}: status.pointer_shown must be false"
        );
        assert!(
            !snap.pointer_shown,
            "{phase}: snap.pointer_shown must be false"
        );

        let (ws_flip, ws_bgra) =
            ws.wait_for_matching_frame(&snap.bgra, expected, false, min_flip, snap.flip);
        let (tip_x, tip_y, ratio) = locate_arrow_tip(&ws_bgra, &tpl);
        let dx = (tip_x - expected.0).abs();
        let dy = (tip_y - expected.1).abs();

        let crosshair_png = render_frame_png(
            &snap,
            ObserveOptions {
                cursor: true,
                ..ObserveOptions::default()
            },
            snap.pointer_pos,
        )
        .expect("render --cursor PNG");
        let cross = find_crosshair_center(&snap.bgra, &crosshair_png);
        assert_eq!(cross, expected, "{phase}: --cursor crosshair center");

        eprintln!(
            "[#374 {phase}] flip={} ws_flip={ws_flip} expected={expected:?} tip=({tip_x},{tip_y}) \
             err=({dx},{dy})px match={ratio:.3} crosshair={cross:?}",
            snap.flip
        );
        assert!(
            ratio >= 0.80,
            "{phase}: ARROW.BMP template match ratio {ratio:.3} too low at ({tip_x}, {tip_y})"
        );
        assert!(
            dx <= MAX_ERR_PX && dy <= MAX_ERR_PX,
            "{phase}: ARROW.BMP tip ({tip_x}, {tip_y}) diverged from {expected:?} by ({dx},{dy})px"
        );
    };

    // Confirm (0, 0) initial frame and (220, 140) both match <= 2 px.
    let (init_x, init_y, init_ratio) = locate_arrow_tip(&ws_bgra0, &tpl);
    assert!(init_ratio >= 0.95 && init_x.abs() <= MAX_ERR_PX && init_y.abs() <= MAX_ERR_PX);
    assert_aligned_target("main-menu-1", (220, 140), mm_init_snap.flip + 1);

    // 4. Headless multi-target moves on Main Menu (including over 3D Redbeard).
    for (label, (tx, ty)) in [
        ("main-menu-2", (420, 260)),
        ("main-menu-3", (160, 380)),
        ("main-menu-4", (560, 100)),
    ] {
        let min_flip = bus.status().flip.unwrap_or(0) + 1;
        let resp = bus.dispatch_sync(
            Command::Act {
                action: Action::Move {
                    x: tx,
                    y: ty,
                    glide_flips: 1,
                },
                auto_step: true,
            },
            Some(SYNC_TIMEOUT),
        );
        assert!(resp.ok, "{label}: move failed: {:?}", resp.error);
        step_flips(&bus, 2);
        assert_aligned_target(label, (tx, ty), min_flip);
    }

    // 5. `/watch` Human Take Over (`owner: "shared"`, `source: "human"` over HTTP POST /api/act):
    //    move + first-click CIRCUIT RACE (107, 195) -> verify 3 targets on Circuit Select!
    let (mode_code, _) = http_request(addr, "POST", "/api/mode", Some(r#"{"owner":"shared"}"#));
    assert_eq!(mode_code, 200);

    let watch_move = |label: &str, tx: i32, ty: i32| {
        let min_flip = bus.status().flip.unwrap_or(0) + 1;
        let body = format!(
            r#"{{"action":"move","x":{tx},"y":{ty},"glide_flips":1,"source":"human","wait":true,"auto_step":true}}"#
        );
        let (code, text) = http_request(addr, "POST", "/api/act", Some(&body));
        assert_eq!(code, 200, "{label}: HTTP {code} {text}");
        assert_eq!(
            parse_json(&text).unwrap().as_object().unwrap()["ok"].as_bool(),
            Some(true)
        );
        step_flips(&bus, 2);
        assert_aligned_target(label, (tx, ty), min_flip);
    };

    let watch_click = |label: &str, tx: i32, ty: i32, settle_after: u64| -> usize {
        let before = bus.latest_frame().expect("before click frame");
        let body = format!(
            r#"{{"action":"click","x":{tx},"y":{ty},"button":"left","hold_flips":2,"source":"human","wait":true,"auto_step":true}}"#
        );
        let (code, text) = http_request(addr, "POST", "/api/act", Some(&body));
        assert_eq!(code, 200, "{label}: HTTP {code} {text}");
        assert_eq!(
            parse_json(&text).unwrap().as_object().unwrap()["ok"].as_bool(),
            Some(true)
        );
        step_flips(&bus, settle_after);
        let after = bus.latest_frame().expect("after click frame");
        count_diff_pixels(&before.bgra, &after.bgra)
    };

    let cs_min_flip = bus.status().flip.unwrap_or(0) + 1;
    let diff_cs = watch_click("watch-click-circuit-race", 107, 195, 60);
    assert!(
        diff_cs > 40_000,
        "first /watch Take Over click on CIRCUIT RACE (107, 195) must open Circuit Select (diff={diff_cs}px)"
    );
    assert_aligned_target("circuit-select-arrival", (107, 195), cs_min_flip);

    watch_move("circuit-select-1", 140, 240);
    watch_move("circuit-select-2", 260, 320);
    watch_move("circuit-select-3", 520, 120);

    let diff_mm = watch_click("watch-click-main-menu", 78, 435, 60);
    assert!(
        diff_mm > 40_000,
        "first /watch Take Over click on MAIN MENU (78, 435) must return to Main Menu (diff={diff_mm}px)"
    );

    let diff_build = watch_click("watch-click-build", 65, 135, 60);
    assert!(
        diff_build > 40_000,
        "first /watch Take Over click on BUILD (65, 135) must open Build Menu (diff={diff_build}px)"
    );
    watch_move("build-menu-1", 260, 200);
    watch_move("build-menu-2", 180, 340);

    let diff_mm2 = watch_click("watch-click-build-back", 78, 435, 60);
    assert!(
        diff_mm2 > 40_000,
        "first /watch Take Over click on Build MAIN MENU (78, 435) must return (diff={diff_mm2}px)"
    );

    // Post-race return continuity: enter Imperial Grand Prix via SINGLE RACE,
    // pause -> EXIT -> YES back to Main Menu, then re-verify (0, 0) reset and moves.
    let diff_sr = watch_click("watch-click-single-race", 101, 235, 80);
    assert!(
        diff_sr > 40_000,
        "first /watch Take Over click on SINGLE RACE (101, 235) must open Track Select (diff={diff_sr}px)"
    );
    for (key, settle) in [
        ("Return", 140),
        ("Return", 650),
        ("Escape", 22),
        ("Down", 10),
        ("Down", 10),
        ("Return", 100),
        ("Down", 10),
    ] {
        press_key(&bus, key);
        step_flips(&bus, settle);
    }
    let post_race_min_flip = bus.status().flip.unwrap_or(0) + 1;
    press_key(&bus, "Return");
    step_flips(&bus, 160);

    assert_aligned_target("post-race-reset", (0, 0), post_race_min_flip);
    watch_move("post-race-1", 200, 150);
    watch_move("post-race-2", 410, 310);
    watch_move("post-race-3", 150, 390);
    let diff_post_cs = watch_click("post-race-click-circuit", 107, 195, 60);
    assert!(
        diff_post_cs > 40_000,
        "post-race first /watch click on CIRCUIT RACE (107, 195) must open Circuit Select (diff={diff_post_cs}px)"
    );

    let (keyframes, deltas) = ws.counts();
    eprintln!("[#374] WS v2 stream stats: keyframes={keyframes} deltas={deltas}");
    assert!(keyframes >= 1, "expected >= 1 WS v2 PNG keyframe");
    assert!(deltas >= 5, "expected >= 5 WS v2 XOR+RLE delta frames");
}
