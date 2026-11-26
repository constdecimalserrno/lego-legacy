//! Real-disc cursor-to-pointer alignment and `/watch` WebSocket v2 proof for
//! LEGO Chess (#330).
//!
//! Boots LEGO Chess headless under `ControlHost<Headless>` + live loopback
//! `HttpServer` (`/api/meta`, `/api/mode`, `/api/act`, `/api/stream` WS v2)
//! against the player's retail disc. Skips cleanly when the disc is absent
//! unless `LEGO_LEGACY_REQUIRE_DISC=1`.

use lego_chess::{GUIDE, LEGO_CHESS as CHESS};
use lego_legacy_core::cli::Cli;
use lego_legacy_core::control::json::parse_json;
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::ws::{
    decode_binary_frame, decode_xor_rle, FRAME_FLAG_HAS_POINTER, FRAME_FLAG_PNG_KEYFRAME,
    FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG,
};
use lego_legacy_core::control::{
    Action, Command, ControlBus, ControlConfig, ControlHost, ExecMode, FrameSnapshot, HttpContext,
    HttpServer, InputOwner,
};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::inflate::inflate;
use lego_legacy_core::{run, testkit};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const BUDGET: u64 = 8_000_000_000;
const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_CURSOR_ERR_PX: i32 = 2;

/// LEGO Chess's `graphics\pointers\*.tga` pointer sprite spans
/// `[x + 1 ..= x + 54, y - 3 ..= y + 42]` relative to the `WM_MOUSEMOVE`
/// hotspot `(x, y)`.
const TGA_BBOX_DX: i32 = 1;
const TGA_BBOX_DY: i32 = -3;

fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    testkit::rips(&CHESS.image)
}

/// Remove scratch files/directories on drop so test failures leave no clutter.
struct TempGuard {
    dir: PathBuf,
    file: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.file);
    }
}

