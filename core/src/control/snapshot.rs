//! Live `LLSNAP v2` snapshot & instant restore control-plane handlers (#313).
//!
//! Serves `GET /api/snapshot`, `POST /api/snapshot`, `POST /api/restore`, and
//! instant in-place `LLSNAP v2` backing for `POST /api/bookmark` and
//! `POST /api/rewind` at flip boundaries without restarting the machine.

use super::bus::{analyze_frame, ControlBus, FrameSnapshot, RestoreSync};
use super::cursor::CursorModel;
use super::http::{HttpContext, HttpRequest, HttpResponse};
use super::inspect::{execute_inspect, BookmarkData, InspectQuery, InspectReply};
use super::json::{parse_json, write_json_f64, write_json_str, JsonValue};
use crate::host::{FlipMeta, Frame, Host};
use crate::machine::Machine;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);

/// One named in-memory `LLSNAP v2` snapshot stored on [`ControlBus`].
#[derive(Clone, Debug)]
pub struct SnapshotSlot {
    pub name: String,
    pub flip: u64,
    pub cursor: u64,
    pub declared_ms: u64,
    pub digest: String,
    pub bytes_len: usize,
    pub bytes: Arc<Vec<u8>>,
    pub frame: Option<FrameSnapshot>,
    pub capture_ms: f64,
}

/// Lightweight metadata summary returned by `GET /api/snapshot`.
#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotSummary {
    pub name: String,
    pub flip: u64,
    pub cursor: u64,
    pub declared_ms: u64,
    pub digest: String,
    pub bytes_len: usize,
    pub capture_ms: f64,
}

impl SnapshotSlot {
    #[must_use]
    pub fn summary(&self) -> SnapshotSummary {
        SnapshotSummary {
            name: self.name.clone(),
            flip: self.flip,
            cursor: self.cursor,
            declared_ms: self.declared_ms,
            digest: self.digest.clone(),
            bytes_len: self.bytes_len,
            capture_ms: self.capture_ms,
        }
    }
}

/// Live machine-boundary snapshot or restore operation executed on `&mut Machine`.
#[derive(Clone, Debug)]
pub enum SnapshotOp {
    Save {
        name: Option<String>,
        path: Option<PathBuf>,
    },
    RestoreSlot {
        name: String,
    },
    RestorePath {
        path: PathBuf,
    },
    SaveBookmark {
        name: String,
    },
    RewindBookmark {
        name: String,
    },
}

/// Reply returned by [`execute_snapshot_op`].
#[derive(Clone, Debug)]
pub enum SnapshotReply {
    Saved {
        name: Option<String>,
        path: Option<String>,
        flip: u64,
        cursor: u64,
        declared_ms: u64,
        digest: String,
        bytes_len: usize,
        capture_ms: f64,
    },
    Restored {
        name: Option<String>,
        path: Option<String>,
        flip: u64,
        cursor: u64,
        declared_ms: u64,
        digest: String,
        expected_digest: Option<String>,
        digest_match: bool,
        restore_ms: f64,
        mode: &'static str,
    },
    BookmarkSaved {
        name: String,
        data: BookmarkData,
        bytes_len: usize,
        capture_ms: f64,
    },
    Error {
        status: u16,
        message: String,
    },
}

