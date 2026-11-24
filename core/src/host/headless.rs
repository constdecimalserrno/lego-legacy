//! The headless host: no window, no device, uncapped.
//!
//! What it presents and mixes is recorded rather than shown, which is what
//! makes it the host every test runs on. Input comes from a route, so a
//! headless run is a function of that route alone.

use super::{sys, Cursor, Frame, Host, HostError, Input, Mapping};

/// One frame this host was asked to present.
pub struct Presented {
    pub flip: u64,
    pub width: u32,
    pub height: u32,
    /// The frame's own bytes, for the first [`KEPT`] flips, any flip the
    /// host was asked to keep, and the latest one; empty for the rest.
    pub pixels: Vec<u8>,
    uniform: bool,
}

/// How many flips from the start of a run keep their pixels.
///
/// A run that plays its movies presents thousands of 640x480 frames, and
/// keeping every one would hold gigabytes for tests that look at a handful.
// ponytail: the first frames, the latest, and the flips a test names with
// `keeping`. A test that wants every frame of a long run wants a digest instead.
pub const KEPT: usize = 16;

impl Presented {
    /// Whether every pixel in the frame is the same. A uniform frame is what a
    /// cleared surface looks like, so first light is the first frame that is
    /// not one.
    #[must_use]
    pub fn is_uniform(&self) -> bool {
        self.uniform
    }
}

/// A host that records instead of showing.
pub struct Headless {
    flips: u64,
    /// Every frame presented, in order.
    pub frames: Vec<Presented>,
    /// Every audio block mixed, in order. Mixing runs whether or not a device
    /// would have taken the samples.
    pub audio_blocks: Vec<Vec<i16>>,
    /// How many flips had been presented when each block was mixed.
    pub audio_flips: Vec<u64>,
    /// Input still to deliver, newest last, each keyed to the flip it belongs
    /// to.
    pending: Vec<(u64, Input)>,
    /// Flips whose pixels are kept however long the run goes on.
    keep: Vec<u64>,
    /// Every pointer shape the host was told of, with how many flips had
    /// been presented when it was (#139).
    pub shapes: Vec<(u64, Option<Cursor>)>,
}

impl Headless {
    #[must_use]
    pub fn new() -> Self {
        Self {
            flips: 0,
            frames: Vec::new(),
            audio_blocks: Vec::new(),
            audio_flips: Vec::new(),
            pending: Vec::new(),
            keep: Vec::new(),
            shapes: Vec::new(),
        }
    }

    /// Keep these flips' pixels, wherever in the run they fall.
    #[must_use]
    pub fn keeping(mut self, flips: &[u64]) -> Self {
        self.keep.extend_from_slice(flips);
        self
    }

    /// A presented frame, by the flip it was presented at.
    #[must_use]
    pub fn frame(&self, flip: u64) -> Option<&Presented> {
        self.frames.iter().find(|f| f.flip == flip)
    }

    /// Queue the input a route asks for. Events are delivered at the flip they
    /// name; one that names a flip the run never reaches stays here, and the
    /// run reports it rather than losing it.
    pub fn queue(&mut self, events: Vec<(u64, Input)>) {
        self.pending.extend(events);
        self.pending.sort_by_key(|(flip, _)| *flip);
    }

