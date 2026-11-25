//! Zero-crate RFC 8259 JSON parser/serializer, RFC 4648 Base64 encoder, and
//! query percent-encoder (#312).

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Minimal zero-crate JSON value tree.
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

    /// Serialize this value back to compact single-line RFC 8259 JSON.
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
        self.pos += 1;
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
        self.pos += 1;
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
        self.pos += 1;
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

/// Standard RFC 4648 Base64 encoder.
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

/// Percent-encode a query parameter value per RFC 3986 unreserved characters.
#[must_use]
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
