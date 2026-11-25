//! Zero-crate CLI parser and command dispatcher for `lego-ctrl` (#312).

use crate::http::{Client, HttpResponse};
use crate::json::{parse_json, url_encode, write_json_f64, write_json_str, JsonValue};
use crate::launch::{launch_title, LaunchOptions};
use crate::sessions::{list_live_sessions, resolve_client, sessions_to_json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: lego-ctrl [--url <URL>] [--title <NAME>] [--token <TOK>] <command> [args]

Discovery & lifecycle:
  sessions                                         list live sessions (prunes stale PIDs)
  launch <title> [--window|--headless] [--bind HOST:PORT] [--data-root DIR]
                 [--route PATH] [--mode step|live] [--token TOK] [--stub]
  watch                                            print the /watch URL

Observation:
  status | meta | controls | goals                 fetch run/title JSON endpoints
  fps [--window start..end]                        fetch frame-time/pace telemetry
  screenshot [--grid] [--cursor] [--crop x,y,w,h] [-o path.png]
  frame-json [--grid CxR]                          fetch downsampled RGB + warnings

Actions & piping:
  act <raw-json | subcommand ...>                  execute raw JSON or friendly action
  click <x> <y> [--right] [--poll] [--hold-flips N] [--hold-ms N]
  move <x> <y> [--glide-flips N]
  delta <dx> <dy> [--flips N]
  drag <x1> <y1> <x2> <y2> [--right] [--step-px N]
  key <NAME> [--hold-flips N] [--hold-ms N]
  chord <K1,K2,...> [--hold-flips N] [--hold-ms N]
  type <TEXT> [--flip-gap N]
  step [flips]
  wait <flips> [--ms N]
  wait-change [max_flips] [--min-diff PCT]
  wait-settled [max_flips] [--stable-flips N] [--max-diff PCT]
  pipe <file | -> [--absolute] [--no-wait]         pipe route or JSONL into /api/pipe

Read-only introspection:
  mem <addr> <len> [--fmt hex|raw|u8|u16|u32|i32|f32|cstr] [-o path]
  mem-map                                          guest address-space map
  mem-search <--hex|--u32|--ascii> <pattern> [--start ADDR] [--end ADDR] [--limit N]
  cpu | win32                                      CPU registers / Win32+DX state
  files [disc|profile] [path]                      list disc or profile directory
  read-file <disc|profile> <path> [-o out]         read disc or profile file bytes

Checkpoints & agent state:
  bookmark [name]                                  save named bookmark (or list if omitted)
  rewind <name>                                    rewind to named bookmark
  snapshot <path> | restore <path>                 save or restore snapshot file
  export-route [-o path]                           export recorded #lego-legacy-route 1
  goal <text> | thought <text> | hint <text>       update live /watch agent state
  mode <step|live|pause|resume|agent|human|shared|close|pace> [pace] [--force]";

/// Global target flags accepted before the subcommand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobalOpts {
    pub url: Option<String>,
    pub title: Option<String>,
    pub token: Option<String>,
}

/// Parsed `lego-ctrl` CLI invocation.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedCli {
    pub global: GlobalOpts,
    pub command: CliCommand,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CliCommand {
    Help,
    Version,
    Sessions,
    Launch(LaunchOptions),
    Status,
    Meta,
    Controls,
    Goals,
    Fps {
        window: Option<String>,
    },
    Watch,
    Screenshot {
        grid: bool,
        cursor: bool,
        crop: Option<String>,
        out: Option<PathBuf>,
    },
    FrameJson {
        grid: Option<String>,
    },
    ActRaw {
        json: String,
    },
    WaitRaw {
        json: String,
    },
    Step {
        flips: u64,
    },
    Pipe {
        source: String,
        absolute: bool,
        no_wait: bool,
    },
    Mem {
        addr: String,
        len: usize,
        fmt: String,
        out: Option<PathBuf>,
    },
    MemMap,
    MemSearch {
        json: String,
    },
    Cpu,
    Win32,
    Files {
        root: String,
        path: String,
    },
    ReadFile {
        root: String,
        path: String,
        out: Option<PathBuf>,
    },
    Bookmark {
        name: Option<String>,
    },
    Rewind {
        name: String,
    },
    Snapshot {
        path: String,
    },
    Restore {
        path: String,
    },
    ExportRoute {
        out: Option<PathBuf>,
    },
    AgentState {
        field: &'static str,
        value: String,
    },
    Mode {
        json: String,
    },
}