/// Decode a 24-bit RGB PNG (filters 0=None and 1=Sub, as emitted by
/// `lego_legacy_core::png::encode`) directly into BGRA32 pixels.
fn decode_png_to_bgra(png: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&png[12..16], b"IHDR", "IHDR chunk");
    let w = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let h = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    assert_eq!(&png[24..29], &[8, 2, 0, 0, 0], "RGB24 non-interlaced");

    let mut pos = 8usize;
    let mut zlib = Vec::new();
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        if &png[pos + 4..pos + 8] == b"IDAT" {
            zlib.extend_from_slice(&png[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    assert!(zlib.len() >= 6 && zlib[0] == 0x78, "zlib header");
    let raw = inflate(&zlib[2..zlib.len() - 4]).expect("inflate PNG IDAT");
    let row_rgb = (w as usize) * 3;
    assert_eq!(raw.len(), (row_rgb + 1) * (h as usize));

    let mut bgra = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for y in 0..h as usize {
        let line = &raw[y * (row_rgb + 1)..(y + 1) * (row_rgb + 1)];
        let (mut pr, mut pg, mut pb) = (0u8, 0u8, 0u8);
        match line[0] {
            0 => {
                for px in line[1..].as_chunks::<3>().0 {
                    bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
            1 => {
                for px in line[1..].as_chunks::<3>().0 {
                    pr = px[0].wrapping_add(pr);
                    pg = px[1].wrapping_add(pg);
                    pb = px[2].wrapping_add(pb);
                    bgra.extend_from_slice(&[pb, pg, pr, 255]);
                }
            }
            other => panic!("unexpected PNG filter {other}"),
        }
    }
    (w, h, bgra)
}

/// Find the center of the crosshair drawn by `render_frame_png(..., cursor=true)`
/// from its 36-pixel symmetric cyan `[255, 255, 0]` arms (immune to 1x1 red
/// background pixel collisions) and verify the red `[0, 0, 255]` center dot.
fn find_crosshair_center(raw_bgra: &[u8], annotated_png: &[u8]) -> (i32, i32) {
    let (w, h, bgra) = decode_png_to_bgra(annotated_png);
    let mut min_x = i32::MAX;
    let mut max_x = i32::MIN;
    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let changed = bgra[i] != raw_bgra[i]
                || bgra[i + 1] != raw_bgra[i + 1]
                || bgra[i + 2] != raw_bgra[i + 2];
            if changed && bgra[i] >= 220 && bgra[i + 1] >= 220 && bgra[i + 2] <= 40 {
                min_x = min_x.min(x as i32);
                max_x = max_x.max(x as i32);
                min_y = min_y.min(y as i32);
                max_y = max_y.max(y as i32);
            }
        }
    }
    assert!(min_x <= max_x, "crosshair cyan arms must be present");
    let (cx, cy) = ((min_x + max_x) / 2, (min_y + max_y) / 2);
    let ci = ((cy as u32 * w + cx as u32) * 4) as usize;
    assert!(
        bgra[ci + 2] >= 220 && bgra[ci + 1] <= 40 && bgra[ci] <= 40,
        "crosshair center ({cx}, {cy}) must be red, got BGR={:?}",
        &bgra[ci..ci + 3]
    );
    (cx, cy)
}

#[derive(Default)]
struct WsShared {
    prev_bgra: Option<Vec<u8>>,
    last_flip: Option<u64>,
    last_ptr: Option<(i32, i32)>,
    last_shown: Option<bool>,
    keyframes: usize,
    deltas: usize,
    error: Option<String>,
    closed: bool,
}

/// Live loopback WebSocket `/api/stream` client backed by a dedicated reader
/// thread so the loopback socket drains continuously during long `step_flips`
/// bursts without ever tripping `STREAM_WRITE_TIMEOUT`.
struct WsClient {
    shared: Arc<(Mutex<WsShared>, Condvar)>,
}

fn read_raw_ws_frame(reader: &mut BufReader<TcpStream>) -> io::Result<(u8, Vec<u8>)> {
    let mut hdr = [0u8; 2];
    reader.read_exact(&mut hdr)?;
    let opcode = hdr[0] & 0x0f;
    let len7 = hdr[1] & 0x7f;
    let len = match len7 {
        126 => {
            let mut ext = [0u8; 2];
            reader.read_exact(&mut ext)?;
            u16::from_be_bytes(ext) as usize
        }
        127 => {
            let mut ext = [0u8; 8];
            reader.read_exact(&mut ext)?;
            u64::from_be_bytes(ext) as usize
        }
        n => usize::from(n),
    };
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok((opcode, payload))
}

impl WsClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut sock = TcpStream::connect(addr).expect("connect WS");
        sock.set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        write!(
            sock,
            "GET /api/stream?fps=60 HTTP/1.1\r\n\
             Host: localhost\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .unwrap();
        sock.flush().unwrap();
        let mut reader = BufReader::new(sock);
        let mut status = String::new();
        reader.read_line(&mut status).unwrap();
        assert!(
            status.contains("101"),
            "expected 101 upgrade, got {status:?}"
        );
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" || line == "\n" || line.is_empty() {
                break;
            }
        }
        let shared = Arc::new((Mutex::new(WsShared::default()), Condvar::new()));
        let bg = Arc::clone(&shared);
        std::thread::spawn(move || loop {
            let (op, raw) = match read_raw_ws_frame(&mut reader) {
                Ok(frame) => frame,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(e) => {
                    let (lock, cv) = &*bg;
                    let mut st = lock.lock().unwrap();
                    st.closed = true;
                    if e.kind() != io::ErrorKind::UnexpectedEof {
                        st.error = Some(e.to_string());
                    }
                    cv.notify_all();
                    break;
                }
            };
            match op {
                OPCODE_BINARY => {
                    let Some(hdr) = decode_binary_frame(&raw) else {
                        let (lock, cv) = &*bg;
                        let mut st = lock.lock().unwrap();
                        st.error = Some("invalid v2 WS binary frame".to_owned());
                        st.closed = true;
                        cv.notify_all();
                        break;
                    };
                    if (hdr.flags & FRAME_FLAG_HAS_POINTER) == 0 {
                        let (lock, cv) = &*bg;
                        let mut st = lock.lock().unwrap();
                        st.error = Some("WS frame missing v2 pointer flag".to_owned());
                        st.closed = true;
                        cv.notify_all();
                        break;
                    }
                    let Some((ptr, shown)) = hdr.pointer else {
                        let (lock, cv) = &*bg;
                        let mut st = lock.lock().unwrap();
                        st.error = Some("WS frame missing v2 pointer metadata".to_owned());
                        st.closed = true;
                        cv.notify_all();
                        break;
                    };
                    let (lock, cv) = &*bg;
                    let mut st = lock.lock().unwrap();
                    let bgra = match hdr.kind {
                        FRAME_FLAG_PNG_KEYFRAME => {
                            st.keyframes += 1;
                            let (w, h, px) = decode_png_to_bgra(hdr.payload);
                            assert_eq!((w as u16, h as u16), (hdr.width, hdr.height));
                            px
                        }
                        FRAME_FLAG_XOR_RLE_DELTA => {
                            st.deltas += 1;
                            let prev = st.prev_bgra.as_ref().expect("keyframe before delta");
                            decode_xor_rle(prev, hdr.payload).expect("decode XOR+RLE delta")
                        }
                        other => {
                            st.error = Some(format!("unexpected WS binary frame kind {other}"));
                            st.closed = true;
                            cv.notify_all();
                            break;
                        }
                    };
                    st.last_flip = Some(u64::from(hdr.flip));
                    st.last_ptr = Some(ptr);
                    st.last_shown = Some(shown);
                    st.prev_bgra = Some(bgra);
                    cv.notify_all();
                }
                OPCODE_PING | OPCODE_PONG => continue,
                OPCODE_CLOSE => {
                    let (lock, cv) = &*bg;
                    lock.lock().unwrap().closed = true;
                    cv.notify_all();
                    break;
                }
                other => {
                    let (lock, cv) = &*bg;
                    let mut st = lock.lock().unwrap();
                    st.error = Some(format!("unexpected WS opcode {other}"));
                    st.closed = true;
                    cv.notify_all();
                    break;
                }
            }
        });
        Self { shared }
    }

    fn counts(&self) -> (usize, usize) {
        let st = self.shared.0.lock().unwrap();
        (st.keyframes, st.deltas)
    }

    /// Wait until decoded WS v2 state matches `snap` (at `snap.flip`, or at an
    /// earlier flip whose pixels and pointer match `snap` bit-for-bit when
    /// `serve_websocket_stream` deduplicates identical consecutive frames).
    fn sync_to_snapshot(&self, snap: &FrameSnapshot) -> (u64, (i32, i32), bool, Vec<u8>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (lock, cv) = &*self.shared;
        let mut st = lock.lock().unwrap();
        loop {
            if let Some(err) = &st.error {
                panic!("WS reader error waiting for flip {}: {err}", snap.flip);
            }
            if let (Some(f), Some(ptr), Some(shown), Some(bgra)) = (
                st.last_flip,
                st.last_ptr,
                st.last_shown,
                st.prev_bgra.as_ref(),
            ) {
                if f >= snap.flip
                    || (ptr == snap.pointer_pos
                        && shown == snap.pointer_shown
                        && bgra.as_slice() == snap.bgra.as_slice())
                {
                    return (f, ptr, shown, bgra.clone());
                }
            }
            assert!(
                !st.closed,
                "WS closed unexpectedly before flip {} (last flip={:?} ptr={:?})",
                snap.flip, st.last_flip, st.last_ptr
            );
            let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
                panic!(
                    "timed out waiting for WS flip {} ptr {:?} (last flip={:?} ptr={:?})",
                    snap.flip, snap.pointer_pos, st.last_flip, st.last_ptr
                );
            };
            let (next, _) = cv.wait_timeout(st, rem).unwrap();
            st = next;
        }
    }
}

