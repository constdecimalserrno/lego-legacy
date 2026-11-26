//! Dev-only host telemetry (`--fps-report`, `--fps-window`), strictly outside
//! guest state (#296).
//!
//! Measures all five speed quantities defined in `CONTEXT.md` (throughput in
//! MIPS excluding passed clock spins, pace ratio, host frame rate, guest
//! presentation rate, and declared intended rate) alongside pause-compensated
//! per-flip wall-time percentiles (`p50`, `p95`, `p99`, `max`), sub-realtime
//! frame counts, frame stalls, uniform frames, and host audio underruns.

use super::{Cursor, FlipMeta, Frame, Host, HostError, Input, Mapping};
use crate::clock::FlipCost;
use std::time::Instant;

/// Per-flip host wall-time threshold (100 ms) counted as a frame stall.
pub const STALL_THRESHOLD_NANOS: u64 = 100_000_000;

/// One flip's host-side observation, recorded strictly outside guest state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlipSample {
    /// Zero-based flip index.
    pub flip: u64,
    /// Cumulative declared clock nanoseconds at this flip.
    pub clock_nanos: u64,
    /// Cumulative guest cursor at this flip.
    pub cursor: u64,
    /// Cumulative clock-spin instructions passed at this flip.
    pub instructions_passed: u64,
    /// Cumulative unpaused host wall nanoseconds at this flip.
    pub wall_nanos: u64,
    /// Unpaused host wall nanoseconds elapsed for this flip.
    pub wall_delta_nanos: u64,
    /// Intentional host pacing sleep nanoseconds during this flip, excluded
    /// from frame compute budget checks (#349).
    pub pace_sleep_delta_nanos: u64,
    /// Declared clock nanoseconds elapsed for this flip.
    pub declared_delta_nanos: u64,
    /// Guest instructions actually executed (excluding passed clock spins) for
    /// this flip.
    pub executed_delta: u64,
    /// Whether every visible pixel of the presented frame was identical.
    pub uniform: bool,
}

/// Aggregated speed and frame-time metrics over a full run or flip window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowMetrics {
    pub flips: u64,
    pub wall_nanos: u64,
    pub declared_nanos: u64,
    pub executed_insns: u64,
    /// Executed guest instructions (excluding passed spins) per unpaused
    /// wall-second, in millions.
    pub throughput_mips: f64,
    /// Declared game-seconds per unpaused wall-second (`>= 1.00` is real time).
    pub pace: f64,
    /// Presented frames per unpaused wall-second.
    pub host_fps: f64,
    /// Presented frames per declared game-second (presentation rate).
    pub guest_fps: f64,
    /// Title's declared refresh rate from `FlipCost::Refresh(hz)`, or `None`
    /// for `FlipCost::Free`.
    pub intended_fps: Option<f64>,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    /// Flips whose compute wall time (excluding intentional pacing sleep)
    /// exceeded their declared frame budget.
    pub over_budget_flips: u64,
    /// Percentage of flips whose compute wall time exceeded their declared
    /// frame budget.
    pub over_budget_pct: f64,
    /// Flips whose unpaused wall time reached or exceeded
    /// [`STALL_THRESHOLD_NANOS`].
    pub stall_flips: u64,
    /// Uniform (solid-color) frames presented.
    pub uniform_flips: u64,
    /// Host audio device underruns reported by the wrapped host.
    pub audio_underruns: Option<u64>,
}

/// Complete telemetry summary for a run, with optional active-window slice.
#[derive(Clone, Debug, PartialEq)]
pub struct FpsReport {
    pub full: WindowMetrics,
    pub window: Option<(u64, u64, WindowMetrics)>,
}

