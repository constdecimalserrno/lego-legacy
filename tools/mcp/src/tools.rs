//! MCP tool definitions and HTTP control-plane adapters for `lego-mcp` (#312).

use lego_ctrl::cli::fetch_frame_png;
use lego_ctrl::http::{Client, HttpResponse};
use lego_ctrl::json::{base64_encode, parse_json, url_encode, write_json_str, JsonValue};
use lego_ctrl::launch::{launch_title, LaunchOptions};
use lego_ctrl::sessions::{list_live_sessions, resolve_client, sessions_to_json};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Default target resolution overrides supplied via `lego-mcp` CLI flags.
#[derive(Clone, Debug, Default)]
pub struct ServerDefaults {
    pub url: Option<String>,
    pub title: Option<String>,
    pub token: Option<String>,
}

/// JSON-RPC protocol error (e.g. invalid params / unknown tool).
#[derive(Clone, Debug)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

/// One MCP `content` block returned by `tools/call`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolContent {
    Text(String),
    Image {
        mime_type: &'static str,
        data_base64: String,
    },
}

/// Result payload of a `tools/call` invocation.
#[derive(Clone, Debug)]
pub struct ToolOutput {
    pub content: Vec<ToolContent>,
    pub is_error: bool,
}

impl ToolOutput {
    #[must_use]
    pub fn text(text: String) -> Self {
        Self {
            content: vec![ToolContent::Text(text)],
            is_error: false,
        }
    }

    #[must_use]
    pub fn error(message: String) -> Self {
        Self {
            content: vec![ToolContent::Text(message)],
            is_error: true,
        }
    }

    /// Serialize this tool output as the JSON-RPC `result` object.
    #[must_use]
    pub fn to_result_json(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push_str("{\"content\":[");
        for (i, item) in self.content.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            match item {
                ToolContent::Text(text) => {
                    out.push_str("{\"type\":\"text\",\"text\":");
                    write_json_str(&mut out, text);
                    out.push('}');
                }
                ToolContent::Image {
                    mime_type,
                    data_base64,
                } => {
                    out.push_str("{\"type\":\"image\",\"mimeType\":");
                    write_json_str(&mut out, mime_type);
                    out.push_str(",\"data\":");
                    write_json_str(&mut out, data_base64);
                    out.push('}');
                }
            }
        }
        let _ = write!(out, "],\"isError\":{}}}", self.is_error);
        out
    }
}

