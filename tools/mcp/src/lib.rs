//! Zero-crate stdio JSON-RPC 2.0 MCP server wrapping `lego-legacy`'s HTTP
//! control and introspection plane (#312).
//!
//! Speaks newline-delimited JSON-RPC 2.0 on `stdin`/`stdout`. Diagnostics are
//! written strictly to `stderr` so `stdout` carries only valid JSON-RPC frames.

pub mod tools;

use lego_ctrl::json::{parse_json, write_json_str, JsonValue};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
pub use tools::{call_tool, parsed_tools_list, tools_list_json, ServerDefaults, ToolContent};

const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "lego-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Process one line of JSON-RPC 2.0 input, returning `Some(reply_json)` for
/// requests or `None` for notifications and empty lines.
#[must_use]
pub fn handle_jsonrpc_line(line: &str, defaults: &ServerDefaults) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let val = match parse_json(trimmed) {
        Ok(v) => v,
        Err(err) => {
            return Some(format_error(None, -32700, &format!("parse error: {err}")));
        }
    };
    let Some(obj) = val.as_object() else {
        return Some(format_error(
            None,
            -32600,
            "invalid request: expected JSON object",
        ));
    };

    let id = obj.get("id");
    let Some(method) = obj.get("method").and_then(JsonValue::as_str) else {
        return id
            .map(|req_id| format_error(Some(req_id), -32600, "invalid request: missing `method`"));
    };

    // JSON-RPC notifications omit `id` and MUST NOT produce a response.
    let Some(req_id) = id else {
        if method != "notifications/initialized" && !method.starts_with("notifications/") {
            eprintln!("lego-mcp: ignoring notification `{method}`");
        }
        return None;
    };

    let params = obj.get("params").and_then(JsonValue::as_object);

    let reply = match method {
        "initialize" => {
            let version = params
                .and_then(|p| p.get("protocolVersion"))
                .and_then(JsonValue::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(DEFAULT_PROTOCOL_VERSION);
            let mut result = String::with_capacity(160);
            result.push_str("{\"protocolVersion\":");
            write_json_str(&mut result, version);
            result.push_str(",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":");
            write_json_str(&mut result, SERVER_NAME);
            result.push_str(",\"version\":");
            write_json_str(&mut result, SERVER_VERSION);
            result.push_str("}}");
            format_result(req_id, &result)
        }
        "ping" => format_result(req_id, "{}"),
        "tools/list" => {
            // Compact single-line JSON so stdio transport never contains embedded newlines.
            let compact = parsed_tools_list()
                .map(|v| v.to_json())
                .unwrap_or_else(|_| "[]".to_owned());
            let result = format!("{{\"tools\":{compact}}}");
            format_result(req_id, &result)
        }
        "tools/call" => {
            let Some(p) = params else {
                return Some(format_error(
                    Some(req_id),
                    -32602,
                    "invalid params: expected object with `name`",
                ));
            };
            let Some(tool_name) = p.get("name").and_then(JsonValue::as_str) else {
                return Some(format_error(
                    Some(req_id),
                    -32602,
                    "invalid params: missing string `name`",
                ));
            };
            let empty = BTreeMap::new();
            let tool_args = match p.get("arguments") {
                None | Some(JsonValue::Null) => &empty,
                Some(JsonValue::Object(map)) => map,
                Some(_) => {
                    return Some(format_error(
                        Some(req_id),
                        -32602,
                        "invalid params: `arguments` must be an object",
                    ));
                }
            };
            match call_tool(tool_name, tool_args, defaults) {
                Ok(out) => format_result(req_id, &out.to_result_json()),
                Err(rpc_err) => format_error(Some(req_id), rpc_err.code, &rpc_err.message),
            }
        }
        other => format_error(Some(req_id), -32601, &format!("method not found: {other}")),
    };

    Some(reply)
}

/// Drive the MCP JSON-RPC 2.0 stdio loop until EOF on `reader`.
///
/// # Errors
/// Returns any underlying I/O error on `reader` or `writer`.
pub fn serve_stdio<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    defaults: &ServerDefaults,
) -> io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if let Some(reply) = handle_jsonrpc_line(&line, defaults) {
            writeln!(writer, "{reply}")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn format_result(id: &JsonValue, result_json: &str) -> String {
    let mut out = String::with_capacity(result_json.len() + 40);
    out.push_str("{\"jsonrpc\":\"2.0\",\"id\":");
    id.write_json(&mut out);
    out.push_str(",\"result\":");
    out.push_str(result_json);
    out.push('}');
    out
}

fn format_error(id: Option<&JsonValue>, code: i32, message: &str) -> String {
    let mut out = String::with_capacity(message.len() + 64);
    out.push_str("{\"jsonrpc\":\"2.0\",\"id\":");
    match id {
        Some(v) => v.write_json(&mut out),
        None => out.push_str("null"),
    }
    let _ = write!(out, ",\"error\":{{\"code\":{code},\"message\":");
    write_json_str(&mut out, message);
    out.push_str("}}");
    out
}
