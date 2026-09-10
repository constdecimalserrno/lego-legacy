//! The scene a title builds through retained mode.
//!
//! Frames are the title's own tree of transforms, and what hangs on them. The
//! COM objects the guest holds are pointers; the tree lives here, keyed by
//! those pointers, so that what the guest reads back from a frame is what it
//! put there.

use std::collections::BTreeMap;

/// A 4x4 matrix in Direct3D's row-vector convention: a point is a row, and it
/// is multiplied on the left.
pub type Matrix = [[f32; 4]; 4];

pub const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// One node of the scene.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The frame this one hangs from, or zero at a root.
    pub parent: u32,
    pub children: Vec<u32>,
    pub visuals: Vec<u32>,
    pub lights: Vec<u32>,
    /// Where this frame is relative to its parent.
    pub local: Matrix,
    pub sort_mode: u32,
    pub zbuffer_mode: u32,
    pub material_mode: u32,
    pub color: u32,
    /// The scene's background colour, when this frame is a scene's root.
    pub background: u32,
    pub fog: Fog,
    pub texture: u32,
    pub material: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Fog {
    pub enabled: bool,
    pub color: u32,
    pub mode: u32,
    pub start: f32,
    pub end: f32,
    pub density: f32,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            parent: 0,
            children: Vec::new(),
            visuals: Vec::new(),
            lights: Vec::new(),
            local: IDENTITY,
            // D3DRMSORT_FROMPARENT, D3DRMZBUFFER_FROMPARENT and
            // D3DRMMATERIAL_FROMMESH: what a new frame is documented to start
            // with.
            sort_mode: 0,
            zbuffer_mode: 0,
            material_mode: 0,
            // D3DRMFRAME's documented default colour: opaque white.
            color: 0xffff_ffff,
            background: 0,
            fog: Fog::default(),
            texture: 0,
            material: 0,
        }
    }
}

/// Every frame the guest has made, by the pointer it holds.
#[derive(Default)]
pub struct Scene {
    pub frames: BTreeMap<u32, Frame>,
}

impl Scene {
    /// Make a frame, hung from `parent` when there is one.
    pub fn add(&mut self, at: u32, parent: u32) {
        self.frames.insert(at, Frame::default());
        if parent != 0 {
            self.attach(parent, at);
        }
    }

    /// Hang `child` from `parent`, taking it off wherever it hung before.
    pub fn attach(&mut self, parent: u32, child: u32) {
        let previous = self.frames.get(&child).map_or(0, |f| f.parent);
        if let Some(old) = self.frames.get_mut(&previous) {
            old.children.retain(|c| *c != child);
        }
        if let Some(frame) = self.frames.get_mut(&child) {
            frame.parent = parent;
        }
        if let Some(frame) = self.frames.get_mut(&parent) {
            if !frame.children.contains(&child) {
                frame.children.push(child);
            }
        }
    }

    pub fn frame(&self, at: u32) -> Option<&Frame> {
        self.frames.get(&at)
    }

    pub fn frame_mut(&mut self, at: u32) -> Option<&mut Frame> {
        self.frames.get_mut(&at)
    }
}
