//! Composable [`ControlHost<H>`] decorator over any [`Host`] (#308).
//!
//! Wraps [`crate::host::headless::Headless`] or any native window backend
//! (`MacWindow` / `LinuxWindow` / `WinWindow`) without touching guest state.
//! Embeds `#296`'s pause-compensated [`crate::host::telemetry::Telemetry`],
//! pumps the wrapped window at 60 Hz (`16 ms` `condvar.wait_timeout`) while
//! paused in `Step` mode so the OS window never beachballs, enforces
//! equal-citizen [`InputOwner`] arbitration, and drives [`super::timeline::Timeline`]
//! events deterministically at flip boundaries and between-flip polls.

use super::bus::{
    analyze_frame, diff_pct_between, BusState, ControlBus, ControlConfig, ExecMode, FrameSnapshot,
    InputOwner, WaitCondition, WaitResult,
};
use crate::host::telemetry::FpsReport;
use crate::host::{Cursor, FlipMeta, Frame, Host, HostError, Input, Mapping};
use crate::machine::Finish;
use std::sync::Arc;
use std::time::Duration;

/// 60 Hz window-pump interval while paused inside [`ControlHost::observe_flip`].
const PAUSE_PUMP_INTERVAL: Duration = Duration::from_millis(16);

/// Drop guard that notifies any blocked clients if [`ControlHost`] unwinds or
/// drops without an explicit [`ControlHost::mark_finished`] call.
struct FinishGuard(ControlBus);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.mark_dropped();
    }
}

/// Composable [`Host`] wrapper uniting human play, live agent control, and
/// piped TAS/route tasks.
pub struct ControlHost<H> {
    inner: H,
    bus: ControlBus,
    _finish_guard: FinishGuard,
    defer_pause: bool,
    last_clock_nanos: u64,
    last_flip_observed: Option<u64>,
    polls_since_flip: u64,
    host_paused: bool,
    pace_anchor_declared: u64,
    pace_anchor_scaled: u64,
    staged_bgra: Vec<u8>,
    staged_size: (u32, u32),
    staged_valid: bool,
}

impl<H> ControlHost<H> {
    #[must_use]
    pub fn new(inner: H, config: ControlConfig) -> Self {
        Self::with_bus(inner, ControlBus::new(config))
    }

    #[must_use]
    pub fn with_bus(inner: H, bus: ControlBus) -> Self {
        let guard = FinishGuard(bus.clone());
        Self {
            inner,
            bus,
            _finish_guard: guard,
            defer_pause: false,
            last_clock_nanos: 0,
            last_flip_observed: None,
            polls_since_flip: 0,
            host_paused: false,
            pace_anchor_declared: 0,
            pace_anchor_scaled: 0,
            staged_bgra: Vec::new(),
            staged_size: (0, 0),
            staged_valid: false,
        }
    }

    /// Defer the `Step`/pause blocking loop to [`service_flip_boundary`] so
    /// read-only `&Machine` inspection requests can be served while paused
    /// without deadlocking (#309).
    #[must_use]
    pub fn with_deferred_pause(mut self, defer: bool) -> Self {
        self.defer_pause = defer;
        self
    }

    /// Reset per-run staging/pacing anchors across an outer-loop Tier-1 rewind (#309).
    pub fn reset_for_rewind(&mut self) {
        self.last_clock_nanos = 0;
        self.last_flip_observed = None;
        self.polls_since_flip = 0;
        self.pace_anchor_declared = 0;
        self.pace_anchor_scaled = 0;
        self.staged_bgra.clear();
        self.staged_size = (0, 0);
        self.staged_valid = false;
    }

    /// Clone a thread-safe handle to this host's [`ControlBus`].
    #[must_use]
    pub fn bus(&self) -> ControlBus {
        self.bus.clone()
    }