impl WindowMetrics {
    /// Format one greppable summary line for `RunLog` and stdout.
    #[must_use]
    pub fn format_line(&self, scope: &str) -> String {
        let wall_s = self.wall_nanos as f64 / 1e9;
        let declared_s = self.declared_nanos as f64 / 1e9;
        let intended = self
            .intended_fps
            .map_or_else(|| "free".to_owned(), |hz| format!("{hz:.1}"));
        let underruns = self
            .audio_underruns
            .map_or_else(|| "none".to_owned(), |n| n.to_string());
        format!(
            "fps-report [{scope}] flips={} wall={wall_s:.2}s declared={declared_s:.2}s \
             throughput={:.1} MIPS pace={:.2}x host_fps={:.1} guest_fps={:.1} \
             intended_fps={intended} p50={:.2}ms p95={:.2}ms p99={:.2}ms max={:.2}ms \
             over_budget={}({:.1}%) stalls={} uniform={} underruns={underruns}",
            self.flips,
            self.throughput_mips,
            self.pace,
            self.host_fps,
            self.guest_fps,
            self.p50_ms,
            self.p95_ms,
            self.p99_ms,
            self.max_ms,
            self.over_budget_flips,
            self.over_budget_pct,
            self.stall_flips,
            self.uniform_flips,
        )
    }
}

impl FpsReport {
    /// One line for `[full]`, plus a second line for `[start..end]` if set.
    #[must_use]
    pub fn format_lines(&self) -> Vec<String> {
        let mut lines = vec![self.full.format_line("full")];
        if let Some((start, end, metrics)) = self.window {
            lines.push(metrics.format_line(&format!("{start}..{end}")));
        }
        lines
    }
}

/// Nearest-rank percentile in milliseconds from an ascending slice of
/// nanosecond durations.
#[must_use]
pub fn percentile_ms(sorted_nanos: &[u64], pct: f64) -> f64 {
    if sorted_nanos.is_empty() {
        return 0.0;
    }
    let rank = ((pct / 100.0) * sorted_nanos.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted_nanos.len() - 1);
    sorted_nanos[idx] as f64 / 1_000_000.0
}

/// Whether every visible 32-bit BGRA pixel in `frame` is identical, ignoring
/// any per-row pitch padding.
#[must_use]
pub fn is_uniform_frame(frame: &Frame<'_>) -> bool {
    let row_bytes = (frame.width as usize).saturating_mul(4);
    let height = frame.height as usize;
    let pitch = frame.pitch as usize;
    if row_bytes == 0 || height == 0 || pitch < row_bytes {
        return true;
    }
    let Some(first_row) = frame.pixels.get(..row_bytes) else {
        return true;
    };
    let first = [first_row[0], first_row[1], first_row[2], first_row[3]];
    let first_u32 = u32::from_ne_bytes(first);
    let first_u128 = u128::from_ne_bytes([
        first[0], first[1], first[2], first[3], first[0], first[1], first[2], first[3], first[0],
        first[1], first[2], first[3], first[0], first[1], first[2], first[3],
    ]);
    let row_uniform = |bytes: &[u8]| {
        let (quads, rem) = bytes.as_chunks::<16>();
        if !quads.iter().all(|&q| u128::from_ne_bytes(q) == first_u128) {
            return false;
        }
        rem.as_chunks::<4>()
            .0
            .iter()
            .all(|&p| u32::from_ne_bytes(p) == first_u32)
    };
    if pitch == row_bytes {
        let total = row_bytes.saturating_mul(height);
        let Some(all) = frame.pixels.get(..total) else {
            return true;
        };
        return row_uniform(all);
    }
    for row in 0..height {
        let start = row * pitch;
        let Some(slice) = frame.pixels.get(start..start + row_bytes) else {
            return true;
        };
        if !row_uniform(slice) {
            return false;
        }
    }
    true
}

/// Pause-compensated per-flip host telemetry collector.
pub struct Telemetry {
    flip_cost: FlipCost,
    started: Instant,
    paused_at: Option<Instant>,
    paused_total_nanos: u128,
    #[cfg(test)]
    simulated_wall: Option<u64>,
    #[cfg(test)]
    simulated_paused_at: Option<u64>,
    last_wall_nanos: u64,
    last_clock_nanos: u64,
    last_executed: u64,
    last_pace_sleep_nanos: u64,
    samples: Vec<FlipSample>,
}

impl Telemetry {
    /// Start collecting telemetry from now for a title with `flip_cost`.
    #[must_use]
    pub fn new(flip_cost: FlipCost) -> Self {
        Self {
            flip_cost,
            started: Instant::now(),
            paused_at: None,
            paused_total_nanos: 0,
            #[cfg(test)]
            simulated_wall: None,
            #[cfg(test)]
            simulated_paused_at: None,
            last_wall_nanos: 0,
            last_clock_nanos: 0,
            last_executed: 0,
            last_pace_sleep_nanos: 0,
            samples: Vec::new(),
        }
    }

