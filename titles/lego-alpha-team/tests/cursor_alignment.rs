//! Real-disc headless + live WebSocket cursor-alignment proof for LEGO Alpha
//! Team (`RelativeSteered`, #332).
//!
//! Boots the retail disc through `ControlHost<Headless>` + live loopback
//! `HttpServer` (`SpectatorRoutes`), proves `status.pointer_pos` starts at
//! `(0, 479)` (never pinned at `SetCursorPos(320, 240)`), drives multi-target
//! `move`, large single-call `mouse_delta` (owed-delta chunking), edge-clamp
//! recovery, `/watch` Human Take Over `POST /api/act` clicks into Single
//! Missions and `M0010` Training Area, and verifies the rendered yellow arrow
//! cursor matches `status.pointer_pos`, `--cursor` crosshair PNG, and live
//! `/api/stream` WebSocket v2 frame-locked pointer coordinates within `<= 2 px`.

use lego_alpha_team::{GUIDE, LEGO_ALPHA_TEAM as ALPHA_TEAM};
use lego_legacy_core::cli::Trace;
use lego_legacy_core::control::json::parse_json;
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::ws::{
    decode_binary_frame, decode_xor_rle, FRAME_FLAG_PNG_KEYFRAME, FRAME_FLAG_XOR_RLE_DELTA,
    OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG,
};
use lego_legacy_core::control::{
    Action, Command, ControlBus, ControlConfig, ControlHost, ExecMode, FrameSnapshot, HttpContext,
    HttpServer, InputOwner,
};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::host::Button;
use lego_legacy_core::inflate::inflate;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::testkit;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const W: u32 = 640;
const H: u32 = 480;
const SYNC_TIMEOUT: Duration = Duration::from_secs(120);

/// Measured offset from `DDMouse` hotspot `(x, y)` to the centroid of Alpha
/// Team's 15×13 yellow arrow cursor sprite (`Cursor_Arrow.tga`), verified on
/// the retail disc across Main Menu and in-mission targets.
const ARROW_CENTROID_OFFSET: (f64, f64) = (8.0, 3.0);

fn scratch_ready() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    testkit::rips(&ALPHA_TEAM.image)
}

fn fresh_profile_dir(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("alpha-cursor-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create profile dir");
    dir
}

/// Decode a `lego_legacy_core::png::encode` RGB PNG into dense BGRA pixels.
fn decode_png_to_bgra(png: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    assert_eq!(&png[12..16], b"IHDR", "IHDR chunk");
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
    assert!(zlib.len() >= 6, "valid zlib stream");
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
            other => panic!("unexpected PNG scanline filter {other}"),
        }
    }

    let mut bgra = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for px in rgb.as_chunks::<3>().0 {
        bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    (w, h, bgra)
}

/// Locate the `--cursor` crosshair center pixel (bright red `(255, 0, 0)` in RGB
/// flanked by bright cyan `(0, 255, 255)` arms) inside a rendered PNG.
fn find_crosshair_center(png: &[u8]) -> (i32, i32) {
    let (w, h, bgra) = decode_png_to_bgra(png);
    let is_cyan = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return false;
        }
        let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
        bgra[i] == 255 && bgra[i + 1] == 255 && bgra[i + 2] == 0
    };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = ((y as usize) * (w as usize) + (x as usize)) * 4;
            if bgra[i] == 0 && bgra[i + 1] == 0 && bgra[i + 2] == 255 {
                let arms = [
                    is_cyan(x - 4, y),
                    is_cyan(x + 4, y),
                    is_cyan(x, y - 4),
                    is_cyan(x, y + 4),
                ]
                .into_iter()
                .filter(|&ok| ok)
                .count();
                if arms >= 2 {
                    return (x, y);
                }
            }
        }
    }
    panic!("--cursor crosshair red center not found in PNG");
}

#[derive(Debug, Clone, Copy)]
struct ArrowDetection {
    count: usize,
    tip: (i32, i32),
    centroid: (f64, f64),
    hotspot_from_centroid: (f64, f64),
}

