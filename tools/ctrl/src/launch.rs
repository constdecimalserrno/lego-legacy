//! Title binary discovery and `--control` launch helper (#312).

use crate::http::Client;
use crate::json::write_json_str;
use crate::sessions::{parse_session_json, resolve_token, sessions_dir};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Options for launching a title binary with an active HTTP control endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchOptions {
    pub title: String,
    pub windowed: bool,
    pub bind: Option<String>,
    pub data_root: Option<String>,
    pub route: Option<String>,
    pub mode: Option<String>,
    pub token: Option<String>,
    pub stub: bool,
    pub timeout: Duration,
}

impl LaunchOptions {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            windowed: false,
            bind: None,
            data_root: None,
            route: None,
            mode: None,
            token: None,
            stub: false,
            timeout: Duration::from_secs(10),
        }
    }
}

/// Information returned once a launched title's control server is ready.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchResult {
    pub pid: u32,
    pub title: String,
    pub control_url: String,
    pub watch_url: Option<String>,
}

impl LaunchResult {
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(200);
        let _ = write!(out, "{{\"pid\":{},\"title\":", self.pid);
        write_json_str(&mut out, &self.title);
        out.push_str(",\"control_url\":");
        write_json_str(&mut out, &self.control_url);
        out.push_str(",\"watch_url\":");
        match &self.watch_url {
            Some(w) => write_json_str(&mut out, w),
            None => out.push_str("null"),
        }
        out.push('}');
        out
    }
}

/// Resolve `<title>` binary from:
/// 1. Sibling of the current executable (`exe_dir/<title>`)
/// 2. `CARGO_TARGET_DIR/release/<title>` or `./target/release/<title>`
/// 3. `./play-kit/<title>` (or parent `play-kit/<title>`)
/// 4. Plain `<title>` on `PATH`
#[must_use]
pub fn resolve_title_binary(title: &str) -> PathBuf {
    let bin_name = format!("{title}{}", std::env::consts::EXE_SUFFIX);

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join(&bin_name);
            if beside.is_file() {
                return beside;
            }
            if let Some(parent) = dir.parent() {
                let kit = parent.join("play-kit").join(&bin_name);
                if kit.is_file() {
                    return kit;
                }
            }
        }
    }

    if let Some(target_dir) = std::env::var_os("CARGO_TARGET_DIR") {
        let cand = PathBuf::from(target_dir).join("release").join(&bin_name);
        if cand.is_file() {
            return cand;
        }
    }

    for rel in [
        Path::new("target").join("release").join(&bin_name),
        Path::new("play-kit").join(&bin_name),
    ] {
        if rel.is_file() {
            return rel;
        }
    }

    PathBuf::from(bin_name)
}

/// Spawn a title binary with `--control` and wait up to `opts.timeout` for its
/// session file / HTTP endpoint to answer `GET /api/status`.
///
/// # Errors
/// Returns an error if the binary cannot be spawned, exits early, or does not
/// expose a reachable control server before `opts.timeout`.
pub fn launch_title(opts: &LaunchOptions) -> Result<LaunchResult, String> {
    let program = resolve_title_binary(&opts.title);
    launch_program(&program, opts)
}

/// Spawn a specific `program` path with `opts` and wait for control readiness.
///
/// # Errors
/// Returns an error if spawning fails, the child exits early, or readiness times out.
pub fn launch_program(program: &Path, opts: &LaunchOptions) -> Result<LaunchResult, String> {
    let bind = opts.bind.as_deref().unwrap_or("127.0.0.1:0");
    let mut cmd = Command::new(program);
    if !opts.windowed {
        cmd.arg("--headless");
    }
    cmd.arg("--control").arg(bind);
    if let Some(dr) = &opts.data_root {
        cmd.arg("--data-root").arg(dr);
    }
    if let Some(route) = &opts.route {
        cmd.arg("--route").arg(route);
    }
    if let Some(mode) = &opts.mode {
        cmd.arg("--control-mode").arg(mode);
    }
    if let Some(tok) = &opts.token {
        cmd.arg("--token").arg(tok);
    }
    if opts.stub {
        cmd.arg("--stub");
    }

    // Fully detach stdio so the launched title never receives SIGPIPE when lego-ctrl exits.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to launch `{}`: {e}", program.display()))?;
    let pid = child.id();

    let token = resolve_token(opts.token.as_deref());
    let session_file = sessions_dir().map(|d| d.join(format!("{pid}.json")));
    let explicit_probe_url = explicit_bind_probe_url(bind);
    let deadline = Instant::now() + opts.timeout;

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("failed to poll launched process {pid}: {e}"))?
        {
            return Err(format!(
                "`{}` (pid {pid}) exited early with {status} before opening control endpoint",
                program.display()
            ));
        }

        if let Some(path) = &session_file {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(info) = parse_session_json(&text) {
                    if let Ok(client) = Client::new(&info.control_url, token.clone()) {
                        let probe = client.with_timeout(Duration::from_millis(500));
                        if probe.get("/api/status").is_ok_and(|r| r.is_success()) {
                            return Ok(LaunchResult {
                                pid,
                                title: info.title,
                                control_url: info.control_url,
                                watch_url: info.watch_url,
                            });
                        }
                    }
                }
            }
        }

        if let Some(url) = &explicit_probe_url {
            if let Ok(client) = Client::new(url, token.clone()) {
                let probe = client.with_timeout(Duration::from_millis(500));
                if probe.get("/api/status").is_ok_and(|r| r.is_success()) {
                    return Ok(LaunchResult {
                        pid,
                        title: opts.title.clone(),
                        control_url: probe.url(),
                        watch_url: probe.watch_url(),
                    });
                }
            }
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timed out after {}s waiting for `{}` (pid {pid}) control endpoint",
                opts.timeout.as_secs(),
                opts.title
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn explicit_bind_probe_url(bind: &str) -> Option<String> {
    let (host, port_str) = bind.rsplit_once(':')?;
    let port: u16 = port_str.parse().ok()?;
    if port == 0 {
        return None;
    }
    let connect_host = if host == "0.0.0.0" { "127.0.0.1" } else { host };
    Some(format!("http://{connect_host}:{port}"))
}
