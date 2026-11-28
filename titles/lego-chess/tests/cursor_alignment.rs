//! Real-disc cursor-to-pointer alignment and `/watch` WebSocket v2 proof for
//! LEGO Chess (#330, #411).
//!
//! Boots LEGO Chess headless under [`ControlRig::in_process`] + live draining
//! [`SpectatorClient`] (`/api/meta`, `/api/mode`, `/api/act`, `/api/stream` WS v2)
//! against the player's retail disc. Skips cleanly when the disc is absent
//! unless `LEGO_LEGACY_REQUIRE_DISC=1`.

use lego_chess::LEGO_CHESS as CHESS;
use lego_legacy_core::cli::Cli;
use lego_legacy_core::control::observe::{render_frame_png, ObserveOptions};
use lego_legacy_core::control::{Action, Command, ControlBus, FrameSnapshot};
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::spectator::{crosshair_center, ControlRig};
use std::path::PathBuf;
use std::time::Duration;

const BUDGET: u64 = 8_000_000_000;
const TIMEOUT: Duration = Duration::from_secs(60);
const WS_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CURSOR_ERR_PX: i32 = 2;

/// LEGO Chess's `graphics\pointers\*.tga` pointer sprite spans
/// `[x + 1 ..= x + 54, y - 3 ..= y + 42]` relative to the `WM_MOUSEMOVE`
/// hotspot `(x, y)`.
const TGA_BBOX_DX: i32 = 1;
const TGA_BBOX_DY: i32 = -3;

/// Minimal boot route: skip Krisalis' logo (flip 290) and intro (flip 300)
/// with poll-released Space, landing on the New Player form (~flip 307) with
/// zero mouse moves issued yet.
const BOOT_ROUTE: &str = "\
#lego-legacy-route 1\n\
flip 290 key Space down\n\
flip 290 poll 300 key Space up\n\
flip 300 key Space down\n\
flip 300 poll 300 key Space up\n";

struct RouteFileGuard(PathBuf);
impl Drop for RouteFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
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