/// Execute one [`SnapshotOp`] on `&mut Machine` at a flip boundary.
pub(crate) fn execute_snapshot_op(
    bus: &ControlBus,
    machine: &mut Machine,
    host: &mut dyn Host,
    op: SnapshotOp,
) -> SnapshotReply {
    match op {
        SnapshotOp::Save { name, path } => {
            let slot_name = match (name, &path) {
                (Some(n), _) => {
                    let trimmed = n.trim().to_owned();
                    if trimmed.is_empty() {
                        return SnapshotReply::Error {
                            status: 400,
                            message: "snapshot `name` must not be empty".to_owned(),
                        };
                    }
                    Some(trimmed)
                }
                (None, None) => Some("default".to_owned()),
                (None, Some(_)) => None,
            };

            let t0 = Instant::now();
            let bytes = match machine.snapshot_v2() {
                Ok(b) => b,
                Err(e) => {
                    return SnapshotReply::Error {
                        status: 500,
                        message: format!("snapshot failed: {e}"),
                    };
                }
            };
            let capture_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let bytes_len = bytes.len();
            let flip = machine.flips();
            let cursor = machine.clock.cursor();
            let declared_ms = machine.clock.millis();
            let digest = machine.state_digest();

            let path_str = if let Some(ref p) = path {
                if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return SnapshotReply::Error {
                            status: 500,
                            message: format!(
                                "failed to create directory {}: {e}",
                                parent.display()
                            ),
                        };
                    }
                }
                if let Err(e) = std::fs::write(p, &bytes) {
                    return SnapshotReply::Error {
                        status: 500,
                        message: format!("failed to write snapshot {}: {e}", p.display()),
                    };
                }
                Some(p.display().to_string())
            } else {
                None
            };

            let frame = bus.latest_frame();
            if let Some(ref key) = slot_name {
                let slot = SnapshotSlot {
                    name: key.clone(),
                    flip,
                    cursor,
                    declared_ms,
                    digest: digest.clone(),
                    bytes_len,
                    bytes: Arc::new(bytes),
                    frame,
                    capture_ms,
                };
                bus.store_snapshot(
                    key.clone(),
                    slot,
                    format!("snapshot `{key}` @ flip {flip} ({capture_ms:.1} ms)"),
                );
            } else if let Some(ref p) = path_str {
                bus.push_feed(
                    "control",
                    format!("snapshot `{p}` @ flip {flip} ({capture_ms:.1} ms)"),
                );
            }

            SnapshotReply::Saved {
                name: slot_name,
                path: path_str,
                flip,
                cursor,
                declared_ms,
                digest,
                bytes_len,
                capture_ms,
            }
        }
        SnapshotOp::RestoreSlot { name } => {
            let Some(slot) = bus.get_snapshot(&name) else {
                return SnapshotReply::Error {
                    status: 404,
                    message: format!("unknown snapshot `{name}`"),
                };
            };
            run_live_restore(
                bus,
                machine,
                host,
                Some(name),
                None,
                &slot.bytes,
                Some(slot.digest),
                slot.frame,
            )
        }
        SnapshotOp::RestorePath { path } => {
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    return SnapshotReply::Error {
                        status: 400,
                        message: format!("failed to read snapshot {}: {e}", path.display()),
                    };
                }
            };
            run_live_restore(
                bus,
                machine,
                host,
                None,
                Some(path.display().to_string()),
                &bytes,
                None,
                None,
            )
        }
        SnapshotOp::SaveBookmark { name } => {
            let trimmed = name.trim().to_owned();
            if trimmed.is_empty() {
                return SnapshotReply::Error {
                    status: 400,
                    message: "bookmark `name` must not be empty".to_owned(),
                };
            }
            let data = match execute_inspect(machine, InspectQuery::BookmarkCapture) {
                InspectReply::Bookmark(bm) => bm,
                InspectReply::Error { status, message } => {
                    return SnapshotReply::Error { status, message };
                }
                _ => {
                    return SnapshotReply::Error {
                        status: 500,
                        message: "unexpected bookmark capture reply".to_owned(),
                    };
                }
            };
            let t0 = Instant::now();
            let snap_bytes = match machine.snapshot_v2() {
                Ok(b) => b,
                Err(e) => {
                    return SnapshotReply::Error {
                        status: 500,
                        message: format!("bookmark snapshot failed: {e}"),
                    };
                }
            };
            let capture_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let bytes_len = snap_bytes.len();
            let frame = bus.latest_frame();
            bus.store_bookmark(
                trimmed.clone(),
                data.clone(),
                Arc::new(snap_bytes),
                frame,
                format!(
                    "bookmark `{trimmed}` @ flip {} ({capture_ms:.1} ms)",
                    data.flip
                ),
            );
            SnapshotReply::BookmarkSaved {
                name: trimmed,
                data,
                bytes_len,
                capture_ms,
            }
        }
        SnapshotOp::RewindBookmark { name } => {
            let Some((bm, snap)) = bus.get_bookmark_snapshot(&name) else {
                return SnapshotReply::Error {
                    status: 404,
                    message: format!("unknown bookmark `{name}`"),
                };
            };
            let Some(snap) = snap else {
                return SnapshotReply::Error {
                    status: 404,
                    message: format!("bookmark `{name}` has no LLSNAP v2 snapshot"),
                };
            };
            run_live_restore(
                bus,
                machine,
                host,
                Some(name),
                None,
                &snap.bytes,
                Some(bm.digest),
                snap.frame,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_live_restore(
    bus: &ControlBus,
    machine: &mut Machine,
    host: &mut dyn Host,
    name: Option<String>,
    path: Option<String>,
    bytes: &[u8],
    expected_digest: Option<String>,
    restored_frame: Option<FrameSnapshot>,
) -> SnapshotReply {
    let t0 = Instant::now();
    if let Err(e) = machine.restore_v2_live(bytes) {
        return SnapshotReply::Error {
            status: 422,
            message: format!("restore failed: {e}"),
        };
    }
    let restore_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let flip = machine.flips();
    let cursor = machine.clock.cursor();
    let declared_ms = machine.clock.millis();
    let digest = machine.state_digest();
    let digest_match = expected_digest.as_deref().is_none_or(|exp| exp == digest);

    sync_after_live_restore(bus, machine, host, restored_frame);

    let label = name
        .as_deref()
        .or(path.as_deref())
        .unwrap_or("snapshot")
        .to_owned();
    bus.push_feed(
        "control",
        format!("restore `{label}` -> flip {flip} ({restore_ms:.1} ms)"),
    );

    SnapshotReply::Restored {
        name,
        path,
        flip,
        cursor,
        declared_ms,
        digest,
        expected_digest,
        digest_match,
        restore_ms,
        mode: "llsnap_v2",
    }
}

/// Keep the host window and [`ControlBus`] synchronized with the freshly
/// restored machine state.
fn sync_after_live_restore(
    bus: &ControlBus,
    machine: &Machine,
    host: &mut dyn Host,
    mut restored_frame: Option<FrameSnapshot>,
) {
    machine.sync_host_after_restore(host);

    let pointer_shown = machine.state.pointer_shown();
    let cursor_blank = machine.state.cursor.is_some_and(|c| {
        crate::win32::user32::shape(&machine.state, c).is_some_and(|s| s.is_blank())
    });
    let pointer_pos = machine.state.pointer;
    let hw_pointer = machine.state.last_move;
    if let Some(ref f) = restored_frame {
        let frame = Frame {
            width: f.width,
            height: f.height,
            pitch: f.width * 4,
            pixels: &f.bgra,
        };
        host.present(&frame);
    } else if let Some((w, h, bgra)) = machine.last_presented_bgra() {
        let frame = Frame {
            width: w,
            height: h,
            pitch: w * 4,
            pixels: bgra,
        };
        host.present(&frame);
        let (diff, frame_hash, uniform) = analyze_frame(None, bgra, w, h, 0);
        restored_frame = Some(FrameSnapshot {
            flip: machine.flips(),
            width: w,
            height: h,
            bgra: Arc::new(bgra.to_vec()),
            frame_hash,
            diff,
            uniform,
            pointer_pos,
            pointer_shown,
            cursor_blank,
            cursor_visible: CursorModel::overlay_visible(pointer_shown, cursor_blank),
        });
    }

    let route_remaining = machine.undelivered();
    let meta = FlipMeta {
        flip: machine.flips(),
        clock_nanos: machine.clock.nanos(),
        cursor: machine.clock.cursor(),
        instructions_passed: machine.instructions_passed(),
        pointer_shown,
        pointer_pos,
        route_active: route_remaining > 0,
        route_remaining,
    };

    let mut held_buttons = Vec::new();
    if machine.state.mouse_down {
        held_buttons.push(crate::host::Button::Left);
    }
    if machine.state.right_down {
        held_buttons.push(crate::host::Button::Right);
    }
    let recording_len = machine
        .recording
        .as_ref()
        .map_or(0, crate::route::Route::len);
    bus.rebaseline_after_restore(RestoreSync {
        meta,
        restored_frame,
        held_keys: machine.state.keys_down.clone(),
        held_buttons,
        recording_len,
        cursor_blank,
        hw_pointer,
        pace_sleep_nanos: host.pace_sleep_nanos(),
    });
}

fn parse_body(req: &HttpRequest) -> Result<BTreeMap<String, JsonValue>, HttpResponse> {
    let text = std::str::from_utf8(&req.body)
        .map_err(|_| HttpResponse::error_json(400, "body must be valid UTF-8"))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(BTreeMap::new());
    }
    let val = parse_json(trimmed).map_err(|e| HttpResponse::error_json(400, &e))?;
    match val {
        JsonValue::Object(map) => Ok(map),
        _ => Err(HttpResponse::error_json(
            400,
            "request body must be a JSON object",
        )),
    }
}

/// `GET /api/snapshot` — list all in-memory snapshot slots.
#[must_use]
pub fn handle_list_snapshots(ctx: &HttpContext) -> HttpResponse {
    let list = ctx.bus.list_snapshots();
    let mut out = String::with_capacity(128 + list.len() * 160);
    out.push_str("{\"ok\":true,\"snapshots\":[");
    for (i, s) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_json_str(&mut out, &s.name);
        let _ = write!(
            out,
            ",\"flip\":{},\"cursor\":{},\"declared_ms\":{},\"bytes_len\":{},\"capture_ms\":",
            s.flip, s.cursor, s.declared_ms, s.bytes_len
        );
        write_json_f64(&mut out, s.capture_ms);
        out.push_str(",\"digest\":");
        write_json_str(&mut out, &s.digest);
        out.push('}');
    }
    out.push_str("]}");
    HttpResponse::json(200, out)
}

