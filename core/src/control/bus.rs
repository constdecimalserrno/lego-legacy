//! Shared control-plane state machine (`ControlBus`) (#308).
//!
//! Strictly outside guest state (Golden Rules 1 & 2, ADR-0001). Every
//! transport (`stdio.rs` here; HTTP/1.1 + UDS in #309) dispatches typed
//! [`Command`] values into [`ControlBus::dispatch`] / [`ControlBus::dispatch_sync`]
//! and observes live frames, [`FrameDiff`], `#296` [`Telemetry`], and
//! [`AgentState`] without touching `Machine`.

use super::timeline::{Action, Timeline};
use crate::clock::FlipCost;
use crate::host::telemetry::{FpsReport, Telemetry};
use crate::host::{FlipMeta, Input};
use crate::machine::Finish;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Maximum entries retained in [`AgentState::action_feed`].
pub const MAX_ACTION_FEED: usize = 200;
/// Minimum configurable pace multiplier in `Live` mode.
pub const MIN_PACE: f64 = 0.25;
/// Maximum configurable pace multiplier in `Live` mode.
pub const MAX_PACE: f64 = 4.0;

/// Execution mode for [`super::host::ControlHost`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecMode {
    /// "Democracy" / pause-to-think: pauses at flip boundaries once
    /// `step_budget == 0` (and `!route_active` and no active wait condition).
    #[default]
    Step,
    /// "Anarchy" / real-time: runs continuously at configurable pace unless
    /// explicitly paused, injecting queued inputs at flip boundaries.
    Live,
}

impl ExecMode {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "step" | "democracy" => Some(Self::Step),
            "live" | "anarchy" => Some(Self::Live),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Live => "live",
        }
    }
}

/// Equal-citizen input ownership between agent/piped tasks and a human player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputOwner {
    /// Ignores stray human window mouse/key inputs (still forwards `Close`),
    /// keeping the windowed host non-hijacking (`Capture::Free`).
    #[default]
    Agent,
    /// Forwards human inputs from `inner.poll_input` directly.
    Human,
    /// Merges human inputs and agent/piped timeline inputs cleanly.
    Shared,
}

impl InputOwner {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "agent" => Some(Self::Agent),
            "human" => Some(Self::Human),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
            Self::Shared => "shared",
        }
    }
}

/// Initial configuration for [`ControlBus`] and [`super::host::ControlHost`].
#[derive(Clone, Copy, Debug)]
pub struct ControlConfig {
    pub mode: ExecMode,
    pub owner: InputOwner,
    /// Start paused even in [`ExecMode::Live`].
    pub start_paused: bool,
    /// Run `Step`-mode bursts uncapped (`true`, default) or at paced `1x`.
    pub step_uncapped: bool,
    /// Live-mode pace multiplier (`0.25..=4.0`, default `1.0`).
    pub pace: f64,
    /// Title's declared pointer speed for `Mouse::Steered` `mouse_delta` scaling.
    pub pointer_speed: (f64, f64),
    /// Title's declared flip cost for embedded `#296` [`Telemetry`].
    pub flip_cost: FlipCost,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            start_paused: false,
            step_uncapped: true,
            pace: 1.0,
            pointer_speed: (1.0, 1.0),
            flip_cost: FlipCost::Refresh(60),
        }
    }
}

/// Frame-to-frame visual change summary computed over dense BGRA pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameDiff {
    pub diff_pixels: u64,
    pub total_pixels: u64,
    pub diff_pct: f64,
    /// Bounding box `(x, y, width, height)` of differing pixels, or `None` if identical.
    pub diff_bbox: Option<(u32, u32, u32, u32)>,
    pub unchanged_streak: u64,
}

/// Immutable snapshot of the latest presented BGRA frame and its diff metadata.
#[derive(Clone, Debug)]
pub struct FrameSnapshot {
    pub flip: u64,
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<Vec<u8>>,
    pub frame_hash: String,
    pub diff: FrameDiff,
    pub uniform: bool,
}

/// Lightweight frame metadata returned by [`Command::FrameInfo`].
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfoData {
    pub flip: u64,
    pub width: u32,
    pub height: u32,
    pub frame_hash: String,
    pub diff: FrameDiff,
    pub uniform: bool,
}

/// One entry in [`AgentState::action_feed`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionFeedEntry {
    pub seq: u64,
    pub flip: u64,
    pub declared_ms: u64,
    pub source: String,
    pub summary: String,
}

/// Structured agent/observer scratchpad shared across transports and UIs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentState {
    pub goal: String,
    pub subgoals: Vec<String>,
    pub thought: String,
    pub commentary: String,
    pub notepad: String,
    pub user_hint: String,
    pub action_feed: VecDeque<ActionFeedEntry>,
}