/// Static JSON array describing all 9 `lego-mcp` tools for `tools/list`.
#[must_use]
pub const fn tools_list_json() -> &'static str {
    r##"[
  {
    "name": "game_sessions",
    "description": "Discover live running lego-legacy game sessions (pruning stale PIDs).",
    "inputSchema": {
      "type": "object",
      "properties": {}
    }
  },
  {
    "name": "game_launch",
    "description": "Launch a lego-legacy title binary with --control and wait up to 10s for its HTTP control endpoint to become ready.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "title": { "type": "string", "description": "Title binary key (run `lego-legacy --help` to list installed titles)." },
        "window": { "type": "boolean", "description": "Open a host window instead of running headless (default false)." },
        "headless": { "type": "boolean", "description": "Run headless (default true unless window=true)." },
        "bind": { "type": "string", "description": "HTTP bind address HOST:PORT (default 127.0.0.1:0)." },
        "data_root": { "type": "string", "description": "Custom --data-root directory for profile/saves." },
        "route": { "type": "string", "description": "Initial --route file to replay on boot." },
        "mode": { "type": "string", "enum": ["step", "live"], "description": "Initial control execution mode." },
        "token": { "type": "string", "description": "Optional Bearer token for mutating POST routes." },
        "stub": { "type": "boolean", "description": "Run the built-in --stub test guest." }
      },
      "required": ["title"]
    }
  },
  {
    "name": "game_meta",
    "description": "Fetch title metadata (/api/meta), declared controls (/api/controls), and goals (/api/goals).",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string", "description": "Optional control URL override." },
        "title": { "type": "string", "description": "Optional live session title filter." },
        "token": { "type": "string", "description": "Optional Bearer token override." }
      }
    }
  },
  {
    "name": "game_observe",
    "description": "Observe live run status (/api/status), frame diff/stuck-loop warnings (/api/frame.json), and an optional native PNG screenshot (ImageContent) with grid/cursor/crop overlays.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string" },
        "title": { "type": "string" },
        "token": { "type": "string" },
        "include_image": { "type": "boolean", "description": "Attach PNG ImageContent (default true)." },
        "grid": { "type": "boolean", "description": "Overlay 80x80 labeled coordinate grid on PNG." },
        "cursor": { "type": "boolean", "description": "Overlay high-contrast cursor crosshair on PNG." },
        "crop": { "type": "string", "description": "Crop and 2x zoom rectangle x,y,w,h." },
        "frame_grid": { "type": "string", "description": "Downsampled RGB grid for /api/frame.json (default 16x12)." }
      }
    }
  },
  {
    "name": "game_act",
    "description": "Execute an input action primitive (move, mouse_delta, mouse_down, mouse_up, click, click_poll, drag, key_hold, chord, type_text), a batch of actions, step/wait, or a pipe route/JSONL script; optionally return a post-action PNG screenshot.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string" },
        "title": { "type": "string" },
        "token": { "type": "string" },
        "action": { "type": "string", "description": "Action kind: move|mouse_delta|mouse_down|mouse_up|click|click_poll|drag|key_hold|chord|type_text|step|wait|wait_until_change|wait_until_settled|pipe|clear_queue." },
        "actions": { "type": "array", "description": "Batch of action objects to dispatch to /api/act." },
        "pipe": { "type": "string", "description": "#lego-legacy-route 1 text or JSONL commands to post to /api/pipe." },
        "route": { "type": "string", "description": "Alias for pipe route text." },
        "relative": { "type": "boolean", "description": "For pipe routes, interpret bare flip N numbers relative to current flip (default false: bare flip N is absolute matching #lego-legacy-route 1; explicit flip +N is always relative)." },
        "x": { "type": "integer" },
        "y": { "type": "integer" },
        "dx": { "type": "number" },
        "dy": { "type": "number" },
        "from_x": { "type": "integer" },
        "from_y": { "type": "integer" },
        "to_x": { "type": "integer" },
        "to_y": { "type": "integer" },
        "button": { "type": "string", "enum": ["left", "right"] },
        "hold_flips": { "type": "integer" },
        "hold_ms": { "type": "integer" },
        "settle_flips": { "type": "integer", "minimum": 0, "description": "Flips between Move and Down on `click` when coordinates are given (default 1; 0 to opt out)." },
        "lead_flips": { "type": "integer", "minimum": 0, "description": "Flips prefix keys lead before the final key and linger after on multi-key `chord` (default 1; 0 for simultaneous press/release or static poll loops with `hold_ms`)." },
        "glide_flips": { "type": "integer" },
        "flips": { "type": "integer" },
        "step_px": { "type": "integer" },
        "key": { "type": "string" },
        "keys": { "type": "array", "items": { "type": "string" } },
        "text": { "type": "string" },
        "flip_gap": { "type": "integer" },
        "ms": { "type": "integer" },
        "max_flips": { "type": "integer" },
        "min_diff_pct": { "type": "number" },
        "stable_flips": { "type": "integer" },
        "max_diff_pct": { "type": "number" },
        "wait": { "type": "boolean" },
        "auto_step": { "type": "boolean" },
        "queue": { "type": "boolean" },
        "include_image": { "type": "boolean", "description": "Attach post-action PNG ImageContent (default false)." },
        "grid": { "type": "boolean" },
        "cursor": { "type": "boolean" },
        "crop": { "type": "string" }
      }
    }
  },
  {
    "name": "game_inspect",
    "description": "Read-only guest introspection: mem, mem_map, mem_search, cpu, win32, files, file_read, or fps.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string" },
        "title": { "type": "string" },
        "token": { "type": "string" },
        "kind": { "type": "string", "enum": ["mem", "mem_map", "mem_search", "cpu", "win32", "files", "file_read", "fps"] },
        "addr": { "description": "Guest address (hex string like 0x00100000 or integer) for mem." },
        "len": { "type": "integer", "description": "Byte length for mem (default 64)." },
        "fmt": { "type": "string", "enum": ["hex", "raw", "u8", "u16", "u32", "i32", "f32", "cstr"], "description": "Output format for mem (default hex)." },
        "bytes_hex": { "type": "string", "description": "Hex byte needle for mem_search." },
        "u32": { "description": "u32 needle (int or 0x... string) for mem_search." },
        "ascii": { "type": "string", "description": "ASCII needle for mem_search." },
        "start": { "description": "Optional start address for mem_search." },
        "end": { "description": "Optional end address for mem_search." },
        "limit": { "type": "integer", "description": "Maximum match count for mem_search." },
        "root": { "type": "string", "enum": ["disc", "profile"], "description": "File root for files/file_read (default disc)." },
        "path": { "type": "string", "description": "Relative path for files/file_read." },
        "window": { "type": "string", "description": "Optional flip window start..end for fps." }
      },
      "required": ["kind"]
    }
  },
  {
    "name": "game_goal",
    "description": "Update or read agent goal, subgoals, thought, commentary, notepad, and user_hint displayed live on /watch.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string" },
        "title": { "type": "string" },
        "token": { "type": "string" },
        "goal": { "type": "string" },
        "subgoals": { "type": "array", "items": { "type": "string" } },
        "thought": { "type": "string" },
        "commentary": { "type": "string" },
        "notepad": { "type": "string" },
        "user_hint": { "type": "string" }
      }
    }
  },
  {
    "name": "game_checkpoint",
    "description": "Manage deterministic checkpoints and routes: bookmark, list_bookmarks, rewind, snapshot, restore, or export_route.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string" },
        "title": { "type": "string" },
        "token": { "type": "string" },
        "op": { "type": "string", "enum": ["bookmark", "list_bookmarks", "rewind", "snapshot", "restore", "export_route"] },
        "name": { "type": "string", "description": "Bookmark name for bookmark/rewind." },
        "path": { "type": "string", "description": "File path for snapshot/restore or optional export_route output path." }
      },
      "required": ["op"]
    }
  },
  {
    "name": "game_mode",
    "description": "Control execution mode (step/live), pause/resume, pace multiplier, input owner (agent/human/shared), clear_queue, or close.",
    "inputSchema": {
      "type": "object",
      "properties": {
        "url": { "type": "string" },
        "title": { "type": "string" },
        "token": { "type": "string" },
        "mode": { "type": "string", "enum": ["step", "live"] },
        "owner": { "type": "string", "enum": ["agent", "human", "shared"] },
        "pace": { "type": "number", "description": "Live pace multiplier (0.25..=4.0)." },
        "pause": { "type": "boolean" },
        "resume": { "type": "boolean" },
        "fast": { "type": "boolean", "description": "Step uncapped mode flag." },
        "clear_queue": { "type": "boolean" },
        "close": { "type": "boolean" },
        "force": { "type": "boolean" }
      }
    }
  }
]"##
}

