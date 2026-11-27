//! Real-disc headless + live loopback `/api/stream` WebSocket v2 proof that
//! Hot Wheels' rendered red-arrow software cursor stays locked to `pointer_pos`,
//! `--cursor` crosshairs, and WS v2 frame-locked pointer coordinates before AND
//! after a 3D race's `SetCursorPos(320, 240)` transition (#329, #411).

use hotwheels_stunt_track_driver::HOT_WHEELS_STUNT_TRACK_DRIVER as HOT_WHEELS;
use lego_legacy_core::control::json::JsonValue;
use lego_legacy_core::control::observe::ObserveOptions;
use lego_legacy_core::png::decode_to_bgra;
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::spectator::{
    count_diff_pixels, crosshair_center, ControlRig, SpectatorClient,
};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

const W: u32 = 640;
const H: u32 = 480;
/// Maximum allowed pixel error on either axis between rendered red-arrow tip
/// and reported `pointer_pos` / WS v2 pointer / `--cursor` crosshair.
const MAX_ERR_PX: i32 = 2;
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);
const WS_TIMEOUT: Duration = Duration::from_secs(10);

fn assert_step_paused(resp: &JsonValue, label: &str) {
    let st = resp.as_object().unwrap()["status"].as_object().unwrap();
    assert!(
        st["paused"].as_bool() == Some(true) && st["finished"].as_bool() == Some(false),
        "{label}: step did not pause cleanly: {resp:?}"
    );
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

fn status_fields(ws: &SpectatorClient) -> (u64, (i32, i32), (i32, i32), bool) {
    let v = ws.status().expect("GET /api/status");
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
        let _ = writeln!(out, "flip {shifted} {tail}");
    }
    out
}