    /// Static per-title [`crate::title::TitleGuide`] configured on this host (#311).
    #[must_use]
    pub fn guide(&self) -> &'static crate::title::TitleGuide {
        self.bus.guide()
    }

    #[must_use]
    pub fn inner(&self) -> &H {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut H {
        &mut self.inner
    }

    #[must_use]
    pub fn into_inner(self) -> H {
        self.inner
    }

    /// Mark the run finished with its final [`Finish`] and `state_digest()`.
    pub fn mark_finished(&self, finish: &Finish, digest: &str) {
        self.bus.mark_finished(finish, digest);
    }

    /// Live `#296` telemetry report collected by this control host.
    #[must_use]
    pub fn telemetry_report(&self, window: Option<(u64, u64)>) -> FpsReport {
        self.bus.telemetry_report(window)
    }

    fn stage_frame(&mut self, frame: &Frame<'_>) {
        let row_bytes = (frame.width as usize).saturating_mul(4);
        let total_bytes = row_bytes.saturating_mul(frame.height as usize);
        let pitch = frame.pitch as usize;
        self.staged_bgra.clear();
        if total_bytes == 0 || pitch < row_bytes {
            self.staged_size = (frame.width, frame.height);
            self.staged_valid = false;
            return;
        }
        self.staged_bgra.reserve(total_bytes);
        if pitch == row_bytes {
            if let Some(src) = frame.pixels.get(..total_bytes) {
                self.staged_bgra.extend_from_slice(src);
                self.staged_size = (frame.width, frame.height);
                self.staged_valid = true;
            } else {
                self.staged_valid = false;
            }
        } else {
            let mut ok = true;
            for row in 0..frame.height as usize {
                let start = row * pitch;
                if let Some(src) = frame.pixels.get(start..start + row_bytes) {
                    self.staged_bgra.extend_from_slice(src);
                } else {
                    ok = false;
                    break;
                }
            }
            self.staged_size = (frame.width, frame.height);
            self.staged_valid = ok;
        }
    }
}

impl<H: Host> ControlHost<H> {
    fn set_host_paused(&mut self, paused: bool) {
        if self.host_paused != paused {
            self.host_paused = paused;
            self.inner.set_paused(paused);
            self.bus.lock().telemetry.set_paused(paused);
        }
    }
}

fn evaluate_wait(state: &mut BusState, meta: &FlipMeta, frame: Option<&FrameSnapshot>) -> bool {
    let Some(mut cond) = state.active_wait.take() else {
        return false;
    };
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
        state.last_wait_result = Some(result);
        state.wait_epoch += 1;
        true
    } else {
        state.active_wait = Some(cond);
        false
    }
}

impl<H: Host> Host for ControlHost<H> {
    fn present(&mut self, frame: &Frame<'_>) {
        self.stage_frame(frame);
        self.inner.present(frame);
    }

