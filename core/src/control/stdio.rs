//! Zero-crate newline-delimited JSON (JSONL) stdio transport (`--control-stdio`) (#308).
//!
//! Reads one JSON command per line from `stdin` on a worker thread, dispatches
//! through [`ControlBus::dispatch`] / [`ControlBus::dispatch_sync`], and writes
//! single-line JSON responses to `stdout`. All diagnostics go to `stderr` or
//! `RunLog`, keeping `stdout` strictly machine-parseable JSONL.

use super::bus::{
    AgentState, AgentStatePatch, Command, ControlBus, ControlStatus, ExecMode, FrameDiff,
    FrameInfoData, InputOwner, Response, ResponseData, WaitResult,
};
use super::timeline::Action;
use crate::host::telemetry::{FpsReport, WindowMetrics};
use crate::host::Button;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::thread;

/// Minimal zero-crate JSON value tree for command parsing.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    #[must_use]
    pub fn as_object(&self) -> Option<&BTreeMap<String, JsonValue>> {
        match self {
            Self::Object(map) => Some(map),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(*f as i64),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        self.as_i64().and_then(|n| u64::try_from(n).ok())
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(f) if f.is_finite() => Some(*f),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Serialize this value back to compact RFC 8259 JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Self::Float(f) => write_json_f64(out, *f),
            Self::Str(s) => write_json_str(out, s),
            Self::Array(arr) => {
                out.push('[');
                for (i, item) in arr.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Self::Object(map) => {
                out.push('{');
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_str(out, k);
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while let Some(&b) = self.bytes.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        self.skip_ws();
        let b = self
            .peek()
            .ok_or_else(|| "unexpected end of JSON".to_owned())?;
        match b {
            b'{' => self.parse_object(),
            b'[' => self.parse_array(),
            b'"' => self.parse_string().map(JsonValue::Str),
            b't' => self.expect_literal(b"true", JsonValue::Bool(true)),
            b'f' => self.expect_literal(b"false", JsonValue::Bool(false)),
            b'n' => self.expect_literal(b"null", JsonValue::Null),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(format!("unexpected byte `{}` at {}", b as char, self.pos)),
        }
    }

    fn expect_literal(&mut self, lit: &[u8], val: JsonValue) -> Result<JsonValue, String> {
        if self.bytes.get(self.pos..self.pos + lit.len()) == Some(lit) {
            self.pos += lit.len();
            Ok(val)
        } else {
            Err(format!("invalid literal at {}", self.pos))
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '{'
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(format!("expected object key at {}", self.pos));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("expected `:` at {}", self.pos));
            }
            self.pos += 1;
            let val = self.parse_value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(map));
                }
                _ => return Err(format!("expected `,` or `}}` at {}", self.pos)),
            }
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.pos += 1; // '['
        let mut arr = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(arr));
        }
        loop {
            arr.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(arr));
                }
                _ => return Err(format!("expected `,` or `]` at {}", self.pos)),
            }
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let slice = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| "truncated \\u escape".to_owned())?;
        let text = std::str::from_utf8(slice).map_err(|_| "invalid utf8 in \\u".to_owned())?;
        let val = u16::from_str_radix(text, 16).map_err(|_| format!("bad \\u escape `{text}`"))?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // '"'
        let mut out = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let esc = *self
                    .bytes
                    .get(self.pos)
                    .ok_or_else(|| "truncated string escape".to_owned())?;
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\x08'),
                    b'f' => out.push('\x0c'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let code = self.parse_hex4()?;
                        if (0xd800..=0xdbff).contains(&code) {
                            if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                return Err("unpaired UTF-16 high surrogate".to_owned());
                            }
                            self.pos += 2;
                            let low = self.parse_hex4()?;
                            if !(0xdc00..=0xdfff).contains(&low) {
                                return Err("invalid UTF-16 low surrogate".to_owned());
                            }
                            let cp = 0x10000
                                + ((u32::from(code) - 0xd800) << 10)
                                + (u32::from(low) - 0xdc00);
                            let ch = char::from_u32(cp)
                                .ok_or_else(|| "invalid unicode codepoint".to_owned())?;
                            out.push(ch);
                        } else {
                            let ch = char::from_u32(u32::from(code))
                                .ok_or_else(|| "invalid unicode codepoint".to_owned())?;
                            out.push(ch);
                        }
                    }
                    _ => return Err(format!("invalid escape `\\{}`", esc as char)),
                }
            } else if b < 0x20 {
                return Err("unescaped control character in JSON string".to_owned());
            } else {
                let rest = std::str::from_utf8(&self.bytes[self.pos..])
                    .map_err(|_| "invalid utf-8 in JSON string".to_owned())?;
                let ch = rest.chars().next().ok_or_else(|| "eof".to_owned())?;
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err("unterminated JSON string".to_owned())
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut is_float = false;
        while let Some(&b) = self.bytes.get(self.pos) {
            match b {
                b'0'..=b'9' => self.pos += 1,
                b'.' | b'e' | b'E' => {
                    is_float = true;
                    self.pos += 1;
                }
                b'+' | b'-'
                    if self.pos > start && matches!(self.bytes[self.pos - 1], b'e' | b'E') =>
                {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| "invalid utf-8 in number".to_owned())?;
        if is_float {
            let f: f64 = text
                .parse()
                .map_err(|_| format!("invalid float `{text}`"))?;
            if !f.is_finite() {
                return Err(format!("non-finite number `{text}`"));
            }
            Ok(JsonValue::Float(f))
        } else {
            let n: i64 = text
                .parse()
                .map_err(|_| format!("invalid integer `{text}`"))?;
            Ok(JsonValue::Int(n))
        }
    }
}

/// Parse one RFC 8259 JSON value.
///
/// # Errors
/// Returns a descriptive syntax error on malformed input or trailing garbage.
pub fn parse_json(input: &str) -> Result<JsonValue, String> {
    let mut parser = JsonParser::new(input);
    let val = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(format!("trailing characters at {}", parser.pos));
    }
    Ok(val)
}

