//! LEGO Island pointer-to-cursor alignment & live `/watch` WebSocket v2 proof (#328, #411).
//!
//! Disc-gated integration test booting LEGO Island through [`ControlRig::in_process`]
//! + continuously draining [`SpectatorClient`] and proving:
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
use lego_legacy_core::control::json::JsonValue;
use lego_legacy_core::control::observe::ObserveOptions;
use lego_legacy_core::control::ws::FRAME_FLAG_PNG_KEYFRAME;
use lego_legacy_core::control::{Action, Command, ControlBus, ControlStatus};
use lego_legacy_core::png::decode_to_bgra;
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::spectator::{crosshair_center, ControlRig, SpectatorClient};
use std::path::PathBuf;
use std::time::Duration;

const FRAME_W: u32 = 640;
const FRAME_H: u32 = 480;
const RUN_BUDGET: u64 = 8_000_000_000;
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const WS_TIMEOUT: Duration = Duration::from_secs(10);

fn step_flips(bus: &ControlBus, flips: u64) {
    let resp = bus.dispatch_sync(Command::Step { flips }, Some(SYNC_TIMEOUT));
    assert!(
        resp.ok && resp.status.paused && !resp.status.finished,
        "step {flips} did not pause cleanly: ok={} paused={} finished={} err={:?}",
        resp.ok,
        resp.status.paused,
        resp.status.finished,
        resp.error
    );
}

fn json_xy(val: &JsonValue) -> (i32, i32) {
    let arr = val.as_array().expect("2-element coordinate array");
    assert_eq!(arr.len(), 2);
    (
        arr[0].as_i64().expect("x") as i32,
        arr[1].as_i64().expect("y") as i32,
    )
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
    bus: &ControlBus,
    ws: &SpectatorClient,
    max_iters: usize,
    flips_per_iter: u64,
    label: &str,
    pred: impl Fn(&[u8], &ControlStatus) -> bool,
) {
    for _ in 0..max_iters {
        let status = bus.status();
        let snap = bus.latest_frame().expect("latest frame");
        assert_eq!((snap.width, snap.height), (FRAME_W, FRAME_H));
        if pred(&snap.bgra, &status) {
            return;
        }
        ws.act(&Action::KeyHold {
            key: "Space".to_owned(),
            hold_flips: 2,
            hold_ms: None,
        })
        .expect("Space key_hold");
        step_flips(bus, flips_per_iter);
    }
    panic!("timed out waiting for {label}");
}