/// Partial update to [`AgentState`] (only `Some` fields are overwritten).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentStatePatch {
    pub goal: Option<String>,
    pub subgoals: Option<Vec<String>>,
    pub thought: Option<String>,
    pub commentary: Option<String>,
    pub notepad: Option<String>,
    pub user_hint: Option<String>,
}

/// Snapshot of the control plane's live status.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlStatus {
    pub flip: Option<u64>,
    pub cursor: u64,
    pub declared_ms: u64,
    pub instructions_passed: u64,
    pub mode: ExecMode,
    pub owner: InputOwner,
    pub paused: bool,
    pub finished: bool,
    pub outcome: Option<String>,
    pub digest: Option<String>,
    pub step_uncapped: bool,
    pub pace: f64,
    pub route_active: bool,
    pub route_remaining: usize,
    pub pointer_pos: (i32, i32),
    pub pointer_shown: bool,
    pub held_keys: Vec<u32>,
    pub pending_actions: usize,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_hash: Option<String>,
    pub diff: FrameDiff,
}

/// Result of a `Wait`, `WaitUntilChange`, or `WaitUntilSettled` command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaitResult {
    pub satisfied: bool,
    pub flips_elapsed: u64,
    pub diff_pct: f64,
}

/// Commands accepted by [`ControlBus::dispatch`] (shared across stdio & future HTTP/UDS).
#[derive(Clone, Debug)]
pub enum Command {
    Status,
    FrameInfo,
    Telemetry {
        window: Option<(u64, u64)>,
    },
    GetAgentState,
    SetAgentState(AgentStatePatch),
    Step {
        flips: u64,
    },
    Act {
        action: Action,
        auto_step: bool,
    },
    Batch {
        actions: Vec<Action>,
        auto_step: bool,
    },
    PipeRoute {
        route_text: String,
        relative: bool,
        auto_step: bool,
    },
    Wait {
        flips: Option<u64>,
        ms: Option<u64>,
    },
    WaitUntilChange {
        max_flips: u64,
        min_diff_pct: f64,
    },
    WaitUntilSettled {
        max_flips: u64,
        stable_flips: u64,
        max_diff_pct: f64,
    },
    Pause,
    Resume,
    SetMode {
        mode: ExecMode,
        step_uncapped: Option<bool>,
    },
    SetOwner(InputOwner),
    SetPace(f64),
    ClearQueue,
    Close {
        force: bool,
    },
}

/// Typed payload accompanying a [`Response`].
#[derive(Clone, Debug)]
pub enum ResponseData {
    None,
    Frame(FrameInfoData),
    Telemetry(FpsReport),
    AgentState(AgentState),
    Scheduled {
        events_scheduled: usize,
        end_flip: u64,
        required_flips: u64,
    },
    WaitResult(WaitResult),
}

/// Response returned by [`ControlBus::dispatch`] and [`ControlBus::dispatch_sync`].
#[derive(Clone, Debug)]
pub struct Response {
    pub ok: bool,
    pub error: Option<String>,
    pub status: ControlStatus,
    pub data: ResponseData,
}

#[derive(Clone, Debug)]
pub(crate) enum WaitCondition {
    Fixed {
        start_flip: u64,
        target_flip: Option<u64>,
        target_ms: Option<u64>,
    },
    UntilChange {
        start_flip: u64,
        max_flips: u64,
        min_diff_pct: f64,
        baseline: Option<(u32, u32, Arc<Vec<u8>>)>,
    },
    UntilSettled {
        start_flip: u64,
        max_flips: u64,
        stable_flips: u64,
        max_diff_pct: f64,
        stable_count: u64,
    },
}

pub(crate) struct BusState {
    pub mode: ExecMode,
    pub owner: InputOwner,
    pub pause_requested: bool,
    pub is_paused: bool,
    pub step_budget: u64,
    pub step_uncapped: bool,
    pub pace: f64,
    pub active_wait: Option<WaitCondition>,
    pub last_wait_result: Option<WaitResult>,
    pub wait_epoch: u64,
    pub step_epoch: u64,
    pub finished: bool,
    pub outcome: Option<String>,
    pub digest: Option<String>,
    pub transport_eof: bool,
    pub close_pending: bool,
    pub force_closes: u32,
    pub last_meta: Option<FlipMeta>,
    pub pointer_pos: (i32, i32),
    pub pointer_shown: bool,
    pub held_keys: Vec<u32>,
    pub latest_frame: Option<FrameSnapshot>,
    pub telemetry: Telemetry,
    pub audio_underruns: Option<u64>,
    pub timeline: Timeline,
    pub agent_state: AgentState,
    pub feed_seq: u64,
}

