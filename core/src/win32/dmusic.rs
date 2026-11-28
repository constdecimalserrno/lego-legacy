//! DirectMusic COM interfaces and declared-clock playback engine.
//!
//! Implements the Windows 98 SE / DirectX 6.1–7 DirectMusic COM surface
//! (`IDirectMusic`, `IDirectMusicPort`, `IDirectMusicPerformance`,
//! `IDirectMusicLoader`, `IDirectMusicSegment`, `IDirectMusicStyle`,
//! `IDirectMusicComposer`, `IDirectMusicBand`) driving [`crate::dmusic`] into
//! [`crate::sound::Mixer`] (and its [`crate::synth::Synth`], ADR-0008).
//!
//! Strictly generic, zero title keying, deterministic on the declared clock,
//! and exhaustively snapshotted in `LLSNAP v2`.

use super::{ComState, Ctx};
use crate::com::{
    self, iid_text, method, ok, read_iid, wall, Interface, ADD_REF, E_NOINTERFACE, QUERY_INTERFACE,
    RELEASE, S_OK,
};
use crate::cpu::Stop;
use crate::dmusic::{
    active_millibpm, nanos_to_ticks, parse_band, parse_segment, parse_style, render_measure,
    ticks_to_nanos, Segment, Style, StyleRef, TimeSig, DMUS_NOTIFICATION_SEGABORT,
    DMUS_NOTIFICATION_SEGALMOSTEND, DMUS_NOTIFICATION_SEGEND, DMUS_NOTIFICATION_SEGLOOP,
    DMUS_NOTIFICATION_SEGSTART,
};
use crate::guest::GuestMem;
use crate::snap::{snap_btree_map, snap_option, snap_vec, snap_vec_deque, Snap, Visitor};
use crate::sound::{Mixer, RATE};
use std::collections::{BTreeMap, VecDeque};

const S_FALSE: u32 = 1;
const E_OUTOFMEMORY: u32 = 0x8007_000e;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_POINTER: u32 = 0x8000_4003;
const DMUS_E_LOADER_OBJECTNOTFOUND: u32 = 0x8878_1161;
const DMUS_E_LOADER_FAILEDOPEN: u32 = 0x8878_1162;

/// `DMUS_OBJ_*` valid-data flags for `DMUS_OBJECTDESC`.
const DMUS_OBJ_OBJECT: u32 = 1 << 0;
const DMUS_OBJ_CLASS: u32 = 1 << 1;
const DMUS_OBJ_NAME: u32 = 1 << 2;
const DMUS_OBJ_FILENAME: u32 = 1 << 4;
const DMUS_OBJ_FULLPATH: u32 = 1 << 5;
const DMUS_OBJ_MEMORY: u32 = 1 << 10;

/// `DMUS_SEGF_MEASURE`: align segment start to the next measure boundary.
const DMUS_SEGF_MEASURE: u32 = 0x0000_2000;

/// `DMUS_PMSGF_REFTIME | DMUS_PMSGF_MUSICTIME` and `DMUS_PMSGT_NOTIFICATION`.
const DMUS_PMSGF_BOTH_TIMES: u32 = 0x0000_0003;
const DMUS_PMSGT_NOTIFICATION: u32 = 6;
/// `DMUS_NOTIFICATION_PMSG` size under MSVC 32-bit `pack(8)` (`dmusici.h`).
const DMUS_NOTIFICATION_PMSG_SIZE: u32 = 0x58;
/// Cap undrained notifications so a silent guest cannot exhaust memory.
const MAX_NOTIFICATIONS: usize = 1024;

/// Well-known DirectMusic GUID strings.
const IID_IUNKNOWN: &str = "00000000-0000-0000-c000-000000000046";
const CLSID_DM_SEGMENT: &str = "d2ac2882-b39b-11d1-8704-00600893b1bd";
const CLSID_DM_STYLE: &str = "d2ac288a-b39b-11d1-8704-00600893b1bd";
const CLSID_DM_BAND: &str = "79ba9e00-b6ee-11d1-86be-00c04fbf8fef";
const GUID_ALL_TYPES: &str = "d2ac2893-b39b-11d1-8704-00600893b1bd";

const IID_DM_SEGMENT: &str = "f96029a2-4282-11d2-8717-00600893b1bd";
const IID_DM_STYLE: &str = "d2ac28bd-b39b-11d1-8704-00600893b1bd";
const IID_DM_BAND: &str = "d2ac28c0-b39b-11d1-8704-00600893b1bd";

const GUID_DOWNLOAD: &str = "d2ac28a7-b39b-11d1-8704-00600893b1bd";
const GUID_UNLOAD: &str = "d2ac28a8-b39b-11d1-8704-00600893b1bd";
const GUID_PERF_MASTER_GROOVE_LEVEL: &str = "d2ac28b2-b39b-11d1-8704-00600893b1bd";
const GUID_PERF_MASTER_VOLUME: &str = "d2ac28b3-b39b-11d1-8704-00600893b1bd";

const GUID_NOTIFICATION_SEGMENT_BYTES: [u8; 16] = [
    0x99, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];
const GUID_NOTIFICATION_MEASUREANDBEAT_BYTES: [u8; 16] = [
    0x9a, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1, 0xbd,
];

/// One file discovered by `IDirectMusicLoader::ScanDirectory`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScannedObject {
    pub class_guid: [u8; 16],
    pub object_guid: [u8; 16],
    pub name: String,
    pub filename: String,
    pub full_path: String,
}

/// One loaded DirectMusic style held by the loader/COM table.
///
/// File-backed styles keep `source_path` and omit `parsed` from `LLSNAP v2`
/// snapshots, rehydrating lazily from `Tree` on cold restore.
#[derive(Clone, Debug, Default)]
pub struct LoadedStyle {
    pub guid: [u8; 16],
    pub name: String,
    pub filename: String,
    pub source_path: Option<String>,
    pub raw_memory: Vec<u8>,
    pub parsed: Option<Style>,
}

/// One queued DirectMusic notification message awaiting `GetNotificationPMsg`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notification {
    pub guid_type: [u8; 16],
    pub option: u32,
    pub field1: u32,
    pub field2: u32,
    pub music_time: u32,
    pub ref_time: u64,
}

/// Active segment playback cursor on the declared clock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayingSegment {
    /// COM identity of the playing `IDirectMusicSegment`.
    pub seg_id: u32,
    /// Declared nanosecond timestamp of the current anchor point.
    pub anchor_nanos: u64,
    /// Music tick at `anchor_nanos`.
    pub anchor_tick: u32,
    /// Active tempo in millibpm from `anchor_nanos`.
    pub millibpm: u32,
    /// Next measure start tick to expand into MIDI events.
    pub next_measure_tick: u32,
    /// Next beat boundary tick for `GUID_NOTIFICATION_MEASUREANDBEAT`.
    pub next_beat_tick: u32,
    /// Cumulative measure index since this segment started.
    pub measure_index: u32,
    /// Beat index within the current measure (`0..beats_per_measure`).
    pub beat_index: u32,
    /// Remaining loop repeats (`u32::MAX` = infinite).
    pub repeats_left: u32,
    /// Whether `DMUS_NOTIFICATION_SEGALMOSTEND` has already been sent for this playthrough.
    pub almost_end_sent: bool,
}

/// Full machine DirectMusic runtime state.
#[derive(Default)]
pub struct DirectMusic {
    /// Search directories keyed by lowercase GUID text (or `GUID_ALL_TYPES`).
    pub search_dirs: BTreeMap<String, String>,
    /// Scanned directory entries per class GUID text (`ScanDirectory` / `EnumObject`).
    pub scanned: BTreeMap<String, Vec<ScannedObject>>,
    /// Loaded segments keyed by their COM identity.
    pub segments: BTreeMap<u32, Segment>,
    /// Loaded styles keyed by their COM identity.
    pub styles: BTreeMap<u32, LoadedStyle>,
    /// Win32 event handle registered via `SetNotificationHandle`.
    pub notify_event: u32,
    /// Enabled notification types.
    pub notify_segment: bool,
    pub notify_measure_beat: bool,
    /// Queued notifications waiting for `GetNotificationPMsg`.
    pub notifications: VecDeque<Notification>,
    /// Guest DirectSound buffer identity bound via `IDirectMusicPort::SetDirectSound`.
    pub bound_dsound_buffer: u32,
    /// Master groove level offset (`GUID_PerfMasterGrooveLevel`, signed `i8`).
    pub master_groove_offset: i8,
    /// Active per-channel band transposition in semitones (`DMUS_IO_INST_TRANSPOSE`).
    pub channel_transpose: [i16; 16],
    /// Currently playing primary segment.
    pub playing: Option<PlayingSegment>,
    /// Segment queued with `DMUS_SEGF_MEASURE` to start at the next measure boundary.
    pub pending_segment: Option<u32>,
    /// Pending timestamped MIDI short messages `(fire_nanos, packed_short_msg)` sorted ascending.
    pub midi_queue: VecDeque<(u64, u32)>,
    /// Currently sounding MIDI notes per channel (bit `k` = key `k`), for clean note-off on Stop.
    pub sounding: [u128; 16],
}