fn http_request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut sock = TcpStream::connect(addr).expect("connect HTTP");
    sock.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    write!(
        sock,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n"
    )
    .unwrap();
    if let Some(payload) = body {
        write!(
            sock,
            "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
            payload.len()
        )
        .unwrap();
    } else {
        write!(sock, "\r\n").unwrap();
    }
    sock.flush().unwrap();
    let mut resp = String::new();
    sock.read_to_string(&mut resp).unwrap();
    let (head, rest) = resp.split_once("\r\n\r\n").expect("HTTP separator");
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("HTTP status");
    (status, rest.to_owned())
}

#[derive(Debug)]
struct DiffCluster {
    count: usize,
    min_x: i32,
    min_y: i32,
}

/// Extract the differing pixel cluster between `before` and `after` inside a
/// search box centered on `center` (`±radius`).
fn diff_cluster_near(
    before: &[u8],
    after: &[u8],
    w: u32,
    h: u32,
    center: (i32, i32),
    radius: i32,
) -> Option<DiffCluster> {
    let x0 = (center.0 - radius).clamp(0, w as i32) as u32;
    let x1 = (center.0 + radius + 1).clamp(0, w as i32) as u32;
    let y0 = (center.1 - radius).clamp(0, h as i32) as u32;
    let y1 = (center.1 + radius + 1).clamp(0, h as i32) as u32;

    let mut count = 0usize;
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;

    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            let dr = (i32::from(before[i + 2]) - i32::from(after[i + 2])).abs();
            let dg = (i32::from(before[i + 1]) - i32::from(after[i + 1])).abs();
            let db = (i32::from(before[i]) - i32::from(after[i])).abs();
            if dr + dg + db > 24 {
                count += 1;
                min_x = min_x.min(x as i32);
                min_y = min_y.min(y as i32);
            }
        }
    }
    (count > 0).then_some(DiffCluster {
        count,
        min_x,
        min_y,
    })
}