fn is_cursor_yellow_bgra(px: &[u8]) -> bool {
    let (b, g, r) = (px[0], px[1], px[2]);
    r > 170 && g > 150 && b < 100
}

/// Extract Alpha Team's rendered yellow arrow cursor by XOR/color diffing
/// `curr_bgra` against `prev_bgra`, finding pixels newly painted in cursor
/// yellow and selecting the densest `37×37` (`±18 px`) cluster (ignoring small
/// background idle animations without taking any coordinate hint).
fn detect_yellow_arrow(
    prev_bgra: &[u8],
    curr_bgra: &[u8],
    region: Option<(i32, i32, i32)>,
) -> ArrowDetection {
    let mut pts: Vec<(i32, i32)> = Vec::new();
    let (x0, x1, y0, y1) = match region {
        Some((cx, cy, rad)) => (
            (cx - rad).max(0),
            (cx + rad).min(W as i32 - 1),
            (cy - rad).max(0),
            (cy + rad).min(H as i32 - 1),
        ),
        None => (0, W as i32 - 1, 0, H as i32 - 1),
    };
    for y in y0..=y1 {
        let row = (y as usize) * (W as usize) * 4;
        for x in x0..=x1 {
            let i = row + (x as usize) * 4;
            let a = &prev_bgra[i..i + 4];
            let b = &curr_bgra[i..i + 4];
            if is_cursor_yellow_bgra(b) && !is_cursor_yellow_bgra(a) {
                pts.push((x, y));
            }
        }
    }
    assert!(
        pts.len() >= 30,
        "expected >= 30 newly drawn yellow arrow pixels, found {}",
        pts.len()
    );

    let mut best_cluster: Vec<(i32, i32)> = Vec::new();
    for &(ax, ay) in &pts {
        let cluster: Vec<(i32, i32)> = pts
            .iter()
            .copied()
            .filter(|&(x, y)| (x - ax).abs() <= 18 && (y - ay).abs() <= 18)
            .collect();
        if cluster.len() > best_cluster.len() {
            best_cluster = cluster;
        }
    }
    assert!(
        best_cluster.len() >= 50,
        "expected >= 50 clustered yellow arrow pixels, found {}",
        best_cluster.len()
    );

    let count = best_cluster.len();
    let sum_x: i64 = best_cluster.iter().map(|&(x, _)| i64::from(x)).sum();
    let sum_y: i64 = best_cluster.iter().map(|&(_, y)| i64::from(y)).sum();
    let cx = sum_x as f64 / count as f64;
    let cy = sum_y as f64 / count as f64;

    // Leftmost yellow pixel sits 1 px right of the arrow's black outline tip.
    let min_x = best_cluster.iter().map(|&(x, _)| x).min().unwrap();
    let left_ys: Vec<i32> = best_cluster
        .iter()
        .filter(|&&(x, _)| x == min_x)
        .map(|&(_, y)| y)
        .collect();
    let tip_y = left_ys.iter().sum::<i32>() / left_ys.len() as i32;
    let tip = (min_x - 1, tip_y);

    ArrowDetection {
        count,
        tip,
        centroid: (cx, cy),
        hotspot_from_centroid: (cx - ARROW_CENTROID_OFFSET.0, cy - ARROW_CENTROID_OFFSET.1),
    }
}

fn read_server_ws_frame(reader: &mut BufReader<TcpStream>) -> (u8, Vec<u8>) {
    let mut hdr = [0u8; 2];
    reader.read_exact(&mut hdr).expect("read ws header");
    let opcode = hdr[0] & 0x0f;
    let len7 = hdr[1] & 0x7f;
    let len: usize = match len7 {
        0..=125 => usize::from(len7),
        126 => {
            let mut ext = [0u8; 2];
            reader.read_exact(&mut ext).expect("read ws len16");
            usize::from(u16::from_be_bytes(ext))
        }
        _ => {
            let mut ext = [0u8; 8];
            reader.read_exact(&mut ext).expect("read ws len64");
            u64::from_be_bytes(ext) as usize
        }
    };
    let mut payload = vec![0u8; len];
    if len > 0 {
        reader.read_exact(&mut payload).expect("read ws payload");
    }
    (opcode, payload)
}

