//! LEGO Island pointer-to-cursor alignment & live `/watch` WebSocket v2 proof (#328).
//!
//! Disc-gated integration test booting LEGO Island through the real `--control`
//! run seam (`run::run_with_guide` + live loopback `HttpServer`) and proving:
//! 1. Blank OS cursor (`SetCursor(0)` during intro at flip 456) sets
//!    `cursor_blank == true` in `/api/status`, `/api/frame.json`, and `/api/stream`
//!    WS v2 headers, and suppresses the `--cursor` (`/api/frame.png?cursor=1`)
//!    crosshair overlay.
//! 2. Information Center before any mouse move reports exact `(320, 240)` across
//!    `status.pointer_pos`, `status.hw_pointer_pos`, `/api/win32` `input.pointer_pos`,
//!    WS v2 frame-locked pointer, and `--cursor` crosshair (`0px` error), with
//!    `cursor_blank == false`.
//! 3. Moves, wide & narrow `/watch` Take Over `stageCoords` clicks (quantized to
//!    real integer CSS pixels inside `fitViewport()`), and drags across
//!    Information Center targets (Registration Book `(322, 294)`, Green Tick
//!    `(519, 135)`, Pepper portrait `(322, 105)`) maintain exact `0px` alignment
//!    across headless status, Win32 state, WS v2 frame-locked pointer (including
//!    live XOR+RLE deltas), and `--cursor` crosshair while hitting each target.

use lego_island::{GUIDE, LEGO_ISLAND as ISLAND};
use lego_legacy_core::cli::Cli;
use lego_legacy_core::control::json::{parse_json, JsonValue};
use lego_legacy_core::control::ws::{
    decode_binary_frame, decode_xor_rle, websocket_accept_key, FRAME_FLAG_PNG_KEYFRAME,
    FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG,
};
use lego_legacy_core::control::ExecMode;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::inflate::inflate;
use lego_legacy_core::run::run_with_guide;
use lego_legacy_core::testkit;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const FRAME_W: u32 = 640;
const FRAME_H: u32 = 480;
const RUN_BUDGET: u64 = 8_000_000_000;
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);

fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    testkit::rips(&ISLAND.image)
}

struct CloseOnDrop(SocketAddr);
impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        let _ = try_http_request(
            self.0,
            "POST",
            "/api/mode",
            Some(r#"{"close":true,"force":true}"#),
        );
    }
}

struct CleanupDir(PathBuf);
impl Drop for CleanupDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn try_http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> std::io::Result<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_read_timeout(Some(SYNC_TIMEOUT))?;
    let payload = body.unwrap_or("");
    let body_headers = if body.is_some() {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        )
    } else {
        String::new()
    };
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{body_headers}Connection: close\r\n\r\n{payload}"
    )?;
    stream.flush()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no header split"))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "no status"))?;
    Ok((status, raw[split + 4..].to_vec()))
}

fn http_request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, Vec<u8>) {
    try_http_request(addr, method, path, body).expect("control http request")
}

fn get_json(addr: SocketAddr, path: &str) -> JsonValue {
    let (status, body) = http_request(addr, "GET", path, None);
    let text = String::from_utf8(body).expect("utf8 json");
    assert_eq!(status, 200, "GET {path} -> {status}: {text}");
    parse_json(&text).expect("valid json")
}

fn post_json(addr: SocketAddr, path: &str, body: &str) -> JsonValue {
    let (status, resp) = http_request(addr, "POST", path, Some(body));
    let text = String::from_utf8(resp).expect("utf8 json");
    assert_eq!(status, 200, "POST {path} -> {status}: {text}");
    parse_json(&text).expect("valid json")
}

