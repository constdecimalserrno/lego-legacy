//! Zero-crate HTTP/1.1 client over TCP (`TcpStream`) and Unix domain sockets
//! (`UnixStream`) (#312).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

/// Default connect timeout for control requests.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default read/write timeout (exceeds the server's 30s synchronous flip timeout).
const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(45);

/// Parsed endpoint address (`http://host:port` or `unix:///path/to.sock`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Tcp {
        host: String,
        port: u16,
        base_path: String,
    },
    #[cfg(unix)]
    Unix { path: PathBuf },
}

impl Endpoint {
    /// Parse an HTTP or Unix socket control URL.
    ///
    /// # Errors
    /// Returns a descriptive error on unsupported schemes or invalid addresses.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return Err("empty control URL".to_owned());
        }
        if let Some(rest) = trimmed.strip_prefix("unix://") {
            #[cfg(unix)]
            {
                if rest.is_empty() {
                    return Err("empty unix socket path".to_owned());
                }
                return Ok(Self::Unix {
                    path: PathBuf::from(rest),
                });
            }
            #[cfg(not(unix))]
            {
                let _ = rest;
                return Err("unix:// control URLs are only supported on Unix hosts".to_owned());
            }
        }
        if trimmed.starts_with("https://") {
            return Err("https:// is not supported by the zero-crate control client; use http:// or unix://".to_owned());
        }
        let without_scheme = trimmed.strip_prefix("http://").unwrap_or(trimmed);
        let (authority, path_part) = match without_scheme.find('/') {
            Some(idx) => (&without_scheme[..idx], &without_scheme[idx..]),
            None => (without_scheme, ""),
        };
        if authority.is_empty() {
            return Err(format!("missing host in URL `{raw}`"));
        }

        let (host, port) = parse_host_port(authority)?;
        Ok(Self::Tcp {
            host,
            port,
            base_path: path_part.trim_end_matches('/').to_owned(),
        })
    }

    /// Canonical control URL string.
    #[must_use]
    pub fn url(&self) -> String {
        match self {
            Self::Tcp {
                host,
                port,
                base_path,
            } => {
                if host.contains(':') {
                    format!("http://[{host}]:{port}{base_path}")
                } else {
                    format!("http://{host}:{port}{base_path}")
                }
            }
            #[cfg(unix)]
            Self::Unix { path } => format!("unix://{}", path.display()),
        }
    }

    /// Browser-openable `/watch` URL for TCP endpoints, or `None` for Unix sockets.
    #[must_use]
    pub fn watch_url(&self) -> Option<String> {
        match self {
            Self::Tcp { .. } => Some(format!("{}/watch", self.url())),
            #[cfg(unix)]
            Self::Unix { .. } => None,
        }
    }

    /// Fast socket connect check used for liveness probing.
    #[must_use]
    pub fn is_reachable(&self, timeout: Duration) -> bool {
        match self {
            Self::Tcp { host, port, .. } => connect_tcp(host, *port, timeout).is_ok(),
            #[cfg(unix)]
            Self::Unix { path } => std::os::unix::net::UnixStream::connect(path).is_ok(),
        }
    }
}

fn parse_host_port(authority: &str) -> Result<(String, u16), String> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest
            .split_once(']')
            .ok_or_else(|| format!("unterminated IPv6 host `{authority}`"))?;
        let port = match after {
            "" => 80,
            s => s
                .strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .ok_or_else(|| format!("invalid port in `{authority}`"))?,
        };
        return Ok((host.to_owned(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port_str)) if !host.is_empty() => {
            let port = port_str
                .parse::<u16>()
                .map_err(|_| format!("invalid port `{port_str}` in `{authority}`"))?;
            Ok((host.to_owned(), port))
        }
        _ => Ok((authority.to_owned(), 80)),
    }
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> io::Result<TcpStream> {
    let connect_host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    let addrs = (connect_host, port).to_socket_addrs()?;
    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable, "no resolved address")))
}