impl DirectMusic {
    /// True while a segment is playing or scheduled MIDI events remain to be delivered.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.playing.is_some() || !self.midi_queue.is_empty()
    }

    /// True if `dsound_buf_identity` is the DirectSound buffer bound to a DirectMusic port.
    #[must_use]
    pub fn is_bound_dsound_buffer(&self, dsound_buf_identity: u32) -> bool {
        self.bound_dsound_buffer != 0 && self.bound_dsound_buffer == dsound_buf_identity
    }

    /// Rehydrate parsed [`Style`] bodies from `Tree` (or embedded memory bytes)
    /// after cold `LLSNAP v2` restore.
    ///
    /// # Errors
    /// Returns a corruption description if a style's backing data is missing or invalid.
    pub fn rehydrate_styles<F>(&mut self, mut read_file: F) -> Result<(), &'static str>
    where
        F: FnMut(&str) -> Option<Vec<u8>>,
    {
        for entry in self.styles.values_mut() {
            if entry.parsed.is_some() {
                continue;
            }
            let bytes = if let Some(path) = &entry.source_path {
                read_file(path).ok_or("dmusic style missing from tree")?
            } else if !entry.raw_memory.is_empty() {
                entry.raw_memory.clone()
            } else {
                return Err("dmusic style missing backing bytes");
            };
            let style = parse_style(&bytes).map_err(|_| "dmusic style rehydrate failed")?;
            entry.parsed = Some(style);
        }
        Ok(())
    }

    /// Advance DirectMusic sequencing up to `now_nanos`, delivering all due MIDI
    /// messages into `mixer` and returning the notification event handle to signal
    /// if any new notification was enqueued.
    pub fn advance(&mut self, now_nanos: u64, mixer: &mut Mixer) -> Option<u32> {
        let prev = self.notifications.len();
        self.step_sequencer(now_nanos);
        self.drain_midi(now_nanos, mixer);
        self.notify_if_queued(prev)
    }

    /// Queue a segment for playback (immediately if idle or non-measure-aligned,
    /// otherwise at the next measure boundary). Returns event handle to signal if any.
    pub fn play_segment(
        &mut self,
        seg_id: u32,
        flags: u32,
        now_nanos: u64,
        mixer: &mut Mixer,
    ) -> Option<u32> {
        if !self.segments.contains_key(&seg_id) {
            return None;
        }
        let prev = self.notifications.len();
        if (flags & DMUS_SEGF_MEASURE) != 0 && self.playing.is_some() {
            self.pending_segment = Some(seg_id);
        } else {
            if self.playing.is_some() {
                self.midi_queue.clear();
                self.silence_and_reset(now_nanos, mixer);
                self.push_segment_notif(DMUS_NOTIFICATION_SEGABORT, 0, now_nanos);
            }
            self.pending_segment = None;
            self.start_segment_now(seg_id, now_nanos);
            self.step_sequencer(now_nanos);
            self.drain_midi(now_nanos, mixer);
        }
        self.notify_if_queued(prev)
    }

    /// Stop active playback (or a specific segment when `filter_seg` is `Some`)
    /// and silence all sounding notes immediately.
    pub fn stop_playback(
        &mut self,
        filter_seg: Option<u32>,
        now_nanos: u64,
        mixer: &mut Mixer,
    ) -> Option<u32> {
        let prev = self.notifications.len();
        if let Some(target) = filter_seg {
            if self.pending_segment == Some(target) {
                self.pending_segment = None;
            }
            if self.playing.as_ref().is_none_or(|p| p.seg_id != target) {
                return None;
            }
        } else {
            self.pending_segment = None;
        }
        let was_playing = self.playing.take().is_some();
        self.midi_queue.clear();
        self.silence_and_reset(now_nanos, mixer);
        if was_playing {
            self.push_segment_notif(DMUS_NOTIFICATION_SEGABORT, 0, now_nanos);
        }
        self.notify_if_queued(prev)
    }

    fn notify_if_queued(&self, prev: usize) -> Option<u32> {
        (self.notifications.len() > prev && self.notify_event != 0).then_some(self.notify_event)
    }

    fn start_segment_now(&mut self, seg_id: u32, now_nanos: u64) {
        let Some(seg) = self.segments.get(&seg_id) else {
            return;
        };
        let start_tick = seg.play_start;
        let millibpm = active_millibpm(seg, start_tick, self.active_style(seg, start_tick));
        let repeats = seg.repeats;
        self.channel_transpose = [0; 16];
        self.playing = Some(PlayingSegment {
            seg_id,
            anchor_nanos: now_nanos,
            anchor_tick: start_tick,
            millibpm,
            next_measure_tick: start_tick,
            next_beat_tick: start_tick,
            measure_index: 0,
            beat_index: 0,
            repeats_left: repeats,
            almost_end_sent: false,
        });
        self.push_segment_notif(DMUS_NOTIFICATION_SEGSTART, start_tick, now_nanos);
    }

    fn step_sequencer(&mut self, now_nanos: u64) {
        for _ in 0..256 {
            let Some(cur) = self.playing.clone() else {
                break;
            };
            let Some(seg) = self.segments.get(&cur.seg_id).cloned() else {
                self.playing = None;
                break;
            };
            let time_sig = self
                .active_style(&seg, cur.next_measure_tick)
                .map_or_else(TimeSig::default, |st| st.time_sig);
            let beat_ticks = (time_sig.beat_ticks() as u32).max(1);
            let measure_ticks = (time_sig.measure_ticks() as u32).max(beat_ticks);
            let beats_per_measure = u32::from(time_sig.beats_per_measure.max(1));
            let seg_len = seg.length.max(measure_ticks);

            let elapsed_ns = now_nanos.saturating_sub(cur.anchor_nanos);
            let cur_tick = cur.anchor_tick.saturating_add(
                nanos_to_ticks(elapsed_ns, cur.millibpm).min(u64::from(u32::MAX)) as u32,
            );

            // Process measure boundary (segment switch / loop / end / measure expansion)
            // before emitting a beat at or past the same tick so old-segment beats
            // never bleed across a switch or loop.
            let measure_due = cur.next_measure_tick <= cur_tick;
            let beat_due = cur.next_beat_tick <= cur_tick && cur.next_beat_tick < seg_len;

            if measure_due && (!beat_due || cur.next_measure_tick <= cur.next_beat_tick) {
                let boundary_ns = cur.anchor_nanos.saturating_add(ticks_to_nanos(
                    u64::from(cur.next_measure_tick.saturating_sub(cur.anchor_tick)),
                    cur.millibpm,
                ));

                if let Some(next_id) = self.pending_segment.take() {
                    self.midi_queue.retain(|&(t, _)| t < boundary_ns);
                    self.schedule_channel_reset(boundary_ns);
                    self.push_segment_notif(
                        DMUS_NOTIFICATION_SEGABORT,
                        cur.next_measure_tick,
                        boundary_ns,
                    );
                    self.start_segment_now(next_id, boundary_ns);
                    continue;
                }

                let has_explicit_loop = seg.loop_end > seg.loop_start && seg.loop_end <= seg_len;
                let loop_end = if has_explicit_loop {
                    seg.loop_end
                } else {
                    seg_len
                };
                if cur.next_measure_tick >= loop_end {
                    if cur.repeats_left > 0 {
                        let next_repeats = if cur.repeats_left == u32::MAX {
                            u32::MAX
                        } else {
                            cur.repeats_left - 1
                        };
                        let restart_tick = if has_explicit_loop { seg.loop_start } else { 0 };
                        let next_bpm = active_millibpm(
                            &seg,
                            restart_tick,
                            self.active_style(&seg, restart_tick),
                        );
                        self.push_segment_notif(
                            DMUS_NOTIFICATION_SEGLOOP,
                            restart_tick,
                            boundary_ns,
                        );
                        if let Some(p) = self.playing.as_mut() {
                            p.anchor_nanos = boundary_ns;
                            p.anchor_tick = restart_tick;
                            p.millibpm = next_bpm;
                            p.next_measure_tick = restart_tick;
                            p.next_beat_tick = restart_tick;
                            p.beat_index = 0;
                            p.repeats_left = next_repeats;
                            p.almost_end_sent = false;
                        }
                        continue;
                    }
                    self.schedule_channel_reset(boundary_ns);
                    self.push_segment_notif(
                        DMUS_NOTIFICATION_SEGEND,
                        cur.next_measure_tick,
                        boundary_ns,
                    );
                    self.playing = None;
                    break;
                }

                if !cur.almost_end_sent
                    && cur.repeats_left == 0
                    && cur.next_measure_tick.saturating_add(measure_ticks) >= seg_len
                {
                    self.push_segment_notif(
                        DMUS_NOTIFICATION_SEGALMOSTEND,
                        cur.next_measure_tick,
                        boundary_ns,
                    );
                    if let Some(p) = self.playing.as_mut() {
                        p.almost_end_sent = true;
                    }
                }

                let measure_bpm = active_millibpm(
                    &seg,
                    cur.next_measure_tick,
                    self.active_style(&seg, cur.next_measure_tick),
                );
                let mut transpose = self.channel_transpose;
                let mut raw_events = Vec::new();
                render_measure(
                    &seg,
                    cur.next_measure_tick,
                    measure_ticks,
                    self.master_groove_offset,
                    &mut transpose,
                    |sref| self.find_style(sref),
                    &mut raw_events,
                );
                self.channel_transpose = transpose;
                for (ev_tick, msg) in raw_events {
                    let rel_ticks = u64::from(ev_tick.saturating_sub(cur.next_measure_tick));
                    let fire_ns =
                        boundary_ns.saturating_add(ticks_to_nanos(rel_ticks, measure_bpm));
                    self.push_midi(fire_ns, msg);
                }

                if let Some(p) = self.playing.as_mut() {
                    if p.millibpm != measure_bpm {
                        p.anchor_nanos = boundary_ns;
                        p.anchor_tick = cur.next_measure_tick;
                        p.millibpm = measure_bpm;
                    }
                    p.next_measure_tick = cur.next_measure_tick.saturating_add(measure_ticks);
                }
                continue;
            }

            if beat_due {
                let event_ns = cur.anchor_nanos.saturating_add(ticks_to_nanos(
                    u64::from(cur.next_beat_tick.saturating_sub(cur.anchor_tick)),
                    cur.millibpm,
                ));
                if self.notify_measure_beat {
                    self.push_notif(Notification {
                        guid_type: GUID_NOTIFICATION_MEASUREANDBEAT_BYTES,
                        option: 0,
                        field1: cur.beat_index,
                        field2: cur.measure_index,
                        music_time: cur.next_beat_tick,
                        ref_time: event_ns / 100,
                    });
                }
                if let Some(p) = self.playing.as_mut() {
                    p.next_beat_tick = p.next_beat_tick.saturating_add(beat_ticks);
                    p.beat_index += 1;
                    if p.beat_index >= beats_per_measure {
                        p.beat_index = 0;
                        p.measure_index += 1;
                    }
                }
                continue;
            }

            break;
        }
    }

    fn push_midi(&mut self, fire_nanos: u64, msg: u32) {
        let pos = self.midi_queue.partition_point(|&(t, _)| t <= fire_nanos);
        self.midi_queue.insert(pos, (fire_nanos, msg));
    }

    fn drain_midi(&mut self, now_nanos: u64, mixer: &mut Mixer) {
        while let Some(&(fire_ns, msg)) = self.midi_queue.front() {
            if fire_ns > now_nanos {
                break;
            }
            self.midi_queue.pop_front();
            self.track_sounding(msg);
            let delivered = mixer.midi(fire_ns, msg);
            debug_assert!(
                delivered || !mixer.midi_out(),
                "MIDI message dropped with active synth"
            );
        }
    }

    fn track_sounding(&mut self, msg: u32) {
        let status = (msg & 0xff) as u8;
        let ch = usize::from(status & 0x0f);
        let key = (msg >> 8) & 0x7f;
        let vel = ((msg >> 16) & 0x7f) as u8;
        match status & 0xf0 {
            0x90 if vel > 0 => self.sounding[ch] |= 1u128 << key,
            0x80 | 0x90 => self.sounding[ch] &= !(1u128 << key),
            0xb0 if key == 120 || (123..=127).contains(&key) => self.sounding[ch] = 0,
            _ => {}
        }
    }

    /// Schedule All Notes Off (CC 123) and center pitch bend (8192) on all 16
    /// channels at `at_nanos` so no queued or active note can stick across a
    /// segment boundary or end.
    fn schedule_channel_reset(&mut self, at_nanos: u64) {
        for ch in 0..16u32 {
            self.push_midi(at_nanos, 0xb0 | ch | (123 << 8));
            self.push_midi(at_nanos, 0xe0 | ch | (0x40 << 16));
        }
    }

    fn silence_and_reset(&mut self, at_nanos: u64, mixer: &mut Mixer) {
        let sounding = std::mem::take(&mut self.sounding);
        for (ch, mask) in sounding.into_iter().enumerate() {
            let c = ch as u32;
            if mask != 0 {
                for key in 0..128u32 {
                    if (mask & (1u128 << key)) != 0 {
                        let _ = mixer.midi(at_nanos, 0x80 | c | (key << 8));
                    }
                }
            }
            let _ = mixer.midi(at_nanos, 0xe0 | c | (0x40 << 16));
        }
    }

    fn push_notif(&mut self, notif: Notification) {
        if self.notifications.len() >= MAX_NOTIFICATIONS {
            self.notifications.pop_front();
        }
        self.notifications.push_back(notif);
    }

    fn push_segment_notif(&mut self, option: u32, music_time: u32, now_nanos: u64) {
        if self.notify_segment {
            self.push_notif(Notification {
                guid_type: GUID_NOTIFICATION_SEGMENT_BYTES,
                option,
                field1: 0,
                field2: 0,
                music_time,
                ref_time: now_nanos / 100,
            });
        }
    }

    fn active_style(&self, seg: &Segment, tick: u32) -> Option<&Style> {
        seg.styles
            .iter()
            .rfind(|s| s.time <= tick)
            .and_then(|r| self.find_style(r))
    }

    fn find_style(&self, sref: &StyleRef) -> Option<&Style> {
        let lookup = |pred: &dyn Fn(&LoadedStyle) -> bool| {
            self.styles
                .values()
                .find(|s| pred(s))
                .and_then(|s| s.parsed.as_ref())
        };
        (sref.guid != [0u8; 16])
            .then(|| lookup(&|s| s.guid == sref.guid))
            .flatten()
            .or_else(|| {
                (!sref.file.is_empty())
                    .then(|| lookup(&|s| s.filename.eq_ignore_ascii_case(&sref.file)))
                    .flatten()
            })
            .or_else(|| {
                (!sref.name.is_empty())
                    .then(|| lookup(&|s| s.name.eq_ignore_ascii_case(&sref.name)))
                    .flatten()
            })
    }
}