    /// Pause or resume the telemetry wall clock so host think/pause time is
    /// excluded from per-flip wall deltas and rates.
    pub fn set_paused(&mut self, paused: bool) {
        #[cfg(test)]
        if let Some(wall) = self.simulated_wall {
            if paused {
                if self.simulated_paused_at.is_none() {
                    self.simulated_paused_at = Some(wall);
                }
            } else if let Some(at) = self.simulated_paused_at.take() {
                self.paused_total_nanos += u128::from(wall.saturating_sub(at));
            }
            return;
        }
        if paused {
            if self.paused_at.is_none() {
                self.paused_at = Some(Instant::now());
            }
        } else if let Some(at) = self.paused_at.take() {
            self.paused_total_nanos += at.elapsed().as_nanos();
        }
    }

    /// Whether the telemetry clock is currently paused.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        #[cfg(test)]
        if self.simulated_wall.is_some() {
            return self.simulated_paused_at.is_some();
        }
        self.paused_at.is_some()
    }

    /// Unpaused host wall nanoseconds elapsed since telemetry started.
    #[must_use]
    pub fn unpaused_wall_nanos(&self) -> u64 {
        #[cfg(test)]
        if let Some(wall) = self.simulated_wall {
            let effective = self.simulated_paused_at.unwrap_or(wall);
            let unpaused = u128::from(effective).saturating_sub(self.paused_total_nanos);
            return u64::try_from(unpaused).unwrap_or(u64::MAX);
        }
        let elapsed = self.paused_at.map_or_else(
            || self.started.elapsed().as_nanos(),
            |at| at.saturating_duration_since(self.started).as_nanos(),
        );
        let unpaused = elapsed.saturating_sub(self.paused_total_nanos);
        u64::try_from(unpaused).unwrap_or(u64::MAX)
    }

    /// Record one flip at the current unpaused wall timestamp, given cumulative
    /// host pacing sleep nanoseconds from [`Host::pace_sleep_nanos`] (#349).
    pub fn record_flip(&mut self, meta: &FlipMeta, uniform: bool, pace_sleep_nanos: u64) {
        let now = self.unpaused_wall_nanos();
        self.record_flip_at(meta, now, pace_sleep_nanos, uniform);
    }

    /// Record one flip at an explicit unpaused wall timestamp and cumulative
    /// pacing sleep total (both in nanoseconds).
    pub fn record_flip_at(
        &mut self,
        meta: &FlipMeta,
        wall_nanos: u64,
        pace_sleep_nanos: u64,
        uniform: bool,
    ) {
        let executed = meta.cursor.saturating_sub(meta.instructions_passed);
        let wall_delta_nanos = wall_nanos.saturating_sub(self.last_wall_nanos);
        let pace_sleep_delta_nanos = pace_sleep_nanos.saturating_sub(self.last_pace_sleep_nanos);
        let declared_delta_nanos = meta.clock_nanos.saturating_sub(self.last_clock_nanos);
        let executed_delta = executed.saturating_sub(self.last_executed);
        self.last_wall_nanos = wall_nanos;
        self.last_pace_sleep_nanos = pace_sleep_nanos;
        self.last_clock_nanos = meta.clock_nanos;
        self.last_executed = executed;
        self.samples.push(FlipSample {
            flip: meta.flip,
            clock_nanos: meta.clock_nanos,
            cursor: meta.cursor,
            instructions_passed: meta.instructions_passed,
            wall_nanos,
            wall_delta_nanos,
            pace_sleep_delta_nanos,
            declared_delta_nanos,
            executed_delta,
            uniform,
        });
    }

    /// Re-baseline cumulative delta anchors at the current unpaused wall time
    /// after a snapshot restore or rewind so the next flip does not inject a
    /// phantom `declared_nanos`, `executed_insns`, or pacing-sleep jump (#349).
    pub fn rebaseline(
        &mut self,
        clock_nanos: u64,
        cursor: u64,
        instructions_passed: u64,
        pace_sleep_nanos: u64,
    ) {
        self.last_wall_nanos = self.unpaused_wall_nanos();
        self.last_clock_nanos = clock_nanos;
        self.last_executed = cursor.saturating_sub(instructions_passed);
        self.last_pace_sleep_nanos = pace_sleep_nanos;
    }

    /// All recorded flip samples in presentation order.
    #[must_use]
    pub fn samples(&self) -> &[FlipSample] {
        &self.samples
    }

    /// Compute aggregated metrics over either all flips (`window = None`) or
    /// the half-open flip range `[start, end)`.
    #[must_use]
    pub fn metrics(
        &self,
        window: Option<(u64, u64)>,
        audio_underruns: Option<u64>,
    ) -> WindowMetrics {
        let intended_fps = match self.flip_cost {
            FlipCost::Refresh(hz) => Some(hz.max(1) as f64),
            FlipCost::Free => None,
        };
        let intended_budget_nanos = match self.flip_cost {
            FlipCost::Refresh(hz) => 1_000_000_000 / hz.max(1),
            FlipCost::Free => 0,
        };

        let mut deltas = Vec::new();
        let mut wall_sum = 0u128;
        let mut declared_sum = 0u128;
        let mut executed_sum = 0u64;
        let mut over_budget = 0u64;
        let mut stalls = 0u64;
        let mut uniform = 0u64;

        for s in &self.samples {
            if let Some((start, end)) = window {
                if s.flip < start || s.flip >= end {
                    continue;
                }
            }
            deltas.push(s.wall_delta_nanos);
            wall_sum += u128::from(s.wall_delta_nanos);
            declared_sum += u128::from(s.declared_delta_nanos);
            executed_sum = executed_sum.saturating_add(s.executed_delta);
            let compute_nanos = s.wall_delta_nanos.saturating_sub(s.pace_sleep_delta_nanos);
            let budget = if s.declared_delta_nanos > 0 {
                s.declared_delta_nanos
            } else {
                intended_budget_nanos
            };
            if budget > 0 && compute_nanos > budget {
                over_budget += 1;
            }
            if s.wall_delta_nanos >= STALL_THRESHOLD_NANOS {
                stalls += 1;
            }
            if s.uniform {
                uniform += 1;
            }
        }

        let flips = deltas.len() as u64;
        deltas.sort_unstable();
        let wall_nanos = u64::try_from(wall_sum).unwrap_or(u64::MAX);
        let declared_nanos = u64::try_from(declared_sum).unwrap_or(u64::MAX);
        let wall_s = wall_sum as f64 / 1e9;
        let declared_s = declared_sum as f64 / 1e9;
        let throughput_mips = if wall_s > 0.0 {
            executed_sum as f64 / wall_s / 1e6
        } else {
            0.0
        };
        let pace = if wall_s > 0.0 {
            declared_s / wall_s
        } else {
            0.0
        };
        let host_fps = if wall_s > 0.0 {
            flips as f64 / wall_s
        } else {
            0.0
        };
        let guest_fps = if declared_s > 0.0 {
            flips as f64 / declared_s
        } else {
            0.0
        };
        let over_budget_pct = if flips > 0 {
            over_budget as f64 * 100.0 / flips as f64
        } else {
            0.0
        };
        let max_ms = deltas
            .last()
            .copied()
            .map_or(0.0, |n| n as f64 / 1_000_000.0);

        WindowMetrics {
            flips,
            wall_nanos,
            declared_nanos,
            executed_insns: executed_sum,
            throughput_mips,
            pace,
            host_fps,
            guest_fps,
            intended_fps,
            p50_ms: percentile_ms(&deltas, 50.0),
            p95_ms: percentile_ms(&deltas, 95.0),
            p99_ms: percentile_ms(&deltas, 99.0),
            max_ms,
            over_budget_flips: over_budget,
            over_budget_pct,
            stall_flips: stalls,
            uniform_flips: uniform,
            audio_underruns,
        }
    }

    /// Build the full-run report plus an optional `[start..end)` window slice.
    #[must_use]
    pub fn report(&self, window: Option<(u64, u64)>, audio_underruns: Option<u64>) -> FpsReport {
        let full = self.metrics(None, audio_underruns);
        let window = window.map(|(start, end)| {
            (
                start,
                end,
                self.metrics(Some((start, end)), audio_underruns),
            )
        });
        FpsReport { full, window }
    }
}