/// Parse command-line arguments (excluding argv[0]).
///
/// # Errors
/// Returns a descriptive error on unknown flags, missing arguments, or invalid values.
pub fn parse_args<I, S>(args: I) -> Result<ParsedCli, String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let raw: Vec<String> = args.into_iter().map(Into::into).collect();
    let mut global = GlobalOpts::default();
    let mut i = 0usize;

    while i < raw.len() {
        let a = raw[i].as_str();
        if let Some(v) = a.strip_prefix("--url=") {
            global.url = Some(v.to_owned());
            i += 1;
        } else if let Some(v) = a.strip_prefix("--title=") {
            global.title = Some(v.to_owned());
            i += 1;
        } else if let Some(v) = a.strip_prefix("--token=") {
            global.token = Some(v.to_owned());
            i += 1;
        } else {
            match a {
                "--url" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "`--url` requires a value".to_owned())?;
                    global.url = Some(v.clone());
                    i += 1;
                }
                "--title" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "`--title` requires a value".to_owned())?;
                    global.title = Some(v.clone());
                    i += 1;
                }
                "--token" => {
                    i += 1;
                    let v = raw
                        .get(i)
                        .ok_or_else(|| "`--token` requires a value".to_owned())?;
                    global.token = Some(v.clone());
                    i += 1;
                }
                "-h" | "--help" => {
                    return Ok(ParsedCli {
                        global,
                        command: CliCommand::Help,
                    });
                }
                "-V" | "--version" => {
                    return Ok(ParsedCli {
                        global,
                        command: CliCommand::Version,
                    });
                }
                _ => break,
            }
        }
    }

    let Some(cmd) = raw.get(i).map(String::as_str) else {
        return Ok(ParsedCli {
            global,
            command: CliCommand::Help,
        });
    };
    let rest = &raw[i + 1..];

    let command = match cmd {
        "help" => CliCommand::Help,
        "sessions" => {
            expect_no_extra("sessions", rest)?;
            CliCommand::Sessions
        }
        "launch" => CliCommand::Launch(parse_launch(rest)?),
        "status" => {
            expect_no_extra("status", rest)?;
            CliCommand::Status
        }
        "meta" => {
            expect_no_extra("meta", rest)?;
            CliCommand::Meta
        }
        "controls" => {
            expect_no_extra("controls", rest)?;
            CliCommand::Controls
        }
        "goals" => {
            expect_no_extra("goals", rest)?;
            CliCommand::Goals
        }
        "watch" => {
            expect_no_extra("watch", rest)?;
            CliCommand::Watch
        }
        "fps" => parse_fps(rest)?,
        "screenshot" => parse_screenshot(rest)?,
        "frame-json" => parse_frame_json(rest)?,
        "act" => parse_act(rest)?,
        "click" | "move" | "delta" | "drag" | "key" | "chord" | "type" | "step" | "wait"
        | "wait-change" | "wait-settled" | "settle" => parse_friendly_action(cmd, rest)?,
        "pipe" => parse_pipe(rest)?,
        "mem" => parse_mem(rest)?,
        "mem-map" => {
            expect_no_extra("mem-map", rest)?;
            CliCommand::MemMap
        }
        "mem-search" => parse_mem_search(rest)?,
        "cpu" => {
            expect_no_extra("cpu", rest)?;
            CliCommand::Cpu
        }
        "win32" => {
            expect_no_extra("win32", rest)?;
            CliCommand::Win32
        }
        "files" => parse_files(rest)?,
        "read-file" => parse_read_file(rest)?,
        "bookmark" => parse_bookmark(rest)?,
        "rewind" => {
            let name = req_single_arg("rewind", rest)?;
            CliCommand::Rewind { name }
        }
        "snapshot" => {
            let path = req_single_arg("snapshot", rest)?;
            CliCommand::Snapshot { path }
        }
        "restore" => {
            let path = req_single_arg("restore", rest)?;
            CliCommand::Restore { path }
        }
        "export-route" => parse_export_route(rest)?,
        "goal" => CliCommand::AgentState {
            field: "goal",
            value: join_text("goal", rest)?,
        },
        "thought" => CliCommand::AgentState {
            field: "thought",
            value: join_text("thought", rest)?,
        },
        "hint" => CliCommand::AgentState {
            field: "user_hint",
            value: join_text("hint", rest)?,
        },
        "mode" => parse_mode(rest)?,
        other => {
            return Err(format!(
                "unknown command `{other}` (run `lego-ctrl --help`)"
            ))
        }
    };

    Ok(ParsedCli { global, command })
}

fn expect_no_extra(cmd: &str, args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(format!("`{cmd}` takes no arguments, got `{}`", args[0]))
    }
}

fn req_single_arg(cmd: &str, args: &[String]) -> Result<String, String> {
    match args {
        [only] => Ok(only.clone()),
        [] => Err(format!("`{cmd}` requires an argument")),
        _ => Err(format!("`{cmd}` takes exactly one argument")),
    }
}

fn join_text(cmd: &str, args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(format!("`{cmd}` requires text"));
    }
    Ok(args.join(" "))
}

fn parse_launch(args: &[String]) -> Result<LaunchOptions, String> {
    let Some(title) = args.first() else {
        return Err("`launch` requires `<title>`".to_owned());
    };
    if title.starts_with('-') {
        return Err("`launch` requires `<title>` as its first argument".to_owned());
    }
    let mut opts = LaunchOptions::new(title);
    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--window" => opts.windowed = true,
            "--headless" => opts.windowed = false,
            "--stub" => opts.stub = true,
            "--bind" => {
                i += 1;
                opts.bind = Some(
                    args.get(i)
                        .ok_or_else(|| "`--bind` requires HOST:PORT".to_owned())?
                        .clone(),
                );
            }
            "--data-root" => {
                i += 1;
                opts.data_root = Some(
                    args.get(i)
                        .ok_or_else(|| "`--data-root` requires DIR".to_owned())?
                        .clone(),
                );
            }
            "--route" => {
                i += 1;
                opts.route = Some(
                    args.get(i)
                        .ok_or_else(|| "`--route` requires PATH".to_owned())?
                        .clone(),
                );
            }
            "--mode" => {
                i += 1;
                let m = args
                    .get(i)
                    .ok_or_else(|| "`--mode` requires `step` or `live`".to_owned())?;
                if !matches!(m.as_str(), "step" | "live") {
                    return Err(format!("invalid `--mode` `{m}` (expected step|live)"));
                }
                opts.mode = Some(m.clone());
            }
            "--token" => {
                i += 1;
                opts.token = Some(
                    args.get(i)
                        .ok_or_else(|| "`--token` requires a value".to_owned())?
                        .clone(),
                );
            }
            other => return Err(format!("unknown `launch` flag `{other}`")),
        }
        i += 1;
    }
    Ok(opts)
}

