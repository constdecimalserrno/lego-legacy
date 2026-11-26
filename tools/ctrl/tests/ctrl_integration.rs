//! Unit and end-to-end integration tests for `lego-ctrl` (#312).

use lego_ctrl::cli::{execute_cli, parse_args, CliCommand};
use lego_ctrl::http::{read_http_response, Client, Endpoint};
use lego_ctrl::json::{base64_encode, parse_json, url_encode, JsonValue};
use lego_ctrl::sessions::{list_sessions_in, parse_session_json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Minimal valid 1x1 PNG file header + payload for binary round-trip tests.
const TINY_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H', b'D', b'R',
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
];

fn unique_tmp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("lego-ctrl-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

#[derive(Clone, Debug)]
struct RecordedReq {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct MockServer {
    addr: SocketAddr,
    recorded: Arc<Mutex<Vec<RecordedReq>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(expected_token: Option<&str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("local addr");
        listener.set_nonblocking(true).expect("nonblocking");
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let rec_clone = Arc::clone(&recorded);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let tok = expected_token.map(str::to_owned);

        let handle = thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        if let Some(req) = read_req(&mut stream) {
                            rec_clone.lock().unwrap().push(req.clone());
                            write_mock_resp(&mut stream, &req, tok.as_deref());
                        }
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
            recorded,
            stop,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn requests(&self) -> Vec<RecordedReq> {
        self.recorded.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn read_req<S: Read>(stream: &mut S) -> Option<RecordedReq> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    if reader.read_line(&mut first).ok()? == 0 {
        return None;
    }
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let mut headers = BTreeMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
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
        reader.read_exact(&mut body).ok()?;
    }
    Some(RecordedReq {
        method,
        target,
        headers,
        body,
    })
}

fn write_mock_resp<W: Write>(stream: &mut W, req: &RecordedReq, expected_token: Option<&str>) {
    let path = req.target.split('?').next().unwrap_or(&req.target);

    if req.method == "POST" && path != "/api/mem/search" {
        if let Some(expected) = expected_token {
            let expected_hdr = format!("Bearer {expected}");
            if req.headers.get("authorization").map(String::as_str) != Some(expected_hdr.as_str()) {
                let body = br#"{"ok":false,"error":"unauthorized"}"#;
                let _ = write!(
                    stream,
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
                return;
            }
        }
    }

    match (req.method.as_str(), path) {
        ("GET", "/api/status") => send_json(
            stream,
            r#"{"ok":true,"title":"stub","flip":4,"mode":"step","paused":true}"#,
        ),
        ("GET", "/api/meta") => {
            send_json(stream, r#"{"ok":true,"title":"stub","version":"1.0.0"}"#)
        }
        ("GET", "/api/controls") => send_json(stream, r#"{"ok":true,"actions":["click","move"]}"#),
        ("GET", "/api/goals") => send_json(stream, r#"{"ok":true,"goals":[]}"#),
        ("GET", "/api/fps") => send_chunked(
            stream,
            "application/json",
            &[
                br#"{"ok":true,"#,
                br#""telemetry":{"full":{"throughput_mips":12.5}}}"#,
            ],
        ),
        ("GET", "/api/frame.png") => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nX-Flip: 4\r\nX-Frame-Hash: deadbeef\r\nConnection: close\r\n\r\n",
                TINY_PNG.len()
            );
            let _ = stream.write_all(TINY_PNG);
        }
        ("GET", "/api/frame.json") => send_json(
            stream,
            r#"{"ok":true,"flip":4,"width":640,"height":480,"warnings":[]}"#,
        ),
        ("GET", "/api/mem") => send_json(
            stream,
            r#"{"ok":true,"addr":"0x00100000","len":4,"fmt":"u32","values":[42]}"#,
        ),
        ("GET", "/api/mem/map") => send_json(stream, r#"{"ok":true,"heap":{"extent":4096}}"#),
        ("POST", "/api/mem/search") => {
            send_json(stream, r#"{"ok":true,"count":1,"matches":["0x00100120"]}"#)
        }
        ("GET", "/api/cpu") => send_json(stream, r#"{"ok":true,"eip":"0x00401000"}"#),
        ("GET", "/api/win32") => send_json(stream, r#"{"ok":true,"ddraw":{}}"#),
        ("GET", "/api/files") => send_json(stream, r#"{"ok":true,"entries":["SAVE.DAT"]}"#),
        ("GET", "/api/files/read") => {
            let data = b"save-file-payload";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nX-Sha256: abc123\r\nConnection: close\r\n\r\n",
                data.len()
            );
            let _ = stream.write_all(data);
        }
        ("GET", "/api/route") => {
            let text = b"#lego-legacy-route 1\nflip 0 key Space down\n";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                text.len()
            );
            let _ = stream.write_all(text);
        }
        ("POST", "/api/act")
        | ("POST", "/api/pipe")
        | ("POST", "/api/step")
        | ("POST", "/api/wait")
        | ("POST", "/api/mode")
        | ("POST", "/api/agent-state")
        | ("POST", "/api/bookmark")
        | ("POST", "/api/rewind")
        | ("POST", "/api/snapshot")
        | ("POST", "/api/restore") => {
            send_json(stream, r#"{"ok":true}"#);
        }
        ("GET", "/api/bookmark") => send_json(stream, r#"{"ok":true,"bookmarks":[]}"#),
        _ => {
            let body = br#"{"ok":false,"error":"not found"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    }
}

fn send_json<W: Write>(stream: &mut W, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
}

fn send_chunked<W: Write>(stream: &mut W, ct: &str, chunks: &[&[u8]]) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    for c in chunks {
        let _ = write!(stream, "{:x}\r\n", c.len());
        let _ = stream.write_all(c);
        let _ = stream.write_all(b"\r\n");
    }
    let _ = stream.write_all(b"0\r\n\r\n");
}

#[test]
fn cli_parses_all_commands_and_friendly_actions() {
    let p = parse_args(["--url", "http://127.0.0.1:7777", "--token", "sec", "status"]).unwrap();
    assert_eq!(p.global.url.as_deref(), Some("http://127.0.0.1:7777"));
    assert_eq!(p.global.token.as_deref(), Some("sec"));
    assert_eq!(p.command, CliCommand::Status);

    // Negative delta coordinates must not be mistaken for flags.
    let p = parse_args(["delta", "-15.5", "20", "--flips", "3"]).unwrap();
    let CliCommand::ActRaw { json } = p.command else {
        panic!("expected ActRaw");
    };
    let obj = parse_json(&json).unwrap();
    assert_eq!(
        obj.as_object().unwrap()["action"].as_str(),
        Some("mouse_delta")
    );
    assert_eq!(obj.as_object().unwrap()["dx"].as_f64(), Some(-15.5));
    assert_eq!(obj.as_object().unwrap()["flips"].as_u64(), Some(3));

    // Friendly click both top-level and under `act`.
    for args in [
        vec![
            "click",
            "320",
            "240",
            "--right",
            "--poll",
            "--hold-flips",
            "4",
        ],
        vec![
            "act",
            "click",
            "320",
            "240",
            "--right",
            "--poll",
            "--hold-flips",
            "4",
        ],
    ] {
        let p = parse_args(args).unwrap();
        let CliCommand::ActRaw { json } = p.command else {
            panic!("expected ActRaw");
        };
        let obj = parse_json(&json).unwrap();
        let map = obj.as_object().unwrap();
        assert_eq!(map["action"].as_str(), Some("click_poll"));
        assert_eq!(map["button"].as_str(), Some("right"));
        assert_eq!(map["hold_flips"].as_u64(), Some(4));
        assert!(!map.contains_key("settle_flips"));
    }

    // Friendly click supports `--settle-flips`, validates values, and rejects `--poll`.
    let p = parse_args(["click", "320", "222", "--settle-flips", "0"]).unwrap();
    let CliCommand::ActRaw { json } = p.command else {
        panic!("expected ActRaw");
    };
    let obj = parse_json(&json).unwrap();
    let map = obj.as_object().unwrap();
    assert_eq!(map["action"].as_str(), Some("click"));
    assert_eq!(map["settle_flips"].as_u64(), Some(0));

    for bad in [
        vec!["click", "320", "222", "--poll", "--settle-flips", "1"],
        vec!["click", "320", "222", "--settle-flips", "0", "--poll"],
    ] {
        let err = parse_args(bad).unwrap_err();
        assert!(
            err.contains("--settle-flips") && err.contains("--poll"),
            "unexpected error: {err}"
        );
    }
    assert!(parse_args(["click", "320", "222", "--settle-flips", "-1"]).is_err());

    let p = parse_args(["chord", "LeftControl, A", "--hold-flips", "2"]).unwrap();
    let CliCommand::ActRaw { json } = p.command else {
        panic!("expected ActRaw");
    };
    let keys = parse_json(&json).unwrap().as_object().unwrap()["keys"]
        .as_array()
        .unwrap()
        .to_vec();
    assert_eq!(
        keys,
        vec![
            JsonValue::Str("LeftControl".to_owned()),
            JsonValue::Str("A".to_owned())
        ]
    );
    assert!(
        !parse_json(&json)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("lead_flips"),
        "omitted --lead-flips leaves server default"
    );

    let p = parse_args(["chord", "F2,W", "--hold-flips", "2", "--lead-flips", "0"]).unwrap();
    let CliCommand::ActRaw { json } = p.command else {
        panic!("expected ActRaw");
    };
    let obj = parse_json(&json).unwrap();
    let map = obj.as_object().unwrap();
    assert_eq!(map["action"].as_str(), Some("chord"));
    assert_eq!(map["hold_flips"].as_u64(), Some(2));
    assert_eq!(map["lead_flips"].as_u64(), Some(0));
    assert!(parse_args(["chord", "F2,W", "--lead-flips", "-1"]).is_err());

    let p = parse_args(["wait-settled", "90", "--stable-flips", "6"]).unwrap();
    let CliCommand::WaitRaw { json } = p.command else {
        panic!("expected WaitRaw");
    };
    assert!(json.contains("\"until_settled\"") && json.contains("\"max_flips\":90"));

    let p = parse_args([
        "launch",
        "lego-chess",
        "--window",
        "--bind",
        "0.0.0.0:7777",
        "--mode",
        "live",
    ])
    .unwrap();
    let CliCommand::Launch(opts) = p.command else {
        panic!("expected Launch");
    };
    assert_eq!(opts.title, "lego-chess");
    assert!(opts.windowed);
    assert_eq!(opts.bind.as_deref(), Some("0.0.0.0:7777"));
    assert_eq!(opts.mode.as_deref(), Some("live"));

    assert!(parse_args(["unknown-cmd"]).is_err());
    assert!(parse_args(["mem-search"]).is_err());
    assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    assert_eq!(url_encode("a/b c"), "a%2Fb%20c");
}

#[test]
fn http_client_decodes_chunked_and_handles_unix_socket() {
    let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;ext=1\r\nhello\r\n6\r\n world\r\n0\r\nX-Trailer: ok\r\n\r\n";
    let resp = read_http_response(Cursor::new(&raw[..])).expect("chunked");
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body, b"hello world");

    let ep = Endpoint::parse("http://127.0.0.1:7777").unwrap();
    assert_eq!(
        ep.watch_url().as_deref(),
        Some("http://127.0.0.1:7777/watch")
    );

    #[cfg(unix)]
    {
        use std::os::unix::net::UnixListener;
        let sock = std::env::temp_dir().join(format!(
            "lc-u-{}-{}.sock",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                % 100_000
        ));
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).expect("bind unix");
        let srv = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept unix");
            let _ = read_req(&mut stream);
            send_json(&mut stream, r#"{"ok":true,"via":"unix"}"#);
        });
        let url = format!("unix://{}", sock.display());
        let client = Client::new(&url, None).unwrap();
        assert_eq!(client.watch_url(), None);
        let r = client.get("/api/status").unwrap();
        assert_eq!(r.status, 200);
        assert!(r.body_text().contains("\"via\":\"unix\""));
        srv.join().unwrap();
        let _ = std::fs::remove_file(&sock);
    }
}

#[test]
fn sessions_discovery_prunes_dead_pids_and_lists_live_server() {
    let dir = unique_tmp_dir("sessions");
    let srv = MockServer::start(None);

    // Dead PID session file should be pruned.
    let dead_path = dir.join("99999999.json");
    std::fs::write(
        &dead_path,
        r#"{"pid":99999999,"title":"dead-game","control_url":"http://127.0.0.1:1","watch_url":null,"windowed":false,"started_at":10}"#,
    )
    .unwrap();

    // Live PID pointing to our running mock server.
    let live_pid = std::process::id();
    let live_path = dir.join(format!("{live_pid}.json"));
    std::fs::write(
        &live_path,
        format!(
            r#"{{"pid":{live_pid},"title":"lego-chess","control_url":"{}","watch_url":"{}/watch","windowed":true,"started_at":42}}"#,
            srv.url(),
            srv.url()
        ),
    )
    .unwrap();

    let found = list_sessions_in(&dir, true);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].pid, live_pid);
    assert_eq!(found[0].title, "lego-chess");
    assert!(found[0].windowed);
    assert!(!dead_path.exists(), "dead PID session file must be pruned");

    let parsed = parse_session_json(&found[0].to_json()).unwrap();
    assert_eq!(parsed, found[0]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lego_ctrl_end_to_end_covers_all_endpoints_auth_and_binary_outputs() {
    let dir = unique_tmp_dir("e2e");
    let srv = MockServer::start(Some("secret-312"));
    let url = srv.url();

    // Unauthorized POST without token fails cleanly.
    let parsed_unauth = parse_args(["--url", &url, "goal", "win"]).unwrap();
    let mut out = Vec::new();
    let err = execute_cli(&parsed_unauth, &mut Cursor::new(&[][..]), &mut out).unwrap_err();
    assert!(err.contains("401"), "expected 401, got {err}");

    let run_ok = |extra: &[&str], stdin_data: &[u8]| -> Vec<u8> {
        let mut args = vec!["--url", &url, "--token", "secret-312"];
        args.extend_from_slice(extra);
        let parsed = parse_args(args).expect("parse args");
        let mut buf = Vec::new();
        execute_cli(&parsed, &mut Cursor::new(stdin_data), &mut buf)
            .unwrap_or_else(|e| panic!("command {extra:?} failed: {e}"));
        buf
    };

    // Status, meta, controls, goals, fps (chunked), watch
    assert!(String::from_utf8(run_ok(&["status"], &[]))
        .unwrap()
        .contains("\"flip\":4"));
    assert!(String::from_utf8(run_ok(&["meta"], &[]))
        .unwrap()
        .contains("\"stub\""));
    assert!(String::from_utf8(run_ok(&["controls"], &[]))
        .unwrap()
        .contains("actions"));
    assert!(String::from_utf8(run_ok(&["goals"], &[]))
        .unwrap()
        .contains("goals"));
    assert!(
        String::from_utf8(run_ok(&["fps", "--window", "0..10"], &[]))
            .unwrap()
            .contains("throughput_mips")
    );
    assert_eq!(
        String::from_utf8(run_ok(&["watch"], &[])).unwrap().trim(),
        format!("{url}/watch")
    );

    // Screenshot to file & raw stdout
    let shot_path = dir.join("shot.png");
    let shot_meta = String::from_utf8(run_ok(
        &[
            "screenshot",
            "--grid",
            "--cursor",
            "--crop",
            "0,0,10,10",
            "-o",
            shot_path.to_str().unwrap(),
        ],
        &[],
    ))
    .unwrap();
    assert!(shot_meta.contains("\"flip\":4") && shot_meta.contains("deadbeef"));
    assert_eq!(std::fs::read(&shot_path).unwrap(), TINY_PNG);

    // Actions & pipe from stdin
    run_ok(&["click", "100", "200", "--poll"], &[]);
    run_ok(&["move", "10", "20"], &[]);
    run_ok(&["delta", "-5", "12"], &[]);
    run_ok(&["drag", "1", "2", "30", "40"], &[]);
    run_ok(&["key", "Space", "--hold-flips", "2"], &[]);
    run_ok(&["chord", "LeftControl,A"], &[]);
    run_ok(&["type", "HELLO"], &[]);
    run_ok(&["wait", "5"], &[]);
    run_ok(&["wait-change", "30"], &[]);
    run_ok(&["wait-settled", "60"], &[]);
    run_ok(&["act", r#"{"action":"click","x":1,"y":2}"#], &[]);
    run_ok(
        &["pipe", "-"],
        b"#lego-legacy-route 1\nflip 0 key Space down\n",
    );

    // Read-only introspection
    run_ok(&["mem", "0x00100000", "4", "--fmt", "u32"], &[]);
    run_ok(&["mem-map"], &[]);
    run_ok(&["mem-search", "--ascii", "CAFE"], &[]);
    run_ok(&["cpu"], &[]);
    run_ok(&["win32"], &[]);
    run_ok(&["files", "profile"], &[]);
    let file_out = dir.join("save.bin");
    run_ok(
        &[
            "read-file",
            "profile",
            "SAVE.DAT",
            "-o",
            file_out.to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(std::fs::read(&file_out).unwrap(), b"save-file-payload");

    // Checkpoints, route export, agent state, mode
    run_ok(&["bookmark", "b1"], &[]);
    run_ok(&["rewind", "b1"], &[]);
    run_ok(&["snapshot", "/tmp/snap.llsnap"], &[]);
    run_ok(&["restore", "/tmp/snap.llsnap"], &[]);
    let route_out = dir.join("out.route");
    run_ok(&["export-route", "-o", route_out.to_str().unwrap()], &[]);
    assert!(std::fs::read_to_string(&route_out)
        .unwrap()
        .starts_with("#lego-legacy-route 1"));
    run_ok(&["goal", "Beat level 1"], &[]);
    run_ok(&["thought", "Clicking start"], &[]);
    run_ok(&["hint", "Use right click"], &[]);
    run_ok(&["mode", "live", "1.5"], &[]);
    run_ok(&["mode", "close"], &[]);

    // Also spawn the compiled `lego-ctrl` binary directly!
    let output = Command::new(env!("CARGO_BIN_EXE_lego-ctrl"))
        .args(["--url", &url, "status"])
        .stdin(Stdio::null())
        .output()
        .expect("run lego-ctrl binary");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"title\":\"stub\""));

    // Verify pipe sent text/plain for `#lego-legacy-route 1`.
    let reqs = srv.requests();
    let pipe_req = reqs
        .iter()
        .find(|r| r.target.starts_with("/api/pipe"))
        .expect("pipe request recorded");
    assert!(pipe_req
        .headers
        .get("content-type")
        .is_some_and(|ct| ct.starts_with("text/plain")));
    assert!(pipe_req.body.starts_with(b"#lego-legacy-route 1"));

    let _ = std::fs::remove_dir_all(&dir);
}
