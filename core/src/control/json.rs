//! Shared zero-crate RFC 8259 JSON parser/serializer, Base64 encoder, and
//! control-plane codecs shared across `stdio.rs`, `observe.rs`, `inspect.rs`,
//! and `http.rs` (#308, #309).

use super::bus::{
    AgentState, AgentStatePatch, ControlStatus, FrameDiff, FrameInfoData, WaitResult,
};
use super::timeline::Action;
use crate::host::telemetry::{FpsReport, WindowMetrics};
use crate::host::Button;
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Minimal zero-crate JSON value tree for command and HTTP body parsing.
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

    /// Parse a `u32` from either a JSON integer or a hex (`0x...`) / decimal string.
    #[must_use]
    pub fn as_u32_flexible(&self) -> Option<u32> {
        if let Some(n) = self.as_u64() {
            return u32::try_from(n).ok();
        }
        self.as_str().and_then(parse_u32_addr)
    }

    /// Serialize this value back to compact RFC 8259 JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    pub fn write_json(&self, out: &mut String) {
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

/// Parse a guest `u32` address from `0x...` hex or decimal text.
#[must_use]
pub fn parse_u32_addr(s: &str) -> Option<u32> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        t.parse::<u32>().ok()
    }
}

/// Decode a whitespace-tolerant hex byte string (with optional `0x` prefix).
///
/// # Errors
/// Returns an error if the string has odd length, is empty, or contains non-hex characters.
pub fn parse_hex_bytes(s: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = s
        .trim()
        .strip_prefix("0x")
        .or_else(|| s.trim().strip_prefix("0X"))
        .unwrap_or(s.trim())
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    if cleaned.is_empty() || !cleaned.len().is_multiple_of(2) {
        return Err("bytes_hex must have a non-empty even number of hex digits".to_owned());
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for pair in cleaned.as_bytes().as_chunks::<2>().0 {
        let txt = std::str::from_utf8(pair).map_err(|_| "invalid utf8 in hex".to_owned())?;
        let b = u8::from_str_radix(txt, 16).map_err(|_| format!("invalid hex byte `{txt}`"))?;
        out.push(b);
    }
    Ok(out)
}

/// Standard RFC 4648 Base64 encoder (zero crates).
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
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

/// Write a finite `f64` (falling back to `0.0` if non-finite).
pub fn write_json_f64(out: &mut String, v: f64) {
    if v.is_finite() {
        let _ = write!(out, "{v}");
    } else {
        out.push_str("0.0");
    }
}

/// Write an `f64` as JSON, emitting `null` for `NaN`/`±Inf` (preserves precision in inspect views).
pub fn write_json_f64_or_null(out: &mut String, v: f64) {
    if v.is_finite() {
        let _ = write!(out, "{v}");
    } else {
        out.push_str("null");
    }
}

/// Parse a mouse button field (`"left"` or `"right"`, default `Left`).
///
/// # Errors
/// Returns an error if `"button"` is present and not `"left"` or `"right"`.
pub fn parse_button(obj: &BTreeMap<String, JsonValue>) -> Result<Button, String> {
    match obj.get("button") {
        None | Some(JsonValue::Null) => Ok(Button::Left),
        Some(JsonValue::Str(s)) if s == "left" => Ok(Button::Left),
        Some(JsonValue::Str(s)) if s == "right" => Ok(Button::Right),
        Some(JsonValue::Str(other)) => {
            Err(format!("unknown button `{other}` (expected left|right)"))
        }
        Some(_) => Err("`button` must be a string (expected left|right)".to_owned()),
    }
}

pub fn req_i32(obj: &BTreeMap<String, JsonValue>, key: &str) -> Result<i32, String> {
    obj.get(key)
        .and_then(JsonValue::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("missing or invalid integer `{key}`"))
}

pub fn opt_i32(obj: &BTreeMap<String, JsonValue>, key: &str) -> Result<Option<i32>, String> {
    match obj.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| format!("invalid integer `{key}`")),
    }
}

fn opt_u64_default(
    obj: &BTreeMap<String, JsonValue>,
    key: &str,
    default: u64,
) -> Result<u64, String> {
    match obj.get(key) {
        None | Some(JsonValue::Null) => Ok(default),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| format!("`{key}` must be a non-negative integer")),
    }
}