    fn poll_input(&mut self, flip: u64) -> Vec<Input> {
        let human = self.inner.poll_input(flip);

        let poll_index = if self.last_flip_observed == Some(flip) {
            let idx = self.polls_since_flip;
            self.polls_since_flip = self.polls_since_flip.saturating_add(1);
            idx
        } else {
            self.last_flip_observed = Some(flip);
            self.polls_since_flip = 1;
            0
        };

        let declared_ms = self.last_clock_nanos / 1_000_000;
        let mut state = self.bus.lock();
        if state.rewind_abort || state.rewind_target_flip.is_some() {
            return Vec::new();
        }
        let owner = state.owner;

        let mut out = Vec::new();
        match owner {
            InputOwner::Agent => {
                // Keep windowed host non-hijacking: ignore stray human
                // mouse/key events while still honoring OS window Close.
                for ev in human {
                    if ev == Input::Close {
                        out.push(ev);
                    }
                }
                let scheduled = state.timeline.drain_due(flip, poll_index, declared_ms);
                for ev in scheduled {
                    if ev == Input::Close && out.contains(&Input::Close) {
                        continue;
                    }
                    out.push(ev);
                }
            }
            InputOwner::Human => {
                out.extend(human);
            }
            InputOwner::Shared => {
                out.extend(human);
                let scheduled = state.timeline.drain_due(flip, poll_index, declared_ms);
                for ev in scheduled {
                    if ev == Input::Close && out.contains(&Input::Close) {
                        continue;
                    }
                    out.push(ev);
                }
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

    fn audio(&mut self, samples: &[i16]) {
        self.inner.audio(samples);
    }

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        self.inner.map(len)
    }

    fn show_pointer(&mut self, show: bool) {
        self.inner.show_pointer(show);
        let mut state = self.bus.lock();
        state.pointer_shown = show;
        state.timeline.set_pointer_shown(show);
    }

    fn free_mouse(&mut self) {
        self.inner.free_mouse();
    }

    fn put_pointer(&mut self, x: i32, y: i32) {
        self.inner.put_pointer(x, y);
        let mut state = self.bus.lock();
        state.pointer_pos = (x, y);
        state.timeline.set_pointer(x, y);
    }

    fn pointer_shape(&mut self, shape: Option<&Cursor>) {
        self.inner.pointer_shape(shape);
    }

    fn closes(&self) -> u32 {
        let rewind_close = if self.bus.is_rewind_aborting() { 2 } else { 0 };
        self.inner
            .closes()
            .saturating_add(self.bus.force_closes())
            .saturating_add(rewind_close)
    }

    fn audio_underruns(&self) -> Option<u64> {
        self.inner.audio_underruns()
    }

    fn pace(&mut self, declared_nanos: u64) -> f64 {
        self.last_clock_nanos = declared_nanos;
        let (uncapped, pace_mult) = {
            let state = self.bus.lock();
            let route_active = state.last_meta.is_some_and(|m| m.route_active);
            let uncapped = route_active
                || state.rewind_target_flip.is_some()
                || (state.mode == ExecMode::Step && state.step_uncapped);
            (uncapped, state.pace)
        };

        if uncapped {
            self.pace_anchor_declared = declared_nanos;
            return 1.0;
        }

        let delta = declared_nanos.saturating_sub(self.pace_anchor_declared);
        let scaled_delta = if (pace_mult - 1.0).abs() < 1e-6 {
            delta
        } else {
            (delta as f64 / pace_mult) as u64
        };
        let scaled = self.pace_anchor_scaled.saturating_add(scaled_delta);
        self.pace_anchor_declared = declared_nanos;
        self.pace_anchor_scaled = scaled;
        self.inner.pace(scaled)
    }

    fn pump(&mut self) {
        self.inner.pump();
    }

    fn set_paused(&mut self, paused: bool) {
        self.set_host_paused(paused);
    }

    fn observe_flip(&mut self, meta: &FlipMeta) {
        self.last_clock_nanos = meta.clock_nanos;
        self.last_flip_observed = Some(meta.flip);
        self.polls_since_flip = 0;

        self.inner.observe_flip(meta);
        let underruns = self.inner.audio_underruns();

        // Update frame snapshot, diff, telemetry, and active wait conditions.
        let mut notify_clients = false;
        let mut enter_pause = false;
        {
            let mut state = self.bus.lock();
            state.last_meta = Some(*meta);
            state.pointer_shown = meta.pointer_shown;
            state.timeline.set_pointer_shown(meta.pointer_shown);
            state.audio_underruns = underruns;

            let uniform = if self.staged_valid {
                let (w, h) = self.staged_size;
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
                    analyze_frame(prev, &self.staged_bgra, w, h, prev_streak)
                };
                let snapshot = FrameSnapshot {
                    flip: meta.flip,
                    width: w,
                    height: h,
                    bgra: Arc::new(self.staged_bgra.clone()),
                    frame_hash,
                    diff,
                    uniform,
                };
                state.latest_frame = Some(snapshot);
                uniform
            } else {
                true
            };

            state.telemetry.record_flip(meta, uniform);

            let latest_ref = state.latest_frame.clone();
            if evaluate_wait(&mut state, meta, latest_ref.as_ref()) {
                notify_clients = true;
            }

            if !self.defer_pause {
                if self.inner.closes() == 0 && state.should_pause(meta.route_active) {
                    if state.transport_eof
                        && state.mode == ExecMode::Step
                        && state.owner == InputOwner::Agent
                    {
                        // Piped stdin finished with no more commands coming; close
                        // cleanly so automated pipelines never hang.
                        state.close_pending = true;
                    } else {
                        state.is_paused = true;
                        state.step_epoch += 1;
                        notify_clients = true;
                        enter_pause = true;
                    }
                } else if !meta.route_active && state.active_wait.is_none() && state.step_budget > 0
                {
                    state.step_budget -= 1;
                }
            }
        }

        if notify_clients {
            self.bus.notify_client();
        }

        if enter_pause {
            self.set_host_paused(true);
            loop {
                let state = self.bus.lock();
                if self.inner.closes() > 0 || !state.should_pause(meta.route_active) {
                    let mut state = state;
                    state.is_paused = false;
                    if !meta.route_active && state.active_wait.is_none() && state.step_budget > 0 {
                        state.step_budget -= 1;
                    }
                    drop(state);
                    break;
                }
                if state.transport_eof
                    && state.mode == ExecMode::Step
                    && state.owner == InputOwner::Agent
                {
                    let mut state = state;
                    state.is_paused = false;
                    state.close_pending = true;
                    drop(state);
                    break;
                }
                let state = self.bus.wait_for_host_wake(state, PAUSE_PUMP_INTERVAL);
                // Release the bus lock before pumping OS events so clients can
                // query status/frames concurrently without waiting on AppKit/X11/Win32.
                drop(state);
                self.inner.pump();
            }
            self.set_host_paused(false);
        }
    }
}

/// Machine-boundary flip observer invoked from `Machine::observe_flip` when
/// [`ControlHost::with_deferred_pause`] is enabled (#309).
///
/// Serves read-only `&Machine` inspect requests, completes Tier-1 rewind
/// replays at their exact target flip, and runs the 60 Hz pause-pump loop
/// while continuing to answer inspect queries with zero deadlock.
pub fn service_flip_boundary(
    bus: &ControlBus,
    machine: &crate::machine::Machine,
    host: &mut dyn Host,
) {
    let rec_len = machine
        .recording
        .as_ref()
        .map_or(0, crate::route::Route::len);
    bus.update_machine_meta(rec_len);
    let _ = bus.complete_rewind_if_reached(machine);
    bus.drain_inspects(machine);

    let route_active = machine.undelivered() > 0;
    let host_closes = host.closes();
    let mut notify_clients = false;
    let mut enter_pause = false;
    {
        let mut state = bus.lock();
        if host_closes == 0 && state.should_pause(route_active) {
            if state.transport_eof
                && state.mode == ExecMode::Step
                && state.owner == InputOwner::Agent
            {
                state.close_pending = true;
            } else {
                state.is_paused = true;
                state.step_epoch += 1;
                notify_clients = true;
                enter_pause = true;
            }
        } else if !route_active
            && state.rewind_target_flip.is_none()
            && state.active_wait.is_none()
            && state.step_budget > 0
        {
            state.step_budget -= 1;
        }
    }

    if notify_clients {
        bus.notify_client();
    }

    if enter_pause {
        host.set_paused(true);
        loop {
            bus.drain_inspects(machine);
            let host_closes = host.closes();
            let state = bus.lock();
            if host_closes > 0 || state.rewind_abort || !state.should_pause(route_active) {
                let mut state = state;
                state.is_paused = false;
                if !route_active
                    && !state.rewind_abort
                    && state.active_wait.is_none()
                    && state.step_budget > 0
                {
                    state.step_budget -= 1;
                }
                drop(state);
                break;
            }
            if state.transport_eof
                && state.mode == ExecMode::Step
                && state.owner == InputOwner::Agent
            {
                let mut state = state;
                state.is_paused = false;
                state.close_pending = true;
                drop(state);
                break;
            }
            if !state.pending_inspects.is_empty() {
                drop(state);
                continue;
            }
            let state = bus.wait_for_host_wake(state, PAUSE_PUMP_INTERVAL);
            drop(state);
            host.pump();
        }
        host.set_paused(false);
        bus.drain_inspects(machine);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::control::bus::Command;
    use crate::control::timeline::Action;
    use crate::guest::GuestMem;
    use crate::host::headless::Headless;
    use crate::host::Button;
    use crate::machine::Machine;
    use crate::route::Route;
    use crate::runlog::RunLog;
    use crate::stub;
    use crate::Title;

    struct FakeWindowHost {
        inner: Headless,
        human_events: Vec<Input>,
        pumps: usize,
        pauses: Vec<bool>,
        closes: u32,
    }

    impl FakeWindowHost {
        fn new(human_events: Vec<Input>) -> Self {
            Self {
                inner: Headless::new(),
                human_events,
                pumps: 0,
                pauses: Vec::new(),
                closes: 0,
            }
        }
    }

    impl Host for FakeWindowHost {
        fn present(&mut self, frame: &Frame<'_>) {
            self.inner.present(frame);
        }
        fn poll_input(&mut self, _flip: u64) -> Vec<Input> {
            std::mem::take(&mut self.human_events)
        }
        fn audio(&mut self, samples: &[i16]) {
            self.inner.audio(samples);
        }
        fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
            self.inner.map(len)
        }
        fn closes(&self) -> u32 {
            self.closes
        }
        fn pump(&mut self) {
            self.pumps += 1;
        }
        fn set_paused(&mut self, paused: bool) {
            self.pauses.push(paused);
        }
    }

    #[test]
    fn owner_switching_arbitrates_agent_human_and_shared_inputs() {
        let mut host = ControlHost::new(
            FakeWindowHost::new(vec![
                Input::Move { x: 10, y: 20 },
                Input::Down(Button::Left),
                Input::Close,
            ]),
            ControlConfig {
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();

        // Agent mode: ignores stray human mouse/keys, still forwards Close,
        // and delivers scheduled agent actions.
        assert!(
            bus.dispatch(Command::Act {
                action: Action::Move {
                    x: 300,
                    y: 200,
                    glide_flips: 1,
                },
                auto_step: false,
            })
            .ok
        );
        let got = host.poll_input(0);
        assert_eq!(got, [Input::Close, Input::Move { x: 300, y: 200 }]);

        // Human mode: forwards human inputs directly, does not inject agent actions.
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Human)).ok);
        host.inner_mut().human_events = vec![
            Input::Move { x: 50, y: 60 },
            Input::Key {
                code: 0x39,
                down: true,
            },
        ];
        assert!(
            bus.dispatch(Command::Act {
                action: Action::Move {
                    x: 400,
                    y: 400,
                    glide_flips: 1,
                },
                auto_step: false,
            })
            .ok
        );
        let got = host.poll_input(1);
        assert_eq!(
            got,
            [
                Input::Move { x: 50, y: 60 },
                Input::Key {
                    code: 0x39,
                    down: true,
                },
            ]
        );

        // Shared mode: merges both human and previously queued agent inputs cleanly.
        assert!(bus.dispatch(Command::SetOwner(InputOwner::Shared)).ok);
        host.inner_mut().human_events = vec![Input::Down(Button::Right)];
        let got = host.poll_input(2);
        assert_eq!(
            got,
            [Input::Down(Button::Right), Input::Move { x: 400, y: 400 }]
        );
    }

