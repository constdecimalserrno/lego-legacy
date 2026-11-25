//! Unified action timeline and piped-route scheduler (#308).
//!
//! Compiles high-level control actions (`move`, `mouse_delta`, `click`,
//! `click_poll`, `drag`, `key_hold`, `chord`, `type_text`, `pipe_route`) into
//! deterministic [`crate::host::Input`] events delivered through
//! [`crate::host::Host::poll_input`], where `Machine::deliver` automatically
//! records them into `#lego-legacy-route 1`.

use crate::host::{Button, Input};
use crate::route::{ParseError, Route};
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
    /// Press and release a mouse button, optionally moving first and releasing
    /// after `hold_flips` or `hold_ms` declared milliseconds.
    Click {
        x: Option<i32>,
        y: Option<i32>,
        button: Button,
        hold_flips: u64,
        hold_ms: Option<u64>,
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
    /// Press and hold multiple keys simultaneously, releasing in reverse order.
    Chord {
        keys: Vec<String>,
        hold_flips: u64,
        hold_ms: Option<u64>,
    },
    /// Type printable ASCII text (plus `\n`, `\t`, `\x08`) using `LeftShift`
    /// where needed, spaced by `flip_gap` flips between characters.
    TypeText { text: String, flip_gap: u64 },
}

impl Action {
    /// Short human-readable summary for the bounded action feed.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Move { x, y, glide_flips } => format!("move ({x},{y}) glide={glide_flips}"),
            Self::MouseDelta { dx, dy, flips, .. } => {
                format!("mouse_delta ({dx:.1},{dy:.1}) flips={flips}")
            }
            Self::Click {
                x,
                y,
                button,
                hold_flips,
                ..
            } => format!("click {button:?} at ({x:?},{y:?}) hold={hold_flips}"),
            Self::ClickPoll {
                x,
                y,
                button,
                hold_flips,
                hold_ms,
            } => format!("click_poll {button:?} at ({x:?},{y:?}) hold={hold_flips}f/{hold_ms}ms"),
            Self::Drag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
                ..
            } => format!("drag {button:?} ({from_x},{from_y})->({to_x},{to_y})"),
            Self::KeyHold {
                key, hold_flips, ..
            } => format!("key_hold {key} hold={hold_flips}"),
            Self::Chord {
                keys, hold_flips, ..
            } => format!("chord [{}] hold={hold_flips}", keys.join("+")),
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
    Point { flip: u64, poll: u64 },
    FlipOrMs { flip: u64, declared_ms: u64 },
    Ms { declared_ms: u64 },
}

impl Due {
    fn is_due(self, flip: u64, poll: u64, declared_ms: u64) -> bool {
        match self {
            Self::Point { flip: f, poll: p } => (flip, poll) >= (f, p),
            Self::FlipOrMs {
                flip: f,
                declared_ms: ms,
            } => flip >= f || declared_ms >= ms,
            Self::Ms { declared_ms: ms } => declared_ms >= ms,
        }
    }