/// Dispatch one MCP `tools/call` request.
///
/// # Errors
/// Returns [`RpcError`] for unknown tool names (`-32602`). Runtime/HTTP failures
/// are returned as `Ok(ToolOutput { is_error: true, .. })` per the MCP spec.
pub fn call_tool(
    name: &str,
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, RpcError> {
    let outcome = match name {
        "game_sessions" => tool_sessions(),
        "game_launch" => tool_launch(args, defaults),
        "game_meta" => tool_meta(args, defaults),
        "game_observe" => tool_observe(args, defaults),
        "game_act" => tool_act(args, defaults),
        "game_inspect" => tool_inspect(args, defaults),
        "game_goal" => tool_goal(args, defaults),
        "game_checkpoint" => tool_checkpoint(args, defaults),
        "game_mode" => tool_mode(args, defaults),
        other => {
            return Err(RpcError {
                code: -32602,
                message: format!("unknown tool `{other}`"),
            });
        }
    };
    Ok(match outcome {
        Ok(out) => out,
        Err(msg) => ToolOutput::error(msg),
    })
}

fn client_from_args(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<Client, String> {
    let url = args
        .get("url")
        .and_then(JsonValue::as_str)
        .or(defaults.url.as_deref());
    let title = args
        .get("title")
        .and_then(JsonValue::as_str)
        .or(defaults.title.as_deref());
    let token = args
        .get("token")
        .and_then(JsonValue::as_str)
        .or(defaults.token.as_deref());
    resolve_client(url, title, token)
}

fn check_ok(resp: HttpResponse) -> Result<HttpResponse, String> {
    if resp.is_success() {
        Ok(resp)
    } else {
        let body = resp.body_text();
        if body.trim().is_empty() {
            Err(format!("HTTP {} {}", resp.status, resp.reason))
        } else {
            Err(format!("HTTP {}: {}", resp.status, body.trim()))
        }
    }
}

fn tool_sessions() -> Result<ToolOutput, String> {
    let sessions = list_live_sessions(true);
    Ok(ToolOutput::text(sessions_to_json(&sessions)))
}

fn tool_launch(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let title = args
        .get("title")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing required string `title`".to_owned())?;
    let mut opts = LaunchOptions::new(title);
    if let Some(w) = args.get("window").and_then(JsonValue::as_bool) {
        opts.windowed = w;
    } else if let Some(h) = args.get("headless").and_then(JsonValue::as_bool) {
        opts.windowed = !h;
    }
    opts.bind = args
        .get("bind")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    opts.data_root = args
        .get("data_root")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    opts.route = args
        .get("route")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    opts.mode = args
        .get("mode")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    opts.token = args
        .get("token")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
        .or_else(|| defaults.token.clone());
    opts.stub = args
        .get("stub")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    let res = launch_title(&opts)?;
    Ok(ToolOutput::text(res.to_json()))
}

fn tool_meta(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let client = client_from_args(args, defaults)?;
    let meta = check_ok(client.get("/api/meta")?)?;
    let controls = check_ok(client.get("/api/controls")?)?;
    let goals = check_ok(client.get("/api/goals")?)?;

    let mut out = String::with_capacity(1024);
    out.push_str("{\"meta\":");
    out.push_str(meta.body_text().trim());
    out.push_str(",\"controls\":");
    out.push_str(controls.body_text().trim());
    out.push_str(",\"goals\":");
    out.push_str(goals.body_text().trim());
    out.push('}');
    Ok(ToolOutput::text(out))
}

fn tool_observe(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let client = client_from_args(args, defaults)?;
    let status = check_ok(client.get("/api/status")?)?;

    let frame_grid = args
        .get("frame_grid")
        .and_then(JsonValue::as_str)
        .unwrap_or("16x12");
    let frame_resp = client.get(&format!("/api/frame.json?grid={}", url_encode(frame_grid)))?;
    let frame_json = if frame_resp.is_success() {
        frame_resp.body_text()
    } else if frame_resp.status == 404 {
        "null".to_owned()
    } else {
        return Err(format!(
            "HTTP {} from /api/frame.json: {}",
            frame_resp.status,
            frame_resp.body_text().trim()
        ));
    };

    let mut combined = String::with_capacity(status.body.len() + frame_json.len() + 32);
    combined.push_str("{\"status\":");
    combined.push_str(status.body_text().trim());
    combined.push_str(",\"frame\":");
    combined.push_str(frame_json.trim());
    combined.push('}');

    let mut content = vec![ToolContent::Text(combined)];

    let include_image = args
        .get("include_image")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    if include_image {
        let grid = args
            .get("grid")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let cursor = args
            .get("cursor")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let crop = args.get("crop").and_then(JsonValue::as_str);
        match fetch_frame_png(&client, grid, cursor, crop) {
            Ok(png) => {
                content.push(ToolContent::Image {
                    mime_type: "image/png",
                    data_base64: base64_encode(&png.body),
                });
            }
            Err(err) if err.contains("404") => {
                // No frame presented yet; return status/frame JSON cleanly without image.
            }
            Err(err) => return Err(err),
        }
    }

    Ok(ToolOutput {
        content,
        is_error: false,
    })
}

fn tool_act(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let client = client_from_args(args, defaults)?;

    let action_kind = args.get("action").and_then(JsonValue::as_str);
    let pipe_payload = args
        .get("pipe")
        .or_else(|| args.get("route"))
        .and_then(JsonValue::as_str);

    let resp = if let Some(text) = pipe_payload {
        let relative = args
            .get("relative")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let wait = args
            .get("wait")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true);
        let auto_step = args
            .get("auto_step")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true);
        let ct = lego_ctrl::cli::pipe_content_type(text);
        let target = format!(
            "/api/pipe?relative={}&wait={}&auto_step={}",
            u8::from(relative),
            u8::from(wait),
            u8::from(auto_step)
        );
        check_ok(client.post_raw(&target, ct, text.as_bytes())?)?
    } else if action_kind == Some("clear_queue") {
        check_ok(client.post_json("/api/clear-queue", "{}")?)?
    } else if action_kind == Some("step") {
        let mut body = BTreeMap::new();
        if let Some(v) = args.get("flips") {
            body.insert("flips".to_owned(), v.clone());
        }
        if let Some(v) = args.get("wait") {
            body.insert("wait".to_owned(), v.clone());
        }
        check_ok(client.post_json("/api/step", &JsonValue::Object(body).to_json())?)?
    } else if matches!(
        action_kind,
        Some("wait" | "wait_until_change" | "wait_until_settled")
    ) {
        let mut body = filter_meta_keys(args);
        if let Some(kind) = action_kind {
            body.remove("action");
            match kind {
                "wait_until_change" => {
                    body.insert("kind".to_owned(), JsonValue::Str("until_change".to_owned()));
                }
                "wait_until_settled" => {
                    body.insert(
                        "kind".to_owned(),
                        JsonValue::Str("until_settled".to_owned()),
                    );
                }
                _ => {}
            }
        }
        check_ok(client.post_json("/api/wait", &JsonValue::Object(body).to_json())?)?
    } else {
        let body = filter_meta_keys(args);
        check_ok(client.post_json("/api/act", &JsonValue::Object(body).to_json())?)?
    };

    let mut content = vec![ToolContent::Text(resp.body_text())];

    let include_image = args
        .get("include_image")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if include_image {
        let grid = args
            .get("grid")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let cursor = args
            .get("cursor")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let crop = args.get("crop").and_then(JsonValue::as_str);
        if let Ok(png) = fetch_frame_png(&client, grid, cursor, crop) {
            content.push(ToolContent::Image {
                mime_type: "image/png",
                data_base64: base64_encode(&png.body),
            });
        }
    }

    Ok(ToolOutput {
        content,
        is_error: false,
    })
}

