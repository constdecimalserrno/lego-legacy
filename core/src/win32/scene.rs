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
            // A frame's own colour starts opaque white.
            color: 0xffff_ffff,
            // A scene's background starts opaque black, as Wine's tests hold
            // native retained mode to.
            background: 0xff00_0000,
            fog: Fog::default(),
            texture: 0,
            material: 0,
        }
    }
}

/// A light, as a title sets it up. What it has not been told is `None`:
/// retained mode's own defaults for those have not been measured, and nothing
/// reads them yet.
#[derive(Clone, Debug, Default)]
pub struct Light {
    pub kind: u32,
    pub colour: u32,
    pub range: Option<f32>,
    pub umbra: Option<f32>,
    pub penumbra: Option<f32>,
    /// Constant, linear and quadratic.
    pub attenuation: [Option<f32>; 3],
    /// The frame whose visuals it lights, when it lights only one's.
    pub enable_frame: u32,
}

/// How a viewport sees its scene: the planes it clips against and how it
/// projects. What retained mode starts a viewport with is what Wine's tests
/// hold native retained mode to.
#[derive(Clone, Debug)]
pub struct View {
    pub front: f32,
    pub back: f32,
    /// Left, right, bottom and top of the front plane.
    pub plane: [f32; 4],
    /// D3DRMPROJECT_PERSPECTIVE, or one of the others.
    pub projection: u32,
    pub uniform_scaling: Option<bool>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            front: 1.0,
            back: 100.0,
            plane: [-0.5, 0.5, -0.5, 0.5],
            projection: 0,
            uniform_scaling: None,
        }
    }
}

/// Every frame the guest has made, by the pointer it holds; every light and
/// every viewport.
#[derive(Default)]
pub struct Scene {
    pub frames: BTreeMap<u32, Frame>,
    pub lights: BTreeMap<u32, Light>,
    pub viewports: BTreeMap<u32, View>,
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

    /// The frame at the top of the tree a frame hangs in: its scene.
    #[must_use]
    pub fn root(&self, mut at: u32) -> Option<u32> {
        let mut steps = 0;
        loop {
            let frame = self.frames.get(&at)?;
            if frame.parent == 0 || steps > self.frames.len() {
                return Some(at);
            }
            at = frame.parent;
            steps += 1;
        }
    }

    /// Where a frame is in its scene: its own transform, then its parent's,
    /// all the way up.
    #[must_use]
    pub fn world(&self, at: u32) -> Option<Matrix> {
        let mut out = IDENTITY;
        let mut next = at;
        for _ in 0..=self.frames.len() {
            if next == 0 {
                return Some(out);
            }
            let frame = self.frames.get(&next)?;
            out = multiply(&out, &frame.local);
            next = frame.parent;
        }
        // A frame that is its own ancestor has no place in any scene.
        None
    }

    pub fn frame_mut(&mut self, at: u32) -> Option<&mut Frame> {
        self.frames.get_mut(&at)
    }
}

