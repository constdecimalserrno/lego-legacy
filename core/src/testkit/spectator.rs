//! Reusable control-plane and live spectator test rig (`ControlRig`,
//! `HttpClient`, `SpectatorClient`, crosshair verification, and frame-diff
//! helpers, #410).
//!
//! Strictly title-agnostic (Golden Rule 4) and pure-`std` (Golden Rule 5):
//! shares one RAII server/subprocess lifecycle, HTTP/1.1 client, continuously
//! draining RFC 6455 WebSocket v2 + XOR-PackBits-RLE decoder, and `--cursor`
//! crosshair detector across every title's alignment and control tests.

use crate::cli::Cli;
use crate::control::json::{parse_json, write_json_f64, write_json_str, JsonValue};
use crate::control::observe::ObserveOptions;
use crate::control::ws::{
    decode_binary_frame, decode_xor_rle, websocket_accept_key, FRAME_FLAG_PNG_KEYFRAME,
    FRAME_FLAG_XOR_RLE_DELTA, OPCODE_BINARY, OPCODE_CLOSE, OPCODE_PING, OPCODE_PONG,
};
use crate::control::{
    service_flip_boundary, Action, Command, ControlBus, ControlConfig, ControlHost, ExecMode,
    HttpContext, HttpServer, InputOwner,
};
use crate::host::headless::Headless;
use crate::host::Button;
use crate::png::decode_to_bgra;
use crate::title::TitleGuide;
use crate::Title;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Default read/write timeout for synchronous HTTP control calls.
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(60);
/// Write timeout for WS control replies (`Pong`, `Close`) so `Drop` never stalls.
const WS_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Short socket poll slice for the background WS reader thread so `Drop` wakes promptly.
const WS_POLL_SLICE: Duration = Duration::from_millis(250);
/// Upper bound on a single server WS payload (64 MiB) to guard against corrupt length headers.
const MAX_WS_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const WS_CLIENT_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";

fn create_rig_scratch(prefix: &str) -> Result<PathBuf, String> {
    let dir = super::scratch_dir(prefix);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create rig scratch dir {}: {e}", dir.display()))?;
    Ok(dir)
}

enum RigBackend {
    InProcess {
        bus: ControlBus,
        runner: Option<JoinHandle<()>>,
        server: Option<HttpServer>,
        data_root: Option<PathBuf>,
        scratch: Option<PathBuf>,
    },
    Spawned {
        child: Child,
        stderr_drainer: Option<JoinHandle<()>>,
        data_root: PathBuf,
    },
}

/// RAII harness running a title under the HTTP/WebSocket control plane, either
/// in-process (`ControlHost<Headless>` + loopback `HttpServer`) or as a spawned
/// child binary (`--headless --control 127.0.0.1:0 --paused`).
///
/// Dropping a `ControlRig` always stops the guest, joins background threads,
/// releases the listening port, and removes rig-owned scratch directories—even
/// while unwinding from a test panic.
pub struct ControlRig {
    addr: SocketAddr,
    backend: RigBackend,
}

impl ControlRig {
    /// Launch a title in-process on a background thread under
    /// [`ControlHost<Headless>`] with a loopback [`HttpServer`] bound to
    /// `127.0.0.1:0`.
    ///
    /// If `cli.data_root` is `None`, a fresh scratch directory under
    /// [`crate::runlog::runs_root`] is created and removed automatically on
    /// [`Drop`].
    ///
    /// # Errors
    /// Returns an error if the scratch directory or loopback HTTP server cannot
    /// be initialized.
    pub fn in_process(
        title: &'static Title,
        guide: &'static TitleGuide,
        mut cli: Cli,
        budget: u64,
    ) -> Result<Self, String> {
        let owned_scratch = if cli.data_root.is_none() {
            let dir = create_rig_scratch("spectator-rig")?;
            cli.data_root = Some(dir.join("data-root"));
            Some(dir)
        } else {
            None
        };
        let data_root = cli.data_root.clone();

        let config = ControlConfig {
            title: title.key,
            mode: cli.control_mode.unwrap_or(ExecMode::Step),
            owner: InputOwner::Agent,
            start_paused: cli.paused,
            step_uncapped: true,
            pace: 1.0,
            pointer_speed: title.pointer_speed,
            flip_cost: title.flip_cost,
            guide,
        };

        cli.headless = true;
        cli.record = true;
        cli.control_http = None;
        cli.control_unix = None;
        cli.control_stdio = false;

        Self::in_process_with_runner(
            title,
            guide,
            config,
            data_root,
            owned_scratch,
            move |control| {
                let mut control = control.with_deferred_pause(true);
                let obs_bus = control.bus();
                obs_bus.set_machine_attached(true);
                let (mut machine, mut log) = match crate::run::setup(title, &cli, &mut control) {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("ControlRig::in_process setup failed for {}: {e}", title.key);
                        return;
                    }
                };
                machine.set_flip_observer(Box::new(move |m, h| {
                    service_flip_boundary(&obs_bus, m, h);
                }));
                let finish = machine.run(&mut control, &mut log, budget);
                let digest =
                    crate::run::wrap_up(title, &mut machine, &mut control, &mut log, &finish);
                control.mark_finished(&finish, &digest);
            },
        )
    }

    fn in_process_with_runner<F>(
        title: &'static Title,
        guide: &'static TitleGuide,
        config: ControlConfig,
        data_root: Option<PathBuf>,
        scratch: Option<PathBuf>,
        runner: F,
    ) -> Result<Self, String>
    where
        F: FnOnce(ControlHost<Headless>) + Send + 'static,
    {
        let control = ControlHost::new(Headless::new(), config);
        let bus = control.bus();
        let ctx = Arc::new(HttpContext::new(bus.clone(), title, None).with_guide(guide));
        let server = HttpServer::start("127.0.0.1:0", ctx).map_err(|e| {
            if let Some(dir) = &scratch {
                let _ = std::fs::remove_dir_all(dir);
            }
            format!("start loopback HttpServer: {e}")
        })?;
        let addr = server.addr();
        let handle = thread::spawn(move || runner(control));

        Ok(Self {
            addr,
            backend: RigBackend::InProcess {
                bus,
                runner: Some(handle),
                server: Some(server),
                data_root,
                scratch,
            },
        })
    }

    /// Spawn a title binary with `--headless --control 127.0.0.1:0 --paused
    /// --data-root <data_root>` plus any extra `args`, wait for it to print its
    /// loopback control URL on stderr, and attach a background stderr drainer.
    ///
    /// Dropping the returned `ControlRig` kills the child process, joins the
    /// stderr drainer, and removes `data_root`.
    ///
    /// # Errors
    /// Returns an error if `data_root` cannot be created, the binary fails to
    /// spawn, or the child exits before printing a valid `http://<host:port>`
    /// control URL on stderr.
    pub fn spawn_binary(bin: &Path, data_root: &Path, args: &[&str]) -> Result<Self, String> {
        let _ = std::fs::remove_dir_all(data_root);
        std::fs::create_dir_all(data_root)
            .map_err(|e| format!("create data_root {}: {e}", data_root.display()))?;
        let runs_dir = data_root.join("runs");

        let mut child = ProcessCommand::new(bin)
            .args(["--headless", "--control", "127.0.0.1:0", "--paused"])
            .arg("--data-root")
            .arg(data_root)
            .args(args)
            .env("LEGO_LEGACY_RUNS", &runs_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                let _ = std::fs::remove_dir_all(data_root);
                format!("spawn {}: {e}", bin.display())
            })?;

        let stderr = child.stderr.take().ok_or_else(|| {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(data_root);
            "child stderr was not piped".to_owned()
        })?;

        let mut rig = Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            backend: RigBackend::Spawned {
                child,
                stderr_drainer: None,
                data_root: data_root.to_path_buf(),
            },
        };

        let mut err_reader = BufReader::new(stderr);
        let mut seen_stderr = String::new();
        let mut raw_line = Vec::new();
        rig.addr = loop {
            raw_line.clear();
            let n = err_reader
                .read_until(b'\n', &mut raw_line)
                .map_err(|e| format!("read child stderr: {e}"))?;
            if n == 0 {
                return Err(format!(
                    "child {} exited before printing control URL; stderr:\n{seen_stderr}",
                    bin.display()
                ));
            }
            let line = String::from_utf8_lossy(&raw_line);
            if seen_stderr.len() < 4096 {
                seen_stderr.push_str(&line);
            }
            if let Some((_, rest)) = line.split_once("http://") {
                let host_port = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('/');
                if let Ok(parsed) = host_port.parse::<SocketAddr>() {
                    break parsed;
                }
            }
        };

        let drainer = thread::spawn(move || {
            let _ = io::copy(&mut err_reader, &mut io::sink());
        });
        if let RigBackend::Spawned { stderr_drainer, .. } = &mut rig.backend {
            *stderr_drainer = Some(drainer);
        }
        Ok(rig)
    }

    /// Loopback TCP address of the running control plane HTTP/WebSocket server.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Direct handle to the in-process [`ControlBus`], or `None` for a spawned
    /// child binary.
    #[must_use]
    pub fn bus(&self) -> Option<&ControlBus> {
        match &self.backend {
            RigBackend::InProcess { bus, .. } => Some(bus),
            RigBackend::Spawned { .. } => None,
        }
    }

    /// Profile data root configured for this rig, if any.
    #[must_use]
    pub fn data_root(&self) -> Option<&Path> {
        match &self.backend {
            RigBackend::InProcess { data_root, .. } => data_root.as_deref(),
            RigBackend::Spawned { data_root, .. } => Some(data_root.as_path()),
        }
    }

    /// Non-WebSocket HTTP/1.1 client targeting this rig's loopback server.
    #[must_use]
    pub const fn http(&self) -> HttpClient {
        HttpClient::new(self.addr)
    }

    /// Open a live [`SpectatorClient`] connected to this rig's loopback server.
    ///
    /// # Errors
    /// Returns an error if the RFC 6455 WebSocket handshake fails.
    pub fn connect(&self) -> Result<SpectatorClient, String> {
        SpectatorClient::connect(self.addr)
    }
}

