//! Real-disc headless + live WebSocket cursor-alignment proof for LEGO Alpha
//! Team (`RelativeSteered`, #332, #411).
//!
//! Boots the retail disc through [`ControlRig::in_process`] + continuously
//! draining [`SpectatorClient`], proves `status.pointer_pos` starts at
//! `(0, 479)` (never pinned at `SetCursorPos(320, 240)`), drives multi-target
//! `move`, large single-call `mouse_delta` (owed-delta chunking), edge-clamp
//! recovery, `/watch` Human Take Over `POST /api/act` clicks into Single
//! Missions and `M0010` Training Area, and verifies the rendered yellow arrow
//! cursor matches `status.pointer_pos`, `--cursor` crosshair PNG, and live
//! `/api/stream` WebSocket v2 frame-locked pointer coordinates within `<= 2 px`.

use lego_alpha_team::{GUIDE, LEGO_ALPHA_TEAM as ALPHA_TEAM};
use lego_legacy_core::cli::Cli;
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::{Action, Command, ControlBus};
use lego_legacy_core::host::Button;
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::spectator::{count_diff_pixels, crosshair_center, ControlRig};
use std::time::Duration;

const W: u32 = 640;
const H: u32 = 480;
const SYNC_TIMEOUT: Duration = Duration::from_secs(120);
const WS_TIMEOUT: Duration = Duration::from_secs(15);

/// Measured offset from `DDMouse` hotspot `(x, y)` to the centroid of Alpha
/// Team's 15×13 yellow arrow cursor sprite (`Cursor_Arrow.tga`), verified on
/// the retail disc across Main Menu and in-mission targets.
const ARROW_CENTROID_OFFSET: (f64, f64) = (8.0, 3.0);

#[derive(Debug, Clone, Copy)]
struct ArrowDetection {
    count: usize,
    tip: (i32, i32),
    centroid: (f64, f64),
    hotspot_from_centroid: (f64, f64),
}

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

