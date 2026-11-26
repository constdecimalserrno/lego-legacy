//! End-to-end stdio JSON-RPC 2.0 integration tests for `lego-mcp` (#312).

use lego_ctrl::json::{base64_encode, parse_json};
use lego_mcp::parsed_tools_list;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const TINY_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
];

struct MockControlServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockControlServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        handle_one(&mut stream);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for MockControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_one<S: Read + Write>(stream: &mut S) {
    let mut reader = BufReader::new(&mut *stream);
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let target = parts.next().unwrap_or("").to_owned();
    let mut headers = BTreeMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let t = line.trim_end_matches(['\r', '\n']);
        if t.is_empty() {
            break;
        }
        if let Some((k, v)) = t.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let len: usize = headers
        .get("content-length")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    if len > 0 {
        let _ = reader.read_exact(&mut body);
    }
    drop(reader);

    let path = target.split('?').next().unwrap_or(&target);
    let send = |s: &mut S, status: &str, ct: &str, payload: &[u8]| {
        let _ = write!(
            s,
            "HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        let _ = s.write_all(payload);
    };

    match (method.as_str(), path) {
        ("GET", "/api/status") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"title":"stub","flip":7,"mode":"step","paused":true}"#,
        ),
        ("GET", "/api/frame.json") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"flip":7,"width":640,"height":480,"warnings":["stuck_loop: 30"]}"#,
        ),
        ("GET", "/api/frame.png") => send(stream, "200 OK", "image/png", TINY_PNG),
        ("GET", "/api/meta") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"title":"stub"}"#,
        ),
        ("GET", "/api/controls") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"actions":["click"]}"#,
        ),
        ("GET", "/api/goals") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"goals":[]}"#,
        ),
        ("GET", "/api/cpu") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"eip":"0x00401000"}"#,
        ),
        ("GET", "/api/mem") => send(
            stream,
            "200 OK",
            "application/json",
            br#"{"ok":true,"hex":"deadbeef"}"#,
        ),
        ("GET", "/api/files/read") => {
            send(stream, "200 OK", "application/octet-stream", b"hello-save")
        }
        ("GET", "/api/route") => send(stream, "200 OK", "text/plain", b"#lego-legacy-route 1\n"),
        ("POST", "/api/act")
        | ("POST", "/api/pipe")
        | ("POST", "/api/clear-queue")
        | ("POST", "/api/agent-state")
        | ("POST", "/api/bookmark")
        | ("POST", "/api/mode") => send(
            stream,
            "200 OK",
            "application/json",
            format!(
                "{{\"ok\":true,\"target\":\"{target}\",\"cleared\":2,\"status\":{{\"flip\":8}}}}"
            )
            .as_bytes(),
        ),
        _ => send(
            stream,
            "404 Not Found",
            "application/json",
            br#"{"ok":false,"error":"not found"}"#,
        ),
    }
}

#[test]
fn tool_schema_json_is_valid_and_contains_all_nine_tools() {
    let val = parsed_tools_list().expect("valid tools_list_json");
    let arr = val.as_array().expect("tools array");
    let names: Vec<&str> = arr
        .iter()
        .map(|t| t.as_object().unwrap()["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "game_sessions",
            "game_launch",
            "game_meta",
            "game_observe",
            "game_act",
            "game_inspect",
            "game_goal",
            "game_checkpoint",
            "game_mode",
        ]
    );

    let act_tool = arr
        .iter()
        .find(|t| t.as_object().unwrap()["name"].as_str() == Some("game_act"))
        .expect("game_act schema");
    let props = act_tool.as_object().unwrap()["inputSchema"]
        .as_object()
        .unwrap()["properties"]
        .as_object()
        .unwrap();
    let lead = props
        .get("lead_flips")
        .expect("game_act exposes lead_flips")
        .as_object()
        .unwrap();
    assert_eq!(lead["type"].as_str(), Some("integer"));
    assert_eq!(lead["minimum"].as_u64(), Some(0));
}

