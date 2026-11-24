//! The scene a title builds through retained mode.
//!
//! Frames are the title's own tree of transforms, and what hangs on them. The
//! COM objects the guest holds are pointers; the tree lives here, keyed by
//! those pointers, so that what the guest reads back from a frame is what it
//! put there.

use std::collections::BTreeMap;

use crate::cpu::x87::answer_single;

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
    /// Cached slab index of `parent` in [`FrameTable`], or `u32::MAX` when unset.
    pub(crate) parent_slot: u32,
    pub children: Vec<u32>,
    pub visuals: Vec<u32>,
    pub lights: Vec<u32>,
    /// Where this frame is relative to its parent.
    pub local: Matrix,
    pub zbuffer_mode: u32,
    pub color: u32,
    /// The scene's background colour, when this frame is a scene's root.
    pub background: u32,
    /// Whether the scene's fog is on, when this frame is a scene's root.
    pub fog: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            parent: 0,
            parent_slot: FRAME_EMPTY,
            children: Vec::new(),
            visuals: Vec::new(),
            lights: Vec::new(),
            local: IDENTITY,
            // D3DRMZBUFFER_FROMPARENT: what a new frame is documented to start
            // with.
            zbuffer_mode: 0,
            // A frame's own colour starts opaque white.
            color: 0xffff_ffff,
            // A scene's background starts opaque black, as Wine's tests hold
            // native retained mode to.
            background: 0xff00_0000,
            fog: false,
        }
    }
}

/// A light, as a title sets it up. What it has not been told is `None`, and
/// the viewport's render lights with a value of its own in its place.
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

/// A mesh's geometry, in the groups a title added it in (#119).
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub groups: Vec<Group>,
}

/// One group of a mesh: its vertices and faces, and what it is drawn with.
/// What a title has not set is `None`.
#[derive(Clone, Debug, Default)]
pub struct Group {
    /// Each vertex as a `D3DRMVERTEX` lays it out, in the guest's own bits:
    /// position, normal, the two texture coordinates, then its colour. Copied
    /// in, because LEGO Island fills every group's from the same scratch.
    pub vertices: Vec<Option<[u32; 9]>>,
    /// How many vertices make a face, and every face's vertex indices, one
    /// face after another.
    pub per_face: u32,
    pub indices: Vec<u32>,
    /// True when `per_face >= 3` and every face vertex index is in bounds of `vertices`.
    pub(crate) all_indices_in_bounds: bool,
    pub colour: Option<u32>,
    pub mapping: Option<u32>,
    pub quality: Option<u32>,
    /// The material and the texture it is drawn with, each as the guest
    /// handed it over, or zero for none.
    pub material: u32,
    pub texture: u32,
}

/// How a material reflects light (#120): what a title has set, and otherwise
/// what Wine's d3drm starts one with, a white highlight and nothing given off.
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub power: f32,
    pub specular: [f32; 3],
    pub emissive: [f32; 3],
}

impl Material {
    #[must_use]
    pub fn new(power: f32) -> Self {
        Self {
            power,
            specular: [1.0; 3],
            emissive: [0.0; 3],
        }
    }
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
    /// The projection a perspective viewport `width` by `height` draws
    /// through, in Direct3D's convention: what reaches -1 and 1 across and up
    /// is on the viewport's edges, and depth from the front plane to the back
    /// one goes 0 to 1.
    ///
    /// Uniform scaling, which retained mode starts with, scales "the viewing
    /// volume into the larger window dimension" (`GetUniformScaling`): the
    /// plane reaches the edges of the viewport's longer side, and the shorter
    /// side shows as much of it as the same scale fits, so a square stays
    /// square. A field of 0.5 on 640 by 480 is half a unit across and 0.375
    /// up. Native retained mode's own viewport underneath has one scale, the
    /// longer side's half (Wine's tests), and in frames of the real games on
    /// Windows, Rock Raiders' Chief and LEGO Island's plants stand where this
    /// puts them (#134).
    #[must_use]
    pub fn perspective(&self, width: u32, height: u32) -> Matrix {
        let [left, right, bottom, top] = self.plane;
        let (n, f) = (self.front, self.back);
        let q = f / (f - n);
        let longer = width.max(height).max(1) as f32;
        let across = longer / width.max(1) as f32;
        let up = longer / height.max(1) as f32;
        [
            [across * 2.0 * n / (right - left), 0.0, 0.0, 0.0],
            [0.0, up * 2.0 * n / (top - bottom), 0.0, 0.0],
            [
                -across * (right + left) / (right - left),
                -up * (top + bottom) / (top - bottom),
                q,
                1.0,
            ],
            [0.0, 0.0, -n * q, 0.0],
        ]
        .map(|row| settled(row, || self.numbers()))
    }