struct WsClient {
    reader: BufReader<TcpStream>,
    bgra: Vec<u8>,
    last_flip: Option<u64>,
    last_pointer: Option<((i32, i32), bool)>,
    keyframes: usize,
    deltas: usize,
}

impl WsClient {
    fn connect(addr: SocketAddr) -> Self {
        let mut sock = TcpStream::connect(addr).expect("connect WS");
        sock.set_read_timeout(Some(Duration::from_secs(10)))
            .expect("set ws timeout");
        write!(
            sock,
            "GET /api/stream?fps=60 HTTP/1.1\r\n\
             Host: localhost\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .expect("write ws upgrade");
        sock.flush().expect("flush ws upgrade");

        let mut reader = BufReader::new(sock);
        let mut status_line = String::new();
        reader.read_line(&mut status_line).expect("ws status line");
        assert!(
            status_line.starts_with("HTTP/1.1 101"),
            "expected 101 Switching Protocols, got {status_line:?}"
        );
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("ws header line");
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        Self {
            reader,
            bgra: Vec::new(),
            last_flip: None,
            last_pointer: None,
            keyframes: 0,
            deltas: 0,
        }
    }

    fn sync_to_snapshot(
        &mut self,
        snap: &FrameSnapshot,
        require_exact_flip: bool,
    ) -> ((i32, i32), bool) {
        let matches = |this: &Self| {
            let exact_flip = this.last_flip == Some(snap.flip);
            let same_content = this.last_flip.is_some()
                && this.bgra == *snap.bgra
                && this.last_pointer == Some((snap.pointer_pos, snap.pointer_shown));
            if require_exact_flip {
                exact_flip
            } else {
                exact_flip || same_content
            }
        };
        while !matches(self) {
            let (op, payload) = read_server_ws_frame(&mut self.reader);
            match op {
                OPCODE_PING => {
                    let inner = self.reader.get_mut();
                    let mask = [0x12u8, 0x34, 0x56, 0x78];
                    let len = payload.len().min(125);
                    let mut pong = Vec::with_capacity(6 + len);
                    pong.push(0x80 | OPCODE_PONG);
                    pong.push(0x80 | len as u8);
                    pong.extend_from_slice(&mask);
                    for (i, &b) in payload[..len].iter().enumerate() {
                        pong.push(b ^ mask[i & 3]);
                    }
                    let _ = inner.write_all(&pong);
                    let _ = inner.flush();
                }
                OPCODE_BINARY => {
                    let hdr = decode_binary_frame(&payload).expect("decode WS v2 binary frame");
                    assert_eq!((u32::from(hdr.width), u32::from(hdr.height)), (W, H));
                    let ptr = hdr.pointer.expect("WS v2 frame-locked pointer extension");
                    match hdr.kind {
                        FRAME_FLAG_PNG_KEYFRAME => {
                            let (pw, ph, decoded) = decode_png_to_bgra(hdr.payload);
                            assert_eq!((pw, ph), (W, H));
                            self.bgra = decoded;
                            self.keyframes += 1;
                        }
                        FRAME_FLAG_XOR_RLE_DELTA => {
                            self.bgra =
                                decode_xor_rle(&self.bgra, hdr.payload).expect("decode XOR-RLE");
                            self.deltas += 1;
                        }
                        other => panic!("unexpected WS binary frame kind {other}"),
                    }
                    self.last_flip = Some(u64::from(hdr.flip));
                    self.last_pointer = Some(ptr);
                }
                other => {
                    panic!(
                        "unexpected WS opcode {other:#x} waiting for flip {}",
                        snap.flip
                    )
                }
            }
        }
        self.last_pointer.expect("pointer synced")
    }

    fn close(mut self) {
        let inner = self.reader.get_mut();
        let _ = inner.write_all(&[0x80 | OPCODE_CLOSE, 0x80, 1, 2, 3, 4]);
        let _ = inner.flush();
    }
}

fn http_request(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> String {
    let mut sock = TcpStream::connect(addr).expect("connect HTTP");
    sock.set_read_timeout(Some(SYNC_TIMEOUT))
        .expect("set http timeout");
    match body {
        Some(payload) => {
            write!(
                sock,
                "{method} {path} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{payload}",
                payload.len()
            )
            .expect("write HTTP POST");
        }
        None => {
            write!(
                sock,
                "{method} {path} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Connection: close\r\n\r\n"
            )
            .expect("write HTTP GET");
        }
    }
    sock.flush().expect("flush HTTP");
    let mut resp = Vec::new();
    sock.read_to_end(&mut resp).expect("read HTTP response");
    let text = String::from_utf8(resp).expect("UTF-8 HTTP response");
    let (head, body) = text.split_once("\r\n\r\n").expect("HTTP header separator");
    assert!(
        head.starts_with("HTTP/1.1 200 OK"),
        "{method} {path} failed: {head}\n{body}"
    );
    body.to_owned()
}

fn count_differing_pixels(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(pa, pb)| pa[..3] != pb[..3])
        .count()
}

#[test]
fn real_disc_relative_steered_cursor_aligns_headlessly_and_over_ws() {
    if !scratch_ready() {
        return;
    }

    let profile = fresh_profile_dir("main-menu-and-mission");
    let mut control = ControlHost::new(
        Headless::new(),
        ControlConfig {
            title: ALPHA_TEAM.key,
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            step_uncapped: true,
            pointer_speed: ALPHA_TEAM.pointer_speed,
            flip_cost: ALPHA_TEAM.flip_cost,
            guide: &GUIDE,
            ..ControlConfig::default()
        },
    );
    let bus: ControlBus = control.bus();

    // Initial startup contract before any flip: (0, 479) visual, (320, 240) hw.
    let init_status = bus.status();
    assert_eq!(init_status.pointer_pos, (0, 479));
    assert_eq!(init_status.hw_pointer_pos, (320, 240));
    assert!(!init_status.pointer_shown);

    let http_ctx = Arc::new(HttpContext::new(bus.clone(), &ALPHA_TEAM, None).with_guide(&GUIDE));
    let mut server = HttpServer::start("127.0.0.1:0", http_ctx).expect("start HTTP server");
    let addr = server.addr();

    // Verify /api/meta advertises RelativeSteered metadata for /watch Take Over.
    let meta = parse_json(&http_request(addr, "GET", "/api/meta", None)).expect("parse /api/meta");
    let mm = meta.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    assert_eq!(mm["kind"].as_str(), Some("relative_steered"));
    assert_eq!(mm["recenter_every_flip"].as_bool(), Some(true));
    assert_eq!(
        mm["virtual_span"].as_array().unwrap()[0].as_u64(),
        Some(1280)
    );
    assert_eq!(
        mm["virtual_span"].as_array().unwrap()[1].as_u64(),
        Some(1280)
    );
    assert_eq!(mm["recenter"].as_array().unwrap()[0].as_i64(), Some(320));
    assert_eq!(mm["recenter"].as_array().unwrap()[1].as_i64(), Some(240));
    assert_eq!(mm["start_corner"].as_array().unwrap()[0].as_i64(), Some(0));
    assert_eq!(
        mm["start_corner"].as_array().unwrap()[1].as_i64(),
        Some(480)
    );

    let emu_thread = thread::spawn(move || {
        let mut machine = Machine::new(&ALPHA_TEAM, &mut control).expect("create machine");
        machine.load_title(&ALPHA_TEAM).expect("load Alpha Team");
        machine.tree.set_profile(profile);
        let mut log = RunLog::create(ALPHA_TEAM.key, Trace::Off).expect("runlog");
        let finish = machine.run(&mut control, &mut log, u64::MAX);
        let digest = machine.state_digest();
        control.mark_finished(&finish, &digest);
    });

    // Wait for initial pause at flip 0, then boot fast to Main Menu (skip intro
    // at flip 450, Space past attract screen, settle on Main Menu).
    let s0 = bus.wait_until_paused(Some(SYNC_TIMEOUT));
    assert!(s0.paused, "expected initial pause at flip 0");

    let step = |flips: u64| {
        let resp = bus.dispatch_sync(Command::Step { flips }, Some(SYNC_TIMEOUT));
        assert!(resp.ok, "step({flips}) failed: {:?}", resp.error);
        resp.status
    };
    let act = |action: Action| {
        let resp = bus.dispatch_sync(
            Command::Act {
                action,
                auto_step: true,
            },
            Some(SYNC_TIMEOUT),
        );
        assert!(resp.ok, "act failed: {:?}", resp.error);
        resp.status
    };

    step(450);
    act(Action::KeyHold {
        key: "Escape".to_owned(),
        hold_flips: 3,
        hold_ms: None,
    });
    step(200);
    act(Action::KeyHold {
        key: "Space".to_owned(),
        hold_flips: 3,
        hold_ms: None,
    });
    let menu_status = step(320);
    assert_eq!(
        menu_status.pointer_pos,
        (0, 479),
        "Main Menu must start at bottom-left (0, 479), never pinned at (320, 240)"
    );
    assert_eq!(menu_status.hw_pointer_pos, (320, 240));
    assert!(!menu_status.pointer_shown);

    // Connect live loopback WebSocket client and verify initial v2 keyframe.
    let mut ws = WsClient::connect(addr);
    let snap_init = bus.latest_frame().expect("Main Menu snapshot");
    let (ws_ptr_init, ws_shown_init) = ws.sync_to_snapshot(&snap_init, true);
    assert_eq!(ws_ptr_init, (0, 479));
    assert!(!ws_shown_init);
    assert_eq!(ws.bgra, *snap_init.bgra);

    let mut prev_bgra = snap_init.bgra.as_ref().clone();

    let assert_aligned = |label: &str,
                          prev: &[u8],
                          ws: &mut WsClient,
                          expected_pos: (i32, i32),
                          region: Option<(i32, i32, i32)>|
     -> Vec<u8> {
        let st = bus.status();
        let snap = bus.latest_frame().expect("latest frame");
        assert_eq!(
            st.pointer_pos, expected_pos,
            "{label}: status.pointer_pos mismatch"
        );
        assert_eq!(
            st.hw_pointer_pos,
            (320, 240),
            "{label}: guest SetCursorPos(320,240) anchor must stay at (320,240)"
        );
        assert!(!st.pointer_shown, "{label}: OS pointer stays hidden");
        assert_eq!(snap.pointer_pos, st.pointer_pos);

        let (ws_ptr, ws_shown) = ws.sync_to_snapshot(&snap, true);
        assert_eq!(ws_ptr, st.pointer_pos, "{label}: WS v2 pointer mismatch");
        assert!(!ws_shown, "{label}: WS v2 pointer_shown mismatch");
        assert_eq!(
            ws.bgra, *snap.bgra,
            "{label}: WS reconstructed BGRA mismatch"
        );

        let crosshair_png = render_frame_png(
            &snap,
            ObserveOptions {
                cursor: true,
                ..ObserveOptions::default()
            },
            snap.pointer_pos,
        )
        .expect("render --cursor PNG");
        assert_eq!(
            find_crosshair_center(&crosshair_png),
            st.pointer_pos,
            "{label}: --cursor crosshair center mismatch"
        );

        let det = detect_yellow_arrow(prev, &ws.bgra, region);
        let dx_centroid = (det.hotspot_from_centroid.0 - f64::from(st.pointer_pos.0)).abs();
        let dy_centroid = (det.hotspot_from_centroid.1 - f64::from(st.pointer_pos.1)).abs();
        let dx_tip = (det.tip.0 - st.pointer_pos.0).abs();
        let dy_tip = (det.tip.1 - st.pointer_pos.1).abs();
        eprintln!(
            "[cursor_alignment] {label}: flip={} status={:?} ws={:?} count={} centroid=({:.1},{:.1}) tip={:?} err_centroid=({:.2},{:.2}) err_tip=({},{})",
            snap.flip,
            st.pointer_pos,
            ws_ptr,
            det.count,
            det.centroid.0,
            det.centroid.1,
            det.tip,
            dx_centroid,
            dy_centroid,
            dx_tip,
            dy_tip,
        );
        assert!(
            dx_centroid <= 2.0 && dy_centroid <= 2.0 && dx_tip <= 2 && dy_tip <= 2,
            "{label}: cursor drift exceeded 2 px: status={:?} ws={:?} det={det:?} \
             centroid_err=({dx_centroid:.2},{dy_centroid:.2}) tip_err=({dx_tip},{dy_tip})",
            st.pointer_pos,
            ws_ptr,
        );
        snap.bgra.as_ref().clone()
    };

    // 1. Multi-target absolute `move` translation on Main Menu (blind full-frame detection).
    act(Action::Move {
        x: 320,
        y: 240,
        glide_flips: 1,
    });
    prev_bgra = assert_aligned("move(320,240)", &prev_bgra, &mut ws, (320, 240), None);

    act(Action::Move {
        x: 480,
        y: 120,
        glide_flips: 1,
    });
    prev_bgra = assert_aligned("move(480,120)", &prev_bgra, &mut ws, (480, 120), None);

    act(Action::Move {
        x: 160,
        y: 360,
        glide_flips: 1,
    });
    prev_bgra = assert_aligned("move(160,360)", &prev_bgra, &mut ws, (160, 360), None);

    // 2. Large single-call `mouse_delta(+300, -200, flips=1)` owed-delta chunking -> (460, 160).
    act(Action::MouseDelta {
        dx: 300.0,
        dy: -200.0,
        flips: 1,
        scale_x: None,
        scale_y: None,
    });
    assert_aligned("delta(+300,-200,1f)", &prev_bgra, &mut ws, (460, 160), None);

    // 3. Edge-clamp recovery: slam bottom-right +100,+100 then return to (320, 240).
    act(Action::Move {
        x: 639,
        y: 479,
        glide_flips: 1,
    });
    act(Action::MouseDelta {
        dx: 100.0,
        dy: 100.0,
        flips: 1,
        scale_x: None,
        scale_y: None,
    });
    assert_eq!(bus.status().pointer_pos, (639, 479));
    let rb_snap = bus.latest_frame().unwrap();
    let rb_bgra = rb_snap.bgra.as_ref().clone();
    ws.sync_to_snapshot(&rb_snap, false);
    act(Action::Move {
        x: 320,
        y: 240,
        glide_flips: 1,
    });
    assert_aligned(
        "return_from_rb(320,240)",
        &rb_bgra,
        &mut ws,
        (320, 240),
        None,
    );

    // 4. Edge-clamp recovery: slam top-left -100,-100 then return to (320, 240).
    act(Action::Move {
        x: 0,
        y: 0,
        glide_flips: 1,
    });
    act(Action::MouseDelta {
        dx: -100.0,
        dy: -100.0,
        flips: 1,
        scale_x: None,
        scale_y: None,
    });
    assert_eq!(bus.status().pointer_pos, (0, 0));
    let tl_snap = bus.latest_frame().unwrap();
    let tl_bgra = tl_snap.bgra.as_ref().clone();
    ws.sync_to_snapshot(&tl_snap, false);
    act(Action::Move {
        x: 320,
        y: 240,
        glide_flips: 1,
    });
    prev_bgra = assert_aligned(
        "return_from_tl(320,240)",
        &tl_bgra,
        &mut ws,
        (320, 240),
        None,
    );

    assert!(
        ws.keyframes >= 1 && ws.deltas >= 1,
        "expected WS keyframe + XOR-RLE deltas on Main Menu (kf={} deltas={})",
        ws.keyframes,
        ws.deltas
    );
    // Close WS before long multi-second scene transitions so server's 2s
    // STREAM_WRITE_TIMEOUT doesn't drop an unread loopback TCP buffer, then
    // reconnect on Single Missions and in-mission to verify fresh v2 streams.
    ws.close();

    // 5. Simulate `/watch` Human Take Over click over HTTP POST /api/act on Single Missions (320, 175).
    let takeover_mode = http_request(addr, "POST", "/api/mode", Some(r#"{"owner":"shared"}"#));
    assert_eq!(
        parse_json(&takeover_mode).unwrap().as_object().unwrap()["ok"].as_bool(),
        Some(true)
    );
    let click_body = http_request(
        addr,
        "POST",
        "/api/act",
        Some(
            r#"{"action":"click","x":320,"y":175,"button":"left","hold_flips":2,"source":"human","wait":true,"auto_step":true}"#,
        ),
    );
    assert_eq!(
        parse_json(&click_body).unwrap().as_object().unwrap()["ok"].as_bool(),
        Some(true)
    );
    step(60);
    let missions_snap = bus.latest_frame().expect("Single Missions frame");
    let changed = count_differing_pixels(&prev_bgra, &missions_snap.bgra);
    assert!(
        changed > 80_000,
        "Take Over click(320, 175) should open Single Missions (only {changed} px changed)"
    );
    let mut ws_missions = WsClient::connect(addr);
    let (ws_ptr_missions, ws_shown_missions) = ws_missions.sync_to_snapshot(&missions_snap, true);
    assert_eq!(ws_ptr_missions, (320, 175));
    assert!(!ws_shown_missions);
    ws_missions.close();

    // 6. Launch M0010 Training Area via ControlBus clicks and prove in-mission alignment.
    act(Action::Click {
        x: Some(320),
        y: Some(130),
        button: Button::Left,
        hold_flips: 2,
        hold_ms: None,
        settle_flips: 1,
    });
    step(20);
    act(Action::Click {
        x: Some(135),
        y: Some(315),
        button: Button::Left,
        hold_flips: 2,
        hold_ms: None,
        settle_flips: 1,
    });
    step(300);
    // Dismiss "Destination: Alpha Team Training Area" modal with an in-place click.
    act(Action::Click {
        x: None,
        y: None,
        button: Button::Left,
        hold_flips: 2,
        hold_ms: None,
        settle_flips: 0,
    });
    step(30);
    let mission_before = bus.latest_frame().expect("mission start frame");
    assert_eq!(bus.status().pointer_pos, (135, 315));
    let mut ws_mission = WsClient::connect(addr);
    let (ws_ptr_m0, ws_shown_m0) = ws_mission.sync_to_snapshot(&mission_before, true);
    assert_eq!(ws_ptr_m0, (135, 315));
    assert!(!ws_shown_m0);

    // Relative delta inside M0010 Training Area -> (235, 265); restrict diff box
    // around target so the rotating 3D scene's yellow tile border isn't confused.
    act(Action::MouseDelta {
        dx: 100.0,
        dy: -50.0,
        flips: 2,
        scale_x: None,
        scale_y: None,
    });
    assert_aligned(
        "mission_delta(+100,-50)",
        &mission_before.bgra,
        &mut ws_mission,
        (235, 265),
        Some((235, 265, 35)),
    );
    assert!(
        ws_mission.keyframes >= 1 && ws_mission.deltas >= 1,
        "expected WS keyframe + XOR-RLE delta in-mission (kf={} deltas={})",
        ws_mission.keyframes,
        ws_mission.deltas
    );

    ws_mission.close();
    let _ = bus.dispatch(Command::Close { force: true });
    let _ = bus.dispatch(Command::SetMode {
        mode: ExecMode::Live,
        step_uncapped: None,
    });
    emu_thread.join().expect("emulator thread clean exit");
    server.stop();
}