fn parse_fps(args: &[String]) -> Result<CliCommand, String> {
    let mut window = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--window" => {
                i += 1;
                window = Some(
                    args.get(i)
                        .ok_or_else(|| "`--window` requires `<start..end>`".to_owned())?
                        .clone(),
                );
            }
            other => return Err(format!("unknown `fps` argument `{other}`")),
        }
        i += 1;
    }
    Ok(CliCommand::Fps { window })
}

fn parse_screenshot(args: &[String]) -> Result<CliCommand, String> {
    let mut grid = false;
    let mut cursor = false;
    let mut crop = None;
    let mut out = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--grid" => grid = true,
            "--cursor" => cursor = true,
            "--crop" => {
                i += 1;
                crop = Some(
                    args.get(i)
                        .ok_or_else(|| "`--crop` requires `x,y,w,h`".to_owned())?
                        .clone(),
                );
            }
            "-o" | "--out" => {
                i += 1;
                out = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| "`-o` requires an output path".to_owned())?,
                ));
            }
            other => return Err(format!("unknown `screenshot` flag `{other}`")),
        }
        i += 1;
    }
    Ok(CliCommand::Screenshot {
        grid,
        cursor,
        crop,
        out,
    })
}

fn parse_frame_json(args: &[String]) -> Result<CliCommand, String> {
    let mut grid = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--grid" => {
                i += 1;
                grid = Some(
                    args.get(i)
                        .ok_or_else(|| "`--grid` requires `<cols>x<rows>`".to_owned())?
                        .clone(),
                );
            }
            other => return Err(format!("unknown `frame-json` flag `{other}`")),
        }
        i += 1;
    }
    Ok(CliCommand::FrameJson { grid })
}

fn parse_act(args: &[String]) -> Result<CliCommand, String> {
    let Some(first) = args.first() else {
        return Err("`act` requires a JSON object or action subcommand".to_owned());
    };
    if first.trim_start().starts_with('{') {
        if args.len() > 1 {
            return Err("`act <raw-json>` takes a single JSON argument".to_owned());
        }
        // Validate syntax locally before hitting the wire.
        let val = parse_json(first).map_err(|e| format!("invalid JSON for `act`: {e}"))?;
        if val.as_object().is_none() {
            return Err("`act` JSON must be an object".to_owned());
        }
        return Ok(CliCommand::ActRaw {
            json: first.clone(),
        });
    }
    parse_friendly_action(first.as_str(), &args[1..])
}

fn parse_i32_arg(args: &[String], idx: usize, name: &str) -> Result<i32, String> {
    let raw = args
        .get(idx)
        .ok_or_else(|| format!("missing `{name}` argument"))?;
    raw.parse::<i32>()
        .map_err(|_| format!("invalid integer `{raw}` for `{name}`"))
}

fn parse_f64_arg(args: &[String], idx: usize, name: &str) -> Result<f64, String> {
    let raw = args
        .get(idx)
        .ok_or_else(|| format!("missing `{name}` argument"))?;
    let v: f64 = raw
        .parse()
        .map_err(|_| format!("invalid number `{raw}` for `{name}`"))?;
    if !v.is_finite() {
        return Err(format!("non-finite number `{raw}` for `{name}`"));
    }
    Ok(v)
}

fn parse_u64_flag(args: &[String], idx: usize, flag: &str) -> Result<u64, String> {
    let raw = args
        .get(idx)
        .ok_or_else(|| format!("`{flag}` requires an integer"))?;
    raw.parse::<u64>()
        .map_err(|_| format!("invalid integer `{raw}` for `{flag}`"))
}

fn parse_f64_flag(args: &[String], idx: usize, flag: &str) -> Result<f64, String> {
    let raw = args
        .get(idx)
        .ok_or_else(|| format!("`{flag}` requires a number"))?;
    let v: f64 = raw
        .parse()
        .map_err(|_| format!("invalid number `{raw}` for `{flag}`"))?;
    if !v.is_finite() {
        return Err(format!("non-finite number `{raw}` for `{flag}`"));
    }
    Ok(v)
}