    /// What the view is set up with, as operands.
    fn numbers(&self) -> impl Iterator<Item = f32> + Clone {
        self.plane.into_iter().chain([self.front, self.back])
    }

    /// Where a point in the scene lands on the device, seen from a camera
    /// placed by `camera`, through a viewport over `rect`: across and down in
    /// pixels, and depth from the front plane to the back one, each times the
    /// point's distance in front of the camera, which is the fourth.
    #[must_use]
    pub fn to_screen(&self, rect: Rect, camera: &Matrix, point: [f32; 3]) -> Option<[f32; 4]> {
        let (x, y, width, height) = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        let clip_from_world = multiply(&invert(camera)?, &self.perspective(rect.2, rect.3));
        let [cx, cy, cz, cw] =
            super::pipeline::project(&clip_from_world, [point[0], point[1], point[2], 1.0]);
        Some(settled(
            [
                (x + width / 2.0) * cw + cx * width / 2.0,
                (y + height / 2.0) * cw - cy * height / 2.0,
                cz,
                cw,
            ],
            || {
                self.numbers()
                    .chain(camera.iter().flatten().copied())
                    .chain(point)
            },
        ))
    }

    /// The point in the scene a place on the device is: [`View::to_screen`]
    /// undone. A place with no distance, or at the depth the back plane
    /// only reaches at infinity, is no point, and answers `None`.
    #[must_use]
    pub fn from_screen(&self, rect: Rect, camera: &Matrix, screen: [f32; 4]) -> Option<[f32; 3]> {
        let (x, y, width, height) = (rect.0 as f32, rect.1 as f32, rect.2 as f32, rect.3 as f32);
        let w = screen[3];
        let [sx, sy, depth] = [screen[0] / w, screen[1] / w, screen[2] / w];
        let p = self.perspective(rect.2, rect.3);
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
        Some(settled(transform(camera, eye), || {
            self.numbers()
                .chain(camera.iter().flatten().copied())
                .chain(screen)
        }))
    }
}

const FRAME_EMPTY: u32 = u32::MAX;

#[inline]
fn hash_u32(mut x: u32) -> usize {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x as usize
}

/// Flat slab of scene frames indexed by a deterministic, zero-entropy open-addressed `u32` table.
#[derive(Clone, Debug, Default)]
pub struct FrameTable {
    slots: Vec<Frame>,
    keys: Vec<u32>,
    table: Vec<(u32, u32)>,
}

impl FrameTable {
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[inline]
    pub(crate) fn slot_of(&self, key: u32) -> Option<u32> {
        if self.table.is_empty() {
            return None;
        }
        let mask = self.table.len() - 1;
        let mut idx = hash_u32(key) & mask;
        loop {
            let (k, v) = self.table[idx];
            if v == FRAME_EMPTY {
                return None;
            }
            if k == key {
                return Some(v);
            }
            idx = (idx + 1) & mask;
        }
    }

    #[inline]
    pub(crate) fn by_slot(&self, slot: u32) -> &Frame {
        &self.slots[slot as usize]
    }

    #[inline]
    pub(crate) fn by_slot_mut(&mut self, slot: u32) -> &mut Frame {
        &mut self.slots[slot as usize]
    }

    #[inline]
    pub(crate) fn parent_slot_of(&self, frame: &Frame) -> Option<u32> {
        if frame.parent == 0 {
            return None;
        }
        let ps = frame.parent_slot;
        if ps != FRAME_EMPTY && self.keys[ps as usize] == frame.parent {
            return Some(ps);
        }
        self.slot_of(frame.parent)
    }

    #[must_use]
    #[inline]
    pub fn get(&self, key: &u32) -> Option<&Frame> {
        self.slot_of(*key).map(|s| &self.slots[s as usize])
    }

    #[must_use]
    #[inline]
    pub fn get_mut(&mut self, key: &u32) -> Option<&mut Frame> {
        self.slot_of(*key).map(|s| &mut self.slots[s as usize])
    }

    #[must_use]
    #[inline]
    pub fn contains_key(&self, key: &u32) -> bool {
        self.slot_of(*key).is_some()
    }