/// Convert DirectSound `SetVolume` hundredths of a decibel (`-10000..=0`) into
/// [`crate::synth::Synth::volume`] attenuation in `1/65,536` centibels.
#[must_use]
pub fn dsound_volume_to_synth_attenuation(hundredths_db: i32) -> i64 {
    let clamped = hundredths_db.clamp(-10_000, 0);
    (i64::from(-clamped) * 65_536) / 10
}

// ---------------------------------------------------------------------------
// COM Interfaces (exact DirectX 6.1/7 dmusicc.h / dmusici.h vtable layouts)
// ---------------------------------------------------------------------------

pub static DIRECT_MUSIC: Interface = Interface {
    name: "IDirectMusic",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("EnumPort", 2),
        wall("CreateMusicBuffer", 3),
        method("CreatePort", 4, dm_create_port),
        wall("EnumMasterClock", 2),
        wall("GetMasterClock", 2),
        wall("SetMasterClock", 1),
        wall("Activate", 1),
        wall("GetDefaultPort", 1),
        wall("SetDirectSound", 2),
    ],
};

pub static DIRECT_MUSIC_PORT: Interface = Interface {
    name: "IDirectMusicPort",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("PlayBuffer", 1),
        wall("SetReadNotificationHandle", 1),
        wall("Read", 1),
        wall("DownloadInstrument", 4),
        wall("UnloadInstrument", 1),
        wall("GetLatencyClock", 1),
        wall("GetRunningStats", 1),
        wall("Compact", 0),
        wall("GetCaps", 1),
        wall("DeviceIoControl", 7),
        wall("SetNumChannelGroups", 1),
        wall("GetNumChannelGroups", 1),
        method("Activate", 1, ok),
        wall("SetChannelPriority", 3),
        wall("GetChannelPriority", 3),
        method("SetDirectSound", 2, port_set_direct_sound),
        method("GetFormat", 3, port_get_format),
    ],
};