fn parse_friendly_action(kind: &str, args: &[String]) -> Result<CliCommand, String> {
    match kind {
        "click" => {
            let x = parse_i32_arg(args, 0, "x")?;
            let y = parse_i32_arg(args, 1, "y")?;
            let mut right = false;
            let mut poll = false;
            let mut hold_flips: Option<u64> = None;
            let mut hold_ms: Option<u64> = None;
            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--right" => right = true,
                    "--poll" => poll = true,
                    "--hold-flips" => {
                        i += 1;
                        hold_flips = Some(parse_u64_flag(args, i, "--hold-flips")?);
                    }
                    "--hold-ms" => {
                        i += 1;
                        hold_ms = Some(parse_u64_flag(args, i, "--hold-ms")?);
                    }
                    other => return Err(format!("unknown `click` flag `{other}`")),
                }
                i += 1;
            }
            let action = if poll { "click_poll" } else { "click" };
            let button = if right { "right" } else { "left" };
            let mut out =
                format!("{{\"action\":\"{action}\",\"x\":{x},\"y\":{y},\"button\":\"{button}\"");
            if let Some(hf) = hold_flips {
                let _ = write!(out, ",\"hold_flips\":{hf}");
            }
            if let Some(hm) = hold_ms {
                let _ = write!(out, ",\"hold_ms\":{hm}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "move" => {
            let x = parse_i32_arg(args, 0, "x")?;
            let y = parse_i32_arg(args, 1, "y")?;
            let mut glide_flips: Option<u64> = None;
            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--glide-flips" => {
                        i += 1;
                        glide_flips = Some(parse_u64_flag(args, i, "--glide-flips")?);
                    }
                    other => return Err(format!("unknown `move` flag `{other}`")),
                }
                i += 1;
            }
            let mut out = format!("{{\"action\":\"move\",\"x\":{x},\"y\":{y}");
            if let Some(gf) = glide_flips {
                let _ = write!(out, ",\"glide_flips\":{gf}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "delta" => {
            let dx = parse_f64_arg(args, 0, "dx")?;
            let dy = parse_f64_arg(args, 1, "dy")?;
            let mut flips: Option<u64> = None;
            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--flips" => {
                        i += 1;
                        flips = Some(parse_u64_flag(args, i, "--flips")?);
                    }
                    other => return Err(format!("unknown `delta` flag `{other}`")),
                }
                i += 1;
            }
            let mut out = String::from("{\"action\":\"mouse_delta\",\"dx\":");
            write_json_f64(&mut out, dx);
            out.push_str(",\"dy\":");
            write_json_f64(&mut out, dy);
            if let Some(f) = flips {
                let _ = write!(out, ",\"flips\":{f}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "drag" => {
            let x1 = parse_i32_arg(args, 0, "x1")?;
            let y1 = parse_i32_arg(args, 1, "y1")?;
            let x2 = parse_i32_arg(args, 2, "x2")?;
            let y2 = parse_i32_arg(args, 3, "y2")?;
            let mut right = false;
            let mut step_px: Option<u64> = None;
            let mut i = 4usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--right" => right = true,
                    "--step-px" => {
                        i += 1;
                        step_px = Some(parse_u64_flag(args, i, "--step-px")?);
                    }
                    other => return Err(format!("unknown `drag` flag `{other}`")),
                }
                i += 1;
            }
            let button = if right { "right" } else { "left" };
            let mut out = format!(
                "{{\"action\":\"drag\",\"from_x\":{x1},\"from_y\":{y1},\"to_x\":{x2},\"to_y\":{y2},\"button\":\"{button}\""
            );
            if let Some(sp) = step_px {
                let _ = write!(out, ",\"step_px\":{sp}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "key" => {
            let Some(name) = args.first() else {
                return Err("`key` requires `<NAME>`".to_owned());
            };
            let mut hold_flips: Option<u64> = None;
            let mut hold_ms: Option<u64> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--hold-flips" => {
                        i += 1;
                        hold_flips = Some(parse_u64_flag(args, i, "--hold-flips")?);
                    }
                    "--hold-ms" => {
                        i += 1;
                        hold_ms = Some(parse_u64_flag(args, i, "--hold-ms")?);
                    }
                    other => return Err(format!("unknown `key` flag `{other}`")),
                }
                i += 1;
            }
            let mut out = String::from("{\"action\":\"key_hold\",\"key\":");
            write_json_str(&mut out, name);
            if let Some(hf) = hold_flips {
                let _ = write!(out, ",\"hold_flips\":{hf}");
            }
            if let Some(hm) = hold_ms {
                let _ = write!(out, ",\"hold_ms\":{hm}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "chord" => {
            let Some(keys_csv) = args.first() else {
                return Err("`chord` requires `<K1,K2,...>`".to_owned());
            };
            let keys: Vec<&str> = keys_csv
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if keys.is_empty() {
                return Err("`chord` requires at least one key name".to_owned());
            }
            let mut hold_flips: Option<u64> = None;
            let mut hold_ms: Option<u64> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--hold-flips" => {
                        i += 1;
                        hold_flips = Some(parse_u64_flag(args, i, "--hold-flips")?);
                    }
                    "--hold-ms" => {
                        i += 1;
                        hold_ms = Some(parse_u64_flag(args, i, "--hold-ms")?);
                    }
                    other => return Err(format!("unknown `chord` flag `{other}`")),
                }
                i += 1;
            }
            let mut out = String::from("{\"action\":\"chord\",\"keys\":[");
            for (idx, k) in keys.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_json_str(&mut out, k);
            }
            out.push(']');
            if let Some(hf) = hold_flips {
                let _ = write!(out, ",\"hold_flips\":{hf}");
            }
            if let Some(hm) = hold_ms {
                let _ = write!(out, ",\"hold_ms\":{hm}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "type" => {
            let Some(text) = args.first() else {
                return Err("`type` requires `<TEXT>`".to_owned());
            };
            let mut flip_gap: Option<u64> = None;
            let mut i = 1usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--flip-gap" => {
                        i += 1;
                        flip_gap = Some(parse_u64_flag(args, i, "--flip-gap")?);
                    }
                    other => return Err(format!("unknown `type` flag `{other}`")),
                }
                i += 1;
            }
            let mut out = String::from("{\"action\":\"type_text\",\"text\":");
            write_json_str(&mut out, text);
            if let Some(fg) = flip_gap {
                let _ = write!(out, ",\"flip_gap\":{fg}");
            }
            out.push('}');
            Ok(CliCommand::ActRaw { json: out })
        }
        "step" => {
            let flips = match args {
                [] => 1,
                [n] => n
                    .parse::<u64>()
                    .map_err(|_| format!("invalid flip count `{n}`"))?,
                _ => return Err("`step` takes at most one `<flips>` argument".to_owned()),
            };
            Ok(CliCommand::Step { flips })
        }
        "wait" => {
            let mut flips: Option<u64> = None;
            let mut ms: Option<u64> = None;
            let mut i = 0usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--ms" => {
                        i += 1;
                        ms = Some(parse_u64_flag(args, i, "--ms")?);
                    }
                    tok if flips.is_none() && !tok.starts_with('-') => {
                        flips = Some(
                            tok.parse::<u64>()
                                .map_err(|_| format!("invalid flip count `{tok}`"))?,
                        );
                    }
                    other => return Err(format!("unknown `wait` argument `{other}`")),
                }
                i += 1;
            }
            if flips.is_none() && ms.is_none() {
                return Err("`wait` requires `<flips>` or `--ms <N>`".to_owned());
            }
            let mut map = BTreeMap::new();
            if let Some(f) = flips {
                map.insert("flips".to_owned(), JsonValue::Int(f as i64));
            }
            if let Some(m) = ms {
                map.insert("ms".to_owned(), JsonValue::Int(m as i64));
            }
            Ok(CliCommand::WaitRaw {
                json: JsonValue::Object(map).to_json(),
            })
        }
        "wait-change" => {
            let mut max_flips: u64 = 60;
            let mut min_diff: Option<f64> = None;
            let mut saw_pos = false;
            let mut i = 0usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--min-diff" => {
                        i += 1;
                        min_diff = Some(parse_f64_flag(args, i, "--min-diff")?);
                    }
                    tok if !saw_pos && !tok.starts_with('-') => {
                        max_flips = tok
                            .parse::<u64>()
                            .map_err(|_| format!("invalid max_flips `{tok}`"))?;
                        saw_pos = true;
                    }
                    other => return Err(format!("unknown `wait-change` argument `{other}`")),
                }
                i += 1;
            }
            let mut out = format!("{{\"kind\":\"until_change\",\"max_flips\":{max_flips}");
            if let Some(d) = min_diff {
                out.push_str(",\"min_diff_pct\":");
                write_json_f64(&mut out, d);
            }
            out.push('}');
            Ok(CliCommand::WaitRaw { json: out })
        }
        "wait-settled" | "settle" => {
            let mut max_flips: u64 = 120;
            let mut stable_flips: Option<u64> = None;
            let mut max_diff: Option<f64> = None;
            let mut saw_pos = false;
            let mut i = 0usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--stable-flips" => {
                        i += 1;
                        stable_flips = Some(parse_u64_flag(args, i, "--stable-flips")?);
                    }
                    "--max-diff" => {
                        i += 1;
                        max_diff = Some(parse_f64_flag(args, i, "--max-diff")?);
                    }
                    tok if !saw_pos && !tok.starts_with('-') => {
                        max_flips = tok
                            .parse::<u64>()
                            .map_err(|_| format!("invalid max_flips `{tok}`"))?;
                        saw_pos = true;
                    }
                    other => return Err(format!("unknown `wait-settled` argument `{other}`")),
                }
                i += 1;
            }
            let mut out = format!("{{\"kind\":\"until_settled\",\"max_flips\":{max_flips}");
            if let Some(sf) = stable_flips {
                let _ = write!(out, ",\"stable_flips\":{sf}");
            }
            if let Some(md) = max_diff {
                out.push_str(",\"max_diff_pct\":");
                write_json_f64(&mut out, md);
            }
            out.push('}');
            Ok(CliCommand::WaitRaw { json: out })
        }
        other => Err(format!("unknown action `{other}`")),
    }
}

