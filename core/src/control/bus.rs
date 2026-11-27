//! Shared control-plane state machine (`ControlBus`) (#308).
//!
//! Strictly outside guest state (Golden Rules 1 & 2, ADR-0001). Every
//! transport (`stdio.rs` here; HTTP/1.1 + UDS in #309) dispatches typed
//! [`Command`] values into [`ControlBus::dispatch`] / [`ControlBus::dispatch_sync`]
//! and observes live frames, [`FrameDiff`], `#296` [`Telemetry`], and
//! [`AgentState`] without touching `Machine`.

use super::inspect::{execute_inspect, BookmarkData, InspectQuery, InspectReply};
use super::observe::STUCK_LOOP_THRESHOLD;
use super::snapshot::{
    execute_snapshot_op, SnapshotOp, SnapshotReply, SnapshotSlot, SnapshotSummary,
};
use super::timeline::{parse_pipe_route, resolve_key, Action, Timeline};
use crate::clock::FlipCost;
use crate::host::telemetry::{FpsReport, Telemetry};
use crate::host::{Button, FlipMeta, Host, Input};
use crate::machine::{Finish, Machine};
use crate::title::TitleGuide;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// DirectInput scancode for `Escape` (guarded when `guide.escape_quits == true`).
const DIK_ESCAPE: u32 = 0x01;

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
    /// Title roster key reported in `/api/status` (e.g. `"stub"`).
    pub title: &'static str,
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
    /// Static per-title playability metadata (#311), used for safety guards
    /// (`escape_quits`), relative-steering scaling hints, and `/api/meta`.
    pub guide: &'static TitleGuide,
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            title: "",
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            start_paused: false,
            step_uncapped: true,
            pace: 1.0,
            pointer_speed: (1.0, 1.0),
            flip_cost: FlipCost::Refresh(60),
            guide: &TitleGuide::EMPTY,
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
    /// Frame-locked in-game visual cursor position at `flip` (#327).
    pub pointer_pos: (i32, i32),
    /// Frame-locked host cursor visibility (`ShowCursor >= 0`) at `flip` (#327).
    pub pointer_shown: bool,
    /// Whether the active OS cursor shape is completely transparent (`SetCursor(NULL)`).
    pub cursor_blank: bool,
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
    pub title: String,
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
    pub recording_len: usize,
    /// Tracked in-game visual cursor coordinates (#327).
    pub pointer_pos: (i32, i32),
    /// Tracked Win32 hardware pointer anchor (`last_move`) (#327).
    pub hw_pointer_pos: (i32, i32),
    pub pointer_shown: bool,
    /// Whether the active OS cursor shape is completely transparent (`SetCursor(NULL)`).
    pub cursor_blank: bool,
    pub held_keys: Vec<u32>,
    pub pending_actions: usize,
    pub frame_width: u32,
    pub frame_height: u32,
    pub frame_hash: Option<String>,
    pub diff: FrameDiff,
}

/// Result of a bookmark rewind (`/api/rewind`, #309, #313).
#[derive(Clone, Debug, PartialEq)]
pub struct RewindResult {
    pub name: String,
    pub flip: u64,
    pub cursor: u64,
    pub declared_ms: u64,
    pub digest: String,
    pub expected_digest: String,
    pub digest_match: bool,
    pub mode: &'static str,
    pub restore_ms: Option<f64>,
}

/// 60 Hz window-pump interval while paused at a flip boundary.
const PAUSE_PUMP_INTERVAL: Duration = Duration::from_millis(16);

struct TicketSlot<T> {
    reply: Mutex<Option<T>>,
    ready: Condvar,
}

impl<T> TicketSlot<T> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(None),
            ready: Condvar::new(),
        })
    }

    fn complete(&self, value: T) {
        let mut guard = self
            .reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(value);
        self.ready.notify_all();
    }

    fn try_take(&self) -> Option<T> {
        self.reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn wait(&self, timeout: Option<Duration>) -> Option<T> {
        let deadline = timeout.map(|d| Instant::now() + d);
        let mut guard = self
            .reply
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while guard.is_none() {
            guard = match deadline {
                Some(dl) => {
                    let rem = dl.checked_duration_since(Instant::now())?;
                    if rem.is_zero() {
                        return None;
                    }
                    let (g, _) = self
                        .ready
                        .wait_timeout(guard, rem)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    g
                }
                None => self
                    .ready
                    .wait(guard)
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            };
        }
        guard.take()
    }
}

struct InspectTicket {
    query: InspectQuery,
    slot: Arc<TicketSlot<InspectReply>>,
}

struct SnapshotTicket {
    op: SnapshotOp,
    slot: Arc<TicketSlot<SnapshotReply>>,
}

struct PendingRewind {
    name: String,
    bookmark: BookmarkData,
    slot: Arc<TicketSlot<Result<RewindResult, String>>>,
}

#[derive(Clone)]
pub(crate) struct BookmarkSnapshot {
    pub bytes: Arc<Vec<u8>>,
    pub frame: Option<FrameSnapshot>,
}

/// Atomic pacing/reset snapshot read by `ControlHost::pace`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PaceSnapshot {
    pub uncapped: bool,
    pub wall_pace_live: bool,
    pub pace: f64,
    pub pace_epoch: u64,
    pub runtime_reset_epoch: u64,
    pub reset_clock_nanos: u64,
}

/// Inputs required to rebaseline [`ControlBus`] after a live `LLSNAP v2` restore.
pub(crate) struct RestoreSync {
    pub meta: FlipMeta,
    pub restored_frame: Option<FrameSnapshot>,
    pub held_keys: Vec<u32>,
    pub held_buttons: Vec<Button>,
    pub recording_len: usize,
    pub cursor_blank: bool,
    pub hw_pointer: (i32, i32),
    pub pace_sleep_nanos: u64,
}