pub static DIRECT_MUSIC_PERFORMANCE: Interface = Interface {
    name: "IDirectMusicPerformance",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("Init", 3, perf_init),
        method("PlaySegment", 5, perf_play_segment),
        method("Stop", 4, perf_stop),
        wall("GetSegmentState", 2),
        wall("SetPrepareTime", 1),
        wall("GetPrepareTime", 1),
        wall("SetBumperLength", 1),
        wall("GetBumperLength", 1),
        wall("SendPMsg", 1),
        wall("MusicToReferenceTime", 2),
        wall("ReferenceToMusicTime", 3),
        wall("IsPlaying", 2),
        wall("GetTime", 2),
        wall("AllocPMsg", 2),
        method("FreePMsg", 1, perf_free_pmsg),
        wall("GetGraph", 1),
        wall("SetGraph", 1),
        method("SetNotificationHandle", 3, perf_set_notification_handle),
        method("GetNotificationPMsg", 1, perf_get_notification_pmsg),
        method("AddNotificationType", 1, |ctx, _, args| {
            set_notification_type(ctx, args, true)
        }),
        method("RemoveNotificationType", 1, |ctx, _, args| {
            set_notification_type(ctx, args, false)
        }),
        method("AddPort", 1, ok),
        wall("RemovePort", 1),
        method("AssignPChannelBlock", 3, ok),
        wall("AssignPChannel", 4),
        wall("PChannelInfo", 4),
        wall("DownloadInstrument", 8),
        wall("Invalidate", 2),
        wall("GetParam", 6),
        wall("SetParam", 5),
        wall("GetGlobalParam", 3),
        method("SetGlobalParam", 3, perf_set_global_param),
        wall("GetLatencyTime", 1),
        wall("GetQueueTime", 1),
        wall("AdjustTime", 2),
        method("CloseDown", 0, perf_close_down),
        wall("GetResolvedTime", 4),
        wall("MIDIToMusic", 5),
        wall("MusicToMIDI", 5),
        wall("TimeToRhythm", 6),
        wall("RhythmToTime", 6),
    ],
};

pub static DIRECT_MUSIC_LOADER: Interface = Interface {
    name: "IDirectMusicLoader",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetObject", 3, loader_get_object),
        wall("SetObject", 1),
        method("SetSearchDirectory", 3, loader_set_search_directory),
        method("ScanDirectory", 3, loader_scan_directory),
        wall("CacheObject", 1),
        wall("ReleaseObject", 1),
        method("ClearCache", 1, ok),
        method("EnableCache", 2, ok),
        method("EnumObject", 3, loader_enum_object),
    ],
};

pub static DIRECT_MUSIC_SEGMENT: Interface = Interface {
    name: "IDirectMusicSegment",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetLength", 1, seg_get_length),
        method("SetLength", 1, seg_set_length),
        method("GetRepeats", 1, seg_get_repeats),
        method("SetRepeats", 1, seg_set_repeats),
        wall("GetDefaultResolution", 1),
        wall("SetDefaultResolution", 1),
        wall("GetTrack", 4),
        wall("GetTrackGroup", 2),
        wall("InsertTrack", 2),
        wall("RemoveTrack", 1),
        wall("InitPlay", 3),
        wall("GetGraph", 1),
        wall("SetGraph", 1),
        wall("AddNotificationType", 1),
        wall("RemoveNotificationType", 1),
        wall("GetParam", 6),
        method("SetParam", 5, seg_set_param),
        wall("Clone", 3),
        wall("SetStartPoint", 1),
        wall("GetStartPoint", 1),
        wall("SetLoopPoints", 2),
        wall("GetLoopPoints", 2),
        wall("SetPChannelsUsed", 2),
    ],
};

pub static DIRECT_MUSIC_STYLE: Interface = Interface {
    name: "IDirectMusicStyle",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetBand", 2),
        wall("EnumBand", 2),
        wall("GetDefaultBand", 1),
        wall("EnumMotif", 2),
        wall("GetMotif", 2),
        wall("GetDefaultChordMap", 1),
        wall("EnumChordMap", 2),
        wall("GetChordMap", 2),
        wall("GetTimeSignature", 1),
        wall("GetEmbellishmentLength", 4),
        wall("GetTempo", 1),
    ],
};

pub static DIRECT_MUSIC_COMPOSER: Interface = Interface {
    name: "IDirectMusicComposer",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("ComposeSegmentFromTemplate", 5),
        wall("ComposeSegmentFromShape", 8),
        wall("ComposeTransition", 7),
        wall("AutoTransition", 8),
        wall("ComposeTemplateFromShape", 6),
        wall("ChangeChordMap", 3),
    ],
};

pub static DIRECT_MUSIC_BAND: Interface = Interface {
    name: "IDirectMusicBand",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("CreateSegment", 1),
        wall("Download", 1),
        wall("Unload", 1),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &DIRECT_MUSIC,
    &DIRECT_MUSIC_PORT,
    &DIRECT_MUSIC_PERFORMANCE,
    &DIRECT_MUSIC_LOADER,
    &DIRECT_MUSIC_SEGMENT,
    &DIRECT_MUSIC_STYLE,
    &DIRECT_MUSIC_COMPOSER,
    &DIRECT_MUSIC_BAND,
];

// ---------------------------------------------------------------------------
// IDirectMusic / IDirectMusicPort methods
// ---------------------------------------------------------------------------

fn dm_create_port(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[2];
    if out == 0 {
        return Ok(E_POINTER);
    }
    let port = com::create(ctx, &DIRECT_MUSIC_PORT, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, port).map_err(Stop::Fault)?;
    ctx.log.line("dmusic: created software synthesizer port");
    Ok(S_OK)
}

fn port_set_direct_sound(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let buf = args[1];
    ctx.state.dmusic.bound_dsound_buffer = if buf != 0 {
        ctx.state.com.identity(buf)
    } else {
        0
    };
    Ok(S_OK)
}