fn parse_pipe(args: &[String]) -> Result<CliCommand, String> {
    let Some(source) = args.first() else {
        return Err("`pipe` requires `<file | ->`".to_owned());
    };
    let mut absolute = false;
    let mut no_wait = false;
    for flag in &args[1..] {
        match flag.as_str() {
            "--absolute" => absolute = true,
            "--no-wait" => no_wait = true,
            other => return Err(format!("unknown `pipe` flag `{other}`")),
        }
    }
    Ok(CliCommand::Pipe {
        source: source.clone(),
        absolute,
        no_wait,
    })
}

fn parse_mem(args: &[String]) -> Result<CliCommand, String> {
    let addr = args
        .first()
        .ok_or_else(|| "`mem` requires `<addr> <len>`".to_owned())?
        .clone();
    let len_raw = args
        .get(1)
        .ok_or_else(|| "`mem` requires `<addr> <len>`".to_owned())?;
    let len: usize = len_raw
        .parse()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| format!("invalid length `{len_raw}`"))?;
    let mut fmt = "hex".to_owned();
    let mut out = None;
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "--fmt" => {
                i += 1;
                let f = args
                    .get(i)
                    .ok_or_else(|| "`--fmt` requires a format".to_owned())?;
                if !matches!(
                    f.as_str(),
                    "hex" | "raw" | "u8" | "u16" | "u32" | "i32" | "f32" | "cstr"
                ) {
                    return Err(format!(
                        "invalid `--fmt` `{f}` (expected hex|raw|u8|u16|u32|i32|f32|cstr)"
                    ));
                }
                fmt = f.clone();
            }
            "-o" | "--out" => {
                i += 1;
                out = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| "`-o` requires an output path".to_owned())?,
                ));
            }
            other => return Err(format!("unknown `mem` flag `{other}`")),
        }
        i += 1;
    }
    Ok(CliCommand::Mem {
        addr,
        len,
        fmt,
        out,
    })
}

fn parse_mem_search(args: &[String]) -> Result<CliCommand, String> {
    let mut map = BTreeMap::new();
    let mut has_pattern = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--hex" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "`--hex` requires hex bytes".to_owned())?;
                map.insert("bytes_hex".to_owned(), JsonValue::Str(v.clone()));
                has_pattern = true;
            }
            "--u32" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "`--u32` requires a value".to_owned())?;
                map.insert("u32".to_owned(), JsonValue::Str(v.clone()));
                has_pattern = true;
            }
            "--ascii" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "`--ascii` requires text".to_owned())?;
                map.insert("ascii".to_owned(), JsonValue::Str(v.clone()));
                has_pattern = true;
            }
            "--start" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "`--start` requires an address".to_owned())?;
                map.insert("start".to_owned(), JsonValue::Str(v.clone()));
            }
            "--end" => {
                i += 1;
                let v = args
                    .get(i)
                    .ok_or_else(|| "`--end` requires an address".to_owned())?;
                map.insert("end".to_owned(), JsonValue::Str(v.clone()));
            }
            "--limit" => {
                i += 1;
                let lim = parse_u64_flag(args, i, "--limit")?;
                map.insert("limit".to_owned(), JsonValue::Int(lim as i64));
            }
            other => return Err(format!("unknown `mem-search` flag `{other}`")),
        }
        i += 1;
    }
    if !has_pattern {
        return Err("`mem-search` requires one of `--hex`, `--u32`, or `--ascii`".to_owned());
    }
    Ok(CliCommand::MemSearch {
        json: JsonValue::Object(map).to_json(),
    })
}