#[test]
fn lego_mcp_binary_handles_jsonrpc_over_piped_stdio() {
    let srv = MockControlServer::start();
    let url = srv.url();

    let mut child = Command::new(env!("CARGO_BIN_EXE_lego-mcp"))
        .args(["--url", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lego-mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let send_recv = |stdin: &mut std::process::ChildStdin,
                     reader: &mut BufReader<std::process::ChildStdout>,
                     req: &str|
     -> serde_like::Obj {
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        let n = reader.read_line(&mut line).unwrap();
        assert!(n > 0, "unexpected EOF waiting for response to {req}");
        let val = parse_json(&line).unwrap_or_else(|e| panic!("invalid JSON `{line}`: {e}"));
        val.as_object().unwrap().clone()
    };

    // 1. initialize
    let init = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
    );
    assert_eq!(init["id"].as_i64(), Some(1));
    let res = init["result"].as_object().unwrap();
    assert_eq!(res["protocolVersion"].as_str(), Some("2024-11-05"));
    assert_eq!(
        res["serverInfo"].as_object().unwrap()["name"].as_str(),
        Some("lego-mcp")
    );

    // 2. notifications/initialized (MUST produce no response!)
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    stdin.flush().unwrap();

    // 3. ping (confirms notification produced no stray reply ahead of id=2)
    let ping = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#,
    );
    assert_eq!(ping["id"].as_i64(), Some(2));
    assert!(ping["result"].as_object().is_some());

    // 4. tools/list
    let list = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#,
    );
    let tools = list["result"].as_object().unwrap()["tools"]
        .as_array()
        .unwrap();
    assert_eq!(tools.len(), 9);

    // 5. tools/call: game_observe -> TextContent + ImageContent (PNG base64)
    let obs = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"game_observe","arguments":{"include_image":true,"grid":true,"cursor":true}}}"#,
    );
    let obs_res = obs["result"].as_object().unwrap();
    assert_eq!(obs_res["isError"].as_bool(), Some(false));
    let content = obs_res["content"].as_array().unwrap();
    assert_eq!(content.len(), 2);
    let text_item = content[0].as_object().unwrap();
    assert_eq!(text_item["type"].as_str(), Some("text"));
    assert!(text_item["text"].as_str().unwrap().contains("stuck_loop"));
    let img_item = content[1].as_object().unwrap();
    assert_eq!(img_item["type"].as_str(), Some("image"));
    assert_eq!(img_item["mimeType"].as_str(), Some("image/png"));
    assert_eq!(
        img_item["data"].as_str(),
        Some(base64_encode(TINY_PNG).as_str())
    );

    // 6. tools/call: game_act with include_image=true
    let act = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"game_act","arguments":{"action":"click","x":120,"y":80,"include_image":true}}}"#,
    );
    let act_content = act["result"].as_object().unwrap()["content"]
        .as_array()
        .unwrap();
    assert_eq!(act_content.len(), 2);
    assert_eq!(
        act_content[1].as_object().unwrap()["type"].as_str(),
        Some("image")
    );

    // 7. tools/call: game_inspect (cpu + file_read)
    let cpu = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"game_inspect","arguments":{"kind":"cpu"}}}"#,
    );
    assert!(cpu["result"].as_object().unwrap()["content"]
        .as_array()
        .unwrap()[0]
        .as_object()
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .contains("0x00401000"));

    let fread = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"game_inspect","arguments":{"kind":"file_read","root":"profile","path":"SAVE.DAT"}}}"#,
    );
    let fread_text = fread["result"].as_object().unwrap()["content"]
        .as_array()
        .unwrap()[0]
        .as_object()
        .unwrap()["text"]
        .as_str()
        .unwrap();
    assert!(fread_text.contains("hello-save"));

    // 8. tools/call: game_goal, game_checkpoint, game_mode, game_meta
    for (id, payload) in [
        (
            8,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"game_goal","arguments":{"goal":"Finish tutorial","thought":"Ready"}}}"#,
        ),
        (
            9,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"game_checkpoint","arguments":{"op":"export_route"}}}"#,
        ),
        (
            10,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"game_mode","arguments":{"mode":"live","pace":2.0}}}"#,
        ),
        (
            11,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"game_meta","arguments":{}}}"#,
        ),
        (
            13,
            r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"game_act","arguments":{"action":"clear_queue"}}}"#,
        ),
        (
            14,
            r##"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"game_act","arguments":{"pipe":"#lego-legacy-route 1\nflip 100 down\n"}}}"##,
        ),
    ] {
        let r = send_recv(&mut stdin, &mut reader, payload);
        assert_eq!(r["id"].as_i64(), Some(id));
        let res = r["result"].as_object().unwrap();
        assert_eq!(res["isError"].as_bool(), Some(false), "id {id}: {r:?}");
        let body_text = res["content"].as_array().unwrap()[0].as_object().unwrap()["text"]
            .as_str()
            .unwrap();
        if id == 13 {
            assert!(
                body_text.contains("/api/clear-queue"),
                "game_act clear_queue must hit /api/clear-queue: {body_text}"
            );
        }
        if id == 14 {
            assert!(
                body_text.contains("relative=0"),
                "game_act pipe must default to relative=0 (absolute): {body_text}"
            );
        }
    }

    // 9. Unknown tool returns JSON-RPC error -32602.
    let bad = send_recv(
        &mut stdin,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"bogus_tool","arguments":{}}}"#,
    );
    assert_eq!(
        bad["error"].as_object().unwrap()["code"].as_i64(),
        Some(-32602)
    );

    drop(stdin);
    let status = child.wait().expect("wait lego-mcp");
    assert!(status.success());
}

mod serde_like {
    pub type Obj = std::collections::BTreeMap<String, lego_ctrl::json::JsonValue>;
}