/// Parsed JSONL command line ready to dispatch onto [`ControlBus`].
#[derive(Clone, Debug)]
pub struct ParsedLine {
    pub id: Option<JsonValue>,
    pub commands: Vec<Command>,
    pub sync: bool,
}

fn parse_button(obj: &BTreeMap<String, JsonValue>) -> Result<Button, String> {
    match obj.get("button").and_then(JsonValue::as_str) {
        None | Some("left") => Ok(Button::Left),
        Some("right") => Ok(Button::Right),
        Some(other) => Err(format!("unknown button `{other}` (expected left|right)")),
    }
}

fn req_i32(obj: &BTreeMap<String, JsonValue>, key: &str) -> Result<i32, String> {
    obj.get(key)
        .and_then(JsonValue::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("missing or invalid integer `{key}`"))
}

fn opt_i32(obj: &BTreeMap<String, JsonValue>, key: &str) -> Result<Option<i32>, String> {
    match obj.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("invalid integer `{key}`")),
    }
}

fn parse_action_obj(kind: &str, obj: &BTreeMap<String, JsonValue>) -> Result<Action, String> {
    match kind {
        "move" => Ok(Action::Move {
            x: req_i32(obj, "x")?,
            y: req_i32(obj, "y")?,
            glide_flips: obj
                .get("glide_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(1),
        }),
        "mouse_delta" => Ok(Action::MouseDelta {
            dx: obj
                .get("dx")
                .and_then(JsonValue::as_f64)
                .ok_or_else(|| "missing or invalid `dx`".to_owned())?,
            dy: obj
                .get("dy")
                .and_then(JsonValue::as_f64)
                .ok_or_else(|| "missing or invalid `dy`".to_owned())?,
            flips: obj.get("flips").and_then(JsonValue::as_u64).unwrap_or(1),
            scale_x: obj.get("scale_x").and_then(JsonValue::as_f64),
            scale_y: obj.get("scale_y").and_then(JsonValue::as_f64),
        }),
        "click" => Ok(Action::Click {
            x: opt_i32(obj, "x")?,
            y: opt_i32(obj, "y")?,
            button: parse_button(obj)?,
            hold_flips: obj
                .get("hold_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(1),
            hold_ms: obj.get("hold_ms").and_then(JsonValue::as_u64),
        }),
        "click_poll" => Ok(Action::ClickPoll {
            x: opt_i32(obj, "x")?,
            y: opt_i32(obj, "y")?,
            button: parse_button(obj)?,
            hold_flips: obj
                .get("hold_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(2),
            hold_ms: obj
                .get("hold_ms")
                .and_then(JsonValue::as_u64)
                .unwrap_or(150),
        }),
        "drag" => Ok(Action::Drag {
            from_x: req_i32(obj, "from_x")?,
            from_y: req_i32(obj, "from_y")?,
            to_x: req_i32(obj, "to_x")?,
            to_y: req_i32(obj, "to_y")?,
            button: parse_button(obj)?,
            step_px: obj
                .get("step_px")
                .and_then(JsonValue::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(16),
        }),
        "key_hold" => {
            let key = obj
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "missing string `key`".to_owned())?
                .to_owned();
            Ok(Action::KeyHold {
                key,
                hold_flips: obj
                    .get("hold_flips")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(1),
                hold_ms: obj.get("hold_ms").and_then(JsonValue::as_u64),
            })
        }
        "chord" => {
            let arr = obj
                .get("keys")
                .and_then(JsonValue::as_array)
                .ok_or_else(|| "missing array `keys`".to_owned())?;
            let mut keys = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or_else(|| "`keys` entries must be strings".to_owned())?;
                keys.push(s.to_owned());
            }
            Ok(Action::Chord {
                keys,
                hold_flips: obj
                    .get("hold_flips")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(1),
                hold_ms: obj.get("hold_ms").and_then(JsonValue::as_u64),
            })
        }
        "type_text" => {
            let text = obj
                .get("text")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "missing string `text`".to_owned())?
                .to_owned();
            Ok(Action::TypeText {
                text,
                flip_gap: obj.get("flip_gap").and_then(JsonValue::as_u64).unwrap_or(2),
            })
        }
        other => Err(format!("unknown action `{other}`")),
    }
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
        "set_agent_state" => {
            let subgoals = match obj.get("subgoals") {
                None | Some(JsonValue::Null) => None,
                Some(JsonValue::Array(arr)) => {
                    let mut list = Vec::with_capacity(arr.len());
                    for item in arr {
                        list.push(
                            item.as_str()
                                .ok_or_else(|| "`subgoals` items must be strings".to_owned())?
                                .to_owned(),
                        );
                    }
                    Some(list)
                }
                _ => return Err("`subgoals` must be an array of strings".to_owned()),
            };
            let opt_str = |k: &str| obj.get(k).and_then(JsonValue::as_str).map(str::to_owned);
            vec![Command::SetAgentState(AgentStatePatch {
                goal: opt_str("goal"),
                subgoals,
                thought: opt_str("thought"),
                commentary: opt_str("commentary"),
                notepad: opt_str("notepad"),
                user_hint: opt_str("user_hint"),
            })]
        }
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
                .unwrap_or(true);
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
                let step_uncapped = obj.get("fast").and_then(JsonValue::as_bool);
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

/// Write a properly escaped JSON string literal.
pub fn write_json_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_json_f64(out: &mut String, v: f64) {
    if v.is_finite() {
        let _ = write!(out, "{v}");
    } else {
        out.push_str("0.0");
    }
}

fn write_diff(out: &mut String, d: &FrameDiff) {
    out.push_str("{\"diff_pixels\":");
    let _ = write!(out, "{}", d.diff_pixels);
    out.push_str(",\"total_pixels\":");
    let _ = write!(out, "{}", d.total_pixels);
    out.push_str(",\"diff_pct\":");
    write_json_f64(out, d.diff_pct);
    out.push_str(",\"diff_bbox\":");
    match d.diff_bbox {
        Some((x, y, w, h)) => {
            let _ = write!(out, "[{x},{y},{w},{h}]");
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"unchanged_streak\":");
    let _ = write!(out, "{}", d.unchanged_streak);
    out.push('}');
}

fn write_status(out: &mut String, st: &ControlStatus) {
    out.push_str("{\"flip\":");
    match st.flip {
        Some(f) => {
            let _ = write!(out, "{f}");
        }
        None => out.push_str("null"),
    }
    let _ = write!(
        out,
        ",\"cursor\":{},\"declared_ms\":{},\"instructions_passed\":{},\"mode\":",
        st.cursor, st.declared_ms, st.instructions_passed
    );
    write_json_str(out, st.mode.as_str());
    out.push_str(",\"owner\":");
    write_json_str(out, st.owner.as_str());
    let _ = write!(
        out,
        ",\"paused\":{},\"finished\":{},\"step_uncapped\":{},\"pace\":",
        st.paused, st.finished, st.step_uncapped
    );
    write_json_f64(out, st.pace);
    let _ = write!(
        out,
        ",\"route_active\":{},\"route_remaining\":{},\"pointer_pos\":[{},{}],\"pointer_shown\":{},\"held_keys\":[",
        st.route_active,
        st.route_remaining,
        st.pointer_pos.0,
        st.pointer_pos.1,
        st.pointer_shown
    );
    for (i, k) in st.held_keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{k}");
    }
    let _ = write!(
        out,
        "],\"pending_actions\":{},\"frame_width\":{},\"frame_height\":{},\"frame_hash\":",
        st.pending_actions, st.frame_width, st.frame_height
    );
    match &st.frame_hash {
        Some(h) => write_json_str(out, h),
        None => out.push_str("null"),
    }
    out.push_str(",\"diff\":");
    write_diff(out, &st.diff);
    out.push_str(",\"outcome\":");
    match &st.outcome {
        Some(o) => write_json_str(out, o),
        None => out.push_str("null"),
    }
    out.push_str(",\"digest\":");
    match &st.digest {
        Some(d) => write_json_str(out, d),
        None => out.push_str("null"),
    }
    out.push('}');
}

fn write_window_metrics(out: &mut String, m: &WindowMetrics) {
    let _ = write!(
        out,
        "{{\"flips\":{},\"wall_nanos\":{},\"declared_nanos\":{},\"executed_insns\":{},\"throughput_mips\":",
        m.flips, m.wall_nanos, m.declared_nanos, m.executed_insns
    );
    write_json_f64(out, m.throughput_mips);
    out.push_str(",\"pace\":");
    write_json_f64(out, m.pace);
    out.push_str(",\"host_fps\":");
    write_json_f64(out, m.host_fps);
    out.push_str(",\"guest_fps\":");
    write_json_f64(out, m.guest_fps);
    out.push_str(",\"intended_fps\":");
    match m.intended_fps {
        Some(hz) => write_json_f64(out, hz),
        None => out.push_str("null"),
    }
    out.push_str(",\"p50_ms\":");
    write_json_f64(out, m.p50_ms);
    out.push_str(",\"p95_ms\":");
    write_json_f64(out, m.p95_ms);
    out.push_str(",\"p99_ms\":");
    write_json_f64(out, m.p99_ms);
    out.push_str(",\"max_ms\":");
    write_json_f64(out, m.max_ms);
    let _ = write!(
        out,
        ",\"over_budget_flips\":{},\"over_budget_pct\":",
        m.over_budget_flips
    );
    write_json_f64(out, m.over_budget_pct);
    let _ = write!(
        out,
        ",\"stall_flips\":{},\"uniform_flips\":{},\"audio_underruns\":",
        m.stall_flips, m.uniform_flips
    );
    match m.audio_underruns {
        Some(u) => {
            let _ = write!(out, "{u}");
        }
        None => out.push_str("null"),
    }
    out.push('}');
}

fn write_fps_report(out: &mut String, r: &FpsReport) {
    out.push_str("{\"full\":");
    write_window_metrics(out, &r.full);
    out.push_str(",\"window\":");
    match &r.window {
        Some((start, end, m)) => {
            let _ = write!(out, "{{\"start\":{start},\"end\":{end},\"metrics\":");
            write_window_metrics(out, m);
            out.push('}');
        }
        None => out.push_str("null"),
    }
    out.push('}');
}

fn write_frame_info(out: &mut String, f: &FrameInfoData) {
    let _ = write!(
        out,
        "{{\"flip\":{},\"width\":{},\"height\":{},\"frame_hash\":",
        f.flip, f.width, f.height
    );
    write_json_str(out, &f.frame_hash);
    out.push_str(",\"uniform\":");
    out.push_str(if f.uniform { "true" } else { "false" });
    out.push_str(",\"diff\":");
    write_diff(out, &f.diff);
    out.push('}');
}

fn write_agent_state(out: &mut String, a: &AgentState) {
    out.push_str("{\"goal\":");
    write_json_str(out, &a.goal);
    out.push_str(",\"subgoals\":[");
    for (i, s) in a.subgoals.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_str(out, s);
    }
    out.push_str("],\"thought\":");
    write_json_str(out, &a.thought);
    out.push_str(",\"commentary\":");
    write_json_str(out, &a.commentary);
    out.push_str(",\"notepad\":");
    write_json_str(out, &a.notepad);
    out.push_str(",\"user_hint\":");
    write_json_str(out, &a.user_hint);
    out.push_str(",\"action_feed\":[");
    for (i, e) in a.action_feed.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"seq\":{},\"flip\":{},\"declared_ms\":{},\"source\":",
            e.seq, e.flip, e.declared_ms
        );
        write_json_str(out, &e.source);
        out.push_str(",\"summary\":");
        write_json_str(out, &e.summary);
        out.push('}');
    }
    out.push_str("]}");
}

fn write_wait_result(out: &mut String, w: &WaitResult) {
    let _ = write!(
        out,
        "{{\"satisfied\":{},\"flips_elapsed\":{},\"diff_pct\":",
        w.satisfied, w.flips_elapsed
    );
    write_json_f64(out, w.diff_pct);
    out.push('}');
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
    use crate::host::headless::Headless;
    use crate::machine::Machine;
    use crate::route::Route;
    use crate::runlog::RunLog;
    use crate::Title;
    use std::io::Cursor;

    #[test]
    fn json_parser_and_command_parser_handle_all_primitives() {
        // Basic JSON roundtrip including escapes and unicode.
        let v = parse_json(r#"{"a":1,"b":-2.5,"c":"hi\n\"world\"\u0041","d":[true,false,null]}"#)
            .unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj["a"].as_i64(), Some(1));
        assert_eq!(obj["b"].as_f64(), Some(-2.5));
        assert_eq!(obj["c"].as_str(), Some("hi\n\"world\"A"));
        assert_eq!(obj["d"].as_array().unwrap().len(), 3);

        // Parse commands: act, shorthand, pipe_route, mode, wait_until_change, set_agent_state.
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

        // Export recorded route and replay on pure Headless -> assert bit-exact match.
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