impl Drop for ControlRig {
    fn drop(&mut self) {
        match &mut self.backend {
            RigBackend::InProcess {
                bus,
                runner,
                server,
                scratch,
                ..
            } => {
                let _ = bus.dispatch(Command::Close { force: true });
                if let Some(handle) = runner.take() {
                    let _ = handle.join();
                }
                if let Some(mut srv) = server.take() {
                    srv.stop();
                }
                if let Some(dir) = scratch.take() {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
            RigBackend::Spawned {
                child,
                stderr_drainer,
                data_root,
            } => {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(handle) = stderr_drainer.take() {
                    let _ = handle.join();
                }
                let _ = std::fs::remove_dir_all(data_root);
            }
        }
    }
}

/// Pure-`std` HTTP/1.1 client for the control plane, usable independently of
/// opening a `/api/stream` WebSocket connection.
#[derive(Clone, Copy, Debug)]
pub struct HttpClient {
    addr: SocketAddr,
    timeout: Duration,
}

impl HttpClient {
    /// Create an HTTP client targeting `addr` with the default 60 s timeout.
    #[must_use]
    pub const fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            timeout: DEFAULT_HTTP_TIMEOUT,
        }
    }

    /// Override the HTTP read/write timeout.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Issue a raw HTTP/1.1 request and return `(status_code, body_bytes)`.
    ///
    /// # Errors
    /// Returns an error on connection, I/O, malformed HTTP status line, or
    /// truncated response body (when `Content-Length` is present).
    pub fn http_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<(u16, Vec<u8>), String> {
        let mut stream = TcpStream::connect(self.addr)
            .map_err(|e| format!("HTTP connect {}: {e}", self.addr))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|e| format!("set HTTP read timeout: {e}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| format!("set HTTP write timeout: {e}"))?;

        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n"
        )
        .map_err(|e| format!("write HTTP {method} {path}: {e}"))?;
        if let Some(payload) = body {
            write!(
                stream,
                "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
                payload.len()
            )
            .map_err(|e| format!("write HTTP body {method} {path}: {e}"))?;
        } else {
            stream
                .write_all(b"\r\n")
                .map_err(|e| format!("write HTTP end {method} {path}: {e}"))?;
        }
        stream
            .flush()
            .map_err(|e| format!("flush HTTP {method} {path}: {e}"))?;

        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|e| format!("read HTTP {method} {path}: {e}"))?;
        let sep = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| format!("missing HTTP header separator for {method} {path}"))?;
        let head = std::str::from_utf8(&raw[..sep])
            .map_err(|e| format!("non-UTF-8 HTTP headers for {method} {path}: {e}"))?;
        let status_line = head.lines().next().unwrap_or("");
        if !status_line.starts_with("HTTP/1.") {
            return Err(format!(
                "malformed HTTP status line for {method} {path}: {status_line}"
            ));
        }
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| format!("malformed HTTP status code for {method} {path}: {head}"))?;

        let resp_body = raw[sep + 4..].to_vec();
        if method != "HEAD" && status != 204 && status != 304 {
            for line in head.lines().skip(1) {
                if let Some((name, val)) = line.split_once(':') {
                    if name.trim().eq_ignore_ascii_case("content-length") {
                        let expected: usize = val.trim().parse().map_err(|_| {
                            format!("invalid Content-Length `{val}` for {method} {path}")
                        })?;
                        if resp_body.len() != expected {
                            return Err(format!(
                                "truncated HTTP body for {method} {path}: got {} bytes, expected {expected}",
                                resp_body.len()
                            ));
                        }
                    }
                }
            }
        }
        Ok((status, resp_body))
    }

    fn request_json(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<JsonValue, String> {
        let (code, resp) = self.http_request(method, path, body)?;
        let text = String::from_utf8(resp).map_err(|e| format!("{method} {path} UTF-8: {e}"))?;
        if code != 200 {
            return Err(format!("{method} {path} returned HTTP {code}: {text}"));
        }
        parse_json(&text).map_err(|e| format!("{method} {path} invalid JSON: {e}\n{text}"))
    }

    /// Issue `GET <path>`, require HTTP 200, and parse JSON.
    ///
    /// # Errors
    /// Returns an error on HTTP error status, non-UTF-8 body, or invalid JSON.
    pub fn get_json(&self, path: &str) -> Result<JsonValue, String> {
        self.request_json("GET", path, None)
    }

    /// Issue `POST <path>` with a JSON body, require HTTP 200, and parse JSON.
    ///
    /// # Errors
    /// Returns an error on HTTP error status, non-UTF-8 body, or invalid JSON.
    pub fn post_json(&self, path: &str, body: &str) -> Result<JsonValue, String> {
        self.request_json("POST", path, Some(body))
    }

    /// Advance the guest by `flips` via `POST /api/step` and assert `ok == true`.
    ///
    /// # Errors
    /// Returns an error if the request fails or `ok` is not `true`.
    pub fn step(&self, flips: u64) -> Result<JsonValue, String> {
        let val = self.post_json("/api/step", &format!("{{\"flips\":{flips}}}"))?;
        require_ok(&val, "POST /api/step")?;
        Ok(val)
    }

    /// Execute a typed [`Action`] synchronously via `POST /api/act` with
    /// `"wait":true,"auto_step":true`.
    ///
    /// # Errors
    /// Returns an error if the request fails or `ok` is not `true`.
    pub fn act(&self, action: &Action) -> Result<JsonValue, String> {
        self.act_json(&serialize_action_json(action))
    }

    /// Execute a raw JSON body via `POST /api/act` and assert `ok == true`.
    ///
    /// # Errors
    /// Returns an error if the request fails or `ok` is not `true`.
    pub fn act_json(&self, body: &str) -> Result<JsonValue, String> {
        let val = self.post_json("/api/act", body)?;
        require_ok(&val, "POST /api/act")?;
        Ok(val)
    }

    /// Fetch `GET /api/status`.
    ///
    /// # Errors
    /// Returns an error if the HTTP request or JSON parsing fails.
    pub fn status(&self) -> Result<JsonValue, String> {
        self.get_json("/api/status")
    }

    /// Fetch `GET /api/meta`.
    ///
    /// # Errors
    /// Returns an error if the HTTP request or JSON parsing fails.
    pub fn meta(&self) -> Result<JsonValue, String> {
        self.get_json("/api/meta")
    }

    /// Pipe `#lego-legacy-route 1` text via `POST /api/pipe` and assert `ok == true`.
    ///
    /// # Errors
    /// Returns an error if the request fails or `ok` is not `true`.
    pub fn pipe(&self, route: &str, relative: bool, auto_step: bool) -> Result<JsonValue, String> {
        let mut body = String::with_capacity(route.len() + 64);
        body.push_str("{\"route\":");
        write_json_str(&mut body, route);
        let _ = write!(body, ",\"relative\":{relative},\"auto_step\":{auto_step}}}");
        let val = self.post_json("/api/pipe", &body)?;
        require_ok(&val, "POST /api/pipe")?;
        Ok(val)
    }

    /// Fetch `GET /api/frame.png` with optional grid, cursor crosshair, and crop
    /// parameters from `opts`.
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or does not return HTTP 200.
    pub fn frame_png(&self, opts: ObserveOptions) -> Result<Vec<u8>, String> {
        let mut path = String::from("/api/frame.png");
        let mut sep = '?';
        if opts.grid {
            let _ = write!(path, "{sep}grid=1");
            sep = '&';
        }
        if opts.cursor {
            let _ = write!(path, "{sep}cursor=1");
            sep = '&';
        }
        if let Some((x, y, w, h)) = opts.crop {
            let _ = write!(path, "{sep}crop={x},{y},{w},{h}");
        }
        let (code, body) = self.http_request("GET", &path, None)?;
        if code != 200 {
            return Err(format!(
                "GET {path} returned HTTP {code}: {}",
                String::from_utf8_lossy(&body)
            ));
        }
        Ok(body)
    }
}

