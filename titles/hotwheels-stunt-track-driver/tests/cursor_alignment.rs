//! Real-disc headless + live loopback `/api/stream` WebSocket v2 proof that
//! Hot Wheels' rendered red-arrow software cursor stays locked to `pointer_pos`,
//! `--cursor` crosshairs, and WS v2 frame-locked pointer coordinates before AND
//! after a 3D race's `SetCursorPos(320, 240)` transition (#329).

use hotwheels_stunt_track_driver::HOT_WHEELS_STUNT_TRACK_DRIVER as HOT_WHEELS;
use lego_legacy_core::control::json::{parse_json, write_json_str, JsonValue};
use lego_legacy_core::control::ws::{
    decode_binary_frame, decode_xor_rle, websocket_accept_key, FRAME_FLAG_PNG_KEYFRAME,
    FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE,
};
use lego_legacy_core::inflate::inflate;
use lego_legacy_core::testkit;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const W: u32 = 640;
const H: u32 = 480;
/// Maximum allowed pixel error on either axis between rendered red-arrow tip
/// and reported `pointer_pos` / WS v2 pointer / `--cursor` crosshair.
const MAX_ERR_PX: i32 = 2;

struct ChildGuard {
    child: Child,
    data_root: std::path::PathBuf,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.data_root);
    }
}

/// Decode an 8-bit RGB PNG written by `lego_legacy_core::png::encode` into BGRA.
fn decode_png_to_bgra(png: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "valid PNG signature");
    assert_eq!(&png[12..16], b"IHDR", "IHDR first");
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(&png[24..29], &[8, 2, 0, 0, 0], "8-bit RGB PNG");

    let mut pos = 8usize;
    let mut zlib = Vec::new();
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        if &png[pos + 4..pos + 8] == b"IDAT" {
            zlib.extend_from_slice(&png[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    assert!(zlib.len() >= 6 && zlib[0] == 0x78, "valid zlib header");
    let raw = inflate(&zlib[2..zlib.len() - 4]).expect("inflate PNG IDAT");
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

    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for px in rgb.as_chunks::<3>().0 {
        bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    (w, h, bgra)
}

/// Locate the `--cursor` crosshair center by diffing the annotated PNG against
/// the raw BGRA framebuffer and taking the midpoint of its bright-cyan arms.
fn crosshair_center_from_png(raw_bgra: &[u8], cursor_png: &[u8]) -> (i32, i32) {
    let (w, h, drawn) = decode_png_to_bgra(cursor_png);
    assert_eq!((w, h), (W, H));
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;
    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let px = &drawn[i..i + 3];
            // Bright cyan arms in BGR: [255, 255, 0].
            if px == [255, 255, 0] && px != &raw_bgra[i..i + 3] {
                let xi = x as i32;
                let yi = y as i32;
                min_x = min_x.min(xi);
                max_x = max_x.max(xi);
                min_y = min_y.min(yi);
                max_y = max_y.max(yi);
            }
        }
    }
    assert!(min_x <= max_x && min_y <= max_y, "crosshair arms not found");
    ((min_x + max_x) / 2, (min_y + max_y) / 2)
}

/// Isolate the rendered Hot Wheels `17×17` red-arrow software cursor at a
/// non-hover position using 3-way pixel difference against two frames where the
/// cursor sits elsewhere. Returns `((tip_x, tip_y), pixel_count)`.
///
/// The sprite draws a 1-pixel dark border around its upper-left arrowhead tip,
/// so the tip sits at `(min_x + 1, min_y + 1)` inside the isolated cluster.
fn isolate_cursor_tip(target: &[u8], ref_a: &[u8], ref_b: &[u8]) -> ((i32, i32), usize) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut count = 0usize;
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let p = &target[i..i + 3];
            if p != &ref_a[i..i + 3] && p != &ref_b[i..i + 3] {
                let xi = x as i32;
                let yi = y as i32;
                min_x = min_x.min(xi);
                min_y = min_y.min(yi);
                count += 1;
            }
        }
    }
    assert!(
        (80..=300).contains(&count),
        "expected isolated ~163 px red-arrow cursor cluster, got {count} px"
    );
    ((min_x + 1, min_y + 1), count)
}

