//! Real-disc pointer-to-cursor alignment proof for LEGO Rock Raiders (#331, #411).
//!
//! Runs retail `LegoRR.exe` under [`ControlRig::in_process`] with live
//! `/api/meta`, `/api/act`, and draining `/api/stream` WebSocket v2
//! (`FRAME_FLAG_HAS_POINTER`), proving across Main Menu, Level Select, and
//! Training Mission 1 (`1. Moving Rock Raiders`) that:
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
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::{Action, Command, ControlBus};
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::spectator::{
    crosshair_center, fraction_diff, ControlRig, SpectatorClient,
};
use lego_rock_raiders::ROCK_RAIDERS;
use std::time::Duration;

const W: u32 = 640;
const H: u32 = 480;
const MAX_TIP_ERROR_PX: i32 = 2;
const SYNC_TIMEOUT: Duration = Duration::from_secs(60);
const WS_TIMEOUT: Duration = Duration::from_secs(15);

struct CursorCluster {
    tip: (i32, i32),
    pixels: usize,
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

fn verify_targets(
    bus: &ControlBus,
    ws: &SpectatorClient,
    phase: &str,
    prev_bgra: &mut Vec<u8>,
    prev_pos: &mut (i32, i32),
    targets: &[(i32, i32)],
) {
    for &(tx, ty) in targets {
        let min_flip = bus.status().flip.unwrap_or(0);
        ws.act(&Action::Move {
            x: tx,
            y: ty,
            glide_flips: 1,
        })
        .unwrap_or_else(|e| panic!("{phase}: move ({tx}, {ty}) failed: {e}"));
        // Let LegoRR.exe render at least 2 frames at the new pointer position.
        step_flips(bus, 2);

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

        let ws_frame = ws
            .wait_for_matching_frame(
                &snap.bgra,
                snap.pointer_pos,
                false,
                min_flip,
                snap.flip,
                WS_TIMEOUT,
            )
            .unwrap_or_else(|e| panic!("{phase}: WS sync ({tx}, {ty}): {e}"));
        assert_eq!((ws_frame.width, ws_frame.height), (W, H));
        assert_eq!(ws_frame.bgra, *snap.bgra, "{phase}: WS BGRA bit-exact");

        let cursor = detect_cursor_tip(prev_bgra, &ws_frame.bgra, *prev_pos, (tx, ty));
        let tip_err = (cursor.tip.0 - st.pointer_pos.0)
            .abs()
            .max((cursor.tip.1 - st.pointer_pos.1).abs());

        let crosshair_png = render_frame_png(
            &snap,
            ObserveOptions {
                cursor: true,
                ..ObserveOptions::default()
            },
            snap.pointer_pos,
        )
        .expect("render --cursor PNG");
        let cross_center =
            crosshair_center(&snap.bgra, &crosshair_png).expect("decode --cursor crosshair");
        assert_eq!(
            cross_center,
            Some(st.pointer_pos),
            "{phase}: --cursor crosshair center {cross_center:?} != pointer_pos {:?}",
            st.pointer_pos
        );

        eprintln!(
            "[#331 {phase}] target=({tx},{ty}) flip={} ws_flip={} pos={:?} \
             tip={:?} (err={}px, px={}) crosshair={:?}",
            snap.flip,
            ws_frame.flip,
            st.pointer_pos,
            cursor.tip,
            tip_err,
            cursor.pixels,
            cross_center
        );
        assert!(
            tip_err <= MAX_TIP_ERROR_PX,
            "{phase}: rendered cursor tip {:?} diverged from pointer_pos {:?} by {tip_err}px (> {MAX_TIP_ERROR_PX}px)",
            cursor.tip,
            st.pointer_pos
        );

        *prev_bgra = ws_frame.bgra;
        *prev_pos = (tx, ty);
    }
}

#[test]
fn real_disc_software_absolute_cursor_aligns_headlessly_and_over_ws() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    if !testkit::rips(&ROCK_RAIDERS.image) {
        return;
    }

    let rig = ControlRig::in_process(&ROCK_RAIDERS, Cli::default(), 12_000_000_000)
        .expect("start Rock Raiders control rig");
    let bus = rig.bus().expect("in-process ControlBus");
    let http = rig.http();

    let meta_json = http.meta().expect("GET /api/meta");
    let mm = meta_json.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    assert_eq!(mm["kind"].as_str(), Some("software_absolute"));
    assert_eq!(mm["requires_poll_release"].as_bool(), Some(false));

    let boot_st = bus.wait_until_paused(Some(Duration::from_secs(30)));
    assert!(
        boot_st.paused && boot_st.flip.is_some(),
        "failed to reach initial flip pause: finished={} outcome={:?}",
        boot_st.finished,
        boot_st.outcome
    );
    assert_eq!(boot_st.pointer_pos, (320, 240));
    assert!(!boot_st.pointer_shown);

    // Let `lmi640.avi` finish (~285 flips), skip `intro.avi` (3821 frames) with
    // `Escape` once it starts (~flip 350), then step through loading until Main Menu settles.
    step_flips(bus, 350);
    http.act(&Action::KeyHold {
        key: "Escape".to_owned(),
        hold_flips: 3,
        hold_ms: None,
    })
    .expect("skip intro");
    step_flips(bus, 40);