#[test]
fn lego_island_pointer_aligns_headlessly_and_over_watch_websocket() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    if !testkit::rips(&ISLAND.image) {
        return;
    }

    let rig = ControlRig::in_process(&ISLAND, &GUIDE, Cli::default(), RUN_BUDGET)
        .expect("start LEGO Island control rig");
    let data_root = rig.data_root().expect("rig data_root").to_path_buf();
    let bus = rig.bus().expect("in-process ControlBus");

    let paused = bus.wait_until_paused(Some(SYNC_TIMEOUT));
    assert!(
        paused.paused && paused.flip == Some(0),
        "expected flip 0 pause: {paused:?}"
    );

    let http = rig.http();

    let meta = http.meta().expect("GET /api/meta");
    assert_eq!(
        meta.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
            .as_object()
            .unwrap()["kind"]
            .as_str(),
        Some("absolute_kept")
    );

    // Stage 1: Blank cursor during intro (flip 456 -> SetCursor(0)).
    step_flips(bus, 460);
    let intro_status = http.status().expect("GET /api/status");
    let st_intro = intro_status.as_object().unwrap()["status"]
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
        http.get_json("/api/frame.json?grid=4x4")
            .expect("GET /api/frame.json")
            .as_object()
            .unwrap()["cursor_blank"]
            .as_bool(),
        Some(true)
    );
    let win32_intro = http.get_json("/api/win32").expect("GET /api/win32");
    let win32_intro_in = win32_intro.as_object().unwrap()["input"]
        .as_object()
        .unwrap();
    assert_eq!(json_xy(&win32_intro_in["pointer_pos"]), (320, 240));
    assert_eq!(win32_intro_in["cursor_handle"].as_str(), Some("0x0"));

    // Keep one continuously draining WS client open from flip 460 through every stage.
    let ws = rig.connect().expect("connect SpectatorClient");
    let ws_intro = ws
        .wait_for_any_frame(WS_TIMEOUT)
        .expect("WS intro keyframe");
    assert_eq!(ws_intro.kind, FRAME_FLAG_PNG_KEYFRAME);
    assert_eq!(ws_intro.flip, 460);
    assert_eq!((ws_intro.width, ws_intro.height), (FRAME_W, FRAME_H));
    assert_eq!(ws_intro.pointer_pos, (320, 240));
    assert!(ws_intro.pointer_shown);
    assert!(ws_intro.pointer_blank);

    let intro_png = ws
        .frame_png(ObserveOptions {
            cursor: true,
            ..ObserveOptions::default()
        })
        .expect("GET /api/frame.png?cursor=1");
    assert_eq!(
        crosshair_center(&ws_intro.bgra, &intro_png).expect("decode crosshair"),
        None,
        "--cursor must not draw crosshair when ISLE.EXE sets blank cursor SetCursor(0)"
    );
    let (iw, ih, intro_bgra) = decode_to_bgra(&intro_png).expect("decode intro PNG");
    assert_eq!((iw, ih), (FRAME_W, FRAME_H));
    assert_eq!(intro_bgra, ws_intro.bgra);
    maybe_save_proof("01-intro-blank-cursor.png", &intro_png);

    // Stage 2: Reach Information Center without any mouse move.
    wait_for_frame(bus, &ws, 8, 200, "Information Center", |bgra, status| {
        has_info_center_plants(bgra) && !status.cursor_blank
    });

    let assert_aligned = |min_flip: u64, expected: (i32, i32), label: &str| -> Vec<u8> {
        let status_json = ws.status().expect("GET /api/status");
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

        let win32 = ws.get_json("/api/win32").expect("GET /api/win32");
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

        let raw_png = ws
            .frame_png(ObserveOptions::default())
            .expect("GET /api/frame.png");
        let (rw, rh, raw_bgra) = decode_to_bgra(&raw_png).expect("decode raw PNG");
        assert_eq!((rw, rh), (FRAME_W, FRAME_H));

        let ws_frame = ws
            .wait_for_matching_frame(&raw_bgra, expected, true, min_flip, flip, WS_TIMEOUT)
            .unwrap_or_else(|e| panic!("{label}: WS sync failed: {e}"));
        assert_eq!((ws_frame.width, ws_frame.height), (FRAME_W, FRAME_H));
        assert!(!ws_frame.pointer_blank, "{label}: WS pointer_blank");
        assert_eq!(
            ws_frame.bgra, raw_bgra,
            "{label}: WS reconstructed framebuffer bit-exact"
        );

        let cursor_png = ws
            .frame_png(ObserveOptions {
                cursor: true,
                ..ObserveOptions::default()
            })
            .expect("GET /api/frame.png?cursor=1");
        assert_eq!(
            crosshair_center(&raw_bgra, &cursor_png).expect("decode crosshair"),
            Some(expected),
            "{label}: --cursor crosshair center"
        );
        cursor_png
    };

    let init_png = assert_aligned(461, (320, 240), "info-center-before-first-move");
    maybe_save_proof("02-info-center-init-320x240.png", &init_png);

    // Let the Infomaniac step aside to uncover the blue Registration Book at (322, 294).
    wait_for_frame(
        bus,
        &ws,
        12,
        120,
        "blue Registration Book uncovered",
        |bgra, _| {
            let [r, _, b] = rgb_at(bgra, 322, 294);
            b > 100 && b > r.saturating_add(40)
        },
    );

    // Stage 3: Live streaming WS v2 across multi-target moves.
    let (_, deltas_before_moves) = ws.counts();
    for (label, target) in [
        ("move-left-arrow", (28, 310)),
        ("move-map-pizzeria", (312, 206)),
        ("move-quit-door", (590, 260)),
        ("move-reg-book", (322, 294)),
    ] {
        let before_flip = bus.status().flip.unwrap_or(0);
        ws.act(&Action::Move {
            x: target.0,
            y: target.1,
            glide_flips: 2,
        })
        .unwrap_or_else(|e| panic!("{label}: {e}"));
        let png = assert_aligned(before_flip, target, label);
        if label == "move-reg-book" {
            maybe_save_proof("03-reg-book-target.png", &png);
        }
    }
    assert!(
        ws.counts().1 > deltas_before_moves,
        "live /api/stream WS v2 must deliver XOR+RLE delta frames alongside keyframes"
    );

    // Stage 4: Real integer CSS wide & narrow /watch Take Over clicks.
    let reg_book_narrow = watch_css_click_to_guest(360.0, 640.0, (322, 294));
    assert!(
        (reg_book_narrow.0 - 322).abs() <= 1 && (reg_book_narrow.1 - 294).abs() <= 1,
        "narrow viewport click quantized unexpectedly far: {reg_book_narrow:?}"
    );
    ws.act_json(&format!(
        r#"{{"action":"click","x":{},"y":{},"hold_flips":2,"wait":true,"auto_step":true}}"#,
        reg_book_narrow.0, reg_book_narrow.1
    ))
    .expect("click Registration Book");
    wait_for_frame(
        bus,
        &ws,
        16,
        120,
        "Registration Book green tick",
        |bgra, _| {
            let [r, g, b] = rgb_at(bgra, 519, 135);
            g > 100 && g > r.saturating_add(30) && g > b.saturating_add(30)
        },
    );

    ws.act(&Action::TypeText {
        text: "LEGO".to_owned(),
        flip_gap: 3,
    })
    .expect("type LEGO");
    step_flips(bus, 20);
    let tick_wide = watch_css_click_to_guest(1280.0, 900.0, (519, 135));
    assert_eq!(tick_wide, (519, 135));
    let before_tick = bus.status().flip.unwrap_or(0);
    ws.act_json(&format!(
        r#"{{"action":"click","x":{},"y":{},"hold_flips":2,"wait":true,"auto_step":true}}"#,
        tick_wide.0, tick_wide.1
    ))
    .expect("click Green Tick");
    let _ = assert_aligned(before_tick, tick_wide, "click-reg-book-tick-wide");

    wait_for_frame(
        bus,
        &ws,
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

    let pepper_narrow = watch_css_click_to_guest(360.0, 640.0, (322, 105));
    assert!(
        (pepper_narrow.0 - 322).abs() <= 1 && (pepper_narrow.1 - 105).abs() <= 1,
        "narrow viewport click quantized unexpectedly far: {pepper_narrow:?}"
    );
    let before_pepper = bus.status().flip.unwrap_or(0);
    ws.act_json(&format!(
        r#"{{"action":"click","x":{},"y":{},"hold_flips":2,"wait":true,"auto_step":true}}"#,
        pepper_narrow.0, pepper_narrow.1
    ))
    .expect("click Pepper portrait");
    let pepper_png = assert_aligned(before_pepper, pepper_narrow, "click-pepper-portrait-narrow");
    maybe_save_proof("04-pepper-portrait-click.png", &pepper_png);

    wait_for_frame(
        bus,
        &ws,
        16,
        120,
        "Pepper portrait framed in blue",
        |bgra, _| rgb_at(bgra, 304, 105) == [0, 0, 255],
    );

    // Stage 5: Drag verification.
    let drag_end = (420, 220);
    let before_drag = bus.status().flip.unwrap_or(0);
    ws.act_json(
        r#"{"action":"drag","from_x":322,"from_y":105,"to_x":420,"to_y":220,"step_px":16,"wait":true,"auto_step":true}"#,
    )
    .expect("drag verification");
    let drag_png = assert_aligned(before_drag, drag_end, "drag-end");
    maybe_save_proof("05-drag-end.png", &drag_png);

    assert!(
        !ws.is_closed(),
        "draining SpectatorClient must stay open across full run"
    );
}