/// Locate the rendered red-arrow tip over a highlighted button (like Green GO)
/// by filtering changed pixels to Hot Wheels' bright-red cursor palette.
fn isolate_red_arrow_tip_over_hover(target: &[u8], ref_a: &[u8]) -> ((i32, i32), usize) {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut count = 0usize;
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            let p = &target[i..i + 3];
            // Bright red in BGR: R > 200, G < 60, B < 60.
            if p != &ref_a[i..i + 3] && p[2] > 200 && p[1] < 60 && p[0] < 60 {
                let xi = x as i32;
                let yi = y as i32;
                min_x = min_x.min(xi);
                min_y = min_y.min(yi);
                count += 1;
            }
        }
    }
    assert!(
        count >= 25,
        "expected red-arrow cursor pixels over Green GO, got {count} px"
    );
    ((min_x, min_y), count)
}

fn read_server_ws_frame<R: Read>(r: &mut BufReader<R>) -> std::io::Result<(u8, Vec<u8>)> {
    let mut h = [0u8; 2];
    r.read_exact(&mut h)?;
    let opcode = h[0] & 0x0f;
    assert_eq!(h[1] & 0x80, 0, "server WS frames must be unmasked");
    let len7 = h[1] & 0x7f;
    let len: usize = match len7 {
        0..=125 => usize::from(len7),
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            usize::from(u16::from_be_bytes(b))
        }
        _ => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            usize::try_from(u64::from_be_bytes(b)).unwrap()
        }
    };
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok((opcode, payload))
}

#[derive(Clone)]
struct WsDecodedFrame {
    flip: u64,
    pointer_pos: (i32, i32),
    pointer_shown: bool,
    bgra: Vec<u8>,
}

struct WsState {
    latest: Option<WsDecodedFrame>,
    keyframes: usize,
    deltas: usize,
    closed: bool,
}

/// Live background `/api/stream?fps=60` WebSocket v2 client that continuously
/// decodes PNG keyframes and XOR+PackBits-RLE delta frames across the entire
/// run (preventing TCP backpressure during multi-thousand-flip races).
struct LiveWsClient {
    shared: Arc<(Mutex<WsState>, Condvar)>,
    writer: Arc<Mutex<Option<TcpStream>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LiveWsClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut sock = TcpStream::connect(addr).expect("connect WS");
        sock.set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set read timeout");
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
        .expect("write WS upgrade");
        sock.flush().expect("flush WS upgrade");

        let writer_sock = sock.try_clone().expect("clone WS socket");
        let mut reader = BufReader::new(sock);
        let mut head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read WS handshake");
            head.push_str(&line);
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        assert!(
            head.starts_with("HTTP/1.1 101 Switching Protocols\r\n"),
            "unexpected WS handshake: {head}"
        );
        let expected_accept = websocket_accept_key(key);
        assert!(
            head.contains(&format!("Sec-WebSocket-Accept: {expected_accept}\r\n")),
            "missing Sec-WebSocket-Accept: {head}"
        );