impl BusState {
    fn new(config: ControlConfig) -> Self {
        Self {
            mode: config.mode,
            owner: config.owner,
            pause_requested: config.start_paused,
            is_paused: false,
            step_budget: 0,
            step_uncapped: config.step_uncapped,
            pace: clamp_pace(config.pace),
            active_wait: None,
            last_wait_result: None,
            wait_epoch: 0,
            step_epoch: 0,
            finished: false,
            outcome: None,
            digest: None,
            transport_eof: false,
            close_pending: false,
            force_closes: 0,
            last_meta: None,
            pointer_pos: (0, 0),
            pointer_shown: true,
            held_keys: Vec::new(),
            latest_frame: None,
            telemetry: Telemetry::new(config.flip_cost),
            audio_underruns: None,
            timeline: Timeline::new(config.pointer_speed),
            agent_state: AgentState::default(),
            feed_seq: 0,
        }
    }

    pub(crate) fn current_flip(&self) -> u64 {
        self.last_meta.map_or(0, |m| m.flip)
    }

    pub(crate) fn current_declared_ms(&self) -> u64 {
        self.last_meta.map_or(0, |m| m.clock_nanos / 1_000_000)
    }

    pub(crate) fn push_feed(&mut self, source: &str, summary: String) {
        self.feed_seq += 1;
        let entry = ActionFeedEntry {
            seq: self.feed_seq,
            flip: self.current_flip(),
            declared_ms: self.current_declared_ms(),
            source: source.to_owned(),
            summary,
        };
        if self.agent_state.action_feed.len() >= MAX_ACTION_FEED {
            self.agent_state.action_feed.pop_front();
        }
        self.agent_state.action_feed.push_back(entry);
    }

    pub(crate) fn record_emitted_inputs(&mut self, inputs: &[Input]) {
        for input in inputs {
            match *input {
                Input::Move { x, y } => {
                    self.pointer_pos = (x, y);
                }
                Input::Key { code, down } => {
                    self.held_keys.retain(|k| *k != code);
                    if down {
                        self.held_keys.push(code);
                    }
                }
                Input::Down(_) | Input::Up(_) | Input::Close => {}
            }
        }
    }

    pub(crate) fn should_pause(&self, route_active: bool) -> bool {
        if self.finished || self.close_pending || self.force_closes > 0 {
            return false;
        }
        if route_active {
            return false;
        }
        if self.active_wait.is_some() {
            return false;
        }
        match self.mode {
            ExecMode::Step => self.step_budget == 0,
            ExecMode::Live => self.pause_requested && self.step_budget == 0,
        }
    }

    pub(crate) fn status(&self) -> ControlStatus {
        let (frame_width, frame_height, frame_hash, diff) = match &self.latest_frame {
            Some(f) => (f.width, f.height, Some(f.frame_hash.clone()), f.diff),
            None => (0, 0, None, FrameDiff::default()),
        };
        ControlStatus {
            flip: self.last_meta.map(|m| m.flip),
            cursor: self.last_meta.map_or(0, |m| m.cursor),
            declared_ms: self.current_declared_ms(),
            instructions_passed: self.last_meta.map_or(0, |m| m.instructions_passed),
            mode: self.mode,
            owner: self.owner,
            paused: self.is_paused,
            finished: self.finished,
            outcome: self.outcome.clone(),
            digest: self.digest.clone(),
            step_uncapped: self.step_uncapped,
            pace: self.pace,
            route_active: self.last_meta.is_some_and(|m| m.route_active),
            route_remaining: self.last_meta.map_or(0, |m| m.route_remaining),
            pointer_pos: self.pointer_pos,
            pointer_shown: self.pointer_shown,
            held_keys: self.held_keys.clone(),
            pending_actions: self.timeline.pending_len(),
            frame_width,
            frame_height,
            frame_hash,
            diff,
        }
    }
}

struct ControlInner {
    state: Mutex<BusState>,
    wake_host: Condvar,
    wake_client: Condvar,
}

/// Thread-safe handle to the unified control plane shared between
/// [`super::host::ControlHost`] and any transport (`stdio`, HTTP, UDS, tests).
#[derive(Clone)]
pub struct ControlBus {
    inner: Arc<ControlInner>,
}

impl ControlBus {
    #[must_use]
    pub fn new(config: ControlConfig) -> Self {
        Self {
            inner: Arc::new(ControlInner {
                state: Mutex::new(BusState::new(config)),
                wake_host: Condvar::new(),
                wake_client: Condvar::new(),
            }),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, BusState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn wait_for_host_wake<'a>(
        &self,
        guard: MutexGuard<'a, BusState>,
        timeout: Duration,
    ) -> MutexGuard<'a, BusState> {
        let (guard, _) = self
            .inner
            .wake_host
            .wait_timeout(guard, timeout)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
    }