/// One decoded frame delivered over `/api/stream` WebSocket v2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsFrame {
    pub kind: u8,
    pub flip: u64,
    pub width: u32,
    pub height: u32,
    pub pointer_pos: (i32, i32),
    pub pointer_shown: bool,
    pub pointer_blank: bool,
    pub bgra: Vec<u8>,
}

#[derive(Default)]
struct WsShared {
    latest: Option<WsFrame>,
    keyframes: u64,
    deltas: u64,
    closed: bool,
    error: Option<String>,
}

/// Live HTTP/1.1 + continuously draining RFC 6455 WebSocket v2 client for a
/// running control server.
///
/// The background reader thread drains PNG keyframes and XOR+PackBits-RLE delta
/// frames continuously and echoes RFC 6455 `Ping` frames with masked `Pong`s so
/// long headless stepping bursts never stall the server's `STREAM_WRITE_TIMEOUT`.
pub struct SpectatorClient {
    http: HttpClient,
    shared: Arc<(Mutex<WsShared>, Condvar)>,
    stopping: Arc<AtomicBool>,
    writer: Arc<Mutex<Option<TcpStream>>>,
    reader_thread: Option<JoinHandle<()>>,
}

fn write_masked_ws_frame(sock: &mut TcpStream, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let len = payload.len().min(125);
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let mut buf = Vec::with_capacity(6 + len);
    buf.push(0x80 | (opcode & 0x0f));
    buf.push(0x80 | (len as u8));
    buf.extend_from_slice(&mask);
    for (i, &b) in payload[..len].iter().enumerate() {
        buf.push(b ^ mask[i & 3]);
    }
    sock.write_all(&buf)?;
    sock.flush()
}

fn read_exact_draining(
    reader: &mut BufReader<TcpStream>,
    mut buf: &mut [u8],
    stopping: &AtomicBool,
) -> io::Result<()> {
    while !buf.is_empty() {
        match reader.read(buf) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "websocket peer closed",
                ));
            }
            Ok(n) => buf = &mut buf[n..],
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                if stopping.load(Ordering::Relaxed) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "client stopping",
                    ));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn read_server_ws_frame(
    reader: &mut BufReader<TcpStream>,
    stopping: &AtomicBool,
) -> io::Result<(u8, Vec<u8>)> {
    let mut hdr = [0u8; 2];
    read_exact_draining(reader, &mut hdr, stopping)?;
    if (hdr[0] & 0x80) == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported fragmented server WS frame (FIN=0)",
        ));
    }
    if (hdr[0] & 0x70) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server sent non-zero WS RSV bits",
        ));
    }
    let opcode = hdr[0] & 0x0f;
    if (hdr[1] & 0x80) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server sent masked WS frame (RFC 6455 violation)",
        ));
    }
    let len7 = hdr[1] & 0x7f;
    let len: usize = match len7 {
        0..=125 => usize::from(len7),
        126 => {
            let mut ext = [0u8; 2];
            read_exact_draining(reader, &mut ext, stopping)?;
            usize::from(u16::from_be_bytes(ext))
        }
        _ => {
            let mut ext = [0u8; 8];
            read_exact_draining(reader, &mut ext, stopping)?;
            let raw_len = u64::from_be_bytes(ext);
            if raw_len >= (1u64 << 63) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WS 64-bit length MSB must be zero",
                ));
            }
            usize::try_from(raw_len)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "WS length overflow"))?
        }
    };
    if opcode >= 0x8 && len > 125 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WS control frame payload exceeds 125 bytes",
        ));
    }
    if len > MAX_WS_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("WS frame payload {len} exceeds {MAX_WS_PAYLOAD_BYTES} byte limit"),
        ));
    }
    let mut payload = vec![0u8; len];
    if len > 0 {
        read_exact_draining(reader, &mut payload, stopping)?;
    }
    Ok((opcode, payload))
}

