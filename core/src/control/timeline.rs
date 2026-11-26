//! Unified action timeline and piped-route scheduler (#308).
//!
//! Compiles high-level control actions (`move`, `mouse_delta`, `click`,
//! `click_poll`, `drag`, `key_hold`, `chord`, `type_text`, `pipe_route`) into
//! deterministic [`crate::host::Input`] events delivered through
//! [`crate::host::Host::poll_input`], where `Machine::deliver` automatically
//! records them into `#lego-legacy-route 1`.

use crate::host::{Button, Input};
use crate::route::{ParseError, Route};
use crate::title::MouseMode;
use std::fmt;

/// DirectInput scancode for `LeftShift`.
pub const DIK_LSHIFT: u32 = 0x2a;

/// High-level action scheduled onto [`Timeline`].
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Move the guest pointer to `(x, y)`, optionally gliding over `glide_flips`.
    Move { x: i32, y: i32, glide_flips: u64 },
    /// Apply a relative pointer delta `(dx, dy)` over `flips`, translated
    /// dynamically against `last_pointer` (updated on `put_pointer` and emitted
    /// `Move`s) so `Mouse::Steered` titles (e.g. Alpha Team `DDMouse`) work.
    MouseDelta {
        dx: f64,
        dy: f64,
        flips: u64,
        scale_x: Option<f64>,
        scale_y: Option<f64>,
    },
    /// Press and release a mouse button, optionally moving first, settling for
    /// `settle_flips` (only when coordinates are given so hover state latches
    /// before button-down), and releasing after `hold_flips` or `hold_ms`
    /// declared milliseconds.
    Click {
        x: Option<i32>,
        y: Option<i32>,
        button: Button,
        hold_flips: u64,
        hold_ms: Option<u64>,
        settle_flips: u64,
    },
    /// Press and release a mouse button using dual elapsed-flip + declared-ms
    /// release tracking so releases fire even inside static non-flipping poll
    /// loops (e.g. LEGO Chess `0x0040D220`).
    ClickPoll {
        x: Option<i32>,
        y: Option<i32>,
        button: Button,
        hold_flips: u64,
        hold_ms: u64,
    },
    /// Press `button` at `(from_x, from_y)`, step toward `(to_x, to_y)` in
    /// increments of `step_px`, then release one flip after reaching the end.
    Drag {
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        button: Button,
        step_px: u32,
    },
    /// Hold a single key (DirectInput name from `route::KEYS` or raw scancode)
    /// for `hold_flips` (and optionally `hold_ms` declared milliseconds).
    KeyHold {
        key: String,
        hold_flips: u64,
        hold_ms: Option<u64>,
    },
    /// Press and hold multiple keys, staggering earlier prefix keys `lead_flips`
    /// ahead of the final key and releasing them `lead_flips` after (default `1`
    /// via JSON/CLI/MCP; pass `0` for simultaneous press/release or static
    /// non-flipping poll loops with `hold_ms`).
    Chord {
        keys: Vec<String>,
        hold_flips: u64,
        hold_ms: Option<u64>,
        lead_flips: u64,
    },
    /// Type printable ASCII text (plus `\n`, `\t`, `\x08`) using `LeftShift`
    /// where needed, spaced by `flip_gap` flips between characters.
    TypeText { text: String, flip_gap: u64 },
}

fn opt_coords_label(x: Option<i32>, y: Option<i32>) -> String {
    match (x, y) {
        (Some(x), Some(y)) => format!("({x}, {y})"),
        _ => "in-place".to_owned(),
    }
}

fn hold_label(hold_flips: u64, hold_ms: Option<u64>) -> String {
    match hold_ms {
        Some(ms) => format!("{hold_flips}f/{ms}ms"),
        None => hold_flips.to_string(),
    }
}

impl Action {
    /// Short human-readable summary for the bounded action feed.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Move { x, y, glide_flips } => format!("move ({x}, {y}) glide={glide_flips}"),
            Self::MouseDelta { dx, dy, flips, .. } => {
                format!("mouse_delta ({dx:.1}, {dy:.1}) flips={flips}")
            }
            Self::Click {
                x,
                y,
                button,
                hold_flips,
                hold_ms,
                ..
            } => format!(
                "click {button:?} {} hold={}",
                opt_coords_label(*x, *y),
                hold_label(*hold_flips, *hold_ms),
            ),
            Self::ClickPoll {
                x,
                y,
                button,
                hold_flips,
                hold_ms,
            } => format!(
                "click_poll {button:?} {} hold={hold_flips}f/{hold_ms}ms",
                opt_coords_label(*x, *y),
            ),
            Self::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                ..
            } => format!("drag {button:?} ({from_x}, {from_y}) -> ({to_x}, {to_y})"),
            Self::KeyHold {
                key,
                hold_flips,
                hold_ms,
            } => format!("key_hold {key} hold={}", hold_label(*hold_flips, *hold_ms)),
            Self::Chord {
                keys,
                hold_flips,
                hold_ms,
                ..
            } => format!(
                "chord [{}] hold={}",
                keys.join("+"),
                hold_label(*hold_flips, *hold_ms),
            ),
            Self::TypeText { text, flip_gap } => {
                format!("type_text len={} gap={flip_gap}", text.len())
            }
        }
    }
}

/// Error returned when an action or piped route cannot be scheduled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionError {
    UnknownKey(String),
    EmptyChord,
    UnsupportedChar(char),
    RouteParse(ParseError),
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey(k) => write!(f, "unknown DirectInput key `{k}`"),
            Self::EmptyChord => write!(f, "chord requires at least one key"),
            Self::UnsupportedChar(ch) => {
                write!(f, "unsupported character in type_text: `{ch:?}`")
            }
            Self::RouteParse(err) => write!(f, "invalid route: {err}"),
        }
    }
}

impl std::error::Error for ActionError {}

/// Summary of events compiled into the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScheduleReport {
    pub events_scheduled: usize,
    pub end_flip: u64,
    pub required_flips: u64,
}

/// Resolve a DirectInput key name (matching `route::KEYS` case-insensitively),
/// decimal scancode, or `0x`-prefixed hex scancode.
#[must_use]
pub fn resolve_key(name: &str) -> Option<u32> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).ok();
    }
    let probe = format!("flip 0 key {trimmed} down\n");
    let route = Route::parse(&probe).ok()?;
    match route.at(0).first() {
        Some(Input::Key { code, .. }) => Some(*code),
        _ => None,
    }
}

/// Map an ASCII character to `(needs_left_shift, direct_input_scancode)`.
#[must_use]
pub fn ascii_to_key(ch: char) -> Option<(bool, u32)> {
    let pair = match ch {
        '\n' | '\r' => (false, 0x1c), // Return
        '\t' => (false, 0x0f),        // Tab
        '\x08' => (false, 0x0e),      // Backspace
        ' ' => (false, 0x39),         // Space
        '1' => (false, 0x02),
        '!' => (true, 0x02),
        '2' => (false, 0x03),
        '@' => (true, 0x03),
        '3' => (false, 0x04),
        '#' => (true, 0x04),
        '4' => (false, 0x05),
        '$' => (true, 0x05),
        '5' => (false, 0x06),
        '%' => (true, 0x06),
        '6' => (false, 0x07),
        '^' => (true, 0x07),
        '7' => (false, 0x08),
        '&' => (true, 0x08),
        '8' => (false, 0x09),
        '*' => (true, 0x09),
        '9' => (false, 0x0a),
        '(' => (true, 0x0a),
        '0' => (false, 0x0b),
        ')' => (true, 0x0b),
        '-' => (false, 0x0c),
        '_' => (true, 0x0c),
        '=' => (false, 0x0d),
        '+' => (true, 0x0d),
        'q' => (false, 0x10),
        'Q' => (true, 0x10),
        'w' => (false, 0x11),
        'W' => (true, 0x11),
        'e' => (false, 0x12),
        'E' => (true, 0x12),
        'r' => (false, 0x13),
        'R' => (true, 0x13),
        't' => (false, 0x14),
        'T' => (true, 0x14),
        'y' => (false, 0x15),
        'Y' => (true, 0x15),
        'u' => (false, 0x16),
        'U' => (true, 0x16),
        'i' => (false, 0x17),
        'I' => (true, 0x17),
        'o' => (false, 0x18),
        'O' => (true, 0x18),
        'p' => (false, 0x19),
        'P' => (true, 0x19),
        '[' => (false, 0x1a),
        '{' => (true, 0x1a),
        ']' => (false, 0x1b),
        '}' => (true, 0x1b),
        'a' => (false, 0x1e),
        'A' => (true, 0x1e),
        's' => (false, 0x1f),
        'S' => (true, 0x1f),
        'd' => (false, 0x20),
        'D' => (true, 0x20),
        'f' => (false, 0x21),
        'F' => (true, 0x21),
        'g' => (false, 0x22),
        'G' => (true, 0x22),
        'h' => (false, 0x23),
        'H' => (true, 0x23),
        'j' => (false, 0x24),
        'J' => (true, 0x24),
        'k' => (false, 0x25),
        'K' => (true, 0x25),
        'l' => (false, 0x26),
        'L' => (true, 0x26),
        ';' => (false, 0x27),
        ':' => (true, 0x27),
        '\'' => (false, 0x28),
        '"' => (true, 0x28),
        '`' => (false, 0x29),
        '~' => (true, 0x29),
        '\\' => (false, 0x2b),
        '|' => (true, 0x2b),
        'z' => (false, 0x2c),
        'Z' => (true, 0x2c),
        'x' => (false, 0x2d),
        'X' => (true, 0x2d),
        'c' => (false, 0x2e),
        'C' => (true, 0x2e),
        'v' => (false, 0x2f),
        'V' => (true, 0x2f),
        'b' => (false, 0x30),
        'B' => (true, 0x30),
        'n' => (false, 0x31),
        'N' => (true, 0x31),
        'm' => (false, 0x32),
        'M' => (true, 0x32),
        ',' => (false, 0x33),
        '<' => (true, 0x33),
        '.' => (false, 0x34),
        '>' => (true, 0x34),
        '/' => (false, 0x35),
        '?' => (true, 0x35),
        _ => return None,
    };
    Some(pair)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    Point {
        flip: u64,
        poll: u64,
    },
    FlipOrMs {
        min_flip: u64,
        min_poll: u64,
        flip: u64,
        declared_ms: u64,
    },
    /// Arms into `FlipOrMs` once `flip >= arm_flip`, measuring `hold_ms` from
    /// the `declared_ms` observed when `arm_flip` arrives so a settle gap,
    /// multi-flip relative move, or chord prefix lead never fires `Up` early.
    AfterFlipOrMs {
        arm_flip: u64,
        flip: u64,
        hold_ms: u64,
    },
    Ms {
        min_flip: u64,
        declared_ms: u64,
    },
}