    /// Route events that named a flip this run never reached.
    #[must_use]
    pub fn undelivered(&self) -> &[(u64, Input)] {
        &self.pending
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

fn is_uniform_bgra(pixels: &[u8]) -> bool {
    let (px, _) = pixels.as_chunks::<4>();
    let Some(&first) = px.first() else {
        return true;
    };
    let first_u32 = u32::from_ne_bytes(first);
    let quad = [
        first[0], first[1], first[2], first[3], first[0], first[1], first[2], first[3], first[0],
        first[1], first[2], first[3], first[0], first[1], first[2], first[3],
    ];
    let first_u128 = u128::from_ne_bytes(quad);
    let (quads, rem) = pixels.as_chunks::<16>();
    if !quads.iter().all(|&q| u128::from_ne_bytes(q) == first_u128) {
        return false;
    }
    rem.as_chunks::<4>()
        .0
        .iter()
        .all(|&p| u32::from_ne_bytes(p) == first_u32)
}

impl Host for Headless {
    fn present(&mut self, frame: &Frame<'_>) {
        let row_bytes = (frame.width * 4) as usize;
        let byte_len = row_bytes * frame.height as usize;
        // The latest frame keeps its pixels until another replaces it; recycle
        // the evicted buffer directly so long headless runs avoid per-flip
        // megabyte allocations and page faults (#271).
        let mut pixels = if self.frames.len() > KEPT {
            match self.frames.last_mut() {
                Some(previous) if !self.keep.contains(&previous.flip) => {
                    let mut recycled = std::mem::take(&mut previous.pixels);
                    recycled.clear();
                    if recycled.capacity() < byte_len {
                        recycled.reserve(byte_len);
                    }
                    recycled
                }
                _ => Vec::with_capacity(byte_len),
            }
        } else {
            Vec::with_capacity(byte_len)
        };
        if frame.pitch as usize == row_bytes {
            pixels.extend_from_slice(&frame.pixels[..byte_len]);
        } else {
            for row in 0..frame.height {
                let start = (row * frame.pitch) as usize;
                let end = start + row_bytes;
                pixels.extend_from_slice(&frame.pixels[start..end]);
            }
        }
        let uniform = is_uniform_bgra(&pixels);
        self.frames.push(Presented {
            flip: self.flips,
            width: frame.width,
            height: frame.height,
            pixels,
            uniform,
        });
        self.flips += 1;
    }

    fn poll_input(&mut self, flip: u64) -> Vec<Input> {
        // Asked between flips, with the flip before (#215), it has nothing to
        // hand over: that flip's events went at the flip, and a later flip's
        // wait for theirs.
        let take = self.pending.partition_point(|(f, _)| *f <= flip);
        self.pending.drain(..take).map(|(_, e)| e).collect()
    }

    fn audio(&mut self, samples: &[i16]) {
        self.audio_blocks.push(samples.to_vec());
        self.audio_flips.push(self.flips);
    }

    fn pointer_shape(&mut self, shape: Option<&Cursor>) {
        self.shapes.push((self.flips, shape.cloned()));
    }

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        let (ptr, len) = sys::map_anon(len)?;
        // Safety: the mapping is live and this is its only owner.
        Ok(unsafe { Mapping::from_raw(ptr, len) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recycles_evicted_pixel_buffer_and_preserves_kept_flips() {
        let kept_flip = (KEPT + 2) as u64;
        let mut host = Headless::new().keeping(&[kept_flip]);
        let buf = vec![0x42u8; 8 * 8 * 4];
        let frame = Frame {
            width: 8,
            height: 8,
            pitch: 32,
            pixels: &buf,
        };

        for _ in 0..=(KEPT + 4) {
            host.present(&frame);
        }

        // First `KEPT` frames retain their pixels.
        for i in 0..KEPT {
            assert_eq!(host.frames[i].pixels.len(), 256);
        }
        // Evicted frames past `KEPT` (not in `keep` and not latest) are empty.
        assert!(host.frames[KEPT].pixels.is_empty());
        assert!(host.frames[KEPT + 1].pixels.is_empty());
        assert_eq!(host.frames[kept_flip as usize].pixels.len(), 256);
        assert!(host.frames[KEPT + 3].pixels.is_empty());
        assert_eq!(host.frames.last().unwrap().pixels.len(), 256);

        // Next present recycles the latest evicted buffer's allocation directly.
        let prev_ptr = host.frames.last().unwrap().pixels.as_ptr();
        host.present(&frame);
        assert!(host.frames[host.frames.len() - 2].pixels.is_empty());
        assert_eq!(host.frames.last().unwrap().pixels.as_ptr(), prev_ptr);
    }

    #[test]
    fn padded_and_dense_present_match_and_uniformity_scan_handles_every_position() {
        let mut host = Headless::new();
        // Padded 3x2 frame (pitch 16 > width*4 = 12).
        let mut padded = vec![0u8; 32];
        for row in 0..2 {
            for px in 0..3 {
                padded[row * 16 + px * 4..row * 16 + px * 4 + 4].copy_from_slice(&[1, 2, 3, 0xff]);
            }
            // Junk in pitch padding must not affect pixels or uniformity.
            padded[row * 16 + 12..row * 16 + 16].copy_from_slice(&[9, 9, 9, 9]);
        }
        host.present(&Frame {
            width: 3,
            height: 2,
            pitch: 16,
            pixels: &padded,
        });
        assert!(host.frames[0].is_uniform());
        assert_eq!(host.frames[0].pixels.len(), 24);

        // Check `is_uniform_bgra` across lengths 0..=19 pixels and every differing position.
        assert!(is_uniform_bgra(&[]));
        for count in 1..=19usize {
            let mut pixels = Vec::with_capacity(count * 4);
            for _ in 0..count {
                pixels.extend_from_slice(&[0x10, 0x20, 0x30, 0xff]);
            }
            assert!(is_uniform_bgra(&pixels), "count={count}");
            for diff in 0..count {
                if count == 1 {
                    continue;
                }
                let mut mutated = pixels.clone();
                mutated[diff * 4 + (diff % 4)] ^= 1;
                assert!(!is_uniform_bgra(&mutated), "count={count} diff={diff}");
            }
        }
    }
}