/// Parse a high-level [`Action`] from a JSON object and action kind name.
///
/// # Errors
/// Returns a descriptive error on missing/invalid fields or unknown action kind.
pub fn parse_action_obj(kind: &str, obj: &BTreeMap<String, JsonValue>) -> Result<Action, String> {
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
        "mouse_down" | "mouse_up" => {
            let x = opt_i32(obj, "x")?;
            let y = opt_i32(obj, "y")?;
            if x.is_some() != y.is_some() {
                return Err(format!(
                    "`{kind}` requires both `x` and `y` when specifying coordinates"
                ));
            }
            let button = parse_button(obj)?;
            Ok(if kind == "mouse_down" {
                Action::MouseDown { x, y, button }
            } else {
                Action::MouseUp { x, y, button }
            })
        }
        "click" => Ok(Action::Click {
            x: opt_i32(obj, "x")?,
            y: opt_i32(obj, "y")?,
            button: parse_button(obj)?,
            hold_flips: obj
                .get("hold_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(1),
            hold_ms: obj.get("hold_ms").and_then(JsonValue::as_u64),
            settle_flips: opt_u64_default(obj, "settle_flips", 1)?,
        }),
        "click_poll" => {
            if matches!(obj.get("settle_flips"), Some(v) if !matches!(v, JsonValue::Null)) {
                return Err("`settle_flips` is not supported on `click_poll`".to_owned());
            }
            Ok(Action::ClickPoll {
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
            })
        }
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
                lead_flips: opt_u64_default(obj, "lead_flips", 1)?,
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

/// Parse an [`AgentStatePatch`] from a JSON object.
///
/// # Errors
/// Returns an error if `subgoals` is present and not an array of strings.
pub fn parse_agent_state_patch(
    obj: &BTreeMap<String, JsonValue>,
) -> Result<AgentStatePatch, String> {
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
    Ok(AgentStatePatch {
        goal: opt_str("goal"),
        subgoals,
        thought: opt_str("thought"),
        commentary: opt_str("commentary"),
        notepad: opt_str("notepad"),
        user_hint: opt_str("user_hint"),
    })
}

pub fn write_diff(out: &mut String, d: &FrameDiff) {
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

pub fn write_status(out: &mut String, st: &ControlStatus) {
    out.push_str("{\"title\":");
    write_json_str(out, &st.title);
    out.push_str(",\"flip\":");
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
        ",\"route_active\":{},\"route_remaining\":{},\"recording_len\":{},\"pointer\":[{},{}],\"pointer_pos\":[{},{}],\"hw_pointer_pos\":[{},{}],\"pointer_shown\":{},\"cursor_blank\":{},\"held_keys\":[",
        st.route_active,
        st.route_remaining,
        st.recording_len,
        st.pointer_pos.0,
        st.pointer_pos.1,
        st.pointer_pos.0,
        st.pointer_pos.1,
        st.hw_pointer_pos.0,
        st.hw_pointer_pos.1,
        st.pointer_shown,
        st.cursor_blank
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
    out.push_str(",\"diff_pct\":");
    write_json_f64(out, st.diff.diff_pct);
    let _ = write!(
        out,
        ",\"unchanged_streak\":{},\"diff\":",
        st.diff.unchanged_streak
    );
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

pub fn write_window_metrics(out: &mut String, m: &WindowMetrics) {
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

pub fn write_fps_report(out: &mut String, r: &FpsReport) {
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

pub fn write_frame_info(out: &mut String, f: &FrameInfoData) {
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

pub fn write_agent_state(out: &mut String, a: &AgentState) {
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

pub fn write_wait_result(out: &mut String, w: &WaitResult) {
    let _ = write!(
        out,
        "{{\"satisfied\":{},\"flips_elapsed\":{},\"diff_pct\":",
        w.satisfied, w.flips_elapsed
    );
    write_json_f64(out, w.diff_pct);
    out.push('}');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_hex_helpers_work() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(parse_u32_addr("0x00401000"), Some(0x0040_1000));
        assert_eq!(parse_u32_addr("4096"), Some(4096));
        assert_eq!(
            parse_hex_bytes("0xde ad_be ef").unwrap(),
            [0xde, 0xad, 0xbe, 0xef]
        );
        assert!(parse_hex_bytes("abc").is_err());
    }
}