/// Composable [`Host`] wrapper that observes flips and frame uniformity
/// strictly outside guest state, forwarding every call identically to `H`.
pub struct TelemetryHost<H> {
    inner: H,
    telemetry: Telemetry,
    last_uniform: bool,
}

impl<H> TelemetryHost<H> {
    #[must_use]
    pub fn new(inner: H, flip_cost: FlipCost) -> Self {
        Self {
            inner,
            telemetry: Telemetry::new(flip_cost),
            last_uniform: false,
        }
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

    #[must_use]
    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }
}

impl<H: Host> TelemetryHost<H> {
    /// Build the run's report, pulling `audio_underruns` from the wrapped host.
    #[must_use]
    pub fn report(&self, window: Option<(u64, u64)>) -> FpsReport {
        self.telemetry.report(window, self.inner.audio_underruns())
    }
}

impl<H: Host> Host for TelemetryHost<H> {
    fn present(&mut self, frame: &Frame<'_>) {
        self.last_uniform = is_uniform_frame(frame);
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

    fn show_pointer(&mut self, show: bool) {
        self.inner.show_pointer(show);
    }

    fn free_mouse(&mut self) {
        self.inner.free_mouse();
    }

    fn put_pointer(&mut self, x: i32, y: i32) {
        self.inner.put_pointer(x, y);
    }

    fn pointer_shape(&mut self, shape: Option<&Cursor>) {
        self.inner.pointer_shape(shape);
    }

    fn closes(&self) -> u32 {
        self.inner.closes()
    }

    fn audio_underruns(&self) -> Option<u64> {
        self.inner.audio_underruns()
    }

    fn pace(&mut self, declared_nanos: u64) -> f64 {
        self.inner.pace(declared_nanos)
    }

    fn is_paced(&self) -> bool {
        self.inner.is_paced()
    }

    fn pace_sleep_nanos(&self) -> u64 {
        self.inner.pace_sleep_nanos()
    }

    fn pump(&mut self) {
        self.inner.pump();
    }

    fn set_paused(&mut self, paused: bool) {
        self.telemetry.set_paused(paused);
        self.inner.set_paused(paused);
    }

    fn observe_flip(&mut self, meta: &FlipMeta) {
        let pace_sleep = self.inner.pace_sleep_nanos();
        self.telemetry
            .record_flip(meta, self.last_uniform, pace_sleep);
        self.inner.observe_flip(meta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use crate::host::headless::Headless;
    use crate::Title;

    const MS: u64 = 1_000_000;

    fn meta(flip: u64, clock_nanos: u64, cursor: u64, instructions_passed: u64) -> FlipMeta {
        FlipMeta {
            flip,
            clock_nanos,
            cursor,
            instructions_passed,
            pointer_shown: true,
            pointer_pos: (0, 0),
            route_active: false,
            route_remaining: 0,
        }
    }

    #[test]
    fn percentile_math_matches_nearest_rank_and_edge_cases() {
        assert_eq!(percentile_ms(&[], 50.0), 0.0);
        assert_eq!(percentile_ms(&[12 * MS], 50.0), 12.0);
        assert_eq!(percentile_ms(&[12 * MS], 99.0), 12.0);

        let hundred: Vec<u64> = (1..=100).map(|i| i * MS).collect();
        assert_eq!(percentile_ms(&hundred, 50.0), 50.0);
        assert_eq!(percentile_ms(&hundred, 95.0), 95.0);
        assert_eq!(percentile_ms(&hundred, 99.0), 99.0);
        assert_eq!(percentile_ms(&hundred, 100.0), 100.0);

        let ten: Vec<u64> = (1..=10).map(|i| i * MS).collect();
        assert_eq!(percentile_ms(&ten, 50.0), 5.0);
        assert_eq!(percentile_ms(&ten, 95.0), 10.0);
        assert_eq!(percentile_ms(&ten, 99.0), 10.0);
    }

    #[test]
    fn metrics_compute_five_speeds_percentiles_budget_stalls_and_windows() {
        let mut tel = Telemetry::new(FlipCost::Refresh(60));
        // 10 flips total:
        // Flips 0..4 (boot): 20 ms declared, 5 ms wall, 2M cursor (1M passed -> 1M executed), uniform.
        // Flips 4..8 (play): 25 ms declared, 10 ms wall, 6M cursor (1M passed -> 5M executed), non-uniform.
        // Flip 8 (slow): 25 ms declared, 30 ms wall (over budget, not stall), 5M executed.
        // Flip 9 (stall): 25 ms declared, 120 ms wall (over budget + stall), 5M executed.
        let mut wall = 0u64;
        let mut clock = 0u64;
        let mut cursor = 0u64;
        let mut passed = 0u64;
        for f in 0..10u64 {
            let (dw, dc, dcur, dpass, uniform) = match f {
                0..=3 => (5 * MS, 20 * MS, 2_000_000, 1_000_000, true),
                4..=7 => (10 * MS, 25 * MS, 6_000_000, 1_000_000, false),
                8 => (30 * MS, 25 * MS, 5_000_000, 0, false),
                _ => (120 * MS, 25 * MS, 5_000_000, 0, false),
            };
            wall += dw;
            clock += dc;
            cursor += dcur;
            passed += dpass;
            tel.record_flip_at(&meta(f, clock, cursor, passed), wall, 0, uniform);
        }

        let report = tel.report(Some((4, 10)), Some(2));
        let full = report.full;
        assert_eq!(full.flips, 10);
        assert_eq!(full.wall_nanos, 210 * MS);
        assert_eq!(full.declared_nanos, 230 * MS);
        assert_eq!(full.executed_insns, 34_000_000);
        assert!((full.throughput_mips - (34.0 / 0.210)).abs() < 1e-6);
        assert!((full.pace - (0.230 / 0.210)).abs() < 1e-6);
        assert!((full.host_fps - (10.0 / 0.210)).abs() < 1e-6);
        assert!((full.guest_fps - (10.0 / 0.230)).abs() < 1e-6);
        assert_eq!(full.intended_fps, Some(60.0));
        // Sorted deltas (ms): [5, 5, 5, 5, 10, 10, 10, 10, 30, 120]
        assert_eq!(full.p50_ms, 10.0);
        assert_eq!(full.p95_ms, 120.0);
        assert_eq!(full.p99_ms, 120.0);
        assert_eq!(full.max_ms, 120.0);
        assert_eq!(full.over_budget_flips, 2);
        assert!((full.over_budget_pct - 20.0).abs() < 1e-9);
        assert_eq!(full.stall_flips, 1);
        assert_eq!(full.uniform_flips, 4);
        assert_eq!(full.audio_underruns, Some(2));

        let (start, end, win) = report.window.expect("window metrics");
        assert_eq!((start, end), (4, 10));
        assert_eq!(win.flips, 6);
        assert_eq!(win.wall_nanos, 190 * MS);
        assert_eq!(win.declared_nanos, 150 * MS);
        assert_eq!(win.executed_insns, 30_000_000);
        assert!((win.throughput_mips - (30.0 / 0.190)).abs() < 1e-6);
        assert!((win.pace - (0.150 / 0.190)).abs() < 1e-6);
        assert_eq!(win.over_budget_flips, 2);
        assert_eq!(win.stall_flips, 1);
        assert_eq!(win.uniform_flips, 0);

        // Out-of-range window yields zeroed metrics without NaN.
        let empty = tel.metrics(Some((100, 200)), None);
        assert_eq!(empty.flips, 0);
        assert_eq!(empty.throughput_mips, 0.0);
        assert_eq!(empty.pace, 0.0);
        assert_eq!(empty.host_fps, 0.0);
        assert_eq!(empty.guest_fps, 0.0);
        assert_eq!(empty.over_budget_pct, 0.0);

        // Formatting covers both full and window lines.
        let lines = report.format_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("fps-report [full] flips=10 "));
        assert!(lines[1].starts_with("fps-report [4..10] flips=6 "));
        assert!(lines[0].contains("underruns=2"));
    }

    #[test]
    fn telemetry_host_excludes_pause_duration_and_forwards_hooks() {
        struct RecordingHost {
            inner: Headless,
            pumps: usize,
            pauses: Vec<bool>,
            observed: Vec<u64>,
        }
        impl Host for RecordingHost {
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
            fn pump(&mut self) {
                self.pumps += 1;
            }
            fn set_paused(&mut self, paused: bool) {
                self.pauses.push(paused);
            }
            fn observe_flip(&mut self, meta: &FlipMeta) {
                self.observed.push(meta.flip);
            }
        }

        let inner = RecordingHost {
            inner: Headless::new(),
            pumps: 0,
            pauses: Vec::new(),
            observed: Vec::new(),
        };
        let mut host = TelemetryHost::new(inner, FlipCost::Refresh(60));
        host.telemetry.simulated_wall = Some(0);

        let solid = vec![0x22u8; 4 * 4 * 4];
        let mut varied = solid.clone();
        varied[12] = 0x99;

        // Flip 0 at 10 ms wall (uniform).
        *host.telemetry.simulated_wall.as_mut().unwrap() = 10 * MS;
        host.present(&Frame {
            width: 4,
            height: 4,
            pitch: 16,
            pixels: &solid,
        });
        host.observe_flip(&meta(0, 16 * MS, 1000, 0));

        // Pause for 5 seconds, pump while paused, then resume.
        host.set_paused(true);
        assert!(host.telemetry().is_paused());
        host.pump();
        *host.telemetry.simulated_wall.as_mut().unwrap() += 5_000 * MS;
        host.set_paused(false);
        assert!(!host.telemetry().is_paused());

        // Flip 1 after 12 ms of active work (non-uniform).
        *host.telemetry.simulated_wall.as_mut().unwrap() += 12 * MS;
        host.present(&Frame {
            width: 4,
            height: 4,
            pitch: 16,
            pixels: &varied,
        });
        host.observe_flip(&meta(1, 32 * MS, 2500, 0));

        let m = host.report(None).full;
        assert_eq!(m.flips, 2);
        assert_eq!(m.wall_nanos, 22 * MS, "5 s pause excluded from wall time");
        assert_eq!(m.max_ms, 12.0);
        assert_eq!(m.stall_flips, 0);
        assert_eq!(m.uniform_flips, 1);
        assert_eq!(host.inner().pumps, 1);
        assert_eq!(host.inner().pauses, [true, false]);
        assert_eq!(host.inner().observed, [0, 1]);
    }

    #[test]
    fn fps_report_is_bit_exact_identical_on_stub_guest() {
        let base_cli = Cli {
            stub: true,
            ..Cli::default()
        };
        let report_cli = Cli {
            stub: true,
            fps_report: true,
            fps_window: Some((0, 1)),
            ..Cli::default()
        };
        let without = crate::testkit::run_headless(&Title::STUB, &base_cli, 100_000);
        let with = crate::testkit::run_headless(&Title::STUB, &report_cli, 100_000);
        without.assert_same_machine(&with);
        assert!(with.finish.flips > 0, "stub presented at least one flip");
        with.assert_logged("fps-report [full]");
        with.assert_logged("fps-report [0..1]");
    }

    #[test]
    fn pace_sleep_is_excluded_from_over_budget_while_compute_overruns_still_count() {
        struct PacedHost {
            inner: Headless,
            pace: crate::host::pace::Pace,
        }
        impl Host for PacedHost {
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
                self.pace.hold(declared_nanos)
            }
            fn is_paced(&self) -> bool {
                true
            }
            fn pace_sleep_nanos(&self) -> u64 {
                self.pace.slept_total_nanos()
            }
        }

        let mut inner = PacedHost {
            inner: Headless::new(),
            pace: crate::host::pace::Pace::new(),
        };
        inner.pace.simulated = Some(0);
        let mut host = TelemetryHost::new(inner, FlipCost::Refresh(60));
        host.telemetry.simulated_wall = Some(0);

        let budget = 16_666_666u64;
        let mut clock = 0u64;
        // 10 flips at steady ~1.0x pace: 2 ms compute, paced via Pace::hold
        // plus +0.5 ms simulated timer overshoot so wall_delta > budget.
        for f in 0..10u64 {
            clock += budget;
            *host.inner_mut().pace.simulated.as_mut().unwrap() += 2 * MS;
            host.pace(clock);
            let wall = host.inner().pace.wall_nanos() + 500_000;
            *host.inner_mut().pace.simulated.as_mut().unwrap() = wall;
            *host.telemetry.simulated_wall.as_mut().unwrap() = wall;
            host.observe_flip(&meta(f, clock, (f + 1) * 1000, 0));
        }

        // Flip 10: genuine compute overrun (20 ms compute, 0 ms sleep).
        clock += budget;
        *host.inner_mut().pace.simulated.as_mut().unwrap() += 20 * MS;
        host.pace(clock);
        let wall = host.inner().pace.wall_nanos();
        *host.telemetry.simulated_wall.as_mut().unwrap() = wall;
        host.observe_flip(&meta(10, clock, 11 * 1000, 0));

        let m = host.report(None).full;
        assert_eq!(m.flips, 11);
        assert!(host.pace_sleep_nanos() > 100 * MS, "Pace::hold slept");
        assert_eq!(
            m.over_budget_flips, 1,
            "only the 20 ms compute flip is over budget"
        );
        assert!((m.over_budget_pct - (100.0 / 11.0)).abs() < 1e-6);
    }

    #[test]
    fn rebaseline_prevents_phantom_declared_and_executed_jump_on_restore() {
        const SEC: u64 = 1_000_000_000;
        let mut tel = Telemetry::new(FlipCost::Refresh(50));
        tel.simulated_wall = Some(0);

        // 2 pre-restore flips: 20 ms declared, 10 ms wall, 2M executed each.
        *tel.simulated_wall.as_mut().unwrap() = 10 * MS;
        tel.record_flip(&meta(0, 20 * MS, 2_000_000, 0), false, 0);
        *tel.simulated_wall.as_mut().unwrap() = 20 * MS;
        tel.record_flip(&meta(1, 40 * MS, 4_000_000, 0), false, 0);

        // Cold snapshot restore jumps declared clock forward by +174.2 s (and
        // cursor by +18B instructions), taking 50 ms of restore wall time, with
        // 15 ms of prior host pacing sleep already accumulated.
        let restored_clock = 174_240 * MS;
        let restored_cursor = 20_000_000_000;
        let restored_passed = 2_000_000_000;
        *tel.simulated_wall.as_mut().unwrap() = 70 * MS;
        tel.rebaseline(restored_clock, restored_cursor, restored_passed, 15 * MS);

        // 2 post-restore flips: +20 ms declared, +10 ms wall, +2M executed each.
        *tel.simulated_wall.as_mut().unwrap() = 80 * MS;
        tel.record_flip(
            &meta(
                5000,
                restored_clock + 20 * MS,
                restored_cursor + 2_000_000,
                restored_passed,
            ),
            false,
            15 * MS,
        );
        *tel.simulated_wall.as_mut().unwrap() = 90 * MS;
        tel.record_flip(
            &meta(
                5001,
                restored_clock + 40 * MS,
                restored_cursor + 4_000_000,
                restored_passed,
            ),
            false,
            15 * MS,
        );

        let m = tel.metrics(None, None);
        assert_eq!(m.flips, 4);
        assert_eq!(
            m.wall_nanos,
            40 * MS,
            "50 ms restore I/O excluded from flip wall time"
        );
        assert_eq!(m.declared_nanos, 80 * MS, "no +174.2 s phantom jump");
        assert_eq!(
            m.executed_insns, 8_000_000,
            "no +18B phantom instruction jump"
        );
        assert!((m.pace - 2.0).abs() < 1e-6);
        assert!((m.guest_fps - 50.0).abs() < 1e-6);
        assert!((m.throughput_mips - 200.0).abs() < 1e-6);

        // Backward rewind to 10.0 s also yields exact single-flip deltas and
        // preserves compute-budget checks against synced pace_sleep_nanos.
        *tel.simulated_wall.as_mut().unwrap() = 100 * MS;
        tel.rebaseline(10 * SEC, 1_000_000_000, 0, 30 * MS);
        *tel.simulated_wall.as_mut().unwrap() = 125 * MS;
        tel.record_flip(
            &meta(300, 10 * SEC + 20 * MS, 1_002_000_000, 0),
            false,
            30 * MS,
        );
        let m2 = tel.metrics(None, None);
        assert_eq!(m2.flips, 5);
        assert_eq!(m2.declared_nanos, 100 * MS);
        assert_eq!(m2.executed_insns, 10_000_000);
        assert_eq!(
            m2.over_budget_flips, 1,
            "25 ms compute flip after rewind must not be masked by pre-rewind sleep"
        );
        assert!((m2.guest_fps - 50.0).abs() < 1e-6);
    }
}