    pub fn insert(&mut self, key: u32, frame: Frame) {
        if let Some(slot) = self.slot_of(key) {
            self.slots[slot as usize] = frame;
            return;
        }
        if (self.slots.len() + 1) * 2 > self.table.len() {
            let new_cap = (self.table.len() * 2).max(64);
            let mut new_table = vec![(0u32, FRAME_EMPTY); new_cap];
            let mask = new_cap - 1;
            for &(k, v) in &self.table {
                if v != FRAME_EMPTY {
                    let mut idx = hash_u32(k) & mask;
                    while new_table[idx].1 != FRAME_EMPTY {
                        idx = (idx + 1) & mask;
                    }
                    new_table[idx] = (k, v);
                }
            }
            self.table = new_table;
        }
        let slot = self.slots.len() as u32;
        self.slots.push(frame);
        self.keys.push(key);
        let mask = self.table.len() - 1;
        let mut idx = hash_u32(key) & mask;
        while self.table[idx].1 != FRAME_EMPTY {
            idx = (idx + 1) & mask;
        }
        self.table[idx] = (key, slot);
    }
}

impl std::ops::Index<&u32> for FrameTable {
    type Output = Frame;

    fn index(&self, key: &u32) -> &Self::Output {
        self.get(key).expect("frame key not present")
    }
}

/// Projected clip-space vertex cached during a mesh group's frustum test.
#[derive(Clone, Copy, Default)]
pub(crate) struct ClipSlot {
    pub(crate) clip: [f32; 4],
    pub(crate) outcode: u8,
    pub(crate) valid: bool,
}

/// Lazily lit vertex colour cached across faces sharing a vertex.
#[derive(Clone, Copy, Default)]
pub(crate) struct LitSlot {
    pub(crate) diffuse: [f32; 4],
    pub(crate) specular: [f32; 3],
    pub(crate) valid: bool,
}

/// Reusable working buffers for retained-mode viewport renders so frame and
/// mesh traversal do not reallocate every frame.
#[derive(Default)]
pub(crate) struct RenderScratch {
    pub(crate) meshes: Vec<(Matrix, u32, u32)>,
    pub(crate) lights: Vec<(u32, super::pipeline::Light)>,
    pub(crate) mesh_lights: Vec<super::pipeline::Light>,
    pub(crate) prepared_lights: super::pipeline::PreparedLights,
    pub(crate) clip_slots: Vec<ClipSlot>,
    pub(crate) lit_slots: Vec<LitSlot>,
    pub(crate) triangles: Vec<[super::pipeline::Clipped; 3]>,
}

/// Every frame the guest has made, by the pointer it holds; every light,
/// every mesh, material and texture, and every viewport.
#[derive(Default)]
pub struct Scene {
    pub frames: FrameTable,
    pub lights: BTreeMap<u32, Light>,
    pub meshes: BTreeMap<u32, Mesh>,
    pub materials: BTreeMap<u32, Material>,
    /// Each texture's surface, which its texels are read from as it is
    /// drawn, never copied: a title that changes the surface's pixels has
    /// changed the texture (#120).
    pub textures: BTreeMap<u32, u32>,
    pub viewports: BTreeMap<u32, View>,
    /// Where `D3DRMVectorRandom`'s sequence has got to, once it has started.
    pub random: Option<u32>,
    pub(crate) scratch: RenderScratch,
}