/// Atomic snapshot served to `/api/events` SSE streams.
pub(crate) struct SseSnapshot {
    pub status: ControlStatus,
    pub agent_state: AgentState,
    pub uniform: bool,
    pub active_driving: bool,
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
    Cleared {
        cleared: usize,
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
enum WaitCondition {
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

struct BusState {
    title: String,
    guide: &'static TitleGuide,
    mode: ExecMode,
    owner: InputOwner,
    pause_requested: bool,
    is_paused: bool,
    step_budget: u64,
    step_uncapped: bool,
    pace: f64,
    pace_epoch: u64,
    runtime_reset_epoch: u64,
    reset_clock_nanos: u64,
    active_wait: Option<WaitCondition>,
    last_wait_result: Option<WaitResult>,
    wait_epoch: u64,
    step_epoch: u64,
    finished: bool,
    outcome: Option<String>,
    digest: Option<String>,
    transport_eof: bool,
    close_pending: bool,
    force_closes: u32,
    last_meta: Option<FlipMeta>,
    recording_len: usize,
    held_keys: Vec<u32>,
    held_buttons: Vec<Button>,
    pending_releases: Vec<Input>,
    latest_frame: Option<FrameSnapshot>,
    telemetry: Telemetry,
    audio_underruns: Option<u64>,
    timeline: Timeline,
    last_agent_action_flip: Option<u64>,
    agent_timeline_end_flip: Option<u64>,
    agent_state: AgentState,
    feed_seq: u64,
    machine_attached: bool,
    pending_inspects: VecDeque<InspectTicket>,
    pending_snapshots: VecDeque<SnapshotTicket>,
    snapshots: BTreeMap<String, SnapshotSlot>,
    bookmarks: BTreeMap<String, BookmarkData>,
    bookmark_snapshots: BTreeMap<String, BookmarkSnapshot>,
    rewind_abort: bool,
    pending_rewind: Option<PendingRewind>,
    replaying_rewind: Option<PendingRewind>,
    rewind_target_flip: Option<u64>,
}

impl BusState {
    fn new(config: ControlConfig) -> Self {
        let guide = config.guide;
        let effective_speed = match guide.relative_steered_scale() {
            Some(scale)
                if (config.pointer_speed.0 - 1.0).abs() < 1e-9
                    && (config.pointer_speed.1 - 1.0).abs() < 1e-9 =>
            {
                scale
            }
            _ => config.pointer_speed,
        };
        let mut timeline = Timeline::new(effective_speed);
        timeline.configure_mode(guide.mouse_mode, guide.resolution);
        Self {
            title: config.title.to_owned(),
            guide,
            mode: config.mode,
            owner: config.owner,
            pause_requested: config.start_paused,
            is_paused: false,
            step_budget: 0,
            step_uncapped: config.step_uncapped,
            pace: clamp_pace(config.pace),
            pace_epoch: 0,
            runtime_reset_epoch: 0,
            reset_clock_nanos: 0,
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
            recording_len: 0,
            held_keys: Vec::new(),
            held_buttons: Vec::new(),
            pending_releases: Vec::new(),
            latest_frame: None,
            telemetry: Telemetry::new(config.flip_cost),
            audio_underruns: None,
            timeline,
            last_agent_action_flip: None,
            agent_timeline_end_flip: None,
            agent_state: AgentState::default(),
            feed_seq: 0,
            machine_attached: false,
            pending_inspects: VecDeque::new(),
            pending_snapshots: VecDeque::new(),
            snapshots: BTreeMap::new(),
            bookmarks: BTreeMap::new(),
            bookmark_snapshots: BTreeMap::new(),
            rewind_abort: false,
            pending_rewind: None,
            replaying_rewind: None,
            rewind_target_flip: None,
        }
    }

    fn current_flip(&self) -> u64 {
        self.last_meta.map_or(0, |m| m.flip)
    }

    fn observed_flip(&self) -> u64 {
        self.last_meta
            .map(|m| m.flip)
            .or_else(|| self.latest_frame.as_ref().map(|f| f.flip))
            .unwrap_or(0)
    }

    /// Whether the agent is actively driving (queued agent actions or a recent
    /// agent action at/after the current unchanged-frame streak began), used to
    /// suppress false `stuck_loop` warnings on static menu idle or Human Take Over (#350).
    fn is_agent_driving(&self) -> bool {
        if self.owner == InputOwner::Human || self.last_meta.is_some_and(|m| m.route_active) {
            return false;
        }
        let current_flip = self.observed_flip();
        if !self.timeline.is_empty()
            && self
                .agent_timeline_end_flip
                .is_some_and(|end| end >= current_flip)
        {
            return true;
        }
        let Some(act_flip) = self.last_agent_action_flip else {
            return false;
        };
        let unchanged_streak = self
            .latest_frame
            .as_ref()
            .map_or(0, |f| f.diff.unchanged_streak);
        let streak_start = current_flip.saturating_sub(unchanged_streak);
        let last_active = self
            .agent_timeline_end_flip
            .map_or(act_flip, |end| act_flip.max(end.min(current_flip)));
        act_flip <= current_flip
            && act_flip >= streak_start
            && current_flip.saturating_sub(last_active) <= STUCK_LOOP_THRESHOLD
    }

    fn current_declared_ms(&self) -> u64 {
        self.last_meta.map_or(0, |m| m.clock_nanos / 1_000_000)
    }

    fn push_feed(&mut self, source: &str, summary: String) {
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

    fn record_emitted_inputs(&mut self, inputs: &[Input]) {
        for input in inputs {
            match *input {
                Input::Key { code, down } => {
                    self.held_keys.retain(|k| *k != code);
                    if down {
                        self.held_keys.push(code);
                    }
                }
                Input::Down(btn) => {
                    if !self.held_buttons.contains(&btn) {
                        self.held_buttons.push(btn);
                    }
                }
                Input::Up(btn) => {
                    self.held_buttons.retain(|b| *b != btn);
                }
                Input::Move { .. } | Input::Close => {}
            }
        }
    }

    fn should_eof_close(&self) -> bool {
        self.transport_eof && self.mode == ExecMode::Step && self.owner == InputOwner::Agent
    }

    fn consume_step_budget(&mut self, route_active: bool) {
        if !route_active
            && !self.rewind_abort
            && self.rewind_target_flip.is_none()
            && self.active_wait.is_none()
            && self.step_budget > 0
        {
            self.step_budget -= 1;
        }
    }

    fn sse_snapshot(&self) -> SseSnapshot {
        SseSnapshot {
            status: self.status(),
            agent_state: self.agent_state.clone(),
            uniform: self.latest_frame.as_ref().is_some_and(|f| f.uniform),
            active_driving: self.is_agent_driving(),
        }
    }

    fn should_pause(&self, route_active: bool) -> bool {
        if self.finished || self.close_pending || self.force_closes > 0 || self.rewind_abort {
            return false;
        }
        if self.rewind_target_flip.is_some() {
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

    fn evaluate_wait(&mut self, meta: &FlipMeta) -> bool {
        let Some(mut cond) = self.active_wait.take() else {
            return false;
        };
        let frame = self.latest_frame.as_ref();
        let declared_ms = meta.clock_nanos / 1_000_000;
        let done = match &mut cond {
            WaitCondition::Fixed {
                start_flip,
                target_flip,
                target_ms,
            } => {
                let by_flip = target_flip.is_some_and(|tf| meta.flip >= tf);
                let by_ms = target_ms.is_some_and(|tm| declared_ms >= tm);
                if by_flip || by_ms {
                    Some(WaitResult {
                        satisfied: true,
                        flips_elapsed: meta.flip.saturating_sub(*start_flip),
                        diff_pct: frame.map_or(0.0, |f| f.diff.diff_pct),
                    })
                } else {
                    None
                }
            }
            WaitCondition::UntilChange {
                start_flip,
                max_flips,
                min_diff_pct,
                baseline,
            } => {
                if meta.flip <= *start_flip {
                    None
                } else {
                    let elapsed = meta.flip.saturating_sub(*start_flip);
                    let pct = match (baseline.as_ref(), frame) {
                        (Some((bw, bh, base)), Some(curr)) => {
                            diff_pct_between(base, *bw, *bh, &curr.bgra, curr.width, curr.height)
                        }
                        (None, Some(curr)) => curr.diff.diff_pct,
                        _ => 0.0,
                    };
                    if pct >= *min_diff_pct {
                        Some(WaitResult {
                            satisfied: true,
                            flips_elapsed: elapsed,
                            diff_pct: pct,
                        })
                    } else if elapsed >= *max_flips {
                        Some(WaitResult {
                            satisfied: false,
                            flips_elapsed: elapsed,
                            diff_pct: pct,
                        })
                    } else {
                        None
                    }
                }
            }
            WaitCondition::UntilSettled {
                start_flip,
                max_flips,
                stable_flips,
                max_diff_pct,
                stable_count,
            } => {
                if meta.flip <= *start_flip {
                    None
                } else {
                    let elapsed = meta.flip.saturating_sub(*start_flip);
                    let pct = frame.map_or(0.0, |f| f.diff.diff_pct);
                    if pct <= *max_diff_pct {
                        *stable_count += 1;
                    } else {
                        *stable_count = 0;
                    }
                    if *stable_count >= *stable_flips {
                        Some(WaitResult {
                            satisfied: true,
                            flips_elapsed: elapsed,
                            diff_pct: pct,
                        })
                    } else if elapsed >= *max_flips {
                        Some(WaitResult {
                            satisfied: false,
                            flips_elapsed: elapsed,
                            diff_pct: pct,
                        })
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(result) = done {
            self.last_wait_result = Some(result);
            self.wait_epoch += 1;
            true
        } else {
            self.active_wait = Some(cond);
            false
        }
    }

    fn status(&self) -> ControlStatus {
        let (frame_width, frame_height, frame_hash, diff) = match &self.latest_frame {
            Some(f) => (f.width, f.height, Some(f.frame_hash.clone()), f.diff),
            None => (0, 0, None, FrameDiff::default()),
        };
        ControlStatus {
            title: self.title.clone(),
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
            recording_len: self.recording_len,
            pointer_pos: self.timeline.visual_cursor_pos(),
            hw_pointer_pos: self.timeline.last_pointer(),
            pointer_shown: self.timeline.pointer_shown(),
            cursor_blank: self.timeline.cursor_blank(),
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

    fn lock(&self) -> MutexGuard<'_, BusState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wait_for_host_wake<'a>(
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

    fn notify_client(&self) {
        self.inner.wake_client.notify_all();
    }

    fn notify_host(&self) {
        self.inner.wake_host.notify_all();
    }

    /// Snapshot of the current control status.
    #[must_use]
    pub fn status(&self) -> ControlStatus {
        self.lock().status()
    }

    /// Monotonic reset epoch incremented on live restore and rewind so
    /// [`super::host::ControlHost`] resets pacing/staging anchors atomically (#408).
    #[must_use]
    pub fn runtime_reset_epoch(&self) -> u64 {
        self.lock().runtime_reset_epoch
    }

    pub(crate) fn runtime_reset_state(&self) -> (u64, u64) {
        let state = self.lock();
        (state.runtime_reset_epoch, state.reset_clock_nanos)
    }

    /// Static per-title [`TitleGuide`] configured on this control plane (#311).
    #[must_use]
    pub fn guide(&self) -> &'static TitleGuide {
        self.lock().guide
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

    pub(crate) fn set_telemetry_paused(&self, paused: bool) {
        self.lock().telemetry.set_paused(paused);
    }

    pub(crate) fn set_pointer_shown(&self, show: bool) {
        self.lock().timeline.set_pointer_shown(show);
    }

    pub(crate) fn set_pointer(&self, x: i32, y: i32) {
        self.lock().timeline.set_pointer(x, y);
    }

    pub(crate) fn set_cursor_blank(&self, blank: bool) {
        self.lock().timeline.set_cursor_blank(blank);
    }

    pub(crate) fn pace_snapshot(&self) -> PaceSnapshot {
        let state = self.lock();
        let route_active = state.last_meta.is_some_and(|m| m.route_active);
        let uncapped = route_active
            || state.rewind_target_flip.is_some()
            || (state.mode == ExecMode::Step && state.step_uncapped);
        let wall_pace_live =
            state.mode == ExecMode::Live && !state.is_paused && !route_active && state.pace > 0.0;
        PaceSnapshot {
            uncapped,
            wall_pace_live,
            pace: state.pace,
            pace_epoch: state.pace_epoch,
            runtime_reset_epoch: state.runtime_reset_epoch,
            reset_clock_nanos: state.reset_clock_nanos,
        }
    }

    pub(crate) fn poll_input_step(
        &self,
        flip: u64,
        poll_index: u64,
        declared_ms: u64,
        human: Vec<Input>,
    ) -> Vec<Input> {
        let mut state = self.lock();
        if state.rewind_abort || state.rewind_target_flip.is_some() {
            return Vec::new();
        }
        state.timeline.note_polled(flip);
        let owner = state.owner;

        let mut out = Vec::new();
        if !state.pending_releases.is_empty() {
            out.append(&mut state.pending_releases);
        }
        if matches!(owner, InputOwner::Human | InputOwner::Shared) {
            for ev in &human {
                if let Input::Move { x, y } = *ev {
                    state.timeline.observe_emitted_move(x, y);
                }
            }
            out.extend(human);
        } else {
            // Keep windowed host non-hijacking: ignore stray human
            // mouse/key events while still honoring OS window Close.
            for ev in human {
                if ev == Input::Close {
                    out.push(ev);
                }
            }
        }
        if matches!(owner, InputOwner::Agent | InputOwner::Shared) {
            let scheduled = state.timeline.drain_due(flip, poll_index, declared_ms);
            for ev in scheduled {
                if ev == Input::Close && out.contains(&Input::Close) {
                    continue;
                }
                out.push(ev);
            }
        }

        if state.close_pending {
            state.close_pending = false;
            if !out.contains(&Input::Close) {
                out.push(Input::Close);
            }
        }

        state.record_emitted_inputs(&out);
        out
    }

    pub(crate) fn observe_flip(
        &self,
        meta: &FlipMeta,
        staged: Option<(&[u8], u32, u32)>,
        underruns: Option<u64>,
        pace_sleep: u64,
        notify_now: bool,
    ) -> bool {
        let wait_completed = {
            let mut state = self.lock();
            state.last_meta = Some(*meta);
            state.timeline.set_pointer_shown(meta.pointer_shown);
            let mode = state.timeline.mouse_mode();
            if !mode.is_relative()
                && (meta.pointer_shown
                    || matches!(mode, crate::title::MouseMode::SoftwareAbsolute { .. }))
            {
                state
                    .timeline
                    .set_pointer(meta.pointer_pos.0, meta.pointer_pos.1);
            }
            state.audio_underruns = underruns;

            let uniform = if let Some((bgra, w, h)) = staged {
                state.timeline.set_frame_size(w, h);
                let prev_streak = state
                    .latest_frame
                    .as_ref()
                    .map_or(0, |f| f.diff.unchanged_streak);
                let (diff, frame_hash, uniform) = {
                    let prev = state
                        .latest_frame
                        .as_ref()
                        .map(|f| (f.bgra.as_slice(), f.width, f.height));
                    analyze_frame(prev, bgra, w, h, prev_streak)
                };
                let snapshot = FrameSnapshot {
                    flip: meta.flip,
                    width: w,
                    height: h,
                    bgra: Arc::new(bgra.to_vec()),
                    frame_hash,
                    diff,
                    uniform,
                    pointer_pos: state.timeline.visual_cursor_pos(),
                    pointer_shown: state.timeline.pointer_shown(),
                    cursor_blank: state.timeline.cursor_blank(),
                };
                state.latest_frame = Some(snapshot);
                uniform
            } else {
                true
            };

            state.telemetry.record_flip(meta, uniform, pace_sleep);
            state.evaluate_wait(meta)
        };
        if wait_completed && notify_now {
            self.notify_client();
        }
        wait_completed
    }

    /// Unified flip-boundary pause / `step_budget` / `transport_eof` / pump
    /// loop shared by `ControlHost::observe_flip` (`!defer_pause`) and
    /// `service_flip_boundary` (`defer_pause`) (#408).
    pub(crate) fn pause_loop(
        &self,
        host: &mut dyn Host,
        mut machine: Option<&mut Machine>,
        static_route_active: bool,
        extra_notify_client: bool,
    ) {
        let route_active = |m: &Option<&mut Machine>| {
            m.as_ref()
                .map_or(static_route_active, |m| m.undelivered() > 0)
        };
        let initial_route_active = route_active(&machine);
        let initial_closes = host.closes();
        let mut notify_clients = extra_notify_client;
        let mut enter_pause = false;
        {
            let mut state = self.lock();
            if initial_closes == 0 && state.should_pause(initial_route_active) {
                if state.should_eof_close() {
                    state.close_pending = true;
                } else {
                    state.is_paused = true;
                    state.step_epoch += 1;
                    notify_clients = true;
                    enter_pause = true;
                }
            } else {
                state.consume_step_budget(initial_route_active);
            }
        }

        if notify_clients {
            self.notify_client();
        }

        if !enter_pause {
            return;
        }

        host.set_paused(true);
        loop {
            if let Some(m) = machine.as_deref_mut() {
                self.drain_inspects(m);
                self.drain_snapshots(m, host);
            }
            let active_route = route_active(&machine);
            let host_closes = host.closes();
            let mut state = self.lock();
            if host_closes > 0 || state.rewind_abort || !state.should_pause(active_route) {
                state.is_paused = false;
                state.consume_step_budget(active_route);
                drop(state);
                break;
            }
            if state.should_eof_close() {
                state.is_paused = false;
                state.close_pending = true;
                drop(state);
                break;
            }
            if machine.is_some()
                && (!state.pending_inspects.is_empty() || !state.pending_snapshots.is_empty())
            {
                drop(state);
                continue;
            }
            let state = self.wait_for_host_wake(state, PAUSE_PUMP_INTERVAL);
            // Release the bus lock before pumping OS events so clients can
            // query status/frames concurrently without waiting on AppKit/X11/Win32.
            drop(state);
            host.pump();
        }
        host.set_paused(false);
        if let Some(m) = machine {
            self.drain_inspects(m);
            self.drain_snapshots(m, host);
        }
    }

    pub(crate) fn rebaseline_for_rewind(&self, pace_sleep_nanos: u64) -> u64 {
        let mut state = self.lock();
        state.timeline.reset_polled();
        state.timeline.set_cursor_blank(false);
        state.last_agent_action_flip = None;
        state.agent_timeline_end_flip = None;
        state.telemetry.rebaseline(0, 0, 0, pace_sleep_nanos);
        state.pace_epoch = state.pace_epoch.wrapping_add(1);
        state.reset_clock_nanos = 0;
        state.runtime_reset_epoch = state.runtime_reset_epoch.wrapping_add(1);
        state.runtime_reset_epoch
    }

    pub(crate) fn rebaseline_after_restore(&self, sync: RestoreSync) {
        let mut state = self.lock();
        state.last_meta = Some(sync.meta);
        let restored_visual = sync.restored_frame.as_ref().map(|f| f.pointer_pos);
        if let Some(f) = sync.restored_frame {
            state.timeline.set_frame_size(f.width, f.height);
            state.latest_frame = Some(f);
        }
        state.held_keys = sync.held_keys;
        state.held_buttons = sync.held_buttons;
        state.pending_releases.clear();
        state.recording_len = sync.recording_len;
        state.timeline.clear();
        state.timeline.reset_polled();
        state.last_agent_action_flip = None;
        state.agent_timeline_end_flip = None;
        state.timeline.set_pointer_shown(sync.meta.pointer_shown);
        state.timeline.set_cursor_blank(sync.cursor_blank);
        if state.timeline.mouse_mode().is_relative() {
            state
                .timeline
                .set_pointer(sync.hw_pointer.0, sync.hw_pointer.1);
            if let Some((vx, vy)) = restored_visual {
                state.timeline.set_visual_cursor(vx, vy);
            }
        } else {
            state
                .timeline
                .set_pointer(sync.meta.pointer_pos.0, sync.meta.pointer_pos.1);
        }
        state.active_wait = None;
        state.step_budget = 0;
        state.close_pending = false;
        state.pace_epoch = state.pace_epoch.wrapping_add(1);
        state.reset_clock_nanos = sync.meta.clock_nanos;
        state.runtime_reset_epoch = state.runtime_reset_epoch.wrapping_add(1);
        state.telemetry.rebaseline(
            sync.meta.clock_nanos,
            sync.meta.cursor,
            sync.meta.instructions_passed,
            sync.pace_sleep_nanos,
        );
        state.force_closes = 0;
    }

    pub(crate) fn store_snapshot(&self, key: String, slot: SnapshotSlot, feed_summary: String) {
        let mut state = self.lock();
        state.snapshots.insert(key, slot);
        state.push_feed("control", feed_summary);
    }

    pub(crate) fn get_snapshot(&self, name: &str) -> Option<SnapshotSlot> {
        self.lock().snapshots.get(name).cloned()
    }

    pub(crate) fn store_bookmark(
        &self,
        name: String,
        data: BookmarkData,
        snap_bytes: Arc<Vec<u8>>,
        frame: Option<FrameSnapshot>,
        feed_summary: String,
    ) {
        let mut state = self.lock();
        state.bookmarks.insert(name.clone(), data);
        state.bookmark_snapshots.insert(
            name,
            BookmarkSnapshot {
                bytes: snap_bytes,
                frame,
            },
        );
        state.push_feed("control", feed_summary);
    }

    pub(crate) fn get_bookmark_snapshot(
        &self,
        name: &str,
    ) -> Option<(BookmarkData, Option<BookmarkSnapshot>)> {
        let state = self.lock();
        let bm = state.bookmarks.get(name).cloned()?;
        let snap = state.bookmark_snapshots.get(name).cloned();
        Some((bm, snap))
    }

    pub(crate) fn push_feed(&self, source: &str, summary: String) {
        self.lock().push_feed(source, summary);
    }

    pub(crate) fn frame_json_snapshot(&self) -> Option<(FrameSnapshot, bool, bool, bool)> {
        let state = self.lock();
        let frame = state.latest_frame.clone()?;
        Some((
            frame,
            state.is_paused,
            state.finished,
            state.is_agent_driving(),
        ))
    }

    pub(crate) fn snapshot_for_sse(&self) -> SseSnapshot {
        self.lock().sse_snapshot()
    }

    pub(crate) fn snapshot_for_sse_with_telemetry(&self) -> (SseSnapshot, FpsReport) {
        let state = self.lock();
        let snap = state.sse_snapshot();
        let report = state.telemetry.report(None, state.audio_underruns);
        (snap, report)
    }

    fn finish_run(&self, outcome: Option<(String, String)>, reason: &str) {
        let mut state = self.lock();
        if outcome.is_none() && state.finished {
            return;
        }
        state.finished = true;
        state.is_paused = false;
        state.active_wait = None;
        state.machine_attached = false;
        if let Some((out, dig)) = outcome {
            state.outcome = Some(out);
            state.digest = Some(dig);
        }
        state.step_epoch += 1;
        state.wait_epoch += 1;
        let inspects: Vec<_> = state.pending_inspects.drain(..).collect();
        let snapshots: Vec<_> = state.pending_snapshots.drain(..).collect();
        let mut rewinds = Vec::new();
        if let Some(rw) = state.pending_rewind.take() {
            rewinds.push(rw);
        }
        if let Some(rw) = state.replaying_rewind.take() {
            rewinds.push(rw);
        }
        state.rewind_abort = false;
        state.rewind_target_flip = None;
        drop(state);
        fail_pending_ops(inspects, snapshots, rewinds, reason);
        self.notify_client();
        self.notify_host();
    }

    /// Mark that the guest run has finished, waking any waiting clients.
    pub fn mark_finished(&self, finish: &Finish, digest: &str) {
        self.finish_run(
            Some((finish.outcome.to_string(), digest.to_owned())),
            "run has finished",
        );
    }

    /// Mark finished without a [`Finish`] (e.g. host dropped on unwind).
    pub(crate) fn mark_dropped(&self) {
        self.finish_run(None, "control host closed");
    }

    /// Declare whether a live flip-boundary machine inspector is attached (#309).
    pub fn set_machine_attached(&self, attached: bool) {
        self.lock().machine_attached = attached;
    }

    /// Update cheap machine-boundary metadata cached for `/api/status` (#309).
    pub fn update_machine_meta(&self, recording_len: usize) {
        self.lock().recording_len = recording_len;
    }

    /// Submit a read-only [`InspectQuery`] to run on `&Machine` at the active or
    /// next flip boundary (deadlock-free when paused in `Step` mode, #309).
    #[must_use]
    pub fn request_inspect(&self, query: InspectQuery, timeout: Option<Duration>) -> InspectReply {
        let slot = TicketSlot::new();
        {
            let mut state = self.lock();
            if state.finished {
                return InspectReply::Error {
                    status: 410,
                    message: "run has finished".to_owned(),
                };
            }
            if !state.machine_attached {
                return InspectReply::Error {
                    status: 503,
                    message: "machine inspector is not attached".to_owned(),
                };
            }
            state.pending_inspects.push_back(InspectTicket {
                query,
                slot: Arc::clone(&slot),
            });
        }
        self.notify_host();

        if let Some(reply) = slot.wait(timeout) {
            return reply;
        }
        {
            let mut state = self.lock();
            state
                .pending_inspects
                .retain(|t| !Arc::ptr_eq(&t.slot, &slot));
        }
        slot.try_take().unwrap_or_else(|| InspectReply::Error {
            status: 504,
            message: "inspect request timed out waiting for flip boundary".to_owned(),
        })
    }

    /// Drain all queued read-only inspect requests against `&Machine` (#309).
    pub fn drain_inspects(&self, machine: &Machine) {
        let tickets: Vec<InspectTicket> = {
            let mut state = self.lock();
            if state.pending_inspects.is_empty() {
                return;
            }
            state.pending_inspects.drain(..).collect()
        };
        for ticket in tickets {
            ticket.slot.complete(execute_inspect(machine, ticket.query));
        }
    }

    /// Submit a live `LLSNAP v2` snapshot or restore operation to run on
    /// `&mut Machine` at the active or next flip boundary (#313).
    #[must_use]
    pub fn request_snapshot_op(&self, op: SnapshotOp, timeout: Option<Duration>) -> SnapshotReply {
        let slot = TicketSlot::new();
        {
            let mut state = self.lock();
            if state.finished {
                return SnapshotReply::Error {
                    status: 410,
                    message: "run has finished".to_owned(),
                };
            }
            if !state.machine_attached {
                return SnapshotReply::Error {
                    status: 503,
                    message: "machine snapshot engine is not attached".to_owned(),
                };
            }
            state.pending_snapshots.push_back(SnapshotTicket {
                op,
                slot: Arc::clone(&slot),
            });
        }
        self.notify_host();

        if let Some(reply) = slot.wait(timeout) {
            return reply;
        }
        {
            let mut state = self.lock();
            state
                .pending_snapshots
                .retain(|t| !Arc::ptr_eq(&t.slot, &slot));
        }
        slot.try_take().unwrap_or_else(|| SnapshotReply::Error {
            status: 504,
            message: "snapshot operation timed out waiting for flip boundary".to_owned(),
        })
    }

    /// Drain all queued live snapshot/restore operations against `&mut Machine` (#313).
    pub fn drain_snapshots(&self, machine: &mut Machine, host: &mut dyn Host) {
        let tickets: Vec<SnapshotTicket> = {
            let mut state = self.lock();
            if state.pending_snapshots.is_empty() {
                return;
            }
            state.pending_snapshots.drain(..).collect()
        };
        for ticket in tickets {
            ticket
                .slot
                .complete(execute_snapshot_op(self, machine, host, ticket.op));
        }
    }

    /// List all saved in-memory `LLSNAP v2` snapshot slots (#313).
    #[must_use]
    pub fn list_snapshots(&self) -> Vec<SnapshotSummary> {
        self.lock()
            .snapshots
            .values()
            .map(SnapshotSlot::summary)
            .collect()
    }

    /// Capture and store a named checkpoint (`POST /api/bookmark`, #309, #313),
    /// backed by an instant `LLSNAP v2` live snapshot in addition to its route text.
    ///
    /// # Errors
    /// Returns an error if the bookmark name is empty or the machine cannot be snapshotted.
    pub fn save_bookmark(
        &self,
        name: String,
        timeout: Option<Duration>,
    ) -> Result<BookmarkData, String> {
        let trimmed = name.trim().to_owned();
        if trimmed.is_empty() {
            return Err("bookmark `name` must not be empty".to_owned());
        }
        match self.request_snapshot_op(SnapshotOp::SaveBookmark { name: trimmed }, timeout) {
            SnapshotReply::BookmarkSaved { data, .. } => Ok(data),
            SnapshotReply::Error { message, .. } => Err(message),
            _ => Err("unexpected bookmark capture reply".to_owned()),
        }
    }

    /// List all saved checkpoints.
    #[must_use]
    pub fn list_bookmarks(&self) -> Vec<(String, BookmarkData)> {
        self.lock()
            .bookmarks
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Rewind to a previously saved bookmark (`POST /api/rewind`, #309, #313):
    /// uses instant live `LLSNAP v2` restore when available, falling back to
    /// Tier-1 outer-loop route replay otherwise.
    ///
    /// # Errors
    /// Returns an error if the bookmark does not exist, a rewind is already in
    /// progress, the run has finished, or the rewind fails/times out.
    pub fn request_rewind(
        &self,
        name: &str,
        timeout: Option<Duration>,
    ) -> Result<RewindResult, String> {
        let has_v2 = {
            let state = self.lock();
            if state.finished {
                return Err("run has finished".to_owned());
            }
            if !state.machine_attached {
                return Err("machine rewind is not available".to_owned());
            }
            if !state.bookmarks.contains_key(name) {
                return Err(format!("unknown bookmark `{name}`"));
            }
            state.bookmark_snapshots.contains_key(name)
        };

        if has_v2 {
            return match self.request_snapshot_op(
                SnapshotOp::RewindBookmark {
                    name: name.to_owned(),
                },
                timeout,
            ) {
                SnapshotReply::Restored {
                    name,
                    flip,
                    cursor,
                    declared_ms,
                    digest,
                    expected_digest,
                    digest_match,
                    restore_ms,
                    mode,
                    ..
                } => Ok(RewindResult {
                    name: name.unwrap_or_default(),
                    flip,
                    cursor,
                    declared_ms,
                    digest: digest.clone(),
                    expected_digest: expected_digest.unwrap_or(digest),
                    digest_match,
                    mode,
                    restore_ms: Some(restore_ms),
                }),
                SnapshotReply::Error { message, .. } => Err(message),
                _ => Err("unexpected bookmark rewind reply".to_owned()),
            };
        }

        let slot = TicketSlot::new();
        {
            let mut state = self.lock();
            if state.finished {
                return Err("run has finished".to_owned());
            }
            if !state.machine_attached {
                return Err("machine rewind is not available".to_owned());
            }
            if state.pending_rewind.is_some() || state.replaying_rewind.is_some() {
                return Err("a rewind is already in progress".to_owned());
            }
            let Some(bookmark) = state.bookmarks.get(name).cloned() else {
                return Err(format!("unknown bookmark `{name}`"));
            };
            state.push_feed(
                "control",
                format!("rewind `{name}` -> flip {}", bookmark.flip),
            );
            state.pending_rewind = Some(PendingRewind {
                name: name.to_owned(),
                bookmark,
                slot: Arc::clone(&slot),
            });
            state.rewind_abort = true;
            state.active_wait = None;
        }
        self.notify_host();

        if let Some(reply) = slot.wait(timeout) {
            return reply;
        }
        {
            let mut state = self.lock();
            if state
                .pending_rewind
                .as_ref()
                .is_some_and(|rw| Arc::ptr_eq(&rw.slot, &slot))
            {
                state.pending_rewind = None;
                state.rewind_abort = false;
            }
            if state
                .replaying_rewind
                .as_ref()
                .is_some_and(|rw| Arc::ptr_eq(&rw.slot, &slot))
            {
                state.replaying_rewind = None;
                state.rewind_target_flip = None;
            }
        }
        slot.try_take()
            .unwrap_or_else(|| Err("rewind timed out".to_owned()))
    }

    #[must_use]
    pub fn is_rewind_aborting(&self) -> bool {
        self.lock().rewind_abort
    }

    #[must_use]
    pub fn rewind_target_flip(&self) -> Option<u64> {
        self.lock().rewind_target_flip
    }

    /// Called by `run.rs`'s outer loop after `Machine::run` stops for a rewind:
    /// transitions `pending_rewind` into `replaying_rewind`, resets per-run bus
    /// state while keeping `bookmarks` and `agent_state`, and returns the
    /// bookmark to replay.
    #[must_use]
    pub fn take_pending_rewind(&self) -> Option<(String, BookmarkData)> {
        let mut state = self.lock();
        let pending = state.pending_rewind.take()?;
        let name = pending.name.clone();
        let bm = pending.bookmark.clone();
        state.rewind_abort = false;
        state.rewind_target_flip = Some(bm.flip);
        state.replaying_rewind = Some(pending);
        state.step_budget = 0;
        state.active_wait = None;
        state.close_pending = false;
        state.force_closes = 0;
        state.finished = false;
        state.is_paused = false;
        state.outcome = None;
        state.digest = None;
        state.last_meta = None;
        state.latest_frame = None;
        state.held_keys.clear();
        state.held_buttons.clear();
        state.pending_releases.clear();
        state.timeline.clear();
        state.timeline.reset_polled();
        state.last_agent_action_flip = None;
        state.agent_timeline_end_flip = None;
        let mode = state.guide.mouse_mode;
        let res = state.guide.resolution;
        state.timeline.configure_mode(mode, res);
        state.recording_len = 0;
        state.reset_clock_nanos = 0;
        state.runtime_reset_epoch = state.runtime_reset_epoch.wrapping_add(1);
        Some((name, bm))
    }

    /// Check whether a rewind replay has reached its target flip, verify
    /// `state_digest()`, and wake the waiting `/api/rewind` caller.
    pub fn complete_rewind_if_reached(&self, machine: &Machine) -> bool {
        let reached = {
            let state = self.lock();
            state
                .rewind_target_flip
                .is_some_and(|target| machine.flips() >= target)
        };
        if !reached {
            return false;
        }
        let actual_digest = machine.state_digest();
        let actual_flip = machine.flips();
        let actual_cursor = machine.clock.cursor();
        let actual_ms = machine.clock.millis();
        let replaying = {
            let mut state = self.lock();
            state.rewind_target_flip = None;
            state.replaying_rewind.take()
        };
        if let Some(rw) = replaying {
            let digest_match = actual_digest == rw.bookmark.digest;
            let result = RewindResult {
                name: rw.name,
                flip: actual_flip,
                cursor: actual_cursor,
                declared_ms: actual_ms,
                digest: actual_digest,
                expected_digest: rw.bookmark.digest,
                digest_match,
                mode: "route_replay",
                restore_ms: None,
            };
            rw.slot.complete(Ok(result));
            true
        } else {
            false
        }
    }

    /// Fail an active rewind if `Machine::run` ended before reaching the target flip.
    pub fn fail_pending_rewind(&self, message: String) {
        let rw = {
            let mut state = self.lock();
            state.rewind_abort = false;
            state.rewind_target_flip = None;
            state
                .replaying_rewind
                .take()
                .or_else(|| state.pending_rewind.take())
        };
        if let Some(rw) = rw {
            rw.slot.complete(Err(message));
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
        self.dispatch_forced(cmd, false)
    }

    /// Non-blocking dispatch with explicit `force` flag to override title
    /// safety guards such as [`TitleGuide::escape_quits`] (#311).
    #[must_use]
    pub fn dispatch_forced(&self, cmd: Command, force: bool) -> Response {
        self.dispatch_forced_with_source(cmd, force, None)
    }

    #[must_use]
    pub(crate) fn dispatch_forced_with_source(
        &self,
        cmd: Command,
        force: bool,
        source: Option<&'static str>,
    ) -> Response {
        let mut state = self.lock();
        let prev_wait_epoch = state.wait_epoch;
        let prev_step_epoch = state.step_epoch;
        let (resp, wake) = apply_command(&mut state, cmd, force, source);
        let notify_client =
            state.wait_epoch != prev_wait_epoch || state.step_epoch != prev_step_epoch;
        drop(state);
        if wake {
            self.notify_host();
        }
        if notify_client {
            self.notify_client();
        }
        resp
    }

    /// Dispatch a command and, for advancing commands (`Step`, `Wait*`, or
    /// `Act`/`Batch`/`PipeRoute` with `auto_step: true`), block until the
    /// requested flips/wait condition finish (or the run ends, or `timeout`
    /// elapses).
    #[must_use]
    pub fn dispatch_sync(&self, cmd: Command, timeout: Option<Duration>) -> Response {
        self.dispatch_sync_forced(cmd, false, timeout)
    }

    /// Synchronous variant of [`Self::dispatch_forced`] (#311).
    #[must_use]
    pub fn dispatch_sync_forced(
        &self,
        cmd: Command,
        force: bool,
        timeout: Option<Duration>,
    ) -> Response {
        self.dispatch_sync_forced_with_source(cmd, force, None, timeout)
    }

    #[must_use]
    pub(crate) fn dispatch_sync_forced_with_source(
        &self,
        cmd: Command,
        force: bool,
        source: Option<&'static str>,
        timeout: Option<Duration>,
    ) -> Response {
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

        let (mut resp, wake) = apply_command(&mut state, cmd, force, source);
        if !resp.ok {
            drop(state);
            return resp;
        }
        if state.wait_epoch != start_wait_epoch || state.step_epoch != start_step_epoch {
            self.inner.wake_client.notify_all();
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

    #[cfg(test)]
    fn seed_route_bookmark(&self, name: &str, data: BookmarkData) {
        self.lock().bookmarks.insert(name.to_owned(), data);
    }

    #[cfg(test)]
    fn pending_op_counts(&self) -> (usize, usize, usize) {
        let state = self.lock();
        let rewinds = usize::from(state.pending_rewind.is_some())
            + usize::from(state.replaying_rewind.is_some());
        (
            state.pending_inspects.len(),
            state.pending_snapshots.len(),
            rewinds,
        )
    }
}

fn fail_pending_ops(
    inspects: Vec<InspectTicket>,
    snapshots: Vec<SnapshotTicket>,
    rewinds: Vec<PendingRewind>,
    reason: &str,
) {
    for ticket in inspects {
        ticket.slot.complete(InspectReply::Error {
            status: 410,
            message: reason.to_owned(),
        });
    }
    for ticket in snapshots {
        ticket.slot.complete(SnapshotReply::Error {
            status: 410,
            message: reason.to_owned(),
        });
    }
    for rw in rewinds {
        rw.slot
            .complete(Err(format!("{reason} before rewind completed")));
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

fn action_presses_escape(action: &Action) -> bool {
    match action {
        Action::KeyHold { key, .. } => resolve_key(key) == Some(DIK_ESCAPE),
        Action::Chord { keys, .. } => keys.iter().any(|k| resolve_key(k) == Some(DIK_ESCAPE)),
        _ => false,
    }
}

fn route_presses_escape(route_text: &str) -> bool {
    let Ok(route) = parse_pipe_route(route_text, 0, false) else {
        return false;
    };
    route.due(0, (u64::MAX, u64::MAX)).iter().any(|(_, input)| {
        matches!(
            input,
            Input::Key {
                code: DIK_ESCAPE,
                down: true
            }
        )
    })
}

fn apply_guide_action_hints(guide: &TitleGuide, action: &Action) -> Action {
    match action {
        Action::MouseDelta {
            dx,
            dy,
            flips,
            scale_x,
            scale_y,
        } => {
            if let Some((gx, gy)) = guide.relative_steered_scale() {
                Action::MouseDelta {
                    dx: *dx,
                    dy: *dy,
                    flips: *flips,
                    scale_x: scale_x.or(Some(gx)),
                    scale_y: scale_y.or(Some(gy)),
                }
            } else {
                action.clone()
            }
        }
        _ => action.clone(),
    }
}

const ESCAPE_QUITS_ERROR: &str =
    "Escape immediately quits this title (`escape_quits: true`); pass `force: true` to confirm exit";

fn apply_command(
    state: &mut BusState,
    cmd: Command,
    force: bool,
    source_override: Option<&'static str>,
) -> (Response, bool) {
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
            (ok_response(state, ResponseData::None), false)
        }
        Command::Step { flips } => {
            if flips > 0 {
                state.step_budget = state.step_budget.saturating_add(flips);
                state.push_feed("control", format!("step {flips}"));
            }
            (ok_response(state, ResponseData::None), flips > 0)
        }
        Command::Act { action, auto_step } => {
            if state.guide.escape_quits && !force && action_presses_escape(&action) {
                return (err_response(state, ESCAPE_QUITS_ERROR), false);
            }
            let effective = apply_guide_action_hints(state.guide, &action);
            let flip = state.current_flip();
            let ms = state.current_declared_ms();
            let summary = effective.summary();
            let source = source_override.unwrap_or(if state.owner == InputOwner::Human {
                "human"
            } else {
                "agent"
            });
            match state.timeline.schedule(&effective, flip, ms) {
                Ok(report) => {
                    if source == "agent" && report.events_scheduled > 0 {
                        state.last_agent_action_flip = Some(state.observed_flip());
                        state.agent_timeline_end_flip = Some(
                            state
                                .agent_timeline_end_flip
                                .map_or(report.end_flip, |prev| prev.max(report.end_flip)),
                        );
                    }
                    state.push_feed(source, summary);
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
            if state.guide.escape_quits && !force && actions.iter().any(action_presses_escape) {
                return (err_response(state, ESCAPE_QUITS_ERROR), false);
            }
            let flip = state.current_flip();
            let ms = state.current_declared_ms();
            let source = source_override.unwrap_or(if state.owner == InputOwner::Human {
                "human"
            } else {
                "agent"
            });
            let mut staged = state.timeline.clone();
            let mut summaries = Vec::with_capacity(actions.len());
            let mut total_events = 0usize;
            let mut max_end = flip;
            let mut max_req = 0u64;
            for action in &actions {
                let effective = apply_guide_action_hints(state.guide, action);
                let summary = effective.summary();
                match staged.schedule(&effective, flip, ms) {
                    Ok(report) => {
                        summaries.push(summary);
                        total_events += report.events_scheduled;
                        max_end = max_end.max(report.end_flip);
                        max_req = max_req.max(report.required_flips);
                    }
                    Err(err) => return (err_response(state, err.to_string()), false),
                }
            }
            state.timeline = staged;
            if source == "agent" && total_events > 0 {
                state.last_agent_action_flip = Some(state.observed_flip());
                state.agent_timeline_end_flip = Some(
                    state
                        .agent_timeline_end_flip
                        .map_or(max_end, |prev| prev.max(max_end)),
                );
            }
            for summary in summaries {
                state.push_feed(source, summary);
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
            if state.guide.escape_quits && !force && route_presses_escape(&route_text) {
                return (err_response(state, ESCAPE_QUITS_ERROR), false);
            }
            let flip = state.current_flip();
            match state.timeline.pipe_route(&route_text, flip, relative) {
                Ok(report) => {
                    if state.owner != InputOwner::Human && report.events_scheduled > 0 {
                        state.last_agent_action_flip = Some(state.observed_flip());
                        state.agent_timeline_end_flip = Some(
                            state
                                .agent_timeline_end_flip
                                .map_or(report.end_flip, |prev| prev.max(report.end_flip)),
                        );
                    }
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
            let wait_summary = match (flips.map(|f| f.max(1)), ms) {
                (Some(f), Some(m)) => format!("wait {f}f/{m}ms"),
                (Some(f), None) => format!("wait {f}f"),
                (None, Some(m)) => format!("wait {m}ms"),
                (None, None) => "wait 1f".to_owned(),
            };
            state.push_feed("control", wait_summary);
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
            state.pace_epoch = state.pace_epoch.wrapping_add(1);
            state.push_feed("control", "pause".to_owned());
            (ok_response(state, ResponseData::None), false)
        }
        Command::Resume => {
            state.pause_requested = false;
            if state.mode == ExecMode::Step {
                state.mode = ExecMode::Live;
            }
            state.pace_epoch = state.pace_epoch.wrapping_add(1);
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
            state.pace_epoch = state.pace_epoch.wrapping_add(1);
            state.push_feed("control", format!("mode {}", mode.as_str()));
            (ok_response(state, ResponseData::None), true)
        }
        Command::SetOwner(owner) => {
            state.owner = owner;
            if owner == InputOwner::Human {
                state.last_agent_action_flip = None;
                state.agent_timeline_end_flip = None;
            }
            state.push_feed("control", format!("owner {}", owner.as_str()));
            (ok_response(state, ResponseData::None), false)
        }
        Command::SetPace(pace) => {
            state.pace = clamp_pace(pace);
            state.pace_epoch = state.pace_epoch.wrapping_add(1);
            state.push_feed("control", format!("pace {:.2}x", state.pace));
            (ok_response(state, ResponseData::None), false)
        }
        Command::ClearQueue => {
            let (cleared, unpaired_releases) = state.timeline.clear_queue();
            for ev in unpaired_releases {
                let is_held = match ev {
                    Input::Key { code, down: false } => state.held_keys.contains(&code),
                    Input::Up(btn) => state.held_buttons.contains(&btn),
                    _ => false,
                };
                if is_held && !state.pending_releases.contains(&ev) {
                    state.pending_releases.push(ev);
                }
            }
            let prev_budget = state.step_budget;
            state.step_budget = 0;
            if prev_budget > 0 && state.is_paused {
                state.step_epoch += 1;
            }
            if let Some(cond) = state.active_wait.take() {
                let start_flip = match cond {
                    WaitCondition::Fixed { start_flip, .. }
                    | WaitCondition::UntilChange { start_flip, .. }
                    | WaitCondition::UntilSettled { start_flip, .. } => start_flip,
                };
                state.last_wait_result = Some(WaitResult {
                    satisfied: false,
                    flips_elapsed: state.current_flip().saturating_sub(start_flip),
                    diff_pct: 0.0,
                });
                state.wait_epoch += 1;
            }
            state.last_agent_action_flip = None;
            state.agent_timeline_end_flip = None;
            state.push_feed("control", format!("clear_queue ({cleared} cleared)"));
            (ok_response(state, ResponseData::Cleared { cleared }), false)
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
fn diff_pct_between(a: &[u8], aw: u32, ah: u32, b: &[u8], bw: u32, bh: u32) -> f64 {
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

    #[test]
    fn escape_quits_guard_blocks_escape_without_force_and_allows_with_force() {
        const GUARDED: TitleGuide = TitleGuide {
            display_name: "Escape Guard Fixture",
            escape_quits: true,
            ..TitleGuide::EMPTY
        };
        let bus = ControlBus::new(ControlConfig {
            guide: &GUARDED,
            ..ControlConfig::default()
        });

        // KeyHold with "Escape", case-insensitive "escape", and hex "0x01" are blocked.
        for key in ["Escape", "escape", "0x01"] {
            let resp = bus.dispatch(Command::Act {
                action: Action::KeyHold {
                    key: key.to_owned(),
                    hold_flips: 1,
                    hold_ms: None,
                },
                auto_step: false,
            });
            assert!(!resp.ok, "expected `{key}` to be blocked");
            assert!(resp.error.as_deref().unwrap_or("").contains("force"));
            assert_eq!(resp.status.pending_actions, 0);
        }

        // Chord containing Escape is blocked.
        let resp = bus.dispatch(Command::Act {
            action: Action::Chord {
                keys: vec!["LeftShift".into(), "Escape".into()],
                hold_flips: 1,
                hold_ms: None,
                lead_flips: 1,
            },
            auto_step: false,
        });
        assert!(!resp.ok);
        assert_eq!(resp.status.pending_actions, 0);

        // Batch containing Escape is rejected atomically (first Space is NOT scheduled).
        let resp = bus.dispatch(Command::Batch {
            actions: vec![
                Action::KeyHold {
                    key: "Space".into(),
                    hold_flips: 1,
                    hold_ms: None,
                },
                Action::KeyHold {
                    key: "Escape".into(),
                    hold_flips: 1,
                    hold_ms: None,
                },
            ],
            auto_step: false,
        });
        assert!(!resp.ok);
        assert_eq!(resp.status.pending_actions, 0);

        // PipeRoute containing Escape (both bare and `+N`) is blocked without force.
        for route in [
            "#lego-legacy-route 1\nflip 0 key Escape down\nflip 1 key Escape up\n",
            "#lego-legacy-route 1\nflip +0 key Escape down\nflip +1 key Escape up\n",
        ] {
            let resp = bus.dispatch(Command::PipeRoute {
                route_text: route.to_owned(),
                relative: false,
                auto_step: false,
            });
            assert!(!resp.ok, "expected Escape in `{route}` to be blocked");
            assert_eq!(resp.status.pending_actions, 0);
        }

        // Non-Escape keys (including "1" = DIK_1 = 0x02 and "Space") are allowed.
        for key in ["Space", "1"] {
            let resp = bus.dispatch(Command::Act {
                action: Action::KeyHold {
                    key: key.to_owned(),
                    hold_flips: 1,
                    hold_ms: None,
                },
                auto_step: false,
            });
            assert!(resp.ok, "expected `{key}` to be allowed");
        }

        // With `force = true`, Escape is allowed for Act, Batch, and PipeRoute.
        let resp = bus.dispatch_forced(
            Command::Act {
                action: Action::KeyHold {
                    key: "Escape".into(),
                    hold_flips: 1,
                    hold_ms: None,
                },
                auto_step: false,
            },
            true,
        );
        assert!(resp.ok);

        let resp = bus.dispatch_forced(
            Command::PipeRoute {
                route_text: "#lego-legacy-route 1\nflip 0 key Escape down\nflip 1 key Escape up\n"
                    .to_owned(),
                relative: true,
                auto_step: false,
            },
            true,
        );
        assert!(resp.ok);
    }

    #[test]
    fn unguarded_title_allows_escape_without_force() {
        let bus = ControlBus::new(ControlConfig::default());
        let resp = bus.dispatch(Command::Act {
            action: Action::KeyHold {
                key: "Escape".into(),
                hold_flips: 1,
                hold_ms: None,
            },
            auto_step: false,
        });
        assert!(resp.ok);
    }

    #[test]
    fn action_feed_labels_human_source_when_owner_human_and_formats_wait_cleanly() {
        let bus = ControlBus::new(ControlConfig {
            owner: InputOwner::Agent,
            ..ControlConfig::default()
        });

        // Agent owner -> "agent" source.
        assert!(
            bus.dispatch(Command::Act {
                action: Action::Click {
                    x: Some(100),
                    y: Some(200),
                    button: crate::host::Button::Left,
                    hold_flips: 2,
                    hold_ms: None,
                    settle_flips: 1,
                },
                auto_step: false,
            })
            .ok
        );
        let last = bus.agent_state().action_feed.back().cloned().unwrap();
        assert_eq!(last.source, "agent");
        assert_eq!(last.summary, "click Left (100, 200) hold=2");

        // Switch owner to human -> Act & Batch record "human" source.
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Human)).ok);
        assert!(
            bus.dispatch(Command::Act {
                action: Action::KeyHold {
                    key: "Space".to_owned(),
                    hold_flips: 3,
                    hold_ms: None,
                },
                auto_step: false,
            })
            .ok
        );
        let last = bus.agent_state().action_feed.back().cloned().unwrap();
        assert_eq!(last.source, "human");
        assert_eq!(last.summary, "key_hold Space hold=3");

        assert!(
            bus.dispatch(Command::Batch {
                actions: vec![Action::Click {
                    x: None,
                    y: None,
                    button: crate::host::Button::Right,
                    hold_flips: 1,
                    hold_ms: None,
                    settle_flips: 1,
                }],
                auto_step: false,
            })
            .ok
        );
        let last = bus.agent_state().action_feed.back().cloned().unwrap();
        assert_eq!(last.source, "human");
        assert_eq!(last.summary, "click Right in-place hold=1");

        // Command::Wait formats cleanly without Rust Debug Some(...)/None (flips=0 clamps to 1f).
        for (flips, ms, expected) in [
            (Some(10), None, "wait 10f"),
            (Some(0), None, "wait 1f"),
            (None, Some(250), "wait 250ms"),
            (Some(5), Some(100), "wait 5f/100ms"),
            (None, None, "wait 1f"),
        ] {
            assert!(bus.dispatch(Command::Wait { flips, ms }).ok);
            let entry = bus.agent_state().action_feed.back().cloned().unwrap();
            assert_eq!(entry.summary, expected);
            assert!(!entry.summary.contains("Some(") && !entry.summary.contains("None"));
        }

        // Batch failure is atomic: timeline, action_feed, and last_agent_action_flip stay unchanged.
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Agent)).ok);
        assert!(bus.dispatch(Command::ClearQueue).ok);
        let feed_before = bus.agent_state().action_feed.len();
        let failed = bus.dispatch(Command::Batch {
            actions: vec![
                Action::KeyHold {
                    key: "Space".to_owned(),
                    hold_flips: 2,
                    hold_ms: None,
                },
                Action::KeyHold {
                    key: "NotARealKey".to_owned(),
                    hold_flips: 2,
                    hold_ms: None,
                },
            ],
            auto_step: false,
        });
        assert!(!failed.ok);
        assert_eq!(bus.status().pending_actions, 0);
        assert_eq!(bus.agent_state().action_feed.len(), feed_before);
        assert!(!bus.snapshot_for_sse().active_driving);
    }

    #[test]
    fn clear_queue_cancels_step_budget_and_wakes_blocked_wait() {
        use crate::control::host::ControlHost;
        use crate::host::headless::Headless;

        let flip_meta = |flip: u64| FlipMeta {
            flip,
            clock_nanos: flip * 16_000_000,
            cursor: flip * 100,
            instructions_passed: 0,
            pointer_shown: true,
            pointer_pos: (0, 0),
            route_active: false,
            route_remaining: 0,
        };

        let bus = ControlBus::new(ControlConfig {
            mode: ExecMode::Step,
            ..ControlConfig::default()
        });
        // Schedule a huge future pipe with auto_step=true, setting a large step_budget,
        // then clear it before the first flip so flip 0 must pause instead of burning 5011 flips.
        let sched = bus.dispatch(Command::PipeRoute {
            route_text: "flip 5000 down\nflip 5010 up\n".to_owned(),
            relative: false,
            auto_step: true,
        });
        assert!(sched.ok);
        assert_eq!(sched.status.pending_actions, 2);

        let clear = bus.dispatch(Command::ClearQueue);
        assert!(clear.ok);
        assert!(matches!(clear.data, ResponseData::Cleared { cleared: 2 }));
        assert_eq!(clear.status.pending_actions, 0);

        // Drive flips under a channel gate so each flip advances only when requested.
        let (tx, rx) = std::sync::mpsc::channel::<Option<u64>>();
        let host_bus = bus.clone();
        let host_thread = std::thread::spawn(move || {
            let mut host = ControlHost::with_bus(Headless::new(), host_bus);
            while let Ok(Some(flip)) = rx.recv() {
                host.observe_flip(&flip_meta(flip));
                let _ = host.poll_input(flip);
            }
        });

        // Flip 0 must pause immediately because ClearQueue zeroed step_budget.
        tx.send(Some(0)).unwrap();
        let paused = bus.wait_until_paused(Some(Duration::from_secs(2)));
        assert!(paused.paused);
        assert_eq!(paused.flip, Some(0));

        // Start a blocking Wait on another thread (host finishes flip 0 and waits on rx,
        // so flip 500 is never reached), then cancel via ClearQueue.
        let waiter_bus = bus.clone();
        let waiter = std::thread::spawn(move || {
            waiter_bus.dispatch_sync(
                Command::Wait {
                    flips: Some(500),
                    ms: None,
                },
                Some(Duration::from_secs(5)),
            )
        });
        for _ in 0..200 {
            if !bus.status().paused
                && bus
                    .agent_state()
                    .action_feed
                    .iter()
                    .any(|e| e.summary == "wait 500f")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!bus.status().paused);
        let clear_wait = bus.dispatch(Command::ClearQueue);
        assert!(clear_wait.ok);
        let wait_resp = waiter.join().unwrap();
        assert!(wait_resp.ok);
        assert!(matches!(
            wait_resp.data,
            ResponseData::WaitResult(WaitResult {
                satisfied: false,
                ..
            })
        ));

        // Pause at flip 1, start a huge Step-mode PipeRoute, cancel with ClearQueue,
        // then advance one flip (flip 2): stepper wakes cleanly at flip 2 (not 9000).
        tx.send(Some(1)).unwrap();
        let paused1 = bus.wait_until_paused(Some(Duration::from_secs(2)));
        assert!(paused1.paused);
        assert_eq!(paused1.flip, Some(1));

        let step_bus = bus.clone();
        let stepper = std::thread::spawn(move || {
            step_bus.dispatch_sync(
                Command::PipeRoute {
                    route_text: "flip 9000 down\nflip 9005 up\n".to_owned(),
                    relative: false,
                    auto_step: true,
                },
                Some(Duration::from_secs(5)),
            )
        });
        for _ in 0..200 {
            if bus.status().pending_actions == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(bus.status().pending_actions, 2);
        let clear_step = bus.dispatch(Command::ClearQueue);
        assert!(matches!(
            clear_step.data,
            ResponseData::Cleared { cleared: 2 }
        ));
        tx.send(Some(2)).unwrap();
        let step_resp = stepper.join().unwrap();
        assert!(step_resp.ok);
        assert_eq!(step_resp.status.pending_actions, 0);
        assert!(step_resp.status.flip.unwrap_or(u64::MAX) <= 2);

        tx.send(None).unwrap();
        assert!(bus.dispatch(Command::Close { force: true }).ok);
        host_thread.join().unwrap();
    }

    #[test]
    fn mark_finished_and_mark_dropped_unblock_concurrent_inspect_snapshot_and_rewind_waiters() {
        use crate::control::host::ControlHost;
        use crate::host::headless::Headless;
        use crate::machine::Outcome;

        let dummy_bookmark = BookmarkData {
            flip: 5,
            cursor: 500,
            declared_ms: 80,
            digest: "deadbeef".to_owned(),
            route_text: "#lego-legacy-route 1\n".to_owned(),
        };

        for via_drop in [false, true] {
            let host = ControlHost::new(Headless::new(), ControlConfig::default());
            let bus = host.bus();
            bus.set_machine_attached(true);
            bus.seed_route_bookmark("cp", dummy_bookmark.clone());

            let b_ins = bus.clone();
            let t_ins = std::thread::spawn(move || {
                b_ins.request_inspect(InspectQuery::Cpu, Some(Duration::from_secs(5)))
            });

            let b_snap = bus.clone();
            let t_snap = std::thread::spawn(move || {
                b_snap.request_snapshot_op(
                    SnapshotOp::Save {
                        name: Some("s".into()),
                        path: None,
                    },
                    Some(Duration::from_secs(5)),
                )
            });

            let b_rw = bus.clone();
            let t_rw =
                std::thread::spawn(move || b_rw.request_rewind("cp", Some(Duration::from_secs(5))));

            for _ in 0..500 {
                if bus.pending_op_counts() == (1, 1, 1) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(
                bus.pending_op_counts(),
                (1, 1, 1),
                "all three waiters must be queued concurrently"
            );

            let expected_reason = if via_drop {
                drop(host);
                "control host closed"
            } else {
                host.mark_finished(
                    &Finish {
                        outcome: Outcome::Exited { code: 0 },
                        cursor: 0,
                        flips: 0,
                        declared_millis: 0,
                    },
                    "0000000000000000",
                );
                "run has finished"
            };

            match t_ins.join().unwrap() {
                InspectReply::Error { status, message } => {
                    assert_eq!(status, 410);
                    assert_eq!(message, expected_reason);
                }
                other => panic!("expected 410 inspect error, got {other:?}"),
            }
            match t_snap.join().unwrap() {
                SnapshotReply::Error { status, message } => {
                    assert_eq!(status, 410);
                    assert_eq!(message, expected_reason);
                }
                other => panic!("expected 410 snapshot error, got {other:?}"),
            }
            let rw_err = t_rw.join().unwrap().expect_err("rewind must fail");
            assert_eq!(rw_err, format!("{expected_reason} before rewind completed"));
            assert_eq!(bus.pending_op_counts(), (0, 0, 0));
        }

        // Also verify a rewind already transitioned into `replaying_rewind` unblocks on finish.
        let bus = ControlBus::new(ControlConfig::default());
        bus.set_machine_attached(true);
        bus.seed_route_bookmark("cp2", dummy_bookmark);
        let b_rw = bus.clone();
        let t_rw =
            std::thread::spawn(move || b_rw.request_rewind("cp2", Some(Duration::from_secs(5))));
        for _ in 0..500 {
            if bus.is_rewind_aborting() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(bus.take_pending_rewind().is_some());
        bus.mark_finished(
            &Finish {
                outcome: Outcome::Exited { code: 0 },
                cursor: 0,
                flips: 0,
                declared_millis: 0,
            },
            "0000000000000000",
        );
        let rw_err = t_rw
            .join()
            .unwrap()
            .expect_err("replaying rewind must fail");
        assert_eq!(rw_err, "run has finished before rewind completed");
    }
}
