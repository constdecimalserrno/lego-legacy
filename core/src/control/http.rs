//! Zero-crate HTTP/1.1 control & introspection server (`--control [HOST:PORT]`)
//! (#309).
//!
//! Serves read-only visual grounding (`/api/frame.png`, `/api/frame.json`),
//! guest memory/CPU/Win32/files introspection (`/api/mem`, `/api/mem/map`,
//! `/api/mem/search`, `/api/cpu`, `/api/win32`, `/api/files`, `/api/files/read`,
//! `/api/route`, `/api/fps`), unified control & task piping (`/api/act`,
//! `/api/pipe`, `/api/step`, `/api/wait`, `/api/mode`, `/api/agent-state`),
//! Tier-1 route checkpoints (`/api/bookmark`, `/api/rewind`), baseline title
//! metadata stubs (`/api/meta`, `/api/controls`, `/api/goals` ready for #311),
//! and a streaming [`ExtraRoutes`] hook ready for #310 (`/watch`, `/api/stream`,
//! `/api/events`).

use super::bus::{Command, ControlBus, ExecMode, InputOwner, Response, ResponseData};
use super::inspect::{FileRoot, InspectQuery, InspectReply, MemFmt};
use super::json::{
    base64_encode, parse_action_obj, parse_agent_state_patch, parse_hex_bytes, parse_json,
    parse_u32_addr, write_agent_state, write_diff, write_fps_report, write_json_f64,
    write_json_str, write_status, write_wait_result, JsonValue,
};
use super::meta::{format_controls_json, format_goals_json, format_meta_json};
use super::observe::{
    format_frame_json, parse_crop, parse_grid_dims, render_frame_png, ObserveOptions,
};
use super::stdio::parse_command_line;
use crate::title::TitleGuide;
use crate::Title;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Maximum accepted HTTP request body size (4 MiB).
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Default timeout for synchronous inspect/action/rewind requests.
const DEFAULT_SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// Parsed HTTP/1.1 request.
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

/// HTTP/1.1 response ready to serialize to a stream.
#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: &'static str,
    pub content_type: &'static str,
    pub extra_headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    #[must_use]
    pub fn json(status: u16, body: String) -> Self {
        Self {
            status,
            reason: status_reason(status),
            content_type: "application/json; charset=utf-8",
            extra_headers: Vec::new(),
            body: body.into_bytes(),
        }
    }

    #[must_use]
    pub fn error_json(status: u16, message: &str) -> Self {
        let mut out = String::with_capacity(64 + message.len());
        out.push_str("{\"ok\":false,\"error\":");
        write_json_str(&mut out, message);
        out.push('}');
        Self::json(status, out)
    }

    #[must_use]
    pub fn text(status: u16, body: String) -> Self {
        Self {
            status,
            reason: status_reason(status),
            content_type: "text/plain; charset=utf-8",
            extra_headers: Vec::new(),
            body: body.into_bytes(),
        }
    }

    #[must_use]
    pub fn raw(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason: status_reason(status),
            content_type,
            extra_headers: Vec::new(),
            body,
        }
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_headers.push((name.into(), value.into()));
        self
    }

    /// Write this HTTP/1.1 response to `writer`.
    ///
    /// # Errors
    /// Returns any I/O error from `writer`.
    pub fn write_to<W: Write + ?Sized>(&self, writer: &mut W) -> io::Result<()> {
        write!(
            writer,
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
            self.status,
            self.reason,
            self.content_type,
            self.body.len()
        )?;
        for (k, v) in &self.extra_headers {
            write!(writer, "{k}: {v}\r\n")?;
        }
        writer.write_all(b"\r\n")?;
        writer.write_all(&self.body)?;
        writer.flush()
    }
}

fn status_reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        410 => "Gone",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    }
}

/// Bidirectional byte stream exposed to streaming [`ExtraRoutes`] handlers
/// (WebSocket `/api/stream`, multipart PNG, and SSE `/api/events`, #310).
pub trait StreamIo: Read + Write {
    /// Set read timeout on the underlying socket, if supported.
    ///
    /// # Errors
    /// Returns any socket option error.
    fn set_stream_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        Ok(())
    }

    /// Set write timeout on the underlying socket, if supported.
    ///
    /// # Errors
    /// Returns any socket option error.
    fn set_stream_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
        Ok(())
    }

    /// Disable Nagle's algorithm on TCP sockets for low-latency frame delivery.
    ///
    /// # Errors
    /// Returns any socket option error.
    fn set_stream_nodelay(&self, _nodelay: bool) -> io::Result<()> {
        Ok(())
    }

    /// Whether [`Self::set_stream_read_timeout`] enforces timed reads (so loops
    /// can poll client Close/Ping/EOF without stalling frame pushes).
    fn supports_read_timeout(&self) -> bool {
        false
    }
}

impl StreamIo for TcpStream {
    fn set_stream_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(timeout)
    }

    fn set_stream_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.set_write_timeout(timeout)
    }

    fn set_stream_nodelay(&self, nodelay: bool) -> io::Result<()> {
        self.set_nodelay(nodelay)
    }

    fn supports_read_timeout(&self) -> bool {
        true
    }
}

struct GenericStream<S>(S);

impl<S: Read> Read for GenericStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl<S: Write> Write for GenericStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<S: Read + Write> StreamIo for GenericStream<S> {}

/// Extensible route hook for downstream tickets (#310 `/watch`, `/api/stream`,
/// `/api/events`; #311 `/api/meta`, `/api/controls`, `/api/goals`).
pub trait ExtraRoutes: Send + Sync {
    /// Attempt to handle `req` directly on `stream`.
    ///
    /// Return `Ok(true)` if the response was written to `stream`, or `Ok(false)`
    /// to fall through to the built-in router.
    ///
    /// # Errors
    /// Returns any I/O error writing to `stream`.
    fn handle(
        &self,
        req: &HttpRequest,
        ctx: &HttpContext,
        stream: &mut dyn Write,
    ) -> io::Result<bool>;

    /// Streaming variant exposing full bidirectional [`StreamIo`] (used by
    /// WebSocket `/api/stream` and SSE `/api/events`). Defaults to [`Self::handle`].
    ///
    /// # Errors
    /// Returns any I/O error reading from or writing to `stream`.
    fn handle_stream(
        &self,
        req: &HttpRequest,
        ctx: &HttpContext,
        stream: &mut dyn StreamIo,
    ) -> io::Result<bool> {
        self.handle(req, ctx, stream)
    }
}

/// Shared context passed to every HTTP/1.1 and UDS request handler.
pub struct HttpContext {
    pub bus: ControlBus,
    pub title: &'static Title,
    pub guide: &'static TitleGuide,
    pub token: Option<String>,
    pub extra: Option<Arc<dyn ExtraRoutes>>,
}

impl HttpContext {
    #[must_use]
    pub fn new(bus: ControlBus, title: &'static Title, token: Option<String>) -> Self {
        let guide = bus.guide();
        Self {
            bus,
            title,
            guide,
            token: token.filter(|t| !t.is_empty()),
            extra: Some(Arc::new(super::spectator::SpectatorRoutes)),
        }
    }

    #[must_use]
    pub fn with_guide(mut self, guide: &'static TitleGuide) -> Self {
        self.guide = guide;
        self
    }

    #[must_use]
    pub fn with_extra(mut self, extra: Arc<dyn ExtraRoutes>) -> Self {
        self.extra = Some(extra);
        self
    }
}

/// RAII guard that removes `~/.local/state/lego-legacy/sessions/<pid>.json` on drop.
pub struct SessionRegistryGuard {
    path: Option<PathBuf>,
}