#[inline]
fn is_exact_identity(m: &Matrix) -> bool {
    for r in 0..4 {
        for c in 0..4 {
            if m[r][c].to_bits() != IDENTITY[r][c].to_bits() {
                return false;
            }
        }
    }
    true
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
        let child_slot = self.frames.slot_of(child);
        let previous = child_slot.map_or(0, |s| self.frames.by_slot(s).parent);
        if previous != 0 {
            if let Some(old) = self.frames.get_mut(&previous) {
                old.children.retain(|c| *c != child);
            }
        }
        let parent_slot = if parent != 0 {
            self.frames.slot_of(parent)
        } else {
            None
        };
        if let Some(cs) = child_slot {
            let frame = self.frames.by_slot_mut(cs);
            frame.parent = parent;
            frame.parent_slot = parent_slot.unwrap_or(FRAME_EMPTY);
        }
        if let Some(ps) = parent_slot {
            let frame = self.frames.by_slot_mut(ps);
            if !frame.children.contains(&child) {
                frame.children.push(child);
            }
        }
    }

    #[inline]
    pub fn frame(&self, at: u32) -> Option<&Frame> {
        self.frames.get(&at)
    }

    /// The frame at the top of the tree a frame hangs in: its scene.
    #[must_use]
    pub fn root(&self, mut at: u32) -> Option<u32> {
        let mut slot = self.frames.slot_of(at)?;
        let mut steps = 0;
        let limit = self.frames.len();
        loop {
            let frame = self.frames.by_slot(slot);
            if frame.parent == 0 || steps > limit {
                return Some(at);
            }
            at = frame.parent;
            slot = self.frames.parent_slot_of(frame)?;
            steps += 1;
        }
    }

    /// Whether `frame` equals `ancestor` or hangs anywhere under it.
    #[must_use]
    pub(crate) fn is_under(&self, mut frame: u32, ancestor: u32) -> bool {
        let mut slot = self.frames.slot_of(frame);
        let limit = self.frames.len();
        for _ in 0..=limit {
            if frame == ancestor {
                return true;
            }
            let Some(s) = slot else {
                return false;
            };
            let node = self.frames.by_slot(s);
            if node.parent == 0 {
                return false;
            }
            frame = node.parent;
            slot = self.frames.parent_slot_of(node);
        }
        false
    }

    /// Where a frame is in its scene: its own transform, then its parent's,
    /// all the way up.
    #[must_use]
    pub fn world(&self, at: u32) -> Option<Matrix> {
        let mut out = IDENTITY;
        let mut out_finite = true;
        let mut next_id = at;
        let mut next_slot = if at != 0 {
            self.frames.slot_of(at)
        } else {
            None
        };
        for _ in 0..=self.frames.len() {
            if next_id == 0 {
                return Some(out);
            }
            let slot = next_slot?;
            let frame = self.frames.by_slot(slot);
            if !(out_finite && is_exact_identity(&frame.local)) {
                out = multiply(&out, &frame.local);
                out_finite = out.iter().flatten().all(|v| v.is_finite());
            }
            next_id = frame.parent;
            next_slot = self.frames.parent_slot_of(frame);
        }
        // A frame that is its own ancestor has no place in any scene.
        None
    }

    /// Row 3 of [`Self::world`]: bit-for-bit identical to `self.world(at).map(|m| m[3])`.
    #[must_use]
    pub fn world_origin_row(&self, at: u32) -> Option<[f32; 4]> {
        let mut row = IDENTITY[3];
        let mut row_finite = true;
        let mut next_id = at;
        let mut next_slot = if at != 0 {
            self.frames.slot_of(at)
        } else {
            None
        };
        for _ in 0..=self.frames.len() {
            if next_id == 0 {
                return Some(row);
            }
            let slot = next_slot?;
            let frame = self.frames.by_slot(slot);
            if !(row_finite && is_exact_identity(&frame.local)) {
                row = multiply_row(row, &frame.local);
                row_finite = row.iter().all(|v| v.is_finite());
            }
            next_id = frame.parent;
            next_slot = self.frames.parent_slot_of(frame);
        }
        None
    }

    #[inline]
    pub fn frame_mut(&mut self, at: u32) -> Option<&mut Frame> {
        self.frames.get_mut(&at)
    }
}

/// One row multiplied on the left of `b`: bit-for-bit identical to `multiply(&a, b)[i]`
/// for `row == a[i]`, including left-fold `-0.0` canonicalisation and NaN settling.
#[must_use]
#[inline]
pub fn multiply_row(row: [f32; 4], b: &Matrix) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (j, cell) in out.iter_mut().enumerate() {
        let sum =
            (((0.0 + row[0] * b[0][j]) + row[1] * b[1][j]) + row[2] * b[2][j]) + row[3] * b[3][j];
        *cell = if sum.is_nan() {
            settle_cell(sum, row, [b[0][j], b[1][j], b[2][j], b[3][j]])
        } else {
            sum
        };
    }
    out
}

/// `a` then `b`, for row vectors: a point goes through `a` first.
#[must_use]
pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        let ai = a[i];
        for (j, cell) in row.iter_mut().enumerate() {
            // Left-fold from 0.0 matches Iterator::sum and turns all-negative-zero
            // products into +0.0.
            let sum =
                (((0.0 + ai[0] * b[0][j]) + ai[1] * b[1][j]) + ai[2] * b[2][j]) + ai[3] * b[3][j];
            *cell = if sum.is_nan() {
                settle_cell(sum, ai, [b[0][j], b[1][j], b[2][j], b[3][j]])
            } else {
                sum
            };
        }
    }
    out
}