/// `POST /api/snapshot` — capture an `LLSNAP v2` snapshot to an in-memory slot
/// (`name`) and/or file (`path`).
#[must_use]
pub fn handle_save_snapshot(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let name = obj
        .get("name")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let path = obj
        .get("path")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    reply_to_http(
        ctx.bus
            .request_snapshot_op(SnapshotOp::Save { name, path }, Some(SNAPSHOT_TIMEOUT)),
    )
}

/// `POST /api/restore` — restore an `LLSNAP v2` snapshot from an in-memory slot
/// (`name`) or disk file (`path`).
#[must_use]
pub fn handle_restore_snapshot(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let op = if let Some(path) = obj
        .get("path")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        SnapshotOp::RestorePath {
            path: PathBuf::from(path),
        }
    } else {
        let name = obj
            .get("name")
            .and_then(JsonValue::as_str)
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|| "default".to_owned());
        if name.is_empty() {
            return HttpResponse::error_json(400, "snapshot `name` must not be empty");
        }
        SnapshotOp::RestoreSlot { name }
    };
    reply_to_http(ctx.bus.request_snapshot_op(op, Some(SNAPSHOT_TIMEOUT)))
}

fn reply_to_http(reply: SnapshotReply) -> HttpResponse {
    match reply {
        SnapshotReply::Saved {
            name,
            path,
            flip,
            cursor,
            declared_ms,
            digest,
            bytes_len,
            capture_ms,
        } => {
            let mut out = String::with_capacity(256);
            out.push_str("{\"ok\":true");
            if let Some(n) = name {
                out.push_str(",\"name\":");
                write_json_str(&mut out, &n);
            }
            if let Some(p) = path {
                out.push_str(",\"path\":");
                write_json_str(&mut out, &p);
            }
            let _ = write!(
                out,
                ",\"flip\":{flip},\"cursor\":{cursor},\"declared_ms\":{declared_ms},\"bytes_len\":{bytes_len},\"capture_ms\":"
            );
            write_json_f64(&mut out, capture_ms);
            out.push_str(",\"digest\":");
            write_json_str(&mut out, &digest);
            out.push('}');
            HttpResponse::json(200, out)
        }
        SnapshotReply::Restored {
            name,
            path,
            flip,
            cursor,
            declared_ms,
            digest,
            expected_digest,
            digest_match,
            restore_ms,
            mode,
        } => {
            let mut out = String::with_capacity(256);
            out.push_str("{\"ok\":true");
            if let Some(n) = name {
                out.push_str(",\"name\":");
                write_json_str(&mut out, &n);
            }
            if let Some(p) = path {
                out.push_str(",\"path\":");
                write_json_str(&mut out, &p);
            }
            let _ = write!(
                out,
                ",\"flip\":{flip},\"cursor\":{cursor},\"declared_ms\":{declared_ms},\"digest\":"
            );
            write_json_str(&mut out, &digest);
            if let Some(exp) = expected_digest {
                out.push_str(",\"expected_digest\":");
                write_json_str(&mut out, &exp);
            }
            let _ = write!(out, ",\"digest_match\":{digest_match},\"restore_ms\":");
            write_json_f64(&mut out, restore_ms);
            out.push_str(",\"mode\":");
            write_json_str(&mut out, mode);
            out.push('}');
            HttpResponse::json(200, out)
        }
        SnapshotReply::BookmarkSaved {
            name,
            data,
            bytes_len,
            capture_ms,
        } => {
            let mut out = String::with_capacity(256);
            out.push_str("{\"ok\":true,\"name\":");
            write_json_str(&mut out, &name);
            let _ = write!(
                out,
                ",\"flip\":{},\"cursor\":{},\"declared_ms\":{},\"bytes_len\":{},\"capture_ms\":",
                data.flip, data.cursor, data.declared_ms, bytes_len
            );
            write_json_f64(&mut out, capture_ms);
            out.push_str(",\"digest\":");
            write_json_str(&mut out, &data.digest);
            out.push_str(",\"route_text\":");
            write_json_str(&mut out, &data.route_text);
            out.push('}');
            HttpResponse::json(200, out)
        }
        SnapshotReply::Error { status, message } => HttpResponse::error_json(status, &message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Trace;
    use crate::clock::FlipCost;
    use crate::control::bus::{ControlConfig, ExecMode};
    use crate::control::host::ControlHost;
    use crate::guest::GuestMem;
    use crate::host::headless::Headless;
    use crate::runlog::RunLog;
    use crate::stub;
    use crate::Title;

    const TIMED_STUB: Title = Title {
        flip_cost: FlipCost::Refresh(60),
        ..Title::STUB
    };

    /// Minimal stub program that presents `count` flips and exits with `eax = count`.
    fn flip_loop_stub(count: u32) -> Vec<u8> {
        let mut code = Vec::new();
        // mov ecx, count
        code.push(0xb9);
        code.extend_from_slice(&count.to_le_bytes());
        // loop: int 0x20 ; dec ecx ; jnz loop ; int 0x21
        code.extend_from_slice(&[0xcd, stub::INT_PRESENT]);
        code.push(0x49);
        code.extend_from_slice(&[0x75, (-5i8) as u8]);
        code.extend_from_slice(&[0xcd, stub::INT_EXIT]);
        code
    }

    fn tick_flips(machine: &mut Machine, host: &mut dyn Host, log: &mut RunLog, target: u64) {
        while machine.flips() < target {
            if machine.tick(host, log).is_some() {
                break;
            }
        }
    }

    fn produce_cold_60_flip_snapshot(log_name: &str, file_name: &str) -> (u64, PathBuf) {
        let mut prod_host = Headless::new();
        let mut prod = Machine::new(&TIMED_STUB, &mut prod_host).expect("producer machine");
        prod.load_stub().expect("stub");
        prod.arena
            .write(stub::ENTRY, &flip_loop_stub(120))
            .expect("write stub");
        let mut prod_log = RunLog::create(log_name, Trace::Off).expect("prod log");
        tick_flips(&mut prod, &mut prod_host, &mut prod_log, 60);
        let snap_clock = prod.clock.nanos();
        let snap_bytes = prod.snapshot_v2().expect("snapshot_v2");
        let snap_path = prod_log.dir().join(file_name);
        std::fs::write(&snap_path, &snap_bytes).expect("write cold snapshot");
        (snap_clock, snap_path)
    }

    #[test]
    fn cold_snapshot_restore_and_rewind_do_not_inject_phantom_declared_nanos() {
        const MS: u64 = 1_000_000;

        let (snap_clock, snap_path) =
            produce_cold_60_flip_snapshot("snap-tel-prod", "cold-60.llsnap");
        assert!(
            snap_clock >= 900 * MS,
            "snapshot clock should be ~1s, got {snap_clock}"
        );

        // 2. Start a fresh machine under ControlHost (Live 1.0x mode), run
        //    2 flips (~33 ms declared), then restore the cold 60-flip (~1.0 s)
        //    snapshot from disk.
        let mut control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                title: &TIMED_STUB,
                mode: ExecMode::Live,
                pace: 1.0,
                ..ControlConfig::default()
            },
        )
        .with_deferred_pause(true);
        let bus = control.bus();

        let mut machine = Machine::new(&TIMED_STUB, &mut control).expect("machine");
        machine.load_stub().expect("stub");
        machine
            .arena
            .write(stub::ENTRY, &flip_loop_stub(120))
            .expect("write stub");
        let mut log = RunLog::create("snap-tel-live", Trace::Off).expect("live log");

        tick_flips(&mut machine, &mut control, &mut log, 2);
        let before_restore = bus.telemetry_report(None).full;
        assert_eq!(before_restore.flips, 2);

        // Save an early in-memory slot at flip 2 for a backward rewind check too.
        let saved = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::Save {
                name: Some("early".into()),
                path: None,
            },
        );
        assert!(matches!(saved, SnapshotReply::Saved { .. }));

        // Cold restore forward from ~33 ms -> ~1000 ms.
        let restored = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::RestorePath {
                path: snap_path.clone(),
            },
        );
        assert!(matches!(restored, SnapshotReply::Restored { .. }));
        assert_eq!(machine.flips(), 60);

        // Run 3 flips post-restore (flips 60, 61, 62 -> ~50 ms declared).
        tick_flips(&mut machine, &mut control, &mut log, 63);
        let after_cold = bus.telemetry_report(None).full;
        assert_eq!(after_cold.flips, 5);
        // 5 flips @ 60 Hz = ~83.3 ms declared (NOT ~1050 ms with a +967 ms phantom jump!).
        assert!(
            after_cold.declared_nanos < 120 * MS,
            "cold restore injected phantom declared_nanos: {} ms (expected ~83 ms)",
            after_cold.declared_nanos / MS
        );
        assert!(
            (after_cold.guest_fps - 60.0).abs() < 5.0,
            "cold restore skewed guest_fps: {:.2} (expected ~60.0)",
            after_cold.guest_fps
        );

        // 3. Rewind backward to the early slot (flip 2, ~33 ms) and run 1 flip:
        //    the first post-rewind flip must still record ~16.67 ms declared (not 0).
        let rewound = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::RestoreSlot {
                name: "early".into(),
            },
        );
        assert!(matches!(rewound, SnapshotReply::Restored { .. }));
        assert_eq!(machine.flips(), 2);

        let before_rewind_flip = bus.telemetry_report(None).full.declared_nanos;
        tick_flips(&mut machine, &mut control, &mut log, 3);
        let after_rewind_flip = bus.telemetry_report(None).full.declared_nanos;
        let delta = after_rewind_flip.saturating_sub(before_rewind_flip);
        assert!(
            (15 * MS..=20 * MS).contains(&delta),
            "backward rewind flip declared delta was {} ms (expected ~16.7 ms)",
            delta / MS
        );
    }

    #[test]
    fn live_restore_clears_control_host_pacing_clock_and_staging_anchors() {
        use crate::control::bus::Command;
        use crate::control::timeline::Action;
        use crate::host::{Button, HostError, Input, Mapping};

        const MS: u64 = 1_000_000;

        struct PaceProbe {
            inner: Headless,
            seen_nanos: Vec<u64>,
        }
        impl Host for PaceProbe {
            fn present(&mut self, frame: &Frame<'_>) {
                self.inner.present(frame);
            }
            fn poll_input(&mut self, flip: u64) -> Vec<Input> {
                self.inner.poll_input(flip)
            }
            fn audio(&mut self, samples: &[i16]) {
                self.inner.audio(samples);
            }
            fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
                self.inner.map(len)
            }
            fn pace(&mut self, declared_nanos: u64) -> f64 {
                self.seen_nanos.push(declared_nanos);
                1.0
            }
            fn is_paced(&self) -> bool {
                true
            }
        }

        // 1. Produce a cold 60-flip (~1.0 s declared) LLSNAP v2 snapshot on disk.
        let (_, snap_path) =
            produce_cold_60_flip_snapshot("snap-anchor-prod", "cold-60-anchor.llsnap");

        // 2. Run a live ControlHost<PaceProbe> for 2 flips (~33 ms) and save an early slot.
        let mut control = ControlHost::new(
            PaceProbe {
                inner: Headless::new(),
                seen_nanos: Vec::new(),
            },
            ControlConfig {
                title: &TIMED_STUB,
                mode: ExecMode::Live,
                pace: 1.0,
                ..ControlConfig::default()
            },
        )
        .with_deferred_pause(true);
        let bus = control.bus();

        let mut machine = Machine::new(&TIMED_STUB, &mut control).expect("machine");
        machine.load_stub().expect("stub");
        machine
            .arena
            .write(stub::ENTRY, &flip_loop_stub(120))
            .expect("write stub");
        let mut log = RunLog::create("snap-anchor-live", Trace::Off).expect("live log");
        tick_flips(&mut machine, &mut control, &mut log, 2);

        let saved = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::Save {
                name: Some("early".into()),
                path: None,
            },
        );
        assert!(matches!(saved, SnapshotReply::Saved { .. }));

        // Stage a distinctive stale pre-restore frame (0xAA fill) without observe_flip.
        let stale_pixels = vec![0xAAu8; 4 * 4 * 4];
        control.present(&Frame {
            width: 4,
            height: 4,
            pitch: 16,
            pixels: &stale_pixels,
        });

        // 3. Forward cold live restore (~33 ms -> ~1000 ms) through `&mut dyn Host`.
        let epoch0 = bus.runtime_reset_epoch();
        let last_pre_restore_pace = *control
            .inner()
            .seen_nanos
            .last()
            .expect("pre-restore pace sample");
        let paces_before = control.inner().seen_nanos.len();
        let restored = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::RestorePath { path: snap_path },
        );
        assert!(matches!(restored, SnapshotReply::Restored { .. }));
        let epoch1 = bus.runtime_reset_epoch();
        assert!(
            epoch1 > epoch0,
            "forward live restore must advance runtime_reset_epoch ({epoch0} -> {epoch1})"
        );

        // Scheduling a hold_ms Click right after forward restore (poll_input before pace)
        // must arm against the restored ~1000 ms clock, NOT 0 ms!
        let restored_clock = machine.clock.nanos();
        assert!(
            bus.dispatch(Command::Act {
                action: Action::Click {
                    x: Some(100),
                    y: Some(100),
                    button: Button::Left,
                    hold_flips: 5,
                    hold_ms: Some(150),
                    settle_flips: 0,
                },
                auto_step: false,
            })
            .ok
        );
        let polled_fwd = control.poll_input(60);
        assert!(polled_fwd.contains(&Input::Down(Button::Left)));
        assert!(!polled_fwd.contains(&Input::Up(Button::Left)));

        // Next pace() after forward restore must keep scaled time monotonic (delta == 0,
        // no +967 ms jump forward AND no backward drop to 0) and advance smoothly after.
        control.pace(restored_clock);
        let first_post_restore_pace = control.inner().seen_nanos[paces_before];
        assert_eq!(
            first_post_restore_pace, last_pre_restore_pace,
            "forward live restore must re-anchor pacing with zero jump"
        );
        control.pace(restored_clock + 16 * MS);
        assert_eq!(
            control.inner().seen_nanos[paces_before + 1],
            last_pre_restore_pace + 16 * MS
        );
        // At restored_clock + 16 ms (< restored_clock + 150 ms), Click hold_ms must still be held.
        assert!(
            !control.poll_input(60).contains(&Input::Up(Button::Left)),
            "forward-restored Click hold_ms must arm at restored clock, not 0 ms"
        );
        assert!(bus.dispatch(Command::ClearQueue).ok);

        // Re-stage stale pixels, restore again, and confirm un-presented observe_flip
        // does NOT publish the stale pre-restore staged buffer.
        control.present(&Frame {
            width: 4,
            height: 4,
            pitch: 16,
            pixels: &stale_pixels,
        });
        let rewound_check = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::RestoreSlot {
                name: "early".into(),
            },
        );
        assert!(matches!(rewound_check, SnapshotReply::Restored { .. }));
        let frame_after_restore = bus.latest_frame().expect("restored frame");
        control.observe_flip(&FlipMeta {
            flip: 2,
            clock_nanos: machine.clock.nanos(),
            cursor: machine.clock.cursor(),
            instructions_passed: machine.instructions_passed(),
            pointer_shown: true,
            pointer_pos: (0, 0),
            route_active: false,
            route_remaining: 0,
        });
        let frame_after_obs = bus.latest_frame().expect("frame after obs");
        assert_eq!(
            frame_after_obs.frame_hash, frame_after_restore.frame_hash,
            "stale pre-restore staged frame must not overwrite restored frame"
        );

        // 4. Backward live restore (~1000 ms -> ~33 ms): stale last_clock_nanos must NOT
        //    prematurely release a ClickPoll with hold_ms=150 on poll_input before pace().
        control.pace(restored_clock);
        let epoch2_before = bus.runtime_reset_epoch();
        let rewound = execute_snapshot_op(
            &bus,
            &mut machine,
            &mut control,
            SnapshotOp::RestoreSlot {
                name: "early".into(),
            },
        );
        assert!(matches!(rewound, SnapshotReply::Restored { .. }));
        let epoch2 = bus.runtime_reset_epoch();
        assert!(
            epoch2 > epoch2_before,
            "backward live restore must advance runtime_reset_epoch ({epoch2_before} -> {epoch2})"
        );
        assert!(
            bus.dispatch(Command::Act {
                action: Action::ClickPoll {
                    x: Some(100),
                    y: Some(100),
                    button: Button::Left,
                    hold_flips: 5,
                    hold_ms: 150,
                },
                auto_step: false,
            })
            .ok
        );
        let polled = control.poll_input(2);
        assert!(
            !polled.contains(&Input::Up(Button::Left)),
            "stale pre-restore last_clock_nanos prematurely released ClickPoll: {polled:?}"
        );

        // 5. Outer-loop reset_for_rewind also advances runtime_reset_epoch monotonically.
        control.reset_for_rewind();
        assert!(
            bus.runtime_reset_epoch() > epoch2,
            "reset_for_rewind must advance runtime_reset_epoch"
        );
    }
}