/// `a` then `b`, for row vectors: a point goes through `a` first.
#[must_use]
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// A rotation by `theta` radians about an axis, as retained mode builds one.
#[must_use]
pub fn rotation(axis: [f32; 3], theta: f32) -> Matrix {
    // An axis with no length is the x axis, as D3DRMVectorNormalize makes it.
    let [x, y, z] = normalise(axis).unwrap_or([1.0, 0.0, 0.0]);
    // The engine's own trigonometry, never the host's: a run gives the same
    // answer on every machine.
    let (sin, cos) = (
        crate::cpu::x87::maths::sin(f64::from(theta)) as f32,
        crate::cpu::x87::maths::cos(f64::from(theta)) as f32,
    );
    let vers = 1.0 - cos;
    [
        [
            vers * x * x + cos,
            vers * y * x + sin * z,
            vers * z * x - sin * y,
            0.0,
        ],
        [
            vers * x * y - sin * z,
            vers * y * y + cos,
            vers * z * y + sin * x,
            0.0,
        ],
        [
            vers * x * z + sin * y,
            vers * y * z - sin * x,
            vers * z * z + cos,
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

#[must_use]
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// A vector scaled to length one, or `None` for one with no length.
#[must_use]
pub fn normalise(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 0.0).then(|| v.map(|c| c / length))
}

/// A direction through a transform: turned and scaled, never moved.
#[must_use]
pub fn direct(m: &Matrix, [x, y, z]: [f32; 3]) -> [f32; 3] {
    [
        x * m[0][0] + y * m[1][0] + z * m[2][0],
        x * m[0][1] + y * m[1][1] + z * m[2][1],
        x * m[0][2] + y * m[1][2] + z * m[2][2],
    ]
}

/// A point through a transform.
#[must_use]
pub fn transform(m: &Matrix, [x, y, z]: [f32; 3]) -> [f32; 3] {
    [
        x * m[0][0] + y * m[1][0] + z * m[2][0] + m[3][0],
        x * m[0][1] + y * m[1][1] + z * m[2][1] + m[3][1],
        x * m[0][2] + y * m[1][2] + z * m[2][2] + m[3][2],
    ]
}

/// The transform that undoes an affine one: the inverse of its linear part,
/// and the translation taken back through that.
///
/// Answers `None` for a transform that flattens space, which has no inverse.
#[must_use]
pub fn invert(m: &Matrix) -> Option<Matrix> {
    let a = |r: usize, c: usize| f64::from(m[r][c]);
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1))
        - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    if det.abs() < 1e-12 {
        return None;
    }
    let cofactor = |r: usize, c: usize| {
        let (r0, r1) = ([1, 0, 0][r], [2, 2, 1][r]);
        let (c0, c1) = ([1, 0, 0][c], [2, 2, 1][c]);
        let minor = a(r0, c0) * a(r1, c1) - a(r0, c1) * a(r1, c0);
        if (r + c).is_multiple_of(2) {
            minor
        } else {
            -minor
        }
    };
    let mut out = IDENTITY;
    for (r, row) in out.iter_mut().enumerate().take(3) {
        for (c, cell) in row.iter_mut().enumerate().take(3) {
            // The adjugate is the transposed cofactors.
            *cell = (cofactor(c, r) / det) as f32;
        }
    }
    let t = transform(&out, [m[3][0], m[3][1], m[3][2]]);
    out[3] = [-t[0], -t[1], -t[2], 1.0];
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{invert, multiply, transform, Matrix, Scene, IDENTITY};

    fn translation(x: f32, y: f32, z: f32) -> Matrix {
        let mut m = IDENTITY;
        m[3] = [x, y, z, 1.0];
        m
    }

    #[test]
    fn a_frame_is_where_its_parents_put_it() {
        let mut scene = Scene::default();
        scene.add(1, 0);
        scene.add(2, 1);
        scene.frame_mut(1).unwrap().local = translation(10.0, 0.0, 0.0);
        scene.frame_mut(2).unwrap().local = translation(0.0, 5.0, 0.0);
        let world = scene.world(2).unwrap();
        assert_eq!(transform(&world, [0.0, 0.0, 0.0]), [10.0, 5.0, 0.0]);
        assert_eq!(scene.root(2), Some(1));
    }

    #[test]
    fn a_transform_and_its_inverse_cancel() {
        // A quarter turn about y, scaled by two, then moved.
        let m: Matrix = [
            [0.0, 0.0, -2.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [2.0, 0.0, 0.0, 0.0],
            [3.0, -4.0, 5.0, 1.0],
        ];
        let back = multiply(&m, &invert(&m).unwrap());
        for (i, row) in back.iter().enumerate() {
            for (j, cell) in row.iter().enumerate() {
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((cell - want).abs() < 1e-5, "[{i}][{j}] is {cell}");
            }
        }
    }
}