#[cold]
#[inline(never)]
fn settle_cell(cell: f32, a_row: [f32; 4], b_col: [f32; 4]) -> f32 {
    answer_single(
        cell,
        [
            a_row[0], b_col[0], a_row[1], b_col[1], a_row[2], b_col[2], a_row[3], b_col[3],
        ],
    )
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
    .map(|row| settled(row, || axis.into_iter().chain([theta])))
}

#[must_use]
pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    settled(
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ],
        || a.into_iter().chain(b),
    )
}

/// A vector scaled to length one, or `None` for one with no length.
#[must_use]
pub fn normalise(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 0.0).then(|| normalise_with_length(v, length))
}

/// Divide `v` by its already-computed positive `length`, settling any NaN payload.
#[must_use]
#[inline]
pub(crate) fn normalise_with_length(v: [f32; 3], length: f32) -> [f32; 3] {
    settled(v.map(|c| c / length), || v)
}

/// A direction through a transform: turned and scaled, never moved.
#[must_use]
pub fn direct(m: &Matrix, [x, y, z]: [f32; 3]) -> [f32; 3] {
    settled(
        [
            x * m[0][0] + y * m[1][0] + z * m[2][0],
            x * m[0][1] + y * m[1][1] + z * m[2][1],
            x * m[0][2] + y * m[1][2] + z * m[2][2],
        ],
        || [x, y, z].into_iter().chain(m.iter().flatten().copied()),
    )
}

/// A point through a transform.
#[must_use]
pub fn transform(m: &Matrix, [x, y, z]: [f32; 3]) -> [f32; 3] {
    settled(
        [
            x * m[0][0] + y * m[1][0] + z * m[2][0] + m[3][0],
            x * m[0][1] + y * m[1][1] + z * m[2][1] + m[3][1],
            x * m[0][2] + y * m[1][2] + z * m[2][2] + m[3][2],
        ],
        || [x, y, z].into_iter().chain(m.iter().flatten().copied()),
    )
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
    Some(out.map(|row| settled(row, || m.iter().flatten().copied())))
}

/// A result's parts as a real x86 machine gives them where one is not a
/// number: by [`answer_single`]'s rule, from the operands it was made from,
/// never the NaN this host made (#109). What the guest reads back from
/// retained mode is made here, so a run's state is the same on every host.
// ponytail: every operand counts for every part, so two parts made from two
// different NaNs both come out the larger; x86 doing these same sums would
// give each its own. Only a guest passing two NaNs to one call would see it.
#[inline]
fn settled<const N: usize, I>(parts: [f32; N], operands: impl Fn() -> I) -> [f32; N]
where
    I: IntoIterator<Item = f32>,
{
    if parts.iter().all(|p| !p.is_nan()) {
        return parts;
    }
    settle_nan(parts, operands)
}

#[cold]
#[inline(never)]
fn settle_nan<const N: usize, I>(parts: [f32; N], operands: impl Fn() -> I) -> [f32; N]
where
    I: IntoIterator<Item = f32>,
{
    parts.map(|p| answer_single(p, operands()))
}

#[cfg(test)]
mod tests {
    use super::{answer_single, invert, multiply, normalise, transform, Matrix, Scene, IDENTITY};

    #[test]
    fn a_nan_retained_mode_makes_is_the_same_bits_on_every_host() {
        let bits = |v: [f32; 3]| v.map(f32::to_bits);
        // An infinite part has no direction: infinity over infinity, which an
        // x86 host makes negative and an Arm host positive.
        let endless = std::hint::black_box([f32::INFINITY, 0.0, 0.0]);
        assert_eq!(bits(normalise(endless).unwrap()), [0xffc0_0000, 0, 0]);
        for made in [0x7fc0_0000, 0xffc0_0000] {
            let answer = answer_single(f32::from_bits(made), [f32::INFINITY, 0.0]);
            assert_eq!(answer.to_bits(), 0xffc0_0000, "whichever the host made");
        }
        // A NaN the guest passed comes through, quieted.
        let signalling = std::hint::black_box(f32::from_bits(0x7f80_0001));
        let moved = transform(&IDENTITY, [signalling, 1.0, 2.0]);
        assert_eq!(bits(moved), [0x7fc0_0001; 3]);
    }