    #[test]
    fn step_mode_pauses_pumps_at_60hz_and_releases_cleanly_on_step() {
        let mut host = ControlHost::new(
            FakeWindowHost::new(Vec::new()),
            ControlConfig {
                mode: ExecMode::Step,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();

        let pixels = vec![0x44u8; 4 * 4 * 4];
        host.present(&Frame {
            width: 4,
            height: 4,
            pitch: 16,
            pixels: &pixels,
        });

        let waiter_bus = bus.clone();
        let releaser = std::thread::spawn(move || {
            let st = waiter_bus.wait_until_paused(Some(Duration::from_secs(2)));
            assert!(st.paused);
            assert_eq!(st.flip, Some(0));
            // Give the pause loop time for at least one 16ms pump tick.
            std::thread::sleep(Duration::from_millis(45));
            let resp = waiter_bus.dispatch(Command::Step { flips: 1 });
            assert!(resp.ok);
        });

        host.observe_flip(&FlipMeta {
            flip: 0,
            clock_nanos: 16_000_000,
            cursor: 100,
            instructions_passed: 0,
            pointer_shown: true,
            pointer_pos: (0, 0),
            route_active: false,
            route_remaining: 0,
        });
        releaser.join().unwrap();

        assert!(host.inner().pumps >= 1, "window pumped while paused");
        assert_eq!(host.inner().pauses, [true, false]);
    }

    #[test]
    fn preroll_route_skips_pause_until_first_post_route_flip() {
        let mut host = ControlHost::new(
            FakeWindowHost::new(Vec::new()),
            ControlConfig {
                mode: ExecMode::Step,
                ..ControlConfig::default()
            },
        );
        let pixels = vec![0x22u8; 4 * 4 * 4];
        for flip in 0..3u64 {
            host.present(&Frame {
                width: 4,
                height: 4,
                pitch: 16,
                pixels: &pixels,
            });
            host.observe_flip(&FlipMeta {
                flip,
                clock_nanos: (flip + 1) * 16_000_000,
                cursor: (flip + 1) * 100,
                instructions_passed: 0,
                pointer_shown: true,
                pointer_pos: (0, 0),
                route_active: true,
                route_remaining: (3 - flip) as usize,
            });
        }
        assert!(
            host.inner().pauses.is_empty(),
            "route pre-roll must never pause"
        );
    }

    /// Build a tiny multi-flip stub program (presents `count` frames, writing a
    /// distinct pixel on flips `0..5` and settling on flips `5..count`, then
    /// exiting with `eax == count`).
    fn multi_flip_stub(count: u32) -> Vec<u8> {
        // 0x400000: mov ecx, count       ; b9 <count>           (5 bytes, 0..5)
        // 0x400005: mov eax, 0           ; b8 00 00 00 00       (5 bytes, 5..10)
        // loop (0x40000a):
        //   cmp eax, 5                   ; 83 f8 05             (3 bytes, 10..13)
        //   jge skip                     ; 7d 05                (2 bytes, 13..15)
        //   mov [0x100000], eax          ; a3 00 00 10 00       (5 bytes, 15..20)
        // skip (0x400014):
        //   int 0x20                     ; cd 20                (2 bytes, 20..22)
        //   inc eax                      ; 40                   (1 byte,  22..23)
        //   dec ecx                      ; 49                   (1 byte,  23..24)
        //   jnz loop                     ; 75 f0 (-16 -> 10)    (2 bytes, 24..26)
        //   int 0x21                     ; cd 21                (2 bytes, 26..28)
        let mut code = Vec::new();
        code.push(0xb9);
        code.extend_from_slice(&count.to_le_bytes());
        code.extend_from_slice(&[0xb8, 0, 0, 0, 0]);
        code.extend_from_slice(&[0x83, 0xf8, 0x05]);
        code.extend_from_slice(&[0x7d, 0x05]);
        code.extend_from_slice(&[0xa3, 0x00, 0x00, 0x10, 0x00]);
        code.extend_from_slice(&[0xcd, stub::INT_PRESENT]);
        code.push(0x40);
        code.push(0x49);
        code.extend_from_slice(&[0x75, (-16i8) as u8]);
        code.extend_from_slice(&[0xcd, stub::INT_EXIT]);
        code
    }

    #[test]
    fn deterministic_equivalence_control_host_recording_replays_bit_exact_on_headless() {
        // 1. Drive a multi-flip stub guest via ControlHost<Headless> + ControlBus
        //    exercising move, click, key_hold, chord, wait, wait_until_change,
        //    wait_until_settled, pause/resume, and pipe_route.
        let mut control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                mode: ExecMode::Step,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = control.bus();

        let driver_bus = bus.clone();
        let driver = std::thread::spawn(move || {
            let st = driver_bus.wait_until_paused(Some(Duration::from_secs(5)));
            assert!(st.paused);
            assert_eq!(st.flip, Some(0));

            // Move + click on flip 0..2.
            let r = driver_bus.dispatch_sync(
                Command::Act {
                    action: Action::Click {
                        x: Some(120),
                        y: Some(80),
                        button: Button::Left,
                        hold_flips: 1,
                        hold_ms: None,
                    },
                    auto_step: true,
                },
                Some(Duration::from_secs(5)),
            );
            assert!(r.ok);
            assert_eq!(r.status.flip, Some(2));

            // Wait until change (flip 2->3 mutates pixel 0).
            let r = driver_bus.dispatch_sync(
                Command::WaitUntilChange {
                    max_flips: 5,
                    min_diff_pct: 1.0,
                },
                Some(Duration::from_secs(5)),
            );
            assert!(r.ok);
            assert!(matches!(
                r.data,
                crate::control::bus::ResponseData::WaitResult(WaitResult {
                    satisfied: true,
                    flips_elapsed: 1,
                    ..
                })
            ));

            // Chord + key hold via batch (flips 3..6).
            let r = driver_bus.dispatch_sync(
                Command::Batch {
                    actions: vec![
                        Action::Chord {
                            keys: vec!["LeftShift".into(), "A".into()],
                            hold_flips: 1,
                            hold_ms: None,
                        },
                        Action::KeyHold {
                            key: "Space".into(),
                            hold_flips: 2,
                            hold_ms: None,
                        },
                    ],
                    auto_step: true,
                },
                Some(Duration::from_secs(5)),
            );
            assert!(r.ok);

            // Pipe a relative #lego-legacy-route 1 snippet (flips 6..9).
            let r = driver_bus.dispatch_sync(
                Command::PipeRoute {
                    route_text: "#lego-legacy-route 1\nflip 0 move 10 15\nflip 1 down right\nflip 2 up right\n".into(),
                    relative: true,
                    auto_step: true,
                },
                Some(Duration::from_secs(5)),
            );
            assert!(r.ok);

            // Fixed wait (2 flips).
            let r = driver_bus.dispatch_sync(
                Command::Wait {
                    flips: Some(2),
                    ms: None,
                },
                Some(Duration::from_secs(5)),
            );
            assert!(r.ok);

            // Wait until settled (flips >= 5 stop changing pixels).
            let r = driver_bus.dispatch_sync(
                Command::WaitUntilSettled {
                    max_flips: 10,
                    stable_flips: 2,
                    max_diff_pct: 0.0,
                },
                Some(Duration::from_secs(5)),
            );
            assert!(r.ok);
            assert!(matches!(
                r.data,
                crate::control::bus::ResponseData::WaitResult(WaitResult {
                    satisfied: true,
                    ..
                })
            ));

            // Pause then resume to finish remaining flips.
            assert!(driver_bus.dispatch(Command::Pause).ok);
            assert!(driver_bus.dispatch(Command::Resume).ok);
        });

        let mut machine = Machine::new(&Title::STUB, &mut control).expect("machine");
        machine.load_stub().expect("stub");
        machine
            .arena
            .write(stub::ENTRY, &multi_flip_stub(20))
            .expect("multi-flip stub");
        machine.recording = Some(Route::new());

        let mut log = RunLog::create("control-eq-live", crate::cli::Trace::Off).expect("log");
        let live_finish = machine.run(&mut control, &mut log, 100_000);
        let live_digest = machine.state_digest();
        control.mark_finished(&live_finish, &live_digest);
        driver.join().unwrap();

        assert_eq!(
            live_finish.outcome,
            crate::machine::Outcome::Exited { code: 20 }
        );
        assert_eq!(live_finish.flips, 20);

        let recorded_text = machine
            .recording
            .as_ref()
            .expect("recording enabled")
            .to_text();
        let recorded_route = Route::parse(&recorded_text).expect("valid recorded route");
        assert!(
            recorded_route.len() >= 10,
            "recorded route captured all actions: {recorded_text}"
        );

        // 2. Replay the exact recorded route on a bare Headless host.
        let mut replay_host = Headless::new();
        let mut replay_machine = Machine::new(&Title::STUB, &mut replay_host).expect("machine");
        replay_machine.load_stub().expect("stub");
        replay_machine
            .arena
            .write(stub::ENTRY, &multi_flip_stub(20))
            .expect("multi-flip stub");
        replay_machine.route = Some(recorded_route);

        let mut replay_log =
            RunLog::create("control-eq-replay", crate::cli::Trace::Off).expect("log");
        let replay_finish = replay_machine.run(&mut replay_host, &mut replay_log, 100_000);
        let replay_digest = replay_machine.state_digest();

        // 3. Assert bit-exact equivalence across all four determinism observables.
        assert_eq!(live_finish.outcome, replay_finish.outcome);
        assert_eq!(live_finish.cursor, replay_finish.cursor);
        assert_eq!(live_finish.flips, replay_finish.flips);
        assert_eq!(live_finish.declared_millis, replay_finish.declared_millis);
        assert_eq!(live_digest, replay_digest);

        // 4. Also verify real `--stub` through `run_digested` with ControlHost.
        let base_cli = Cli {
            stub: true,
            ..Cli::default()
        };
        let baseline = crate::testkit::run_headless(&Title::STUB, &base_cli, 100_000);
        let mut stub_control = ControlHost::new(
            Headless::new(),
            ControlConfig {
                mode: ExecMode::Live,
                ..ControlConfig::default()
            },
        );
        let (ctrl_finish, _, ctrl_digest) =
            crate::run::run_digested(&Title::STUB, &base_cli, &mut stub_control, 100_000)
                .expect("run stub under ControlHost");
        assert_eq!(baseline.finish, ctrl_finish);
        assert_eq!(baseline.digest, ctrl_digest);
    }

    #[test]
    fn static_menu_click_poll_releases_via_pace_declared_ms_without_new_flip() {
        let mut host = ControlHost::new(
            FakeWindowHost::new(Vec::new()),
            ControlConfig {
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();

        let pixels = vec![0x33u8; 4 * 4 * 4];
        host.present(&Frame {
            width: 4,
            height: 4,
            pitch: 16,
            pixels: &pixels,
        });
        host.pace(1_000_000_000); // 1000 ms
        host.observe_flip(&FlipMeta {
            flip: 10,
            clock_nanos: 1_000_000_000,
            cursor: 500,
            instructions_passed: 0,
            pointer_shown: true,
            pointer_pos: (0, 0),
            route_active: false,
            route_remaining: 0,
        });

        assert!(
            bus.dispatch(Command::Act {
                action: Action::ClickPoll {
                    x: Some(320),
                    y: Some(240),
                    button: Button::Left,
                    hold_flips: 5,
                    hold_ms: 150,
                },
                auto_step: false,
            })
            .ok
        );

        // Flip 10 boundary: Move + Down fire.
        assert_eq!(
            host.poll_input(10),
            [Input::Move { x: 320, y: 240 }, Input::Down(Button::Left),]
        );
        // Guest enters static menu loop (no new flip!): poll_slow advances pace
        // by 100 ms declared time and calls poll_input(10) again.
        host.pace(1_100_000_000);
        assert_eq!(host.poll_input(10), []);
        // Second 100 ms poll reaches 1200 ms >= 1000 + 150 ms -> Up fires!
        host.pace(1_200_000_000);
        assert_eq!(host.poll_input(10), [Input::Up(Button::Left)]);
    }

    #[test]
    fn pace_scales_live_declared_time_and_avoids_sleep_jump_after_uncapped_burst() {
        struct PaceProbe {
            inner: Headless,
            seen_nanos: Vec<u64>,
        }
        impl Host for PaceProbe {
            fn present(&mut self, frame: &Frame<'_>) {
                self.inner.present(frame);
            }
            fn poll_input(&mut self, flip: u64) -> Vec<Input> {
                self.inner.poll_input(flip)
            }
            fn audio(&mut self, samples: &[i16]) {
                self.inner.audio(samples);
            }
            fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
                self.inner.map(len)
            }
            fn pace(&mut self, declared_nanos: u64) -> f64 {
                self.seen_nanos.push(declared_nanos);
                1.0
            }
        }

        let mut host = ControlHost::new(
            PaceProbe {
                inner: Headless::new(),
                seen_nanos: Vec::new(),
            },
            ControlConfig {
                mode: ExecMode::Step,
                step_uncapped: true,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();

        // Uncapped burst in Step mode skips inner.pace completely.
        host.pace(10_000_000_000);
        assert!(host.inner().seen_nanos.is_empty());

        // Switch to Live 2.0x pace: +2s declared maps to +1s scaled, starting from 0 (no 10s jump).
        assert!(
            bus.dispatch(Command::SetMode {
                mode: ExecMode::Live,
                step_uncapped: None,
            })
            .ok
        );
        assert!(bus.dispatch(Command::SetPace(2.0)).ok);
        host.pace(10_000_000_000);
        host.pace(12_000_000_000);
        assert_eq!(host.inner().seen_nanos, [0, 1_000_000_000]);

        // Switch to 0.5x slow-mo: +1s declared maps to +2s scaled -> 3s total.
        assert!(bus.dispatch(Command::SetPace(0.5)).ok);
        host.pace(13_000_000_000);
        assert_eq!(host.inner().seen_nanos, [0, 1_000_000_000, 3_000_000_000]);
    }

    #[test]
    fn control_host_applies_relative_steered_scaling_hint_and_escape_guard() {
        use crate::title::{MouseMode, TitleGuide};

        const STEERED_GUIDE: TitleGuide = TitleGuide {
            display_name: "Steered Test",
            resolution: (640, 480),
            mouse_mode: MouseMode::RelativeSteered {
                virtual_span: (1280, 1280),
                recenter: (320, 240),
                start_corner: (0, 480),
            },
            ..TitleGuide::EMPTY
        };

        let mut host = ControlHost::new(
            Headless::new(),
            ControlConfig {
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                guide: &STEERED_GUIDE,
                ..ControlConfig::default()
            },
        );
        let bus = host.bus();
        assert_eq!(host.guide().display_name, "Steered Test");
        assert_eq!(bus.status().pointer_pos, (320, 240));
        assert!(!bus.status().pointer_shown);

        // Even with default pointer_speed (1.0, 1.0) on ControlConfig, MouseMode::RelativeSteered
        // auto-scales dx by 1280/640 = 2.0 and dy by 1280/480 = 8/3 around recenter (320, 240).
        assert!(
            bus.dispatch(Command::Act {
                action: Action::MouseDelta {
                    dx: 30.0,
                    dy: -15.0,
                    flips: 1,
                    scale_x: None,
                    scale_y: None,
                },
                auto_step: false,
            })
            .ok
        );
        assert_eq!(host.poll_input(0), [Input::Move { x: 380, y: 200 }]);

        // Explicit scale override still wins over the RelativeSteered hint.
        host.put_pointer(320, 240);
        assert!(
            bus.dispatch(Command::Act {
                action: Action::MouseDelta {
                    dx: 10.0,
                    dy: 10.0,
                    flips: 1,
                    scale_x: Some(1.0),
                    scale_y: Some(1.0),
                },
                auto_step: false,
            })
            .ok
        );
        assert_eq!(host.poll_input(1), [Input::Move { x: 330, y: 250 }]);
    }
}