#[test]
fn real_disc_relative_steered_cursor_aligns_headlessly_and_over_ws() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    if !testkit::rips(&ALPHA_TEAM.image) {
        return;
    }

    let rig = ControlRig::in_process(&ALPHA_TEAM, &GUIDE, Cli::default(), u64::MAX)
        .expect("start Alpha Team control rig");
    let bus = rig.bus().expect("in-process ControlBus");
    let http = rig.http().with_timeout(SYNC_TIMEOUT);

    let meta = http.meta().expect("GET /api/meta");
    let mm = meta.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    let xy = |k: &str| {
        let a = mm[k].as_array().unwrap();
        (a[0].as_i64(), a[1].as_i64())
    };
    assert_eq!(mm["kind"].as_str(), Some("relative_steered"));
    assert_eq!(mm["recenter_every_flip"].as_bool(), Some(true));
    assert_eq!(xy("virtual_span"), (Some(1280), Some(1280)));
    assert_eq!(xy("recenter"), (Some(320), Some(240)));
    assert_eq!(xy("start_corner"), (Some(0), Some(480)));

    let s0 = bus.wait_until_paused(Some(SYNC_TIMEOUT));
    assert!(s0.paused, "expected initial pause at flip 0");
    assert_eq!(s0.pointer_pos, (0, 479));
    assert_eq!(s0.hw_pointer_pos, (320, 240));
    assert!(!s0.pointer_shown);

    step_flips(bus, 450);
    http.act(&Action::KeyHold {
        key: "Escape".to_owned(),
        hold_flips: 3,
        hold_ms: None,
    })
    .expect("skip intro");
    step_flips(bus, 200);
    http.act(&Action::KeyHold {
        key: "Space".to_owned(),
        hold_flips: 3,
        hold_ms: None,
    })
    .expect("skip attract");
    step_flips(bus, 320);

    let menu_status = bus.status();
    assert_eq!(
        menu_status.pointer_pos,
        (0, 479),
        "Main Menu must start at bottom-left (0, 479), never pinned at (320, 240)"
    );
    assert_eq!(menu_status.hw_pointer_pos, (320, 240));
    assert!(!menu_status.pointer_shown);

    // Keep one draining WS v2 client open across Main Menu -> Single Missions -> M0010.
    let ws = rig
        .connect()
        .expect("connect SpectatorClient")
        .with_http_timeout(SYNC_TIMEOUT);
    let snap_init = bus.latest_frame().expect("Main Menu snapshot");
    let init_ws = ws
        .wait_for_matching_frame(
            &snap_init.bgra,
            (0, 479),
            false,
            snap_init.flip,
            snap_init.flip,
            WS_TIMEOUT,
        )
        .expect("initial Main Menu WS keyframe");
    assert_eq!((init_ws.width, init_ws.height), (W, H));
    assert_eq!(init_ws.bgra, *snap_init.bgra);

    let mut prev_bgra = snap_init.bgra.as_ref().clone();

    let assert_aligned = |label: &str,
                          prev: &[u8],
                          min_flip: u64,
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

        let ws_frame = ws
            .wait_for_matching_frame(
                &snap.bgra,
                st.pointer_pos,
                false,
                min_flip,
                snap.flip,
                WS_TIMEOUT,
            )
            .unwrap_or_else(|e| panic!("{label}: WS sync failed: {e}"));
        assert_eq!((ws_frame.width, ws_frame.height), (W, H));
        assert_eq!(
            ws_frame.bgra, *snap.bgra,
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
            crosshair_center(&snap.bgra, &crosshair_png).expect("decode crosshair"),
            Some(st.pointer_pos),
            "{label}: --cursor crosshair center mismatch"
        );

        let det = detect_yellow_arrow(prev, &ws_frame.bgra, region);
        let dx_centroid = (det.hotspot_from_centroid.0 - f64::from(st.pointer_pos.0)).abs();
        let dy_centroid = (det.hotspot_from_centroid.1 - f64::from(st.pointer_pos.1)).abs();
        let dx_tip = (det.tip.0 - st.pointer_pos.0).abs();
        let dy_tip = (det.tip.1 - st.pointer_pos.1).abs();
        eprintln!(
            "[cursor_alignment] {label}: flip={} status={:?} ws={:?} count={} centroid=({:.1},{:.1}) tip={:?} err_centroid=({:.2},{:.2}) err_tip=({},{})",
            snap.flip,
            st.pointer_pos,
            ws_frame.pointer_pos,
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
            ws_frame.pointer_pos,
        );
        snap.bgra.as_ref().clone()
    };

    for (label, (x, y)) in [
        ("move(320,240)", (320, 240)),
        ("move(480,120)", (480, 120)),
        ("move(160,360)", (160, 360)),
    ] {
        let min_flip = bus.status().flip.unwrap_or(0) + 1;
        ws.act(&Action::Move {
            x,
            y,
            glide_flips: 1,
        })
        .unwrap_or_else(|e| panic!("{label}: {e}"));
        prev_bgra = assert_aligned(label, &prev_bgra, min_flip, (x, y), None);
    }

    // Large single-call `mouse_delta(+300, -200, flips=1)` owed-delta chunking -> (460, 160).
    let min_flip = bus.status().flip.unwrap_or(0) + 1;
    ws.act(&Action::MouseDelta {
        dx: 300.0,
        dy: -200.0,
        flips: 1,
        scale_x: None,
        scale_y: None,
    })
    .expect("large mouse_delta");
    assert_aligned(
        "delta(+300,-200,1f)",
        &prev_bgra,
        min_flip,
        (460, 160),
        None,
    );

    // Edge-clamp recovery: slam bottom-right +100,+100 and top-left -100,-100,
    // returning to (320, 240) each time.
    for (label, (cx, cy), (dx, dy)) in [
        ("return_from_rb(320,240)", (639, 479), (100.0, 100.0)),
        ("return_from_tl(320,240)", (0, 0), (-100.0, -100.0)),
    ] {
        let clamp_min_flip = bus.status().flip.unwrap_or(0);
        ws.act(&Action::Move {
            x: cx,
            y: cy,
            glide_flips: 1,
        })
        .unwrap_or_else(|e| panic!("{label}: move corner: {e}"));
        ws.act(&Action::MouseDelta {
            dx,
            dy,
            flips: 1,
            scale_x: None,
            scale_y: None,
        })
        .unwrap_or_else(|e| panic!("{label}: slam corner: {e}"));
        assert_eq!(bus.status().pointer_pos, (cx, cy));
        let corner_snap = bus.latest_frame().unwrap();
        let corner_bgra = corner_snap.bgra.as_ref().clone();
        ws.wait_for_matching_frame(
            &corner_bgra,
            (cx, cy),
            false,
            clamp_min_flip,
            corner_snap.flip,
            WS_TIMEOUT,
        )
        .unwrap_or_else(|e| panic!("{label}: corner WS sync: {e}"));
        let ret_min_flip = bus.status().flip.unwrap_or(0) + 1;
        ws.act(&Action::Move {
            x: 320,
            y: 240,
            glide_flips: 1,
        })
        .unwrap_or_else(|e| panic!("{label}: return move: {e}"));
        prev_bgra = assert_aligned(label, &corner_bgra, ret_min_flip, (320, 240), None);
    }

    let (kf_menu, deltas_menu) = ws.counts();
    assert!(
        kf_menu >= 1 && deltas_menu >= 1,
        "expected WS keyframe + XOR-RLE deltas on Main Menu (kf={kf_menu} deltas={deltas_menu})"
    );

    let mode_resp = ws
        .post_json("/api/mode", r#"{"owner":"shared"}"#)
        .expect("POST /api/mode shared");
    assert_eq!(mode_resp.as_object().unwrap()["ok"].as_bool(), Some(true));
    let missions_min_flip = bus.status().flip.unwrap_or(0) + 1;
    ws.act_json(
        r#"{"action":"click","x":320,"y":175,"button":"left","hold_flips":2,"source":"human","wait":true,"auto_step":true}"#,
    )
    .expect("Take Over click Single Missions");
    step_flips(bus, 60);
    let missions_snap = bus.latest_frame().expect("Single Missions frame");
    let changed = count_diff_pixels(&prev_bgra, &missions_snap.bgra);
    assert!(
        changed > 80_000,
        "Take Over click(320, 175) should open Single Missions (only {changed} px changed)"
    );
    let ws_missions = ws
        .wait_for_matching_frame(
            &missions_snap.bgra,
            (320, 175),
            false,
            missions_min_flip,
            missions_snap.flip,
            WS_TIMEOUT,
        )
        .expect("Single Missions WS sync");
    assert_eq!(ws_missions.bgra, *missions_snap.bgra);

    ws.act(&Action::Click {
        x: Some(320),
        y: Some(130),
        button: Button::Left,
        hold_flips: 2,
        hold_ms: None,
        settle_flips: 1,
    })
    .expect("click Training Area entry");
    step_flips(bus, 20);
    let mission_min_flip = bus.status().flip.unwrap_or(0) + 1;
    ws.act(&Action::Click {
        x: Some(135),
        y: Some(315),
        button: Button::Left,
        hold_flips: 2,
        hold_ms: None,
        settle_flips: 1,
    })
    .expect("launch Training Area");
    step_flips(bus, 300);
    // Dismiss "Destination: Alpha Team Training Area" modal with an in-place click.
    ws.act(&Action::Click {
        x: None,
        y: None,
        button: Button::Left,
        hold_flips: 2,
        hold_ms: None,
        settle_flips: 0,
    })
    .expect("dismiss briefing modal");
    step_flips(bus, 30);
    let mission_before = bus.latest_frame().expect("mission start frame");
    assert_eq!(bus.status().pointer_pos, (135, 315));
    let ws_m0 = ws
        .wait_for_matching_frame(
            &mission_before.bgra,
            (135, 315),
            false,
            mission_min_flip,
            mission_before.flip,
            WS_TIMEOUT,
        )
        .expect("mission start WS sync");
    assert_eq!(ws_m0.bgra, *mission_before.bgra);

    let (kf_before_mission_move, deltas_before_mission_move) = ws.counts();

    // Restrict diff box around target so the rotating 3D scene's yellow tile border isn't confused.
    let delta_min_flip = bus.status().flip.unwrap_or(0) + 1;
    ws.act(&Action::MouseDelta {
        dx: 100.0,
        dy: -50.0,
        flips: 2,
        scale_x: None,
        scale_y: None,
    })
    .expect("in-mission mouse_delta");
    assert_aligned(
        "mission_delta(+100,-50)",
        &mission_before.bgra,
        delta_min_flip,
        (235, 265),
        Some((235, 265, 35)),
    );

    let (kf_end, deltas_end) = ws.counts();
    assert!(
        kf_end >= 1
            && deltas_end > deltas_menu
            && (kf_end > kf_before_mission_move || deltas_end > deltas_before_mission_move),
        "expected live WS frames across transitions and in-mission move \
         (kf={kf_end}, deltas={deltas_end}, menu_deltas={deltas_menu}, before_move=({kf_before_mission_move},{deltas_before_mission_move}))"
    );
    assert!(
        !ws.is_closed(),
        "draining SpectatorClient must stay open across full run"
    );
}