    let loading_frame = bus.latest_frame().expect("loading frame").bgra.clone();
    let mut reached_menu = false;
    let mut static_runs = 0u32;
    for _ in 0..80 {
        step_flips(bus, 15);
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

    step_flips(bus, 4);
    let menu_base_snap = bus.latest_frame().expect("menu baseline snap");
    assert!(
        !bus.status().pointer_shown,
        "Rock Raiders hides OS cursor on Main Menu"
    );
    let ws = rig.connect().expect("connect SpectatorClient");
    let menu_ws = ws
        .wait_for_matching_frame(
            &menu_base_snap.bgra,
            menu_base_snap.pointer_pos,
            false,
            menu_base_snap.flip,
            menu_base_snap.flip,
            WS_TIMEOUT,
        )
        .expect("initial menu WS keyframe");
    assert_eq!(menu_ws.bgra, *menu_base_snap.bgra);
    let mut prev_bgra = menu_ws.bgra;
    let mut prev_pos = menu_base_snap.pointer_pos;

    // Hover `Start Game` at (365, 222) -> (320, 222) so the button highlight
    // stays lit on both frames and the diff isolates the cursor sprite cleanly.
    verify_targets(
        bus,
        &ws,
        "main-menu",
        &mut prev_bgra,
        &mut prev_pos,
        &[(200, 150), (460, 320), (160, 380)],
    );
    let hover_min_flip = bus.status().flip.unwrap_or(0);
    ws.act(&Action::Move {
        x: 365,
        y: 222,
        glide_flips: 1,
    })
    .expect("hover Start Game");
    step_flips(bus, 2);
    let hover_snap = bus.latest_frame().expect("hover snap");
    assert_eq!(hover_snap.pointer_pos, (365, 222));
    let hover_ws = ws
        .wait_for_matching_frame(
            &hover_snap.bgra,
            hover_snap.pointer_pos,
            false,
            hover_min_flip,
            hover_snap.flip,
            WS_TIMEOUT,
        )
        .expect("hover WS sync");
    assert_eq!(hover_ws.bgra, *hover_snap.bgra);
    prev_bgra = hover_ws.bgra;
    prev_pos = hover_snap.pointer_pos;
    verify_targets(
        bus,
        &ws,
        "main-menu-start-game",
        &mut prev_bgra,
        &mut prev_pos,
        &[(320, 222)],
    );

    let menu_before_click = prev_bgra.clone();
    let ls_start_flip = bus.status().flip.unwrap_or(0);
    ws.act_json(
        r#"{"action":"click","x":320,"y":222,"button":"left","hold_flips":2,"source":"human"}"#,
    )
    .expect("POST /api/act Start Game click");
    step_flips(bus, 95);
    let ls_snap = bus.latest_frame().expect("level select snap");
    assert!(
        fraction_diff(&menu_before_click, &ls_snap.bgra) > 0.35,
        "Start Game click did not transition from Main Menu to Level Select"
    );
    let ls_ws = ws
        .wait_for_matching_frame(
            &ls_snap.bgra,
            ls_snap.pointer_pos,
            false,
            ls_start_flip,
            ls_snap.flip,
            WS_TIMEOUT,
        )
        .expect("Level Select WS sync");
    assert_eq!(ls_ws.bgra, *ls_snap.bgra);
    prev_bgra = ls_ws.bgra;
    prev_pos = ls_snap.pointer_pos;

    verify_targets(
        bus,
        &ws,
        "level-select",
        &mut prev_bgra,
        &mut prev_pos,
        &[(240, 200), (420, 300)],
    );

    // Click Training Mission 1 (`1. Moving Rock Raiders`) at (155, 160), dismiss
    // Mission Brief with Space, and pause (`P`) to freeze 3D cavern animation.
    let ls_before_click = prev_bgra.clone();
    let m1_start_flip = bus.status().flip.unwrap_or(0);
    ws.act_json(
        r#"{"action":"click","x":155,"y":160,"button":"left","hold_flips":2,"source":"human"}"#,
    )
    .expect("POST /api/act mission click");
    step_flips(bus, 110);
    for (key, settle) in [("Space", 25), ("Space", 25), ("P", 10)] {
        ws.act(&Action::KeyHold {
            key: key.to_owned(),
            hold_flips: 2,
            hold_ms: None,
        })
        .unwrap_or_else(|e| panic!("key_hold {key}: {e}"));
        step_flips(bus, settle);
    }

    let m1_snap = bus.latest_frame().expect("training mission 1 snap");
    // Mission load calls SetCursorPos(320, 240); SoftwareAbsolute reconciles it.
    assert_eq!(m1_snap.pointer_pos, (320, 240));
    assert!(
        fraction_diff(&ls_before_click, &m1_snap.bgra) > 0.35,
        "Mission click did not leave Level Select"
    );
    let m1_ws = ws
        .wait_for_matching_frame(
            &m1_snap.bgra,
            m1_snap.pointer_pos,
            false,
            m1_start_flip,
            m1_snap.flip,
            WS_TIMEOUT,
        )
        .expect("Training Mission 1 WS sync");
    assert_eq!(m1_ws.bgra, *m1_snap.bgra);
    prev_bgra = m1_ws.bgra;
    prev_pos = m1_snap.pointer_pos;

    verify_targets(
        bus,
        &ws,
        "training-mission-1",
        &mut prev_bgra,
        &mut prev_pos,
        &[(260, 240), (450, 180), (495, 12)],
    );

    // Unpause (`P`) and click Top Panel Priorities icon `(495, 12)` via HTTP `/api/act`
    // to prove `/watch` Take Over HUD targeting opens the Priorities panel (`398..540, 50..260`).
    ws.act(&Action::KeyHold {
        key: "P".to_owned(),
        hold_flips: 2,
        hold_ms: None,
    })
    .expect("unpause mission");
    step_flips(bus, 10);
    let before_prio = bus.latest_frame().expect("before prio").bgra.clone();
    ws.act_json(
        r#"{"action":"click","x":495,"y":12,"button":"left","hold_flips":2,"source":"human"}"#,
    )
    .expect("POST /api/act priorities click");
    step_flips(bus, 25);
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