        let shared = Arc::new((
            Mutex::new(WsState {
                latest: None,
                keyframes: 0,
                deltas: 0,
                closed: false,
            }),
            Condvar::new(),
        ));
        let bg_shared = Arc::clone(&shared);
        let handle = thread::spawn(move || {
            struct NotifyClosed(Arc<(Mutex<WsState>, Condvar)>);
            impl Drop for NotifyClosed {
                fn drop(&mut self) {
                    let (lock, cv) = &*self.0;
                    if let Ok(mut st) = lock.lock() {
                        st.closed = true;
                        cv.notify_all();
                    }
                }
            }
            let _notify = NotifyClosed(Arc::clone(&bg_shared));
            let mut prev_bgra: Option<Vec<u8>> = None;
            loop {
                let (opcode, raw) = match read_server_ws_frame(&mut reader) {
                    Ok(frame) => frame,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        if bg_shared.0.lock().map(|st| st.closed).unwrap_or(true) {
                            break;
                        }
                        continue;
                    }
                    Err(_) => break,
                };
                if opcode == OPCODE_CLOSE {
                    break;
                }
                if opcode != OPCODE_BINARY {
                    continue;
                }
                let hdr = decode_binary_frame(&raw).expect("valid v2 binary frame");
                assert_eq!((u32::from(hdr.width), u32::from(hdr.height)), (W, H));
                let (pointer_pos, pointer_shown) =
                    hdr.pointer.expect("v2 frame-locked pointer extension");
                let bgra = match hdr.kind {
                    FRAME_FLAG_PNG_KEYFRAME => {
                        let (pw, ph, buf) = decode_png_to_bgra(hdr.payload);
                        assert_eq!((pw, ph), (W, H));
                        buf
                    }
                    FRAME_FLAG_XOR_RLE_DELTA => {
                        let base = prev_bgra
                            .as_ref()
                            .expect("WS delta frame must follow a keyframe");
                        decode_xor_rle(base, hdr.payload).expect("valid XOR+RLE delta")
                    }
                    other => panic!("unexpected WS binary frame kind {other}"),
                };
                prev_bgra = Some(bgra.clone());
                let (lock, cv) = &*bg_shared;
                let Ok(mut st) = lock.lock() else { break };
                if hdr.kind == FRAME_FLAG_PNG_KEYFRAME {
                    st.keyframes += 1;
                } else {
                    st.deltas += 1;
                }
                st.latest = Some(WsDecodedFrame {
                    flip: u64::from(hdr.flip),
                    pointer_pos,
                    pointer_shown,
                    bgra,
                });
                cv.notify_all();
            }
        });

        Self {
            shared,
            writer: Arc::new(Mutex::new(Some(writer_sock))),
            handle: Some(handle),
        }
    }

    fn wait_for_any_frame(&self) -> WsDecodedFrame {
        let (lock, cv) = &*self.shared;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut st = lock.lock().unwrap();
        loop {
            if let Some(frame) = &st.latest {
                return frame.clone();
            }
            assert!(!st.closed, "WS closed before initial frame");
            let rem = deadline.saturating_duration_since(Instant::now());
            assert!(!rem.is_zero(), "timed out waiting for initial WS frame");
            let (guard, _) = cv.wait_timeout(st, rem).unwrap();
            st = guard;
        }
    }

    /// Wait until the live WS stream delivers a frame at or after `min_flip`
    /// whose reconstructed BGRA matches `expected_bgra`. Does NOT filter on
    /// `pointer_pos` so the caller's pointer assertion tests frame-locking.
    fn wait_for_pixels(&self, min_flip: u64, expected_bgra: &[u8]) -> WsDecodedFrame {
        let (lock, cv) = &*self.shared;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut st = lock.lock().unwrap();
        loop {
            if let Some(frame) = &st.latest {
                if frame.flip >= min_flip && frame.bgra.as_slice() == expected_bgra {
                    return frame.clone();
                }
            }
            assert!(!st.closed, "WS closed before flip >= {min_flip}");
            let rem = deadline.saturating_duration_since(Instant::now());
            assert!(
                !rem.is_zero(),
                "timed out waiting for WS pixels at flip >= {min_flip} (latest={:?})",
                st.latest.as_ref().map(|f| (f.flip, f.pointer_pos))
            );
            let (guard, _) = cv.wait_timeout(st, rem).unwrap();
            st = guard;
        }
    }

    fn stats(&self) -> (usize, usize) {
        let st = self.shared.0.lock().unwrap();
        (st.keyframes, st.deltas)
    }

    fn shutdown(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.closed = true;
            self.shared.1.notify_all();
        }
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(mut s) = guard.take() {
                let _ = s.write_all(&[0x80 | OPCODE_CLOSE, 0x80, 1, 2, 3, 4]);
                let _ = s.flush();
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }

    fn close(mut self) {
        self.shutdown();
    }
}