fn run_ws_reader(
    reader: &mut BufReader<TcpStream>,
    shared: &(Mutex<WsShared>, Condvar),
    writer: &Mutex<Option<TcpStream>>,
    stopping: &AtomicBool,
) -> Result<(), String> {
    let mut prev: Option<(u32, u32, Vec<u8>)> = None;
    loop {
        let (opcode, payload) = match read_server_ws_frame(reader, stopping) {
            Ok(frame) => frame,
            Err(e)
                if stopping.load(Ordering::Relaxed) || e.kind() == io::ErrorKind::UnexpectedEof =>
            {
                return Ok(());
            }
            Err(e) => return Err(format!("WS read failed: {e}")),
        };

        match opcode {
            OPCODE_BINARY => {
                let hdr = decode_binary_frame(&payload)
                    .ok_or_else(|| "invalid WS v2 binary frame header".to_owned())?;
                let (pointer_pos, pointer_shown) = hdr
                    .pointer
                    .ok_or_else(|| "WS binary frame missing v2 pointer".to_owned())?;
                let width = u32::from(hdr.width);
                let height = u32::from(hdr.height);
                let (bgra, is_keyframe) = match hdr.kind {
                    FRAME_FLAG_PNG_KEYFRAME => {
                        let (pw, ph, buf) = decode_to_bgra(hdr.payload)?;
                        if (pw, ph) != (width, height) {
                            return Err(format!(
                                "WS keyframe PNG dims {pw}x{ph} != header {width}x{height}"
                            ));
                        }
                        (buf, true)
                    }
                    FRAME_FLAG_XOR_RLE_DELTA => {
                        let (pw, ph, base) = prev
                            .as_ref()
                            .ok_or_else(|| "WS delta arrived before keyframe".to_owned())?;
                        if (*pw, *ph) != (width, height) {
                            return Err(format!(
                                "WS delta dims {width}x{height} != previous {pw}x{ph}"
                            ));
                        }
                        let buf = decode_xor_rle(base, hdr.payload)
                            .map_err(|e| format!("WS XOR-RLE decode failed: {e}"))?;
                        (buf, false)
                    }
                    other => return Err(format!("unexpected WS binary frame kind {other}")),
                };
                prev = Some((width, height, bgra.clone()));
                let (lock, cv) = shared;
                let Ok(mut st) = lock.lock() else {
                    return Ok(());
                };
                if is_keyframe {
                    st.keyframes += 1;
                } else {
                    st.deltas += 1;
                }
                st.latest = Some(WsFrame {
                    kind: hdr.kind,
                    flip: u64::from(hdr.flip),
                    width,
                    height,
                    pointer_pos,
                    pointer_shown,
                    pointer_blank: hdr.pointer_blank,
                    bgra,
                });
                cv.notify_all();
            }
            OPCODE_PING => {
                if let Ok(mut guard) = writer.lock() {
                    if let Some(sock) = guard.as_mut() {
                        let _ = write_masked_ws_frame(sock, OPCODE_PONG, &payload);
                    }
                }
            }
            OPCODE_PONG => {}
            OPCODE_CLOSE => return Ok(()),
            other => return Err(format!("unexpected WS opcode {other:#x}")),
        }
    }
}

impl SpectatorClient {
    /// Connect to `/api/stream?fps=60` on `addr` and spawn the draining
    /// WebSocket v2 reader thread.
    ///
    /// # Errors
    /// Returns an error if the TCP connection or RFC 6455 WebSocket upgrade
    /// handshake fails.
    pub fn connect(addr: SocketAddr) -> Result<Self, String> {
        let mut sock = TcpStream::connect(addr).map_err(|e| format!("WS connect {addr}: {e}"))?;
        sock.set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| format!("set WS handshake read timeout: {e}"))?;
        sock.set_write_timeout(Some(WS_WRITE_TIMEOUT))
            .map_err(|e| format!("set WS write timeout: {e}"))?;