/// Parsed HTTP/1.1 response.
#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    #[must_use]
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Configured zero-crate HTTP/1.1 control client.
#[derive(Clone, Debug)]
pub struct Client {
    pub endpoint: Endpoint,
    pub token: Option<String>,
    pub io_timeout: Duration,
}

impl Client {
    /// Create a client targeting `url` with optional Bearer `token`.
    ///
    /// # Errors
    /// Returns an error if `url` is malformed.
    pub fn new(url: &str, token: Option<String>) -> Result<Self, String> {
        Ok(Self {
            endpoint: Endpoint::parse(url)?,
            token: token.filter(|t| !t.is_empty()),
            io_timeout: DEFAULT_IO_TIMEOUT,
        })
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.io_timeout = timeout;
        self
    }

    #[must_use]
    pub fn url(&self) -> String {
        self.endpoint.url()
    }

    #[must_use]
    pub fn watch_url(&self) -> Option<String> {
        self.endpoint.watch_url()
    }

    /// Perform an HTTP `GET` request.
    ///
    /// # Errors
    /// Returns connection, I/O, or HTTP framing errors.
    pub fn get(&self, target: &str) -> Result<HttpResponse, String> {
        self.request("GET", target, None, &[])
    }

    /// Perform an HTTP `POST` request with a JSON payload.
    ///
    /// # Errors
    /// Returns connection, I/O, or HTTP framing errors.
    pub fn post_json(&self, target: &str, json_body: &str) -> Result<HttpResponse, String> {
        self.request(
            "POST",
            target,
            Some("application/json; charset=utf-8"),
            json_body.as_bytes(),
        )
    }