impl Drop for LiveWsClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn http_call(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> Vec<u8> {
    let mut s = TcpStream::connect(addr).expect("connect HTTP");
    s.set_read_timeout(Some(Duration::from_secs(120)))
        .expect("timeout");
    match body {
        Some(json) => {
            write!(
                s,
                "{method} {path} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{json}",
                json.len()
            )
            .expect("write HTTP POST");
        }
        None => {
            write!(
                s,
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .expect("write HTTP GET");
        }
    }
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).expect("read HTTP response");
    let sep = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP header separator");
    let head = std::str::from_utf8(&resp[..sep]).expect("UTF-8 headers");
    assert!(
        head.starts_with("HTTP/1.1 200 OK\r\n"),
        "{method} {path} failed: {head}\nbody={}",
        String::from_utf8_lossy(&resp[sep + 4..])
    );
    resp[sep + 4..].to_vec()
}

fn http_json(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> JsonValue {
    let raw = http_call(addr, method, path, body);
    let txt = String::from_utf8(raw).expect("UTF-8 JSON");
    parse_json(&txt).unwrap_or_else(|e| panic!("invalid JSON from {method} {path}: {e}\n{txt}"))
}

fn status_fields(addr: SocketAddr) -> (u64, (i32, i32), (i32, i32), bool) {
    let v = http_json(addr, "GET", "/api/status", None);
    let st = v.as_object().unwrap()["status"].as_object().unwrap();
    let flip = st["flip"].as_u64().unwrap_or(0);
    let pair = |key: &str| {
        let arr = st[key].as_array().unwrap();
        (
            arr[0].as_i64().unwrap() as i32,
            arr[1].as_i64().unwrap() as i32,
        )
    };
    (
        flip,
        pair("pointer_pos"),
        pair("hw_pointer_pos"),
        st["pointer_shown"].as_bool().unwrap(),
    )
}

fn step_flips(addr: SocketAddr, flips: u64) {
    let body = format!("{{\"flips\":{flips}}}");
    let v = http_json(addr, "POST", "/api/step", Some(&body));
    assert_eq!(
        v.as_object().unwrap()["ok"].as_bool(),
        Some(true),
        "step {flips} failed: {v:?}"
    );
}

fn act_json(addr: SocketAddr, body: &str, extra_flips: u64) {
    let v = http_json(addr, "POST", "/api/act", Some(body));
    assert_eq!(
        v.as_object().unwrap()["ok"].as_bool(),
        Some(true),
        "act `{body}` failed: {v:?}"
    );
    if extra_flips > 0 {
        step_flips(addr, extra_flips);
    }
}

/// Extract Championship Track 1 from `routes/secret-track.route` (from the
/// first menu move at flip 382 through Track 1 finish before flip 3200),
/// shifting flip numbers so the first menu move lands at `start_flip`.
fn track1_route_shifted(start_flip: u64) -> String {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/secret-track.route"),
    )
    .expect("read secret-track.route");
    let mut out = String::from("#lego-legacy-route 1\n");
    for line in src.lines() {
        let Some(rest) = line.strip_prefix("flip ") else {
            continue;
        };
        let Some((flip_str, tail)) = rest.split_once(' ') else {
            continue;
        };
        let flip: u64 = flip_str.parse().expect("valid flip number");
        if flip < 382 {
            continue;
        }
        if flip >= 3200 {
            break;
        }
        let shifted = start_flip + (flip - 382);
        out.push_str(&format!("flip {shifted} {tail}\n"));
    }
    out
}

fn count_diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(pa, pb)| pa[..3] != pb[..3])
        .count()
}

