//! The headless host: no window, no device, uncapped.
//!
//! What it presents and mixes is recorded rather than shown, which is what
//! makes it the host every test runs on. Input comes from a route, so a
//! headless run is a function of that route alone.

use super::{sys, Frame, Host, HostError, Input, Mapping};
use std::time::Instant;

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
// `keep`. A test that wants every frame of a long run wants a digest instead.
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
    started: Instant,
    flips: u64,
    /// Every frame presented, in order.
    pub frames: Vec<Presented>,
    /// Every audio block mixed, in order. Mixing runs whether or not a device
    /// would have taken the samples.
    pub audio_blocks: Vec<Vec<i16>>,
    /// Input still to deliver, newest last, each keyed to the flip it belongs
    /// to.
    pending: Vec<(u64, Input)>,
    /// Flips whose pixels are kept however long the run goes on.
    keep: Vec<u64>,
}

impl Headless {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            flips: 0,
            frames: Vec::new(),
            audio_blocks: Vec::new(),
            pending: Vec::new(),
            keep: Vec::new(),
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

    #[must_use]
    pub fn flips(&self) -> u64 {
        self.flips
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

impl Host for Headless {
    fn present(&mut self, frame: &Frame<'_>) {
        let mut pixels = Vec::with_capacity((frame.width * frame.height * 4) as usize);
        for row in 0..frame.height {
            let start = (row * frame.pitch) as usize;
            let end = start + (frame.width * 4) as usize;
            pixels.extend_from_slice(&frame.pixels[start..end]);
        }
        let uniform = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[..] == pixels[..4]);
        // The latest frame keeps its pixels until another replaces it.
        if self.frames.len() > KEPT {
            if let Some(previous) = self.frames.last_mut() {
                if !self.keep.contains(&previous.flip) {
                    previous.pixels = Vec::new();
                }
            }
        }
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
        let take = self.pending.partition_point(|(f, _)| *f <= flip);
        self.pending.drain(..take).map(|(_, e)| e).collect()
    }

    fn audio(&mut self, samples: &[i16]) {
        self.audio_blocks.push(samples.to_vec());
    }

    fn wall_nanos(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        let (ptr, len) = sys::map_anon(len)?;
        // Safety: the mapping is live and this is its only owner.
        Ok(unsafe { Mapping::from_raw(ptr, len) })
    }

    fn load_library(&mut self, name: &str) -> Result<sys::Library, HostError> {
        sys::Library::open(name)
    }
}