impl SessionRegistryGuard {
    #[must_use]
    pub fn empty() -> Self {
        Self { path: None }
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

impl Drop for SessionRegistryGuard {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn sessions_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    if home.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("lego-legacy")
            .join("sessions"),
    )
}

/// Write a session discovery file into `~/.local/state/lego-legacy/sessions/<pid>.json`,
/// returning an RAII guard that removes it cleanly on drop.
///
/// Silently skips if `$HOME` / `%USERPROFILE%` is unavailable or unwritable.
#[must_use]
pub fn write_session_registry(
    title: &str,
    control_url: &str,
    watch_url: Option<&str>,
    windowed: bool,
) -> SessionRegistryGuard {
    let Some(dir) = sessions_dir() else {
        return SessionRegistryGuard::empty();
    };
    write_session_registry_in(&dir, title, control_url, watch_url, windowed)
}

pub(crate) fn write_session_registry_in(
    dir: &Path,
    title: &str,
    control_url: &str,
    watch_url: Option<&str>,
    windowed: bool,
) -> SessionRegistryGuard {
    if std::fs::create_dir_all(dir).is_err() {
        return SessionRegistryGuard::empty();
    }
    let pid = std::process::id();
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut json = String::with_capacity(256);
    let _ = write!(json, "{{\"pid\":{pid},\"title\":");
    write_json_str(&mut json, title);
    json.push_str(",\"control_url\":");
    write_json_str(&mut json, control_url);
    json.push_str(",\"watch_url\":");
    match watch_url {
        Some(w) => write_json_str(&mut json, w),
        None => json.push_str("null"),
    }
    let _ = writeln!(
        json,
        ",\"windowed\":{windowed},\"started_at\":{started_at}}}"
    );
    let path = dir.join(format!("{pid}.json"));
    if std::fs::write(&path, json).is_ok() {
        SessionRegistryGuard { path: Some(path) }
    } else {
        SessionRegistryGuard::empty()
    }
}

/// Running TCP HTTP/1.1 control server.
pub struct HttpServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl HttpServer {
    /// Bind `bind` (e.g. `"127.0.0.1:0"` or `"0.0.0.0:7777"`) and start serving.
    ///
    /// # Errors
    /// Returns any socket bind or thread spawn error.
    pub fn start(bind: &str, ctx: Arc<HttpContext>) -> io::Result<Self> {
        let listener = TcpListener::bind(bind)?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("control-http".to_owned())
            .spawn(move || {
                while !stop_flag.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                            let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
                            let conn_ctx = Arc::clone(&ctx);
                            let _ = thread::Builder::new()
                                .name("control-http-req".to_owned())
                                .spawn(move || {
                                    handle_stream_connection(stream, &conn_ctx);
                                });
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => {
                            if stop_flag.load(Ordering::Relaxed) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
            })?;
        Ok(Self {
            addr,
            stop,
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    #[must_use]
    pub fn watch_url(&self) -> String {
        format!("http://{}/watch", self.addr)
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Handle one HTTP/1.1 connection over any `Read + Write` stream (TCP or Unix socket).
pub fn handle_connection<S: Read + Write>(stream: S, ctx: &HttpContext) {
    handle_stream_connection(GenericStream(stream), ctx);
}

fn handle_stream_connection<S: StreamIo>(mut stream: S, ctx: &HttpContext) {
    let req = match read_http_request(&mut stream) {
        Ok(Some(r)) => r,
        Ok(None) => return,
        Err((status, msg)) => {
            let _ = HttpResponse::error_json(status, &msg).write_to(&mut stream);
            return;
        }
    };

    if let Some(extra) = &ctx.extra {
        match extra.handle_stream(&req, ctx, &mut stream) {
            Ok(true) => return,
            Ok(false) => {}
            Err(_) => return,
        }
    }

    let resp = dispatch_http(&req, ctx);
    let _ = resp.write_to(&mut stream);
}

fn read_http_request<R: Read>(stream: &mut R) -> Result<Option<HttpRequest>, (u16, String)> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    match reader.read_line(&mut request_line) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(e) => return Err((400, format!("read error: {e}"))),
    }
    let trimmed = request_line.trim_end_matches(['\r', '\n']);
    let mut parts = trimmed.split_whitespace();
    let Some(method) = parts.next() else {
        return Err((400, "empty request line".to_owned()));
    };
    let Some(target) = parts.next() else {
        return Err((400, "missing request target".to_owned()));
    };

    let mut headers = BTreeMap::new();
    let mut line = String::new();
    let mut header_count = 0usize;
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| (400, format!("header read error: {e}")))?;
        if n == 0 {
            break;
        }
        let h = line.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        header_count += 1;
        if header_count > 128 {
            return Err((400, "too many headers".to_owned()));
        }
        if let Some((name, val)) = h.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), val.trim().to_owned());
        }
    }

    let content_len: usize = match headers.get("content-length") {
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| (400, "invalid Content-Length".to_owned()))?,
        None => 0,
    };
    if content_len > MAX_BODY_BYTES {
        return Err((
            413,
            format!("request body exceeds {MAX_BODY_BYTES} byte limit"),
        ));
    }

    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|e| (400, format!("body read error: {e}")))?;
    }

    let (raw_path, raw_query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(raw_path);
    let query = parse_query(raw_query);

    Ok(Some(HttpRequest {
        method: method.to_ascii_uppercase(),
        path,
        query,
        headers,
        body,
    }))
}

fn parse_query(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if raw.is_empty() {
        return map;
    }
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(percent_decode(k), percent_decode(v));
    }
    map
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(val) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(val);
                i += 3;
                continue;
            }
            out.push(b'%');
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

fn check_auth(req: &HttpRequest, ctx: &HttpContext) -> bool {
    let Some(expected) = &ctx.token else {
        return true;
    };
    if let Some(header) = req.headers.get("authorization") {
        if let Some(provided) = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))
        {
            return constant_time_eq(provided.trim().as_bytes(), expected.as_bytes());
        }
    }
    if let Some(provided) = req.query.get("token") {
        return constant_time_eq(provided.trim().as_bytes(), expected.as_bytes());
    }
    false
}

