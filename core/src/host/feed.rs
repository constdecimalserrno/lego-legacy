//! The samples waiting for an audio device, on any host that has one.
//!
//! The mix reaches the host a block at a time on the declared clock, and a
//! device pulls a buffer at a time in real time. What waits between the two,
//! and what a device is handed when nothing does, is the same on every host:
//! every audio backend shares it, and the tests reach it on every host (#108).

use crate::sound::BLOCK_FRAMES;
use std::collections::VecDeque;

/// Bytes in one frame: two channels of sixteen-bit samples.
pub const FRAME_BYTES: u32 = 4;

/// Frames in one of the device's buffers, about 23 ms at the mix's rate.
pub const BUFFER_FRAMES: u32 = 512;

/// Buffers the device holds at once.
pub const BUFFERS: usize = 3;

/// Samples kept waiting before the oldest are dropped: eight of the mix's
/// blocks, about 370 ms. The mix reaches the host a flip at a time, and a
/// title playing a movie flips at its frame rate, fifteen a second: a block
/// and a half at once, every 67 ms. Two blocks' room would skip; eight drops
/// only what a run that stalled and caught up has piled up.
pub const MOST_WAITING: usize = 8 * BLOCK_FRAMES * 2;

/// Samples waited for before the device plays, and after it has run dry:
/// three blocks, about 139 ms, which rides out a movie's bursts and a late
/// flip without running dry again straight away.
pub const ENOUGH_TO_PLAY: usize = 3 * BLOCK_FRAMES * 2;

/// The samples waiting to be heard, and whether enough have waited to play.
#[derive(Debug, Default)]
pub struct Waiting {
    pub(crate) samples: VecDeque<i16>,
    pub(crate) playing: bool,
    /// How many times the device has asked for a frame and found nothing
    /// waiting, having been playing. Each one is a gap a person hears, and
    /// counting them is how a run says whether the mix is keeping up (#75).
    pub(crate) underruns: u64,
}

/// Add samples to those waiting, dropping the oldest whole frames past
/// [`MOST_WAITING`], and letting the device play once
/// [`ENOUGH_TO_PLAY`] have waited.
pub fn add_waiting(waiting: &mut Waiting, samples: &[i16]) {
    waiting.samples.extend(samples);
    let over = waiting.samples.len().saturating_sub(MOST_WAITING) & !1;
    waiting.samples.drain(..over);
    if waiting.samples.len() >= ENOUGH_TO_PLAY {
        waiting.playing = true;
    }
}