        write!(
            sock,
            "GET /api/stream?fps=60 HTTP/1.1\r\n\
             Host: localhost\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {WS_CLIENT_KEY}\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        )
        .map_err(|e| format!("write WS upgrade: {e}"))?;
        sock.flush().map_err(|e| format!("flush WS upgrade: {e}"))?;

        let mut reader = BufReader::new(sock);
        let mut status_line = String::new();
        reader
            .read_line(&mut status_line)
            .map_err(|e| format!("read WS status line: {e}"))?;
        if !status_line.starts_with("HTTP/1.1 ")
            || status_line.split_whitespace().nth(1) != Some("101")
        {
            return Err(format!("expected HTTP 101 upgrade, got {status_line:?}"));
        }

        let mut got_accept = None;
        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| format!("read WS header: {e}"))?;
            if n == 0 {
                return Err("truncated WS handshake headers before blank line".to_owned());
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some((name, val)) = line.split_once(':') {
                if name.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                    got_accept = Some(val.trim().to_owned());
                }
            }
        }
        let expected_accept = websocket_accept_key(WS_CLIENT_KEY);
        if got_accept.as_deref() != Some(expected_accept.as_str()) {
            return Err(format!(
                "Sec-WebSocket-Accept mismatch: expected {expected_accept:?}, got {got_accept:?}"
            ));
        }

        reader
            .get_ref()
            .set_read_timeout(Some(WS_POLL_SLICE))
            .map_err(|e| format!("set WS poll timeout: {e}"))?;
        let writer_sock = reader
            .get_ref()
            .try_clone()
            .map_err(|e| format!("clone WS socket: {e}"))?;
        let writer = Arc::new(Mutex::new(Some(writer_sock)));
        let writer_bg = Arc::clone(&writer);
        let shared = Arc::new((Mutex::new(WsShared::default()), Condvar::new()));
        let shared_bg = Arc::clone(&shared);
        let stopping = Arc::new(AtomicBool::new(false));
        let stopping_bg = Arc::clone(&stopping);

        let reader_thread = thread::spawn(move || {
            let result = run_ws_reader(&mut reader, &shared_bg, &writer_bg, &stopping_bg);
            let (lock, cv) = &*shared_bg;
            if let Ok(mut st) = lock.lock() {
                if let Err(err) = result {
                    st.error = Some(err);
                }
                st.closed = true;
                cv.notify_all();
            }
        });

        Ok(Self {
            http: HttpClient::new(addr),
            shared,
            stopping,
            writer,
            reader_thread: Some(reader_thread),
        })
    }

    /// Override the HTTP request timeout (default 60 s).
    #[must_use]
    pub const fn with_http_timeout(mut self, timeout: Duration) -> Self {
        self.http = self.http.with_timeout(timeout);
        self
    }

    /// Issue a raw HTTP/1.1 request and return `(status_code, body_bytes)`.
    ///
    /// # Errors
    /// As [`HttpClient::http_request`].
    pub fn http_request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<(u16, Vec<u8>), String> {
        self.http.http_request(method, path, body)
    }

    /// Issue `GET <path>`, require HTTP 200, and parse JSON.
    ///
    /// # Errors
    /// As [`HttpClient::get_json`].
    pub fn get_json(&self, path: &str) -> Result<JsonValue, String> {
        self.http.get_json(path)
    }

    /// Issue `POST <path>` with a JSON body, require HTTP 200, and parse JSON.
    ///
    /// # Errors
    /// As [`HttpClient::post_json`].
    pub fn post_json(&self, path: &str, body: &str) -> Result<JsonValue, String> {
        self.http.post_json(path, body)
    }

    /// Advance the guest by `flips` via `POST /api/step` and assert `ok == true`.
    ///
    /// # Errors
    /// As [`HttpClient::step`].
    pub fn step(&self, flips: u64) -> Result<JsonValue, String> {
        self.http.step(flips)
    }

    /// Execute a typed [`Action`] synchronously via `POST /api/act`.
    ///
    /// # Errors
    /// As [`HttpClient::act`].
    pub fn act(&self, action: &Action) -> Result<JsonValue, String> {
        self.http.act(action)
    }

    /// Execute a raw JSON body via `POST /api/act` and assert `ok == true`.
    ///
    /// # Errors
    /// As [`HttpClient::act_json`].
    pub fn act_json(&self, body: &str) -> Result<JsonValue, String> {
        self.http.act_json(body)
    }

    /// Fetch `GET /api/status`.
    ///
    /// # Errors
    /// As [`HttpClient::status`].
    pub fn status(&self) -> Result<JsonValue, String> {
        self.http.status()
    }

    /// Fetch `GET /api/meta`.
    ///
    /// # Errors
    /// As [`HttpClient::meta`].
    pub fn meta(&self) -> Result<JsonValue, String> {
        self.http.meta()
    }

    /// Pipe `#lego-legacy-route 1` text via `POST /api/pipe`.
    ///
    /// # Errors
    /// As [`HttpClient::pipe`].
    pub fn pipe(&self, route: &str, relative: bool, auto_step: bool) -> Result<JsonValue, String> {
        self.http.pipe(route, relative, auto_step)
    }

    /// Fetch `GET /api/frame.png` with optional overlays from `opts`.
    ///
    /// # Errors
    /// As [`HttpClient::frame_png`].
    pub fn frame_png(&self, opts: ObserveOptions) -> Result<Vec<u8>, String> {
        self.http.frame_png(opts)
    }

    /// Wait until at least one decoded WebSocket v2 frame has arrived.
    ///
    /// # Errors
    /// Returns an error if the WebSocket closes, hits a protocol error, or times
    /// out before receiving a frame.
    pub fn wait_for_any_frame(&self, timeout: Duration) -> Result<WsFrame, String> {
        self.wait_for_predicate(timeout, "initial WS frame", |_| true)
    }

    /// Wait until the reconstructed WebSocket v2 stream matches `expected_bgra`
    /// (on all RGB channels), `expected_ptr`, and `expected_shown` at a flip in
    /// `min_flip..=max_flip` (accounting for server deduplication of identical
    /// consecutive frames).
    ///
    /// # Errors
    /// Returns an error if the WebSocket fails, closes early, or times out.
    pub fn wait_for_matching_frame(
        &self,
        expected_bgra: &[u8],
        expected_ptr: (i32, i32),
        expected_shown: bool,
        min_flip: u64,
        max_flip: u64,
        timeout: Duration,
    ) -> Result<WsFrame, String> {
        let desc = format!(
            "WS frame in {min_flip}..={max_flip} ptr={expected_ptr:?} shown={expected_shown}"
        );
        self.wait_for_predicate(timeout, &desc, |f| {
            f.flip >= min_flip
                && f.flip <= max_flip
                && f.pointer_pos == expected_ptr
                && f.pointer_shown == expected_shown
                && bgra_rgb_eq(&f.bgra, expected_bgra)
        })
    }

    /// Wait until the reconstructed WebSocket v2 stream delivers a frame at or
    /// after `min_flip` whose RGB pixels match `expected_bgra` without
    /// filtering on pointer metadata (so callers can assert frame-locked
    /// pointer fields).
    ///
    /// # Errors
    /// Returns an error if the WebSocket fails, closes early, or times out.
    pub fn wait_for_pixels(
        &self,
        min_flip: u64,
        expected_bgra: &[u8],
        timeout: Duration,
    ) -> Result<WsFrame, String> {
        let desc = format!("WS pixels at flip >= {min_flip}");
        self.wait_for_predicate(timeout, &desc, |f| {
            f.flip >= min_flip && bgra_rgb_eq(&f.bgra, expected_bgra)
        })
    }

    fn wait_for_predicate(
        &self,
        timeout: Duration,
        desc: &str,
        pred: impl Fn(&WsFrame) -> bool,
    ) -> Result<WsFrame, String> {
        let deadline = Instant::now() + timeout;
        let (lock, cv) = &*self.shared;
        let mut st = lock
            .lock()
            .map_err(|_| "WS state mutex poisoned".to_owned())?;
        loop {
            if let Some(err) = &st.error {
                return Err(format!("WS reader error while waiting for {desc}: {err}"));
            }
            if let Some(frame) = &st.latest {
                if pred(frame) {
                    return Ok(frame.clone());
                }
            }
            if st.closed {
                return Err(format!(
                    "WS closed before {desc} (latest={:?})",
                    st.latest
                        .as_ref()
                        .map(|f| (f.flip, f.pointer_pos, f.pointer_shown))
                ));
            }
            let rem = deadline.saturating_duration_since(Instant::now());
            if rem.is_zero() {
                return Err(format!(
                    "timed out waiting for {desc} (latest={:?})",
                    st.latest
                        .as_ref()
                        .map(|f| (f.flip, f.pointer_pos, f.pointer_shown))
                ));
            }
            let (guard, _) = cv
                .wait_timeout(st, rem)
                .map_err(|_| "WS condvar wait failed".to_owned())?;
            st = guard;
        }
    }

    /// Number of `(keyframes, deltas)` decoded over the WebSocket stream so far.
    #[must_use]
    pub fn counts(&self) -> (u64, u64) {
        self.shared
            .0
            .lock()
            .map(|st| (st.keyframes, st.deltas))
            .unwrap_or((0, 0))
    }

    /// Whether the WebSocket stream has closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shared.0.lock().map_or(true, |st| st.closed)
    }
}

impl Drop for SpectatorClient {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(mut sock) = guard.take() {
                let _ = write_masked_ws_frame(&mut sock, OPCODE_CLOSE, &[]);
                let _ = sock.shutdown(Shutdown::Both);
            }
        }
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

fn require_ok(val: &JsonValue, label: &str) -> Result<(), String> {
    let ok = val
        .as_object()
        .and_then(|o| o.get("ok"))
        .and_then(JsonValue::as_bool);
    if ok == Some(true) {
        Ok(())
    } else {
        Err(format!("{label} returned error: {}", val.to_json()))
    }
}

fn button_str(button: Button) -> &'static str {
    match button {
        Button::Left => "left",
        Button::Right => "right",
    }
}

fn write_opt_xy(out: &mut String, x: Option<i32>, y: Option<i32>) {
    if let Some(x) = x {
        let _ = write!(out, ",\"x\":{x}");
    }
    if let Some(y) = y {
        let _ = write!(out, ",\"y\":{y}");
    }
}