fn parse_bool_param(val: Option<&String>) -> bool {
    matches!(
        val.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// Core HTTP/1.1 router shared across TCP and Unix domain sockets.
#[must_use]
pub fn dispatch_http(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    if req.method == "OPTIONS" {
        return HttpResponse::raw(204, "text/plain", Vec::new())
            .with_header("Allow", "GET, POST, OPTIONS");
    }

    let is_mutating_post = req.method == "POST"
        && matches!(
            req.path.as_str(),
            "/api/act"
                | "/api/pipe"
                | "/api/step"
                | "/api/wait"
                | "/api/mode"
                | "/api/agent-state"
                | "/api/bookmark"
                | "/api/rewind"
                | "/api/snapshot"
                | "/api/restore"
        );
    if is_mutating_post && !check_auth(req, ctx) {
        return HttpResponse::error_json(401, "unauthorized: valid Bearer token required")
            .with_header("WWW-Authenticate", "Bearer");
    }

    match (req.method.as_str(), req.path.as_str()) {
        // --- Status & Telemetry ---
        ("GET", "/api/status") => {
            let st = ctx.bus.status();
            let mut out = String::with_capacity(512);
            out.push_str("{\"ok\":true,\"status\":");
            write_status(&mut out, &st);
            // Also flatten top-level fields for ergonomic direct reads.
            let _ = write!(out, ",\"title\":");
            write_json_str(&mut out, &st.title);
            let _ = write!(out, ",\"mode\":");
            write_json_str(&mut out, st.mode.as_str());
            let _ = write!(out, ",\"owner\":");
            write_json_str(&mut out, st.owner.as_str());
            let _ = write!(out, ",\"paused\":{},\"flip\":", st.paused);
            match st.flip {
                Some(f) => {
                    let _ = write!(out, "{f}");
                }
                None => out.push_str("null"),
            }
            let _ = write!(
                out,
                ",\"cursor\":{},\"declared_ms\":{},\"pointer\":[{},{}],\"held_keys\":[",
                st.cursor, st.declared_ms, st.pointer_pos.0, st.pointer_pos.1
            );
            for (i, k) in st.held_keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{k}");
            }
            out.push_str("],\"diff_pct\":");
            write_json_f64(&mut out, st.diff.diff_pct);
            let _ = write!(
                out,
                ",\"unchanged_streak\":{},\"route_active\":{},\"recording_len\":{},\"outcome\":",
                st.diff.unchanged_streak, st.route_active, st.recording_len
            );
            match &st.outcome {
                Some(o) => write_json_str(&mut out, o),
                None => out.push_str("null"),
            }
            out.push('}');
            HttpResponse::json(200, out)
        }
        ("GET", "/api/fps") => handle_fps(req, ctx),

        // --- Visual Grounding ---
        ("GET", "/api/frame.png") => handle_frame_png(req, ctx),
        ("GET", "/api/frame.json") => handle_frame_json(req, ctx),

        // --- Read-Only Machine Introspection ---
        ("GET", "/api/mem") => handle_mem_read(req, ctx),
        ("GET", "/api/mem/map") => inspect_to_http(
            ctx.bus
                .request_inspect(InspectQuery::MemMap, Some(DEFAULT_SYNC_TIMEOUT)),
        ),
        ("POST", "/api/mem/search") => handle_mem_search(req, ctx),
        ("GET", "/api/cpu") => inspect_to_http(
            ctx.bus
                .request_inspect(InspectQuery::Cpu, Some(DEFAULT_SYNC_TIMEOUT)),
        ),
        ("GET", "/api/win32") => inspect_to_http(
            ctx.bus
                .request_inspect(InspectQuery::Win32, Some(DEFAULT_SYNC_TIMEOUT)),
        ),
        ("GET", "/api/files") => handle_files_list(req, ctx),
        ("GET", "/api/files/read") => handle_files_read(req, ctx),
        ("GET", "/api/route") => inspect_to_http(
            ctx.bus
                .request_inspect(InspectQuery::Route, Some(DEFAULT_SYNC_TIMEOUT)),
        ),

        // --- Agent State ---
        ("GET", "/api/agent-state") => {
            let st = ctx.bus.agent_state();
            let mut out = String::with_capacity(256);
            out.push_str("{\"ok\":true,\"agent_state\":");
            write_agent_state(&mut out, &st);
            out.push('}');
            HttpResponse::json(200, out)
        }
        ("POST", "/api/agent-state") => handle_set_agent_state(req, ctx),

        // --- First-Class Per-Title Metadata (#311) ---
        ("GET", "/api/meta") => HttpResponse::json(200, format_meta_json(ctx.title, ctx.guide)),
        ("GET", "/api/controls") => {
            HttpResponse::json(200, format_controls_json(ctx.title, ctx.guide))
        }
        ("GET", "/api/goals") => HttpResponse::json(200, format_goals_json(ctx.title, ctx.guide)),

        // --- Control, Piping & Tier-1 Checkpoints ---
        ("POST", "/api/act") => handle_act(req, ctx),
        ("POST", "/api/pipe") => handle_pipe(req, ctx),
        ("POST", "/api/step") => handle_step(req, ctx),
        ("POST", "/api/wait") => handle_wait(req, ctx),
        ("POST", "/api/mode") => handle_mode(req, ctx),
        ("GET", "/api/bookmark") => handle_list_bookmarks(ctx),
        ("POST", "/api/bookmark") => handle_bookmark(req, ctx),
        ("POST", "/api/rewind") => handle_rewind(req, ctx),
        ("GET", "/api/snapshot") => super::snapshot::handle_list_snapshots(ctx),
        ("POST", "/api/snapshot") => super::snapshot::handle_save_snapshot(req, ctx),
        ("POST", "/api/restore") => super::snapshot::handle_restore_snapshot(req, ctx),

        _ => HttpResponse::error_json(404, &format!("no route for {} {}", req.method, req.path)),
    }
}

fn inspect_to_http(reply: InspectReply) -> HttpResponse {
    match reply {
        InspectReply::Json(body) => HttpResponse::json(200, body),
        InspectReply::Raw {
            content_type,
            bytes,
            sha256,
        } => {
            let resp = HttpResponse::raw(200, content_type, bytes);
            match sha256 {
                Some(h) => resp.with_header("X-Sha256", h),
                None => resp,
            }
        }
        InspectReply::Text(text) => HttpResponse::text(200, text),
        InspectReply::Bookmark(bm) => {
            let mut out = String::with_capacity(256);
            let _ = write!(
                out,
                "{{\"ok\":true,\"flip\":{},\"cursor\":{},\"declared_ms\":{},\"digest\":",
                bm.flip, bm.cursor, bm.declared_ms
            );
            write_json_str(&mut out, &bm.digest);
            out.push('}');
            HttpResponse::json(200, out)
        }
        InspectReply::Error { status, message } => HttpResponse::error_json(status, &message),
    }
}

fn handle_fps(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let window = match req.query.get("window") {
        None => None,
        Some(spec) => {
            let parsed = spec
                .split_once("..")
                .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
                .filter(|(a, b)| a < b);
            match parsed {
                Some(w) => Some(w),
                None => {
                    return HttpResponse::error_json(
                        400,
                        "invalid `window` query parameter (expected `start..end` with start < end)",
                    );
                }
            }
        }
    };
    let report = ctx.bus.telemetry_report(window);
    let mut out = String::with_capacity(512);
    out.push_str("{\"ok\":true,\"telemetry\":");
    write_fps_report(&mut out, &report);
    out.push('}');
    HttpResponse::json(200, out)
}

fn handle_frame_png(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let Some(snap) = ctx.bus.latest_frame() else {
        return HttpResponse::error_json(404, "no frame presented yet");
    };
    let crop = match req.query.get("crop") {
        None => None,
        Some(spec) if spec.trim().is_empty() => None,
        Some(spec) => match parse_crop(spec) {
            Ok(c) => Some(c),
            Err(e) => return HttpResponse::error_json(400, &e),
        },
    };
    let opts = ObserveOptions {
        grid: parse_bool_param(req.query.get("grid")),
        cursor: parse_bool_param(req.query.get("cursor")),
        crop,
    };
    let st = ctx.bus.status();
    match render_frame_png(&snap, opts, st.pointer_pos) {
        Ok(png) => HttpResponse::raw(200, "image/png", png)
            .with_header("X-Flip", snap.flip.to_string())
            .with_header("X-Cursor", st.cursor.to_string())
            .with_header("X-Frame-Hash", snap.frame_hash)
            .with_header("X-Diff-Pct", format!("{:.2}", snap.diff.diff_pct)),
        Err(e) => HttpResponse::error_json(400, &e),
    }
}

fn handle_frame_json(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let Some(snap) = ctx.bus.latest_frame() else {
        return HttpResponse::error_json(404, "no frame presented yet");
    };
    let (cols, rows) = match req.query.get("grid") {
        None => (16, 12),
        Some(spec) => match parse_grid_dims(spec) {
            Ok(dims) => dims,
            Err(e) => return HttpResponse::error_json(400, &e),
        },
    };
    let st = ctx.bus.status();
    let json = format_frame_json(
        &snap,
        cols,
        rows,
        st.pointer_pos,
        st.pointer_shown,
        st.paused,
        st.finished,
    );
    HttpResponse::json(200, json)
}

fn handle_mem_read(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let Some(addr_str) = req.query.get("addr") else {
        return HttpResponse::error_json(400, "missing `addr` query parameter");
    };
    let Some(addr) = parse_u32_addr(addr_str) else {
        return HttpResponse::error_json(400, &format!("invalid `addr` `{addr_str}`"));
    };
    let len = match req.query.get("len") {
        None => 64usize,
        Some(s) => match s.trim().parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => return HttpResponse::error_json(400, &format!("invalid `len` `{s}`")),
        },
    };
    let fmt = match req.query.get("fmt") {
        None => MemFmt::Hex,
        Some(s) => match MemFmt::parse(s) {
            Some(f) => f,
            None => {
                return HttpResponse::error_json(
                    400,
                    &format!("invalid `fmt` `{s}` (expected hex|raw|u8|u16|u32|i32|f32|cstr)"),
                );
            }
        },
    };
    inspect_to_http(ctx.bus.request_inspect(
        InspectQuery::MemRead { addr, len, fmt },
        Some(DEFAULT_SYNC_TIMEOUT),
    ))
}

fn parse_body_obj(req: &HttpRequest) -> Result<BTreeMap<String, JsonValue>, HttpResponse> {
    let text = std::str::from_utf8(&req.body)
        .map_err(|_| HttpResponse::error_json(400, "body must be valid UTF-8"))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(BTreeMap::new());
    }
    let val = parse_json(trimmed).map_err(|e| HttpResponse::error_json(400, &e))?;
    match val {
        JsonValue::Object(map) => Ok(map),
        _ => Err(HttpResponse::error_json(
            400,
            "request body must be a JSON object",
        )),
    }
}

fn handle_mem_search(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let needle = if let Some(hex) = obj.get("bytes_hex").and_then(JsonValue::as_str) {
        match parse_hex_bytes(hex) {
            Ok(b) => b,
            Err(e) => return HttpResponse::error_json(400, &e),
        }
    } else if let Some(val) = obj.get("u32").and_then(JsonValue::as_u32_flexible) {
        val.to_le_bytes().to_vec()
    } else if let Some(ascii) = obj.get("ascii").and_then(JsonValue::as_str) {
        ascii.as_bytes().to_vec()
    } else {
        return HttpResponse::error_json(
            400,
            "search requires one of `bytes_hex`, `u32`, or `ascii`",
        );
    };
    let start = obj.get("start").and_then(JsonValue::as_u32_flexible);
    let end = obj.get("end").and_then(JsonValue::as_u32_flexible);
    let limit = obj
        .get("limit")
        .and_then(JsonValue::as_u64)
        .map_or(64, |v| v as usize);
    inspect_to_http(ctx.bus.request_inspect(
        InspectQuery::MemSearch {
            needle,
            start,
            end,
            limit,
        },
        Some(DEFAULT_SYNC_TIMEOUT),
    ))
}

fn handle_files_list(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let root = match req.query.get("root") {
        None => FileRoot::Disc,
        Some(r) => match FileRoot::parse(r) {
            Some(parsed) => parsed,
            None => {
                return HttpResponse::error_json(
                    400,
                    &format!("invalid `root` `{r}` (expected disc|profile)"),
                );
            }
        },
    };
    let path = req.query.get("path").cloned().unwrap_or_default();
    inspect_to_http(ctx.bus.request_inspect(
        InspectQuery::FilesList { root, path },
        Some(DEFAULT_SYNC_TIMEOUT),
    ))
}

