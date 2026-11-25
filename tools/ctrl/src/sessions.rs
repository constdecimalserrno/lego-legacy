//! Session discovery, stale PID pruning, and control-target resolution (#312).
//!
//! Reads `~/.local/state/lego-legacy/sessions/*.json` written by
//! `lego-legacy-core`'s HTTP/UDS control server (#309).

use crate::http::{Client, Endpoint};
use crate::json::{parse_json, write_json_str, JsonValue};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Metadata for one live control session discovered on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub pid: u32,
    pub title: String,
    pub control_url: String,
    pub watch_url: Option<String>,
    pub windowed: bool,
    pub started_at: u64,
}

impl SessionInfo {
    /// Serialize this session record as a JSON object string.
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
        let _ = write!(
            out,
            ",\"windowed\":{},\"started_at\":{}}}",
            self.windowed, self.started_at
        );
        out
    }
}

/// Format a slice of [`SessionInfo`] entries as a JSON array.
#[must_use]
pub fn sessions_to_json(sessions: &[SessionInfo]) -> String {
    let mut out = String::from("[");
    for (i, s) in sessions.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&s.to_json());
    }
    out.push(']');
    out
}

/// Default session discovery directory (`~/.local/state/lego-legacy/sessions`).
#[must_use]
pub fn sessions_dir() -> Option<PathBuf> {
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

/// Check whether `pid` is alive on the current host.
#[cfg(unix)]
#[must_use]
pub fn pid_is_alive(pid: u32) -> bool {
    let Ok(raw_pid) = i32::try_from(pid) else {
        return false;
    };
    if raw_pid <= 0 {
        return false;
    }
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: `kill(pid, 0)` only checks process existence/permission without sending a signal.
    let rc = unsafe { kill(raw_pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::PermissionDenied
}

/// Non-Unix hosts fall back to socket reachability for liveness checks.
#[cfg(not(unix))]
#[must_use]
pub fn pid_is_alive(_pid: u32) -> bool {
    true
}

/// Parse one `<pid>.json` session file.
///
/// # Errors
/// Returns an error if the JSON is malformed or missing required fields.
pub fn parse_session_json(text: &str) -> Result<SessionInfo, String> {
    let val = parse_json(text)?;
    let obj = val
        .as_object()
        .ok_or_else(|| "session record must be a JSON object".to_owned())?;
    let pid = obj
        .get("pid")
        .and_then(JsonValue::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| "missing `pid`".to_owned())?;
    let title = obj
        .get("title")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing `title`".to_owned())?
        .to_owned();
    let control_url = obj
        .get("control_url")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "missing `control_url`".to_owned())?
        .to_owned();
    let watch_url = obj
        .get("watch_url")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let windowed = obj
        .get("windowed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let started_at = obj
        .get("started_at")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    Ok(SessionInfo {
        pid,
        title,
        control_url,
        watch_url,
        windowed,
        started_at,
    })
}

/// Discover live sessions in `dir`, newest first.
///
/// When `prune_stale` is true, session files whose PID is dead on Unix (or whose
/// endpoint is unreachable on non-Unix) are unlinked.
#[must_use]
pub fn list_sessions_in(dir: &Path, prune_stale: bool) -> Vec<SessionInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut live = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(info) = parse_session_json(&text) else {
            continue;
        };
        if !pid_is_alive(info.pid) {
            if prune_stale {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        let reachable =
            Endpoint::parse(&info.control_url).is_ok_and(|ep| ep.is_reachable(PROBE_TIMEOUT));
        if !reachable {
            #[cfg(not(unix))]
            if prune_stale {
                let _ = std::fs::remove_file(&path);
            }
            continue;
        }
        live.push(info);
    }
    live.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.pid.cmp(&a.pid)));
    live
}

/// Discover live sessions in the default session directory.
#[must_use]
pub fn list_live_sessions(prune_stale: bool) -> Vec<SessionInfo> {
    let Some(dir) = sessions_dir() else {
        return Vec::new();
    };
    list_sessions_in(&dir, prune_stale)
}

/// Resolve Bearer token from explicit `--token` flag or `LEGO_LEGACY_TOKEN`.
#[must_use]
pub fn resolve_token(flag_token: Option<&str>) -> Option<String> {
    flag_token
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("LEGO_LEGACY_TOKEN")
                .ok()
                .filter(|t| !t.is_empty())
        })
}

/// Resolve a [`Client`] using priority:
/// 1. Explicit `--url <url>`
/// 2. `LEGO_CTRL_URL` environment variable
/// 3. Latest live session in `~/.local/state/lego-legacy/sessions/` (filtered by `--title` if given)
///
/// # Errors
/// Returns a clear error if no live session or explicit URL is available.
pub fn resolve_client(
    flag_url: Option<&str>,
    flag_title: Option<&str>,
    flag_token: Option<&str>,
) -> Result<Client, String> {
    let token = resolve_token(flag_token);
    if let Some(url) = flag_url.filter(|u| !u.is_empty()) {
        return Client::new(url, token);
    }
    if let Ok(env_url) = std::env::var("LEGO_CTRL_URL") {
        if !env_url.trim().is_empty() {
            return Client::new(env_url.trim(), token);
        }
    }
    let sessions = list_live_sessions(true);
    let chosen = match flag_title.filter(|t| !t.is_empty()) {
        Some(title) => sessions
            .into_iter()
            .find(|s| s.title.eq_ignore_ascii_case(title))
            .ok_or_else(|| {
                format!(
                    "no live session found for title `{title}` (start one with `lego-ctrl launch {title}` or pass `--url`)"
                )
            })?,
        None => sessions.into_iter().next().ok_or_else(|| {
            "no live lego-legacy session found (start one with `lego-ctrl launch <title>`, set LEGO_CTRL_URL, or pass `--url`)"
                .to_owned()
        })?,
    };
    Client::new(&chosen.control_url, token)
}
