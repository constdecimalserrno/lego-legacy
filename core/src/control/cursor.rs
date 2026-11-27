//! Single-source host-side cursor kinematics, overlay visibility, and
//! flip/restore reconciliation model (#409).
//!
//! Strictly outside guest state (Golden Rules 1 & 2, ADR-0001): tracks the
//! Win32 hardware pointer anchor (`hw_pointer`, mirroring
//! `win32::State::last_move`) separately from the rendered in-game software
//! cursor (`visual_cursor`) so relative-mouse titles never lose alignment on
//! `SetCursorPos` recenter calls (#327, #374).

use crate::host::Input;
use crate::title::MouseMode;

/// Single-source host cursor kinematics and overlay visibility state.
#[derive(Clone, Debug)]
pub struct CursorModel {
    hw_pointer: (i32, i32),
    visual_cursor: (f64, f64),
    pointer_shown: bool,
    cursor_blank: bool,
    mouse_mode: MouseMode,
    pointer_speed: (f64, f64),
    frame_size: (u32, u32),
    owed_delta: (f64, f64),
}

impl CursorModel {
    #[must_use]
    pub fn new(pointer_speed: (f64, f64)) -> Self {
        Self {
            hw_pointer: (320, 240),
            visual_cursor: (320.0, 240.0),
            pointer_shown: true,
            cursor_blank: false,
            mouse_mode: MouseMode::AbsoluteKept,
            pointer_speed,
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
        self.cursor_blank = false;
        self.owed_delta = (0.0, 0.0);
    }

    /// Update the tracked Win32 hardware pointer anchor (from `Host::put_pointer`
    /// or non-relative flip reconciliation).
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

    /// Record one emitted `Input::Move { x, y }` (whether from the timeline,
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
    ///
    /// On [`MouseMode::SoftwareRelative`], transitioning from OS-visible
    /// (`true`) back to software-hidden (`false`) — e.g. when a title
    /// unacquires and re-acquires `DISCL_EXCLUSIVE` DirectInput mouse across a
    /// menu/gameplay transition — re-anchors `visual_cursor` to `start_pos`
    /// while leaving always-hidden (`false -> false`) transitions untouched
    /// (#374).
    pub fn set_pointer_shown(&mut self, shown: bool) {
        let was_shown = self.pointer_shown;
        self.pointer_shown = shown;
        if was_shown && !shown {
            if let MouseMode::SoftwareRelative { start_pos, .. } = self.mouse_mode {
                self.visual_cursor =
                    self.clamp_visual(f64::from(start_pos.0), f64::from(start_pos.1));
                self.owed_delta = (0.0, 0.0);
            }
        }
    }

    /// Update whether the active OS cursor shape is blank (`SetCursor(NULL)` /
    /// all-transparent `Cursor::none()`).
    pub fn set_cursor_blank(&mut self, blank: bool) {
        self.cursor_blank = blank;
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

    /// Single-source flip-boundary cursor reconciliation (#409).
    pub fn reconcile_flip(
        &mut self,
        pointer_pos: (i32, i32),
        pointer_shown: bool,
        frame_size: Option<(u32, u32)>,
    ) {
        self.set_pointer_shown(pointer_shown);
        if !self.mouse_mode.is_relative()
            && (pointer_shown || matches!(self.mouse_mode, MouseMode::SoftwareAbsolute { .. }))
        {
            self.set_pointer(pointer_pos.0, pointer_pos.1);
        }
        if let Some((w, h)) = frame_size {
            self.set_frame_size(w, h);
        }
    }

    /// Single-source post-restore cursor reconciliation (#409).
    pub fn reconcile_restore(
        &mut self,
        pointer_pos: (i32, i32),
        hw_pointer: (i32, i32),
        pointer_shown: bool,
        cursor_blank: bool,
        restored_visual: Option<(i32, i32)>,
        frame_size: Option<(u32, u32)>,
    ) {
        if let Some((w, h)) = frame_size {
            self.set_frame_size(w, h);
        }
        self.owed_delta = (0.0, 0.0);
        self.set_pointer_shown(pointer_shown);
        self.set_cursor_blank(cursor_blank);
        if self.mouse_mode.is_relative() {
            self.set_pointer(hw_pointer.0, hw_pointer.1);
            if let Some((vx, vy)) = restored_visual {
                self.visual_cursor = self.clamp_visual(f64::from(vx), f64::from(vy));
            }
        } else {
            self.set_pointer(pointer_pos.0, pointer_pos.1);
        }
    }

    #[must_use]
    pub const fn mouse_mode(&self) -> MouseMode {
        self.mouse_mode
    }

    /// Win32 hardware pointer anchor (`last_move`).
    #[must_use]
    pub const fn hw_pointer(&self) -> (i32, i32) {
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
    pub const fn pointer_shown(&self) -> bool {
        self.pointer_shown
    }

    #[must_use]
    pub const fn cursor_blank(&self) -> bool {
        self.cursor_blank
    }

    /// Single-source predicate for whether cursor overlays (`--cursor`,
    /// `/watch` `#reticle`, WS v2 `cursor_visible()`) should render: `false`
    /// only when the OS cursor is shown (`ShowCursor >= 0`) with a blank
    /// cursor shape (`SetCursor(NULL)` / `Cursor::none()`).
    #[must_use]
    pub const fn overlay_visible(pointer_shown: bool, cursor_blank: bool) -> bool {
        !(pointer_shown && cursor_blank)
    }

    /// Whether cursor overlays should render for the current cursor state.
    #[must_use]
    pub const fn is_overlay_visible(&self) -> bool {
        Self::overlay_visible(self.pointer_shown, self.cursor_blank)
    }

    /// Whether unemitted owed relative delta (> 0.5 px) remains for subsequent
    /// recenter flips (strict `> 0.5` paired with `round_ties_even` prevents
    /// half-integer `±0.5` oscillation).
    #[must_use]
    pub fn has_owed_delta(&self) -> bool {
        self.owed_delta.0.abs() > 0.5 || self.owed_delta.1.abs() > 0.5
    }

    /// Number of recenter flips needed to pay back currently owed relative delta (`0` if none).
    #[must_use]
    pub fn owed_payback_flips(&self) -> u64 {
        if self.has_owed_delta() {
            self.min_flips_for_raw_delta(self.owed_delta.0, self.owed_delta.1)
        } else {
            0
        }
    }

    pub fn clear_owed_delta(&mut self) {
        self.owed_delta = (0.0, 0.0);
    }

    pub fn add_owed_delta(&mut self, dx: f64, dy: f64) {
        self.owed_delta.0 += dx;
        self.owed_delta.1 += dy;
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

    fn planned_visual_with_pending(&self, pending_raw: (f64, f64)) -> (f64, f64) {
        let raw_x = self.owed_delta.0 + pending_raw.0;
        let raw_y = self.owed_delta.1 + pending_raw.1;
        let (sx, sy) = self.active_scale();
        self.clamp_visual(
            self.visual_cursor.0 + raw_x / sx,
            self.visual_cursor.1 + raw_y / sy,
        )
    }

    /// Compute `(steps, step_dx, step_dy)` for a relative move toward
    /// `(target_x, target_y)` given already-queued raw delta `pending_raw`.
    #[must_use]
    pub fn plan_relative_move(
        &self,
        target_x: i32,
        target_y: i32,
        pending_raw: (f64, f64),
        min_steps: u64,
    ) -> (u64, f64, f64) {
        let (tx, ty) = self.clamp_screen_i32(target_x, target_y);
        let (cur_x, cur_y) = self.planned_visual_with_pending(pending_raw);
        let (sx, sy) = self.active_scale();
        let raw_dx = (f64::from(tx) - cur_x) * sx;
        let raw_dy = (f64::from(ty) - cur_y) * sy;
        let steps = min_steps
            .max(self.min_flips_for_raw_delta(raw_dx, raw_dy))
            .max(1);
        (steps, raw_dx / steps as f64, raw_dy / steps as f64)
    }

    /// Two-flip hover-arrival move steps for settled clicks in
    /// [`MouseMode::SoftwareRelative`]: lands within 1 px of
    /// `(target_x, target_y)` on the first flip, then completes the final 1-px
    /// arrival delta on the second flip (or a ±1 px touch when already at
    /// target) (#374).
    #[must_use]
    pub fn plan_software_relative_click_move(
        &self,
        target_x: i32,
        target_y: i32,
        pending_raw: (f64, f64),
    ) -> [(f64, f64); 2] {
        let (tx, ty) = self.clamp_screen_i32(target_x, target_y);
        let (cur_x, cur_y) = self.planned_visual_with_pending(pending_raw);
        let (sx, sy) = self.active_scale();
        let raw_dx = (f64::from(tx) - cur_x) * sx;
        let raw_dy = (f64::from(ty) - cur_y) * sy;
        if raw_dx.abs() >= 2.0 || raw_dy.abs() >= 2.0 {
            let (nx, ny) = if raw_dx.abs() >= raw_dy.abs() {
                (raw_dx.signum(), 0.0)
            } else {
                (0.0, raw_dy.signum())
            };
            [(raw_dx - nx, raw_dy - ny), (nx, ny)]
        } else {
            let mut touch_mag = sx.ceil().max(1.0);
            if ((raw_dx + touch_mag).fract().abs() - 0.5).abs() < 1e-9 {
                touch_mag += 1.0;
            }
            let has_right_room = self.frame_size.0 == 0
                || f64::from(tx) + touch_mag / sx <= f64::from(self.frame_size.0.saturating_sub(1));
            let touch = if has_right_room {
                touch_mag
            } else {
                -touch_mag
            };
            [(raw_dx + touch, raw_dy), (-touch, 0.0)]
        }
    }

    /// Plan scaled per-flip `(steps, step_dx, step_dy, needed_flips)` for an
    /// `Action::MouseDelta`.
    #[must_use]
    pub fn plan_mouse_delta(
        &self,
        dx: f64,
        dy: f64,
        flips: u64,
        scale_x: Option<f64>,
        scale_y: Option<f64>,
    ) -> (u64, f64, f64, u64) {
        let steps = flips.max(1);
        let (def_x, def_y) = if self.pointer_shown && !self.mouse_mode.is_relative() {
            (1.0, 1.0)
        } else {
            self.active_scale()
        };
        let sane_scale = |v: Option<f64>, d: f64| match v {
            Some(s) if s.is_finite() && s > 0.0 => s,
            _ => d,
        };
        let sx = sane_scale(scale_x, def_x);
        let sy = sane_scale(scale_y, def_y);
        let sane_d = |v: f64| {
            if v.is_finite() {
                v.clamp(-1_000_000.0, 1_000_000.0)
            } else {
                0.0
            }
        };
        let total_dx = (sane_d(dx) * sx).clamp(-1_000_000.0, 1_000_000.0);
        let total_dy = (sane_d(dy) * sy).clamp(-1_000_000.0, 1_000_000.0);
        let step_dx = total_dx / steps as f64;
        let step_dy = total_dy / steps as f64;
        let needed =
            steps.max(self.min_flips_for_raw_delta(
                self.owed_delta.0 + total_dx,
                self.owed_delta.1 + total_dy,
            ));
        (steps, step_dx, step_dy, needed)
    }

    /// Pay back accumulated `owed_delta` against `hw_pointer`, clamping to the
    /// active frame when required and carrying remainder across recenter flips.
    pub fn flush_owed_delta(&mut self, force_emit: bool) -> Option<Input> {
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
            return None;
        }
        let raw_nx = self.hw_pointer.0.saturating_add(want_x);
        let raw_ny = self.hw_pointer.1.saturating_add(want_y);
        let (nx, ny) = if self.clamps_hw_pointer() {
            self.clamp_screen_i32(raw_nx, raw_ny)
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
            return None;
        }
        self.observe_emitted_move(nx, ny);
        Some(Input::Move { x: nx, y: ny })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::bus::{ControlBus, ControlConfig, ExecMode, InputOwner};
    use crate::control::host::ControlHost;
    use crate::control::observe::{render_frame_png, ObserveOptions};
    use crate::control::ws::{
        decode_binary_frame, encode_binary_frame_with_pointer_state, FRAME_FLAG_PNG_KEYFRAME,
    };
    use crate::host::headless::Headless;
    use crate::host::{Cursor, FlipMeta, Frame, Host};
    use crate::testkit::spectator::crosshair_center;
    use crate::title::TitleGuide;

    #[test]
    fn mouse_mode_transitions_cover_all_four_regimes() {
        // 1. AbsoluteKept: hw_pointer and visual_cursor stay locked; flip updates both.
        let mut abs = CursorModel::new((1.0, 1.0));
        abs.configure_mode(MouseMode::AbsoluteKept, (640, 480));
        assert!(abs.pointer_shown());
        assert_eq!(abs.hw_pointer(), (320, 240));
        assert_eq!(abs.visual_cursor_pos(), (320, 240));
        abs.reconcile_flip((120, 80), true, Some((640, 480)));
        assert_eq!(abs.hw_pointer(), (120, 80));
        assert_eq!(abs.visual_cursor_pos(), (120, 80));
        // If OS cursor is temporarily hidden in AbsoluteKept, flip does not clobber pointer.
        abs.reconcile_flip((50, 50), false, None);
        assert!(!abs.pointer_shown());
        assert_eq!(abs.visual_cursor_pos(), (120, 80));

        // 2. SoftwareAbsolute: starts hidden at center, flip reconciles even when !pointer_shown,
        //    and wall overflow on owed_delta discards excess without wedging.
        let mut sa = CursorModel::new((1.0, 1.0));
        sa.configure_mode(
            MouseMode::SoftwareAbsolute {
                requires_poll_release: true,
            },
            (640, 480),
        );
        assert!(!sa.pointer_shown());
        assert_eq!(sa.visual_cursor_pos(), (320, 240));
        sa.reconcile_flip((410, 190), false, Some((640, 480)));
        assert_eq!(sa.hw_pointer(), (410, 190));
        assert_eq!(sa.visual_cursor_pos(), (410, 190));
        sa.add_owed_delta(500.0, 0.0);
        assert_eq!(
            sa.flush_owed_delta(true),
            Some(Input::Move { x: 639, y: 190 })
        );
        assert!(!sa.has_owed_delta());

        // 3. SoftwareRelative: starts at start_pos, preserves visual_cursor across SetCursorPos
        //    and false->false transitions, and re-anchors to start_pos on shown true->false.
        let mut sr = CursorModel::new((1.0, 1.0));
        sr.configure_mode(
            MouseMode::SoftwareRelative {
                virtual_span: (640, 480),
                start_pos: (0, 0),
            },
            (640, 480),
        );
        assert!(!sr.pointer_shown());
        assert_eq!(sr.hw_pointer(), (320, 240));
        assert_eq!(sr.visual_cursor_pos(), (0, 0));
        sr.observe_emitted_move(420, 290);
        assert_eq!(sr.visual_cursor_pos(), (100, 50));
        // SetCursorPos transition updates hw_pointer only.
        sr.set_pointer(320, 240);
        assert_eq!(sr.hw_pointer(), (320, 240));
        assert_eq!(sr.visual_cursor_pos(), (100, 50));
        // Always-hidden false->false transition preserves visual_cursor.
        sr.reconcile_flip((320, 240), false, Some((640, 480)));
        assert_eq!(sr.visual_cursor_pos(), (100, 50));
        // Unacquire (false->true) then re-acquire (true->false) re-anchors to start_pos.
        sr.reconcile_flip((320, 240), true, None);
        assert_eq!(sr.visual_cursor_pos(), (100, 50));
        sr.add_owed_delta(40.0, 20.0);
        sr.reconcile_flip((320, 240), false, None);
        assert_eq!(sr.visual_cursor_pos(), (0, 0));
        assert!(!sr.has_owed_delta());

        // 4. RelativeSteered: starts at clamped start_corner, chunks owed_delta across
        //    recenter flips, and never oscillates on half-integer ±0.5 remainders.
        let mut rs = CursorModel::new((2.0, 2.0));
        rs.configure_mode(
            MouseMode::RelativeSteered {
                virtual_span: (1280, 960),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            (640, 480),
        );
        assert!(!rs.pointer_shown());
        assert_eq!(rs.hw_pointer(), (320, 240));
        assert_eq!(rs.visual_cursor_pos(), (0, 479));
        // +600 raw px exceeds 639 - 320 = 319 -> carries 281 raw px across recenter flip.
        rs.add_owed_delta(600.0, 0.0);
        assert_eq!(rs.min_flips_for_raw_delta(600.0, 0.0), 2);
        assert_eq!(
            rs.flush_owed_delta(true),
            Some(Input::Move { x: 639, y: 240 })
        );
        assert!(rs.has_owed_delta());
        rs.set_pointer(320, 240);
        assert_eq!(
            rs.flush_owed_delta(false),
            Some(Input::Move { x: 601, y: 240 })
        );
        assert!(!rs.has_owed_delta());
        assert_eq!(rs.visual_cursor_pos(), (300, 479));
        // Half-integer ±0.5 remainder must not emit oscillation moves.
        rs.add_owed_delta(0.5, -0.5);
        assert!(!rs.has_owed_delta());
        assert_eq!(rs.flush_owed_delta(false), None);
    }

    #[test]
    fn reconcile_restore_resets_cursor_for_relative_and_absolute_modes() {
        let mut rel = CursorModel::new((2.0, 2.0));
        rel.configure_mode(
            MouseMode::RelativeSteered {
                virtual_span: (1280, 960),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            (640, 480),
        );
        rel.add_owed_delta(100.0, 50.0);
        rel.reconcile_restore(
            (320, 240),
            (310, 230),
            false,
            true,
            Some((444, 222)),
            Some((640, 480)),
        );
        assert_eq!(rel.hw_pointer(), (310, 230));
        assert_eq!(rel.visual_cursor_pos(), (444, 222));
        assert!(!rel.pointer_shown());
        assert!(rel.cursor_blank());
        assert!(rel.is_overlay_visible());
        assert!(!rel.has_owed_delta());

        let mut abs = CursorModel::new((1.0, 1.0));
        abs.configure_mode(MouseMode::AbsoluteKept, (640, 480));
        abs.reconcile_restore((180, 95), (10, 10), true, true, None, Some((640, 480)));
        assert_eq!(abs.hw_pointer(), (180, 95));
        assert_eq!(abs.visual_cursor_pos(), (180, 95));
        assert!(abs.pointer_shown());
        assert!(abs.cursor_blank());
        assert!(!abs.is_overlay_visible());
    }

    #[test]
    fn overlay_visibility_parity_across_cursor_model_observe_and_ws() {
        const GUIDE: TitleGuide = TitleGuide {
            display_name: "Visibility Parity",
            resolution: (64, 48),
            mouse_mode: MouseMode::AbsoluteKept,
            ..TitleGuide::EMPTY
        };
        let mut host = ControlHost::new(
            Headless::new(),
            ControlConfig {
                title: "parity",
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                guide: &GUIDE,
                ..ControlConfig::default()
            },
        );
        let bus: ControlBus = host.bus();
        let bgra = vec![0x20u8; 64 * 48 * 4];
        let opaque = Cursor {
            width: 1,
            height: 1,
            hot: (0, 0),
            pixels: vec![255, 255, 255, 255],
        };
        let blank_cursor = Cursor::none();

        for (idx, (shown, blank, expected_visible)) in [
            (true, false, true),
            (true, true, false),
            (false, false, true),
            (false, true, true),
        ]
        .into_iter()
        .enumerate()
        {
            let mut model = CursorModel::new((1.0, 1.0));
            model.set_pointer_shown(shown);
            model.set_cursor_blank(blank);
            assert_eq!(
                model.is_overlay_visible(),
                expected_visible,
                "CursorModel visibility for shown={shown} blank={blank}"
            );

            host.pointer_shape(if blank {
                Some(&blank_cursor)
            } else {
                Some(&opaque)
            });
            host.present(&Frame {
                width: 64,
                height: 48,
                pitch: 64 * 4,
                pixels: &bgra,
            });
            host.observe_flip(&FlipMeta {
                flip: idx as u64,
                clock_nanos: (idx as u64 + 1) * 16_000_000,
                cursor: (idx as u64 + 1) * 100,
                instructions_passed: 0,
                pointer_shown: shown,
                pointer_pos: (32, 24),
                route_active: false,
                route_remaining: 0,
            });

            let snap = bus.latest_frame().expect("frame snapshot");
            let status = bus.status();
            assert_eq!(snap.cursor_visible, expected_visible);
            assert_eq!(status.cursor_visible, expected_visible);

            let mut status_json = String::new();
            crate::control::json::write_status(&mut status_json, &status);
            let status_parsed = crate::control::json::parse_json(&status_json).unwrap();
            let status_obj = status_parsed.as_object().unwrap();
            assert_eq!(status_obj["pointer_shown"].as_bool(), Some(shown));
            assert_eq!(status_obj["cursor_blank"].as_bool(), Some(blank));
            assert_eq!(
                status_obj["cursor_visible"].as_bool(),
                Some(expected_visible)
            );

            let frame_json = crate::control::observe::format_frame_json(
                &snap,
                4,
                3,
                snap.pointer_pos,
                snap.pointer_shown,
                false,
                false,
                false,
            );
            let frame_parsed = crate::control::json::parse_json(&frame_json).unwrap();
            let frame_obj = frame_parsed.as_object().unwrap();
            assert_eq!(frame_obj["pointer_shown"].as_bool(), Some(shown));
            assert_eq!(frame_obj["cursor_blank"].as_bool(), Some(blank));
            assert_eq!(
                frame_obj["cursor_visible"].as_bool(),
                Some(expected_visible)
            );

            let png = render_frame_png(
                &snap,
                ObserveOptions {
                    cursor: true,
                    ..ObserveOptions::default()
                },
                snap.pointer_pos,
            )
            .expect("render png");
            let observe_visible = crosshair_center(&snap.bgra, &png)
                .expect("decode png")
                .is_some();

            let wire = encode_binary_frame_with_pointer_state(
                FRAME_FLAG_PNG_KEYFRAME,
                snap.flip,
                snap.width,
                snap.height,
                snap.pointer_pos,
                snap.pointer_shown,
                snap.cursor_blank,
                b"ok",
            );
            let ws_visible = decode_binary_frame(&wire)
                .expect("decode ws header")
                .cursor_visible();

            assert_eq!(
                model.is_overlay_visible(),
                observe_visible,
                "CursorModel vs observe mismatch for shown={shown} blank={blank}"
            );
            assert_eq!(
                model.is_overlay_visible(),
                ws_visible,
                "CursorModel vs ws mismatch for shown={shown} blank={blank}"
            );
        }
    }
}