fn handle_files_read(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let root = match req.query.get("root") {
        None => FileRoot::Disc,
        Some(r) => match FileRoot::parse(r) {
            Some(parsed) => parsed,
            None => {
                return HttpResponse::error_json(
                    400,
                    &format!("invalid `root` `{r}` (expected disc|profile)"),
                );
            }
        },
    };
    let path = req.query.get("path").cloned().unwrap_or_default();
    inspect_to_http(ctx.bus.request_inspect(
        InspectQuery::FilesRead { root, path },
        Some(DEFAULT_SYNC_TIMEOUT),
    ))
}

fn handle_set_agent_state(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let patch = match parse_agent_state_patch(&obj) {
        Ok(p) => p,
        Err(e) => return HttpResponse::error_json(400, &e),
    };
    let resp = ctx.bus.dispatch(Command::SetAgentState(patch));
    control_resp_to_http(&resp, None)
}

fn handle_act(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let sync = obj.get("wait").and_then(JsonValue::as_bool).unwrap_or(true);
    let force = obj
        .get("force")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let auto_step = obj
        .get("auto_step")
        .and_then(JsonValue::as_bool)
        .unwrap_or_else(|| {
            !obj.get("queue")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        });

    let cmd = if let Some(arr) = obj.get("actions").and_then(JsonValue::as_array) {
        let mut actions = Vec::with_capacity(arr.len());
        for item in arr {
            let Some(item_obj) = item.as_object() else {
                return HttpResponse::error_json(400, "`actions` items must be objects");
            };
            let Some(kind) = item_obj
                .get("action")
                .or_else(|| item_obj.get("cmd"))
                .and_then(JsonValue::as_str)
            else {
                return HttpResponse::error_json(400, "each action needs `action`");
            };
            match parse_action_obj(kind, item_obj) {
                Ok(a) => actions.push(a),
                Err(e) => return HttpResponse::error_json(400, &e),
            }
        }
        Command::Batch { actions, auto_step }
    } else {
        let Some(kind) = obj
            .get("action")
            .or_else(|| obj.get("cmd"))
            .and_then(JsonValue::as_str)
        else {
            return HttpResponse::error_json(400, "missing string `action` or array `actions`");
        };
        let action = match parse_action_obj(kind, &obj) {
            Ok(a) => a,
            Err(e) => return HttpResponse::error_json(400, &e),
        };
        Command::Act { action, auto_step }
    };

    let observe_after = match obj.get("observe_after") {
        None | Some(JsonValue::Null) | Some(JsonValue::Bool(false)) => None,
        Some(JsonValue::Bool(true)) => Some((ObserveOptions::default(), false)),
        Some(JsonValue::Object(obs)) => {
            let crop = match obs.get("crop").and_then(JsonValue::as_str) {
                None => None,
                Some(spec) => match parse_crop(spec) {
                    Ok(c) => Some(c),
                    Err(e) => return HttpResponse::error_json(400, &e),
                },
            };
            let opts = ObserveOptions {
                grid: obs
                    .get("grid")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                cursor: obs
                    .get("cursor")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                crop,
            };
            let include_png = obs
                .get("include_png")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            Some((opts, include_png))
        }
        _ => {
            return HttpResponse::error_json(400, "`observe_after` must be a boolean or object");
        }
    };

    let resp = if sync {
        ctx.bus
            .dispatch_sync_forced(cmd, force, Some(DEFAULT_SYNC_TIMEOUT))
    } else {
        ctx.bus.dispatch_forced(cmd, force)
    };

    let obs_json = if resp.ok {
        if let Some((opts, include_png)) = observe_after {
            if let Some(snap) = ctx.bus.latest_frame() {
                let mut obs = String::with_capacity(256);
                let _ = write!(
                    obs,
                    "{{\"flip\":{},\"width\":{},\"height\":{},\"frame_hash\":",
                    snap.flip, snap.width, snap.height
                );
                write_json_str(&mut obs, &snap.frame_hash);
                obs.push_str(",\"diff\":");
                write_diff(&mut obs, &snap.diff);
                if include_png {
                    match render_frame_png(&snap, opts, resp.status.pointer_pos) {
                        Ok(png) => {
                            obs.push_str(",\"png_base64\":");
                            write_json_str(&mut obs, &base64_encode(&png));
                        }
                        Err(e) => return HttpResponse::error_json(400, &e),
                    }
                }
                obs.push('}');
                Some(obs)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    control_resp_to_http(&resp, obs_json.as_deref())
}

fn handle_pipe(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let text = match std::str::from_utf8(&req.body) {
        Ok(s) => s,
        Err(_) => return HttpResponse::error_json(400, "body must be valid UTF-8"),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return HttpResponse::error_json(400, "empty pipe payload");
    }

    let relative = req
        .query
        .get("relative")
        .is_none_or(|v| !matches!(v.as_str(), "0" | "false"));
    let sync = req
        .query
        .get("wait")
        .is_none_or(|v| !matches!(v.as_str(), "0" | "false"));
    let auto_step = req
        .query
        .get("auto_step")
        .is_none_or(|v| !matches!(v.as_str(), "0" | "false"));
    let force = parse_bool_param(req.query.get("force"));

    // 1. Raw `#lego-legacy-route 1` text (or `flip ...` route lines).
    let content_type = req
        .headers
        .get("content-type")
        .map(String::as_str)
        .unwrap_or("");
    if content_type.starts_with("text/plain")
        || trimmed.starts_with("#lego-legacy-route")
        || trimmed.starts_with("flip ")
    {
        let cmd = Command::PipeRoute {
            route_text: trimmed.to_owned(),
            relative,
            auto_step,
        };
        let resp = if sync {
            ctx.bus
                .dispatch_sync_forced(cmd, force, Some(DEFAULT_SYNC_TIMEOUT))
        } else {
            ctx.bus.dispatch_forced(cmd, force)
        };
        return control_resp_to_http(&resp, None);
    }

    // 2. Single JSON object `{ route, relative?, auto_step?, wait?, force? }`.
    if let Ok(JsonValue::Object(obj)) = parse_json(trimmed) {
        if let Some(route_text) = obj.get("route").and_then(JsonValue::as_str) {
            let rel = obj
                .get("relative")
                .and_then(JsonValue::as_bool)
                .unwrap_or(relative);
            let step = obj
                .get("auto_step")
                .and_then(JsonValue::as_bool)
                .unwrap_or(auto_step);
            let wait = obj.get("wait").and_then(JsonValue::as_bool).unwrap_or(sync);
            let obj_force = obj
                .get("force")
                .and_then(JsonValue::as_bool)
                .unwrap_or(force);
            let cmd = Command::PipeRoute {
                route_text: route_text.to_owned(),
                relative: rel,
                auto_step: step,
            };
            let resp = if wait {
                ctx.bus
                    .dispatch_sync_forced(cmd, obj_force, Some(DEFAULT_SYNC_TIMEOUT))
            } else {
                ctx.bus.dispatch_forced(cmd, obj_force)
            };
            return control_resp_to_http(&resp, None);
        }
    }

    // 3. JSONL task script (one JSON command per line, executed sequentially).
    let mut executed = 0usize;
    let mut last_resp: Option<Response> = None;
    for (idx, line) in trimmed.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let line_force = parse_json(l)
            .ok()
            .and_then(|v| v.as_object()?.get("force")?.as_bool())
            .unwrap_or(force);
        let parsed = match parse_command_line(l) {
            Ok(p) => p,
            Err(e) => {
                return HttpResponse::error_json(400, &format!("jsonl line {}: {e}", idx + 1));
            }
        };
        executed += 1;
        for cmd in parsed.commands {
            let resp = if parsed.sync && sync {
                ctx.bus
                    .dispatch_sync_forced(cmd, line_force, Some(DEFAULT_SYNC_TIMEOUT))
            } else {
                ctx.bus.dispatch_forced(cmd, line_force)
            };
            let ok = resp.ok;
            last_resp = Some(resp);
            if !ok {
                return control_resp_to_http(last_resp.as_ref().expect("set"), None);
            }
        }
    }
    let resp = last_resp.unwrap_or_else(|| ctx.bus.dispatch(Command::Status));
    let mut out = String::with_capacity(256);
    let _ = write!(
        out,
        "{{\"ok\":{},\"commands_executed\":{},\"status\":",
        resp.ok, executed
    );
    write_status(&mut out, &resp.status);
    out.push('}');
    HttpResponse::json(if resp.ok { 200 } else { 400 }, out)
}

fn handle_step(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let flips = obj
        .get("flips")
        .and_then(JsonValue::as_u64)
        .or_else(|| req.query.get("flips").and_then(|s| s.parse().ok()))
        .unwrap_or(1);
    let sync = obj.get("wait").and_then(JsonValue::as_bool).unwrap_or(true);
    let cmd = Command::Step { flips };
    let resp = if sync {
        ctx.bus.dispatch_sync(cmd, Some(DEFAULT_SYNC_TIMEOUT))
    } else {
        ctx.bus.dispatch(cmd)
    };
    control_resp_to_http(&resp, None)
}

fn handle_wait(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let sync = obj.get("wait").and_then(JsonValue::as_bool).unwrap_or(true);
    let cmd = if let Some(uc) = obj.get("until_change").and_then(JsonValue::as_object) {
        Command::WaitUntilChange {
            max_flips: uc
                .get("max_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(60),
            min_diff_pct: uc
                .get("min_diff_pct")
                .and_then(JsonValue::as_f64)
                .unwrap_or(0.5),
        }
    } else if let Some(us) = obj.get("until_settled").and_then(JsonValue::as_object) {
        Command::WaitUntilSettled {
            max_flips: us
                .get("max_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(120),
            stable_flips: us
                .get("stable_flips")
                .and_then(JsonValue::as_u64)
                .unwrap_or(5),
            max_diff_pct: us
                .get("max_diff_pct")
                .and_then(JsonValue::as_f64)
                .unwrap_or(0.1),
        }
    } else {
        match obj.get("kind").and_then(JsonValue::as_str) {
            Some("change" | "until_change") => Command::WaitUntilChange {
                max_flips: obj
                    .get("max_flips")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(60),
                min_diff_pct: obj
                    .get("min_diff_pct")
                    .and_then(JsonValue::as_f64)
                    .unwrap_or(0.5),
            },
            Some("settled" | "until_settled") => Command::WaitUntilSettled {
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
            },
            _ => Command::Wait {
                flips: obj.get("flips").and_then(JsonValue::as_u64),
                ms: obj.get("ms").and_then(JsonValue::as_u64),
            },
        }
    };
    let resp = if sync {
        ctx.bus.dispatch_sync(cmd, Some(DEFAULT_SYNC_TIMEOUT))
    } else {
        ctx.bus.dispatch(cmd)
    };
    control_resp_to_http(&resp, None)
}

fn handle_mode(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let mut cmds = Vec::new();
    if let Some(m) = obj.get("mode").and_then(JsonValue::as_str) {
        let Some(mode) = ExecMode::parse(m) else {
            return HttpResponse::error_json(
                400,
                &format!("unknown mode `{m}` (expected step|live)"),
            );
        };
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
        let Some(owner) = InputOwner::parse(o) else {
            return HttpResponse::error_json(
                400,
                &format!("unknown owner `{o}` (expected agent|human|shared)"),
            );
        };
        cmds.push(Command::SetOwner(owner));
    }
    if let Some(p) = obj.get("pace").and_then(JsonValue::as_f64) {
        cmds.push(Command::SetPace(p));
    }
    if obj.get("pause").and_then(JsonValue::as_bool) == Some(true)
        || obj.get("paused").and_then(JsonValue::as_bool) == Some(true)
    {
        cmds.push(Command::Pause);
    }
    if obj.get("resume").and_then(JsonValue::as_bool) == Some(true)
        || obj.get("paused").and_then(JsonValue::as_bool) == Some(false)
    {
        cmds.push(Command::Resume);
    }
    if obj.get("clear_queue").and_then(JsonValue::as_bool) == Some(true) {
        cmds.push(Command::ClearQueue);
    }
    if obj.get("close").and_then(JsonValue::as_bool) == Some(true) {
        let force = obj
            .get("force")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        cmds.push(Command::Close { force });
    }
    if cmds.is_empty() {
        return HttpResponse::error_json(
            400,
            "`/api/mode` requires at least one of `mode`, `owner`, `pace`, `pause`, `resume`, `clear_queue`, or `close`",
        );
    }
    let mut last = None;
    for cmd in cmds {
        let r = ctx.bus.dispatch(cmd);
        let ok = r.ok;
        last = Some(r);
        if !ok {
            break;
        }
    }
    let resp = last.unwrap_or_else(|| ctx.bus.dispatch(Command::Status));
    control_resp_to_http(&resp, None)
}

fn handle_list_bookmarks(ctx: &HttpContext) -> HttpResponse {
    let list = ctx.bus.list_bookmarks();
    let mut out = String::with_capacity(128 + list.len() * 128);
    out.push_str("{\"ok\":true,\"bookmarks\":[");
    for (i, (name, bm)) in list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_json_str(&mut out, name);
        let _ = write!(
            out,
            ",\"flip\":{},\"cursor\":{},\"declared_ms\":{},\"digest\":",
            bm.flip, bm.cursor, bm.declared_ms
        );
        write_json_str(&mut out, &bm.digest);
        out.push('}');
    }
    out.push_str("]}");
    HttpResponse::json(200, out)
}

fn handle_bookmark(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let Some(name) = obj.get("name").and_then(JsonValue::as_str) else {
        return HttpResponse::error_json(400, "missing string `name`");
    };
    match ctx
        .bus
        .save_bookmark(name.to_owned(), Some(DEFAULT_SYNC_TIMEOUT))
    {
        Ok(bm) => {
            let mut out = String::with_capacity(256);
            out.push_str("{\"ok\":true,\"name\":");
            write_json_str(&mut out, name.trim());
            let _ = write!(
                out,
                ",\"flip\":{},\"cursor\":{},\"declared_ms\":{},\"digest\":",
                bm.flip, bm.cursor, bm.declared_ms
            );
            write_json_str(&mut out, &bm.digest);
            out.push_str(",\"route_text\":");
            write_json_str(&mut out, &bm.route_text);
            out.push('}');
            HttpResponse::json(200, out)
        }
        Err(e) => HttpResponse::error_json(400, &e),
    }
}

fn handle_rewind(req: &HttpRequest, ctx: &HttpContext) -> HttpResponse {
    let obj = match parse_body_obj(req) {
        Ok(o) => o,
        Err(resp) => return resp,
    };
    let Some(name) = obj.get("name").and_then(JsonValue::as_str) else {
        return HttpResponse::error_json(400, "missing string `name`");
    };
    match ctx
        .bus
        .request_rewind(name.trim(), Some(DEFAULT_SYNC_TIMEOUT))
    {
        Ok(res) => {
            let mut out = String::with_capacity(256);
            out.push_str("{\"ok\":true,\"name\":");
            write_json_str(&mut out, &res.name);
            let _ = write!(
                out,
                ",\"flip\":{},\"cursor\":{},\"declared_ms\":{},\"digest\":",
                res.flip, res.cursor, res.declared_ms
            );
            write_json_str(&mut out, &res.digest);
            out.push_str(",\"expected_digest\":");
            write_json_str(&mut out, &res.expected_digest);
            let _ = write!(out, ",\"digest_match\":{},\"mode\":", res.digest_match);
            write_json_str(&mut out, res.mode);
            if let Some(ms) = res.restore_ms {
                out.push_str(",\"restore_ms\":");
                write_json_f64(&mut out, ms);
            }
            out.push('}');
            HttpResponse::json(200, out)
        }
        Err(e) => HttpResponse::error_json(400, &e),
    }
}

fn control_resp_to_http(resp: &Response, observe_after_json: Option<&str>) -> HttpResponse {
    let mut out = String::with_capacity(384);
    let _ = write!(out, "{{\"ok\":{}", resp.ok);
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
            super::json::write_frame_info(&mut out, f);
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
    if let Some(obs) = observe_after_json {
        out.push_str(",\"observe_after\":");
        out.push_str(obs);
    }
    out.push('}');
    HttpResponse::json(if resp.ok { 200 } else { 400 }, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::control::bus::ControlConfig;
    use crate::control::host::{service_flip_boundary, ControlHost};
    use crate::guest::GuestMem;
    use crate::host::headless::Headless;
    use crate::machine::Machine;
    use crate::route::Route;
    use crate::runlog::RunLog;
    use crate::stub;
    use std::net::TcpStream;

    /// Minimal zero-crate HTTP/1.1 test client over any `Read + Write` stream.
    fn http_call<S: Read + Write>(
        mut stream: S,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> (u16, BTreeMap<String, String>, Vec<u8>) {
        let mut req =
            format!("{method} {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
        for (k, v) in headers {
            let _ = write!(req, "{k}: {v}\r\n");
        }
        if !body.is_empty() || method == "POST" {
            let _ = write!(req, "Content-Length: {}\r\n", body.len());
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).unwrap();
        if !body.is_empty() {
            stream.write_all(body).unwrap();
        }
        stream.flush().unwrap();

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).unwrap();
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("HTTP header separator");
        let head = std::str::from_utf8(&raw[..split]).unwrap();
        let mut lines = head.lines();
        let status_line = lines.next().unwrap();
        let status: u16 = status_line
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let mut resp_headers = BTreeMap::new();
        for line in lines {
            if let Some((k, v)) = line.split_once(':') {
                resp_headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
            }
        }
        (status, resp_headers, raw[split + 4..].to_vec())
    }

    /// Guard that force-closes the bus if a test driver thread panics so
    /// `Machine::run` never stays paused waiting for a dead client.
    struct CloseOnDrop(ControlBus);
    impl Drop for CloseOnDrop {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let _ = self.0.dispatch(Command::Close { force: true });
            }
        }
    }

    fn tcp_call(
        addr: SocketAddr,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> (u16, BTreeMap<String, String>, Vec<u8>) {
        let stream = TcpStream::connect(addr).expect("connect tcp");
        http_call(stream, method, target, headers, body)
    }

    /// Tiny multi-flip stub guest (presents `count` frames, mutating pixel 0 each flip).
    fn multi_flip_stub(count: u32) -> Vec<u8> {
        let mut code = Vec::new();
        code.push(0xb9);
        code.extend_from_slice(&count.to_le_bytes()); // mov ecx, count
        code.extend_from_slice(&[0xb8, 0, 0, 0, 0]); // mov eax, 0
                                                     // loop:
        code.extend_from_slice(&[0xa3, 0x00, 0x00, 0x10, 0x00]); // mov [0x100000], eax
        code.extend_from_slice(&[0xcd, stub::INT_PRESENT]); // int 0x20
        code.push(0x40); // inc eax
        code.push(0x49); // dec ecx
        code.extend_from_slice(&[0x75, (-11i8) as u8]); // jnz loop
        code.extend_from_slice(&[0xcd, stub::INT_EXIT]); // int 0x21
        code
    }

    #[test]
    fn http_server_exercises_all_endpoints_auth_rewind_and_session_registry() {
        let tmp_dir = std::env::temp_dir().join(format!("lego-http-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        let profile_dir = tmp_dir.join("profile");
        let sessions_dir = tmp_dir.join("sessions");
        std::fs::create_dir_all(&profile_dir).unwrap();
        std::fs::write(profile_dir.join("PROFILE.SAV"), b"save-bytes").unwrap();

        let mut control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                title: Title::STUB.key,
                mode: ExecMode::Step,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        )
        .with_deferred_pause(true);
        let bus = control.bus();
        bus.set_machine_attached(true);

        let token = "test-secret-309";
        let ctx = Arc::new(HttpContext::new(
            bus.clone(),
            &Title::STUB,
            Some(token.to_owned()),
        ));
        let mut server = HttpServer::start("127.0.0.1:0", Arc::clone(&ctx)).expect("http start");
        let addr = server.addr();

        // Session registry writes and cleans up on drop.
        let reg_path = {
            let guard = write_session_registry_in(
                &sessions_dir,
                Title::STUB.key,
                &server.url(),
                Some(&server.watch_url()),
                false,
            );
            let p = guard.path().expect("session file").to_path_buf();
            let text = std::fs::read_to_string(&p).unwrap();
            assert!(text.contains(&server.url()));
            assert!(text.contains("\"title\":\"stub\""));
            p
        };
        assert!(!reg_path.exists(), "session registry removed on drop");

        let client_bus = bus.clone();
        let tmp_dir_client = tmp_dir.clone();
        let profile_dir_client = profile_dir.clone();
        let client = thread::spawn(move || {
            let _guard = CloseOnDrop(client_bus.clone());
            let st = client_bus.wait_until_paused(Some(Duration::from_secs(5)));
            assert!(st.paused);
            assert_eq!(st.flip, Some(0));

            let auth = [("Authorization", "Bearer test-secret-309")];

            // 1. Auth enforcement: mutating POST without token returns 401.
            for ep in ["/api/step", "/api/snapshot", "/api/restore"] {
                let (code, _, _) = tcp_call(addr, "POST", ep, &[], b"{}");
                assert_eq!(code, 401, "POST {ep} requires bearer token");
            }

            // 2. GET /api/status, /api/fps, /api/meta, /api/controls, /api/goals
            let (code, _, body) = tcp_call(addr, "GET", "/api/status", &[], &[]);
            assert_eq!(code, 200);
            let st_json = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(st_json.as_object().unwrap()["title"].as_str(), Some("stub"));
            assert_eq!(st_json.as_object().unwrap()["flip"].as_u64(), Some(0));

            let (code, _, body) = tcp_call(addr, "GET", "/api/fps", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body)
                .unwrap()
                .contains("throughput_mips"));

            for ep in ["/api/meta", "/api/controls", "/api/goals"] {
                let (code, _, body) = tcp_call(addr, "GET", ep, &[], &[]);
                assert_eq!(code, 200, "GET {ep}");
                let j = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
                assert_eq!(j.as_object().unwrap()["ok"].as_bool(), Some(true));
            }

            // 3. GET /api/frame.png (grid + cursor + crop) & /api/frame.json
            let (code, hdrs, body) = tcp_call(
                addr,
                "GET",
                "/api/frame.png?grid=1&cursor=1&crop=0,0,4,4",
                &[],
                &[],
            );
            assert_eq!(code, 200);
            assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n");
            assert_eq!(hdrs.get("x-flip").map(String::as_str), Some("0"));
            assert!(hdrs.contains_key("x-cursor"));
            assert!(hdrs.contains_key("x-frame-hash"));
            assert!(hdrs.contains_key("x-diff-pct"));

            let (code, _, body) = tcp_call(addr, "GET", "/api/frame.json?grid=4x4", &[], &[]);
            assert_eq!(code, 200);
            let fj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(fj.as_object().unwrap()["flip"].as_u64(), Some(0));

            // 4. Read-only /api/mem, /api/mem/map, /api/mem/search, /api/cpu, /api/win32, /api/files
            let (code, _, body) = tcp_call(
                addr,
                "GET",
                "/api/mem?addr=0x00100000&len=4&fmt=u32",
                &[],
                &[],
            );
            assert_eq!(code, 200);
            let mj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(
                mj.as_object().unwrap()["values"].as_array().unwrap()[0].as_u64(),
                Some(0)
            );

            let (code, _, body) = tcp_call(addr, "GET", "/api/mem/map", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body).unwrap().contains("\"heap\""));

            let (code, _, body) = tcp_call(
                addr,
                "POST",
                "/api/mem/search",
                &[],
                br#"{"ascii":"CAFE","start":"0x00100100","end":"0x00100200"}"#,
            );
            assert_eq!(code, 200);
            let sj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(sj.as_object().unwrap()["count"].as_u64(), Some(1));

            let (code, _, body) = tcp_call(addr, "GET", "/api/cpu", &[], &[]);
            assert_eq!(code, 200);
            let cj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert!(cj.as_object().unwrap()["x87"].as_object().is_some());
            assert!(cj.as_object().unwrap()["state_digest"].as_str().is_some());

            let (code, _, body) = tcp_call(addr, "GET", "/api/win32", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body).unwrap().contains("\"ddraw\""));

            let (code, _, body) = tcp_call(addr, "GET", "/api/files?root=profile&path=", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body).unwrap().contains("PROFILE.SAV"));

            let (code, hdrs, body) = tcp_call(
                addr,
                "GET",
                "/api/files/read?root=profile&path=PROFILE.SAV",
                &[],
                &[],
            );
            assert_eq!(code, 200);
            assert_eq!(body, b"save-bytes");
            assert!(hdrs.contains_key("x-sha256"));

            // 5. GET/POST /api/agent-state
            let (code, _, _) = tcp_call(
                addr,
                "POST",
                "/api/agent-state",
                &auth,
                br#"{"goal":"verify http","thought":"paused at flip 0"}"#,
            );
            assert_eq!(code, 200);
            let (code, _, body) = tcp_call(addr, "GET", "/api/agent-state", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body).unwrap().contains("verify http"));

            // 6. POST /api/act with observe_after (include_png: true) -> advances to flip 2
            let (code, _, body) = tcp_call(
                addr,
                "POST",
                "/api/act",
                &auth,
                br#"{"action":"click","x":12,"y":14,"hold_flips":1,"observe_after":{"grid":true,"cursor":true,"include_png":true}}"#,
            );
            assert_eq!(code, 200);
            let aj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(aj.as_object().unwrap()["ok"].as_bool(), Some(true));
            assert!(aj.as_object().unwrap()["observe_after"]
                .as_object()
                .unwrap()["png_base64"]
                .as_str()
                .is_some_and(|s| !s.is_empty()));

            // 7. POST /api/bookmark at flip 2
            let (code, _, body) =
                tcp_call(addr, "POST", "/api/bookmark", &auth, br#"{"name":"at2"}"#);
            assert_eq!(code, 200);
            let bmj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            let bm_flip = bmj.as_object().unwrap()["flip"].as_u64().unwrap();
            let bm_digest = bmj.as_object().unwrap()["digest"]
                .as_str()
                .unwrap()
                .to_owned();
            assert_eq!(bm_flip, 2);

            // 8. POST /api/pipe (raw route) + POST /api/step + POST /api/wait -> advance to flip 5
            let (code, _, _) = tcp_call(
                addr,
                "POST",
                "/api/pipe",
                &[
                    ("Authorization", "Bearer test-secret-309"),
                    ("Content-Type", "text/plain"),
                ],
                b"#lego-legacy-route 1\nflip 0 key Space down\nflip 1 key Space up\n",
            );
            assert_eq!(code, 200);

            let (code, _, _) = tcp_call(addr, "POST", "/api/wait", &auth, br#"{"flips":1}"#);
            assert_eq!(code, 200);

            let (code, _, body) = tcp_call(addr, "GET", "/api/route", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body)
                .unwrap()
                .starts_with("#lego-legacy-route 1"));

            // 9. POST /api/rewind back to "at2" -> verifies instant LLSNAP v2 + exact digest_match == true!
            let (code, _, body) =
                tcp_call(addr, "POST", "/api/rewind", &auth, br#"{"name":"at2"}"#);
            assert_eq!(code, 200);
            let rwj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(rwj.as_object().unwrap()["flip"].as_u64(), Some(2));
            assert_eq!(
                rwj.as_object().unwrap()["digest"].as_str(),
                Some(bm_digest.as_str())
            );
            assert_eq!(
                rwj.as_object().unwrap()["digest_match"].as_bool(),
                Some(true)
            );
            assert_eq!(rwj.as_object().unwrap()["mode"].as_str(), Some("llsnap_v2"));

            // 9b. POST /api/snapshot (slot + file), GET /api/snapshot, mutate profile + advance,
            // then POST /api/restore (slot AND file) and verify bit-exact rollback.
            let snap_file = tmp_dir_client.join("slot.snapshot");
            let snap_req = format!(
                "{{\"name\":\"slot2\",\"path\":\"{}\"}}",
                snap_file.display().to_string().replace('\\', "\\\\")
            );
            let (code, _, body) =
                tcp_call(addr, "POST", "/api/snapshot", &auth, snap_req.as_bytes());
            assert_eq!(code, 200);
            let spj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(spj.as_object().unwrap()["flip"].as_u64(), Some(2));
            assert_eq!(
                spj.as_object().unwrap()["digest"].as_str(),
                Some(bm_digest.as_str())
            );
            assert!(snap_file.is_file());

            let (code, _, body) = tcp_call(addr, "GET", "/api/snapshot", &[], &[]);
            assert_eq!(code, 200);
            assert!(std::str::from_utf8(&body).unwrap().contains("\"slot2\""));

            std::fs::write(profile_dir_client.join("PROFILE.SAV"), b"mutated").unwrap();
            let (code, _, _) = tcp_call(addr, "POST", "/api/step", &auth, br#"{"flips":2}"#);
            assert_eq!(code, 200);

            let (code, _, body) =
                tcp_call(addr, "POST", "/api/restore", &auth, br#"{"name":"slot2"}"#);
            assert_eq!(code, 200);
            let rsj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(rsj.as_object().unwrap()["flip"].as_u64(), Some(2));
            assert_eq!(
                rsj.as_object().unwrap()["digest"].as_str(),
                Some(bm_digest.as_str())
            );
            assert_eq!(
                rsj.as_object().unwrap()["digest_match"].as_bool(),
                Some(true)
            );
            assert_eq!(
                std::fs::read(profile_dir_client.join("PROFILE.SAV")).unwrap(),
                b"save-bytes"
            );

            // Also verify disk file restore via `path`.
            let (code, _, _) = tcp_call(addr, "POST", "/api/step", &auth, br#"{"flips":1}"#);
            assert_eq!(code, 200);
            let restore_req = format!(
                "{{\"path\":\"{}\"}}",
                snap_file.display().to_string().replace('\\', "\\\\")
            );
            let (code, _, body) =
                tcp_call(addr, "POST", "/api/restore", &auth, restore_req.as_bytes());
            assert_eq!(code, 200);
            let rpj = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(rpj.as_object().unwrap()["flip"].as_u64(), Some(2));
            assert_eq!(
                rpj.as_object().unwrap()["digest"].as_str(),
                Some(bm_digest.as_str())
            );

            // 10. Switch to Live mode via /api/mode to finish the run cleanly.
            let (code, _, _) = tcp_call(addr, "POST", "/api/mode", &auth, br#"{"mode":"live"}"#);
            assert_eq!(code, 200);
        });

        // Drive machine with outer Tier-1 rewind loop.
        let mut initial_route: Option<Route> = None;
        let prog = multi_flip_stub(8);
        let mut log = RunLog::create("http-e2e-stub", crate::cli::Trace::Off).unwrap();
        loop {
            let mut machine = Machine::new(&Title::STUB, &mut control).unwrap();
            machine.load_stub().unwrap();
            machine.arena.write(stub::ENTRY, &prog).unwrap();
            machine.arena.write(0x0010_0120, b"CAFE").unwrap();
            machine.tree.set_profile(profile_dir.clone());
            machine.recording = Some(Route::new());
            if let Some(route) = initial_route.take() {
                machine.route = Some(route);
            }
            let obs_bus = bus.clone();
            machine.set_flip_observer(Box::new(move |m, h| {
                service_flip_boundary(&obs_bus, m, h);
            }));
            let finish = machine.run(&mut control, &mut log, 200_000);
            if let Some((_name, bm)) = bus.take_pending_rewind() {
                control.reset_for_rewind();
                initial_route = Some(Route::parse(&bm.route_text).expect("parse bookmark route"));
                continue;
            }
            let digest = machine.state_digest();
            control.mark_finished(&finish, &digest);
            break;
        }

        client.join().unwrap();
        server.stop();
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn polling_all_readonly_endpoints_preserves_bit_exact_determinism() {
        // 1. Baseline unpolled run on pure Headless.
        let prog = multi_flip_stub(6);
        let mut base_host = Headless::new();
        let mut base_machine = Machine::new(&Title::STUB, &mut base_host).unwrap();
        base_machine.load_stub().unwrap();
        base_machine.arena.write(stub::ENTRY, &prog).unwrap();
        let mut base_log = RunLog::create("http-det-base", crate::cli::Trace::Off).unwrap();
        let base_finish = base_machine.run(&mut base_host, &mut base_log, 200_000);
        let base_digest = base_machine.state_digest();

        // 2. Polled run hammering /api/mem, /api/cpu, /api/win32, /api/files, /api/frame.png at every flip.
        let mut control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                title: Title::STUB.key,
                mode: ExecMode::Step,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        )
        .with_deferred_pause(true);
        let bus = control.bus();
        bus.set_machine_attached(true);
        let ctx = Arc::new(HttpContext::new(bus.clone(), &Title::STUB, None));
        let mut server = HttpServer::start("127.0.0.1:0", ctx).unwrap();
        let addr = server.addr();

        let poller_bus = bus.clone();
        let poller = thread::spawn(move || {
            let _guard = CloseOnDrop(poller_bus.clone());
            for expected_flip in 0..6u64 {
                let st = poller_bus.wait_until_paused(Some(Duration::from_secs(5)));
                assert_eq!(st.flip, Some(expected_flip));
                for ep in [
                    "/api/mem?addr=0x00100000&len=16&fmt=hex",
                    "/api/mem/map",
                    "/api/cpu",
                    "/api/win32",
                    "/api/files?root=disc&path=",
                    "/api/frame.png?grid=1&cursor=1",
                    "/api/frame.json?grid=8x6",
                    "/api/route",
                ] {
                    let (code, _, _) = tcp_call(addr, "GET", ep, &[], &[]);
                    assert_eq!(code, 200, "flip {expected_flip} GET {ep}");
                }
                let (code, _, _) = tcp_call(addr, "POST", "/api/step", &[], br#"{"flips":1}"#);
                assert_eq!(code, 200);
            }
        });

        let mut polled_machine = Machine::new(&Title::STUB, &mut control).unwrap();
        polled_machine.load_stub().unwrap();
        polled_machine.arena.write(stub::ENTRY, &prog).unwrap();
        polled_machine.recording = Some(Route::new());
        let obs_bus = bus.clone();
        polled_machine.set_flip_observer(Box::new(move |m, h| {
            service_flip_boundary(&obs_bus, m, h);
        }));
        let mut polled_log = RunLog::create("http-det-polled", crate::cli::Trace::Off).unwrap();
        let polled_finish = polled_machine.run(&mut control, &mut polled_log, 200_000);
        let polled_digest = polled_machine.state_digest();
        control.mark_finished(&polled_finish, &polled_digest);
        poller.join().unwrap();
        server.stop();

        assert_eq!(polled_finish, base_finish);
        assert_eq!(polled_digest, base_digest);
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_serves_same_http_handler_and_cleans_up_socket() {
        use crate::control::unix::UnixServer;
        use std::os::unix::net::UnixStream;

        let sock_path = std::env::temp_dir().join(format!(
            "lego-unix-test-{}-{}.sock",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        let bus = ControlBus::new(ControlConfig {
            title: Title::STUB.key,
            ..ControlConfig::default()
        });
        let ctx = Arc::new(HttpContext::new(bus, &Title::STUB, None));
        {
            let _server = UnixServer::start(sock_path.clone(), ctx).expect("unix server");
            let stream = UnixStream::connect(&sock_path).expect("connect unix");
            let (code, _, body) = http_call(stream, "GET", "/api/status", &[], &[]);
            assert_eq!(code, 200);
            let j = parse_json(std::str::from_utf8(&body).unwrap()).unwrap();
            assert_eq!(j.as_object().unwrap()["title"].as_str(), Some("stub"));
        }
        assert!(!sock_path.exists(), "unix socket cleaned up on drop");
    }

    #[test]
    fn run_digested_with_stub_and_control_http_runs_cleanly() {
        let cli = Cli::parse([
            "--headless",
            "--stub",
            "--control",
            "127.0.0.1:0",
            "--control-mode",
            "live",
        ])
        .unwrap();
        let mut host = Headless::new();
        let (finish, _log, digest) =
            crate::run::run_digested(&Title::STUB, &cli, &mut host, 100_000).expect("run");
        assert_eq!(finish.flips, 1);
        assert!(!digest.is_empty());
    }

    #[test]
    fn extra_routes_hook_and_jsonl_pipe_dispatch_cleanly() {
        struct StubExtra;
        impl ExtraRoutes for StubExtra {
            fn handle(
                &self,
                req: &HttpRequest,
                _ctx: &HttpContext,
                stream: &mut dyn Write,
            ) -> io::Result<bool> {
                if req.method == "GET" && req.path == "/watch" {
                    HttpResponse::text(200, "watch-ok".to_owned()).write_to(stream)?;
                    return Ok(true);
                }
                Ok(false)
            }
        }

        let bus = ControlBus::new(ControlConfig {
            title: Title::STUB.key,
            mode: ExecMode::Live,
            ..ControlConfig::default()
        });
        let ctx = HttpContext::new(bus, &Title::STUB, None).with_extra(Arc::new(StubExtra));

        // 1. ExtraRoutes handles `/watch` via handle_connection without TCP.
        struct MemStream {
            input: io::Cursor<Vec<u8>>,
            output: Vec<u8>,
        }
        impl Read for MemStream {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                self.input.read(buf)
            }
        }
        impl Write for MemStream {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.output.write(buf)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut mem = MemStream {
            input: io::Cursor::new(b"GET /watch HTTP/1.1\r\nHost: x\r\n\r\n".to_vec()),
            output: Vec::new(),
        };
        handle_connection(&mut mem, &ctx);
        let text = String::from_utf8(mem.output).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK"));
        assert!(text.ends_with("watch-ok"));

        // 2. JSONL task script via POST /api/pipe schedules cleanly.
        let jsonl = "{\"cmd\":\"move\",\"x\":40,\"y\":50,\"queue\":true}\n{\"cmd\":\"key_hold\",\"key\":\"Space\",\"hold_flips\":2,\"queue\":true}\n";
        let req = HttpRequest {
            method: "POST".to_owned(),
            path: "/api/pipe".to_owned(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: jsonl.as_bytes().to_vec(),
        };
        let resp = dispatch_http(&req, &ctx);
        assert_eq!(resp.status, 200);
        let body = parse_json(std::str::from_utf8(&resp.body).unwrap()).unwrap();
        assert_eq!(
            body.as_object().unwrap()["commands_executed"].as_u64(),
            Some(2)
        );
        assert!(ctx.bus.status().pending_actions >= 3);
    }

    #[test]
    fn http_serves_title_guide_and_enforces_escape_quits_guard() {
        use crate::title::{AgentCadence, ControlHint, Goal, Milestone, MouseMode};

        const GUARDED_GUIDE: TitleGuide = TitleGuide {
            display_name: "Guarded Title",
            summary: "Escape immediately quits.",
            resolution: (640, 480),
            bpp: 8,
            refresh_hz: 60,
            mouse_mode: MouseMode::AbsoluteKept,
            escape_quits: true,
            skip_keys: &["Space"],
            cadence: AgentCadence::MicroStep { flips_per_step: 8 },
            controls: &[ControlHint {
                input: "Up",
                action: "Throttle",
                notes: "Hold on green.",
            }],
            hazards: &["Escape quits immediately."],
            goals: &[Goal {
                id: "first-run",
                name: "First Run",
                milestone: Milestone::Playable,
                description: "Finish the first course.",
            }],
            save_files: &["saves\\times.dat"],
        };

        let bus = ControlBus::new(ControlConfig {
            title: Title::STUB.key,
            mode: ExecMode::Live,
            guide: &GUARDED_GUIDE,
            ..ControlConfig::default()
        });
        let ctx = HttpContext::new(bus, &Title::STUB, None);

        // GET /api/meta, /api/controls, /api/goals serve the populated TitleGuide.
        let meta_resp = dispatch_http(
            &HttpRequest {
                method: "GET".to_owned(),
                path: "/api/meta".to_owned(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
            &ctx,
        );
        assert_eq!(meta_resp.status, 200);
        let mj = parse_json(std::str::from_utf8(&meta_resp.body).unwrap()).unwrap();
        let g = mj.as_object().unwrap()["guide"].as_object().unwrap();
        assert_eq!(g["display_name"].as_str(), Some("Guarded Title"));
        assert_eq!(g["bpp"].as_u64(), Some(8));
        assert_eq!(g["escape_quits"].as_bool(), Some(true));

        let ctrl_resp = dispatch_http(
            &HttpRequest {
                method: "GET".to_owned(),
                path: "/api/controls".to_owned(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
            &ctx,
        );
        assert_eq!(ctrl_resp.status, 200);
        let cj = parse_json(std::str::from_utf8(&ctrl_resp.body).unwrap()).unwrap();
        assert_eq!(
            cj.as_object().unwrap()["escape_quits"].as_bool(),
            Some(true)
        );
        assert_eq!(
            cj.as_object().unwrap()["controls"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let goals_resp = dispatch_http(
            &HttpRequest {
                method: "GET".to_owned(),
                path: "/api/goals".to_owned(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: Vec::new(),
            },
            &ctx,
        );
        assert_eq!(goals_resp.status, 200);
        let gj = parse_json(std::str::from_utf8(&goals_resp.body).unwrap()).unwrap();
        assert_eq!(
            gj.as_object().unwrap()["goals"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            gj.as_object().unwrap()["save_files"].as_array().unwrap()[0].as_str(),
            Some("saves\\times.dat")
        );

        // POST /api/act with Escape is rejected (400) without force: true.
        let blocked = dispatch_http(
            &HttpRequest {
                method: "POST".to_owned(),
                path: "/api/act".to_owned(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: br#"{"action":"key_hold","key":"Escape","queue":true,"wait":false}"#.to_vec(),
            },
            &ctx,
        );
        assert_eq!(blocked.status, 400);
        assert!(std::str::from_utf8(&blocked.body)
            .unwrap()
            .contains("force"));

        // POST /api/act with Escape + force: true succeeds (200).
        let forced = dispatch_http(
            &HttpRequest {
                method: "POST".to_owned(),
                path: "/api/act".to_owned(),
                query: BTreeMap::new(),
                headers: BTreeMap::new(),
                body: br#"{"action":"key_hold","key":"Escape","force":true,"queue":true,"wait":false}"#
                    .to_vec(),
            },
            &ctx,
        );
        assert_eq!(forced.status, 200);

        // POST /api/pipe route with Escape is rejected without force, allowed with ?force=1.
        let pipe_blocked = dispatch_http(
            &HttpRequest {
                method: "POST".to_owned(),
                path: "/api/pipe".to_owned(),
                query: BTreeMap::from([("wait".to_owned(), "0".to_owned())]),
                headers: BTreeMap::from([("content-type".to_owned(), "text/plain".to_owned())]),
                body: b"#lego-legacy-route 1\nflip 0 key Escape down\n".to_vec(),
            },
            &ctx,
        );
        assert_eq!(pipe_blocked.status, 400);

        let pipe_forced = dispatch_http(
            &HttpRequest {
                method: "POST".to_owned(),
                path: "/api/pipe".to_owned(),
                query: BTreeMap::from([
                    ("wait".to_owned(), "0".to_owned()),
                    ("force".to_owned(), "1".to_owned()),
                ]),
                headers: BTreeMap::from([("content-type".to_owned(), "text/plain".to_owned())]),
                body: b"#lego-legacy-route 1\nflip 0 key Escape down\n".to_vec(),
            },
            &ctx,
        );
        assert_eq!(pipe_forced.status, 200);
    }
}
