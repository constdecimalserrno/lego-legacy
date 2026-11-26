//! Zero-crate newline-delimited JSON (JSONL) stdio transport (`--control-stdio`) (#308).
//!
//! Reads one JSON command per line from `stdin` on a worker thread, dispatches
//! through [`ControlBus::dispatch`] / [`ControlBus::dispatch_sync`], and writes
//! single-line JSON responses to `stdout`. All diagnostics go to `stderr` or
//! `RunLog`, keeping `stdout` strictly machine-parseable JSONL.

use super::bus::{Command, ControlBus, ExecMode, InputOwner, Response, ResponseData};
use super::json::{
    parse_action_obj, parse_agent_state_patch, write_agent_state, write_fps_report,
    write_frame_info, write_status, write_wait_result,
};
pub use super::json::{parse_json, write_json_str, JsonValue};
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::thread;

/// Parsed JSONL command line ready to dispatch onto [`ControlBus`].
#[derive(Clone, Debug)]
pub struct ParsedLine {
    pub id: Option<JsonValue>,
    pub commands: Vec<Command>,
    pub sync: bool,
}

/// Parse one JSONL command line into [`ParsedLine`].
///
/// # Errors
/// Returns a descriptive error on invalid JSON, missing fields, or unknown commands.
pub fn parse_command_line(line: &str) -> Result<ParsedLine, String> {
    let val = parse_json(line)?;
    let obj = val
        .as_object()
        .ok_or_else(|| "command must be a JSON object".to_owned())?;
    let id = obj.get("id").cloned();
    let sync = obj.get("wait").and_then(JsonValue::as_bool).unwrap_or(true);
    let cmd_name = obj
        .get("cmd")
        .or_else(|| obj.get("action"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing string `cmd`".to_owned())?;

    let auto_step = obj
        .get("auto_step")
        .and_then(JsonValue::as_bool)
        .unwrap_or_else(|| {
            !obj.get("queue")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        });

    let commands = match cmd_name {
        "status" => vec![Command::Status],
        "frame" | "frame_info" => vec![Command::FrameInfo],
        "telemetry" | "fps" => {
            let window = match obj.get("window") {
                None | Some(JsonValue::Null) => None,
                Some(JsonValue::Array(arr)) if arr.len() == 2 => {
                    let a = arr[0]
                        .as_u64()
                        .ok_or_else(|| "invalid window start".to_owned())?;
                    let b = arr[1]
                        .as_u64()
                        .ok_or_else(|| "invalid window end".to_owned())?;
                    if a >= b {
                        return Err("telemetry window requires start < end".to_owned());
                    }
                    Some((a, b))
                }
                _ => return Err("`window` must be [start, end]".to_owned()),
            };
            vec![Command::Telemetry { window }]
        }
        "get_agent_state" => vec![Command::GetAgentState],
        "set_agent_state" => vec![Command::SetAgentState(parse_agent_state_patch(obj)?)],
        "step" => {
            let flips = obj.get("flips").and_then(JsonValue::as_u64).unwrap_or(1);
            vec![Command::Step { flips }]
        }
        "act" => {
            let action_name = obj
                .get("action")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "missing string `action`".to_owned())?;
            let action = parse_action_obj(action_name, obj)?;
            vec![Command::Act { action, auto_step }]
        }
        "move" | "mouse_delta" | "click" | "click_poll" | "drag" | "key_hold" | "chord"
        | "type_text" => {
            let action = parse_action_obj(cmd_name, obj)?;
            vec![Command::Act { action, auto_step }]
        }
        "batch" => {
            let arr = obj
                .get("actions")
                .and_then(JsonValue::as_array)
                .ok_or_else(|| "missing array `actions`".to_owned())?;
            let mut actions = Vec::with_capacity(arr.len());
            for item in arr {
                let item_obj = item
                    .as_object()
                    .ok_or_else(|| "`actions` items must be objects".to_owned())?;
                let kind = item_obj
                    .get("action")
                    .or_else(|| item_obj.get("cmd"))
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| "each batch action needs `action`".to_owned())?;
                actions.push(parse_action_obj(kind, item_obj)?);
            }
            vec![Command::Batch { actions, auto_step }]
        }
        "pipe_route" | "pipe" => {
            let route_text = obj
                .get("route")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "missing string `route`".to_owned())?
                .to_owned();
            let relative = obj
                .get("relative")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            vec![Command::PipeRoute {
                route_text,
                relative,
                auto_step,
            }]
        }
        "wait" => vec![Command::Wait {
            flips: obj.get("flips").and_then(JsonValue::as_u64),
            ms: obj.get("ms").and_then(JsonValue::as_u64),
        }],
        "wait_until_change" => vec![Command::WaitUntilChange {
            max_flips: obj
                .get("max_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(60),
            min_diff_pct: obj
                .get("min_diff_pct")
                .and_then(JsonValue::as_f64)
                .unwrap_or(0.5),
        }],
        "wait_until_settled" => vec![Command::WaitUntilSettled {
            max_flips: obj
                .get("max_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(120),
            stable_flips: obj
                .get("stable_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(5),
            max_diff_pct: obj
                .get("max_diff_pct")
                .and_then(JsonValue::as_f64)
                .unwrap_or(0.1),
        }],
        "pause" => vec![Command::Pause],
        "resume" => vec![Command::Resume],
        "mode" | "set_mode" => {
            let mut cmds = Vec::new();
            if let Some(m) = obj.get("mode").and_then(JsonValue::as_str) {
                let mode = ExecMode::parse(m)
                    .ok_or_else(|| format!("unknown mode `{m}` (expected step|live)"))?;
                let step_uncapped = obj
                    .get("fast")
                    .or_else(|| obj.get("step_uncapped"))
                    .and_then(JsonValue::as_bool);
                cmds.push(Command::SetMode {
                    mode,
                    step_uncapped,
                });
            }
            if let Some(o) = obj.get("owner").and_then(JsonValue::as_str) {
                let owner = InputOwner::parse(o)
                    .ok_or_else(|| format!("unknown owner `{o}` (expected agent|human|shared)"))?;
                cmds.push(Command::SetOwner(owner));
            }
            if let Some(p) = obj.get("pace").and_then(JsonValue::as_f64) {
                cmds.push(Command::SetPace(p));
            }
            if cmds.is_empty() {
                return Err("`mode` requires at least one of `mode`, `owner`, or `pace`".to_owned());
            }
            cmds
        }
        "set_owner" | "owner" => {
            let o = obj
                .get("owner")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "missing string `owner`".to_owned())?;
            let owner = InputOwner::parse(o)
                .ok_or_else(|| format!("unknown owner `{o}` (expected agent|human|shared)"))?;
            vec![Command::SetOwner(owner)]
        }
        "set_pace" | "pace" => {
            let p = obj
                .get("pace")
                .and_then(JsonValue::as_f64)
                .ok_or_else(|| "missing number `pace`".to_owned())?;
            vec![Command::SetPace(p)]
        }
        "clear_queue" => vec![Command::ClearQueue],
        "close" => vec![Command::Close {
            force: obj
                .get("force")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
        }],
        other => return Err(format!("unknown cmd `{other}`")),
    };

    Ok(ParsedLine { id, commands, sync })
}

/// Format a [`Response`] (with optional request `id`) as a single-line JSON string.
#[must_use]
pub fn format_response_json(id: Option<&JsonValue>, resp: &Response) -> String {
    let mut out = String::with_capacity(256);
    out.push('{');
    if let Some(id) = id {
        out.push_str("\"id\":");
        id.write_json(&mut out);
        out.push(',');
    }
    let _ = write!(out, "\"ok\":{}", resp.ok);
    if let Some(err) = &resp.error {
        out.push_str(",\"error\":");
        write_json_str(&mut out, err);
    }
    out.push_str(",\"status\":");
    write_status(&mut out, &resp.status);
    match &resp.data {
        ResponseData::None => {}
        ResponseData::Frame(f) => {
            out.push_str(",\"frame\":");
            write_frame_info(&mut out, f);
        }
        ResponseData::Telemetry(t) => {
            out.push_str(",\"telemetry\":");
            write_fps_report(&mut out, t);
        }
        ResponseData::AgentState(a) => {
            out.push_str(",\"agent_state\":");
            write_agent_state(&mut out, a);
        }
        ResponseData::Scheduled {
            events_scheduled,
            end_flip,
            required_flips,
        } => {
            let _ = write!(
                out,
                ",\"scheduled\":{{\"events_scheduled\":{events_scheduled},\"end_flip\":{end_flip},\"required_flips\":{required_flips}}}"
            );
        }
        ResponseData::Cleared { cleared } => {
            let _ = write!(out, ",\"cleared\":{cleared}");
        }
        ResponseData::WaitResult(w) => {
            out.push_str(",\"wait\":");
            write_wait_result(&mut out, w);
        }
    }
    out.push('}');
    out
}

/// Stdio JSONL transport bridge wired to a [`ControlBus`].
pub struct StdioBridge;

impl StdioBridge {
    /// Spawn a background worker reading JSONL commands from process `stdin`
    /// and writing JSONL responses to process `stdout`.
    pub fn spawn(bus: ControlBus) {
        let _ = thread::Builder::new()
            .name("control-stdio".to_owned())
            .spawn(move || {
                let stdin = io::stdin();
                let stdout = io::stdout();
                let reader = BufReader::new(stdin.lock());
                let writer = stdout.lock();
                if let Err(err) = Self::drive(bus, reader, writer) {
                    eprintln!("lego-legacy: control-stdio error: {err}");
                }
            });
    }

    /// Drive the JSONL protocol over arbitrary `reader`/`writer` streams until
    /// EOF, returning the number of command lines processed.
    ///
    /// # Errors
    /// Returns any I/O error writing responses or reading lines.
    pub fn drive<R: BufRead, W: Write>(
        bus: ControlBus,
        mut reader: R,
        mut writer: W,
    ) -> io::Result<usize> {
        let mut line = String::new();
        let mut count = 0usize;
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            count += 1;
            let json_out = match parse_command_line(trimmed) {
                Ok(parsed) => {
                    let mut last_resp = None;
                    for cmd in parsed.commands {
                        let resp = if parsed.sync {
                            bus.dispatch_sync(cmd, None)
                        } else {
                            bus.dispatch(cmd)
                        };
                        let ok = resp.ok;
                        last_resp = Some(resp);
                        if !ok {
                            break;
                        }
                    }
                    let resp = last_resp.unwrap_or_else(|| bus.dispatch(Command::Status));
                    format_response_json(parsed.id.as_ref(), &resp)
                }
                Err(err) => {
                    let resp = Response {
                        ok: false,
                        error: Some(err),
                        status: bus.status(),
                        data: ResponseData::None,
                    };
                    format_response_json(None, &resp)
                }
            };
            writeln!(writer, "{json_out}")?;
            writer.flush()?;
        }
        bus.mark_transport_eof();
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::bus::ControlConfig;
    use crate::control::host::ControlHost;
    use crate::control::timeline::Action;
    use crate::host::headless::Headless;
    use crate::host::Button;
    use crate::machine::Machine;
    use crate::route::Route;
    use crate::runlog::RunLog;
    use crate::Title;
    use std::io::Cursor;

    #[test]
    fn json_parser_and_command_parser_handle_all_primitives() {
        let v = parse_json(r#"{"a":1,"b":-2.5,"c":"hi\n\"world\"\u0041","d":[true,false,null]}"#)
            .unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj["a"].as_i64(), Some(1));
        assert_eq!(obj["b"].as_f64(), Some(-2.5));
        assert_eq!(obj["c"].as_str(), Some("hi\n\"world\"A"));
        assert_eq!(obj["d"].as_array().unwrap().len(), 3);

        let p = parse_command_line(
            r#"{"id":7,"cmd":"act","action":"click_poll","x":320,"y":240,"button":"right","hold_flips":2,"hold_ms":120}"#,
        )
        .unwrap();
        assert_eq!(p.id, Some(JsonValue::Int(7)));
        assert!(matches!(
            p.commands.as_slice(),
            [Command::Act {
                action: Action::ClickPoll {
                    x: Some(320),
                    y: Some(240),
                    button: Button::Right,
                    hold_flips: 2,
                    hold_ms: 120
                },
                auto_step: true
            }]
        ));

        let p = parse_command_line(
            r#"{"cmd":"chord","keys":["LeftControl","A"],"hold_flips":3,"queue":true}"#,
        )
        .unwrap();
        assert!(matches!(
            p.commands.as_slice(),
            [Command::Act {
                action: Action::Chord { hold_flips: 3, .. },
                auto_step: false
            }]
        ));

        let p = parse_command_line(r#"{"cmd":"mode","mode":"live","owner":"shared","pace":2.0}"#)
            .unwrap();
        assert_eq!(p.commands.len(), 3);

        let p = parse_command_line(
            r##"{"cmd":"pipe_route","route":"#lego-legacy-route 1\nflip 0 key Space down\n","relative":true}"##,
        )
        .unwrap();
        assert!(matches!(
            p.commands.as_slice(),
            [Command::PipeRoute {
                relative: true,
                auto_step: true,
                ..
            }]
        ));

        // Omitting `relative` defaults to `false` (absolute bare flip numbers, #346).
        let p = parse_command_line(
            r##"{"cmd":"pipe_route","route":"#lego-legacy-route 1\nflip 100 key Space down\n"}"##,
        )
        .unwrap();
        assert!(matches!(
            p.commands.as_slice(),
            [Command::PipeRoute {
                relative: false,
                auto_step: true,
                ..
            }]
        ));

        let cleared_bus = ControlBus::new(ControlConfig::default());
        let cleared_json = format_response_json(
            Some(&JsonValue::Int(9)),
            &cleared_bus.dispatch(Command::ClearQueue),
        );
        assert!(
            cleared_json.contains("\"cleared\":0"),
            "ClearQueue JSON must include cleared count: {cleared_json}"
        );

        assert!(parse_command_line(r#"{"cmd":"bogus"}"#).is_err());
        assert!(parse_command_line("not json").is_err());
    }

    #[test]
    fn stdio_jsonl_drives_stub_and_records_bit_exact_route() {
        let mut control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                mode: ExecMode::Step,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = control.bus();

        let script = "\
{\"id\":1,\"cmd\":\"status\"}\n\
{\"id\":2,\"cmd\":\"set_agent_state\",\"goal\":\"test stub\",\"thought\":\"moving pointer\"}\n\
{\"id\":3,\"cmd\":\"move\",\"x\":111,\"y\":222,\"queue\":true}\n\
{\"id\":4,\"cmd\":\"key_hold\",\"key\":\"Space\",\"hold_flips\":1,\"queue\":true}\n\
{\"id\":5,\"cmd\":\"step\",\"flips\":1}\n";

        let driver_bus = bus.clone();
        let driver = thread::spawn(move || {
            let _ = driver_bus.wait_until_paused(Some(std::time::Duration::from_secs(5)));
            let mut out = Vec::new();
            let n = StdioBridge::drive(driver_bus, Cursor::new(script.as_bytes()), &mut out)
                .expect("stdio drive");
            assert_eq!(n, 5);
            String::from_utf8(out).expect("utf8 jsonl")
        });

        let mut machine = Machine::new(&Title::STUB, &mut control).expect("machine");
        machine.load_stub().expect("stub");
        machine.recording = Some(Route::new());
        let mut log = RunLog::create("stdio-stub-live", crate::cli::Trace::Off).expect("log");
        let live_finish = machine.run(&mut control, &mut log, 100_000);
        let live_digest = machine.state_digest();
        control.mark_finished(&live_finish, &live_digest);

        let output = driver.join().unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 5, "one JSONL line per command: {output}");
        for (i, line) in lines.iter().enumerate() {
            let parsed = parse_json(line).expect("valid JSON response");
            let obj = parsed.as_object().unwrap();
            assert_eq!(obj["ok"].as_bool(), Some(true), "line {i}: {line}");
            assert_eq!(obj["id"].as_i64(), Some((i + 1) as i64));
        }

        let recorded = machine.recording.expect("recording");
        assert!(!recorded.is_empty());
        let mut replay_host = Headless::new();
        let mut replay_machine = Machine::new(&Title::STUB, &mut replay_host).expect("machine");
        replay_machine.load_stub().expect("stub");
        replay_machine.route = Some(Route::parse(&recorded.to_text()).expect("parse route"));
        let mut replay_log =
            RunLog::create("stdio-stub-replay", crate::cli::Trace::Off).expect("log");
        let replay_finish = replay_machine.run(&mut replay_host, &mut replay_log, 100_000);
        let replay_digest = replay_machine.state_digest();

        assert_eq!(live_finish, replay_finish);
        assert_eq!(live_digest, replay_digest);
    }
}