impl Due {
    fn is_due(self, flip: u64, poll: u64, declared_ms: u64) -> bool {
        match self {
            Self::Point { flip: f, poll: p } => (flip, poll) >= (f, p),
            Self::FlipOrMs {
                min_flip,
                min_poll,
                flip: f,
                declared_ms: ms,
            } => {
                flip >= f
                    || ((flip > min_flip || (flip == min_flip && poll > min_poll))
                        && declared_ms >= ms)
            }
            Self::AfterFlipOrMs { .. } => false,
            Self::Ms {
                min_flip,
                declared_ms: ms,
            } => (flip > min_flip || (flip == min_flip && poll > 0)) && declared_ms >= ms,
        }
    }

    fn sort_flip(self) -> u64 {
        match self {
            Self::Point { flip, .. }
            | Self::FlipOrMs { flip, .. }
            | Self::AfterFlipOrMs { flip, .. } => flip,
            Self::Ms { min_flip, .. } => min_flip,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Payload {
    Input(Input),
    MouseDeltaStep { dx: f64, dy: f64 },
}

#[derive(Clone, Copy, Debug)]
struct Scheduled {
    seq: u64,
    due: Due,
    payload: Payload,
}

/// Deterministic timeline that expands high-level actions and piped routes into
/// `Host::Input` events.
///
/// Tracks the Win32 hardware pointer anchor (`hw_pointer`, mirroring
/// `win32::State::last_move`) separately from the rendered in-game software
/// cursor (`visual_cursor`) so relative-mouse titles never lose alignment on
/// `SetCursorPos` recenter calls (#327).
#[derive(Clone, Debug)]
pub struct Timeline {
    next_seq: u64,
    pending: Vec<Scheduled>,
    last_polled_flip: Option<u64>,
    hw_pointer: (i32, i32),
    visual_cursor: (f64, f64),
    pointer_shown: bool,
    pointer_speed: (f64, f64),
    mouse_mode: MouseMode,
    frame_size: (u32, u32),
    owed_delta: (f64, f64),
}

impl Timeline {
    #[must_use]
    pub fn new(pointer_speed: (f64, f64)) -> Self {
        Self {
            next_seq: 0,
            pending: Vec::new(),
            last_polled_flip: None,
            hw_pointer: (320, 240),
            visual_cursor: (320.0, 240.0),
            pointer_shown: true,
            pointer_speed,
            mouse_mode: MouseMode::AbsoluteKept,
            frame_size: (0, 0),
            owed_delta: (0.0, 0.0),
        }
    }

    fn clamp_visual(&self, x: f64, y: f64) -> (f64, f64) {
        if self.frame_size.0 > 0 && self.frame_size.1 > 0 {
            let max_x = f64::from(self.frame_size.0.saturating_sub(1));
            let max_y = f64::from(self.frame_size.1.saturating_sub(1));
            (x.clamp(0.0, max_x), y.clamp(0.0, max_y))
        } else {
            (x, y)
        }
    }

    fn clamp_screen_i32(&self, x: i32, y: i32) -> (i32, i32) {
        if self.frame_size.0 > 0 && self.frame_size.1 > 0 {
            (
                x.clamp(0, (self.frame_size.0 as i32).saturating_sub(1)),
                y.clamp(0, (self.frame_size.1 as i32).saturating_sub(1)),
            )
        } else {
            (x, y)
        }
    }

    fn active_scale(&self) -> (f64, f64) {
        let sane = |v: f64| if v.is_finite() && v > 0.0 { v } else { 1.0 };
        (sane(self.pointer_speed.0), sane(self.pointer_speed.1))
    }

    /// Configure declared [`MouseMode`] and frame resolution, initializing
    /// `hw_pointer` and `visual_cursor` to match Win32 / guest startup state (#327).
    pub fn configure_mode(&mut self, mode: MouseMode, resolution: (u32, u32)) {
        self.mouse_mode = mode;
        if resolution.0 > 0 && resolution.1 > 0 {
            self.frame_size = resolution;
        }
        let center = if self.frame_size.0 > 0 && self.frame_size.1 > 0 {
            (self.frame_size.0 as i32 / 2, self.frame_size.1 as i32 / 2)
        } else {
            (320, 240)
        };
        let (hw, vis) = match mode {
            MouseMode::AbsoluteKept | MouseMode::SoftwareAbsolute { .. } => {
                (center, (f64::from(center.0), f64::from(center.1)))
            }
            MouseMode::SoftwareRelative { start_pos, .. } => (
                center,
                self.clamp_visual(f64::from(start_pos.0), f64::from(start_pos.1)),
            ),
            MouseMode::RelativeSteered {
                recenter,
                start_corner,
                ..
            } => (
                recenter,
                self.clamp_visual(f64::from(start_corner.0), f64::from(start_corner.1)),
            ),
        };
        self.hw_pointer = hw;
        self.visual_cursor = vis;
        self.pointer_shown = matches!(mode, MouseMode::AbsoluteKept);
        self.owed_delta = (0.0, 0.0);
    }

    /// Update the tracked Win32 hardware pointer anchor (from `Host::put_pointer`
    /// or non-relative `observe_flip` reconciliation).
    ///
    /// On relative modes (`SoftwareRelative`, `RelativeSteered`), only the
    /// hardware anchor changes — `visual_cursor` and `owed_delta` are preserved
    /// so `SetCursorPos` transitions and per-flip recenters never pin or shift
    /// the reported in-game cursor (#327).
    pub fn set_pointer(&mut self, x: i32, y: i32) {
        self.hw_pointer = (x, y);
        if !self.mouse_mode.is_relative() {
            self.visual_cursor = (f64::from(x), f64::from(y));
        }
    }

    /// Explicitly restore the tracked visual cursor position (e.g. after live
    /// snapshot restore on relative-mouse titles).
    pub fn set_visual_cursor(&mut self, x: i32, y: i32) {
        self.visual_cursor = self.clamp_visual(f64::from(x), f64::from(y));
    }

    /// Record one emitted `Input::Move { x, y }` (whether from this timeline,
    /// `PipeRoute`, or a human window in `Human`/`Shared` mode) (#327).
    pub fn observe_emitted_move(&mut self, x: i32, y: i32) {
        let prev = self.hw_pointer;
        self.hw_pointer = (x, y);
        if self.mouse_mode.is_relative() {
            let (sx, sy) = self.active_scale();
            let dx = f64::from(x.saturating_sub(prev.0)) / sx;
            let dy = f64::from(y.saturating_sub(prev.1)) / sy;
            self.visual_cursor =
                self.clamp_visual(self.visual_cursor.0 + dx, self.visual_cursor.1 + dy);
        } else {
            self.visual_cursor = (f64::from(x), f64::from(y));
        }
    }

    /// Update whether the host pointer is shown (`false` means `Mouse::Steered`).
    pub fn set_pointer_shown(&mut self, shown: bool) {
        self.pointer_shown = shown;
    }

    /// Update the active guest frame dimensions used to clamp relative moves.
    ///
    /// When the guest switches display resolution mid-run, `ddraw::set_display_mode`
    /// resets `state.last_move` to the new screen center; mirror that here so
    /// subsequent relative deltas stay aligned.
    pub fn set_frame_size(&mut self, width: u32, height: u32) {
        let changed = width > 0
            && height > 0
            && self.frame_size != (0, 0)
            && self.frame_size != (width, height);
        self.frame_size = (width, height);
        if changed {
            self.hw_pointer = (width as i32 / 2, height as i32 / 2);
            if !self.mouse_mode.is_relative() {
                self.visual_cursor = (f64::from(self.hw_pointer.0), f64::from(self.hw_pointer.1));
            }
        }
        if self.mouse_mode.is_relative() {
            self.visual_cursor = self.clamp_visual(self.visual_cursor.0, self.visual_cursor.1);
        }
    }

    #[must_use]
    pub fn mouse_mode(&self) -> MouseMode {
        self.mouse_mode
    }

    /// Win32 hardware pointer anchor (`last_move`).
    #[must_use]
    pub fn last_pointer(&self) -> (i32, i32) {
        self.hw_pointer
    }

    /// Tracked rendered in-game cursor coordinates in guest pixel space (#327).
    #[must_use]
    pub fn visual_cursor_pos(&self) -> (i32, i32) {
        let vx = self.visual_cursor.0.round() as i32;
        let vy = self.visual_cursor.1.round() as i32;
        if self.mouse_mode.is_relative() {
            self.clamp_screen_i32(vx, vy)
        } else {
            (vx, vy)
        }
    }

    #[must_use]
    pub fn pointer_shown(&self) -> bool {
        self.pointer_shown
    }

    /// Whether unemitted owed relative delta (> 0.5 px) remains for subsequent
    /// recenter flips (strict `> 0.5` paired with `round_ties_even` prevents
    /// half-integer `±0.5` oscillation).
    #[must_use]
    pub fn has_owed_delta(&self) -> bool {
        self.owed_delta.0.abs() > 0.5 || self.owed_delta.1.abs() > 0.5
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len() + usize::from(self.has_owed_delta())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && !self.has_owed_delta()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.owed_delta = (0.0, 0.0);
    }

    fn clamps_hw_pointer(&self) -> bool {
        self.frame_size.0 > 0
            && self.frame_size.1 > 0
            && (self.mouse_mode.recenters_every_flip()
                || (!self.mouse_mode.is_relative() && !self.pointer_shown))
    }

    fn min_flips_for_raw_delta(&self, raw_dx: f64, raw_dy: f64) -> u64 {
        if !self.mouse_mode.recenters_every_flip()
            || self.frame_size.0 == 0
            || self.frame_size.1 == 0
        {
            return 1;
        }
        let max_x = (self.frame_size.0 as i32).saturating_sub(1);
        let max_y = (self.frame_size.1 as i32).saturating_sub(1);
        let anchor = match self.mouse_mode {
            MouseMode::RelativeSteered { recenter, .. } => {
                (recenter.0.clamp(0, max_x), recenter.1.clamp(0, max_y))
            }
            _ => (
                self.hw_pointer.0.clamp(0, max_x),
                self.hw_pointer.1.clamp(0, max_y),
            ),
        };
        let axis = |delta: f64, pos: i32, max: i32| -> u64 {
            if !delta.is_finite() || delta == 0.0 {
                return 1;
            }
            let cap = if delta > 0.0 {
                (max - pos).max(1)
            } else {
                pos.max(1)
            };
            (delta.abs() / f64::from(cap)).ceil().min(1024.0) as u64
        };
        axis(raw_dx, anchor.0, max_x)
            .max(axis(raw_dy, anchor.1, max_y))
            .max(1)
    }

    fn next_relative_flip(&self, start_flip: u64) -> u64 {
        let mut base = start_flip;
        for item in &self.pending {
            if matches!(item.payload, Payload::MouseDeltaStep { .. }) {
                base = base.max(item.due.sort_flip().saturating_add(1));
            }
        }
        if base == start_flip && self.has_owed_delta() {
            let payback = self.min_flips_for_raw_delta(self.owed_delta.0, self.owed_delta.1);
            base = base.max(start_flip.saturating_add(payback));
        }
        base
    }

    fn planned_visual(&self) -> (f64, f64) {
        let (mut raw_x, mut raw_y) = self.owed_delta;
        for item in &self.pending {
            if let Payload::MouseDeltaStep { dx, dy } = item.payload {
                raw_x += dx;
                raw_y += dy;
            }
        }
        let (sx, sy) = self.active_scale();
        self.clamp_visual(
            self.visual_cursor.0 + raw_x / sx,
            self.visual_cursor.1 + raw_y / sy,
        )
    }

    fn schedule_relative_move_to(
        &mut self,
        target_x: i32,
        target_y: i32,
        start_flip: u64,
        min_steps: u64,
    ) -> u64 {
        let effective_start = self.next_relative_flip(start_flip);
        let (tx, ty) = self.clamp_screen_i32(target_x, target_y);
        let (cur_x, cur_y) = self.planned_visual();
        let (sx, sy) = self.active_scale();
        let raw_dx = (f64::from(tx) - cur_x) * sx;
        let raw_dy = (f64::from(ty) - cur_y) * sy;
        let steps = min_steps
            .max(self.min_flips_for_raw_delta(raw_dx, raw_dy))
            .max(1);
        let step_dx = raw_dx / steps as f64;
        let step_dy = raw_dy / steps as f64;
        for i in 0..steps {
            self.push(
                Due::Point {
                    flip: effective_start.saturating_add(i),
                    poll: 0,
                },
                Payload::MouseDeltaStep {
                    dx: step_dx,
                    dy: step_dy,
                },
            );
        }
        effective_start.saturating_add(steps - 1)
    }

    fn schedule_optional_move(&mut self, x: Option<i32>, y: Option<i32>, start_flip: u64) -> u64 {
        match (x, y) {
            (Some(x), Some(y)) if self.mouse_mode.is_relative() => {
                self.schedule_relative_move_to(x, y, start_flip, 1)
            }
            (Some(x), Some(y)) => {
                self.push_input(start_flip, 0, Input::Move { x, y });
                start_flip
            }
            _ if self.mouse_mode.is_relative() => self.next_relative_flip(start_flip),
            _ => start_flip,
        }
    }

    fn flush_owed_delta(&mut self, force_emit: bool, out: &mut Vec<Input>) {
        if !self.owed_delta.0.is_finite() {
            self.owed_delta.0 = 0.0;
        }
        if !self.owed_delta.1.is_finite() {
            self.owed_delta.1 = 0.0;
        }
        // `round_ties_even()` maps ±0.5 to 0, preventing ±1 px oscillation.
        let want_x = self.owed_delta.0.round_ties_even() as i32;
        let want_y = self.owed_delta.1.round_ties_even() as i32;
        if !force_emit && want_x == 0 && want_y == 0 {
            return;
        }
        let raw_nx = self.hw_pointer.0.saturating_add(want_x);
        let raw_ny = self.hw_pointer.1.saturating_add(want_y);
        let (nx, ny) = if self.clamps_hw_pointer() {
            (
                raw_nx.clamp(0, (self.frame_size.0 as i32).saturating_sub(1)),
                raw_ny.clamp(0, (self.frame_size.1 as i32).saturating_sub(1)),
            )
        } else {
            (raw_nx, raw_ny)
        };
        let emitted_x = nx.saturating_sub(self.hw_pointer.0);
        let emitted_y = ny.saturating_sub(self.hw_pointer.1);
        if self.mouse_mode.recenters_every_flip() {
            // Per-flip recenter mode: carry clamped remainder into subsequent flips.
            self.owed_delta.0 -= f64::from(emitted_x);
            self.owed_delta.1 -= f64::from(emitted_y);
        } else {
            // Non-recentering mode: discard screen-edge overflow instead of wedging debt.
            self.owed_delta.0 = if nx != raw_nx {
                0.0
            } else {
                self.owed_delta.0 - f64::from(emitted_x)
            };
            self.owed_delta.1 = if ny != raw_ny {
                0.0
            } else {
                self.owed_delta.1 - f64::from(emitted_y)
            };
        }
        if !force_emit && emitted_x == 0 && emitted_y == 0 {
            return;
        }
        self.observe_emitted_move(nx, ny);
        out.push(Input::Move { x: nx, y: ny });
    }

    /// Record that `flip` has entered `Host::poll_input` so `Action::TypeText`
    /// and `Action::Chord` scheduled mid-frame (e.g. `Live` mode between
    /// `deliver_input(N)` and `observe_flip(N+1)`) advance past the already-polled
    /// flip (#345, #348).
    pub(crate) fn note_polled(&mut self, flip: u64) {
        self.last_polled_flip = self.last_polled_flip.max(Some(flip));
    }

    fn next_unpolled_flip(&self, start_flip: u64) -> u64 {
        self.last_polled_flip.map_or(start_flip, |polled| {
            start_flip.max(polled.saturating_add(1))
        })
    }

    /// Clear tracked polled-flip watermark across rewind or live snapshot restore.
    pub(crate) fn reset_polled(&mut self) {
        self.last_polled_flip = None;
    }

    fn push(&mut self, due: Due, payload: Payload) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending.push(Scheduled { seq, due, payload });
    }

    /// Schedule one raw [`Input`] at `(flip, poll)`.
    pub fn push_input(&mut self, flip: u64, poll: u64, input: Input) {
        self.push(Due::Point { flip, poll }, Payload::Input(input));
    }

    /// Schedule a high-level [`Action`] beginning at `(start_flip, start_ms)`.
    ///
    /// # Errors
    /// Returns [`ActionError`] if a key name is unknown, a chord is empty, or
    /// `type_text` contains an unsupported character.
    pub fn schedule(
        &mut self,
        action: &Action,
        mut start_flip: u64,
        start_ms: u64,
    ) -> Result<ScheduleReport, ActionError> {
        let before = self.pending.len();
        let end_flip = match action {
            Action::Move { x, y, glide_flips } => {
                let steps = (*glide_flips).max(1);
                if self.mouse_mode.is_relative() {
                    self.schedule_relative_move_to(*x, *y, start_flip, steps)
                } else if steps == 1 {
                    self.push_input(start_flip, 0, Input::Move { x: *x, y: *y });
                    start_flip
                } else {
                    let (x0, y0) = self.hw_pointer;
                    for i in 1..=steps {
                        let t = i as f64 / steps as f64;
                        let ix = if i == steps {
                            *x
                        } else {
                            (f64::from(x0) + f64::from(*x - x0) * t).round() as i32
                        };
                        let iy = if i == steps {
                            *y
                        } else {
                            (f64::from(y0) + f64::from(*y - y0) * t).round() as i32
                        };
                        self.push_input(start_flip + i - 1, 0, Input::Move { x: ix, y: iy });
                    }
                    start_flip + steps - 1
                }
            }
            Action::MouseDelta {
                dx,
                dy,
                flips,
                scale_x,
                scale_y,
            } => {
                let steps = (*flips).max(1);
                let (def_x, def_y) = if self.pointer_shown && !self.mouse_mode.is_relative() {
                    (1.0, 1.0)
                } else {
                    self.active_scale()
                };
                let sane_scale = |v: Option<f64>, d: f64| match v {
                    Some(s) if s.is_finite() && s > 0.0 => s,
                    _ => d,
                };
                let sx = sane_scale(*scale_x, def_x);
                let sy = sane_scale(*scale_y, def_y);
                let sane_d = |v: f64| {
                    if v.is_finite() {
                        v.clamp(-1_000_000.0, 1_000_000.0)
                    } else {
                        0.0
                    }
                };
                let total_dx = (sane_d(*dx) * sx).clamp(-1_000_000.0, 1_000_000.0);
                let total_dy = (sane_d(*dy) * sy).clamp(-1_000_000.0, 1_000_000.0);
                let step_dx = total_dx / steps as f64;
                let step_dy = total_dy / steps as f64;
                for i in 0..steps {
                    self.push(
                        Due::Point {
                            flip: start_flip.saturating_add(i),
                            poll: 0,
                        },
                        Payload::MouseDeltaStep {
                            dx: step_dx,
                            dy: step_dy,
                        },
                    );
                }
                let needed = steps.max(self.min_flips_for_raw_delta(
                    self.owed_delta.0 + total_dx,
                    self.owed_delta.1 + total_dy,
                ));
                start_flip.saturating_add(needed.saturating_sub(1))
            }
            Action::Click {
                x,
                y,
                button,
                hold_flips,
                hold_ms,
                settle_flips,
            } => {
                let move_flip = self.schedule_optional_move(*x, *y, start_flip);
                let down_flip = if x.is_some() && y.is_some() {
                    move_flip.saturating_add(*settle_flips)
                } else {
                    move_flip
                };
                self.push_input(down_flip, 0, Input::Down(*button));
                let hold = (*hold_flips).max(1);
                let rel_flip = down_flip.saturating_add(hold);
                let due = match hold_ms {
                    Some(ms) => Due::AfterFlipOrMs {
                        arm_flip: down_flip,
                        flip: rel_flip,
                        hold_ms: *ms,
                    },
                    None => Due::Point {
                        flip: rel_flip,
                        poll: 0,
                    },
                };
                self.push(due, Payload::Input(Input::Up(*button)));
                rel_flip
            }
            Action::ClickPoll {
                x,
                y,
                button,
                hold_flips,
                hold_ms,
            } => {
                let click_flip = self.schedule_optional_move(*x, *y, start_flip);
                self.push_input(click_flip, 0, Input::Down(*button));
                let target_ms = start_ms.saturating_add(*hold_ms);
                let (due, end_flip) = if *hold_flips == 0 {
                    (
                        Due::Ms {
                            min_flip: click_flip,
                            declared_ms: target_ms,
                        },
                        click_flip,
                    )
                } else {
                    let rel_flip = click_flip.saturating_add(*hold_flips);
                    (
                        Due::FlipOrMs {
                            min_flip: click_flip,
                            min_poll: 0,
                            flip: rel_flip,
                            declared_ms: target_ms,
                        },
                        rel_flip,
                    )
                };
                self.push(due, Payload::Input(Input::Up(*button)));
                end_flip
            }
            Action::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                step_px,
            } => {
                let dist = (*to_x - *from_x)
                    .unsigned_abs()
                    .max((*to_y - *from_y).unsigned_abs());
                let step = (*step_px).max(1);
                let steps = u64::from(dist.div_ceil(step).max(1));
                let drag_end = if self.mouse_mode.is_relative() {
                    let down_flip = self.schedule_relative_move_to(*from_x, *from_y, start_flip, 1);
                    self.push_input(down_flip, 0, Input::Down(*button));
                    self.schedule_relative_move_to(*to_x, *to_y, down_flip.saturating_add(1), steps)
                } else {
                    self.push_input(
                        start_flip,
                        0,
                        Input::Move {
                            x: *from_x,
                            y: *from_y,
                        },
                    );
                    self.push_input(start_flip, 0, Input::Down(*button));
                    for i in 1..=steps {
                        let t = i as f64 / steps as f64;
                        let ix = if i == steps {
                            *to_x
                        } else {
                            (f64::from(*from_x) + f64::from(*to_x - *from_x) * t).round() as i32
                        };
                        let iy = if i == steps {
                            *to_y
                        } else {
                            (f64::from(*from_y) + f64::from(*to_y - *from_y) * t).round() as i32
                        };
                        self.push_input(start_flip + i, 0, Input::Move { x: ix, y: iy });
                    }
                    start_flip + steps
                };
                let rel_flip = drag_end.saturating_add(1);
                self.push_input(rel_flip, 0, Input::Up(*button));
                rel_flip
            }
            Action::KeyHold {
                key,
                hold_flips,
                hold_ms,
            } => {
                let code = resolve_key(key).ok_or_else(|| ActionError::UnknownKey(key.clone()))?;
                self.push_input(start_flip, 0, Input::Key { code, down: true });
                let hold = (*hold_flips).max(1);
                let rel_flip = start_flip.saturating_add(hold);
                let due = match hold_ms {
                    Some(ms) => Due::FlipOrMs {
                        min_flip: start_flip,
                        min_poll: 0,
                        flip: rel_flip,
                        declared_ms: start_ms.saturating_add(*ms),
                    },
                    None => Due::Point {
                        flip: rel_flip,
                        poll: 0,
                    },
                };
                self.push(due, Payload::Input(Input::Key { code, down: false }));
                rel_flip
            }
            Action::Chord {
                keys,
                hold_flips,
                hold_ms,
                lead_flips,
            } => {
                if keys.is_empty() {
                    return Err(ActionError::EmptyChord);
                }
                let mut codes = Vec::with_capacity(keys.len());
                for k in keys {
                    let code = resolve_key(k).ok_or_else(|| ActionError::UnknownKey(k.clone()))?;
                    if !codes.contains(&code) {
                        codes.push(code);
                    }
                }
                let hold = (*hold_flips).max(1);
                let lead = if codes.len() > 1 { *lead_flips } else { 0 };
                // Preserve static non-flipping poll-loop delivery when caller opts
                // out of lead stagger with `hold_ms`; otherwise advance past an
                // already-polled flip so `Live`-mode chords never collapse.
                if lead > 0 || hold_ms.is_none() {
                    start_flip = self.next_unpolled_flip(start_flip);
                }
                let (&final_code, prefix_codes) =
                    codes.split_last().expect("non-empty chord has final key");
                for &code in prefix_codes {
                    self.push_input(start_flip, 0, Input::Key { code, down: true });
                }
                let action_down_flip = start_flip.saturating_add(lead);
                self.push_input(
                    action_down_flip,
                    0,
                    Input::Key {
                        code: final_code,
                        down: true,
                    },
                );
                let action_rel_flip = action_down_flip.saturating_add(hold);
                let prefix_rel_flip = action_rel_flip.saturating_add(lead);
                let release_due = |flip: u64| match hold_ms {
                    Some(ms) => Due::AfterFlipOrMs {
                        arm_flip: action_down_flip,
                        flip,
                        hold_ms: *ms,
                    },
                    None => Due::Point { flip, poll: 0 },
                };
                self.push(
                    release_due(action_rel_flip),
                    Payload::Input(Input::Key {
                        code: final_code,
                        down: false,
                    }),
                );
                let prefix_due = release_due(prefix_rel_flip);
                for &code in prefix_codes.iter().rev() {
                    self.push(prefix_due, Payload::Input(Input::Key { code, down: false }));
                }
                prefix_rel_flip
            }
            Action::TypeText { text, flip_gap } => {
                let mut mapped = Vec::with_capacity(text.len());
                for ch in text.chars() {
                    let key = ascii_to_key(ch).ok_or(ActionError::UnsupportedChar(ch))?;
                    mapped.push(key);
                }
                if mapped.is_empty() {
                    return Ok(ScheduleReport {
                        events_scheduled: 0,
                        end_flip: start_flip,
                        required_flips: 0,
                    });
                }
                let gap = (*flip_gap).max(1);
                start_flip = self.next_unpolled_flip(start_flip);
                let mut cursor = start_flip;
                let mut last_flip = start_flip;
                for (shift, code) in mapped {
                    if shift {
                        self.push_input(
                            cursor,
                            0,
                            Input::Key {
                                code: DIK_LSHIFT,
                                down: true,
                            },
                        );
                    }
                    self.push_input(cursor, 0, Input::Key { code, down: true });
                    let rel = cursor.saturating_add(1);
                    self.push_input(rel, 0, Input::Key { code, down: false });
                    if shift {
                        self.push_input(
                            rel,
                            0,
                            Input::Key {
                                code: DIK_LSHIFT,
                                down: false,
                            },
                        );
                    }
                    last_flip = rel;
                    cursor = rel.saturating_add(gap);
                }
                last_flip
            }
        };
        let events_scheduled = self.pending.len() - before;
        let required_flips = if events_scheduled == 0 {
            0
        } else {
            end_flip.saturating_sub(start_flip).saturating_add(1)
        };
        Ok(ScheduleReport {
            events_scheduled,
            end_flip,
            required_flips,
        })
    }

    /// Parse `#lego-legacy-route 1` text and schedule its events onto this
    /// timeline (relative to `current_flip` when `relative == true`, or at
    /// their literal flip numbers otherwise).
    ///
    /// # Errors
    /// Returns [`ActionError::RouteParse`] if `route_text` is not a valid route.
    pub fn pipe_route(
        &mut self,
        route_text: &str,
        current_flip: u64,
        relative: bool,
    ) -> Result<ScheduleReport, ActionError> {
        let route = Route::parse(route_text).map_err(ActionError::RouteParse)?;
        let entries = route.due(0, (u64::MAX, u64::MAX));
        if entries.is_empty() {
            return Ok(ScheduleReport {
                events_scheduled: 0,
                end_flip: current_flip,
                required_flips: 0,
            });
        }
        let mut end_flip = current_flip;
        for &((flip, poll), input) in entries {
            let target_flip = if relative {
                current_flip.saturating_add(flip)
            } else {
                flip
            };
            end_flip = end_flip.max(target_flip);
            self.push_input(target_flip, poll, input);
        }
        let required_flips = end_flip.saturating_sub(current_flip).saturating_add(1);
        Ok(ScheduleReport {
            events_scheduled: entries.len(),
            end_flip,
            required_flips,
        })
    }

    /// Drain every scheduled event due at `(flip, poll, declared_ms)` in
    /// deterministic order, translating relative [`Payload::MouseDeltaStep`]s
    /// against the live hardware pointer and paying back any owed delta chunks
    /// across recenter flips (#327).
    pub fn drain_due(&mut self, flip: u64, poll: u64, declared_ms: u64) -> Vec<Input> {
        self.note_polled(flip);
        if self.pending.is_empty() && (poll > 0 || !self.has_owed_delta()) {
            return Vec::new();
        }
        let mut due_items = Vec::new();
        let mut remaining = Vec::with_capacity(self.pending.len());
        for mut item in self.pending.drain(..) {
            if let Due::AfterFlipOrMs {
                arm_flip,
                flip: rel_flip,
                hold_ms,
            } = item.due
            {
                if flip >= arm_flip {
                    item.due = Due::FlipOrMs {
                        min_flip: flip,
                        min_poll: poll,
                        flip: rel_flip,
                        declared_ms: declared_ms.saturating_add(hold_ms),
                    };
                }
            }
            if item.due.is_due(flip, poll, declared_ms) {
                due_items.push(item);
            } else {
                remaining.push(item);
            }
        }
        self.pending = remaining;
        due_items.sort_by_key(|item| (item.due.sort_flip(), item.seq));

        let mut out = Vec::with_capacity(due_items.len() + 1);
        let mut emitted_delta = false;
        for item in due_items {
            match item.payload {
                Payload::Input(input) => {
                    if let Input::Move { x, y } = input {
                        self.observe_emitted_move(x, y);
                        self.owed_delta = (0.0, 0.0);
                    } else if !emitted_delta && poll == 0 && self.has_owed_delta() {
                        // Finish carry-over owed delta before emitting clicks/keys on this flip.
                        self.flush_owed_delta(false, &mut out);
                        emitted_delta = true;
                    }
                    out.push(input);
                }
                Payload::MouseDeltaStep { dx, dy } => {
                    self.owed_delta.0 += dx;
                    self.owed_delta.1 += dy;
                    self.flush_owed_delta(true, &mut out);
                    emitted_delta = true;
                }
            }
        }
        if !emitted_delta && poll == 0 && self.has_owed_delta() {
            self.flush_owed_delta(false, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_delta_translates_steered_put_pointer_and_scaling() {
        let mut tl = Timeline::new((2.0, 2.0));
        tl.set_frame_size(640, 480);
        // Guest hides cursor (`Mouse::Steered`, like Alpha Team DDMouse) and
        // recenters to (320, 240) each flip via `put_pointer`.
        tl.set_pointer_shown(false);
        tl.set_pointer(320, 240);

        let report = tl
            .schedule(
                &Action::MouseDelta {
                    dx: 30.0,
                    dy: -15.0,
                    flips: 3,
                    scale_x: None,
                    scale_y: None,
                },
                10,
                160,
            )
            .expect("schedules");
        assert_eq!(report.events_scheduled, 3);
        assert_eq!(report.end_flip, 12);
        assert_eq!(report.required_flips, 3);

        // Total scaled delta is (+60, -30) -> (+20, -10) per flip relative to
        // whatever `put_pointer` recentered to before each flip.
        for flip in 10..=12 {
            tl.set_pointer(320, 240);
            let ev = tl.drain_due(flip, 0, flip * 16);
            assert_eq!(ev, [Input::Move { x: 340, y: 230 }], "flip {flip}");
        }

        // Explicit scale override works even when host pointer is shown.
        tl.set_pointer_shown(true);
        tl.set_pointer(100, 100);
        tl.schedule(
            &Action::MouseDelta {
                dx: 5.0,
                dy: 10.0,
                flips: 1,
                scale_x: Some(3.0),
                scale_y: Some(0.5),
            },
            20,
            320,
        )
        .unwrap();
        assert_eq!(
            tl.drain_due(20, 0, 320),
            [Input::Move { x: 115, y: 105 }],
            "explicit override scales delta"
        );
    }

    #[test]
    fn click_inserts_default_settle_gap_and_respects_opt_out_and_in_place() {
        use crate::control::json::{parse_action_obj, parse_json};

        let parse_click = |raw: &str| {
            let v = parse_json(raw).expect("valid json");
            parse_action_obj("click", v.as_object().expect("object")).expect("valid click")
        };

        // 1. Default click with coordinates settles 1 flip between Move and Down.
        let mut tl = Timeline::new((1.0, 1.0));
        let act = parse_click(r#"{"x":320,"y":222,"hold_flips":1}"#);
        let report = tl.schedule(&act, 10, 1000).unwrap();
        assert_eq!(report.events_scheduled, 3);
        assert_eq!(report.end_flip, 12);
        assert_eq!(report.required_flips, 3);
        assert_eq!(
            tl.drain_due(10, 0, 1000),
            [Input::Move { x: 320, y: 222 }],
            "move fires alone on start flip"
        );
        assert_eq!(
            tl.drain_due(11, 0, 1016),
            [Input::Down(Button::Left)],
            "down fires after 1-flip settle gap"
        );
        assert_eq!(tl.drain_due(12, 0, 1032), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // 2. Explicit settle_flips: 0 opts out and keeps same-flip Move + Down.
        let act = parse_click(r#"{"x":320,"y":222,"hold_flips":1,"settle_flips":0}"#);
        let report = tl.schedule(&act, 20, 2000).unwrap();
        assert_eq!(report.events_scheduled, 3);
        assert_eq!(report.end_flip, 21);
        assert_eq!(report.required_flips, 2);
        assert_eq!(
            tl.drain_due(20, 0, 2000),
            [Input::Move { x: 320, y: 222 }, Input::Down(Button::Left)]
        );
        assert_eq!(tl.drain_due(21, 0, 2016), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // 3. Explicit settle_flips: 2 leaves an idle flip between Move and Down.
        let act = parse_click(r#"{"x":320,"y":222,"hold_flips":2,"settle_flips":2}"#);
        let report = tl.schedule(&act, 30, 3000).unwrap();
        assert_eq!(report.end_flip, 34);
        assert_eq!(report.required_flips, 5);
        assert_eq!(tl.drain_due(30, 0, 3000), [Input::Move { x: 320, y: 222 }]);
        assert_eq!(tl.drain_due(31, 0, 3016), []);
        assert_eq!(tl.drain_due(32, 0, 3032), [Input::Down(Button::Left)]);
        assert_eq!(tl.drain_due(33, 0, 3048), []);
        assert_eq!(tl.drain_due(34, 0, 3064), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // 4. In-place click (no coordinates) omits settle gap because no Move is emitted.
        let act = parse_click(r#"{"hold_flips":1}"#);
        let report = tl.schedule(&act, 40, 4000).unwrap();
        assert_eq!(report.events_scheduled, 2);
        assert_eq!(report.end_flip, 41);
        assert_eq!(report.required_flips, 2);
        assert_eq!(tl.drain_due(40, 0, 4000), [Input::Down(Button::Left)]);
        assert_eq!(tl.drain_due(41, 0, 4016), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // 5. Partial coordinates (only x, no y) emits no Move and omits settle gap.
        let act = parse_click(r#"{"x":100,"hold_flips":1}"#);
        let report = tl.schedule(&act, 50, 5000).unwrap();
        assert_eq!(report.events_scheduled, 2);
        assert_eq!(report.end_flip, 51);
        assert_eq!(tl.drain_due(50, 0, 5000), [Input::Down(Button::Left)]);
        assert_eq!(tl.drain_due(51, 0, 5016), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());
    }

    #[test]
    fn click_with_settle_and_hold_ms_never_releases_before_down() {
        use crate::control::json::{parse_action_obj, parse_json};

        let v = parse_json(r#"{"x":100,"y":200,"hold_flips":3,"hold_ms":16,"settle_flips":2}"#)
            .unwrap();
        let act = parse_action_obj("click", v.as_object().unwrap()).unwrap();

        let mut tl = Timeline::new((1.0, 1.0));
        let report = tl.schedule(&act, 10, 1000).unwrap();
        assert_eq!(report.events_scheduled, 3);
        assert_eq!(report.end_flip, 15);
        assert_eq!(report.required_flips, 6);

        assert_eq!(tl.drain_due(10, 0, 1000), [Input::Move { x: 100, y: 200 }]);
        // Flip 11 is at 1020 ms (> 1000 + 16), but Down hasn't fired yet -> must NOT emit Up.
        assert_eq!(tl.drain_due(11, 0, 1020), []);
        // Flip 12 fires Down and arms the 16 ms release window from 1040 ms -> 1056 ms.
        assert_eq!(tl.drain_due(12, 0, 1040), [Input::Down(Button::Left)]);
        assert_eq!(tl.drain_due(12, 1, 1050), []);
        assert_eq!(tl.drain_due(12, 2, 1056), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // Flip-triggered release when flip reaches rel_flip before hold_ms elapses.
        let v =
            parse_json(r#"{"x":10,"y":20,"hold_flips":1,"hold_ms":500,"settle_flips":1}"#).unwrap();
        let act = parse_action_obj("click", v.as_object().unwrap()).unwrap();
        tl.schedule(&act, 20, 2000).unwrap();
        assert_eq!(tl.drain_due(20, 0, 2000), [Input::Move { x: 10, y: 20 }]);
        assert_eq!(tl.drain_due(21, 0, 2016), [Input::Down(Button::Left)]);
        assert_eq!(tl.drain_due(22, 0, 2032), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // Zero hold_ms with settle_flips > 0 still delivers Down alone on poll 0
        // (min_flip guard) and releases strictly after on poll 1.
        let v =
            parse_json(r#"{"x":10,"y":20,"hold_flips":1,"hold_ms":0,"settle_flips":1}"#).unwrap();
        let act = parse_action_obj("click", v.as_object().unwrap()).unwrap();
        tl.schedule(&act, 30, 3000).unwrap();
        assert_eq!(tl.drain_due(30, 0, 3000), [Input::Move { x: 10, y: 20 }]);
        assert_eq!(tl.drain_due(31, 0, 3016), [Input::Down(Button::Left)]);
        assert_eq!(tl.drain_due(31, 1, 3016), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // Invalid settle_flips values and settle_flips on click_poll are rejected.
        for bad in [
            r#"{"x":1,"y":2,"settle_flips":-1}"#,
            r#"{"x":1,"y":2,"settle_flips":"0"}"#,
        ] {
            let v = parse_json(bad).unwrap();
            assert!(parse_action_obj("click", v.as_object().unwrap()).is_err());
        }
        let v = parse_json(r#"{"x":1,"y":2,"settle_flips":1}"#).unwrap();
        assert!(parse_action_obj("click_poll", v.as_object().unwrap()).is_err());
    }

    #[test]
    fn click_poll_releases_on_declared_ms_without_flipping_and_on_flip() {
        // Case 1: static menu loop — flips stay stuck at 50, declared_ms ticks up.
        let mut tl = Timeline::new((1.0, 1.0));
        tl.schedule(
            &Action::ClickPoll {
                x: Some(200),
                y: Some(150),
                button: Button::Left,
                hold_flips: 4,
                hold_ms: 150,
            },
            50,
            1000,
        )
        .unwrap();

        assert_eq!(
            tl.drain_due(50, 0, 1000),
            [Input::Move { x: 200, y: 150 }, Input::Down(Button::Left),]
        );
        // Poll 1 at 1100 ms: still below 1150 ms, stays held.
        assert_eq!(tl.drain_due(50, 1, 1100), []);
        // Poll 2 at 1200 ms (still flip 50!): declared_ms threshold reached -> releases!
        assert_eq!(tl.drain_due(50, 2, 1200), [Input::Up(Button::Left)]);
        assert!(tl.is_empty());

        // Case 2: normal flipping title releases by flip count even if ms hasn't elapsed.
        tl.schedule(
            &Action::ClickPoll {
                x: None,
                y: None,
                button: Button::Right,
                hold_flips: 2,
                hold_ms: 500,
            },
            60,
            2000,
        )
        .unwrap();
        assert_eq!(tl.drain_due(60, 0, 2000), [Input::Down(Button::Right)]);
        assert_eq!(tl.drain_due(61, 0, 2016), []);
        assert_eq!(tl.drain_due(62, 0, 2032), [Input::Up(Button::Right)]);
    }

    #[test]
    fn chord_presses_simultaneously_and_releases_in_reverse_order() {
        let mut tl = Timeline::new((1.0, 1.0));
        let report = tl
            .schedule(
                &Action::Chord {
                    keys: vec!["LeftControl".into(), "A".into(), "0x39".into()],
                    hold_flips: 2,
                    hold_ms: None,
                    lead_flips: 0,
                },
                5,
                0,
            )
            .unwrap();
        assert_eq!(report.events_scheduled, 6);
        assert_eq!(report.end_flip, 7);

        assert_eq!(
            tl.drain_due(5, 0, 0),
            [
                Input::Key {
                    code: 0x1d,
                    down: true
                },
                Input::Key {
                    code: 0x1e,
                    down: true
                },
                Input::Key {
                    code: 0x39,
                    down: true
                },
            ]
        );
        assert_eq!(tl.drain_due(6, 0, 16), []);
        assert_eq!(
            tl.drain_due(7, 0, 32),
            [
                Input::Key {
                    code: 0x39,
                    down: false
                },
                Input::Key {
                    code: 0x1e,
                    down: false
                },
                Input::Key {
                    code: 0x1d,
                    down: false
                },
            ]
        );
        assert!(matches!(
            tl.schedule(
                &Action::Chord {
                    keys: vec![],
                    hold_flips: 1,
                    hold_ms: None,
                    lead_flips: 1,
                },
                0,
                0
            ),
            Err(ActionError::EmptyChord)
        ));
    }

    #[test]
    fn drag_steps_and_releases_after_target() {
        let mut tl = Timeline::new((1.0, 1.0));
        let report = tl
            .schedule(
                &Action::Drag {
                    from_x: 10,
                    from_y: 20,
                    to_x: 50,
                    to_y: 20,
                    button: Button::Left,
                    step_px: 20,
                },
                1,
                0,
            )
            .unwrap();
        // dist = 40, step_px = 20 -> 2 move steps (flips 2 and 3), release at flip 4.
        assert_eq!(report.end_flip, 4);
        assert_eq!(
            tl.drain_due(1, 0, 0),
            [Input::Move { x: 10, y: 20 }, Input::Down(Button::Left)]
        );
        assert_eq!(tl.drain_due(2, 0, 16), [Input::Move { x: 30, y: 20 }]);
        assert_eq!(tl.drain_due(3, 0, 32), [Input::Move { x: 50, y: 20 }]);
        assert_eq!(tl.drain_due(4, 0, 48), [Input::Up(Button::Left)]);
    }

    #[test]
    fn type_text_handles_shift_and_flip_gap() {
        let mut tl = Timeline::new((1.0, 1.0));
        let report = tl
            .schedule(
                &Action::TypeText {
                    text: "Ab".to_owned(),
                    flip_gap: 2,
                },
                10,
                0,
            )
            .unwrap();
        // 'A': down @ 10 (Shift+A), up @ 11 (A+Shift), gap 2 -> 'b': down @ 13, up @ 14.
        assert_eq!(report.end_flip, 14);
        assert_eq!(
            tl.drain_due(10, 0, 0),
            [
                Input::Key {
                    code: DIK_LSHIFT,
                    down: true
                },
                Input::Key {
                    code: 0x1e,
                    down: true
                },
            ]
        );
        assert_eq!(
            tl.drain_due(11, 0, 16),
            [
                Input::Key {
                    code: 0x1e,
                    down: false
                },
                Input::Key {
                    code: DIK_LSHIFT,
                    down: false
                },
            ]
        );
        assert_eq!(tl.drain_due(12, 0, 32), []);
        assert_eq!(
            tl.drain_due(13, 0, 48),
            [Input::Key {
                code: 0x30,
                down: true
            }]
        );
        assert_eq!(
            tl.drain_due(14, 0, 64),
            [Input::Key {
                code: 0x30,
                down: false
            }]
        );
    }

    #[test]
    fn type_text_after_polled_flip_preserves_nonzero_hold_and_gap() {
        let drain_keys = |tl: &mut Timeline, flips: std::ops::RangeInclusive<u64>| {
            let (mut downs, mut ups) = (Vec::new(), Vec::new());
            for flip in flips {
                for ev in tl.drain_due(flip, 0, flip * 16) {
                    if let Input::Key { code, down } = ev {
                        if code != DIK_LSHIFT {
                            if down {
                                downs.push((code, flip));
                            } else {
                                ups.push((code, flip));
                            }
                        }
                    }
                }
            }
            (downs, ups)
        };

        // Mid-frame `Live`-mode repro (#345): flip 554 already polled, then
        // `type_text "AGY" --flip-gap 8` arrives while `current_flip` is 554.
        let mut tl = Timeline::new((1.0, 1.0));
        assert_eq!(tl.drain_due(554, 0, 554 * 16), []);
        let report = tl
            .schedule(
                &Action::TypeText {
                    text: "AGY".to_owned(),
                    flip_gap: 8,
                },
                554,
                554 * 16,
            )
            .unwrap();
        assert_eq!(report.end_flip, 574);
        assert_eq!(report.required_flips, 20);
        let (downs, ups) = drain_keys(&mut tl, 555..=576);
        assert_eq!(downs, [(0x1e, 555), (0x22, 564), (0x15, 573)]);
        assert_eq!(ups, [(0x1e, 556), (0x22, 565), (0x15, 574)]);

        // Single-char (unshifted & shifted), spaces, and mixed punctuation after
        // an already-polled flip all enforce `up_flip > down_flip` and `flip_gap`.
        for (text, gap) in [("x", 1u64), ("Z", 3), ("a B!", 4)] {
            let mut tl = Timeline::new((1.0, 1.0));
            assert_eq!(tl.drain_due(100, 0, 1600), []);
            let rep = tl
                .schedule(
                    &Action::TypeText {
                        text: text.to_owned(),
                        flip_gap: gap,
                    },
                    100,
                    1600,
                )
                .unwrap();
            let (downs, ups) = drain_keys(&mut tl, 101..=rep.end_flip + 1);
            assert_eq!(downs.len(), text.len(), "text={text:?}");
            assert_eq!(ups.len(), text.len(), "text={text:?}");
            let mut prev_up = None;
            for i in 0..text.len() {
                assert_eq!(downs[i].0, ups[i].0, "text={text:?} idx={i}");
                assert!(
                    ups[i].1 > downs[i].1,
                    "text={text:?} idx={i}: up_flip={} must exceed down_flip={}",
                    ups[i].1,
                    downs[i].1
                );
                if let Some(pu) = prev_up {
                    assert_eq!(downs[i].1 - pu, gap.max(1), "text={text:?} idx={i} gap");
                }
                prev_up = Some(ups[i].1);
            }
        }

        // Clearing pending queue mid-frame preserves `last_polled_flip`;
        // `reset_polled` clears it for rewind/restore to an earlier flip.
        let mut tl = Timeline::new((1.0, 1.0));
        assert_eq!(tl.drain_due(200, 0, 3200), []);
        tl.clear();
        tl.schedule(
            &Action::TypeText {
                text: "k".to_owned(),
                flip_gap: 2,
            },
            200,
            3200,
        )
        .unwrap();
        assert_eq!(
            drain_keys(&mut tl, 201..=202),
            (vec![(0x25, 201)], vec![(0x25, 202)])
        );

        tl.reset_polled();
        tl.schedule(
            &Action::TypeText {
                text: "k".to_owned(),
                flip_gap: 2,
            },
            50,
            800,
        )
        .unwrap();
        assert_eq!(
            drain_keys(&mut tl, 50..=51),
            (vec![(0x25, 50)], vec![(0x25, 51)])
        );
    }

    #[test]
    fn pipe_route_supports_relative_and_absolute_flips_and_polls() {
        let mut tl = Timeline::new((1.0, 1.0));
        let route_text = "#lego-legacy-route 1\n\
                          flip 0 move 10 20\n\
                          flip 0 poll 2 key Space down\n\
                          flip 2 key Space up\n";
        let report = tl.pipe_route(route_text, 100, true).unwrap();
        assert_eq!(report.events_scheduled, 3);
        assert_eq!(report.end_flip, 102);
        assert_eq!(report.required_flips, 3);

        assert_eq!(tl.drain_due(100, 0, 0), [Input::Move { x: 10, y: 20 }]);
        assert_eq!(tl.drain_due(100, 1, 100), []);
        assert_eq!(
            tl.drain_due(100, 2, 200),
            [Input::Key {
                code: 0x39,
                down: true
            }]
        );
        assert_eq!(
            tl.drain_due(102, 0, 232),
            [Input::Key {
                code: 0x39,
                down: false
            }]
        );

        // Absolute mode schedules at literal flip numbers.
        let abs = tl.pipe_route("flip 150 down right\n", 105, false).unwrap();
        assert_eq!(abs.end_flip, 150);
        assert_eq!(tl.drain_due(149, 0, 0), []);
        assert_eq!(tl.drain_due(150, 0, 16), [Input::Down(Button::Right)]);
    }

    #[test]
    fn timeline_initializes_at_screen_center_and_chunks_owed_delta_across_recenter_flips() {
        let mut tl = Timeline::new((2.0, 2.0));
        assert_eq!(
            tl.last_pointer(),
            (320, 240),
            "initial hw pointer matches win32::State::new center"
        );
        tl.configure_mode(
            MouseMode::RelativeSteered {
                virtual_span: (1280, 960),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            (640, 480),
        );

        // +300 screen px * 2.0 scale = +600 raw px: exceeds 639 - 320 = 319 raw px
        // in one flip, so the remaining 281 raw px must carry over to the next
        // flip after recentering rather than being dropped.
        let report = tl
            .schedule(
                &Action::MouseDelta {
                    dx: 300.0,
                    dy: 0.0,
                    flips: 1,
                    scale_x: None,
                    scale_y: None,
                },
                10,
                160,
            )
            .expect("schedules");
        assert!(report.required_flips >= 2, "{report:?}");

        let ev0 = tl.drain_due(10, 0, 160);
        assert_eq!(ev0, [Input::Move { x: 639, y: 240 }]);

        // Guest recenters to (320, 240) before flip 11; owed +281 raw px pays back.
        tl.set_pointer(320, 240);
        let ev1 = tl.drain_due(11, 0, 176);
        assert_eq!(ev1, [Input::Move { x: 601, y: 240 }]);
        assert_eq!(tl.drain_due(12, 0, 192), []);
        assert!(tl.is_empty());
    }

    #[test]
    fn half_integer_owed_delta_does_not_oscillate_and_software_absolute_does_not_wedge() {
        // 1. Half-integer raw delta (±0.5) must never oscillate ±1 px across flips.
        let mut tl = Timeline::new((1.0, 1.0));
        tl.configure_mode(
            MouseMode::RelativeSteered {
                virtual_span: (640, 480),
                recenter: (320, 240),
                start_corner: (320, 240),
            },
            (640, 480),
        );
        tl.schedule(
            &Action::MouseDelta {
                dx: 0.5,
                dy: -1.5,
                flips: 1,
                scale_x: Some(1.0),
                scale_y: Some(1.0),
            },
            0,
            0,
        )
        .unwrap();
        let _ = tl.drain_due(0, 0, 0);
        tl.set_pointer(320, 240);
        assert_eq!(tl.drain_due(1, 0, 16), [], "no ±1 oscillation on ±0.5 rem");
        assert!(tl.is_empty());

        // 2. Non-recentering SoftwareAbsolute discards wall overflow on MouseDelta
        //    so is_empty() stays true and opposite moves reverse immediately.
        let mut sa = Timeline::new((1.0, 1.0));
        sa.configure_mode(
            MouseMode::SoftwareAbsolute {
                requires_poll_release: true,
            },
            (640, 480),
        );
        sa.schedule(
            &Action::MouseDelta {
                dx: 500.0,
                dy: 0.0,
                flips: 1,
                scale_x: None,
                scale_y: None,
            },
            0,
            0,
        )
        .unwrap();
        assert_eq!(sa.drain_due(0, 0, 0), [Input::Move { x: 639, y: 240 }]);
        assert!(
            sa.is_empty(),
            "wall overflow must not wedge SoftwareAbsolute"
        );
        sa.schedule(
            &Action::MouseDelta {
                dx: -20.0,
                dy: 0.0,
                flips: 1,
                scale_x: None,
                scale_y: None,
            },
            1,
            16,
        )
        .unwrap();
        assert_eq!(sa.drain_due(1, 0, 16), [Input::Move { x: 619, y: 240 }]);

        // 3. Multi-flip relative Click with hold_ms and ClickPoll never emit Up before Down.
        let mut rel = Timeline::new((2.0, 1280.0 / 480.0));
        rel.configure_mode(
            MouseMode::RelativeSteered {
                virtual_span: (1280, 1280),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            (640, 480),
        );
        let r = rel
            .schedule(
                &Action::ClickPoll {
                    x: Some(480),
                    y: Some(120),
                    button: Button::Left,
                    hold_flips: 2,
                    hold_ms: 50,
                },
                0,
                0,
            )
            .unwrap();
        assert!(r.end_flip >= 4);
        let mut saw_down = false;
        let mut saw_up = false;
        for f in 0..=r.end_flip {
            rel.set_pointer(320, 240);
            for ev in rel.drain_due(f, 0, f * 33) {
                match ev {
                    Input::Down(Button::Left) => {
                        assert!(!saw_up, "Up must never fire before Down");
                        saw_down = true;
                    }
                    Input::Up(Button::Left) => {
                        assert!(saw_down, "Up must follow Down");
                        saw_up = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_down && saw_up);
        assert_eq!(rel.visual_cursor_pos(), (480, 120));

        // 4. Mid-run display mode switch resets hw_pointer to new screen center.
        let mut res = Timeline::new((1.0, 1.0));
        res.configure_mode(
            MouseMode::SoftwareRelative {
                virtual_span: (640, 480),
                start_pos: (320, 240),
            },
            (640, 480),
        );
        res.observe_emitted_move(500, 400);
        res.set_frame_size(320, 200);
        assert_eq!(res.last_pointer(), (160, 100));
    }

    #[test]
    fn click_settle_applies_after_relative_move_chunking_and_software_relative() {
        // 1. RelativeSteered: multi-flip delta chunking lands at move_flip, then
        //    settle_flips waits before Down, and hold_ms arms at down_flip.
        let mut rel = Timeline::new((2.0, 1280.0 / 480.0));
        rel.configure_mode(
            MouseMode::RelativeSteered {
                virtual_span: (1280, 1280),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            (640, 480),
        );
        let r = rel
            .schedule(
                &Action::Click {
                    x: Some(480),
                    y: Some(120),
                    button: Button::Left,
                    hold_flips: 2,
                    hold_ms: Some(25),
                    settle_flips: 1,
                },
                0,
                0,
            )
            .unwrap();
        let mut move_flips = Vec::new();
        let mut down_flip = None;
        let mut up_flip = None;
        for f in 0..=r.end_flip {
            rel.set_pointer(320, 240);
            for ev in rel.drain_due(f, 0, f * 33) {
                match ev {
                    Input::Move { .. } => move_flips.push(f),
                    Input::Down(Button::Left) => {
                        assert!(up_flip.is_none(), "Up must never fire before Down");
                        down_flip = Some(f);
                    }
                    Input::Up(Button::Left) => {
                        assert!(down_flip.is_some(), "Up must follow Down");
                        up_flip = Some(f);
                    }
                    _ => {}
                }
            }
        }
        let last_move = *move_flips.last().expect("at least one move chunk");
        assert!(move_flips.len() >= 2, "expected multi-flip relative chunks");
        assert_eq!(down_flip, Some(last_move + 1));
        assert_eq!(up_flip, Some(last_move + 2));
        assert_eq!(rel.visual_cursor_pos(), (480, 120));
        assert!(rel.is_empty());

        // 2. RelativeSteered with settle_flips: 0 fires Down on last move chunk flip.
        let r0 = rel
            .schedule(
                &Action::Click {
                    x: Some(120),
                    y: Some(360),
                    button: Button::Left,
                    hold_flips: 1,
                    hold_ms: None,
                    settle_flips: 0,
                },
                20,
                1000,
            )
            .unwrap();
        let mut last_move0 = None;
        let mut down0 = None;
        for f in 20..=r0.end_flip {
            rel.set_pointer(320, 240);
            for ev in rel.drain_due(f, 0, 1000 + (f - 20) * 16) {
                match ev {
                    Input::Move { .. } => last_move0 = Some(f),
                    Input::Down(Button::Left) => down0 = Some(f),
                    _ => {}
                }
            }
        }
        assert_eq!(down0, last_move0);
        assert_eq!(rel.visual_cursor_pos(), (120, 360));
        assert!(rel.is_empty());

        // 3. SoftwareRelative: single-flip unclamped move + 1-flip settle + Down + Up,
        //    and in-place click omits settle gap.
        let mut sr = Timeline::new((1.0, 1.0));
        sr.configure_mode(
            MouseMode::SoftwareRelative {
                virtual_span: (640, 480),
                start_pos: (320, 240),
            },
            (640, 480),
        );
        let r = sr
            .schedule(
                &Action::Click {
                    x: Some(100),
                    y: Some(50),
                    button: Button::Left,
                    hold_flips: 1,
                    hold_ms: None,
                    settle_flips: 1,
                },
                10,
                0,
            )
            .unwrap();
        assert_eq!(r.end_flip, 12);
        assert_eq!(sr.drain_due(10, 0, 0), [Input::Move { x: 100, y: 50 }]);
        assert_eq!(sr.drain_due(11, 0, 16), [Input::Down(Button::Left)]);
        assert_eq!(sr.drain_due(12, 0, 32), [Input::Up(Button::Left)]);
        assert_eq!(sr.visual_cursor_pos(), (100, 50));

        let r_in_place = sr
            .schedule(
                &Action::Click {
                    x: None,
                    y: None,
                    button: Button::Right,
                    hold_flips: 1,
                    hold_ms: None,
                    settle_flips: 1,
                },
                20,
                0,
            )
            .unwrap();
        assert_eq!(r_in_place.end_flip, 21);
        assert_eq!(sr.drain_due(20, 0, 0), [Input::Down(Button::Right)]);
        assert_eq!(sr.drain_due(21, 0, 16), [Input::Up(Button::Right)]);
        assert!(sr.is_empty());
    }

    #[test]
    fn action_summary_formats_cleanly_without_rust_debug_leaks() {
        let cases: &[(Action, &str)] = &[
            (
                Action::Move {
                    x: 120,
                    y: 240,
                    glide_flips: 3,
                },
                "move (120, 240) glide=3",
            ),
            (
                Action::MouseDelta {
                    dx: 12.5,
                    dy: -4.0,
                    flips: 2,
                    scale_x: None,
                    scale_y: None,
                },
                "mouse_delta (12.5, -4.0) flips=2",
            ),
            (
                Action::Click {
                    x: Some(508),
                    y: Some(377),
                    button: Button::Left,
                    hold_flips: 2,
                    hold_ms: None,
                    settle_flips: 1,
                },
                "click Left (508, 377) hold=2",
            ),
            (
                Action::Click {
                    x: None,
                    y: None,
                    button: Button::Right,
                    hold_flips: 3,
                    hold_ms: None,
                    settle_flips: 1,
                },
                "click Right in-place hold=3",
            ),
            (
                Action::Click {
                    x: Some(320),
                    y: Some(240),
                    button: Button::Left,
                    hold_flips: 2,
                    hold_ms: Some(150),
                    settle_flips: 1,
                },
                "click Left (320, 240) hold=2f/150ms",
            ),
            (
                Action::ClickPoll {
                    x: Some(508),
                    y: Some(377),
                    button: Button::Left,
                    hold_flips: 2,
                    hold_ms: 150,
                },
                "click_poll Left (508, 377) hold=2f/150ms",
            ),
            (
                Action::ClickPoll {
                    x: None,
                    y: None,
                    button: Button::Right,
                    hold_flips: 2,
                    hold_ms: 120,
                },
                "click_poll Right in-place hold=2f/120ms",
            ),
            (
                Action::Drag {
                    from_x: 10,
                    from_y: 20,
                    to_x: 100,
                    to_y: 200,
                    button: Button::Left,
                    step_px: 16,
                },
                "drag Left (10, 20) -> (100, 200)",
            ),
            (
                Action::KeyHold {
                    key: "Space".to_owned(),
                    hold_flips: 4,
                    hold_ms: None,
                },
                "key_hold Space hold=4",
            ),
            (
                Action::KeyHold {
                    key: "Return".to_owned(),
                    hold_flips: 3,
                    hold_ms: Some(150),
                },
                "key_hold Return hold=3f/150ms",
            ),
            (
                Action::Chord {
                    keys: vec!["LeftShift".to_owned(), "A".to_owned()],
                    hold_flips: 2,
                    hold_ms: None,
                    lead_flips: 1,
                },
                "chord [LeftShift+A] hold=2",
            ),
            (
                Action::TypeText {
                    text: "lego".to_owned(),
                    flip_gap: 4,
                },
                "type_text len=4 gap=4",
            ),
        ];

        for (action, expected) in cases {
            let summary = action.summary();
            assert_eq!(summary, *expected);
            assert!(
                !summary.contains("Some(") && !summary.contains("None"),
                "Rust Debug artifact leaked in `{summary}`"
            );
        }
    }

    #[test]
    fn chord_staggers_prefix_lead_and_trail_by_default_and_supports_opt_out_and_hold_ms() {
        use crate::control::json::{parse_action_obj, parse_json};

        let parse_chord = |raw: &str| {
            let v = parse_json(raw).expect("valid json");
            parse_action_obj("chord", v.as_object().expect("object")).expect("valid chord")
        };
        let key = |code: u32, down: bool| Input::Key { code, down };

        // 1. Default 2-key chord (F2, W) staggers prefix lead/trail by 1 flip.
        let mut tl = Timeline::new((1.0, 1.0));
        let act = parse_chord(r#"{"keys":["F2","W"],"hold_flips":1}"#);
        let report = tl.schedule(&act, 10, 1000).unwrap();
        assert_eq!(report.events_scheduled, 4);
        assert_eq!(report.end_flip, 13);
        assert_eq!(report.required_flips, 4);
        assert_eq!(tl.drain_due(10, 0, 1000), [key(0x3c, true)]);
        assert_eq!(tl.drain_due(11, 0, 1016), [key(0x11, true)]);
        assert_eq!(tl.drain_due(12, 0, 1032), [key(0x11, false)]);
        assert_eq!(tl.drain_due(13, 0, 1048), [key(0x3c, false)]);
        assert!(tl.is_empty());

        // 2. Default 3-key chord presses earlier prefix keys together, leads final key,
        //    releases final key first, then releases prefix keys in reverse order.
        let act = parse_chord(r#"{"keys":["LeftControl","LeftShift","S"],"hold_flips":2}"#);
        let report = tl.schedule(&act, 20, 2000).unwrap();
        assert_eq!(report.events_scheduled, 6);
        assert_eq!(report.end_flip, 24);
        assert_eq!(report.required_flips, 5);
        assert_eq!(
            tl.drain_due(20, 0, 2000),
            [key(0x1d, true), key(0x2a, true)]
        );
        assert_eq!(tl.drain_due(21, 0, 2016), [key(0x1f, true)]);
        assert_eq!(tl.drain_due(22, 0, 2032), []);
        assert_eq!(tl.drain_due(23, 0, 2048), [key(0x1f, false)]);
        assert_eq!(
            tl.drain_due(24, 0, 2064),
            [key(0x2a, false), key(0x1d, false)]
        );
        assert!(tl.is_empty());

        // 3. Explicit lead_flips: 0 opts out and keeps simultaneous press/release.
        let act = parse_chord(r#"{"keys":["F2","Q"],"hold_flips":2,"lead_flips":0}"#);
        let report = tl.schedule(&act, 30, 3000).unwrap();
        assert_eq!(report.events_scheduled, 4);
        assert_eq!(report.end_flip, 32);
        assert_eq!(report.required_flips, 3);
        assert_eq!(
            tl.drain_due(30, 0, 3000),
            [key(0x3c, true), key(0x10, true)]
        );
        assert_eq!(tl.drain_due(31, 0, 3016), []);
        assert_eq!(
            tl.drain_due(32, 0, 3032),
            [key(0x10, false), key(0x3c, false)]
        );
        assert!(tl.is_empty());

        // 4. Explicit lead_flips: 2 leaves idle flips around prefix lead and trail.
        let act = parse_chord(r#"{"keys":["F2","W"],"hold_flips":1,"lead_flips":2}"#);
        let report = tl.schedule(&act, 40, 4000).unwrap();
        assert_eq!(report.end_flip, 45);
        assert_eq!(report.required_flips, 6);
        assert_eq!(tl.drain_due(40, 0, 4000), [key(0x3c, true)]);
        assert_eq!(tl.drain_due(41, 0, 4016), []);
        assert_eq!(tl.drain_due(42, 0, 4032), [key(0x11, true)]);
        assert_eq!(tl.drain_due(43, 0, 4048), [key(0x11, false)]);
        assert_eq!(tl.drain_due(44, 0, 4064), []);
        assert_eq!(tl.drain_due(45, 0, 4080), [key(0x3c, false)]);
        assert!(tl.is_empty());

        // 5. Single-key chord and duplicate-key chord omit stagger.
        for raw in [
            r#"{"keys":["Space"],"hold_flips":1,"lead_flips":2}"#,
            r#"{"keys":["F2","F2"],"hold_flips":1,"lead_flips":2}"#,
        ] {
            let mut single = Timeline::new((1.0, 1.0));
            let act = parse_chord(raw);
            let report = single.schedule(&act, 50, 5000).unwrap();
            assert_eq!(report.events_scheduled, 2);
            assert_eq!(report.end_flip, 51);
            assert_eq!(report.required_flips, 2);
            assert_eq!(single.drain_due(50, 0, 5000).len(), 1);
            assert_eq!(single.drain_due(51, 0, 5016).len(), 1);
            assert!(single.is_empty());
        }

        // 6. hold_ms with lead_flips > 0 never releases before final key presses,
        //    and releases in order in a static poll loop when hold_ms elapses.
        let act = parse_chord(r#"{"keys":["F2","W"],"hold_flips":4,"hold_ms":16,"lead_flips":2}"#);
        let report = tl.schedule(&act, 60, 6000).unwrap();
        assert_eq!(report.end_flip, 68);
        assert_eq!(tl.drain_due(60, 0, 6000), [key(0x3c, true)]);
        // Flip 61 is at 6020 ms (> 6000 + 16), before W presses at 62 -> must NOT release.
        assert_eq!(tl.drain_due(61, 0, 6020), []);
        // Flip 62 fires W down and arms 16 ms window from 6040 ms -> 6056 ms.
        assert_eq!(tl.drain_due(62, 0, 6040), [key(0x11, true)]);
        assert_eq!(tl.drain_due(62, 1, 6050), []);
        assert_eq!(
            tl.drain_due(62, 2, 6056),
            [key(0x11, false), key(0x3c, false)]
        );
        assert!(tl.is_empty());

        // 7. Mid-frame Live scheduling (start_flip already polled) advances to polled + 1
        //    so lead stagger and non-zero hold never collapse onto one flip.
        let mut live = Timeline::new((1.0, 1.0));
        assert_eq!(live.drain_due(100, 0, 1600), []);
        let act = parse_chord(r#"{"keys":["F2","W"],"hold_flips":1}"#);
        let report = live.schedule(&act, 100, 1600).unwrap();
        assert_eq!(report.end_flip, 104);
        assert_eq!(report.required_flips, 4);
        assert_eq!(live.drain_due(101, 0, 1616), [key(0x3c, true)]);
        assert_eq!(live.drain_due(102, 0, 1632), [key(0x11, true)]);
        assert_eq!(live.drain_due(103, 0, 1648), [key(0x11, false)]);
        assert_eq!(live.drain_due(104, 0, 1664), [key(0x3c, false)]);
        assert!(live.is_empty());

        // 8. Static non-flipping poll loop with lead_flips: 0 and hold_ms (including 0)
        //    fires on poll > 0 without deadlocking or collapsing Down+Up onto one poll.
        for (ms, rel_ms) in [(150u64, 5150u64), (0u64, 5000u64)] {
            let mut static_tl = Timeline::new((1.0, 1.0));
            assert_eq!(static_tl.drain_due(80, 0, 5000), []);
            let act = parse_chord(&format!(
                r#"{{"keys":["F2","W"],"hold_flips":4,"hold_ms":{ms},"lead_flips":0}}"#
            ));
            let rep = static_tl.schedule(&act, 80, 5000).unwrap();
            assert_eq!(rep.end_flip, 84);
            assert_eq!(
                static_tl.drain_due(80, 1, 5000),
                [key(0x3c, true), key(0x11, true)],
                "ms={ms}: down fires on poll 1 without collapsing up"
            );
            if ms > 0 {
                assert_eq!(static_tl.drain_due(80, 2, 5100), []);
                assert_eq!(
                    static_tl.drain_due(80, 3, rel_ms),
                    [key(0x11, false), key(0x3c, false)]
                );
            } else {
                assert_eq!(
                    static_tl.drain_due(80, 2, rel_ms),
                    [key(0x11, false), key(0x3c, false)]
                );
            }
            assert!(static_tl.is_empty());
        }

        // 9. Flip-driven trail stagger preserved when hold_ms exceeds flip hold window.
        let act = parse_chord(r#"{"keys":["F2","W"],"hold_flips":1,"hold_ms":500,"lead_flips":1}"#);
        let mut tlong = Timeline::new((1.0, 1.0));
        assert_eq!(tlong.schedule(&act, 10, 1000).unwrap().end_flip, 13);
        assert_eq!(tlong.drain_due(10, 0, 1000), [key(0x3c, true)]);
        assert_eq!(tlong.drain_due(11, 0, 1016), [key(0x11, true)]);
        assert_eq!(tlong.drain_due(12, 0, 1032), [key(0x11, false)]);
        assert_eq!(tlong.drain_due(13, 0, 1048), [key(0x3c, false)]);
        assert!(tlong.is_empty());

        // 10. Invalid lead_flips values are rejected; null falls back to default 1.
        let act_null = parse_chord(r#"{"keys":["F2","W"],"lead_flips":null}"#);
        let mut tnull = Timeline::new((1.0, 1.0));
        assert_eq!(tnull.schedule(&act_null, 0, 0).unwrap().end_flip, 3);
        for bad in [
            r#"{"keys":["F2","W"],"lead_flips":-1}"#,
            r#"{"keys":["F2","W"],"lead_flips":"0"}"#,
        ] {
            let v = parse_json(bad).unwrap();
            assert!(parse_action_obj("chord", v.as_object().unwrap()).is_err());
        }
    }
}