fn serialize_action_json(action: &Action) -> String {
    let mut out = String::with_capacity(128);
    out.push('{');
    match action {
        Action::Move { x, y, glide_flips } => {
            let _ = write!(
                out,
                "\"action\":\"move\",\"x\":{x},\"y\":{y},\"glide_flips\":{glide_flips}"
            );
        }
        Action::MouseDelta {
            dx,
            dy,
            flips,
            scale_x,
            scale_y,
        } => {
            out.push_str("\"action\":\"mouse_delta\",\"dx\":");
            write_json_f64(&mut out, *dx);
            out.push_str(",\"dy\":");
            write_json_f64(&mut out, *dy);
            let _ = write!(out, ",\"flips\":{flips}");
            if let Some(sx) = scale_x {
                out.push_str(",\"scale_x\":");
                write_json_f64(&mut out, *sx);
            }
            if let Some(sy) = scale_y {
                out.push_str(",\"scale_y\":");
                write_json_f64(&mut out, *sy);
            }
        }
        Action::MouseDown { x, y, button } => {
            let _ = write!(
                out,
                "\"action\":\"mouse_down\",\"button\":\"{}\"",
                button_str(*button)
            );
            write_opt_xy(&mut out, *x, *y);
        }
        Action::MouseUp { x, y, button } => {
            let _ = write!(
                out,
                "\"action\":\"mouse_up\",\"button\":\"{}\"",
                button_str(*button)
            );
            write_opt_xy(&mut out, *x, *y);
        }
        Action::Click {
            x,
            y,
            button,
            hold_flips,
            hold_ms,
            settle_flips,
        } => {
            let _ = write!(
                out,
                "\"action\":\"click\",\"button\":\"{}\",\"hold_flips\":{hold_flips},\"settle_flips\":{settle_flips}",
                button_str(*button)
            );
            write_opt_xy(&mut out, *x, *y);
            if let Some(ms) = hold_ms {
                let _ = write!(out, ",\"hold_ms\":{ms}");
            }
        }
        Action::ClickPoll {
            x,
            y,
            button,
            hold_flips,
            hold_ms,
        } => {
            let _ = write!(
                out,
                "\"action\":\"click_poll\",\"button\":\"{}\",\"hold_flips\":{hold_flips},\"hold_ms\":{hold_ms}",
                button_str(*button)
            );
            write_opt_xy(&mut out, *x, *y);
        }
        Action::Drag {
            from_x,
            from_y,
            to_x,
            to_y,
            button,
            step_px,
        } => {
            let _ = write!(
                out,
                "\"action\":\"drag\",\"from_x\":{from_x},\"from_y\":{from_y},\"to_x\":{to_x},\"to_y\":{to_y},\"button\":\"{}\",\"step_px\":{step_px}",
                button_str(*button)
            );
        }
        Action::KeyHold {
            key,
            hold_flips,
            hold_ms,
        } => {
            out.push_str("\"action\":\"key_hold\",\"key\":");
            write_json_str(&mut out, key);
            let _ = write!(out, ",\"hold_flips\":{hold_flips}");
            if let Some(ms) = hold_ms {
                let _ = write!(out, ",\"hold_ms\":{ms}");
            }
        }
        Action::Chord {
            keys,
            hold_flips,
            hold_ms,
            lead_flips,
        } => {
            out.push_str("\"action\":\"chord\",\"keys\":[");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_str(&mut out, k);
            }
            let _ = write!(
                out,
                "],\"hold_flips\":{hold_flips},\"lead_flips\":{lead_flips}"
            );
            if let Some(ms) = hold_ms {
                let _ = write!(out, ",\"hold_ms\":{ms}");
            }
        }
        Action::TypeText { text, flip_gap } => {
            out.push_str("\"action\":\"type_text\",\"text\":");
            write_json_str(&mut out, text);
            let _ = write!(out, ",\"flip_gap\":{flip_gap}");
        }
    }
    out.push_str(",\"wait\":true,\"auto_step\":true}");
    out
}

/// Locate the exact `(cx, cy)` center of `control::observe::render_frame_png`
/// (`cursor = true`)'s red-dot + cyan-arm + black-outline crosshair by comparing
/// `annotated_png` against `raw_bgra`.
///
/// Returns `Ok(None)` when no crosshair was drawn (e.g. `cursor = false` or a
/// blank OS cursor `SetCursor(NULL)` suppressed the overlay), and `Ok(Some((x,
/// y)))` with `0 px` accuracy across interior positions, screen corners like
/// `(0, 0)` or `(w - 1, h - 1)`, red/cyan backgrounds, and grid overlays.
///
/// # Errors
/// Returns an error if `annotated_png` fails to decode, `raw_bgra` length does
/// not match the decoded dimensions, or multiple indistinguishable crosshair
/// centers match.
pub fn crosshair_center(
    raw_bgra: &[u8],
    annotated_png: &[u8],
) -> Result<Option<(i32, i32)>, String> {
    const BLACK: [u8; 3] = [0, 0, 0];
    const CYAN: [u8; 3] = [255, 255, 0];
    const RED: [u8; 3] = [0, 0, 255];

    let (w, h, drawn) = decode_to_bgra(annotated_png)?;
    if raw_bgra.len() != drawn.len() {
        return Err(format!(
            "raw_bgra length {} does not match decoded PNG {}x{} ({} bytes)",
            raw_bgra.len(),
            w,
            h,
            drawn.len()
        ));
    }

    let wi = w as i32;
    let hi = h as i32;
    let stride = w as usize;
    let px_bgr = |buf: &[u8], x: i32, y: i32| -> [u8; 3] {
        let i = ((y as usize) * stride + (x as usize)) * 4;
        [buf[i], buf[i + 1], buf[i + 2]]
    };

    let match_stencil = |cx: i32, cy: i32| -> Option<usize> {
        let mut changed = usize::from(px_bgr(&drawn, cx, cy) != px_bgr(raw_bgra, cx, cy));
        let mut check = |x: i32, y: i32, expected: [u8; 3]| -> bool {
            if x < 0 || y < 0 || x >= wi || y >= hi {
                return true;
            }
            let got = px_bgr(&drawn, x, y);
            if got != expected {
                return false;
            }
            if got != px_bgr(raw_bgra, x, y) {
                changed += 1;
            }
            true
        };

        for d in (-11..=-3).chain(3..=11) {
            if !check(cx + d, cy, CYAN) || !check(cx, cy + d, CYAN) {
                return None;
            }
        }
        for d in (-12..=-3).chain(3..=12) {
            if !check(cx + d, cy - 1, BLACK)
                || !check(cx + d, cy + 1, BLACK)
                || !check(cx - 1, cy + d, BLACK)
                || !check(cx + 1, cy + d, BLACK)
            {
                return None;
            }
        }
        if !check(cx - 12, cy, BLACK)
            || !check(cx + 12, cy, BLACK)
            || !check(cx, cy - 12, BLACK)
            || !check(cx, cy + 12, BLACK)
        {
            return None;
        }
        (changed > 0).then_some(changed)
    };

    let mut best_changed = 0usize;
    let mut best: Vec<(i32, i32)> = Vec::new();
    for y in 0..hi {
        let row = (y as usize) * stride * 4;
        for x in 0..wi {
            let i = row + (x as usize) * 4;
            if drawn[i..i + 3] != RED {
                continue;
            }
            if let Some(changed) = match_stencil(x, y) {
                if changed > best_changed {
                    best_changed = changed;
                    best.clear();
                    best.push((x, y));
                } else if changed == best_changed {
                    best.push((x, y));
                }
            }
        }
    }

    match best.as_slice() {
        [] => Ok(None),
        &[center] => Ok(Some(center)),
        many => Err(format!(
            "ambiguous --cursor crosshair centers matching stencil: {many:?}"
        )),
    }
}

fn bgra_rgb_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && count_diff_pixels(a, b) == 0
}

/// Count how many BGRA pixels differ in their RGB channels between `a` and `b`.
#[must_use]
pub fn count_diff_pixels(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(pa, pb)| pa[..3] != pb[..3])
        .count()
}