    #[test]
    fn a_perspective_viewport_puts_its_front_plane_on_the_edges_of_the_screen() {
        let view = super::View::default();
        let m = view.perspective(640, 480);
        let clip = |[x, y, z]: [f32; 3]| {
            let w = x * m[0][3] + y * m[1][3] + z * m[2][3] + m[3][3];
            [
                (x * m[0][0] + y * m[1][0] + z * m[2][0] + m[3][0]) / w,
                (x * m[0][1] + y * m[1][1] + z * m[2][1] + m[3][1]) / w,
                (x * m[0][2] + y * m[1][2] + z * m[2][2] + m[3][2]) / w,
            ]
        };
        // The front plane is half a unit either side across, the longer side,
        // and up the shorter side as much of it as the same scale fits.
        assert_eq!(clip([0.5, 0.375, 1.0]), [1.0, 1.0, 0.0]);
        assert_eq!(clip([-1.0, -0.75, 2.0]), [-1.0, -1.0, 0.5050505]);
        let back = clip([0.0, 0.0, 100.0]);
        assert!(
            (back[2] - 1.0).abs() < 1e-6,
            "the back plane is as deep as it goes"
        );
    }

    /// A field is the half-width at the front plane, wherever that plane is:
    /// LEGO Island's is a tenth of a unit away, with a field of 0.06128 there
    /// (#120), where Rock Raiders' plane a unit away hid the difference. On a
    /// viewport taller than it is wide, it is the half-height.
    #[test]
    fn a_field_is_the_half_width_at_the_front_plane() {
        let field = f32::from_bits(0x3d7b_00d2);
        let view = super::View {
            front: 0.1,
            back: 250.0,
            plane: [-field, field, -field, field],
            projection: 0,
        };
        let m = view.perspective(640, 480);
        let across = |x: f32, z: f32| (x * m[0][0] + z * m[2][0]) / (z * m[2][3]);
        assert!((across(field, 0.1) - 1.0).abs() < 1e-6);
        // Ten times as far, the edge is ten times as far out.
        assert!((across(field * 10.0, 1.0) - 1.0).abs() < 1e-6);
        let m = view.perspective(480, 640);
        let up = |y: f32, z: f32| (y * m[1][1] + z * m[2][1]) / (z * m[2][3]);
        assert!((up(field, 0.1) - 1.0).abs() < 1e-6);
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

    #[test]
    fn multiply_preserves_zero_sign_and_cold_nan_payloads() {
        // All -0.0 products fold from +0.0 into +0.0, matching Iterator::sum.
        let neg_zero = [[-0.0f32; 4]; 4];
        let ones = [[1.0f32; 4]; 4];
        let prod = multiply(&neg_zero, &ones);
        for row in prod {
            for cell in row {
                assert_eq!(cell.to_bits(), 0.0f32.to_bits());
            }
        }
        // Inf * 0.0 settles to x86 indeterminate NaN regardless of host ISA.
        let mut inf = IDENTITY;
        inf[0][0] = std::hint::black_box(f32::INFINITY);
        let mut zero = IDENTITY;
        zero[0][0] = std::hint::black_box(0.0);
        let res = multiply(&inf, &zero);
        assert_eq!(res[0][0].to_bits(), 0xffc0_0000);
        // Guest signaling NaN payload propagates quieted; when two NaNs share a
        // dot product, the larger significand wins.
        let mut nans = IDENTITY;
        nans[0][0] = std::hint::black_box(f32::from_bits(0x7f80_0001));
        nans[1][0] = std::hint::black_box(f32::from_bits(0x7f80_0001));
        nans[1][1] = std::hint::black_box(f32::from_bits(0xff80_0005));
        let res = multiply(&nans, &IDENTITY);
        assert_eq!(res[0][0].to_bits(), 0x7fc0_0001);
        assert_eq!(res[1][0].to_bits(), 0xffc0_0005);
        // Direct and cross also take the cold NaN path accurately.
        let sig = std::hint::black_box(f32::from_bits(0x7f80_0003));
        assert_eq!(
            super::direct(&IDENTITY, [sig, 0.0, 0.0])[0].to_bits(),
            0x7fc0_0003
        );
        assert_eq!(
            super::cross([sig, 0.0, 0.0], [0.0, 1.0, 0.0])[2].to_bits(),
            0x7fc0_0003
        );
    }

    #[test]
    fn flat_frame_table_world_and_origin_row_match_reference_bit_for_bit() {
        let mut scene = Scene::default();
        // Insert enough frames to trigger multiple table rehashes, including key 0x1000..0x1100.
        let root = 0x1000;
        scene.add(root, 0);
        // Leave root at exact IDENTITY to exercise the identity skip path.
        let mut prev = root;
        for i in 1u32..=120 {
            let id = 0x1000 + i * 16;
            scene.add(id, prev);
            let f = scene.frame_mut(id).unwrap();
            if i % 3 == 0 {
                // Keep exact IDENTITY on every third frame.
                f.local = IDENTITY;
            } else {
                let mut m = super::rotation([0.3, 0.7, -0.5], i as f32 * 0.13);
                m[3] = [i as f32 * 0.5, -(i as f32) * 0.25, i as f32 * 0.125, 1.0];
                f.local = m;
            }
            if i % 10 == 0 {
                prev = root;
            } else {
                prev = id;
            }
        }
        // Re-parent a subtree to verify parent_slot updates cleanly.
        let reparented = 0x1000 + 25 * 16;
        let new_parent = 0x1000 + 40 * 16;
        scene.attach(new_parent, reparented);

        // Reference bottom-up multiply without identity skip.
        fn ref_world(scene: &Scene, at: u32) -> Option<Matrix> {
            let mut out = IDENTITY;
            let mut next = at;
            for _ in 0..=scene.frames.len() {
                if next == 0 {
                    return Some(out);
                }
                let f = scene.frame(next)?;
                out = multiply(&out, &f.local);
                next = f.parent;
            }
            None
        }

        for i in 0u32..=120 {
            let id = 0x1000 + i * 16;
            let want = ref_world(&scene, id).unwrap();
            let got = scene.world(id).unwrap();
            let row = scene.world_origin_row(id).unwrap();
            for r in 0..4 {
                assert_eq!(got[r].map(f32::to_bits), want[r].map(f32::to_bits));
            }
            assert_eq!(row.map(f32::to_bits), want[3].map(f32::to_bits));
            assert_eq!(scene.root(id), Some(root));
            assert!(scene.is_under(id, root));
        }
        assert!(scene.is_under(reparented, new_parent));
        assert!(!scene.is_under(new_parent, reparented));

        // Signaling NaN and -0.0 in local matrix propagate identically through world & world_origin_row.
        let neg_only = 0x8ff0;
        let nan_frame = 0x9000;
        let neg_zero_frame = 0x9010;
        scene.add(neg_only, root);
        scene.add(nan_frame, root);
        scene.add(neg_zero_frame, nan_frame);
        let mut neg_m = IDENTITY;
        neg_m[3][0] = -0.0;
        scene.frame_mut(neg_only).unwrap().local = neg_m;
        scene.frame_mut(neg_zero_frame).unwrap().local = neg_m;
        let mut nan_m = IDENTITY;
        nan_m[3][1] = std::hint::black_box(f32::from_bits(0x7f80_0007));
        scene.frame_mut(nan_frame).unwrap().local = nan_m;

        // -0.0 canonicalises to +0.0 even when root is IDENTITY.
        let want_neg = ref_world(&scene, neg_only).unwrap();
        let got_neg = scene.world(neg_only).unwrap();
        let row_neg = scene.world_origin_row(neg_only).unwrap();
        assert_eq!(row_neg.map(f32::to_bits), want_neg[3].map(f32::to_bits));
        assert_eq!(got_neg[3].map(f32::to_bits), want_neg[3].map(f32::to_bits));
        assert_eq!(row_neg[0].to_bits(), 0.0f32.to_bits());

        // Non-finite row still multiplies through root's IDENTITY so NaN * 0.0 propagates identically.
        let want = ref_world(&scene, neg_zero_frame).unwrap();
        let got = scene.world(neg_zero_frame).unwrap();
        let row = scene.world_origin_row(neg_zero_frame).unwrap();
        for r in 0..4 {
            assert_eq!(got[r].map(f32::to_bits), want[r].map(f32::to_bits));
        }
        assert_eq!(row.map(f32::to_bits), want[3].map(f32::to_bits));
        assert_eq!(row[0].to_bits(), 0x7fc0_0007);
        assert_eq!(row[1].to_bits(), 0x7fc0_0007);
    }
}