#[test]
fn lego_chess_rendered_cursor_matches_pointer_and_ws_v2_across_screens() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    if !testkit::rips(&CHESS.image) {
        return;
    }

    let boot_route = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("chess-cursor-boot-{}.route", std::process::id()));
    std::fs::write(&boot_route, BOOT_ROUTE).expect("write boot route");
    let _route_guard = RouteFileGuard(boot_route.clone());

    let cli = Cli {
        route: Some(boot_route),
        ..Cli::default()
    };
    let rig = ControlRig::in_process(&CHESS, cli, BUDGET).expect("start LEGO Chess control rig");
    let bus = rig.bus().expect("in-process ControlBus");
    let http = rig.http();

    let meta_json = http.meta().expect("GET /api/meta");
    let mm = meta_json.as_object().unwrap()["guide"].as_object().unwrap()["mouse_mode"]
        .as_object()
        .unwrap();
    assert_eq!(mm["kind"].as_str(), Some("software_absolute"));
    assert_eq!(mm["requires_poll_release"].as_bool(), Some(true));

    // Pre-boot init sync (#327): SoftwareAbsolute starts at (320, 240), hidden.
    let paused = bus.wait_until_paused(Some(TIMEOUT));
    assert!(
        paused.paused && !paused.finished,
        "expected paused post-boot state: {paused:?}"
    );
    assert_eq!(paused.pointer_pos, (320, 240));
    assert_eq!(paused.hw_pointer_pos, (320, 240));
    assert!(!paused.pointer_shown);

    let to_312 = 312u64.saturating_sub(paused.flip.unwrap_or(0));
    if to_312 > 0 {
        step_flips(bus, to_312);
    }

    let ws = rig.connect().expect("connect SpectatorClient");
    let f_init = bus.latest_frame().expect("initial New Player frame");
    assert_eq!(f_init.pointer_pos, (320, 240));
    let ws_f0 = ws
        .wait_for_matching_frame(&f_init.bgra, (320, 240), false, 0, f_init.flip, WS_TIMEOUT)
        .expect("initial New Player WS frame");
    assert_eq!(ws_f0.bgra, *f_init.bgra);

    let move_and_sync_ws =
        |label: &str, tx: i32, ty: i32| -> (FrameSnapshot, FrameSnapshot, Vec<u8>) {
            let before = bus.latest_frame().expect("before frame");
            ws.act(&Action::Move {
                x: tx,
                y: ty,
                glide_flips: 1,
            })
            .unwrap_or_else(|e| panic!("move {label} failed: {e}"));
            step_flips(bus, 2);
            let after = bus.latest_frame().expect("after frame");
            let st = bus.status();
            let wf = ws
                .wait_for_matching_frame(
                    &after.bgra,
                    (tx, ty),
                    false,
                    before.flip,
                    after.flip,
                    WS_TIMEOUT,
                )
                .unwrap_or_else(|e| panic!("{label}: WS sync failed: {e}"));
            assert_eq!(st.pointer_pos, (tx, ty), "{label}: status.pointer_pos");
            assert_eq!(after.pointer_pos, (tx, ty), "{label}: snap.pointer_pos");
            assert_eq!(wf.bgra, *after.bgra, "{label}: WS BGRA bit-exact match");

            let png = render_frame_png(
                &after,
                ObserveOptions {
                    cursor: true,
                    ..ObserveOptions::default()
                },
                after.pointer_pos,
            )
            .expect("render cursor png");
            let cross = crosshair_center(&after.bgra, &png).expect("decode crosshair");
            assert_eq!(
                cross,
                Some(wf.pointer_pos),
                "{label}: --cursor crosshair matches ws_ptr"
            );
            (before, after, wf.bgra)
        };

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

    ws.post_json("/api/mode", r#"{"owner":"shared"}"#)
        .expect("POST /api/mode shared");

    let watch_click_poll = |label: &str, x: i32, y: i32| {
        let body = format!(
            "{{\"action\":\"click_poll\",\"x\":{x},\"y\":{y},\"button\":\"left\",\"hold_flips\":2,\"hold_ms\":150,\"source\":\"human\",\"wait\":true,\"auto_step\":true}}"
        );
        ws.act_json(&body)
            .unwrap_or_else(|e| panic!("watch click_poll {label}: {e}"));
        let st = bus.status();
        assert!(
            st.paused && !st.finished,
            "{label}: expected paused status after click_poll: {st:?}"
        );
        assert_eq!(st.pointer_pos, (x, y), "{label}: pointer_pos after click");
    };

    // New Player green tick -> Who's Playing -> Throne Room.
    watch_click_poll("new-player-tick", 576, 420);
    step_flips(bus, 10);
    watch_click_poll("whos-playing-tick", 576, 420);
    step_flips(bus, 15);

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
    step_flips(bus, 28);
    watch_click_poll("throne-chess-table", 385, 285);
    step_flips(bus, 10);

    for (label, tx, ty) in [
        ("quick-game-1", 160, 130),
        ("quick-game-2", 480, 260),
        ("quick-game-3", 320, 380),
    ] {
        let (_, _, ws_bgra) = move_and_sync_ws(label, tx, ty);
        assert_template_match(label, &ws_bgra, (tx, ty), Some(((tx, ty), 64)));
    }

    let before_board_flip = bus.status().flip.unwrap_or(0);
    watch_click_poll("quick-game-start", 590, 410);
    step_flips(bus, 120);
    let board_ready = bus.latest_frame().expect("board ready frame");
    let board_ws = ws
        .wait_for_matching_frame(
            &board_ready.bgra,
            board_ready.pointer_pos,
            false,
            before_board_flip,
            board_ready.flip,
            WS_TIMEOUT,
        )
        .expect("3D board WS sync");
    assert_eq!(board_ws.bgra, *board_ready.bgra);

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

    // Toggle 3D -> 2D overhead board before any AI move so board state is deterministic.
    let before_2d = bus.latest_frame().expect("before 2d toggle");
    watch_click_poll("toggle-2d", 440, 60);
    step_flips(bus, 12);
    let after_2d = bus.latest_frame().expect("after 2d toggle");
    let changed_2d = diff_cluster_near(&before_2d.bgra, &after_2d.bgra, 640, 480, (320, 240), 200)
        .map_or(0, |c| c.count);
    assert!(
        changed_2d > 100_000,
        "2D toggle should repaint the board (changed={changed_2d})"
    );

    for (label, tx, ty) in [
        ("board-2d-c5", 254, 164),
        ("board-2d-e4", 342, 204),
        ("board-2d-g7", 430, 84),
    ] {
        let (_, _, ws_bgra) = move_and_sync_ws(label, tx, ty);
        assert_template_match(label, &ws_bgra, (tx, ty), Some(((tx, ty), 64)));
    }

    let before_3d = bus.latest_frame().expect("before 3d return");
    watch_click_poll("toggle-3d-back", 585, 200);
    step_flips(bus, 12);
    let after_3d = bus.latest_frame().expect("after 3d return");
    let changed_3d = diff_cluster_near(&before_3d.bgra, &after_3d.bgra, 640, 480, (320, 240), 200)
        .map_or(0, |c| c.count);
    assert!(
        changed_3d > 100_000,
        "3D return toggle should repaint the board (changed={changed_3d})"
    );

    // Hover + `click_poll` selects and plays `1. e2-e4` on the 3D board.
    let before_e2_move = bus.latest_frame().expect("before e2-e4");
    let _ = move_and_sync_ws("hover-e2", 356, 308);
    watch_click_poll("select-e2", 356, 308);
    step_flips(bus, 4);
    let _ = move_and_sync_ws("hover-e4", 351, 241);
    watch_click_poll("move-e4", 351, 241);
    step_flips(bus, 60);
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

    // Documented #235 (`wontfix`) check: pressing Tab calls `SetCursorPos(70, 60)`
    // without posting `WM_MOUSEMOVE`, moving `pointer_pos` to `(70, 60)` while the
    // rendered TGA cursor stays at pre-Tab `(80, 400)` until the next `WM_MOUSEMOVE`.
    let _ = move_and_sync_ws("pre-tab-anchor", 80, 400);
    ws.act(&Action::KeyHold {
        key: "Tab".to_owned(),
        hold_flips: 3,
        hold_ms: None,
    })
    .expect("Tab key_hold");
    step_flips(bus, 4);
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

    let (keyframes, deltas) = ws.counts();
    assert!(keyframes > 0, "expected >=1 WS v2 PNG keyframe");
    assert!(deltas > 0, "expected >=1 WS v2 XOR+RLE delta");
}