fn parse_files(args: &[String]) -> Result<CliCommand, String> {
    match args {
        [] => Ok(CliCommand::Files {
            root: "disc".to_owned(),
            path: String::new(),
        }),
        [a] => {
            if matches!(a.as_str(), "disc" | "profile") {
                Ok(CliCommand::Files {
                    root: a.clone(),
                    path: String::new(),
                })
            } else {
                Ok(CliCommand::Files {
                    root: "disc".to_owned(),
                    path: a.clone(),
                })
            }
        }
        [root, path] => {
            if !matches!(root.as_str(), "disc" | "profile") {
                return Err(format!("invalid root `{root}` (expected disc|profile)"));
            }
            Ok(CliCommand::Files {
                root: root.clone(),
                path: path.clone(),
            })
        }
        _ => Err("`files` takes `[disc|profile] [path]`".to_owned()),
    }
}

fn parse_read_file(args: &[String]) -> Result<CliCommand, String> {
    let root = args
        .first()
        .ok_or_else(|| "`read-file` requires `<disc|profile> <path>`".to_owned())?;
    if !matches!(root.as_str(), "disc" | "profile") {
        return Err(format!("invalid root `{root}` (expected disc|profile)"));
    }
    let path = args
        .get(1)
        .ok_or_else(|| "`read-file` requires `<disc|profile> <path>`".to_owned())?
        .clone();
    let mut out = None;
    let mut i = 2usize;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                out = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| "`-o` requires an output path".to_owned())?,
                ));
            }
            other => return Err(format!("unknown `read-file` flag `{other}`")),
        }
        i += 1;
    }
    Ok(CliCommand::ReadFile {
        root: root.clone(),
        path,
        out,
    })
}

fn parse_bookmark(args: &[String]) -> Result<CliCommand, String> {
    match args {
        [] => Ok(CliCommand::Bookmark { name: None }),
        [name] => Ok(CliCommand::Bookmark {
            name: Some(name.clone()),
        }),
        _ => Err("`bookmark` takes at most one `<name>`".to_owned()),
    }
}

fn parse_export_route(args: &[String]) -> Result<CliCommand, String> {
    let mut out = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                out = Some(PathBuf::from(
                    args.get(i)
                        .ok_or_else(|| "`-o` requires an output path".to_owned())?,
                ));
            }
            tok if out.is_none() && !tok.starts_with('-') => {
                out = Some(PathBuf::from(tok));
            }
            other => return Err(format!("unknown `export-route` argument `{other}`")),
        }
        i += 1;
    }
    Ok(CliCommand::ExportRoute { out })
}

fn parse_mode(args: &[String]) -> Result<CliCommand, String> {
    let Some(first) = args.first().map(String::as_str) else {
        return Err(
            "`mode` requires one of step|live|pause|resume|agent|human|shared|close|pace"
                .to_owned(),
        );
    };
    let mut map = BTreeMap::new();
    match first {
        "step" | "live" => {
            map.insert("mode".to_owned(), JsonValue::Str(first.to_owned()));
            if let Some(pace_str) = args.get(1) {
                let p: f64 = pace_str
                    .parse()
                    .map_err(|_| format!("invalid pace `{pace_str}`"))?;
                map.insert("pace".to_owned(), JsonValue::Float(p));
            }
            if args.len() > 2 {
                return Err(format!("unexpected extra argument `{}`", args[2]));
            }
        }
        "pause" => {
            expect_no_extra("mode pause", &args[1..])?;
            map.insert("pause".to_owned(), JsonValue::Bool(true));
        }
        "resume" => {
            expect_no_extra("mode resume", &args[1..])?;
            map.insert("resume".to_owned(), JsonValue::Bool(true));
        }
        "agent" | "human" | "shared" => {
            expect_no_extra("mode owner", &args[1..])?;
            map.insert("owner".to_owned(), JsonValue::Str(first.to_owned()));
        }
        "close" => {
            let mut force = false;
            for f in &args[1..] {
                match f.as_str() {
                    "--force" => force = true,
                    other => return Err(format!("unknown `mode close` flag `{other}`")),
                }
            }
            map.insert("close".to_owned(), JsonValue::Bool(true));
            if force {
                map.insert("force".to_owned(), JsonValue::Bool(true));
            }
        }
        "pace" => {
            let pace_str = args
                .get(1)
                .ok_or_else(|| "`mode pace` requires a multiplier (e.g. 1.0)".to_owned())?;
            let p: f64 = pace_str
                .parse()
                .map_err(|_| format!("invalid pace `{pace_str}`"))?;
            map.insert("pace".to_owned(), JsonValue::Float(p));
        }
        other => {
            return Err(format!(
                "unknown `mode` subcommand `{other}` (expected step|live|pause|resume|agent|human|shared|close|pace)"
            ));
        }
    }
    Ok(CliCommand::Mode {
        json: JsonValue::Object(map).to_json(),
    })
}