fn step_flips(addr: SocketAddr, flips: u64) -> JsonValue {
    post_json(addr, "/api/step", &format!(r#"{{"flips":{flips}}}"#))
}

fn act(addr: SocketAddr, body: &str) {
    let resp = post_json(addr, "/api/act", body);
    assert_eq!(resp.as_object().unwrap()["ok"].as_bool(), Some(true));
}

fn json_xy(val: &JsonValue) -> (i32, i32) {
    let arr = val.as_array().expect("2-element coordinate array");
    assert_eq!(arr.len(), 2);
    (
        arr[0].as_i64().expect("x") as i32,
        arr[1].as_i64().expect("y") as i32,
    )
}

/// Minimal RGB PNG decoder matching `core::png::encode` (8-bit RGB, filter None/Sub).
fn decode_png_rgb(png: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&png[12..16], b"IHDR");
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    let mut pos = 8usize;
    let mut zlib = Vec::new();
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        if &png[pos + 4..pos + 8] == b"IDAT" {
            zlib.extend_from_slice(&png[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    let raw = inflate(&zlib[2..zlib.len() - 4]).expect("inflate PNG IDAT");
    let row_bytes = (w as usize) * 3;
    assert_eq!(raw.len(), (row_bytes + 1) * (h as usize));
    let mut rgb = Vec::with_capacity(row_bytes * (h as usize));
    for y in 0..h as usize {
        let line = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let data = &line[1..];
        match line[0] {
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
    (w, h, rgb)
}

fn png_to_bgra(png: &[u8]) -> Vec<u8> {
    let (w, h, rgb) = decode_png_rgb(png);
    assert_eq!((w, h), (FRAME_W, FRAME_H));
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for px in rgb.as_chunks::<3>().0 {
        bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    bgra
}

/// Locate the `--cursor` crosshair by its bright cyan arms (`rgb(0,255,255)`)
/// and red center dot (`rgb(255,0,0)`).
fn find_crosshair_center(png: &[u8]) -> Option<(i32, i32)> {
    let (w, h, rgb) = decode_png_rgb(png);
    let px = |x: i32, y: i32| -> Option<[u8; 3]> {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return None;
        }
        let i = ((y as u32 * w + x as u32) * 3) as usize;
        Some([rgb[i], rgb[i + 1], rgb[i + 2]])
    };
    let cyan = [0u8, 255, 255];
    let red = [255u8, 0, 0];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if px(x, y) != Some(red) {
                continue;
            }
            let arm_hits = [(-4, 0), (4, 0), (0, -4), (0, 4)]
                .into_iter()
                .filter(|(dx, dy)| px(x + dx, y + dy) == Some(cyan))
                .count();
            if arm_hits >= 2 {
                return Some((x, y));
            }
        }
    }
    None
}

struct WsFrame {
    kind: u8,
    flip: u64,
    pointer: (i32, i32),
    pointer_shown: bool,
    pointer_blank: bool,
    bgra: Vec<u8>,
}

struct WsClient {
    reader: BufReader<TcpStream>,
    prev_bgra: Option<Vec<u8>>,
    deltas_seen: usize,
}

impl WsClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut sock = TcpStream::connect(addr).expect("connect ws");
        sock.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        write!(
            sock,
            "GET /api/stream?fps=60 HTTP/1.1\r\n\
             Host: localhost\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap();
        sock.flush().unwrap();
        let mut reader = BufReader::new(sock);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            head.push_str(&line);
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        assert!(head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        let expected_accept = websocket_accept_key(key);
        assert!(head.contains(&format!("Sec-WebSocket-Accept: {expected_accept}\r\n")));
        Self {
            reader,
            prev_bgra: None,
            deltas_seen: 0,
        }
    }

    fn read_frame(&mut self) -> WsFrame {
        loop {
            let mut h = [0u8; 2];
            self.reader.read_exact(&mut h).expect("ws frame header");
            let opcode = h[0] & 0x0f;
            assert_eq!(h[1] & 0x80, 0, "server frames must be unmasked");
            let len7 = h[1] & 0x7f;
            let len: usize = match len7 {
                0..=125 => usize::from(len7),
                126 => {
                    let mut b = [0u8; 2];
                    self.reader.read_exact(&mut b).unwrap();
                    usize::from(u16::from_be_bytes(b))
                }
                _ => {
                    let mut b = [0u8; 8];
                    self.reader.read_exact(&mut b).unwrap();
                    usize::try_from(u64::from_be_bytes(b)).unwrap()
                }
            };
            let mut payload = vec![0u8; len];
            self.reader.read_exact(&mut payload).unwrap();
            if opcode == OPCODE_PING {
                let mask = [0x11u8, 0x22, 0x33, 0x44];
                let mut pong = vec![0x80 | OPCODE_PONG, 0x80 | (payload.len() as u8)];
                pong.extend_from_slice(&mask);
                for (i, b) in payload.iter().enumerate() {
                    pong.push(*b ^ mask[i & 3]);
                }
                let inner = self.reader.get_mut();
                let _ = inner.write_all(&pong);
                let _ = inner.flush();
                continue;
            }
            if opcode == OPCODE_PONG {
                continue;
            }
            assert_eq!(opcode, OPCODE_BINARY, "expected binary video frame");
            let hdr = decode_binary_frame(&payload).expect("v2 binary header");
            let (pointer, pointer_shown) =
                hdr.pointer.expect("WS v2 frame-locked pointer extension");
            assert_eq!(
                (u32::from(hdr.width), u32::from(hdr.height)),
                (FRAME_W, FRAME_H)
            );
            let bgra = if hdr.kind == FRAME_FLAG_PNG_KEYFRAME {
                png_to_bgra(hdr.payload)
            } else {
                assert_eq!(hdr.kind, FRAME_FLAG_XOR_RLE_DELTA);
                self.deltas_seen += 1;
                let prev = self.prev_bgra.as_ref().expect("keyframe before delta");
                decode_xor_rle(prev, hdr.payload).expect("decode XOR+RLE delta")
            };
            self.prev_bgra = Some(bgra.clone());
            return WsFrame {
                kind: hdr.kind,
                flip: u64::from(hdr.flip),
                pointer,
                pointer_shown,
                pointer_blank: hdr.pointer_blank,
                bgra,
            };
        }
    }

    fn read_until_matching(
        &mut self,
        target_flip: u64,
        expected_ptr: (i32, i32),
        expected_bgra: &[u8],
    ) -> WsFrame {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for WS frame at flip {target_flip} ptr {expected_ptr:?}"
            );
            let frame = self.read_frame();
            if frame.flip >= target_flip
                || (frame.pointer == expected_ptr && frame.bgra == expected_bgra)
            {
                return frame;
            }
        }
    }

    fn close(mut self) {
        let inner = self.reader.get_mut();
        let _ = inner.write_all(&[0x80 | OPCODE_CLOSE, 0x80, 0x11, 0x22, 0x33, 0x44]);
        let _ = inner.flush();
    }
}

/// Mirror `watch.html`'s `fitViewport()` + `stageCoords()` math using real
/// integer CSS pointer coordinates (`clientX`/`clientY`) inside a host
/// container `(wrap_w, wrap_h)` with `12px` padding and `2px` border.
fn watch_css_click_to_guest(wrap_w: f64, wrap_h: f64, target: (i32, i32)) -> (i32, i32) {
    let avail_w = (wrap_w - 24.0 - 4.0).max(1.0);
    let avail_h = (wrap_h - 24.0 - 4.0).max(1.0);
    let scale = (avail_w / f64::from(FRAME_W)).min(avail_h / f64::from(FRAME_H));
    let cw = (f64::from(FRAME_W) * scale).floor().max(1.0);
    let ch = (f64::from(FRAME_H) * scale).floor().max(1.0);
    let aspect_err = (cw / ch - (f64::from(FRAME_W) / f64::from(FRAME_H))).abs();
    assert!(
        aspect_err < 0.01,
        "viewport ({wrap_w}x{wrap_h} -> {cw}x{ch}) skewed aspect ratio: {aspect_err}"
    );
    // Browser MouseEvent reports integer CSS pixel coordinates inside rect.
    let css_x = ((f64::from(target.0) + 0.5) * cw / f64::from(FRAME_W))
        .floor()
        .clamp(0.0, cw - 1.0);
    let css_y = ((f64::from(target.1) + 0.5) * ch / f64::from(FRAME_H))
        .floor()
        .clamp(0.0, ch - 1.0);
    let gx = ((css_x * f64::from(FRAME_W) / cw).floor() as i32).clamp(0, FRAME_W as i32 - 1);
    let gy = ((css_y * f64::from(FRAME_H) / ch).floor() as i32).clamp(0, FRAME_H as i32 - 1);
    (gx, gy)
}

fn maybe_save_proof(name: &str, png: &[u8]) {
    if let Ok(dir) = std::env::var("LEGO_LEGACY_PROOF_DIR") {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(name), png);
    }
}

fn rgb_at(bgra: &[u8], x: usize, y: usize) -> [u8; 3] {
    let i = (y * FRAME_W as usize + x) * 4;
    [bgra[i + 2], bgra[i + 1], bgra[i]]
}

fn has_info_center_plants(bgra: &[u8]) -> bool {
    let left = rgb_at(bgra, 180, 342);
    let right = rgb_at(bgra, 472, 343);
    left[1] > left[0] && left[1] > left[2] && right[1] > right[0] && right[1] > right[2]
}

fn wait_for_frame(
    addr: SocketAddr,
    max_iters: usize,
    flips_per_iter: u64,
    label: &str,
    pred: impl Fn(&[u8], &JsonValue) -> bool,
) {
    for _ in 0..max_iters {
        let status = get_json(addr, "/api/status");
        let (_, raw_png) = http_request(addr, "GET", "/api/frame.png", None);
        let bgra = png_to_bgra(&raw_png);
        if pred(&bgra, &status) {
            return;
        }
        act(
            addr,
            r#"{"action":"key_hold","key":"Space","hold_flips":2,"wait":true,"auto_step":true}"#,
        );
        let _ = step_flips(addr, flips_per_iter);
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn lego_island_pointer_aligns_headlessly_and_over_watch_websocket() {
    if !scratch() {
        return;
    }

    let data_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("lego-island-cursor-328-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);
    std::fs::create_dir_all(&data_root).expect("fresh data root");
    let _cleanup_dir = CleanupDir(data_root.clone());

    let probe = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = probe.local_addr().expect("local addr");
    drop(probe);

    let cli = Cli {
        headless: true,
        control_http: Some(addr.to_string()),
        control_mode: Some(ExecMode::Step),
        data_root: Some(data_root.clone()),
        ..Cli::default()
    };

    let machine_thread = thread::spawn(move || {
        let mut host = Headless::new();
        run_with_guide(&ISLAND, &GUIDE, &cli, &mut host, RUN_BUDGET).expect("run LEGO Island")
    });

    let close_guard = CloseOnDrop(addr);

    // Wait for HTTP control server to come online and pause at flip 0.
    let deadline = Instant::now() + SYNC_TIMEOUT;
    loop {
        assert!(
            !machine_thread.is_finished(),
            "LEGO Island thread exited before flip 0 pause"
        );
        assert!(
            Instant::now() < deadline,
            "timed out waiting for flip 0 pause"
        );
        if let Ok((200, body)) = try_http_request(addr, "GET", "/api/status", None) {
            if let Ok(val) = parse_json(&String::from_utf8_lossy(&body)) {
                let st = val.as_object().unwrap()["status"].as_object().unwrap();
                if st["paused"].as_bool() == Some(true) && st["flip"].as_u64() == Some(0) {
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    }

    // Verify /api/meta reports AbsoluteKept.
    let meta = get_json(addr, "/api/meta");
    assert_eq!(
        meta.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
            .as_object()
            .unwrap()["kind"]
            .as_str(),
        Some("absolute_kept")
    );

    // --- Stage 1: Blank cursor during intro (flip 456 -> SetCursor(0)) ---
    let step_intro = step_flips(addr, 460);
    let st_intro = step_intro.as_object().unwrap()["status"]
        .as_object()
        .unwrap();
    assert_eq!(st_intro["flip"].as_u64(), Some(460));
    assert_eq!(json_xy(&st_intro["pointer_pos"]), (320, 240));
    assert_eq!(json_xy(&st_intro["hw_pointer_pos"]), (320, 240));
    assert_eq!(st_intro["pointer_shown"].as_bool(), Some(true));
    assert_eq!(
        st_intro["cursor_blank"].as_bool(),
        Some(true),
        "ISLE.EXE SetCursor(0) at flip 456 must mark cursor_blank=true"
    );
    assert_eq!(
        get_json(addr, "/api/frame.json?grid=4x4")
            .as_object()
            .unwrap()["cursor_blank"]
            .as_bool(),
        Some(true)
    );
    let win32_intro = get_json(addr, "/api/win32");
    let win32_intro_in = win32_intro.as_object().unwrap()["input"]
        .as_object()
        .unwrap();
    assert_eq!(json_xy(&win32_intro_in["pointer_pos"]), (320, 240));
    assert_eq!(win32_intro_in["cursor_handle"].as_str(), Some("0x0"));

    // Connect live WS client at flip 460: keyframe must carry blank cursor flag.
    let mut ws_intro_client = WsClient::connect(addr);
    let ws_intro = ws_intro_client.read_frame();
    assert_eq!(ws_intro.kind, FRAME_FLAG_PNG_KEYFRAME);
    assert_eq!(ws_intro.flip, 460);
    assert_eq!(ws_intro.pointer, (320, 240));
    assert!(ws_intro.pointer_shown);
    assert!(ws_intro.pointer_blank);
    ws_intro_client.close();

    // `/api/frame.png?cursor=1` must suppress crosshair while SetCursor(0) is active.
    let (png_st, intro_png) = http_request(addr, "GET", "/api/frame.png?cursor=1", None);
    assert_eq!(png_st, 200);
    assert_eq!(
        find_crosshair_center(&intro_png),
        None,
        "--cursor must not draw crosshair when ISLE.EXE sets blank cursor SetCursor(0)"
    );
    assert_eq!(png_to_bgra(&intro_png), ws_intro.bgra);
    maybe_save_proof("01-intro-blank-cursor.png", &intro_png);

    // --- Stage 2: Reach Information Center WITHOUT any mouse move ---
    wait_for_frame(addr, 8, 200, "Information Center", |bgra, status| {
        let st = status.as_object().unwrap()["status"].as_object().unwrap();
        has_info_center_plants(bgra) && st["cursor_blank"].as_bool() == Some(false)
    });

    let assert_aligned =
        |ws: Option<&mut WsClient>, expected: (i32, i32), label: &str| -> Vec<u8> {
            let status_json = get_json(addr, "/api/status");
            let st_obj = status_json.as_object().unwrap()["status"]
                .as_object()
                .unwrap();
            let flip = st_obj["flip"].as_u64().expect("flip");
            assert_eq!(
                json_xy(&st_obj["pointer_pos"]),
                expected,
                "{label}: status.pointer_pos"
            );
            assert_eq!(
                json_xy(&st_obj["hw_pointer_pos"]),
                expected,
                "{label}: status.hw_pointer_pos"
            );
            assert_eq!(
                st_obj["pointer_shown"].as_bool(),
                Some(true),
                "{label}: pointer_shown"
            );
            assert_eq!(
                st_obj["cursor_blank"].as_bool(),
                Some(false),
                "{label}: cursor_blank should be false"
            );

            let win32 = get_json(addr, "/api/win32");
            let w_in = win32.as_object().unwrap()["input"].as_object().unwrap();
            assert_eq!(
                json_xy(&w_in["pointer_pos"]),
                expected,
                "{label}: inspect_win32.input.pointer_pos"
            );
            assert_eq!(
                json_xy(&w_in["last_move"]),
                expected,
                "{label}: inspect_win32.input.last_move"
            );
            assert_ne!(
                w_in["cursor_handle"].as_str(),
                Some("0x0"),
                "{label}: non-blank OS cursor handle"
            );

            let (raw_code, raw_png) = http_request(addr, "GET", "/api/frame.png", None);
            assert_eq!(raw_code, 200);
            let raw_bgra = png_to_bgra(&raw_png);

            let ws_frame = if let Some(client) = ws {
                client.read_until_matching(flip, expected, &raw_bgra)
            } else {
                let mut oneshot = WsClient::connect(addr);
                let f = oneshot.read_frame();
                oneshot.close();
                f
            };
            assert!(ws_frame.flip <= flip, "{label}: WS flip within range");
            assert_eq!(
                ws_frame.pointer, expected,
                "{label}: WS v2 frame-locked pointer"
            );
            assert!(ws_frame.pointer_shown, "{label}: WS pointer_shown");
            assert!(!ws_frame.pointer_blank, "{label}: WS pointer_blank");
            assert_eq!(
                ws_frame.bgra, raw_bgra,
                "{label}: WS reconstructed framebuffer bit-exact"
            );

            let (code, png) = http_request(addr, "GET", "/api/frame.png?cursor=1", None);
            assert_eq!(code, 200);
            assert_eq!(
                find_crosshair_center(&png),
                Some(expected),
                "{label}: --cursor crosshair center"
            );
            png
        };

    // Information Center before any mouse move: exact (320, 240) everywhere!
    let init_png = assert_aligned(None, (320, 240), "info-center-before-first-move");
    maybe_save_proof("02-info-center-init-320x240.png", &init_png);

    // Let the Infomaniac step aside to uncover the blue Registration Book at (322, 294).
    wait_for_frame(
        addr,
        12,
        120,
        "blue Registration Book uncovered",
        |bgra, _| {
            let [r, _, b] = rgb_at(bgra, 322, 294);
            b > 100 && b > r.saturating_add(40)
        },
    );

    // --- Stage 3: Live streaming WS v2 across multi-target moves & drag ---
    // Keep one live WS stream open across consecutive moves so it delivers real
    // XOR+RLE delta frames (`deltas_seen > 0`) without sitting undrained across
    // multi-hundred-flip scene transitions.
    let mut ws = WsClient::connect(addr);
    let _ = ws.read_frame();
    for (label, target) in [
        ("move-left-arrow", (28, 310)),
        ("move-map-pizzeria", (312, 206)),
        ("move-quit-door", (590, 260)),
        ("move-reg-book", (322, 294)),
    ] {
        act(
            addr,
            &format!(
                r#"{{"action":"move","x":{},"y":{},"glide_flips":2,"wait":true,"auto_step":true}}"#,
                target.0, target.1
            ),
        );
        let png = assert_aligned(Some(&mut ws), target, label);
        if label == "move-reg-book" {
            maybe_save_proof("03-reg-book-target.png", &png);
        }
    }
    assert!(
        ws.deltas_seen > 0,
        "live /api/stream WS v2 must deliver XOR+RLE delta frames alongside keyframes"
    );
    ws.close();

    // --- Stage 4: Real integer CSS wide & narrow /watch Take Over clicks ---
    // Narrow 360x640 -> 332x249 content box (~1.93 guest px/CSS px -> <= 1px quantization error);
    // Wide 1280x900 -> 1168x876 content box (~0.55 guest px/CSS px -> 0px error).
    let reg_book_narrow = watch_css_click_to_guest(360.0, 640.0, (322, 294));
    assert!(
        (reg_book_narrow.0 - 322).abs() <= 1 && (reg_book_narrow.1 - 294).abs() <= 1,
        "narrow viewport click quantized unexpectedly far: {reg_book_narrow:?}"
    );
    act(
        addr,
        &format!(
            r#"{{"action":"click","x":{},"y":{},"hold_flips":2,"wait":true,"auto_step":true}}"#,
            reg_book_narrow.0, reg_book_narrow.1
        ),
    );
    wait_for_frame(addr, 16, 120, "Registration Book green tick", |bgra, _| {
        let [r, g, b] = rgb_at(bgra, 519, 135);
        g > 100 && g > r.saturating_add(30) && g > b.saturating_add(30)
    });

    act(
        addr,
        r#"{"action":"type_text","text":"LEGO","flip_gap":3,"wait":true,"auto_step":true}"#,
    );
    let _ = step_flips(addr, 20);
    let tick_wide = watch_css_click_to_guest(1280.0, 900.0, (519, 135));
    assert_eq!(tick_wide, (519, 135));
    act(
        addr,
        &format!(
            r#"{{"action":"click","x":{},"y":{},"hold_flips":2,"wait":true,"auto_step":true}}"#,
            tick_wide.0, tick_wide.1
        ),
    );
    let _ = assert_aligned(None, tick_wide, "click-reg-book-tick-wide");

    wait_for_frame(
        addr,
        16,
        120,
        "Information Center after signing",
        |bgra, _| has_info_center_plants(bgra),
    );
    assert!(
        data_root
            .join("profile/C/PROGRAM FILES/LEGO ISLAND/PLAYERS.GSI")
            .exists()
            || data_root
                .join("profile/C/PROGRAM FILES/LEGO ISLAND/Players.gsi")
                .exists(),
        "Registration Book + Tick click must persist Players.gsi"
    );

    // Narrow-viewport click on Pepper portrait (322, 105).
    let pepper_narrow = watch_css_click_to_guest(360.0, 640.0, (322, 105));
    assert!(
        (pepper_narrow.0 - 322).abs() <= 1 && (pepper_narrow.1 - 105).abs() <= 1,
        "narrow viewport click quantized unexpectedly far: {pepper_narrow:?}"
    );
    act(
        addr,
        &format!(
            r#"{{"action":"click","x":{},"y":{},"hold_flips":2,"wait":true,"auto_step":true}}"#,
            pepper_narrow.0, pepper_narrow.1
        ),
    );
    let pepper_png = assert_aligned(None, pepper_narrow, "click-pepper-portrait-narrow");
    maybe_save_proof("04-pepper-portrait-click.png", &pepper_png);

    wait_for_frame(
        addr,
        16,
        120,
        "Pepper portrait framed in blue",
        |bgra, _| rgb_at(bgra, 304, 105) == [0, 0, 255],
    );

    // --- Stage 5: Drag verification ---
    let drag_end = (420, 220);
    act(
        addr,
        r#"{"action":"drag","from_x":322,"from_y":105,"to_x":420,"to_y":220,"step_px":16,"wait":true,"auto_step":true}"#,
    );
    let drag_png = assert_aligned(None, drag_end, "drag-end");
    maybe_save_proof("05-drag-end.png", &drag_png);

    drop(close_guard);
    let _ = machine_thread.join().expect("machine thread finished");
}
