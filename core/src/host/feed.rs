//! Feeding an audio device the mix, the same on every host.
//!
//! The mix reaches the host a block at a flip, on the declared clock, and a
//! device pulls frames from its own thread in real time. What sits between
//! them is the same whatever the device is: the samples waiting, a start that
//! waits for enough of them, the oldest dropped when a stalled run catches
//! up, and every time the device ran dry counted (#75). It was written for
//! macOS's audio queue and lives here, with no `cfg`, so every backend feeds
//! its device this way and every host's tests hold it (#107).

use crate::sound::BLOCK_FRAMES;
use std::collections::VecDeque;

/// Bytes in one frame: two channels of sixteen-bit samples.
pub const FRAME_BYTES: u32 = 4;

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
    pub samples: VecDeque<i16>,
    pub playing: bool,
    /// How many times the device has asked for a frame and found nothing
    /// waiting, having been playing. Each one is a gap a person hears, and
    /// counting them is how a run says whether the mix is keeping up (#75).
    pub underruns: u64,
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
    use crate::host::pace::Pace;
    use crate::host::stretch::Stretch;
    use crate::sound::{BLOCK_FRAMES, RATE};

    /// Frames in one of the device's buffers: macOS's audio queue's.
    const BUFFER_FRAMES: u32 = 512;

    /// The device against the mix, with no device: one wall minute of a host
    /// holding `pace`, the mix handing over blocks on the declared clock and
    /// the queue pulling buffers in real time. Answers how many times the
    /// device ran dry (#75).
    fn a_minute_at(pace: f64, stretching: bool) -> u64 {
        let mut waiting = Waiting::default();
        let mut stretch = Stretch::default();
        let mut bytes = vec![0u8; BUFFER_FRAMES as usize * 4];
        // The device pulls one buffer at a time; a wall minute is this many.
        let pulls = 60 * RATE as usize / BUFFER_FRAMES as usize;
        // Declared frames owed per device buffer pulled, at this pace.
        let per_pull = f64::from(BUFFER_FRAMES) * pace;
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

    /// A run at a window, simulated in declared and wall time (#217).
    ///
    /// The machine makes the mix on the declared clock, a declared
    /// millisecond at a time, at the speed each stretch of the run gives:
    /// declared seconds it could make a wall second. It flips every `frame`
    /// declared milliseconds. The host holds each block of the mix with
    /// `Pace::hold_mixed` and each flip with `Pace::hold`, on a wall clock of
    /// the test's own, and stretches each block by the pace it is given. The
    /// device pulls a buffer each buffer's worth of wall time. Each stretch is
    /// its declared milliseconds, its speed, and whether it sounds. Answers
    /// how often the device ran dry while a stretch that sounds played, how
    /// many samples were dropped past its cap, and how many sounding blocks
    /// were stretched. A silent stretch that is slow can run the device dry
    /// before its pace is worked out, as it always could, and nobody hears
    /// silence go missing.
    fn a_window_run(stretches: &[(u64, f64, bool)], frame: u64) -> (u64, usize, usize) {
        const MS: u64 = 1_000_000;
        let pull_nanos = u64::from(BUFFER_FRAMES) * 1_000_000_000 / u64::from(RATE);
        let end_of =
            |block: u64| (block + 1) * BLOCK_FRAMES as u64 * 1_000_000_000 / u64::from(RATE);
        let mut pace = Pace::new();
        pace.simulated = Some(0);
        let mut stretch = Stretch::default();
        let mut waiting = Waiting::default();
        let mut bytes = vec![0u8; BUFFER_FRAMES as usize * 4];
        let (mut pulled, mut declared, mut blocks, mut flip) = (0u64, 0u64, 0u64, frame * MS);
        let (mut dry, mut dropped, mut stretched) = (0, 0usize, 0usize);
        let tone: Vec<i16> = (0..BLOCK_FRAMES as i16 * 2).map(|i| i % 1000 + 1).collect();
        let silence = vec![0i16; BLOCK_FRAMES * 2];
        // The device pulls whatever wall time has gone by.
        let mut pull = |pace: &Pace, waiting: &mut Waiting| {
            while pulled + pull_nanos <= pace.wall_nanos() {
                fill_frames(waiting, &mut bytes);
                pulled += pull_nanos;
            }
        };
        for &(millis, speed, sounds) in stretches {
            let (end, dry_before) = (declared + millis * MS, waiting.underruns);
            while declared < end {
                declared += MS;
                if let Some(wall) = &mut pace.simulated {
                    *wall += (MS as f64 / speed) as u64;
                }
                pull(&pace, &mut waiting);
                // Each block of the mix the clock has passed, held and
                // handed over.
                while declared >= end_of(blocks) {
                    let held = pace.hold_mixed();
                    pull(&pace, &mut waiting);
                    if sounds && held < 1.0 {
                        stretched += 1;
                    }
                    let out = stretch.block(if sounds { &tone } else { &silence }, held);
                    let before = waiting.samples.len() + out.len();
                    add_waiting(&mut waiting, &out);
                    dropped += before - waiting.samples.len();
                    blocks += 1;
                }
                if declared >= flip {
                    pace.hold(flip);
                    pull(&pace, &mut waiting);
                    flip += frame * MS;
                }
            }
            if sounds {
                dry += waiting.underruns - dry_before;
            }
        }
        (dry, dropped, stretched)
    }

    /// LEGO Chess's intro at a window (#217): its silent holds, which poll as
    /// they wait and so are run pass by pass, at about 0.6 of real time, each
    /// followed by a movie whose clock spins are passed, at tens of times real
    /// time, its frames 66 ms apart. After each hold the movie's sound is not
    /// stretched by the hold's pace: at most the one block that straddles the
    /// hold's end, handed over before the machine is ahead again, is, where
    /// the hold's pace once stretched up to a second of it. Nothing is cut,
    /// and the device never runs dry.
    #[test]
    fn a_movie_after_a_slow_hold_is_neither_stretched_nor_cut() {
        let intro = [
            (500, 0.6, false),
            (19_000, 40.0, true),
            (2_070, 0.6, false),
            (1_000, 0.6, false),
            (10_000, 40.0, true),
        ];
        let (dry, dropped, stretched) = a_window_run(&intro, 67);
        assert!(stretched <= 2, "{stretched} blocks of the movies stretched");
        assert_eq!(dropped, 0, "nothing is cut");
        assert_eq!(dry, 0, "and the device never runs dry in a movie");
    }

    /// A wait passed between two flips, as Hot Wheels' reveal waits are,
    /// hands its sound over at its own pace, not all at once: nothing is
    /// dropped past the cap (#217).
    #[test]
    fn a_wait_passed_between_flips_hands_its_sound_over_at_its_own_pace() {
        let reveal = [(2_000, 1.5, true), (350, 300.0, true), (2_000, 1.5, true)];
        let (dry, dropped, stretched) = a_window_run(&reveal, 500);
        assert_eq!((dry, dropped, stretched), (0, 0, 0));
    }

    /// A host that cannot keep pace still has the mix stretched, as before.
    #[test]
    fn a_slow_run_is_still_stretched() {
        let (_, _, stretched) = a_window_run(&[(5_000, 0.6, true)], 67);
        assert!(stretched > 50, "{stretched} blocks stretched");
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