#[test]
fn cursor_stays_aligned_headlessly_and_over_ws_pre_and_post_race() {
    if !testkit::rips(&HOT_WHEELS.image) {
        return;
    }

    let data_root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("hotwheels-cursor-align-{}", std::process::id()));
    let rig = ControlRig::spawn_binary(
        Path::new(env!("CARGO_BIN_EXE_hotwheels-stunt-track-driver")),
        &data_root,
        &[],
    )
    .expect("spawn hotwheels-stunt-track-driver");
    let http = rig.http().with_timeout(HTTP_TIMEOUT);

    let meta = http.meta().expect("GET /api/meta");
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

    assert_step_paused(&http.step(285).expect("step past splash"), "splash");
    http.act_json(r#"{"action":"key_hold","key":"Space","hold_flips":9,"auto_step":true}"#)
        .expect("skip intro");
    assert_step_paused(&http.step(70).expect("settle Main Menu"), "main-menu");

    let ws = rig
        .connect()
        .expect("connect SpectatorClient")
        .with_http_timeout(HTTP_TIMEOUT);
    let _ = ws.wait_for_any_frame(WS_TIMEOUT).expect("initial WS frame");

    let grab_bgra = |label: &str| -> Vec<u8> {
        let png = ws
            .frame_png(ObserveOptions::default())
            .unwrap_or_else(|e| panic!("{label}: GET /api/frame.png: {e}"));
        let (w, h, bgra) = decode_to_bgra(&png).expect("decode PNG");
        assert_eq!((w, h), (W, H));
        bgra
    };

    let verify_three_targets = |phase: &str, targets: [(i32, i32); 3]| {
        let mut captures = Vec::with_capacity(3);
        for (tx, ty) in targets {
            let (before_flip, _, _, _) = status_fields(&ws);
            ws.act_json(&format!(
                r#"{{"action":"move","x":{tx},"y":{ty},"glide_flips":1,"auto_step":true}}"#
            ))
            .unwrap_or_else(|e| panic!("{phase}: move ({tx}, {ty}): {e}"));
            assert_step_paused(&ws.step(3).expect("settle move"), phase);

            let (flip, ptr, _hw, shown) = status_fields(&ws);
            assert!(!shown, "{phase}: Hot Wheels hides OS cursor");
            assert_eq!(
                ptr,
                (tx, ty),
                "{phase}: status.pointer_pos matches requested target"
            );

            let raw_bgra = grab_bgra(phase);
            let cursor_png = ws
                .frame_png(ObserveOptions {
                    cursor: true,
                    ..ObserveOptions::default()
                })
                .expect("GET /api/frame.png?cursor=1");
            assert_eq!(
                crosshair_center(&raw_bgra, &cursor_png).expect("decode crosshair"),
                Some(ptr),
                "{phase}: --cursor crosshair center must match pointer_pos"
            );

            // Spectator deduplicates unchanged settle flips, so WS flip lies in (before_flip..=flip).
            let ws_frame = ws
                .wait_for_pixels(before_flip + 1, &raw_bgra, WS_TIMEOUT)
                .unwrap_or_else(|e| panic!("{phase}: WS pixels ({tx}, {ty}): {e}"));
            assert_eq!((ws_frame.width, ws_frame.height), (W, H));
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
            assert_eq!(ws_frame.bgra, raw_bgra, "{phase}: WS BGRA bit-exact");
            captures.push(((tx, ty), ptr, raw_bgra));
        }

        for idx in 0..3 {
            let (tgt, ptr, raw_bgra) = &captures[idx];
            let o1 = &captures[(idx + 1) % 3];
            let o2 = &captures[(idx + 2) % 3];
            let (hl_tip, px_count) = isolate_cursor_tip(raw_bgra, &o1.2, &o2.2);

            let dx = (hl_tip.0 - ptr.0).abs();
            let dy = (hl_tip.1 - ptr.1).abs();
            eprintln!(
                "hotwheels cursor [{phase}] target={tgt:?} ptr={ptr:?} rendered_tip={hl_tip:?} err=({dx},{dy})px cluster={px_count}px"
            );
            assert!(
                dx <= MAX_ERR_PX && dy <= MAX_ERR_PX,
                "{phase}: target {tgt:?} desynced! reported={ptr:?} rendered_tip={hl_tip:?} err=({dx},{dy})px"
            );
        }
    };

    // Finish pre-race at (320, 240) so Track 1 route enters Championship cleanly from center.
    verify_three_targets("pre-race", [(270, 100), (120, 340), (320, 240)]);

    // Enter Championship, click Green GO (583, 430), and finish Track 1 to
    // trigger STUNT.EXE's real SetCursorPos(320, 240) race transitions.
    let (cur_flip, _, _, _) = status_fields(&ws);
    let pipe_start = cur_flip + 5;
    ws.pipe(&track1_route_shifted(pipe_start), false, false)
        .expect("pipe Track 1 route");

    let results_flip = pipe_start + (3339 - 382);
    loop {
        let (now, _, _, _) = status_fields(&ws);
        if now >= results_flip {
            break;
        }
        let chunk = (results_flip - now).min(600);
        assert_step_paused(&ws.step(chunk).expect("step Track 1 chunk"), "track1");
    }

    let (post_flip, post_ptr, post_hw, _) = status_fields(&ws);
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

    verify_three_targets("post-race", [(270, 100), (320, 240), (440, 220)]);

    // Verify cursor lands on Green GO (585, 430) post-race and first click advances.
    let (before_go_flip, _, _, _) = status_fields(&ws);
    let before_go_bgra = grab_bgra("before-go");
    ws.act_json(r#"{"action":"move","x":585,"y":430,"glide_flips":1,"auto_step":true}"#)
        .expect("move to Green GO");
    assert_step_paused(&ws.step(3).expect("settle Green GO hover"), "go-hover");
    let (go_flip, go_ptr, _, _) = status_fields(&ws);
    assert_eq!(go_ptr, (585, 430));
    let on_go_bgra = grab_bgra("on-go");
    let ws_go = ws
        .wait_for_pixels(before_go_flip + 1, &on_go_bgra, WS_TIMEOUT)
        .expect("WS Green GO frame");
    assert_eq!(ws_go.pointer_pos, (585, 430));
    assert_eq!(ws_go.bgra, on_go_bgra);
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

    ws.act_json(
        r#"{"action":"click","x":585,"y":430,"button":"left","hold_flips":6,"auto_step":true}"#,
    )
    .expect("click Green GO");
    assert_step_paused(&ws.step(60).expect("step after Green GO"), "after-go");
    let (after_flip, _, _, _) = status_fields(&ws);
    let after_go_bgra = grab_bgra("after-go");
    let changed = count_diff_pixels(&on_go_bgra, &after_go_bgra);
    eprintln!(
        "hotwheels post-race Green GO first-click screen diff: {changed} px (flip {go_flip} -> {after_flip})"
    );
    assert!(
        changed > 50_000,
        "first click on Green GO (585, 430) post-race must advance screen (only {changed} px changed)"
    );

    let (keyframes, deltas) = ws.counts();
    eprintln!("hotwheels WS v2 stream stats: keyframes={keyframes} deltas={deltas}");
    assert!(keyframes >= 1, "expected at least one WS v2 PNG keyframe");
    assert!(deltas >= 5, "expected multiple WS v2 XOR+RLE delta frames");
}