    pub(crate) fn notify_client(&self) {
        self.inner.wake_client.notify_all();
    }

    pub(crate) fn notify_host(&self) {
        self.inner.wake_host.notify_all();
    }

    /// Snapshot of the current control status.
    #[must_use]
    pub fn status(&self) -> ControlStatus {
        self.lock().status()
    }

    /// Latest presented BGRA frame snapshot, if any flip has occurred.
    #[must_use]
    pub fn latest_frame(&self) -> Option<FrameSnapshot> {
        self.lock().latest_frame.clone()
    }

    /// Snapshot of the current [`AgentState`].
    #[must_use]
    pub fn agent_state(&self) -> AgentState {
        self.lock().agent_state.clone()
    }

    /// Live `#296` telemetry report over the full run and optional flip window.
    #[must_use]
    pub fn telemetry_report(&self, window: Option<(u64, u64)>) -> FpsReport {
        let state = self.lock();
        state.telemetry.report(window, state.audio_underruns)
    }

    /// Number of forced closes requested through the control plane.
    #[must_use]
    pub fn force_closes(&self) -> u32 {
        self.lock().force_closes
    }

    /// Mark that the guest run has finished, waking any waiting clients.
    pub fn mark_finished(&self, finish: &Finish, digest: &str) {
        let mut state = self.lock();
        state.finished = true;
        state.is_paused = false;
        state.active_wait = None;
        state.outcome = Some(finish.outcome.to_string());
        state.digest = Some(digest.to_owned());
        state.step_epoch += 1;
        state.wait_epoch += 1;
        drop(state);
        self.notify_client();
        self.notify_host();
    }

    /// Mark finished without a [`Finish`] (e.g. host dropped on unwind).
    pub(crate) fn mark_dropped(&self) {
        let mut state = self.lock();
        if !state.finished {
            state.finished = true;
            state.is_paused = false;
            state.active_wait = None;
            state.step_epoch += 1;
            state.wait_epoch += 1;
            drop(state);
            self.notify_client();
            self.notify_host();
        }
    }

    /// Notify the bus that an interactive transport (like `stdin`) reached EOF.
    ///
    /// If the host is in `Step` + `Agent` mode with no pending budget, this
    /// wakes the host so it can exit cleanly instead of blocking forever.
    pub fn mark_transport_eof(&self) {
        let mut state = self.lock();
        state.transport_eof = true;
        drop(state);
        self.notify_host();
    }

    /// Block until the host enters the paused state at a flip boundary or the
    /// run finishes (useful for tests and orchestrators before sending the
    /// first command).
    #[must_use]
    pub fn wait_until_paused(&self, timeout: Option<Duration>) -> ControlStatus {
        let deadline = timeout.map(|d| Instant::now() + d);
        let mut state = self.lock();
        while !state.is_paused && !state.finished {
            state = match deadline {
                Some(dl) => {
                    let Some(rem) = dl.checked_duration_since(Instant::now()) else {
                        break;
                    };
                    let (g, _) = self
                        .inner
                        .wake_client
                        .wait_timeout(state, rem)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    g
                }
                None => self
                    .inner
                    .wake_client
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            };
        }
        state.status()
    }

    /// Non-blocking dispatch: mutates control state, wakes the host if needed,
    /// and returns immediately (never deadlocks even when called on the host's
    /// own thread).
    #[must_use]
    pub fn dispatch(&self, cmd: Command) -> Response {
        let mut state = self.lock();
        let (resp, wake) = apply_command(&mut state, cmd);
        drop(state);
        if wake {
            self.notify_host();
        }
        resp
    }