fn filter_meta_keys(args: &BTreeMap<String, JsonValue>) -> BTreeMap<String, JsonValue> {
    let mut out = BTreeMap::new();
    for (k, v) in args {
        if matches!(
            k.as_str(),
            "url" | "title" | "token" | "include_image" | "grid" | "cursor" | "crop"
        ) {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    out
}

fn tool_inspect(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let kind = args
        .get("kind")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing required string `kind`".to_owned())?;
    let client = client_from_args(args, defaults)?;

    match kind {
        "mem" => {
            let addr = match args.get("addr") {
                Some(JsonValue::Str(s)) => s.clone(),
                Some(JsonValue::Int(n)) if *n >= 0 => format!("0x{n:x}"),
                _ => return Err("`kind: mem` requires `addr` (hex string or integer)".to_owned()),
            };
            let len = args.get("len").and_then(JsonValue::as_u64).unwrap_or(64);
            let fmt = args.get("fmt").and_then(JsonValue::as_str).unwrap_or("hex");
            let target = format!(
                "/api/mem?addr={}&len={len}&fmt={}",
                url_encode(&addr),
                url_encode(fmt)
            );
            let resp = check_ok(client.get(&target)?)?;
            if fmt == "raw" {
                Ok(ToolOutput::text(format_binary_payload(&resp)))
            } else {
                Ok(ToolOutput::text(resp.body_text()))
            }
        }
        "mem_map" => Ok(ToolOutput::text(
            check_ok(client.get("/api/mem/map")?)?.body_text(),
        )),
        "mem_search" => {
            let mut body = BTreeMap::new();
            for key in ["bytes_hex", "u32", "ascii", "start", "end", "limit"] {
                if let Some(v) = args.get(key) {
                    body.insert(key.to_owned(), v.clone());
                }
            }
            let resp = check_ok(
                client.post_json("/api/mem/search", &JsonValue::Object(body).to_json())?,
            )?;
            Ok(ToolOutput::text(resp.body_text()))
        }
        "cpu" => Ok(ToolOutput::text(
            check_ok(client.get("/api/cpu")?)?.body_text(),
        )),
        "win32" => Ok(ToolOutput::text(
            check_ok(client.get("/api/win32")?)?.body_text(),
        )),
        "files" => {
            let root = args
                .get("root")
                .and_then(JsonValue::as_str)
                .unwrap_or("disc");
            let path = args.get("path").and_then(JsonValue::as_str).unwrap_or("");
            let target = format!(
                "/api/files?root={}&path={}",
                url_encode(root),
                url_encode(path)
            );
            Ok(ToolOutput::text(check_ok(client.get(&target)?)?.body_text()))
        }
        "file_read" => {
            let root = args
                .get("root")
                .and_then(JsonValue::as_str)
                .unwrap_or("disc");
            let path = args
                .get("path")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "`kind: file_read` requires `path`".to_owned())?;
            let target = format!(
                "/api/files/read?root={}&path={}",
                url_encode(root),
                url_encode(path)
            );
            let resp = check_ok(client.get(&target)?)?;
            Ok(ToolOutput::text(format_binary_payload(&resp)))
        }
        "fps" => {
            let target = match args.get("window").and_then(JsonValue::as_str) {
                Some(w) if !w.is_empty() => format!("/api/fps?window={}", url_encode(w)),
                _ => "/api/fps".to_owned(),
            };
            Ok(ToolOutput::text(check_ok(client.get(&target)?)?.body_text()))
        }
        other => Err(format!(
            "unknown inspect `kind` `{other}` (expected mem|mem_map|mem_search|cpu|win32|files|file_read|fps)"
        )),
    }
}

fn format_binary_payload(resp: &HttpResponse) -> String {
    let mut out = String::with_capacity(128 + resp.body.len() * 4 / 3);
    let _ = write!(out, "{{\"ok\":true,\"bytes\":{}", resp.body.len());
    if let Some(sha) = resp.header("x-sha256") {
        out.push_str(",\"sha256\":");
        write_json_str(&mut out, sha);
    }
    out.push_str(",\"data_base64\":");
    write_json_str(&mut out, &base64_encode(&resp.body));
    if resp.body.len() <= 64 * 1024 {
        if let Ok(text) = std::str::from_utf8(&resp.body) {
            out.push_str(",\"text\":");
            write_json_str(&mut out, text);
        }
    }
    out.push('}');
    out
}

fn tool_goal(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let client = client_from_args(args, defaults)?;
    let mut patch = BTreeMap::new();
    for key in [
        "goal",
        "subgoals",
        "thought",
        "commentary",
        "notepad",
        "user_hint",
    ] {
        if let Some(v) = args.get(key) {
            patch.insert(key.to_owned(), v.clone());
        }
    }
    let resp = if patch.is_empty() {
        check_ok(client.get("/api/agent-state")?)?
    } else {
        check_ok(client.post_json("/api/agent-state", &JsonValue::Object(patch).to_json())?)?
    };
    Ok(ToolOutput::text(resp.body_text()))
}

fn tool_checkpoint(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let op = args
        .get("op")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing required string `op`".to_owned())?;
    let client = client_from_args(args, defaults)?;

    let resp = match op {
        "list_bookmarks" => check_ok(client.get("/api/bookmark")?)?,
        "bookmark" => {
            let name = args
                .get("name")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "`op: bookmark` requires `name`".to_owned())?;
            let mut body = String::from("{\"name\":");
            write_json_str(&mut body, name);
            body.push('}');
            check_ok(client.post_json("/api/bookmark", &body)?)?
        }
        "rewind" => {
            let name = args
                .get("name")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "`op: rewind` requires `name`".to_owned())?;
            let mut body = String::from("{\"name\":");
            write_json_str(&mut body, name);
            body.push('}');
            check_ok(client.post_json("/api/rewind", &body)?)?
        }
        "snapshot" => {
            let path = args
                .get("path")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "`op: snapshot` requires `path`".to_owned())?;
            let mut body = String::from("{\"path\":");
            write_json_str(&mut body, path);
            body.push('}');
            check_ok(client.post_json("/api/snapshot", &body)?)?
        }
        "restore" => {
            let path = args
                .get("path")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "`op: restore` requires `path`".to_owned())?;
            let mut body = String::from("{\"path\":");
            write_json_str(&mut body, path);
            body.push('}');
            check_ok(client.post_json("/api/restore", &body)?)?
        }
        "export_route" => {
            let r = check_ok(client.get("/api/route")?)?;
            if let Some(path) = args.get("path").and_then(JsonValue::as_str) {
                std::fs::write(path, &r.body)
                    .map_err(|e| format!("failed to write route to `{path}`: {e}"))?;
            }
            r
        }
        other => {
            return Err(format!(
                "unknown checkpoint `op` `{other}` (expected bookmark|list_bookmarks|rewind|snapshot|restore|export_route)"
            ));
        }
    };
    Ok(ToolOutput::text(resp.body_text()))
}

fn tool_mode(
    args: &BTreeMap<String, JsonValue>,
    defaults: &ServerDefaults,
) -> Result<ToolOutput, String> {
    let client = client_from_args(args, defaults)?;
    let mut body = BTreeMap::new();
    for key in [
        "mode",
        "owner",
        "pace",
        "pause",
        "resume",
        "fast",
        "step_uncapped",
        "clear_queue",
        "close",
        "force",
    ] {
        if let Some(v) = args.get(key) {
            body.insert(key.to_owned(), v.clone());
        }
    }
    if body.is_empty() {
        return Err(
            "game_mode requires at least one of mode|owner|pace|pause|resume|fast|clear_queue|close"
                .to_owned(),
        );
    }
    let resp = check_ok(client.post_json("/api/mode", &JsonValue::Object(body).to_json())?)?;
    Ok(ToolOutput::text(resp.body_text()))
}

/// Validate that the static tool schema JSON parses cleanly (used in tests).
///
/// # Errors
/// Returns any JSON syntax error.
pub fn parsed_tools_list() -> Result<JsonValue, String> {
    parse_json(tools_list_json())
}