    /// Perform an HTTP `POST` request with arbitrary content type and bytes.
    ///
    /// # Errors
    /// Returns connection, I/O, or HTTP framing errors.
    pub fn post_raw(
        &self,
        target: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<HttpResponse, String> {
        self.request("POST", target, Some(content_type), body)
    }

    /// Send an HTTP/1.1 request and parse the response.
    ///
    /// # Errors
    /// Returns connection, I/O, or HTTP framing errors.
    pub fn request(
        &self,
        method: &str,
        target: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<HttpResponse, String> {
        let normalized_target = if target.starts_with('/') {
            target.to_owned()
        } else {
            format!("/{target}")
        };
        match &self.endpoint {
            Endpoint::Tcp {
                host,
                port,
                base_path,
            } => {
                let full_target = format!("{base_path}{normalized_target}");
                let host_header = if host.contains(':') {
                    format!("[{host}]:{port}")
                } else {
                    format!("{host}:{port}")
                };
                let mut stream = connect_tcp(host, *port, DEFAULT_CONNECT_TIMEOUT)
                    .map_err(|e| format!("failed to connect to {}: {e}", self.endpoint.url()))?;
                let _ = stream.set_read_timeout(Some(self.io_timeout));
                let _ = stream.set_write_timeout(Some(self.io_timeout));
                exchange_http(
                    &mut stream,
                    method,
                    &full_target,
                    &host_header,
                    self.token.as_deref(),
                    content_type,
                    body,
                )
            }
            #[cfg(unix)]
            Endpoint::Unix { path } => {
                let mut stream = std::os::unix::net::UnixStream::connect(path)
                    .map_err(|e| format!("failed to connect to {}: {e}", path.display()))?;
                let _ = stream.set_read_timeout(Some(self.io_timeout));
                let _ = stream.set_write_timeout(Some(self.io_timeout));
                exchange_http(
                    &mut stream,
                    method,
                    &normalized_target,
                    "localhost",
                    self.token.as_deref(),
                    content_type,
                    body,
                )
            }
        }
    }
}

/// Write an HTTP/1.1 request to `stream` and read the response (handling
/// `Content-Length`, `Transfer-Encoding: chunked`, and EOF-delimited bodies).
///
/// # Errors
/// Returns an error on write, read, or malformed HTTP response framing.
pub fn exchange_http<S: Read + Write>(
    stream: &mut S,
    method: &str,
    target: &str,
    host: &str,
    token: Option<&str>,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<HttpResponse, String> {
    let mut head = String::with_capacity(256);
    let _ = write!(
        head,
        "{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n"
    );
    if let Some(tok) = token {
        let _ = write!(head, "Authorization: Bearer {tok}\r\n");
    }
    if let Some(ct) = content_type {
        let _ = write!(head, "Content-Type: {ct}\r\n");
    }
    if !body.is_empty() || method == "POST" {
        let _ = write!(head, "Content-Length: {}\r\n", body.len());
    }
    head.push_str("\r\n");

    stream
        .write_all(head.as_bytes())
        .map_err(|e| format!("failed to write HTTP headers: {e}"))?;
    if !body.is_empty() {
        stream
            .write_all(body)
            .map_err(|e| format!("failed to write HTTP body: {e}"))?;
    }
    stream
        .flush()
        .map_err(|e| format!("failed to flush HTTP request: {e}"))?;

    read_http_response(stream)
}

/// Parse an HTTP/1.1 response from any `Read` stream.
///
/// # Errors
/// Returns an error on truncated responses, invalid status lines, or malformed chunks.
pub fn read_http_response<R: Read>(stream: R) -> Result<HttpResponse, String> {
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    let n = reader
        .read_line(&mut status_line)
        .map_err(|e| format!("failed to read HTTP status line: {e}"))?;
    if n == 0 {
        return Err("empty HTTP response (connection closed)".to_owned());
    }
    let trimmed = status_line.trim_end_matches(['\r', '\n']);
    let mut parts = trimmed.splitn(3, ' ');
    let _version = parts
        .next()
        .filter(|v| v.starts_with("HTTP/"))
        .ok_or_else(|| format!("malformed HTTP status line `{trimmed}`"))?;
    let status: u16 = parts
        .next()
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| format!("malformed HTTP status code in `{trimmed}`"))?;
    let reason = parts.next().unwrap_or("").to_owned();

    let mut headers = BTreeMap::new();
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read HTTP header: {e}"))?;
        if read == 0 {
            break;
        }
        let h = line.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }

    let is_chunked = headers
        .get("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));

    let body = if matches!(status, 204 | 304) {
        Vec::new()
    } else if is_chunked {
        read_chunked_body(&mut reader)?
    } else if let Some(cl) = headers.get("content-length") {
        let len: usize = cl
            .trim()
            .parse()
            .map_err(|_| format!("invalid Content-Length `{cl}`"))?;
        let mut buf = vec![0u8; len];
        reader
            .read_exact(&mut buf)
            .map_err(|e| format!("failed to read HTTP body ({len} bytes): {e}"))?;
        buf
    } else {
        let mut buf = Vec::new();
        reader
            .read_to_end(&mut buf)
            .map_err(|e| format!("failed to read HTTP body: {e}"))?;
        buf
    };

    Ok(HttpResponse {
        status,
        reason,
        headers,
        body,
    })
}

fn read_chunked_body<R: BufRead>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read chunk size: {e}"))?;
        if n == 0 {
            return Err("unexpected EOF in chunked body".to_owned());
        }
        let size_token = line
            .trim_end_matches(['\r', '\n'])
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size = usize::from_str_radix(size_token, 16)
            .map_err(|_| format!("invalid chunk size `{size_token}`"))?;
        if size == 0 {
            // Drain optional trailer lines until the terminating empty line.
            loop {
                line.clear();
                let t = reader
                    .read_line(&mut line)
                    .map_err(|e| format!("failed to read chunk trailer: {e}"))?;
                if t == 0 || line.trim_end_matches(['\r', '\n']).is_empty() {
                    break;
                }
            }
            break;
        }
        let start = out.len();
        out.resize(start + size, 0);
        reader
            .read_exact(&mut out[start..start + size])
            .map_err(|e| format!("failed to read chunk payload ({size} bytes): {e}"))?;
        // Consume trailing CRLF after chunk payload.
        line.clear();
        let _ = reader
            .read_line(&mut line)
            .map_err(|e| format!("failed to read chunk terminator: {e}"))?;
    }
    Ok(out)
}