    /// Dispatch a command and, for advancing commands (`Step`, `Wait*`, or
    /// `Act`/`Batch`/`PipeRoute` with `auto_step: true`), block until the
    /// requested flips/wait condition finish (or the run ends, or `timeout`
    /// elapses).
    #[must_use]
    pub fn dispatch_sync(&self, cmd: Command, timeout: Option<Duration>) -> Response {
        enum SyncKind {
            None,
            Step { start_epoch: u64 },
            Wait { start_epoch: u64 },
        }

        let mut state = self.lock();
        let start_step_epoch = state.step_epoch;
        let start_wait_epoch = state.wait_epoch;
        let sync_kind = match &cmd {
            Command::Step { flips } if *flips > 0 => SyncKind::Step {
                start_epoch: start_step_epoch,
            },
            Command::Act {
                auto_step: true, ..
            }
            | Command::Batch {
                auto_step: true, ..
            }
            | Command::PipeRoute {
                auto_step: true, ..
            } => SyncKind::Step {
                start_epoch: start_step_epoch,
            },
            Command::Wait { .. }
            | Command::WaitUntilChange { .. }
            | Command::WaitUntilSettled { .. } => SyncKind::Wait {
                start_epoch: start_wait_epoch,
            },
            _ => SyncKind::None,
        };

        let (mut resp, wake) = apply_command(&mut state, cmd);
        if !resp.ok {
            drop(state);
            return resp;
        }
        if wake {
            self.inner.wake_host.notify_all();
        }

        let deadline = timeout.map(|d| Instant::now() + d);
        match sync_kind {
            SyncKind::None => {}
            SyncKind::Step { start_epoch } => {
                // Only block when execution is actually bounded (Step mode or
                // paused Live mode with a step budget).
                let bounded = state.mode == ExecMode::Step || state.pause_requested;
                if bounded && (state.step_budget > 0 || !state.is_paused) {
                    while !state.finished && state.step_epoch == start_epoch {
                        state = match deadline {
                            Some(dl) => {
                                let Some(rem) = dl.checked_duration_since(Instant::now()) else {
                                    break;
                                };
                                let (g, _) = self
                                    .inner
                                    .wake_client
                                    .wait_timeout(state, rem)
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                g
                            }
                            None => self
                                .inner
                                .wake_client
                                .wait(state)
                                .unwrap_or_else(std::sync::PoisonError::into_inner),
                        };
                    }
                    resp.status = state.status();
                }
            }
            SyncKind::Wait { start_epoch } => {
                while !state.finished && state.wait_epoch == start_epoch {
                    state = match deadline {
                        Some(dl) => {
                            let Some(rem) = dl.checked_duration_since(Instant::now()) else {
                                break;
                            };
                            let (g, _) = self
                                .inner
                                .wake_client
                                .wait_timeout(state, rem)
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            g
                        }
                        None => self
                            .inner
                            .wake_client
                            .wait(state)
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    };
                }
                resp.status = state.status();
                if let Some(wr) = state.last_wait_result {
                    resp.data = ResponseData::WaitResult(wr);
                }
            }
        }
        resp
    }
}

fn clamp_pace(pace: f64) -> f64 {
    if pace.is_finite() {
        pace.clamp(MIN_PACE, MAX_PACE)
    } else {
        1.0
    }
}

fn ok_response(state: &BusState, data: ResponseData) -> Response {
    Response {
        ok: true,
        error: None,
        status: state.status(),
        data,
    }
}

fn err_response(state: &BusState, msg: impl Into<String>) -> Response {
    Response {
        ok: false,
        error: Some(msg.into()),
        status: state.status(),
        data: ResponseData::None,
    }
}