fn port_get_format(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    const WFX_SIZE: u32 = 18;
    const BUF_SIZE: u32 = RATE * 4;
    let (wfx_ptr, wfx_sz_ptr, buf_sz_ptr) = (args[0], args[1], args[2]);
    if wfx_sz_ptr == 0 {
        return Ok(E_POINTER);
    }
    let avail = ctx.mem.read_u32(wfx_sz_ptr).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(wfx_sz_ptr, WFX_SIZE)
        .map_err(Stop::Fault)?;
    if buf_sz_ptr != 0 {
        ctx.mem
            .write_u32(buf_sz_ptr, BUF_SIZE)
            .map_err(Stop::Fault)?;
    }
    if wfx_ptr != 0 {
        let mut wfx = [0u8; 18];
        wfx[0..2].copy_from_slice(&1u16.to_le_bytes()); // WAVE_FORMAT_PCM
        wfx[2..4].copy_from_slice(&2u16.to_le_bytes()); // stereo
        wfx[4..8].copy_from_slice(&RATE.to_le_bytes());
        wfx[8..12].copy_from_slice(&(RATE * 4).to_le_bytes());
        wfx[12..14].copy_from_slice(&4u16.to_le_bytes());
        wfx[14..16].copy_from_slice(&16u16.to_le_bytes());
        wfx[16..18].copy_from_slice(&0u16.to_le_bytes());
        let to_write = usize::try_from(avail.min(WFX_SIZE)).unwrap_or(0);
        if to_write > 0 {
            ctx.mem
                .write(wfx_ptr, &wfx[..to_write])
                .map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

// ---------------------------------------------------------------------------
// IDirectMusicPerformance methods
// ---------------------------------------------------------------------------

fn perf_init(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let pp_dm = args[0];
    if pp_dm != 0 {
        let existing = ctx.mem.read_u32(pp_dm).map_err(Stop::Fault)?;
        if existing == 0 {
            let dm = com::create(ctx, &DIRECT_MUSIC, ComState::None).map_err(Stop::Fault)?;
            ctx.mem.write_u32(pp_dm, dm).map_err(Stop::Fault)?;
        }
    }
    ctx.log.line("dmusic: performance initialized");
    Ok(S_OK)
}

fn perf_play_segment(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (seg_ptr, flags, out_state) = (args[0], args[1], args[4]);
    if out_state != 0 {
        return Err(Stop::NotWritten {
            what: "IDirectMusicPerformance::PlaySegment: non-null ppSegmentState".to_owned(),
            caller: 0,
        });
    }
    if seg_ptr == 0 {
        return Ok(E_POINTER);
    }
    let seg_id = ctx.state.com.identity(seg_ptr);
    let now_nanos = ctx.declared_millis().saturating_mul(1_000_000);
    let seg_name = ctx
        .state
        .dmusic
        .segments
        .get(&seg_id)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let event = ctx
        .state
        .dmusic
        .play_segment(seg_id, flags, now_nanos, &mut ctx.state.mixer);
    if let Some(h) = event {
        let _ = super::kernel32::signal_event(ctx.state, h);
    }
    ctx.log.line(&format!(
        "dmusic: PlaySegment {seg_name:?} flags={flags:#x}"
    ));
    Ok(S_OK)
}

fn perf_stop(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Stop(pSegment, pSegmentState, mtTime, dwFlags)
    let filter_seg = (args[0] != 0).then(|| ctx.state.com.identity(args[0]));
    let now_nanos = ctx.declared_millis().saturating_mul(1_000_000);
    let event = ctx
        .state
        .dmusic
        .stop_playback(filter_seg, now_nanos, &mut ctx.state.mixer);
    if let Some(h) = event {
        let _ = super::kernel32::signal_event(ctx.state, h);
    }
    Ok(S_OK)
}

fn perf_free_pmsg(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        ctx.state.heap.free(args[0]);
    }
    Ok(S_OK)
}

fn perf_set_notification_handle(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let handle = args[0];
    ctx.state.dmusic.notify_event = handle;
    if handle != 0 && !ctx.state.dmusic.notifications.is_empty() {
        let _ = super::kernel32::signal_event(ctx.state, handle);
    }
    Ok(S_OK)
}

fn perf_get_notification_pmsg(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[0];
    if out == 0 {
        return Ok(E_POINTER);
    }
    let Some(notif) = ctx.state.dmusic.notifications.pop_front() else {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
        return Ok(S_FALSE);
    };
    let at = ctx.state.heap.alloc(DMUS_NOTIFICATION_PMSG_SIZE);
    if at == 0 {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
        return Ok(E_OUTOFMEMORY);
    }
    // 32-bit x86 MSVC pack(8) layout of DMUS_NOTIFICATION_PMSG (dmusici.h):
    // +0x00 dwSize, +0x04 pad, +0x08 rtTime(8), +0x10 mtTime(4), +0x14 dwFlags(4),
    // +0x28 dwType(4), +0x38 guidNotificationType(16), +0x48 dwNotificationOption(4),
    // +0x4c dwField1(4), +0x50 dwField2(4).
    let mut buf = [0u8; DMUS_NOTIFICATION_PMSG_SIZE as usize];
    buf[0x00..0x04].copy_from_slice(&DMUS_NOTIFICATION_PMSG_SIZE.to_le_bytes());
    buf[0x08..0x10].copy_from_slice(&notif.ref_time.to_le_bytes());
    buf[0x10..0x14].copy_from_slice(&notif.music_time.to_le_bytes());
    buf[0x14..0x18].copy_from_slice(&DMUS_PMSGF_BOTH_TIMES.to_le_bytes());
    buf[0x28..0x2c].copy_from_slice(&DMUS_PMSGT_NOTIFICATION.to_le_bytes());
    buf[0x38..0x48].copy_from_slice(&notif.guid_type);
    buf[0x48..0x4c].copy_from_slice(&notif.option.to_le_bytes());
    buf[0x4c..0x50].copy_from_slice(&notif.field1.to_le_bytes());
    buf[0x50..0x54].copy_from_slice(&notif.field2.to_le_bytes());
    ctx.mem.write(at, &buf).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, at).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_notification_type(ctx: &mut Ctx<'_>, args: &[u32], enabled: bool) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let guid = read_iid(ctx, args[0])?;
    if guid == GUID_NOTIFICATION_SEGMENT_BYTES {
        ctx.state.dmusic.notify_segment = enabled;
        Ok(S_OK)
    } else if guid == GUID_NOTIFICATION_MEASUREANDBEAT_BYTES {
        ctx.state.dmusic.notify_measure_beat = enabled;
        Ok(S_OK)
    } else {
        Err(Stop::NotWritten {
            what: format!(
                "IDirectMusicPerformance notification type {}",
                iid_text(&guid)
            ),
            caller: 0,
        })
    }
}

fn perf_set_global_param(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 || args[1] == 0 {
        return Ok(E_POINTER);
    }
    let guid = iid_text(&read_iid(ctx, args[0])?);
    let size = args[2];
    if guid == GUID_PERF_MASTER_GROOVE_LEVEL {
        if size < 1 {
            return Ok(E_INVALIDARG);
        }
        let raw = ctx.mem.read_u8(args[1]).map_err(Stop::Fault)?;
        ctx.state.dmusic.master_groove_offset = raw as i8;
        Ok(S_OK)
    } else if guid == GUID_PERF_MASTER_VOLUME {
        if size < 4 {
            return Ok(E_INVALIDARG);
        }
        let vol = ctx.mem.read_u32(args[1]).map_err(Stop::Fault)? as i32;
        let now_nanos = ctx.declared_millis().saturating_mul(1_000_000);
        ctx.state.mixer.midi_volume(
            now_nanos,
            &*ctx.mem,
            dsound_volume_to_synth_attenuation(vol),
        );
        Ok(S_OK)
    } else {
        Err(Stop::NotWritten {
            what: format!("IDirectMusicPerformance::SetGlobalParam: {guid}"),
            caller: 0,
        })
    }
}

fn perf_close_down(ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let now_nanos = ctx.declared_millis().saturating_mul(1_000_000);
    ctx.state
        .dmusic
        .stop_playback(None, now_nanos, &mut ctx.state.mixer);
    ctx.state.dmusic.notify_event = 0;
    ctx.state.dmusic.notifications.clear();
    Ok(S_OK)
}

// ---------------------------------------------------------------------------
// IDirectMusicLoader methods
// ---------------------------------------------------------------------------

fn search_dir(ctx: &Ctx<'_>, class: &str) -> String {
    ctx.state
        .dmusic
        .search_dirs
        .get(class)
        .or_else(|| ctx.state.dmusic.search_dirs.get(GUID_ALL_TYPES))
        .cloned()
        .unwrap_or_else(|| ctx.state.current_directory.clone())
}

fn loader_set_search_directory(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let class = iid_text(&read_iid(ctx, args[0])?);
    let path = if args[1] != 0 {
        ctx.wide_string(args[1]).map_err(Stop::Fault)?
    } else {
        String::new()
    };
    let abs = if path.is_empty() {
        ctx.state.current_directory.clone()
    } else {
        ctx.absolute(&path)
    };
    ctx.state
        .dmusic
        .search_dirs
        .insert(class.clone(), abs.clone());
    ctx.log
        .line(&format!("dmusic: SetSearchDirectory {class} -> {abs}"));
    Ok(S_OK)
}

fn loader_scan_directory(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 || args[1] == 0 {
        return Ok(E_POINTER);
    }
    let class_bytes = read_iid(ctx, args[0])?;
    let class = iid_text(&class_bytes);
    let ext = ctx.wide_string(args[1]).map_err(Stop::Fault)?;
    let ext_clean = ext.trim_start_matches(['*', '.']);
    let dir = search_dir(ctx, &class);

    let pattern = if ext_clean.is_empty() {
        "*".to_owned()
    } else {
        format!("*.{ext_clean}")
    };
    let matched = ctx.tree.matching(&dir, &pattern);
    let mut added = 0usize;
    let bucket = ctx.state.dmusic.scanned.entry(class.clone()).or_default();
    for (filename, _size) in matched {
        let full_path = join_guest_path(&dir, &filename);
        if bucket
            .iter()
            .any(|e| e.full_path.eq_ignore_ascii_case(&full_path))
        {
            added += 1;
            continue;
        }
        let stem = filename
            .rsplit_once('.')
            .map_or(filename.as_str(), |(s, _)| s)
            .to_owned();
        let (object_guid, name) = match ctx.tree.read(&full_path) {
            Some(bytes) => peek_riff_identity(&bytes).unwrap_or(([0u8; 16], stem)),
            None => ([0u8; 16], stem),
        };
        bucket.push(ScannedObject {
            class_guid: class_bytes,
            object_guid,
            name,
            filename,
            full_path,
        });
        added += 1;
    }
    ctx.log.line(&format!(
        "dmusic: ScanDirectory {class} {pattern} in {dir} -> {added} files"
    ));
    Ok(if added > 0 { S_OK } else { S_FALSE })
}

fn loader_enum_object(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 || args[2] == 0 {
        return Ok(E_POINTER);
    }
    let class = iid_text(&read_iid(ctx, args[0])?);
    let index = args[1] as usize;
    let desc_at = args[2];
    let Some(entry) = ctx
        .state
        .dmusic
        .scanned
        .get(&class)
        .and_then(|v| v.get(index))
        .cloned()
    else {
        return Ok(S_FALSE);
    };

    let mut desc = [0u8; 0x350];
    desc[0x00..0x04].copy_from_slice(&0x350u32.to_le_bytes());
    let mut valid = DMUS_OBJ_CLASS | DMUS_OBJ_NAME | DMUS_OBJ_FILENAME | DMUS_OBJ_FULLPATH;
    if entry.object_guid != [0u8; 16] {
        valid |= DMUS_OBJ_OBJECT;
    }
    desc[0x04..0x08].copy_from_slice(&valid.to_le_bytes());
    desc[0x08..0x18].copy_from_slice(&entry.object_guid);
    desc[0x18..0x28].copy_from_slice(&entry.class_guid);
    write_wide_fixed(&mut desc[0x38..0xb8], &entry.name);
    write_wide_fixed(&mut desc[0x138..0x340], &entry.full_path);
    ctx.mem.write(desc_at, &desc).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn loader_get_object(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (desc_at, riid_at, out_at) = (args[0], args[1], args[2]);
    if desc_at == 0 || riid_at == 0 || out_at == 0 {
        return Ok(E_POINTER);
    }
    ctx.mem.write_u32(out_at, 0).map_err(Stop::Fault)?;

    let valid = ctx.mem.read_u32(desc_at + 0x04).map_err(Stop::Fault)?;
    let obj_guid = if (valid & DMUS_OBJ_OBJECT) != 0 {
        read_iid(ctx, desc_at + 0x08)?
    } else {
        [0u8; 16]
    };
    let class_str = if (valid & DMUS_OBJ_CLASS) != 0 {
        iid_text(&read_iid(ctx, desc_at + 0x18)?)
    } else {
        String::new()
    };
    let riid_str = iid_text(&read_iid(ctx, riid_at)?);

    let desc_name = if (valid & DMUS_OBJ_NAME) != 0 {
        ctx.wide_string(desc_at + 0x38).map_err(Stop::Fault)?
    } else {
        String::new()
    };
    let desc_file = if (valid & DMUS_OBJ_FILENAME) != 0 {
        ctx.wide_string(desc_at + 0x138).map_err(Stop::Fault)?
    } else {
        String::new()
    };

    let (bytes, source_path, filename): (Vec<u8>, Option<String>, String) =
        if (valid & DMUS_OBJ_MEMORY) != 0 {
            let len_lo = ctx.mem.read_u32(desc_at + 0x340).map_err(Stop::Fault)?;
            let len_hi = ctx.mem.read_u32(desc_at + 0x344).map_err(Stop::Fault)?;
            let mem_ptr = ctx.mem.read_u32(desc_at + 0x348).map_err(Stop::Fault)?;
            if mem_ptr == 0 || len_hi != 0 || len_lo == 0 {
                return Ok(E_INVALIDARG);
            }
            let slice = ctx
                .mem
                .slice(mem_ptr, len_lo as usize)
                .map_err(Stop::Fault)?;
            (slice.to_vec(), None, desc_file.clone())
        } else {
            let resolved =
                resolve_loader_file(ctx, &class_str, valid, obj_guid, &desc_name, &desc_file);
            let Some((full_path, fname)) = resolved else {
                return Ok(DMUS_E_LOADER_OBJECTNOTFOUND);
            };
            let Some(data) = ctx.tree.read(&full_path) else {
                return Ok(DMUS_E_LOADER_FAILEDOPEN);
            };
            (data, Some(full_path), fname)
        };

    match class_str.as_str() {
        CLSID_DM_SEGMENT => {
            if riid_str != IID_DM_SEGMENT && riid_str != IID_IUNKNOWN {
                return Ok(E_NOINTERFACE);
            }
            let Ok(mut seg) = parse_segment(&bytes) else {
                return Ok(DMUS_E_LOADER_FAILEDOPEN);
            };
            if seg.name.is_empty() && !desc_name.is_empty() {
                seg.name = desc_name;
            }
            let obj =
                com::create(ctx, &DIRECT_MUSIC_SEGMENT, ComState::None).map_err(Stop::Fault)?;
            ctx.state.dmusic.segments.insert(obj, seg);
            ctx.mem.write_u32(out_at, obj).map_err(Stop::Fault)?;
            Ok(S_OK)
        }
        CLSID_DM_STYLE => {
            if riid_str != IID_DM_STYLE && riid_str != IID_IUNKNOWN {
                return Ok(E_NOINTERFACE);
            }
            let Ok(style) = parse_style(&bytes) else {
                return Ok(DMUS_E_LOADER_FAILEDOPEN);
            };
            let guid = if style.guid != [0u8; 16] {
                style.guid
            } else {
                obj_guid
            };
            let name = if !style.name.is_empty() {
                style.name.clone()
            } else {
                desc_name
            };
            let obj = com::create(ctx, &DIRECT_MUSIC_STYLE, ComState::None).map_err(Stop::Fault)?;
            let raw_memory = if source_path.is_none() {
                bytes
            } else {
                Vec::new()
            };
            ctx.state.dmusic.styles.insert(
                obj,
                LoadedStyle {
                    guid,
                    name,
                    filename,
                    source_path,
                    raw_memory,
                    parsed: Some(style),
                },
            );
            ctx.mem.write_u32(out_at, obj).map_err(Stop::Fault)?;
            Ok(S_OK)
        }
        CLSID_DM_BAND => {
            if riid_str != IID_DM_BAND && riid_str != IID_IUNKNOWN {
                return Ok(E_NOINTERFACE);
            }
            if parse_band(&bytes).is_err() {
                return Ok(DMUS_E_LOADER_FAILEDOPEN);
            }
            let obj = com::create(ctx, &DIRECT_MUSIC_BAND, ComState::None).map_err(Stop::Fault)?;
            ctx.mem.write_u32(out_at, obj).map_err(Stop::Fault)?;
            Ok(S_OK)
        }
        _ => Ok(DMUS_E_LOADER_OBJECTNOTFOUND),
    }
}

fn resolve_loader_file(
    ctx: &Ctx<'_>,
    class_str: &str,
    valid: u32,
    obj_guid: [u8; 16],
    name: &str,
    file: &str,
) -> Option<(String, String)> {
    if (valid & DMUS_OBJ_FILENAME) != 0 && !file.is_empty() {
        let fname = basename(file).to_owned();
        if (valid & DMUS_OBJ_FULLPATH) != 0 || file.contains(':') {
            return Some((file.to_owned(), fname));
        }
        return Some((join_guest_path(&search_dir(ctx, class_str), file), fname));
    }

    if let Some(list) = ctx.state.dmusic.scanned.get(class_str) {
        if (valid & DMUS_OBJ_OBJECT) != 0 && obj_guid != [0u8; 16] {
            if let Some(e) = list.iter().find(|e| e.object_guid == obj_guid) {
                return Some((e.full_path.clone(), e.filename.clone()));
            }
        }
        if (valid & DMUS_OBJ_NAME) != 0 && !name.is_empty() {
            if let Some(e) = list.iter().find(|e| e.name.eq_ignore_ascii_case(name)) {
                return Some((e.full_path.clone(), e.filename.clone()));
            }
        }
    }
    None
}

fn join_guest_path(dir: &str, file: &str) -> String {
    let d = dir.trim_end_matches(['\\', '/']);
    let f = file.trim_start_matches(['\\', '/']);
    if d.is_empty() {
        f.to_owned()
    } else {
        format!("{d}\\{f}")
    }
}

fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn write_wide_fixed(dst: &mut [u8], s: &str) {
    let max = (dst.len() / 2).saturating_sub(1);
    for (slot, unit) in dst
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .take(max)
        .zip(s.encode_utf16())
    {
        *slot = unit.to_le_bytes();
    }
}

fn peek_riff_identity(bytes: &[u8]) -> Option<([u8; 16], String)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" {
        return None;
    }
    match &bytes[8..12] {
        b"DMST" => parse_style(bytes).ok().map(|s| (s.guid, s.name)),
        b"DMSG" => parse_segment(bytes).ok().map(|s| (s.guid, s.name)),
        b"DMBD" => parse_band(bytes).ok().map(|b| (b.guid, b.name)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// IDirectMusicSegment methods
// ---------------------------------------------------------------------------

fn seg_get_length(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let id = ctx.state.com.identity(this);
    let len = ctx.state.dmusic.segments.get(&id).map_or(0, |s| s.length);
    ctx.mem.write_u32(args[0], len).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn seg_set_length(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let id = ctx.state.com.identity(this);
    if let Some(seg) = ctx.state.dmusic.segments.get_mut(&id) {
        seg.length = args[0];
    }
    Ok(S_OK)
}

fn seg_get_repeats(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let id = ctx.state.com.identity(this);
    let rep = ctx.state.dmusic.segments.get(&id).map_or(0, |s| s.repeats);
    ctx.mem.write_u32(args[0], rep).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn seg_set_repeats(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let id = ctx.state.com.identity(this);
    if let Some(seg) = ctx.state.dmusic.segments.get_mut(&id) {
        seg.repeats = args[0];
    }
    Ok(S_OK)
}

fn seg_set_param(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(E_POINTER);
    }
    let guid = iid_text(&read_iid(ctx, args[0])?);
    if guid == GUID_DOWNLOAD || guid == GUID_UNLOAD {
        // Instruments are already backed by the declared DLS bank in `Mixer::synth`.
        return Ok(S_OK);
    }
    Err(Stop::NotWritten {
        what: format!("IDirectMusicSegment::SetParam: {guid}"),
        caller: 0,
    })
}

// ---------------------------------------------------------------------------
// LLSNAP v2 Snap implementations (exhaustive destructuring)
// ---------------------------------------------------------------------------

impl Snap for ScannedObject {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            class_guid,
            object_guid,
            name,
            filename,
            full_path,
        } = self;
        v.bytes(class_guid);
        v.bytes(object_guid);
        v.string(name);
        v.string(filename);
        v.string(full_path);
    }
}

impl Snap for LoadedStyle {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            guid,
            name,
            filename,
            source_path,
            raw_memory,
            parsed,
        } = self;
        v.bytes(guid);
        v.string(name);
        v.string(filename);
        snap_option(v, source_path, |p, v| v.string(p));
        v.byte_vec(raw_memory);
        if v.is_reading() {
            *parsed = None;
        }
    }
}

impl Snap for Notification {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            guid_type,
            option,
            field1,
            field2,
            music_time,
            ref_time,
        } = self;
        v.bytes(guid_type);
        v.u32(option);
        v.u32(field1);
        v.u32(field2);
        v.u32(music_time);
        v.u64(ref_time);
    }
}

