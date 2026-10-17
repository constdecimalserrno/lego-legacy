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
}

/// A scene's fog: whether it is on, its colour, how it thickens (linear,
/// exponential or exponential squared) and over what distances, and where
/// the title asked for it to be computed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fog {
    pub enabled: bool,
    pub color: u32,
    pub mode: u32,
    pub start: f32,
    pub end: f32,
    pub density: f32,
    pub method: u32,
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

/// A viewport's rectangle on its device: across, down, wide and high, in
/// pixels.
pub type Rect = (u32, u32, u32, u32);

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
}

impl Default for View {
    fn default() -> Self {
        Self {
            front: 1.0,
            back: 100.0,
            plane: [-0.5, 0.5, -0.5, 0.5],
            projection: 0,
        }
    }
}

impl View {
    /// The projection a perspective viewport draws through, in Direct3D's
    /// convention: the front plane's edges go to -1 and 1 across and up, and
    /// depth from the front plane to the back one goes 0 to 1.
    ///
    /// The plane goes to the viewport's own edges whatever its shape, so a
    /// square field on a 640 by 480 viewport is wider than it is tall. That
    /// is what Rock Raiders' levels are drawn for: cut to the viewport's
    /// shape instead, as uniform scaling reads in retained mode's
    /// documentation, a raider stands a third too tall and the level's edges
    /// fall outside what the title works out it can see, and are left black.
    // ponytail: measured against an earlier, unpublished emulator's frames of
    // that mission, which a person played, not against native retained mode;
    // a title drawn for the cut would show it.
    #[must_use]
    pub fn perspective(&self) -> Matrix {
        let [left, right, bottom, top] = self.plane;
        let (n, f) = (self.front, self.back);
        let q = f / (f - n);
        [
            [2.0 * n / (right - left), 0.0, 0.0, 0.0],
            [0.0, 2.0 * n / (top - bottom), 0.0, 0.0],
            [
                -(right + left) / (right - left),
                -(top + bottom) / (top - bottom),
                q,
                1.0,
            ],
            [0.0, 0.0, -n * q, 0.0],
        ]
    }

    /// Where a point in the scene lands on the device, seen from a camera
    /// placed by `camera`, through a viewport over `rect`: across and down in
    /// pixels, and depth from the front plane to the back one, each times the
    /// point's distance in front of the camera, which is the fourth.
    #[must_use]
    pub fn to_screen(&self, rect: Rect, camera: &Matrix, point: [f32; 3]) -> Option<[f32; 4]> {
        let (x, y, width, height) = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        let clip_from_world = multiply(&invert(camera)?, &self.perspective());
        let [cx, cy, cz, cw] =
            super::pipeline::project(&clip_from_world, [point[0], point[1], point[2], 1.0]);
        Some([
            (x + width / 2.0) * cw + cx * width / 2.0,
            (y + height / 2.0) * cw - cy * height / 2.0,
            cz,
            cw,
        ])
    }

    /// The point in the scene a place on the device is: [`View::to_screen`]
    /// undone. A place with no distance, or at the depth the back plane
    /// only reaches at infinity, is no point, and answers `None`.
    #[must_use]
    pub fn from_screen(&self, rect: Rect, camera: &Matrix, screen: [f32; 4]) -> Option<[f32; 3]> {
        let (x, y, width, height) = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        let w = screen[3];
        let [sx, sy, depth] = [screen[0] / w, screen[1] / w, screen[2] / w];
        let p = self.perspective();
        // Depth goes q(z - front)/z, with q the back over the distance
        // between the planes.
        let (q, front) = (p[2][2], self.front);
        if w == 0.0 || depth == q {
            return None;
        }
        let z = q * front / (q - depth);
        let across = (sx - x - width / 2.0) / (width / 2.0);
        let up = (y + height / 2.0 - sy) / (height / 2.0);
        let eye = [
            z * (across - p[2][0]) / p[0][0],
            z * (up - p[2][1]) / p[1][1],
            z,
        ];
        Some(transform(camera, eye))
    }
}

/// Every frame the guest has made, by the pointer it holds; every light and
/// every viewport.
#[derive(Default)]
pub struct Scene {
    pub frames: BTreeMap<u32, Frame>,
    pub lights: BTreeMap<u32, Light>,
    pub viewports: BTreeMap<u32, View>,
    /// Where `D3DRMVectorRandom`'s sequence has got to, once it has started.
    pub random: Option<u32>,
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

    #[test]
    fn a_perspective_viewport_puts_its_front_plane_on_the_edges_of_the_screen() {
        let view = super::View::default();
        let m = view.perspective();
        let clip = |[x, y, z]: [f32; 3]| {
            let w = x * m[0][3] + y * m[1][3] + z * m[2][3] + m[3][3];
            [
                (x * m[0][0] + y * m[1][0] + z * m[2][0] + m[3][0]) / w,
                (x * m[0][1] + y * m[1][1] + z * m[2][1] + m[3][1]) / w,
                (x * m[0][2] + y * m[1][2] + z * m[2][2] + m[3][2]) / w,
            ]
        };
        // The front plane is half a unit either side, across and up alike.
        assert_eq!(clip([0.5, 0.5, 1.0]), [1.0, 1.0, 0.0]);
        assert_eq!(clip([-1.0, -1.0, 2.0]), [-1.0, -1.0, 0.5050505]);
        let back = clip([0.0, 0.0, 100.0]);
        assert!(
            (back[2] - 1.0).abs() < 1e-6,
            "the back plane is as deep as it goes"
        );
    }

    fn translation(x: f32, y: f32, z: f32) -> Matrix {
        let mut m = IDENTITY;
        m[3] = [x, y, z, 1.0];
        m
    }

    #[test]
    fn a_point_put_on_the_screen_comes_back_to_where_it_was() {
        let view = super::View {
            back: 50.0,
            ..super::View::default()
        };
        // A camera moved and turned a quarter about y.
        let camera: Matrix = [
            [0.0, 0.0, -1.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [3.0, 2.0, 1.0, 1.0],
        ];
        let rect = (10, 20, 320, 240);
        // Straight ahead of the camera is the middle of its viewport.
        let ahead = transform(&camera, [0.0, 0.0, 5.0]);
        let screen = view.to_screen(rect, &camera, ahead).unwrap();
        assert_eq!(
            [screen[0] / screen[3], screen[1] / screen[3]],
            [170.0, 140.0]
        );
        let point = transform(&camera, [0.5, -0.25, 7.0]);
        let screen = view.to_screen(rect, &camera, point).unwrap();
        let back = view.from_screen(rect, &camera, screen).unwrap();
        for (b, p) in back.iter().zip(point) {
            assert!((b - p).abs() < 1e-4, "{back:?} is not {point:?}");
        }
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