fn apply_command(state: &mut BusState, cmd: Command) -> (Response, bool) {
    match cmd {
        Command::Status => (ok_response(state, ResponseData::None), false),
        Command::FrameInfo => match &state.latest_frame {
            Some(f) => {
                let info = FrameInfoData {
                    flip: f.flip,
                    width: f.width,
                    height: f.height,
                    frame_hash: f.frame_hash.clone(),
                    diff: f.diff,
                    uniform: f.uniform,
                };
                (ok_response(state, ResponseData::Frame(info)), false)
            }
            None => (err_response(state, "no frame presented yet"), false),
        },
        Command::Telemetry { window } => {
            let report = state.telemetry.report(window, state.audio_underruns);
            (ok_response(state, ResponseData::Telemetry(report)), false)
        }
        Command::GetAgentState => (
            ok_response(state, ResponseData::AgentState(state.agent_state.clone())),
            false,
        ),
        Command::SetAgentState(patch) => {
            if let Some(v) = patch.goal {
                state.agent_state.goal = v;
            }
            if let Some(v) = patch.subgoals {
                state.agent_state.subgoals = v;
            }
            if let Some(v) = patch.thought {
                state.agent_state.thought = v;
            }
            if let Some(v) = patch.commentary {
                state.agent_state.commentary = v;
            }
            if let Some(v) = patch.notepad {
                state.agent_state.notepad = v;
            }
            if let Some(v) = patch.user_hint {
                state.agent_state.user_hint = v;
            }
            (
                ok_response(state, ResponseData::AgentState(state.agent_state.clone())),
                false,
            )
        }
        Command::Step { flips } => {
            if flips > 0 {
                state.step_budget = state.step_budget.saturating_add(flips);
                state.push_feed("control", format!("step {flips}"));
            }
            (ok_response(state, ResponseData::None), flips > 0)
        }
        Command::Act { action, auto_step } => {
            let flip = state.current_flip();
            let ms = state.current_declared_ms();
            let summary = action.summary();
            match state.timeline.schedule(&action, flip, ms) {
                Ok(report) => {
                    state.push_feed("agent", summary);
                    let wake = if auto_step && report.required_flips > 0 {
                        state.step_budget = state.step_budget.max(report.required_flips);
                        true
                    } else {
                        false
                    };
                    (
                        ok_response(
                            state,
                            ResponseData::Scheduled {
                                events_scheduled: report.events_scheduled,
                                end_flip: report.end_flip,
                                required_flips: report.required_flips,
                            },
                        ),
                        wake,
                    )
                }
                Err(err) => (err_response(state, err.to_string()), false),
            }
        }
        Command::Batch { actions, auto_step } => {
            let flip = state.current_flip();
            let ms = state.current_declared_ms();
            let mut total_events = 0usize;
            let mut max_end = flip;
            let mut max_req = 0u64;
            for action in &actions {
                let summary = action.summary();
                match state.timeline.schedule(action, flip, ms) {
                    Ok(report) => {
                        state.push_feed("agent", summary);
                        total_events += report.events_scheduled;
                        max_end = max_end.max(report.end_flip);
                        max_req = max_req.max(report.required_flips);
                    }
                    Err(err) => return (err_response(state, err.to_string()), false),
                }
            }
            let wake = if auto_step && max_req > 0 {
                state.step_budget = state.step_budget.max(max_req);
                true
            } else {
                false
            };
            (
                ok_response(
                    state,
                    ResponseData::Scheduled {
                        events_scheduled: total_events,
                        end_flip: max_end,
                        required_flips: max_req,
                    },
                ),
                wake,
            )
        }
        Command::PipeRoute {
            route_text,
            relative,
            auto_step,
        } => {
            let flip = state.current_flip();
            match state.timeline.pipe_route(&route_text, flip, relative) {
                Ok(report) => {
                    state.push_feed(
                        "pipe",
                        format!(
                            "pipe_route {} events (end flip {})",
                            report.events_scheduled, report.end_flip
                        ),
                    );
                    let wake = if auto_step && report.required_flips > 0 {
                        state.step_budget = state.step_budget.max(report.required_flips);
                        true
                    } else {
                        false
                    };
                    (
                        ok_response(
                            state,
                            ResponseData::Scheduled {
                                events_scheduled: report.events_scheduled,
                                end_flip: report.end_flip,
                                required_flips: report.required_flips,
                            },
                        ),
                        wake,
                    )
                }
                Err(err) => (err_response(state, err.to_string()), false),
            }
        }
        Command::Wait { flips, ms } => {
            let start_flip = state.current_flip();
            let start_ms = state.current_declared_ms();
            let (target_flip, target_ms) = match (flips, ms) {
                (None, None) => (Some(start_flip.saturating_add(1)), None),
                (f, m) => (
                    f.map(|n| start_flip.saturating_add(n.max(1))),
                    m.map(|n| start_ms.saturating_add(n)),
                ),
            };
            state.active_wait = Some(WaitCondition::Fixed {
                start_flip,
                target_flip,
                target_ms,
            });
            state.last_wait_result = None;
            state.push_feed("control", format!("wait flips={flips:?} ms={ms:?}"));
            (ok_response(state, ResponseData::None), true)
        }
        Command::WaitUntilChange {
            max_flips,
            min_diff_pct,
        } => {
            let start_flip = state.current_flip();
            let baseline = state
                .latest_frame
                .as_ref()
                .map(|f| (f.width, f.height, Arc::clone(&f.bgra)));
            state.active_wait = Some(WaitCondition::UntilChange {
                start_flip,
                max_flips: max_flips.max(1),
                min_diff_pct: if min_diff_pct.is_finite() {
                    min_diff_pct.max(0.0)
                } else {
                    0.0
                },
                baseline,
            });
            state.last_wait_result = None;
            state.push_feed(
                "control",
                format!("wait_until_change max={max_flips} min_diff={min_diff_pct:.2}%"),
            );
            (ok_response(state, ResponseData::None), true)
        }
        Command::WaitUntilSettled {
            max_flips,
            stable_flips,
            max_diff_pct,
        } => {
            let start_flip = state.current_flip();
            state.active_wait = Some(WaitCondition::UntilSettled {
                start_flip,
                max_flips: max_flips.max(1),
                stable_flips: stable_flips.max(1),
                max_diff_pct: if max_diff_pct.is_finite() {
                    max_diff_pct.max(0.0)
                } else {
                    0.0
                },
                stable_count: 0,
            });
            state.last_wait_result = None;
            state.push_feed(
                "control",
                format!(
                    "wait_until_settled max={max_flips} stable={stable_flips} max_diff={max_diff_pct:.2}%"
                ),
            );
            (ok_response(state, ResponseData::None), true)
        }
        Command::Pause => {
            state.pause_requested = true;
            state.step_budget = 0;
            state.active_wait = None;
            state.push_feed("control", "pause".to_owned());
            (ok_response(state, ResponseData::None), false)
        }
        Command::Resume => {
            state.pause_requested = false;
            if state.mode == ExecMode::Step {
                state.mode = ExecMode::Live;
            }
            state.push_feed("control", "resume".to_owned());
            (ok_response(state, ResponseData::None), true)
        }
        Command::SetMode {
            mode,
            step_uncapped,
        } => {
            state.mode = mode;
            if let Some(u) = step_uncapped {
                state.step_uncapped = u;
            }
            if mode == ExecMode::Live {
                state.pause_requested = false;
            }
            state.push_feed("control", format!("mode {}", mode.as_str()));
            (ok_response(state, ResponseData::None), true)
        }
        Command::SetOwner(owner) => {
            state.owner = owner;
            state.push_feed("control", format!("owner {}", owner.as_str()));
            (ok_response(state, ResponseData::None), false)
        }
        Command::SetPace(pace) => {
            state.pace = clamp_pace(pace);
            state.push_feed("control", format!("pace {:.2}x", state.pace));
            (ok_response(state, ResponseData::None), false)
        }
        Command::ClearQueue => {
            state.timeline.clear();
            state.active_wait = None;
            state.push_feed("control", "clear_queue".to_owned());
            (ok_response(state, ResponseData::None), false)
        }
        Command::Close { force } => {
            state.close_pending = true;
            if force {
                state.force_closes = state.force_closes.saturating_add(2);
            }
            state.active_wait = None;
            state.push_feed(
                "control",
                if force {
                    "close (force)".to_owned()
                } else {
                    "close".to_owned()
                },
            );
            (ok_response(state, ResponseData::None), true)
        }
    }
}