impl Snap for PlayingSegment {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            seg_id,
            anchor_nanos,
            anchor_tick,
            millibpm,
            next_measure_tick,
            next_beat_tick,
            measure_index,
            beat_index,
            repeats_left,
            almost_end_sent,
        } = self;
        v.u32(seg_id);
        v.u64(anchor_nanos);
        v.u32(anchor_tick);
        v.u32(millibpm);
        v.u32(next_measure_tick);
        v.u32(next_beat_tick);
        v.u32(measure_index);
        v.u32(beat_index);
        v.u32(repeats_left);
        v.bool(almost_end_sent);
    }
}

impl Snap for DirectMusic {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            search_dirs,
            scanned,
            segments,
            styles,
            notify_event,
            notify_segment,
            notify_measure_beat,
            notifications,
            bound_dsound_buffer,
            master_groove_offset,
            channel_transpose,
            playing,
            pending_segment,
            midi_queue,
            sounding,
        } = self;
        snap_btree_map(v, search_dirs, |k, v| v.string(k), |val, v| v.string(val));
        snap_btree_map(
            v,
            scanned,
            |k, v| v.string(k),
            |list, v| snap_vec(v, list, |item, v| item.snap(v)),
        );
        snap_btree_map(v, segments, |k, v| v.u32(k), |s, v| s.snap(v));
        snap_btree_map(v, styles, |k, v| v.u32(k), |s, v| s.snap(v));
        v.u32(notify_event);
        v.bool(notify_segment);
        v.bool(notify_measure_beat);
        snap_vec_deque(v, notifications, |n, v| n.snap(v));
        v.u32(bound_dsound_buffer);
        let mut groove_u8 = *master_groove_offset as u8;
        v.u8(&mut groove_u8);
        if v.is_reading() {
            *master_groove_offset = groove_u8 as i8;
        }
        for t in channel_transpose {
            v.i16(t);
        }
        snap_option(v, playing, |p, v| p.snap(v));
        snap_option(v, pending_segment, |id, v| v.u32(id));
        snap_vec_deque(v, midi_queue, |(t, m), v| {
            v.u64(t);
            v.u32(m);
        });
        for s in sounding {
            v.u128(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dls::{tiny, Bank};
    use crate::dmusic::{self, Band, BandEvent, BandInstrument, StyleRef, DMUS_IO_INST_PATCH};
    use crate::guest::Arena;
    use crate::host::headless::Headless;
    use crate::snap::{Reader, Writer};
    use crate::synth::Synth;
    use crate::win32::tests::with_test_ctx;

    const CLSID_PERF_BYTES: [u8; 16] = [
        0x81, 0x28, 0xac, 0xd2, 0x9b, 0xb3, 0xd1, 0x11, 0x87, 0x04, 0x00, 0x60, 0x08, 0x93, 0xb1,
        0xbd,
    ];
    const IID_PERF_BYTES: [u8; 16] = [
        0x03, 0x3d, 0xd4, 0x07, 0x23, 0x65, 0xd2, 0x11, 0x87, 0x1d, 0x00, 0x60, 0x08, 0x93, 0xb1,
        0xbd,
    ];

    #[test]
    fn dsound_volume_maps_linearly_to_synth_centibel_attenuation() {
        assert_eq!(dsound_volume_to_synth_attenuation(0), 0);
        assert_eq!(dsound_volume_to_synth_attenuation(-100), 10 << 16);
        assert_eq!(dsound_volume_to_synth_attenuation(-10_000), 1000 << 16);
        assert_eq!(dsound_volume_to_synth_attenuation(500), 0);
    }

    #[test]
    fn com_lifecycle_regdb_fallback_pmsg_pack8_and_dsound_volume_propagation() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let clsid_at = 0x1000u32;
            let iid_at = 0x1020u32;
            let out_at = 0x1040u32;
            ctx.mem.write(clsid_at, &CLSID_PERF_BYTES).unwrap();
            ctx.mem.write(iid_at, &IID_PERF_BYTES).unwrap();
            let co_create = crate::win32::find("ole32", "CoCreateInstance").unwrap();

            // 1. Without a loaded DLS bank, CoCreateInstance refuses with REGDB_E_CLASSNOTREG.
            ctx.mem.write_u32(out_at, 0xdead_beef).unwrap();
            let hr = (co_create.run)(ctx, &[clsid_at, 0, 3, iid_at, out_at]).unwrap();
            assert_eq!(hr, 0x8004_0154);
            assert_eq!(ctx.mem.read_u32(out_at).unwrap(), 0);

            // 2. With a DLS bank, CoCreateInstance instantiates IDirectMusicPerformance.
            let bank = Bank::parse(&tiny::bytes()).expect("tiny bank");
            ctx.state.mixer.open_synth(Synth::new(bank));
            let hr = (co_create.run)(ctx, &[clsid_at, 0, 3, iid_at, out_at]).unwrap();
            assert_eq!(hr, S_OK);
            let perf = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!(
                ctx.state.com.interface_of(perf),
                Some("IDirectMusicPerformance")
            );

            // 3. Init -> CreatePort -> GetFormat -> SetDirectSound -> DSound SetVolume propagates.
            let dm_out = 0x1060u32;
            ctx.mem.write_u32(dm_out, 0).unwrap();
            assert_eq!(perf_init(ctx, perf, &[dm_out, 0, 0]).unwrap(), S_OK);
            let dm = ctx.mem.read_u32(dm_out).unwrap();
            let port_out = 0x1080u32;
            assert_eq!(dm_create_port(ctx, dm, &[0, 0, port_out, 0]).unwrap(), S_OK);
            let port = ctx.mem.read_u32(port_out).unwrap();

            let wfx_at = 0x10a0u32;
            let wfx_sz = 0x10c0u32;
            let buf_sz = 0x10c4u32;
            ctx.mem.write_u32(wfx_sz, 18).unwrap();
            assert_eq!(
                port_get_format(ctx, port, &[wfx_at, wfx_sz, buf_sz]).unwrap(),
                S_OK
            );
            assert_eq!(ctx.mem.read_u32(wfx_sz).unwrap(), 18);
            assert_eq!(ctx.mem.read_u32(buf_sz).unwrap(), RATE * 4);

            // Bind a DirectSound buffer and verify SetVolume propagates silence (-10000).
            let ds = crate::win32::dsound::make(ctx).unwrap();
            let dsbd = 0x1100u32;
            let mut desc = [0u8; 20];
            desc[0..4].copy_from_slice(&20u32.to_le_bytes());
            desc[4..8].copy_from_slice(&0x80u32.to_le_bytes()); // DSBCAPS_CTRLVOLUME
            desc[8..12].copy_from_slice(&(RATE * 4).to_le_bytes());
            desc[16..20].copy_from_slice(&wfx_at.to_le_bytes());
            ctx.mem.write(dsbd, &desc).unwrap();
            let dsbuf_out = 0x1120u32;
            let create_buf = crate::win32::com_method("IDirectSound", 3).unwrap();
            assert_eq!(
                (create_buf.run)(ctx, ds, &[dsbd, dsbuf_out, 0]).unwrap(),
                S_OK
            );
            let dsbuf = ctx.mem.read_u32(dsbuf_out).unwrap();
            assert_eq!(
                port_set_direct_sound(ctx, port, &[ds, dsbuf]).unwrap(),
                S_OK
            );
            assert!(ctx
                .state
                .dmusic
                .is_bound_dsound_buffer(ctx.state.com.identity(dsbuf)));

            let set_vol = crate::win32::com_method("IDirectSoundBuffer", 15).unwrap();
            assert_eq!(
                (set_vol.run)(ctx, dsbuf, &[(-10_000i32) as u32]).unwrap(),
                S_OK
            );

            // 4. Verify pack(8) DMUS_NOTIFICATION_PMSG offsets returned by GetNotificationPMsg.
            ctx.state.dmusic.notifications.push_back(Notification {
                guid_type: GUID_NOTIFICATION_SEGMENT_BYTES,
                option: DMUS_NOTIFICATION_SEGALMOSTEND,
                field1: 3,
                field2: 1,
                music_time: 3072,
                ref_time: 0x1122_3344_5566_7788,
            });
            let pmsg_out = 0x1140u32;
            assert_eq!(
                perf_get_notification_pmsg(ctx, perf, &[pmsg_out]).unwrap(),
                S_OK
            );
            let pmsg = ctx.mem.read_u32(pmsg_out).unwrap();
            assert_eq!(ctx.mem.read_u32(pmsg).unwrap(), DMUS_NOTIFICATION_PMSG_SIZE);
            assert_eq!(ctx.mem.read_u32(pmsg + 0x08).unwrap(), 0x5566_7788);
            assert_eq!(ctx.mem.read_u32(pmsg + 0x0c).unwrap(), 0x1122_3344);
            assert_eq!(ctx.mem.read_u32(pmsg + 0x10).unwrap(), 3072);
            assert_eq!(
                ctx.mem.read_u32(pmsg + 0x14).unwrap(),
                DMUS_PMSGF_BOTH_TIMES
            );
            assert_eq!(ctx.mem.read_u32(pmsg + 0x24).unwrap(), 0); // pGraph untouched
            assert_eq!(
                ctx.mem.read_u32(pmsg + 0x28).unwrap(),
                DMUS_PMSGT_NOTIFICATION
            );
            assert_eq!(
                ctx.mem.read_u32(pmsg + 0x48).unwrap(),
                DMUS_NOTIFICATION_SEGALMOSTEND
            );
            assert_eq!(ctx.mem.read_u32(pmsg + 0x4c).unwrap(), 3);
            assert_eq!(ctx.mem.read_u32(pmsg + 0x50).unwrap(), 1);
            assert_eq!(perf_free_pmsg(ctx, perf, &[pmsg]).unwrap(), S_OK);
        });
    }

    #[test]
    fn directmusic_sequencer_feeds_synth_deterministically_and_snaps_cleanly() {
        let bank = Bank::parse(&tiny::bytes()).expect("tiny DLS bank");
        let mut mixer = Mixer::default();
        mixer.open_synth(Synth::new(bank));
        let mem = Arena::new(&mut Headless::new(), 1 << 16).expect("arena");

        let mut dm = DirectMusic {
            notify_segment: true,
            notify_measure_beat: true,
            notify_event: 0x100,
            ..DirectMusic::default()
        };

        let part_guid = [9u8; 16];
        let style_guid = [8u8; 16];
        let style = Style {
            guid: style_guid,
            name: "UnitStyle".to_owned(),
            time_sig: TimeSig {
                beats_per_measure: 4,
                beat: 4,
                grids_per_beat: 4,
            },
            millibpm: 120_000,
            bands: Vec::new(),
            parts: vec![dmusic::StylePart {
                guid: part_guid,
                time_sig: TimeSig {
                    beats_per_measure: 4,
                    beat: 4,
                    grids_per_beat: 4,
                },
                measures: 1,
                play_mode: dmusic::DMUS_PLAYMODE_FIXED,
                notes: vec![dmusic::StyleNote {
                    grid_start: 0,
                    variation: 1,
                    duration: 768,
                    time_offset: 0,
                    music_value: 60,
                    velocity: 127,
                    play_mode: dmusic::DMUS_PLAYMODE_FIXED,
                }],
                curves: Vec::new(),
            }],
            patterns: vec![dmusic::Pattern {
                groove_bottom: 1,
                groove_top: 100,
                embellishment: 0,
                measures: 1,
                part_refs: vec![dmusic::PartRef {
                    part_guid,
                    pchannel: 0,
                    subchord_level: 0,
                }],
            }],
        };
        dm.styles.insert(
            1,
            LoadedStyle {
                guid: style_guid,
                name: "UnitStyle".to_owned(),
                filename: "unit.sty".to_owned(),
                source_path: None,
                raw_memory: vec![1, 2, 3, 4],
                parsed: Some(style),
            },
        );
        dm.segments.insert(
            2,
            Segment {
                name: "UnitSeg".to_owned(),
                length: 3072,
                repeats: 0,
                bands: vec![BandEvent {
                    time: 0,
                    band: Band {
                        guid: [0; 16],
                        name: String::new(),
                        instruments: vec![BandInstrument {
                            patch: 1,
                            pchannel: 0,
                            flags: DMUS_IO_INST_PATCH,
                            pan: 64,
                            volume: 127,
                            transpose: 0,
                        }],
                    },
                }],
                styles: vec![StyleRef {
                    time: 0,
                    guid: style_guid,
                    name: "UnitStyle".to_owned(),
                    file: "unit.sty".to_owned(),
                }],
                ..Segment::default()
            },
        );

        let sig = dm.play_segment(2, 0, 0, &mut mixer);
        assert_eq!(sig, Some(0x100));
        assert!(dm.notifications.iter().any(|n| {
            n.guid_type == GUID_NOTIFICATION_SEGMENT_BYTES && n.option == DMUS_NOTIFICATION_SEGSTART
        }));

        dm.advance(250_000_000, &mut mixer);
        let blocks = mixer.due(250_000_000, &mem);
        assert!(!blocks.is_empty());
        assert!(
            blocks.iter().flatten().any(|&s| s != 0),
            "DirectMusic playback must render audible PCM via Synth"
        );

        // Snap round-trip preserves memory-backed raw_memory, playback cursor, and notifications.
        let mut w = Writer::with_capacity(1024);
        dm.snap(&mut w);
        let mut restored = DirectMusic::default();
        let mut r = Reader::new(&w.out);
        restored.snap(&mut r);
        assert!(!r.failed());
        assert_eq!(restored.styles[&1].raw_memory, vec![1, 2, 3, 4]);
        assert_eq!(restored.playing, dm.playing);
        assert_eq!(restored.notifications, dm.notifications);
        assert_eq!(restored.midi_queue, dm.midi_queue);
        assert_eq!(restored.sounding, dm.sounding);

        // Stop with a non-matching segment filter leaves playback active.
        dm.stop_playback(Some(999), 300_000_000, &mut mixer);
        assert!(dm.playing.is_some());
        dm.stop_playback(Some(2), 300_000_000, &mut mixer);
        assert!(dm.playing.is_none());
    }
}