/// Fill `bytes` with whole frames of what is waiting, so the channels never
/// swap. Until enough has waited, and from when it runs dry until enough has
/// waited again, the device plays silence.
pub fn fill_frames(waiting: &mut Waiting, bytes: &mut [u8]) {
    for frame in bytes.as_chunks_mut::<{ FRAME_BYTES as usize }>().0 {
        let samples = &mut waiting.samples;
        let (left, right) = match (waiting.playing, samples.front(), samples.get(1)) {
            (true, Some(&left), Some(&right)) => {
                samples.drain(..2);
                (left, right)
            }
            _ => {
                if waiting.playing {
                    waiting.underruns += 1;
                }
                waiting.playing = false;
                (0, 0)
            }
        };
        frame[..2].copy_from_slice(&left.to_le_bytes());
        frame[2..].copy_from_slice(&right.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::{add_waiting, fill_frames, Waiting, ENOUGH_TO_PLAY, MOST_WAITING};
    use crate::host::stretch::Stretch;
    use crate::sound::{BLOCK_FRAMES, RATE};

    /// The device against the mix, with no device: one wall minute of a host
    /// holding `pace`, the mix handing over blocks on the declared clock and
    /// the queue pulling buffers in real time. Answers how many times the
    /// device ran dry (#75).
    fn a_minute_at(pace: f64, stretching: bool) -> u64 {
        let mut waiting = Waiting::default();
        let mut stretch = Stretch::default();
        let mut bytes = vec![0u8; super::BUFFER_FRAMES as usize * 4];
        // The device pulls one buffer at a time; a wall minute is this many.
        let pulls = 60 * RATE as usize / super::BUFFER_FRAMES as usize;
        // Declared frames owed per device buffer pulled, at this pace.
        let per_pull = f64::from(super::BUFFER_FRAMES) * pace;
        let mut owed = 0.0;
        let block: Vec<i16> = (0..BLOCK_FRAMES as i16 * 2).map(|i| i % 1000).collect();
        for _ in 0..pulls {
            owed += per_pull;
            while owed >= BLOCK_FRAMES as f64 {
                owed -= BLOCK_FRAMES as f64;
                let handed = if stretching {
                    stretch.block(&block, pace)
                } else {
                    block.clone()
                };
                add_waiting(&mut waiting, &handed);
            }
            fill_frames(&mut waiting, &mut bytes);
        }
        waiting.underruns
    }

    /// A host that cannot keep pace starves the device, and stretching the
    /// mix by the same ratio stops it (#75).
    #[test]
    fn a_slow_host_stops_starving_the_device_once_the_mix_is_stretched() {
        for pace in [0.45, 0.6, 0.8] {
            let torn = a_minute_at(pace, false);
            let stretched = a_minute_at(pace, true);
            assert!(
                torn > 50,
                "pace {pace}: the device should be starving today, and ran dry {torn} times"
            );
            assert_eq!(
                stretched, 0,
                "pace {pace}: a stretched mix never leaves the device short"
            );
        }
    }

    /// And a host that does keep pace is handed exactly what it is today.
    #[test]
    fn a_host_at_full_pace_is_unchanged() {
        assert_eq!(a_minute_at(1.0, false), a_minute_at(1.0, true));
        assert_eq!(a_minute_at(1.0, true), 0);
    }

    #[test]
    fn the_device_waits_for_enough_to_play_and_again_after_running_dry() {
        let mut waiting = Waiting::default();
        let mut bytes = [0xaa; 8];
        // Two frames waiting is not enough to start on.
        add_waiting(&mut waiting, &[1, -1, 2, -2]);
        fill_frames(&mut waiting, &mut bytes);
        assert_eq!(bytes, [0; 8], "silence until enough has waited");
        // Enough, less one sample: still not.
        let more: Vec<i16> = (0..(ENOUGH_TO_PLAY - 5) as i16).collect();
        add_waiting(&mut waiting, &more);
        assert!(!waiting.playing);
        add_waiting(&mut waiting, &[7, 7]);
        assert!(waiting.playing);
        fill_frames(&mut waiting, &mut bytes);
        assert_eq!(
            bytes,
            [1, 0, 0xff, 0xff, 2, 0, 0xfe, 0xff],
            "whole frames, in order"
        );
        // Played dry, it falls silent and waits again, and says it ran dry.
        let mut rest = vec![0u8; ENOUGH_TO_PLAY * 2 + 8];
        fill_frames(&mut waiting, &mut rest);
        assert!(!waiting.playing);
        assert_eq!(waiting.underruns, 1, "running dry is counted once");
        add_waiting(&mut waiting, &[5, 5]);
        fill_frames(&mut waiting, &mut bytes);
        assert_eq!(bytes, [0; 8], "a dry device waits for enough again");
    }

    #[test]
    fn a_run_that_stalls_and_catches_up_drops_its_oldest_frames() {
        let mut waiting = Waiting::default();
        let block: Vec<i16> = (0..2048).collect();
        for _ in 0..(MOST_WAITING / block.len() + 3) {
            add_waiting(&mut waiting, &block);
        }
        assert_eq!(waiting.samples.len(), MOST_WAITING);
        assert_eq!(waiting.samples.back(), Some(&2047), "the newest is kept");
        assert_eq!(
            waiting.samples.front().map(|s| s % 2),
            Some(0),
            "a frame's left"
        );
    }
}
