//! Real-disc headless + live loopback `/api/stream` WebSocket v2 cursor
//! alignment proof for LEGO Racers (`SoftwareRelative`, #374, #411).
//!
//! Runs retail Masterpiece `LEGORacers.exe` unflagged under
//! [`ControlRig::in_process`] with live draining [`SpectatorClient`] serving
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
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::{Action, Command, ControlBus};
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::spectator::{count_diff_pixels, crosshair_center, ControlRig};
use lego_racers::LEGO_RACERS as RACERS;
use std::time::Duration;

const W: u32 = 640;
const H: u32 = 480;
const MAX_ERR_PX: i32 = 2;
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const WS_TIMEOUT: Duration = Duration::from_secs(15);
const RUN_BUDGET: u64 = 4_000_000_000;

/// Unflagged boot route skipping skippable intro AVIs (`hvscmp.avi` via Space,
/// `introcmp.avi` via click) before DirectInput exclusive mouse acquisition.
const BOOT_ROUTE: &str = "\
#lego-legacy-route 1\n\
flip 240 key Space down\n\
flip 241 key Space up\n\
flip 245 move 320 240\n\
flip 246 down left\n\
flip 247 up left\n";

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

#[test]
fn real_disc_software_relative_cursor_aligns_headlessly_and_over_ws_across_menus_and_race() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    if !testkit::rips(&RACERS.image) {
        return;
    }

    let rig = ControlRig::in_process(&RACERS, Cli::default(), RUN_BUDGET)
        .expect("start LEGO Racers control rig");
    let bus = rig.bus().expect("in-process ControlBus");
    let http = rig.http();

    let meta = http.meta().expect("GET /api/meta");
    let mm = meta.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    let xy = |k: &str| {
        let a = mm[k].as_array().unwrap();
        (a[0].as_i64(), a[1].as_i64())
    };
    assert_eq!(mm["kind"].as_str(), Some("software_relative"));
    assert_eq!(mm["recenter_every_flip"].as_bool(), Some(false));
    assert_eq!(xy("virtual_span"), (Some(640), Some(480)));
    assert_eq!(xy("start_pos"), (Some(0), Some(0)));

    let ctrl_json = http.get_json("/api/controls").expect("GET /api/controls");
    assert_eq!(
        ctrl_json.as_object().unwrap()["escape_quits"].as_bool(),
        Some(false)
    );

    let goals_json = http.get_json("/api/goals").expect("GET /api/goals");
    assert_eq!(
        goals_json.as_object().unwrap()["goals"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    // Pre-boot initial state at flip 0 before any route input runs.
    let init_st = bus.wait_until_paused(Some(SYNC_TIMEOUT));
    assert!(
        init_st.paused && init_st.flip == Some(0) && !init_st.finished,
        "expected flip 0 pause: {init_st:?}"
    );
    assert_eq!(
        init_st.pointer_pos,
        (0, 0),
        "initial start_pos must be (0, 0)"
    );
    assert_eq!(init_st.hw_pointer_pos, (320, 240));
    assert!(!init_st.pointer_shown);

    http.pipe(BOOT_ROUTE, false, false)
        .expect("pipe boot route");
    step_flips(bus, 555);

    let mm_st = bus.status();
    assert_eq!(mm_st.pointer_pos, (0, 0), "Main Menu starts at (0, 0)");
    assert!(
        !mm_st.pointer_shown,
        "DISCL_EXCLUSIVE GUID_SysMouse must hide OS pointer (pointer_shown == false)"
    );
    let mm_init_snap = bus.latest_frame().expect("Main Menu init frame");
    assert_eq!(mm_init_snap.pointer_pos, (0, 0));
    assert!(!mm_init_snap.pointer_shown);

    let ws = rig.connect().expect("connect SpectatorClient");
    let ws_f0 = ws
        .wait_for_matching_frame(
            &mm_init_snap.bgra,
            (0, 0),
            false,
            mm_init_snap.flip,
            mm_init_snap.flip,
            WS_TIMEOUT,
        )
        .expect("initial Main Menu WS frame");
    assert_eq!((ws_f0.width, ws_f0.height), (W, H));
    assert_eq!(ws_f0.bgra, *mm_init_snap.bgra);

    // Move away once to (220, 140) to extract the exact opaque ARROW.BMP template
    // from the clean (0..32, 0..32) top-left corner.
    ws.act(&Action::Move {
        x: 220,
        y: 140,
        glide_flips: 1,
    })
    .expect("initial template move");
    step_flips(bus, 2);
    let clear_snap = bus.latest_frame().expect("post-move frame");
    let tpl = extract_arrow_template(&ws_f0.bgra, &clear_snap.bgra);

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

        let ws_frame = ws
            .wait_for_matching_frame(&snap.bgra, expected, false, min_flip, snap.flip, WS_TIMEOUT)
            .unwrap_or_else(|e| panic!("{phase}: WS sync failed: {e}"));
        assert_eq!((ws_frame.width, ws_frame.height), (W, H));
        assert_eq!(ws_frame.bgra, *snap.bgra, "{phase}: WS BGRA bit-exact");
        let (tip_x, tip_y, ratio) = locate_arrow_tip(&ws_frame.bgra, &tpl);
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
        let cross = crosshair_center(&snap.bgra, &crosshair_png).expect("decode crosshair");
        assert_eq!(cross, Some(expected), "{phase}: --cursor crosshair center");

        eprintln!(
            "[#374 {phase}] flip={} ws_flip={} expected={expected:?} tip=({tip_x},{tip_y}) \
             err=({dx},{dy})px match={ratio:.3} crosshair={cross:?}",
            snap.flip, ws_frame.flip
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

    let (init_x, init_y, init_ratio) = locate_arrow_tip(&ws_f0.bgra, &tpl);
    assert!(init_ratio >= 0.95 && init_x.abs() <= MAX_ERR_PX && init_y.abs() <= MAX_ERR_PX);
    assert_aligned_target("main-menu-1", (220, 140), mm_init_snap.flip + 1);

    for (label, (tx, ty)) in [
        ("main-menu-2", (420, 260)),
        ("main-menu-3", (160, 380)),
        ("main-menu-4", (560, 100)),
    ] {
        let min_flip = bus.status().flip.unwrap_or(0) + 1;
        ws.act(&Action::Move {
            x: tx,
            y: ty,
            glide_flips: 1,
        })
        .unwrap_or_else(|e| panic!("{label}: move failed: {e}"));
        step_flips(bus, 2);
        assert_aligned_target(label, (tx, ty), min_flip);
    }

    ws.post_json("/api/mode", r#"{"owner":"shared"}"#)
        .expect("POST /api/mode shared");

    let watch_move = |label: &str, tx: i32, ty: i32| {
        let min_flip = bus.status().flip.unwrap_or(0) + 1;
        let body = format!(
            r#"{{"action":"move","x":{tx},"y":{ty},"glide_flips":1,"source":"human","wait":true,"auto_step":true}}"#
        );
        ws.act_json(&body)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        step_flips(bus, 2);
        assert_aligned_target(label, (tx, ty), min_flip);
    };

    let watch_click = |label: &str, tx: i32, ty: i32, settle_after: u64| -> usize {
        let before = bus.latest_frame().expect("before click frame");
        let body = format!(
            r#"{{"action":"click","x":{tx},"y":{ty},"button":"left","hold_flips":2,"source":"human","wait":true,"auto_step":true}}"#
        );
        ws.act_json(&body)
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        step_flips(bus, settle_after);
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
        ws.act(&Action::KeyHold {
            key: key.to_owned(),
            hold_flips: 2,
            hold_ms: None,
        })
        .unwrap_or_else(|e| panic!("key_hold {key}: {e}"));
        step_flips(bus, settle);
    }
    let post_race_min_flip = bus.status().flip.unwrap_or(0) + 1;
    ws.act(&Action::KeyHold {
        key: "Return".to_owned(),
        hold_flips: 2,
        hold_ms: None,
    })
    .expect("confirm exit race");
    step_flips(bus, 160);

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