/// Opaque pixel sample `(dx, dy, [b, g, r])` relative to pointer hotspot `(0, 0)`.
type CursorTemplate = Vec<(i32, i32, [u8; 3])>;

/// Extract opaque TGA cursor pixels from two static-background frames with the
/// cursor at two well-separated targets `p1` and `p2`.
fn extract_cursor_template(
    f1: &[u8],
    p1: (i32, i32),
    f2: &[u8],
    p2: (i32, i32),
    w: u32,
    h: u32,
) -> CursorTemplate {
    let px_at = |buf: &[u8], x: i32, y: i32| -> [u8; 3] {
        let i = ((y as u32 * w + x as u32) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2]]
    };
    let mut tpl = Vec::new();
    for dy in -12..=52 {
        for dx in -12..=64 {
            let (x1, y1) = (p1.0 + dx, p1.1 + dy);
            let (x2, y2) = (p2.0 + dx, p2.1 + dy);
            if !(0..w as i32).contains(&x1)
                || !(0..h as i32).contains(&y1)
                || !(0..w as i32).contains(&x2)
                || !(0..h as i32).contains(&y2)
            {
                continue;
            }
            let c1 = px_at(f1, x1, y1);
            let bg1 = px_at(f2, x1, y1);
            let c2 = px_at(f2, x2, y2);
            let bg2 = px_at(f1, x2, y2);
            let diff1 = (i32::from(c1[0]) - i32::from(bg1[0])).abs()
                + (i32::from(c1[1]) - i32::from(bg1[1])).abs()
                + (i32::from(c1[2]) - i32::from(bg1[2])).abs();
            let diff2 = (i32::from(c2[0]) - i32::from(bg2[0])).abs()
                + (i32::from(c2[1]) - i32::from(bg2[1])).abs()
                + (i32::from(c2[2]) - i32::from(bg2[2])).abs();
            let same = (i32::from(c1[0]) - i32::from(c2[0])).abs() <= 4
                && (i32::from(c1[1]) - i32::from(c2[1])).abs() <= 4
                && (i32::from(c1[2]) - i32::from(c2[2])).abs() <= 4;
            if same && diff1 > 24 && diff2 > 24 {
                tpl.push((dx, dy, c1));
            }
        }
    }
    tpl
}