    fn sort_flip(self) -> u64 {
        match self {
            Self::Point { flip, .. } | Self::FlipOrMs { flip, .. } => flip,
            Self::Ms { .. } => 0,
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
#[derive(Clone, Debug)]
pub struct Timeline {
    next_seq: u64,
    pending: Vec<Scheduled>,
    last_pointer: (i32, i32),
    pointer_shown: bool,
    pointer_speed: (f64, f64),
    frame_size: (u32, u32),
    delta_rem: (f64, f64),
}

impl Timeline {
    #[must_use]
    pub fn new(pointer_speed: (f64, f64)) -> Self {
        Self {
            next_seq: 0,
            pending: Vec::new(),
            last_pointer: (0, 0),
            pointer_shown: true,
            pointer_speed,
            frame_size: (0, 0),
            delta_rem: (0.0, 0.0),
        }
    }

    /// Update the tracked guest pointer origin (e.g. from `Host::put_pointer`
    /// or an observed move). Fractional sub-pixel remainder is preserved across
    /// `put_pointer` recentering so `Mouse::Steered` titles do not drop
    /// sub-pixel deltas.
    pub fn set_pointer(&mut self, x: i32, y: i32) {
        self.last_pointer = (x, y);
    }

    /// Update whether the host pointer is shown (`false` means `Mouse::Steered`).
    pub fn set_pointer_shown(&mut self, shown: bool) {
        self.pointer_shown = shown;
    }

    /// Update the active guest frame dimensions used to clamp relative moves.
    pub fn set_frame_size(&mut self, width: u32, height: u32) {
        self.frame_size = (width, height);
    }

    #[must_use]
    pub fn last_pointer(&self) -> (i32, i32) {
        self.last_pointer
    }

    #[must_use]
    pub fn pointer_shown(&self) -> bool {
        self.pointer_shown
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.delta_rem = (0.0, 0.0);
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
        start_flip: u64,
        start_ms: u64,
    ) -> Result<ScheduleReport, ActionError> {
        let before = self.pending.len();
        let end_flip = match action {
            Action::Move { x, y, glide_flips } => {
                let steps = (*glide_flips).max(1);
                if steps == 1 {
                    self.push_input(start_flip, 0, Input::Move { x: *x, y: *y });
                    start_flip
                } else {
                    let (x0, y0) = self.last_pointer;
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
                let sx = scale_x.unwrap_or_else(|| {
                    if self.pointer_shown {
                        1.0
                    } else {
                        self.pointer_speed.0
                    }
                });
                let sy = scale_y.unwrap_or_else(|| {
                    if self.pointer_shown {
                        1.0
                    } else {
                        self.pointer_speed.1
                    }
                });
                let step_dx = (*dx * sx) / steps as f64;
                let step_dy = (*dy * sy) / steps as f64;
                for i in 0..steps {
                    self.push(
                        Due::Point {
                            flip: start_flip + i,
                            poll: 0,
                        },
                        Payload::MouseDeltaStep {
                            dx: step_dx,
                            dy: step_dy,
                        },
                    );
                }
                start_flip + steps - 1
            }
            Action::Click {
                x,
                y,
                button,
                hold_flips,
                hold_ms,
            } => {
                if let (Some(x), Some(y)) = (*x, *y) {
                    self.push_input(start_flip, 0, Input::Move { x, y });
                }
                self.push_input(start_flip, 0, Input::Down(*button));
                let hold = (*hold_flips).max(1);
                let rel_flip = start_flip.saturating_add(hold);
                let due = match hold_ms {
                    Some(ms) => Due::FlipOrMs {
                        flip: rel_flip,
                        declared_ms: start_ms.saturating_add(*ms),
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
                if let (Some(x), Some(y)) = (*x, *y) {
                    self.push_input(start_flip, 0, Input::Move { x, y });
                }
                self.push_input(start_flip, 0, Input::Down(*button));
                let target_ms = start_ms.saturating_add(*hold_ms);
                let (due, end_flip) = if *hold_flips == 0 {
                    (
                        Due::Ms {
                            declared_ms: target_ms,
                        },
                        start_flip,
                    )
                } else {
                    let rel_flip = start_flip.saturating_add(*hold_flips);
                    (
                        Due::FlipOrMs {
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
                self.push_input(
                    start_flip,
                    0,
                    Input::Move {
                        x: *from_x,
                        y: *from_y,
                    },
                );
                self.push_input(start_flip, 0, Input::Down(*button));
                let dx = (*to_x - *from_x).unsigned_abs();
                let dy = (*to_y - *from_y).unsigned_abs();
                let dist = dx.max(dy);
                let step = (*step_px).max(1);
                let steps = u64::from(dist.div_ceil(step).max(1));
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
                let rel_flip = start_flip + steps + 1;
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
                for &code in &codes {
                    self.push_input(start_flip, 0, Input::Key { code, down: true });
                }
                let hold = (*hold_flips).max(1);
                let rel_flip = start_flip.saturating_add(hold);
                let due = match hold_ms {
                    Some(ms) => Due::FlipOrMs {
                        flip: rel_flip,
                        declared_ms: start_ms.saturating_add(*ms),
                    },
                    None => Due::Point {
                        flip: rel_flip,
                        poll: 0,
                    },
                };
                for &code in codes.iter().rev() {
                    self.push(due, Payload::Input(Input::Key { code, down: false }));
                }
                rel_flip
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
    /// against the live tracked pointer.
    pub fn drain_due(&mut self, flip: u64, poll: u64, declared_ms: u64) -> Vec<Input> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let mut due_items = Vec::new();
        let mut remaining = Vec::with_capacity(self.pending.len());
        for item in self.pending.drain(..) {
            if item.due.is_due(flip, poll, declared_ms) {
                due_items.push(item);
            } else {
                remaining.push(item);
            }
        }
        self.pending = remaining;
        due_items.sort_by_key(|item| (item.due.sort_flip(), item.seq));

        let mut out = Vec::with_capacity(due_items.len());
        for item in due_items {
            match item.payload {
                Payload::Input(input) => {
                    if let Input::Move { x, y } = input {
                        self.last_pointer = (x, y);
                        self.delta_rem = (0.0, 0.0);
                    }
                    out.push(input);
                }
                Payload::MouseDeltaStep { dx, dy } => {
                    self.delta_rem.0 += dx;
                    self.delta_rem.1 += dy;
                    let step_x = self.delta_rem.0.round() as i32;
                    let step_y = self.delta_rem.1.round() as i32;
                    self.delta_rem.0 -= f64::from(step_x);
                    self.delta_rem.1 -= f64::from(step_y);
                    let mut nx = self.last_pointer.0.saturating_add(step_x);
                    let mut ny = self.last_pointer.1.saturating_add(step_y);
                    if self.frame_size.0 > 0 && self.frame_size.1 > 0 {
                        nx = nx.clamp(0, (self.frame_size.0 as i32).saturating_sub(1));
                        ny = ny.clamp(0, (self.frame_size.1 as i32).saturating_sub(1));
                    }
                    self.last_pointer = (nx, ny);
                    out.push(Input::Move { x: nx, y: ny });
                }
            }
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
                    hold_ms: None
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
}