/// Fraction in `0.0..=1.0` of BGRA pixels whose RGB channels differ between `a`
/// and `b` (`0.0` when either slice has no complete pixel).
#[must_use]
pub fn fraction_diff(a: &[u8], b: &[u8]) -> f64 {
    let total = a.as_chunks::<4>().0.len().min(b.as_chunks::<4>().0.len());
    if total == 0 {
        return 0.0;
    }
    count_diff_pixels(a, b) as f64 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Trace;
    use crate::control::bus::{FrameDiff, FrameSnapshot};
    use crate::control::json::parse_action_obj;
    use crate::control::observe::render_frame_png;
    use crate::guest::GuestMem;
    use crate::machine::Machine;
    use crate::runlog::RunLog;
    use crate::stub;

    fn multi_flip_stub(count: u32) -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0xb9);
        code.extend_from_slice(&count.to_le_bytes());
        code.extend_from_slice(&[0xb8, 0x01, 0x00, 0x00, 0x00]);
        code.extend_from_slice(&[0xa3, 0x00, 0x00, 0x10, 0x00]);
        code.extend_from_slice(&[0xcd, stub::INT_PRESENT]);
        code.push(0x40);
        code.push(0x49);
        code.extend_from_slice(&[0x75, (-11i8) as u8]);
        code.extend_from_slice(&[0xcd, stub::INT_EXIT]);
        code
    }

    fn stub_rig(flips: u32, scratch: PathBuf) -> ControlRig {
        let prog = multi_flip_stub(flips);
        let config = ControlConfig {
            title: Title::STUB.key,
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            step_uncapped: true,
            ..ControlConfig::default()
        };
        let data_root = Some(scratch.join("data-root"));
        ControlRig::in_process_with_runner(
            &Title::STUB,
            &TitleGuide::EMPTY,
            config,
            data_root,
            Some(scratch),
            move |control| {
                let mut control = control.with_deferred_pause(true);
                let obs_bus = control.bus();
                obs_bus.set_machine_attached(true);
                let mut machine = Machine::new(&Title::STUB, &mut control).expect("machine");
                machine.load_stub().expect("load stub");
                machine.arena.write(stub::ENTRY, &prog).expect("write stub");
                let mut log = RunLog::create("spectator-stub-test", Trace::Off).expect("runlog");
                machine.set_flip_observer(Box::new(move |m, h| {
                    service_flip_boundary(&obs_bus, m, h);
                }));
                let finish = machine.run(&mut control, &mut log, 200_000);
                let digest = machine.state_digest();
                control.mark_finished(&finish, &digest);
            },
        )
        .expect("start stub rig")
    }

    #[test]
    fn multi_flip_stub_loopback_draining_ws_survives_idle_and_delivers_keyframes_and_deltas() {
        let scratch = create_rig_scratch("spectator-stub").expect("scratch dir");
        let rig = stub_rig(12, scratch);
        let bus = rig.bus().expect("in-process rig exposes ControlBus");
        let st0 = bus.wait_until_paused(Some(Duration::from_secs(5)));
        assert!(
            st0.paused && st0.flip == Some(0),
            "initial flip 0 pause: {st0:?}"
        );

        // Non-WS HTTP calls work before opening `/api/stream`.
        let pre_meta = rig.http().meta().expect("pre-WS GET /api/meta");
        assert_eq!(
            pre_meta.as_object().unwrap()["title"].as_str(),
            Some("stub")
        );

        let client = rig.connect().expect("connect SpectatorClient");
        let meta = client.meta().expect("GET /api/meta");
        assert_eq!(meta.as_object().unwrap()["title"].as_str(), Some("stub"));

        let f0 = client
            .wait_for_any_frame(Duration::from_secs(5))
            .expect("initial keyframe");
        assert_eq!(f0.kind, FRAME_FLAG_PNG_KEYFRAME);
        assert_eq!(f0.flip, 0);
        assert_eq!((f0.width, f0.height), (4, 4));
        let snap0 = bus.latest_frame().expect("snap 0");
        assert_eq!(count_diff_pixels(&f0.bgra, &snap0.bgra), 0);

        let png0 = client
            .frame_png(ObserveOptions::default())
            .expect("GET /api/frame.png");
        let (pw, ph, bgra0) = decode_to_bgra(&png0).expect("decode /api/frame.png");
        assert_eq!((pw, ph), (4, 4));
        assert_eq!(bgra0, f0.bgra);

        client.step(1).expect("step 1");
        let snap1 = bus.latest_frame().expect("snap 1");
        assert_eq!(snap1.flip, 1);
        let f1 = client
            .wait_for_matching_frame(
                &snap1.bgra,
                snap1.pointer_pos,
                snap1.pointer_shown,
                1,
                1,
                Duration::from_secs(5),
            )
            .expect("wait_for_matching_frame flip 1");
        assert_eq!(f1.kind, FRAME_FLAG_XOR_RLE_DELTA);
        assert_eq!(count_diff_pixels(&f1.bgra, &snap1.bgra), 0);
        assert_eq!(count_diff_pixels(&f0.bgra, &f1.bgra), 1);

        // Exceed server STREAM_WRITE_TIMEOUT (2 s) while idle to prove draining WS stays alive.
        thread::sleep(Duration::from_millis(2_300));
        assert!(
            !client.is_closed(),
            "draining WS client must survive > STREAM_WRITE_TIMEOUT (2 s) idle without disconnect"
        );

        client
            .act(&Action::Move {
                x: 2,
                y: 1,
                glide_flips: 1,
            })
            .expect("typed act move");
        let st_after_move = client.status().expect("GET /api/status");
        let flip_after_move = st_after_move.as_object().unwrap()["status"]
            .as_object()
            .unwrap()["flip"]
            .as_u64()
            .expect("flip");
        let snap_move = bus.latest_frame().expect("post-move snap");
        let f_move = client
            .wait_for_pixels(flip_after_move, &snap_move.bgra, Duration::from_secs(5))
            .expect("wait_for_pixels after idle");
        assert_eq!(count_diff_pixels(&f_move.bgra, &snap_move.bgra), 0);

        client
            .act_json(
                r#"{"action":"key_hold","key":"Space","hold_flips":1,"wait":true,"auto_step":true}"#,
            )
            .expect("act_json key_hold");
        client
            .pipe(
                "#lego-legacy-route 1\nflip +1 key Space down\n",
                true,
                false,
            )
            .expect("pipe route");
        client.step(1).expect("step after pipe");
        let snap_end = bus.latest_frame().expect("end snap");
        let _ = client
            .wait_for_pixels(snap_end.flip, &snap_end.bgra, Duration::from_secs(5))
            .expect("wait_for_pixels end");

        let (keyframes, deltas) = client.counts();
        assert!(
            keyframes >= 1 && deltas >= 1,
            "expected keyframes >= 1 && deltas >= 1, got ({keyframes}, {deltas})"
        );
    }

    #[test]
    fn in_process_rig_and_client_clean_up_on_panic() {
        let scratch = create_rig_scratch("spectator-panic").expect("scratch");
        let scratch_check = scratch.clone();
        let addr_slot: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        let addr_for_panic = Arc::clone(&addr_slot);

        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let rig = stub_rig(20, scratch);
            *addr_for_panic.lock().unwrap() = Some(rig.addr());
            let bus = rig.bus().unwrap();
            let _ = bus.wait_until_paused(Some(Duration::from_secs(5)));
            let client = rig.connect().expect("connect WS");
            let _ = client
                .wait_for_any_frame(Duration::from_secs(5))
                .expect("first frame");
            panic!("intentional test panic to prove Drop cleanup");
        }));
        assert!(panicked.is_err(), "closure must panic");
        assert!(
            !scratch_check.exists(),
            "rig-owned scratch dir must be deleted during panic unwind"
        );
        let addr = addr_slot.lock().unwrap().expect("captured addr");
        assert!(
            TcpStream::connect(addr).is_err(),
            "loopback HTTP server must be stopped and port closed after Drop"
        );
    }

    #[test]
    fn in_process_default_stub_runs_and_cleans_owned_scratch() {
        let cli = Cli {
            stub: true,
            ..Cli::default()
        };
        let rig = ControlRig::in_process(&Title::STUB, &TitleGuide::EMPTY, cli, 100_000)
            .expect("in_process stub");
        assert!(rig.data_root().is_some());
        let bus = rig.bus().expect("bus");
        let paused = bus.wait_until_paused(Some(Duration::from_secs(5)));
        assert!(paused.paused && paused.flip == Some(0));
        let client = rig.connect().expect("connect");
        let frame = client
            .wait_for_any_frame(Duration::from_secs(5))
            .expect("stub frame 0");
        assert_eq!(frame.flip, 0);
        assert_eq!((frame.width, frame.height), (4, 4));
    }

    #[test]
    fn spawn_binary_cleans_up_data_root_on_early_exit() {
        let data_root = create_rig_scratch("spectator-spawn-fail").expect("scratch");
        let missing = data_root.join("does-not-exist-binary");
        let err = ControlRig::spawn_binary(&missing, &data_root, &[])
            .err()
            .expect("missing binary must fail");
        assert!(err.contains("spawn"), "unexpected error: {err}");
        assert!(
            !data_root.exists(),
            "data_root must be removed when spawn_binary fails"
        );
    }

    #[cfg(unix)]
    #[test]
    fn spawn_binary_parses_control_url_drains_stderr_and_cleans_up_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let script_dir = create_rig_scratch("spectator-spawn-bin").expect("script dir");
        let script = script_dir.join("fake-title.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             echo 'booting fake title...' >&2\n\
             echo 'lego-legacy: control http://127.0.0.1:43210/' >&2\n\
             echo 'extra stderr line' >&2\n\
             exec sleep 30\n",
        )
        .expect("write fake binary");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake binary");

        let data_root = create_rig_scratch("spectator-spawn-data").expect("data_root");
        let data_root_check = data_root.clone();
        {
            let rig =
                ControlRig::spawn_binary(&script, &data_root, &["--stub"]).expect("spawn_binary");
            assert_eq!(rig.addr(), "127.0.0.1:43210".parse::<SocketAddr>().unwrap());
            assert!(rig.bus().is_none(), "spawned rig has no in-process bus");
            assert_eq!(rig.data_root(), Some(data_root_check.as_path()));
            assert!(data_root_check.exists());
        }
        assert!(
            !data_root_check.exists(),
            "spawn_binary Drop must remove data_root after killing child"
        );
        let _ = std::fs::remove_dir_all(&script_dir);
    }

    fn make_snap(
        w: u32,
        h: u32,
        bg: [u8; 4],
        pointer_pos: (i32, i32),
        blank: bool,
    ) -> FrameSnapshot {
        let mut bgra = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            bgra.extend_from_slice(&bg);
        }
        FrameSnapshot {
            flip: 1,
            width: w,
            height: h,
            bgra: Arc::new(bgra),
            frame_hash: "0000000000000000".to_owned(),
            diff: FrameDiff::default(),
            uniform: true,
            pointer_pos,
            pointer_shown: true,
            cursor_blank: blank,
            cursor_visible: !blank,
        }
    }

    #[test]
    fn crosshair_center_achieves_zero_px_accuracy_across_corners_backgrounds_and_suppression() {
        let (w, h) = (64u32, 48u32);
        let backgrounds = [
            ("dark", [20, 40, 60, 255]),
            ("pure_red", [0, 0, 255, 255]),
            ("pure_cyan", [255, 255, 0, 255]),
            ("pure_black", [0, 0, 0, 255]),
        ];
        let positions = [(32, 24), (0, 0), (63, 47), (0, 47), (63, 0), (1, 1)];

        for (bg_name, bg) in backgrounds {
            for pos in positions {
                let snap = make_snap(w, h, bg, pos, false);
                let png = render_frame_png(
                    &snap,
                    ObserveOptions {
                        grid: false,
                        cursor: true,
                        crop: None,
                    },
                    pos,
                )
                .expect("render cursor png");
                let detected = crosshair_center(&snap.bgra, &png).expect("crosshair_center decode");
                assert_eq!(
                    detected,
                    Some(pos),
                    "0 px crosshair mismatch on {bg_name} at {pos:?}"
                );
            }
        }

        let grid_snap = make_snap(160, 120, [30, 50, 70, 255], (80, 80), false);
        let grid_png = render_frame_png(
            &grid_snap,
            ObserveOptions {
                grid: true,
                cursor: true,
                crop: None,
            },
            (80, 80),
        )
        .expect("render grid+cursor png");
        assert_eq!(
            crosshair_center(&grid_snap.bgra, &grid_png).expect("grid+cursor center"),
            Some((80, 80))
        );

        let blank_snap = make_snap(w, h, [20, 40, 60, 255], (32, 24), true);
        let blank_png = render_frame_png(
            &blank_snap,
            ObserveOptions {
                cursor: true,
                ..ObserveOptions::default()
            },
            (32, 24),
        )
        .expect("render blank cursor png");
        assert_eq!(
            crosshair_center(&blank_snap.bgra, &blank_png).expect("suppressed crosshair"),
            None
        );
    }

    #[test]
    fn diff_helpers_count_rgb_changes_and_ignore_alpha() {
        let a = [
            10, 20, 30, 255, 40, 50, 60, 0, 70, 80, 90, 255, 1, 2, 3, 255,
        ];
        let b = [
            10, 20, 30, 0, 40, 50, 61, 255, 70, 80, 90, 128, 9, 2, 3, 255,
        ];
        assert_eq!(count_diff_pixels(&a, &b), 2);
        assert!((fraction_diff(&a, &b) - 0.5).abs() < 1e-12);
        assert_eq!(count_diff_pixels(&[], &[]), 0);
        assert_eq!(fraction_diff(&[], &[]), 0.0);
    }

    #[test]
    fn serialize_action_json_roundtrips_every_variant_through_parse_action_obj() {
        let actions = vec![
            Action::Move {
                x: 123,
                y: 456,
                glide_flips: 3,
            },
            Action::MouseDelta {
                dx: -12.5,
                dy: 40.25,
                flips: 2,
                scale_x: Some(2.0),
                scale_y: Some(1.5),
            },
            Action::MouseDown {
                x: Some(10),
                y: Some(20),
                button: Button::Left,
            },
            Action::MouseUp {
                x: None,
                y: None,
                button: Button::Right,
            },
            Action::Click {
                x: Some(320),
                y: Some(240),
                button: Button::Left,
                hold_flips: 4,
                hold_ms: Some(120),
                settle_flips: 2,
            },
            Action::ClickPoll {
                x: Some(576),
                y: Some(420),
                button: Button::Right,
                hold_flips: 2,
                hold_ms: 150,
            },
            Action::Drag {
                from_x: 10,
                from_y: 20,
                to_x: 300,
                to_y: 200,
                button: Button::Left,
                step_px: 12,
            },
            Action::KeyHold {
                key: "Space".to_owned(),
                hold_flips: 5,
                hold_ms: Some(80),
            },
            Action::Chord {
                keys: vec!["LeftControl".to_owned(), "S".to_owned()],
                hold_flips: 3,
                hold_ms: None,
                lead_flips: 1,
            },
            Action::TypeText {
                text: "Hello \"world\"!\n".to_owned(),
                flip_gap: 3,
            },
        ];

        for action in actions {
            let json = serialize_action_json(&action);
            let parsed = parse_json(&json)
                .unwrap_or_else(|e| panic!("invalid JSON for {action:?}: {e}\n{json}"));
            let obj = parsed.as_object().expect("JSON object");
            let kind = obj["action"].as_str().expect("action string");
            let roundtripped = parse_action_obj(kind, obj)
                .unwrap_or_else(|e| panic!("parse_action_obj failed for {action:?}: {e}\n{json}"));
            assert_eq!(roundtripped, action);
        }
    }
}