/// Compute pixel-level diff, bounding box, unchanged streak, deterministic
/// FNV-1a 64-bit hex hash, and uniformity over dense BGRA pixels in one pass.
#[must_use]
pub fn analyze_frame(
    prev: Option<(&[u8], u32, u32)>,
    curr: &[u8],
    width: u32,
    height: u32,
    prev_streak: u64,
) -> (FrameDiff, String, bool) {
    let total_pixels = u64::from(width).saturating_mul(u64::from(height));
    let expected_bytes = (total_pixels as usize).saturating_mul(4);
    if total_pixels == 0 || curr.len() < expected_bytes {
        let diff = FrameDiff {
            diff_pixels: 0,
            total_pixels: 0,
            diff_pct: 0.0,
            diff_bbox: None,
            unchanged_streak: prev_streak.saturating_add(1),
        };
        return (diff, format!("{:016x}", 0xcbf2_9ce4_8422_2325u64), true);
    }

    let first_px = [curr[0], curr[1], curr[2], curr[3]];
    let mut uniform = true;
    let mut fnv: u64 = 0xcbf2_9ce4_8422_2325;

    let same_dims =
        prev.filter(|&(p, pw, ph)| pw == width && ph == height && p.len() >= expected_bytes);

    let mut diff_pixels = 0u64;
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;

    if let Some((prev_bytes, _, _)) = same_dims {
        let mut idx = 0usize;
        for y in 0..height {
            for x in 0..width {
                let c = [curr[idx], curr[idx + 1], curr[idx + 2], curr[idx + 3]];
                if c != first_px {
                    uniform = false;
                }
                for byte in c {
                    fnv ^= u64::from(byte);
                    fnv = fnv.wrapping_mul(0x0100_0000_01b3);
                }
                // Compare RGB channels (top byte in BGRA32 is unused by DirectDraw).
                if c[0] != prev_bytes[idx]
                    || c[1] != prev_bytes[idx + 1]
                    || c[2] != prev_bytes[idx + 2]
                {
                    diff_pixels += 1;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
                idx += 4;
            }
        }
        let diff_pct = diff_pixels as f64 * 100.0 / total_pixels as f64;
        let (diff_bbox, unchanged_streak) = if diff_pixels == 0 {
            (None, prev_streak.saturating_add(1))
        } else {
            (
                Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)),
                0,
            )
        };
        (
            FrameDiff {
                diff_pixels,
                total_pixels,
                diff_pct,
                diff_bbox,
                unchanged_streak,
            },
            format!("{fnv:016x}"),
            uniform,
        )
    } else {
        for chunk in curr[..expected_bytes].as_chunks::<4>().0 {
            if *chunk != first_px {
                uniform = false;
            }
            for &byte in chunk {
                fnv ^= u64::from(byte);
                fnv = fnv.wrapping_mul(0x0100_0000_01b3);
            }
        }
        let diff = FrameDiff {
            diff_pixels: total_pixels,
            total_pixels,
            diff_pct: 100.0,
            diff_bbox: Some((0, 0, width, height)),
            unchanged_streak: 0,
        };
        (diff, format!("{fnv:016x}"), uniform)
    }
}