/// Execute a parsed CLI command against the live control plane.
///
/// # Errors
/// Returns a user-facing error message (including HTTP 4xx/5xx bodies).
pub fn execute_cli<R: Read, W: Write>(
    parsed: &ParsedCli,
    stdin: &mut R,
    stdout: &mut W,
) -> Result<(), String> {
    match &parsed.command {
        CliCommand::Help => {
            writeln!(stdout, "{USAGE}").map_err(|e| e.to_string())?;
            return Ok(());
        }
        CliCommand::Version => {
            writeln!(stdout, "lego-ctrl {}", env!("CARGO_PKG_VERSION"))
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        CliCommand::Sessions => {
            let list = list_live_sessions(true);
            writeln!(stdout, "{}", sessions_to_json(&list)).map_err(|e| e.to_string())?;
            return Ok(());
        }
        CliCommand::Launch(opts) => {
            let mut opts = opts.clone();
            if opts.token.is_none() {
                opts.token.clone_from(&parsed.global.token);
            }
            let res = launch_title(&opts)?;
            writeln!(stdout, "{}", res.to_json()).map_err(|e| e.to_string())?;
            return Ok(());
        }
        _ => {}
    }

    let client = resolve_client(
        parsed.global.url.as_deref(),
        parsed.global.title.as_deref(),
        parsed.global.token.as_deref(),
    )?;

    match &parsed.command {
        CliCommand::Watch => {
            let watch = client.watch_url().ok_or_else(|| {
                format!(
                    "endpoint `{}` has no browser /watch URL (use TCP --control)",
                    client.url()
                )
            })?;
            writeln!(stdout, "{watch}").map_err(|e| e.to_string())?;
            Ok(())
        }
        CliCommand::Status => write_text_resp(client.get("/api/status")?, stdout),
        CliCommand::Meta => write_text_resp(client.get("/api/meta")?, stdout),
        CliCommand::Controls => write_text_resp(client.get("/api/controls")?, stdout),
        CliCommand::Goals => write_text_resp(client.get("/api/goals")?, stdout),
        CliCommand::Fps { window } => {
            let path = match window {
                Some(w) => format!("/api/fps?window={}", url_encode(w)),
                None => "/api/fps".to_owned(),
            };
            write_text_resp(client.get(&path)?, stdout)
        }
        CliCommand::Screenshot {
            grid,
            cursor,
            crop,
            out,
        } => {
            let path = build_frame_png_query(*grid, *cursor, crop.as_deref());
            let resp = check_http(client.get(&path)?)?;
            if let Some(file_path) = out {
                std::fs::write(file_path, &resp.body)
                    .map_err(|e| format!("failed to write `{}`: {e}", file_path.display()))?;
                let mut meta = String::from("{\"ok\":true,\"path\":");
                write_json_str(&mut meta, &file_path.to_string_lossy());
                let _ = write!(meta, ",\"bytes\":{}", resp.body.len());
                if let Some(flip) = resp.header("x-flip") {
                    let _ = write!(meta, ",\"flip\":{flip}");
                }
                if let Some(hash) = resp.header("x-frame-hash") {
                    meta.push_str(",\"frame_hash\":");
                    write_json_str(&mut meta, hash);
                }
                meta.push('}');
                writeln!(stdout, "{meta}").map_err(|e| e.to_string())?;
            } else {
                stdout.write_all(&resp.body).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        CliCommand::FrameJson { grid } => {
            let path = match grid {
                Some(g) => format!("/api/frame.json?grid={}", url_encode(g)),
                None => "/api/frame.json".to_owned(),
            };
            write_text_resp(client.get(&path)?, stdout)
        }
        CliCommand::ActRaw { json } => write_text_resp(client.post_json("/api/act", json)?, stdout),
        CliCommand::WaitRaw { json } => {
            write_text_resp(client.post_json("/api/wait", json)?, stdout)
        }
        CliCommand::Step { flips } => {
            let body = format!("{{\"flips\":{flips}}}");
            write_text_resp(client.post_json("/api/step", &body)?, stdout)
        }
        CliCommand::Pipe {
            source,
            absolute,
            no_wait,
        } => {
            let payload = if source == "-" {
                let mut buf = String::new();
                stdin
                    .read_to_string(&mut buf)
                    .map_err(|e| format!("failed to read stdin: {e}"))?;
                buf
            } else {
                std::fs::read_to_string(source)
                    .map_err(|e| format!("failed to read `{source}`: {e}"))?
            };
            let trimmed = payload.trim();
            let content_type =
                if trimmed.starts_with("#lego-legacy-route") || trimmed.starts_with("flip ") {
                    "text/plain; charset=utf-8"
                } else {
                    "application/x-ndjson; charset=utf-8"
                };
            let mut qs = Vec::new();
            if *absolute {
                qs.push("relative=0");
            }
            if *no_wait {
                qs.push("wait=0");
            }
            let target = if qs.is_empty() {
                "/api/pipe".to_owned()
            } else {
                format!("/api/pipe?{}", qs.join("&"))
            };
            write_text_resp(
                client.post_raw(&target, content_type, payload.as_bytes())?,
                stdout,
            )
        }
        CliCommand::Mem {
            addr,
            len,
            fmt,
            out,
        } => {
            let target = format!(
                "/api/mem?addr={}&len={len}&fmt={}",
                url_encode(addr),
                url_encode(fmt)
            );
            let resp = check_http(client.get(&target)?)?;
            if fmt == "raw" {
                write_binary_or_file(&resp, out.as_deref(), stdout)
            } else {
                write_text_body(&resp, stdout)
            }
        }
        CliCommand::MemMap => write_text_resp(client.get("/api/mem/map")?, stdout),
        CliCommand::MemSearch { json } => {
            write_text_resp(client.post_json("/api/mem/search", json)?, stdout)
        }
        CliCommand::Cpu => write_text_resp(client.get("/api/cpu")?, stdout),
        CliCommand::Win32 => write_text_resp(client.get("/api/win32")?, stdout),
        CliCommand::Files { root, path } => {
            let target = format!(
                "/api/files?root={}&path={}",
                url_encode(root),
                url_encode(path)
            );
            write_text_resp(client.get(&target)?, stdout)
        }
        CliCommand::ReadFile { root, path, out } => {
            let target = format!(
                "/api/files/read?root={}&path={}",
                url_encode(root),
                url_encode(path)
            );
            let resp = check_http(client.get(&target)?)?;
            write_binary_or_file(&resp, out.as_deref(), stdout)
        }
        CliCommand::Bookmark { name } => match name {
            Some(n) => {
                let mut body = String::from("{\"name\":");
                write_json_str(&mut body, n);
                body.push('}');
                write_text_resp(client.post_json("/api/bookmark", &body)?, stdout)
            }
            None => write_text_resp(client.get("/api/bookmark")?, stdout),
        },
        CliCommand::Rewind { name } => {
            let mut body = String::from("{\"name\":");
            write_json_str(&mut body, name);
            body.push('}');
            write_text_resp(client.post_json("/api/rewind", &body)?, stdout)
        }
        CliCommand::Snapshot { path } => {
            let mut body = String::from("{\"path\":");
            write_json_str(&mut body, path);
            body.push('}');
            write_text_resp(client.post_json("/api/snapshot", &body)?, stdout)
        }
        CliCommand::Restore { path } => {
            let mut body = String::from("{\"path\":");
            write_json_str(&mut body, path);
            body.push('}');
            write_text_resp(client.post_json("/api/restore", &body)?, stdout)
        }
        CliCommand::ExportRoute { out } => {
            let resp = check_http(client.get("/api/route")?)?;
            if let Some(file_path) = out {
                std::fs::write(file_path, &resp.body)
                    .map_err(|e| format!("failed to write `{}`: {e}", file_path.display()))?;
                let mut meta = String::from("{\"ok\":true,\"path\":");
                write_json_str(&mut meta, &file_path.to_string_lossy());
                let _ = write!(meta, ",\"bytes\":{}}}", resp.body.len());
                writeln!(stdout, "{meta}").map_err(|e| e.to_string())?;
            } else {
                write_text_body(&resp, stdout)?;
            }
            Ok(())
        }
        CliCommand::AgentState { field, value } => {
            let mut body = format!("{{\"{field}\":");
            write_json_str(&mut body, value);
            body.push('}');
            write_text_resp(client.post_json("/api/agent-state", &body)?, stdout)
        }
        CliCommand::Mode { json } => write_text_resp(client.post_json("/api/mode", json)?, stdout),
        CliCommand::Help | CliCommand::Version | CliCommand::Sessions | CliCommand::Launch(_) => {
            unreachable!()
        }
    }
}

/// Build `/api/frame.png` query path from overlay options.
#[must_use]
pub fn build_frame_png_query(grid: bool, cursor: bool, crop: Option<&str>) -> String {
    let mut params = Vec::new();
    if grid {
        params.push("grid=1".to_owned());
    }
    if cursor {
        params.push("cursor=1".to_owned());
    }
    if let Some(c) = crop.filter(|s| !s.trim().is_empty()) {
        params.push(format!("crop={}", url_encode(c)));
    }
    if params.is_empty() {
        "/api/frame.png".to_owned()
    } else {
        format!("/api/frame.png?{}", params.join("&"))
    }
}

fn check_http(resp: HttpResponse) -> Result<HttpResponse, String> {
    if resp.is_success() {
        Ok(resp)
    } else {
        let detail = resp.body_text();
        if detail.trim().is_empty() {
            Err(format!("HTTP {} {}", resp.status, resp.reason))
        } else {
            Err(format!("HTTP {}: {}", resp.status, detail.trim()))
        }
    }
}

fn write_text_resp<W: Write>(resp: HttpResponse, stdout: &mut W) -> Result<(), String> {
    let resp = check_http(resp)?;
    write_text_body(&resp, stdout)
}

fn write_text_body<W: Write>(resp: &HttpResponse, stdout: &mut W) -> Result<(), String> {
    stdout.write_all(&resp.body).map_err(|e| e.to_string())?;
    if !resp.body.ends_with(b"\n") {
        stdout.write_all(b"\n").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn write_binary_or_file<W: Write>(
    resp: &HttpResponse,
    out: Option<&std::path::Path>,
    stdout: &mut W,
) -> Result<(), String> {
    if let Some(file_path) = out {
        std::fs::write(file_path, &resp.body)
            .map_err(|e| format!("failed to write `{}`: {e}", file_path.display()))?;
        let mut meta = String::from("{\"ok\":true,\"path\":");
        write_json_str(&mut meta, &file_path.to_string_lossy());
        let _ = write!(meta, ",\"bytes\":{}", resp.body.len());
        if let Some(sha) = resp.header("x-sha256") {
            meta.push_str(",\"sha256\":");
            write_json_str(&mut meta, sha);
        }
        meta.push('}');
        writeln!(stdout, "{meta}").map_err(|e| e.to_string())?;
    } else {
        stdout.write_all(&resp.body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Convenience helper for MCP / programmatic callers to execute `GET /api/frame.png`.
///
/// # Errors
/// Returns an error if the HTTP request fails or returns a non-2xx status.
pub fn fetch_frame_png(
    client: &Client,
    grid: bool,
    cursor: bool,
    crop: Option<&str>,
) -> Result<HttpResponse, String> {
    let target = build_frame_png_query(grid, cursor, crop);
    check_http(client.get(&target)?)
}