/// Locate the TGA cursor hotspot `(x, y)` in `bgra` by sliding `tpl` across
/// `near` (`center ± radius`) or the whole screen (`None`).
fn locate_cursor_by_template(
    bgra: &[u8],
    w: u32,
    h: u32,
    tpl: &CursorTemplate,
    near: Option<((i32, i32), i32)>,
) -> (i32, i32, f64) {
    let stride = w as usize;
    let scan = |x0: i32, x1: i32, y0: i32, y1: i32, step: i32| -> (i32, i32, usize, usize) {
        let mut best = (0i32, 0i32, 0usize, 0usize);
        let mut y = y0.max(0);
        let y_end = y1.min(h as i32 - 1);
        let x_start = x0.max(0);
        let x_end = x1.min(w as i32 - 1);
        while y <= y_end {
            let mut x = x_start;
            while x <= x_end {
                let mut hits = 0usize;
                let mut total = 0usize;
                for &(dx, dy, [tb, tg, tr]) in tpl {
                    let px = x + dx;
                    let py = y + dy;
                    if (0..w as i32).contains(&px) && (0..h as i32).contains(&py) {
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
                if hits > best.2 {
                    best = (x, y, hits, total);
                }
                x += step;
            }
            y += step;
        }
        best
    };

    let best = match near {
        Some(((cx, cy), r)) => scan(cx - r, cx + r, cy - r, cy + r, 1),
        None => {
            let coarse = scan(0, w as i32 - 1, 0, h as i32 - 1, 2);
            scan(coarse.0 - 2, coarse.0 + 2, coarse.1 - 2, coarse.1 + 2, 1)
        }
    };
    (best.0, best.1, best.2 as f64 / best.3.max(1) as f64)
}

fn step_flips(bus: &ControlBus, flips: u64) {
    let resp = bus.dispatch_sync(Command::Step { flips }, Some(TIMEOUT));
    assert!(
        resp.ok && resp.status.paused && !resp.status.finished,
        "step {flips} did not pause cleanly: ok={} paused={} finished={} err={:?}",
        resp.ok,
        resp.status.paused,
        resp.status.finished,
        resp.error
    );
}

#[test]
fn lego_chess_rendered_cursor_matches_pointer_and_ws_v2_across_screens() {
    if !scratch() {
        return;
    }

    // Minimal boot route: skip Krisalis' logo (flip 290) and intro (flip 300)
    // with poll-released Space, landing on the New Player form (~flip 307)
    // with zero mouse moves issued yet.
    let pid = std::process::id();
    let boot_route =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("chess-cursor-boot-{pid}.route"));
    let data_root =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("chess-cursor-data-{pid}"));
    let _ = std::fs::remove_dir_all(&data_root);
    std::fs::create_dir_all(&data_root).expect("create fresh data root");
    std::fs::write(
        &boot_route,
        "#lego-legacy-route 1\n\
         flip 290 key Space down\n\
         flip 290 poll 300 key Space up\n\
         flip 300 key Space down\n\
         flip 300 poll 300 key Space up\n",
    )
    .expect("write boot route");
    let _cleanup = TempGuard {
        dir: data_root.clone(),
        file: boot_route.clone(),
    };

    let cli = Cli {
        headless: true,
        route: Some(boot_route),
        data_root: Some(data_root),
        ..Cli::default()
    };

    let mut control = ControlHost::new(
        Headless::new(),
        ControlConfig {
            title: CHESS.key,
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            start_paused: false,
            step_uncapped: true,
            pace: 1.0,
            pointer_speed: CHESS.pointer_speed,
            flip_cost: CHESS.flip_cost,
            guide: &GUIDE,
        },
    );
    let bus = control.bus();

    // 1. Pre-boot init sync (#327): SoftwareAbsolute starts at (320, 240), hidden.
    let init_st = bus.status();
    assert_eq!(init_st.pointer_pos, (320, 240));
    assert_eq!(init_st.hw_pointer_pos, (320, 240));
    assert!(!init_st.pointer_shown);

    let ctx = Arc::new(HttpContext::new(bus.clone(), &CHESS, None).with_guide(&GUIDE));
    let mut server = HttpServer::start("127.0.0.1:0", ctx).expect("start HTTP server");
    let addr = server.addr();

    // 2. /api/meta advertises SoftwareAbsolute { requires_poll_release: true } so
    //    /watch Human Take Over dispatches `click_poll`.
    let (status, meta_body) = http_request(addr, "GET", "/api/meta", None);
    assert_eq!(status, 200);
    let meta_json = parse_json(&meta_body).expect("valid /api/meta JSON");
    let mm = meta_json.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    assert_eq!(mm["kind"].as_str(), Some("software_absolute"));
    assert_eq!(mm["requires_poll_release"].as_bool(), Some(true));

    let guest_thread = std::thread::spawn(move || {
        let res = run::run_digested_with_guide(&CHESS, &GUIDE, &cli, &mut control, BUDGET);
        match res {
            Ok((finish, _log, digest)) => {
                control.mark_finished(&finish, &digest);
                finish
            }
            Err(e) => panic!("run_digested_with_guide failed: {e}"),
        }
    });

    // Wait for boot route to complete, then step to flip 312 (New Player settled).
    let paused = bus.wait_until_paused(Some(TIMEOUT));
    assert!(
        paused.paused && !paused.finished,
        "expected paused post-boot state: {paused:?}"
    );
    assert_eq!(paused.pointer_pos, (320, 240));
    assert!(!paused.pointer_shown);
    let to_312 = 312u64.saturating_sub(paused.flip.unwrap_or(0));
    if to_312 > 0 {
        step_flips(&bus, to_312);
    }

    // Connect live loopback WebSocket stream and verify initial v2 frame.
    let ws = WsClient::connect(addr);
    let f_init = bus.latest_frame().expect("initial New Player frame");
    let (ws_f0, ws_p0, ws_s0, ws_bgra0) = ws.sync_to_snapshot(&f_init);
    assert!(ws_f0 <= f_init.flip);
    assert_eq!(ws_p0, (320, 240));
    assert_eq!(ws_p0, f_init.pointer_pos);
    assert!(!ws_s0);
    assert_eq!(ws_bgra0, *f_init.bgra);

    // Move to `(tx, ty)`, step 2 flips, verify headless `FrameSnapshot`, live WS v2
    // frame (bit-exact pixels + frame-locked pointer), and `--cursor` crosshair.
    let move_and_sync_ws =
        |label: &str, tx: i32, ty: i32| -> (FrameSnapshot, FrameSnapshot, Vec<u8>) {
            let before = bus.latest_frame().expect("before frame");
            let resp = bus.dispatch_sync(
                Command::Act {
                    action: Action::Move {
                        x: tx,
                        y: ty,
                        glide_flips: 1,
                    },
                    auto_step: true,
                },
                Some(TIMEOUT),
            );
            assert!(resp.ok, "move {label} failed: {:?}", resp.error);
            step_flips(&bus, 2);
            let after = bus.latest_frame().expect("after frame");
            let st = bus.status();
            let (wf, wp, wshown, wbgra) = ws.sync_to_snapshot(&after);
            assert!(
                wf >= before.flip && wf <= after.flip,
                "{label}: WS flip {wf} outside [{}, {}]",
                before.flip,
                after.flip
            );
            assert_eq!(st.pointer_pos, (tx, ty), "{label}: status.pointer_pos");
            assert_eq!(after.pointer_pos, (tx, ty), "{label}: snap.pointer_pos");
            assert_eq!(wp, (tx, ty), "{label}: ws v2 pointer_pos");
            assert!(!wshown, "{label}: pointer_shown must be false");
            assert_eq!(wbgra, *after.bgra, "{label}: WS BGRA bit-exact match");

            let png = render_frame_png(
                &after,
                ObserveOptions {
                    cursor: true,
                    ..ObserveOptions::default()
                },
                after.pointer_pos,
            )
            .expect("render cursor png");
            let cross = find_crosshair_center(&after.bgra, &png);
            assert_eq!(cross, wp, "{label}: --cursor crosshair matches ws_ptr");
            (before, after, wbgra)
        };

    // Assert static-background consecutive-frame diff cluster tip == pointer_pos.
    let assert_static_diff_tip = |label: &str, before: &FrameSnapshot, after: &FrameSnapshot| {
        let cluster = diff_cluster_near(
            &before.bgra,
            &after.bgra,
            after.width,
            after.height,
            after.pointer_pos,
            64,
        )
        .unwrap_or_else(|| panic!("{label}: expected TGA cursor diff cluster"));
        assert!(
            cluster.count >= 500,
            "{label}: diff cluster too small ({})",
            cluster.count
        );
        let tip = (cluster.min_x - TGA_BBOX_DX, cluster.min_y - TGA_BBOX_DY);
        assert!(
            (tip.0 - after.pointer_pos.0).abs() <= MAX_CURSOR_ERR_PX
                && (tip.1 - after.pointer_pos.1).abs() <= MAX_CURSOR_ERR_PX,
            "{label}: diff-extracted TGA tip {tip:?} diverged from pointer_pos {:?}",
            after.pointer_pos
        );
    };

    // 3. New Player form: static diff-cluster tip proof + opaque TGA sprite extraction.
    let (b1, f_np1, _) = move_and_sync_ws("new-player-1", 150, 140);
    assert_static_diff_tip("new-player-1", &b1, &f_np1);
    let (b2, f_np2, _) = move_and_sync_ws("new-player-2", 480, 180);
    assert_static_diff_tip("new-player-2", &b2, &f_np2);

    let tpl = extract_cursor_template(&f_np1.bgra, (150, 140), &f_np2.bgra, (480, 180), 640, 480);
    assert!(
        tpl.len() >= 500,
        "expected >=500 opaque TGA cursor pixels, got {}",
        tpl.len()
    );

    let assert_template_match =
        |label: &str, bgra: &[u8], expected: (i32, i32), near: Option<((i32, i32), i32)>| {
            let (gx, gy, ratio) = locate_cursor_by_template(bgra, 640, 480, &tpl, near);
            assert!(
                ratio >= 0.60,
                "{label}: TGA template match ratio {ratio:.3} too low at ({gx}, {gy})"
            );
            assert!(
                (gx - expected.0).abs() <= MAX_CURSOR_ERR_PX
                    && (gy - expected.1).abs() <= MAX_CURSOR_ERR_PX,
                "{label}: rendered TGA cursor ({gx}, {gy}) diverged from pointer_pos {expected:?}"
            );
        };

    // Confirm guest `.bss` cursor sat at (0, 0) before the first WM_MOUSEMOVE.
    assert_template_match("new-player-init-bss", &f_init.bgra, (0, 0), None);

    let (b3, f_np3, ws_np3) = move_and_sync_ws("new-player-3", 220, 360);
    assert_static_diff_tip("new-player-3", &b3, &f_np3);
    assert_template_match("new-player-3-ws", &ws_np3, (220, 360), None);

    // 4. `/watch` Human Take Over `POST /api/act` `click_poll` helper: matches
    //    `core/src/control/watch.html` (`owner=shared`, `hold_flips=2`, `hold_ms=150`,
    //    `source=human`), keeping `ExecMode::Step` and `"wait":true` so each click
    //    synchronizes deterministically on `step_epoch` without racing `is_paused`.
    let (mode_code, _) = http_request(addr, "POST", "/api/mode", Some("{\"owner\":\"shared\"}"));
    assert_eq!(mode_code, 200);

    let watch_click_poll = |label: &str, x: i32, y: i32| {
        let body = format!(
            "{{\"action\":\"click_poll\",\"x\":{x},\"y\":{y},\"button\":\"left\",\"hold_flips\":2,\"hold_ms\":150,\"source\":\"human\",\"wait\":true,\"auto_step\":true}}"
        );
        let (code, text) = http_request(addr, "POST", "/api/act", Some(&body));
        assert_eq!(code, 200, "watch click_poll {label}: {text}");
        let v = parse_json(&text).expect("valid act json");
        assert_eq!(
            v.as_object().unwrap()["ok"].as_bool(),
            Some(true),
            "{label}: {text}"
        );
        let st = bus.status();
        assert!(
            st.paused && !st.finished,
            "{label}: expected paused status after click_poll: {st:?}"
        );
        assert_eq!(st.pointer_pos, (x, y), "{label}: pointer_pos after click");
    };

    // New Player green tick -> Who's Playing -> Throne Room.
    watch_click_poll("new-player-tick", 576, 420);
    step_flips(&bus, 10);
    watch_click_poll("whos-playing-tick", 576, 420);
    step_flips(&bus, 15);

    // 5. Throne Room: verify cursor alignment while camera settles on Chess Table (385, 285),
    //    then click Chess Table via `/watch` `click_poll` (zero poll-release hang).
    let (_, _, ws_throne_l) = move_and_sync_ws("throne-room-left", 220, 260);
    assert_template_match(
        "throne-room-left-ws",
        &ws_throne_l,
        (220, 260),
        Some(((220, 260), 64)),
    );
    let (_, _, ws_throne_table) = move_and_sync_ws("throne-room-table", 385, 285);
    assert_template_match(
        "throne-room-table-ws",
        &ws_throne_table,
        (385, 285),
        Some(((385, 285), 64)),
    );
    step_flips(&bus, 28);
    watch_click_poll("throne-chess-table", 385, 285);
    step_flips(&bus, 10);

    // 6. Quick Game screen: template-matched alignment across 3 menu targets.
    for (label, tx, ty) in [
        ("quick-game-1", 160, 130),
        ("quick-game-2", 480, 260),
        ("quick-game-3", 320, 380),
    ] {
        let (_, _, ws_bgra) = move_and_sync_ws(label, tx, ty);
        assert_template_match(label, &ws_bgra, (tx, ty), Some(((tx, ty), 64)));
    }

    // Start match via `/watch` `click_poll` (590, 410) -> 3D Chessboard.
    watch_click_poll("quick-game-start", 590, 410);
    step_flips(&bus, 120);
    let _ = ws.sync_to_snapshot(&bus.latest_frame().expect("board ready frame"));

    // 7. 3D Chessboard (deterministic initial position): static diff-cluster tip
    //    on margins + template match on 3 board squares.
    let (bm1, fm1, wsm1) = move_and_sync_ws("board-3d-margin-bl", 80, 400);
    assert_static_diff_tip("board-3d-margin-bl", &bm1, &fm1);
    assert_template_match(
        "board-3d-margin-bl-ws",
        &wsm1,
        (80, 400),
        Some(((80, 400), 64)),
    );

    let (bm2, fm2, wsm2) = move_and_sync_ws("board-3d-margin-br", 560, 400);
    assert_static_diff_tip("board-3d-margin-br", &bm2, &fm2);
    assert_template_match(
        "board-3d-margin-br-ws",
        &wsm2,
        (560, 400),
        Some(((560, 400), 64)),
    );

    for (label, tx, ty) in [
        ("board-3d-e2", 356, 308),
        ("board-3d-c4", 228, 241),
        ("board-3d-f7", 396, 180),
    ] {
        let (_, _, ws_bgra) = move_and_sync_ws(label, tx, ty);
        assert_template_match(label, &ws_bgra, (tx, ty), Some(((tx, ty), 64)));
    }

    // 8. Toggle 3D -> 2D overhead board BEFORE any AI move so the initial board
    //    state is 100% deterministic across all 2D square alignment checks.
    let before_2d = bus.latest_frame().expect("before 2d toggle");
    watch_click_poll("toggle-2d", 440, 60);
    step_flips(&bus, 12);
    let after_2d = bus.latest_frame().expect("after 2d toggle");
    let changed_2d = diff_cluster_near(&before_2d.bgra, &after_2d.bgra, 640, 480, (320, 240), 200)
        .map_or(0, |c| c.count);
    assert!(
        changed_2d > 100_000,
        "2D toggle should repaint the board (changed={changed_2d})"
    );

    // 9. 2D Overhead Board: exact alignment across 3 uniform grid squares
    //    (c5 empty, e4 empty, g7 Black pawn — zero AI non-determinism).
    for (label, tx, ty) in [
        ("board-2d-c5", 254, 164),
        ("board-2d-e4", 342, 204),
        ("board-2d-g7", 430, 84),
    ] {
        let (_, _, ws_bgra) = move_and_sync_ws(label, tx, ty);
        assert_template_match(label, &ws_bgra, (tx, ty), Some(((tx, ty), 64)));
    }

    // Toggle back 2D -> 3D via `/watch` `click_poll (585, 200)`.
    let before_3d = bus.latest_frame().expect("before 3d return");
    watch_click_poll("toggle-3d-back", 585, 200);
    step_flips(&bus, 12);
    let after_3d = bus.latest_frame().expect("after 3d return");
    let changed_3d = diff_cluster_near(&before_3d.bgra, &after_3d.bgra, 640, 480, (320, 240), 200)
        .map_or(0, |c| c.count);
    assert!(
        changed_3d > 100_000,
        "3D return toggle should repaint the board (changed={changed_3d})"
    );

    // 10. `/watch` Take Over hover + `click_poll (hold_flips: 2, hold_ms: 150)` cleanly
    //     selects and plays `1. e2-e4` on the 3D board (unlike unhovered CLI `--poll`
    //     with `hold_flips: 0`, hovering first lets the board hit-test latch the square).
    let before_e2_move = bus.latest_frame().expect("before e2-e4");
    let _ = move_and_sync_ws("hover-e2", 356, 308);
    watch_click_poll("select-e2", 356, 308);
    step_flips(&bus, 4);
    let _ = move_and_sync_ws("hover-e4", 351, 241);
    watch_click_poll("move-e4", 351, 241);
    step_flips(&bus, 60);
    let after_e2_move = bus.latest_frame().expect("after e2-e4");
    let e2_changed = diff_cluster_near(
        &before_e2_move.bgra,
        &after_e2_move.bgra,
        640,
        480,
        (356, 308),
        20,
    )
    .map_or(0, |c| c.count);
    assert!(
        e2_changed > 500,
        "expected White pawn to vacate e2 after /watch click_poll (diff={e2_changed})"
    );

    // 11. Documented #235 (`wontfix`) check: pressing Tab calls `SetCursorPos(70, 60)`
    //     without posting `WM_MOUSEMOVE`, moving `pointer_pos` to `(70, 60)` while the
    //     rendered TGA cursor stays at pre-Tab `(80, 400)` until the next `WM_MOUSEMOVE`,
    //     which immediately re-synchronizes within `MAX_CURSOR_ERR_PX` (0 px observed).
    let _ = move_and_sync_ws("pre-tab-anchor", 80, 400);
    let resp = bus.dispatch_sync(
        Command::Act {
            action: Action::KeyHold {
                key: "Tab".to_owned(),
                hold_flips: 3,
                hold_ms: None,
            },
            auto_step: true,
        },
        Some(TIMEOUT),
    );
    assert!(resp.ok, "Tab key_hold failed: {:?}", resp.error);
    step_flips(&bus, 4);
    let after_tab = bus.status();
    assert_eq!(after_tab.pointer_pos, (70, 60), "Tab SetCursorPos target");
    assert_eq!(after_tab.hw_pointer_pos, (70, 60));
    let tab_frame = bus.latest_frame().expect("frame after Tab");
    assert_template_match(
        "tab-unchanged-tga",
        &tab_frame.bgra,
        (80, 400),
        Some(((80, 400), 32)),
    );

    let (_, _, ws_resync) = move_and_sync_ws("post-tab-resync", 320, 240);
    assert_template_match(
        "post-tab-resync-ws",
        &ws_resync,
        (320, 240),
        Some(((320, 240), 64)),
    );

    // Verify WS v2 exercised both PNG keyframes and XOR+RLE delta frames.
    let (keyframes, deltas) = ws.counts();
    assert!(keyframes > 0, "expected >=1 WS v2 PNG keyframe");
    assert!(deltas > 0, "expected >=1 WS v2 XOR+RLE delta");

    // Cleanly terminate guest thread (mid-match ignores WM_QUIT, so force-close).
    let _ = bus.dispatch_forced(Command::Close { force: true }, true);
    let _finish = guest_thread.join().expect("guest thread exits");
    server.stop();
}