/// Compute RGB diff percentage between two dense BGRA buffers (used by
/// `WaitUntilChange` against its captured baseline).
#[must_use]
pub(crate) fn diff_pct_between(a: &[u8], aw: u32, ah: u32, b: &[u8], bw: u32, bh: u32) -> f64 {
    if aw != bw || ah != bh {
        return 100.0;
    }
    let total = u64::from(aw).saturating_mul(u64::from(ah));
    let bytes = (total as usize).saturating_mul(4);
    if total == 0 || a.len() < bytes || b.len() < bytes {
        return 0.0;
    }
    let mut diff = 0u64;
    for (pa, pb) in a[..bytes]
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b[..bytes].as_chunks::<4>().0)
    {
        if pa[0] != pb[0] || pa[1] != pb[1] || pa[2] != pb[2] {
            diff += 1;
        }
    }
    diff as f64 * 100.0 / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_diff_computes_pct_bbox_and_unchanged_streak() {
        let base = vec![0x10u8; 4 * 4 * 4];
        let (d0, h0, u0) = analyze_frame(None, &base, 4, 4, 0);
        assert!(u0);
        assert_eq!(d0.diff_pixels, 16);
        assert_eq!(d0.diff_pct, 100.0);
        assert_eq!(d0.diff_bbox, Some((0, 0, 4, 4)));
        assert_eq!(d0.unchanged_streak, 0);

        // Identical frame increments unchanged_streak.
        let (d1, h1, u1) = analyze_frame(Some((&base, 4, 4)), &base, 4, 4, d0.unchanged_streak);
        assert!(u1);
        assert_eq!(h0, h1);
        assert_eq!(d1.diff_pixels, 0);
        assert_eq!(d1.diff_pct, 0.0);
        assert_eq!(d1.diff_bbox, None);
        assert_eq!(d1.unchanged_streak, 1);

        // Second identical frame -> streak 2.
        let (d2, _, _) = analyze_frame(Some((&base, 4, 4)), &base, 4, 4, d1.unchanged_streak);
        assert_eq!(d2.unchanged_streak, 2);

        // Mutate pixels at (1, 1) and (3, 2) -> 2 of 16 pixels = 12.5%, bbox (1, 1, 3, 2).
        let mut next = base.clone();
        next[(4 + 1) * 4] = 0xfe;
        next[(2 * 4 + 3) * 4 + 2] = 0x77;
        let (d3, h3, u3) = analyze_frame(Some((&base, 4, 4)), &next, 4, 4, d2.unchanged_streak);
        assert!(!u3);
        assert_ne!(h3, h0);
        assert_eq!(d3.diff_pixels, 2);
        assert!((d3.diff_pct - 12.5).abs() < 1e-9);
        assert_eq!(d3.diff_bbox, Some((1, 1, 3, 2)));
        assert_eq!(d3.unchanged_streak, 0);
    }

    #[test]
    fn agent_state_patch_updates_subset_and_bounds_action_feed() {
        let bus = ControlBus::new(ControlConfig::default());
        let r1 = bus.dispatch(Command::SetAgentState(AgentStatePatch {
            goal: Some("win chess".into()),
            subgoals: Some(vec!["open menu".into(), "click play".into()]),
            thought: Some("looking at title screen".into()),
            commentary: Some("boot complete".into()),
            notepad: Some("button at 320,240".into()),
            user_hint: Some("try white pieces".into()),
        }));
        assert!(r1.ok);

        // Partial patch only replaces thought.
        let r2 = bus.dispatch(Command::SetAgentState(AgentStatePatch {
            thought: Some("clicking play".into()),
            ..AgentStatePatch::default()
        }));
        assert!(r2.ok);
        let st = bus.agent_state();
        assert_eq!(st.goal, "win chess");
        assert_eq!(st.subgoals, ["open menu", "click play"]);
        assert_eq!(st.thought, "clicking play");
        assert_eq!(st.commentary, "boot complete");
        assert_eq!(st.notepad, "button at 320,240");
        assert_eq!(st.user_hint, "try white pieces");

        // Action feed stays bounded to MAX_ACTION_FEED.
        for _ in 0..(MAX_ACTION_FEED + 25) {
            let _ = bus.dispatch(Command::SetPace(1.0));
        }
        assert_eq!(bus.agent_state().action_feed.len(), MAX_ACTION_FEED);
    }
}