#[test]
fn cursor_stays_aligned_headlessly_and_over_ws_pre_and_post_race() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    if !testkit::rips(&HOT_WHEELS.image) {
        return;
    }

    let data_root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("hotwheels-cursor-align-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);
    std::fs::create_dir_all(&data_root).expect("create scratch data_root");

    let mut child = Command::new(env!("CARGO_BIN_EXE_hotwheels-stunt-track-driver"))
        .args(["--headless", "--control", "127.0.0.1:0", "--paused"])
        .arg("--data-root")
        .arg(&data_root)
        .env("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hotwheels-stunt-track-driver");

    let stderr = child.stderr.take().expect("piped stderr");
    let _guard = ChildGuard { child, data_root };
    let mut err_reader = BufReader::new(stderr);
    let addr: SocketAddr = loop {
        let mut line = String::new();
        let n = err_reader.read_line(&mut line).expect("read stderr");
        assert!(n > 0, "child exited before printing control URL");
        if let Some((_, rest)) = line.split_once("http://") {
            let host_port = rest
                .split_whitespace()
                .next()
                .unwrap()
                .trim_end_matches('/');
            break host_port.parse().expect("valid control SocketAddr");
        }
    };
    // Drain stderr in background so the child never blocks on a full pipe.
    thread::spawn(move || {
        let mut sink = String::new();
        while err_reader.read_line(&mut sink).unwrap_or(0) > 0 {
            sink.clear();
        }
    });

    // 1. Verify /api/meta advertises SoftwareRelative (640x480, start (320,240)).
    let meta = http_json(addr, "GET", "/api/meta", None);
    let mm = meta.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    assert_eq!(
        mm["kind"].as_str(),
        Some("software_relative"),
        "/api/meta must declare software_relative for Hot Wheels"
    );
    assert_eq!(
        mm["recenter_every_flip"].as_bool(),
        Some(false),
        "Hot Wheels does not recenter every flip"
    );

    // 2. Boot past splashes/intro to Main Menu (~flip 365).
    step_flips(addr, 285);
    act_json(
        addr,
        r#"{"action":"key_hold","key":"Space","hold_flips":9,"auto_step":true}"#,
        70,
    );

    // Connect live /api/stream WebSocket v2 client across pre-race + race + post-race.
    let ws = LiveWsClient::connect(addr);
    let _ = ws.wait_for_any_frame();

    // Helper: move cursor to 3 non-hover targets, verify headless diff-tip,
    // --cursor crosshair, AND live WS v2 frame-locked pointer + reconstructed pixels.
    let verify_three_targets = |phase: &str, targets: [(i32, i32); 3]| {
        let mut captures = Vec::with_capacity(3);
        for (tx, ty) in targets {
            let (before_flip, _, _, _) = status_fields(addr);
            act_json(
                addr,
                &format!(
                    r#"{{"action":"move","x":{tx},"y":{ty},"glide_flips":1,"auto_step":true}}"#
                ),
                3,
            );
            let (flip, ptr, _hw, shown) = status_fields(addr);
            assert!(!shown, "{phase}: Hot Wheels hides OS cursor");
            assert_eq!(
                ptr,
                (tx, ty),
                "{phase}: status.pointer_pos matches requested target"
            );

            let raw_png = http_call(addr, "GET", "/api/frame.png", None);
            let (rw, rh, raw_bgra) = decode_png_to_bgra(&raw_png);
            assert_eq!((rw, rh), (W, H));

            // Verify --cursor crosshair overlay lands on reported pointer_pos.
            let cursor_png = http_call(addr, "GET", "/api/frame.png?cursor=1", None);
            let cross = crosshair_center_from_png(&raw_bgra, &cursor_png);
            assert_eq!(
                cross, ptr,
                "{phase}: --cursor crosshair center must match pointer_pos"
            );

            // Verify live WS v2 frame delivers exact pixels and frame-locked pointer.
            // Spectator deduplicates unchanged settle flips, so WS flip lies in (before_flip..=flip).
            let ws_frame = ws.wait_for_pixels(before_flip + 1, &raw_bgra);
            assert!(
                ws_frame.flip <= flip,
                "{phase}: WS flip {} exceeds status flip {flip}",
                ws_frame.flip
            );
            assert!(
                !ws_frame.pointer_shown,
                "{phase}: WS pointer_shown is false"
            );
            assert_eq!(
                ws_frame.pointer_pos, ptr,
                "{phase}: WS v2 pointer_pos matches status"
            );
            captures.push(((tx, ty), ptr, raw_bgra, ws_frame));
        }

        for idx in 0..3 {
            let (tgt, ptr, raw_bgra, ws_frame) = &captures[idx];
            let o1 = &captures[(idx + 1) % 3];
            let o2 = &captures[(idx + 2) % 3];
            let (hl_tip, px_count) = isolate_cursor_tip(raw_bgra, &o1.2, &o2.2);
            let (ws_tip, _) = isolate_cursor_tip(&ws_frame.bgra, &o1.3.bgra, &o2.3.bgra);
            assert_eq!(hl_tip, ws_tip, "{phase}: WS tip == headless tip");

            let dx = (hl_tip.0 - ptr.0).abs();
            let dy = (hl_tip.1 - ptr.1).abs();
            eprintln!(
                "hotwheels cursor [{phase}] target={tgt:?} ptr={ptr:?} ws_ptr={:?} rendered_tip={hl_tip:?} err=({dx},{dy})px cluster={px_count}px",
                ws_frame.pointer_pos
            );
            assert!(
                dx <= MAX_ERR_PX && dy <= MAX_ERR_PX,
                "{phase}: target {tgt:?} desynced! reported={ptr:?} rendered_tip={hl_tip:?} err=({dx},{dy})px"
            );
        }
    };

    // 3. Pre-race alignment proof on Main Menu; finish at (320, 240) so
    //    Track 1 route enters Championship cleanly from center.
    verify_three_targets("pre-race", [(270, 100), (120, 340), (320, 240)]);

    // 4. Enter Championship, click Green GO (583, 430), and finish Track 1
    //    to trigger STUNT.EXE's real SetCursorPos(320, 240) race transitions.
    let (cur_flip, _, _, _) = status_fields(addr);
    let pipe_start = cur_flip + 5;
    let route_text = track1_route_shifted(pipe_start);
    let mut pipe_body = String::from(r#"{"route":"#);
    write_json_str(&mut pipe_body, &route_text);
    pipe_body.push_str(r#","relative":false,"auto_step":false}"#);
    let pipe_resp = http_json(addr, "POST", "/api/pipe", Some(&pipe_body));
    assert_eq!(
        pipe_resp.as_object().unwrap()["ok"].as_bool(),
        Some(true),
        "pipe Track 1 route failed: {pipe_resp:?}"
    );

    // Step in chunks to the settled Track 1 Results screen (~2,960 flips past pipe_start).
    let results_flip = pipe_start + (3339 - 382);
    loop {
        let (now, _, _, _) = status_fields(addr);
        if now >= results_flip {
            break;
        }
        let chunk = (results_flip - now).min(600);
        step_flips(addr, chunk);
    }

    // Confirm real SetCursorPos(320, 240) happened while visual cursor stayed at (583, 430).
    let (post_flip, post_ptr, post_hw, _) = status_fields(addr);
    eprintln!(
        "hotwheels post-race status: flip={post_flip} pointer_pos={post_ptr:?} hw_pointer_pos={post_hw:?}"
    );
    assert_eq!(
        post_hw,
        (320, 240),
        "race transition must have triggered STUNT.EXE SetCursorPos(320, 240)"
    );
    assert_eq!(
        post_ptr,
        (583, 430),
        "SoftwareRelative must preserve visual cursor at Green GO (583, 430) across SetCursorPos"
    );

    // 5. Post-race alignment proof across 3 targets on the Results screen.
    verify_three_targets("post-race", [(270, 100), (320, 240), (440, 220)]);

    // 6. Verify cursor lands on Green GO (585, 430) post-race and FIRST click advances!
    let (before_go_flip, _, _, _) = status_fields(addr);
    let (_, _, before_go_bgra) =
        decode_png_to_bgra(&http_call(addr, "GET", "/api/frame.png", None));
    act_json(
        addr,
        r#"{"action":"move","x":585,"y":430,"glide_flips":1,"auto_step":true}"#,
        3,
    );
    let (go_flip, go_ptr, _, _) = status_fields(addr);
    assert_eq!(go_ptr, (585, 430));
    let (_, _, on_go_bgra) = decode_png_to_bgra(&http_call(addr, "GET", "/api/frame.png", None));
    let ws_go = ws.wait_for_pixels(before_go_flip + 1, &on_go_bgra);
    assert_eq!(ws_go.pointer_pos, (585, 430));
    let (go_tip, go_red_px) = isolate_red_arrow_tip_over_hover(&on_go_bgra, &before_go_bgra);
    let go_dx = (go_tip.0 - 585).abs();
    let go_dy = (go_tip.1 - 430).abs();
    eprintln!(
        "hotwheels cursor [post-race Green GO] target=(585,430) ws_ptr={:?} red_tip={go_tip:?} err=({go_dx},{go_dy})px red_px={go_red_px}",
        ws_go.pointer_pos
    );
    assert!(
        go_dx <= MAX_ERR_PX && go_dy <= MAX_ERR_PX,
        "Green GO red-arrow tip {go_tip:?} missed (585, 430) by ({go_dx},{go_dy})px"
    );

    // Click Green GO once and confirm the Results screen advances cleanly.
    act_json(
        addr,
        r#"{"action":"click","x":585,"y":430,"button":"left","hold_flips":6,"auto_step":true}"#,
        60,
    );
    let (after_flip, _, _, _) = status_fields(addr);
    let (_, _, after_go_bgra) = decode_png_to_bgra(&http_call(addr, "GET", "/api/frame.png", None));
    let changed = count_diff_pixels(&on_go_bgra, &after_go_bgra);
    eprintln!(
        "hotwheels post-race Green GO first-click screen diff: {changed} px (flip {go_flip} -> {after_flip})"
    );
    assert!(
        changed > 50_000,
        "first click on Green GO (585, 430) post-race must advance screen (only {changed} px changed)"
    );

    let (keyframes, deltas) = ws.stats();
    eprintln!("hotwheels WS v2 stream stats: keyframes={keyframes} deltas={deltas}");
    assert!(keyframes >= 1, "expected at least one WS v2 PNG keyframe");
    assert!(deltas >= 5, "expected multiple WS v2 XOR+RLE delta frames");

    ws.close();
}
