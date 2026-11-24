//! Triangles into a surface's own bytes, on the host's processor and never
//! its GPU (ADR-0004).
//!
//! What Direct3D's fixed pipeline does once a triangle is on the screen:
//! which pixels it covers, how deep each is, what colour and texel each takes,
//! and how that lands in a surface laid out however the guest asked for it.
//! Nothing here knows about the guest: the caller hands over the surface's
//! bytes, the depth buffer, and a texture's bytes.

mod span;

use span::DrawPlan;

const PLANES: usize = 12;

/// How a surface lays out one pixel: its size, and where each channel is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits: u32,
    /// Red, green, blue and alpha. An alpha mask of zero is an opaque format.
    pub masks: [u32; 4],
}

impl PixelFormat {
    /// The 5-6-5 format the declared card's 16-bit screen has.
    pub const RGB565: Self = Self {
        bits: 16,
        masks: [0xf800, 0x07e0, 0x001f, 0],
    };

    fn bytes(self) -> usize {
        self.bits.div_ceil(8) as usize
    }

    /// Channels narrowed to this format's own widths.
    #[must_use]
    pub fn encode(self, rgba: [u8; 4]) -> u32 {
        self.masks
            .iter()
            .zip(rgba)
            .filter(|(mask, _)| **mask != 0)
            .map(|(mask, c)| {
                let width = mask.count_ones();
                (u32::from(c) >> (8 - width.min(8))) << mask.trailing_zeros()
            })
            .fold(0, |v, c| v | c)
    }
}

/// A pixel format's channels, worked out once for a draw rather than per
/// pixel: where each channel is, and what one step of it is worth from 0 to 1.
#[derive(Clone, Copy, Debug)]
struct Channels {
    bytes: usize,
    shift: [u32; 4],
    mask: [u32; 4],
    width: [u32; 4],
    step: [f32; 4],
    /// What a channel the format does not have reads as: no colour, and
    /// opaque.
    absent: [f32; 4],
}

impl Channels {
    fn of(format: PixelFormat) -> Self {
        let mut out = Self {
            bytes: format.bytes(),
            shift: [0; 4],
            mask: [0; 4],
            width: [0; 4],
            step: [0.0; 4],
            absent: [0.0, 0.0, 0.0, 1.0],
        };
        for (i, mask) in format.masks.iter().enumerate() {
            if *mask != 0 {
                let width = mask.count_ones();
                out.shift[i] = mask.trailing_zeros();
                out.mask[i] = mask >> out.shift[i];
                out.width[i] = width;
                out.step[i] = 1.0 / ((1u64 << width) - 1) as f32;
            }
        }
        out
    }

    fn read(&self, at: &[u8]) -> u32 {
        match *at {
            [b0] => u32::from(b0),
            [b0, b1] => u32::from(u16::from_le_bytes([b0, b1])),
            [b0, b1, b2] => u32::from_le_bytes([b0, b1, b2, 0]),
            [b0, b1, b2, b3] => u32::from_le_bytes([b0, b1, b2, b3]),
            _ => 0,
        }
    }

    /// A pixel's channels from 0 to 1.
    fn colour(&self, value: u32) -> [f32; 4] {
        std::array::from_fn(|i| {
            if self.mask[i] == 0 {
                self.absent[i]
            } else {
                ((value >> self.shift[i]) & self.mask[i]) as f32 * self.step[i]
            }
        })
    }

    /// Channels from 0 to 1 as a pixel, each rounded to eight bits and then
    /// narrowed to its own width, as [`PixelFormat::encode`] narrows it.
    fn value(&self, colour: [f32; 4]) -> u32 {
        let mut out = 0;
        for (i, c) in colour.iter().enumerate() {
            if self.mask[i] != 0 {
                // Rounded to nearest: never negative, so adding a half and
                // truncating is the same as rounding.
                let eight = (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
                out |= (eight >> (8 - self.width[i].min(8))) << self.shift[i];
            }
        }
        out
    }
}

/// A texture's bytes, in its own format, and a palette if it is indexed.
pub struct Texture<'a> {
    pub bytes: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: PixelFormat,
    /// Eight-bit indexed textures look their colour up here.
    pub palette: Option<&'a [[u8; 4]; 256]>,
    /// The texel value, in the texture's own format, that is not drawn when
    /// colour keying is on.
    pub colour_key: Option<u32>,
    /// Across and down, whether coordinates past the edge stop at it rather
    /// than wrapping round.
    pub clamp: [bool; 2],
}

/// Where a triangle lands: a surface's bytes and its depth buffer.
pub struct Target<'a> {
    pub pixels: &'a mut [u8],
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: PixelFormat,
    /// One depth a pixel, row by row, `width` to a row.
    pub depth: &'a mut [f32],
    /// Left, top, right and bottom, in pixels: the viewport's rectangle.
    pub clip: [i32; 4],
    /// The row of the surface `pixels` and `depth` start at: zero for a
    /// whole surface, and a band's first row for a band of it.
    pub first_row: u32,
}

/// A vertex on the screen: pixels across and down, depth from 0 to 1, the
/// reciprocal of its distance, its colours and texture coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rhw: f32,
    /// Red, green, blue, alpha, from 0 to 1.
    pub diffuse: [f32; 4],
    pub specular: [f32; 3],
    pub uv: [f32; 2],
    /// How much of this vertex's own colour survives the fog: one is clear
    /// air, zero is the fog's colour alone. Read only when there is fog.
    pub fog: f32,
}

/// `D3DCMPFUNC`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

impl Compare {
    /// From `D3DCMP_NEVER` (1) to `D3DCMP_ALWAYS` (8).
    #[must_use]
    pub fn from_d3d(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Never,
            2 => Self::Less,
            3 => Self::Equal,
            4 => Self::LessEqual,
            5 => Self::Greater,
            6 => Self::NotEqual,
            7 => Self::GreaterEqual,
            8 => Self::Always,
            _ => return None,
        })
    }

    fn passes<T: PartialOrd>(self, value: T, reference: T) -> bool {
        match self {
            Self::Never => false,
            Self::Less => value < reference,
            Self::Equal => value == reference,
            Self::LessEqual => value <= reference,
            Self::Greater => value > reference,
            Self::NotEqual => value != reference,
            Self::GreaterEqual => value >= reference,
            Self::Always => true,
        }
    }
}

/// `D3DBLEND`: a factor a colour is weighted by when it is blended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Zero,
    One,
    SourceColour,
    InverseSourceColour,
    SourceAlpha,
    InverseSourceAlpha,
    DestinationAlpha,
    InverseDestinationAlpha,
    DestinationColour,
    InverseDestinationColour,
}

impl Blend {
    /// From `D3DBLEND_ZERO` (1) to `D3DBLEND_INVDESTCOLOR` (10).
    #[must_use]
    pub fn from_d3d(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Zero,
            2 => Self::One,
            3 => Self::SourceColour,
            4 => Self::InverseSourceColour,
            5 => Self::SourceAlpha,
            6 => Self::InverseSourceAlpha,
            7 => Self::DestinationAlpha,
            8 => Self::InverseDestinationAlpha,
            9 => Self::DestinationColour,
            10 => Self::InverseDestinationColour,
            _ => return None,
        })
    }

    fn factor(self, source: [f32; 4], destination: [f32; 4]) -> [f32; 4] {
        let all = |v: f32| [v; 4];
        match self {
            Self::Zero => all(0.0),
            Self::One => all(1.0),
            Self::SourceColour => source,
            Self::InverseSourceColour => source.map(|c| 1.0 - c),
            Self::SourceAlpha => all(source[3]),
            Self::InverseSourceAlpha => all(1.0 - source[3]),
            Self::DestinationAlpha => all(destination[3]),
            Self::InverseDestinationAlpha => all(1.0 - destination[3]),
            Self::DestinationColour => destination,
            Self::InverseDestinationColour => destination.map(|c| 1.0 - c),
        }
    }
}

/// How a texel and the vertex colour make a pixel's colour or alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combine {
    /// The texture's alone.
    Texture,
    /// The vertex colour's alone.
    Diffuse,
    /// The two multiplied.
    Modulate,
    /// The two added.
    Add,
}

/// Everything a triangle is drawn with that is not its vertices.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct States {
    /// The depth comparison, when depth testing is on.
    pub depth_test: Option<Compare>,
    pub depth_write: bool,
    /// Flat shading takes the first vertex's colours for the whole triangle.
    pub flat: bool,
    pub colour: Combine,
    pub alpha: Combine,
    pub linear: bool,
    /// Texels that are the texture's colour key are not drawn.
    pub colour_key: bool,
    /// The alpha comparison and its reference, when alpha testing is on.
    pub alpha_test: Option<(Compare, u8)>,
    /// Source and destination factors, when blending is on.
    pub blend: Option<(Blend, Blend)>,
    pub specular: bool,
    /// The fog's colour, when there is fog. How much of it each pixel gets is
    /// the vertices' own fog factor, interpolated.
    pub fog: Option<[f32; 3]>,
}

impl Default for States {
    fn default() -> Self {
        Self {
            depth_test: Some(Compare::LessEqual),
            depth_write: true,
            flat: false,
            colour: Combine::Modulate,
            alpha: Combine::Modulate,
            linear: false,
            colour_key: false,
            alpha_test: None,
            blend: None,
            specular: false,
            fog: None,
        }
    }
}

/// Draw one triangle.
///
/// Pixels whose centres fall inside it are drawn, a pixel's centre being its
/// whole coordinates, as Direct3D before version 10 puts it; an edge shared by
/// two triangles belongs to whichever it is the top or left edge of, so
/// neither draws it twice nor leaves a gap.
///
/// Each row is walked across three phases: left-edge pixels tested against
/// the three edges, strictly-interior pixels dispatched through `span.rs` with
/// no edge tests and monomorphized hot-path state/format kernels (#254), and
/// right-edge pixels. What is interpolated steps sequentially along each row:
/// depth, the reciprocal of distance, and the colours and texture coordinates
/// weighted by it, so that they are straight in the scene rather than on the
/// screen.
pub fn triangle(
    target: &mut Target<'_>,
    v: [Vertex; 3],
    texture: Option<&Texture<'_>>,
    states: &States,
) {
    let plan = DrawPlan::new(target.format, texture, states);
    triangle_with_plan(target, v, &plan, states);
}

fn triangle_with_plan(
    target: &mut Target<'_>,
    v: [Vertex; 3],
    plan: &DrawPlan<'_>,
    states: &States,
) {
    let [a, b, c] = v;
    let area = edge(a, b, c.x, c.y);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    // Wind every triangle the same way, so one inside test serves both.
    let (b, c, area) = if area < 0.0 {
        (c, b, -area)
    } else {
        (b, c, area)
    };

    let [clip_left, clip_top, clip_right, clip_bottom] = target.clip;
    let left = (a.x.min(b.x).min(c.x).floor() as i32).max(clip_left).max(0);
    let top = (a.y.min(b.y).min(c.y).floor() as i32)
        .max(clip_top)
        .max(target.first_row as i32);
    let right = (a.x.max(b.x).max(c.x).ceil() as i32)
        .min(clip_right)
        .min(target.width as i32);
    let bottom = (a.y.max(b.y).max(c.y).ceil() as i32)
        .min(clip_bottom)
        .min((target.first_row + target.height) as i32);
    if left >= right || top >= bottom {
        return;
    }

    // Each vertex's weight is the edge opposite it, over the area: a plane in
    // x and y. What is interpolated is a sum of those planes.
    let edges = [(b, c), (c, a), (a, b)];
    let coefficient = |(p, q): (Vertex, Vertex)| {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        (-dy, dx, -dx * p.y + dy * p.x)
    };
    let weights = edges.map(coefficient);
    let plane = |values: [f32; 3]| -> (f32, f32, f32) {
        let mut out = (0.0, 0.0, 0.0);
        for ((ex, ey, ek), value) in weights.iter().zip(values) {
            out.0 += ex * value / area;
            out.1 += ey * value / area;
            out.2 += ek * value / area;
        }
        out
    };
    let over_w = |f: fn(&Vertex) -> f32| plane([f(&a) * a.rhw, f(&b) * b.rhw, f(&c) * c.rhw]);
    // A triangle at one depth, its three depths equal bit for bit, has that
    // depth at every pixel: the steps of its depth are differences of equal
    // floats, which are exactly zero. These planes would carry an ulp of noise
    // across it, enough for a quad to fail `LESSEQUAL` against one under it at
    // the same depth: LEGO Chess draws its glyphs so (#210).
    // ponytail: exact for bit-equal depths only; depths equal to within
    // rounding, and sloped triangles sharing a plane, keep the noise. Planes
    // from the vertices' differences for every triangle would mend them, and
    // move every released title's frames, so that waits on a blessing.
    let depth = if a.z == b.z && b.z == c.z && a.z.is_finite() {
        (0.0, 0.0, a.z)
    } else {
        plane([a.z, b.z, c.z])
    };
    // Depth, the reciprocal of distance, texture coordinates, diffuse and
    // specular colour, and fog factor.
    let planes = [
        depth,
        plane([a.rhw, b.rhw, c.rhw]),
        over_w(|v| v.uv[0]),
        over_w(|v| v.uv[1]),
        over_w(|v| v.diffuse[0]),
        over_w(|v| v.diffuse[1]),
        over_w(|v| v.diffuse[2]),
        over_w(|v| v.diffuse[3]),
        over_w(|v| v.specular[0]),
        over_w(|v| v.specular[1]),
        over_w(|v| v.specular[2]),
        over_w(|v| v.fog),
    ];
    let step: [f32; PLANES] = planes.map(|p| p.0);
    let carried = plan.carried;
    let bytes = plan.pixel_channels.bytes;
    let flat = (a.diffuse, a.specular);

    for py in top..bottom {
        let y = py as f32;
        // The span of the row the triangle can cover: past each edge's own
        // crossing of it, a pixel either side to spare, and the exact test
        // deciding the pixels at its ends.
        let (mut from, mut to) = (left as f32, (right - 1) as f32);
        // A row lying along a level edge is decided pixel by pixel.
        let mut on_level_edge = false;
        for (ex, ey, ek) in weights {
            let at_row = ey * y + ek;
            if ex > 0.0 {
                from = from.max(-at_row / ex - 1.0);
            } else if ex < 0.0 {
                to = to.min(-at_row / ex + 1.0);
            } else if at_row < 0.0 {
                to = from - 1.0;
            } else if at_row == 0.0 {
                on_level_edge = true;
            }
        }
        // Nothing to cover, or a crossing that is not a number.
        if from.partial_cmp(&to).is_none_or(|order| order.is_gt()) {
            continue;
        }
        let (from, to) = (
            (from.floor() as i32).max(left),
            (to.ceil() as i32).min(right - 1),
        );
        if from > to {
            continue;
        }
        // Three pixels in from each end of the span, every sloping edge is
        // strictly past its crossing, so only the ends need the exact test.
        let (inside_from, inside_to) = if on_level_edge {
            (to + 1, to)
        } else {
            (from + 3, to - 3)
        };
        let start = from as f32;
        let mut value: [f32; PLANES] = planes.map(|(px, py_, k)| px * start + py_ * y + k);
        let row = (py as u32 - target.first_row) as usize;
        let row_depth = row * target.width as usize;
        let row_pixels = row * target.pitch as usize;

        // Every edge function is monotonic in `x`, so covered pixels on a row
        // form one contiguous interval `[cov_from..=cov_to]`. Trimming the left
        // and right edge pixels (only testing the three end pixels when an
        // interior exists) sends the entire covered span into `span::draw_interior`
        // with zero per-pixel edge tests.
        let is_covered = |px: i32| {
            let x = px as f32;
            let (w0, w1, w2) = (edge(b, c, x, y), edge(c, a, x, y), edge(a, b, x, y));
            covers(w0, b, c) && covers(w1, c, a) && covers(w2, a, b)
        };
        let (cov_from, cov_to) = if inside_from <= inside_to {
            let mut l = from;
            while l < inside_from && !is_covered(l) {
                for (v, s) in value.iter_mut().zip(step).take(carried) {
                    *v += s;
                }
                l += 1;
            }
            let mut r = to;
            while r > inside_to && !is_covered(r) {
                r -= 1;
            }
            (l, r)
        } else {
            let mut l = from;
            while l <= to && !is_covered(l) {
                for (v, s) in value.iter_mut().zip(step).take(carried) {
                    *v += s;
                }
                l += 1;
            }
            if l > to {
                continue;
            }
            let mut r = to;
            while r > l && !is_covered(r) {
                r -= 1;
            }
            (l, r)
        };

        let u_from = cov_from as usize;
        let u_to = cov_to as usize;
        let depth_start = row_depth + u_from;
        let depth_end = row_depth + u_to + 1;
        let pix_start = row_pixels + u_from * bytes;
        let pix_end = row_pixels + (u_to + 1) * bytes;

        if bytes > 0 && depth_end <= target.depth.len() && pix_end <= target.pixels.len() {
            let depth_span = &mut target.depth[depth_start..depth_end];
            let pix_span = &mut target.pixels[pix_start..pix_end];
            span::draw_interior(plan, depth_span, pix_span, &mut value, &step, flat, states);
        } else {
            // Degenerate truncated target slice: fall back to per-pixel checked
            // bounds access so behaviour matches the general loop exactly.
            draw_checked_scanline(
                target, row_depth, row_pixels, cov_from, cov_to, plan, &mut value, &step, carried,
                flat, states,
            );
        }
    }
}

/// Checked scanline fallback used only when the caller passed a truncated
/// `target.pixels` or `target.depth` buffer shorter than the clipped span.
#[cold]
#[allow(clippy::too_many_arguments)]
fn draw_checked_scanline(
    target: &mut Target<'_>,
    row_depth: usize,
    row_pixels: usize,
    from: i32,
    to: i32,
    plan: &DrawPlan<'_>,
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    carried: usize,
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let pixel_channels = &plan.pixel_channels;
    let bytes = pixel_channels.bytes;
    for px in from..=to {
        let here = *value;
        for (v, s) in value.iter_mut().zip(step).take(carried) {
            *v += *s;
        }
        let z = here[0];
        let at = row_depth + px as usize;
        if let Some(test) = states.depth_test {
            if !target.depth.get(at).is_some_and(|d| test.passes(z, *d)) {
                continue;
            }
        }
        let near = if here[1] > 0.0 { 1.0 / here[1] } else { 0.0 };
        let (diffuse, specular) = if states.flat {
            flat
        } else {
            (
                [
                    here[4] * near,
                    here[5] * near,
                    here[6] * near,
                    here[7] * near,
                ],
                [here[8] * near, here[9] * near, here[10] * near],
            )
        };
        let (texel, keyed) = match plan.texture.as_ref() {
            Some(t) => t.sample_fast(here[2] * near, here[3] * near, states.linear),
            None => ([1.0; 4], false),
        };
        if keyed && states.colour_key {
            continue;
        }
        let mut colour = [
            span::combine_channel(states.colour, texel[0], diffuse[0]),
            span::combine_channel(states.colour, texel[1], diffuse[1]),
            span::combine_channel(states.colour, texel[2], diffuse[2]),
            span::combine_channel(states.alpha, texel[3], diffuse[3]),
        ];
        if states.specular {
            for (c, s) in colour.iter_mut().zip(specular) {
                *c = (*c + s).min(1.0);
            }
        }
        if let Some(fog) = states.fog {
            let f = (here[11] * near).clamp(0.0, 1.0);
            for (c, air) in colour.iter_mut().zip(fog) {
                *c = *c * f + air * (1.0 - f);
            }
        }
        if let Some((test, reference)) = states.alpha_test {
            let alpha = (colour[3].clamp(0.0, 1.0) * 255.0).round() as u8;
            if !test.passes(alpha, reference) {
                continue;
            }
        }
        let offset = row_pixels + px as usize * bytes;
        let Some(pixel) = target.pixels.get_mut(offset..offset + bytes) else {
            continue;
        };
        if let Some((source, destination)) = states.blend {
            let under = pixel_channels.colour(pixel_channels.read(pixel));
            let (s, d) = (
                source.factor(colour, under),
                destination.factor(colour, under),
            );
            for i in 0..4 {
                colour[i] = colour[i] * s[i] + under[i] * d[i];
            }
        }
        let packed = pixel_channels.value(colour).to_le_bytes();
        match pixel {
            [lo, hi] => {
                *lo = packed[0];
                *hi = packed[1];
            }
            _ => pixel.copy_from_slice(&packed[..bytes]),
        }
        if states.depth_write {
            if let Some(d) = target.depth.get_mut(at) {
                *d = z;
            }
        }
    }
}

/// How many bands of rows a draw with many pixels is split into, each drawn
/// on its own thread (ADR-0004).
///
/// A declared number, not the host's count of cores: each pixel is worked out
/// by the same triangles in the same order whichever band it is in, so what
/// is drawn is the same on every host, and four bands is what took Rock
/// Raiders' rockfall, 1.2 million pixels a frame of filtered, blended
/// texture at 26 ms on one thread, back under a 60 Hz frame.
///
/// #72 asked for two changes here and both were built and measured, and
/// neither is worth having. A pool of workers that outlive the draw, with
/// this thread drawing the first band itself, moves the spawn and the wait
/// off this thread exactly as predicted — `_pthread_create` and
/// `_dispatch_semaphore_wait_slow` go from 371 of 6755 main-thread samples
/// to 2 — and changes no wall clock at all, because the wait was never
/// waste: it was this thread waiting for band work that still has to happen,
/// on the same four cores either way. Eight bands is the same story: at
/// Alpha Team's menu 106 against 105 M instructions a second, and Rock
/// Raiders' rockfall 5.5 ms a flip against 5.6, both inside the noise of
/// this host. The time at that menu is the interpreter's (#69, #70, #71).
pub const BANDS: u32 = 4;

/// The pixels a draw has to cover before it is worth splitting into bands:
/// about a fifth of a 640x480 screen.
const WORTH_BANDS: f32 = 64_000.0;

/// Draw triangles in order: in bands of rows at once when there are enough
/// pixels to be worth it, one after another otherwise, with the same result.
pub fn triangles(
    target: &mut Target<'_>,
    list: &[[Vertex; 3]],
    texture: Option<&Texture<'_>>,
    states: &States,
) {
    let area: f32 = list
        .iter()
        .map(|[a, b, c]| (edge(*a, *b, c.x, c.y) / 2.0).abs())
        .sum();
    let [left, top, right, bottom] = target.clip;
    let top = top.max(target.first_row as i32);
    let bottom = bottom.min((target.first_row + target.height) as i32);
    let plan = DrawPlan::new(target.format, texture, states);
    if area < WORTH_BANDS || bottom - top < BANDS as i32 * 2 {
        for triangle_ in list {
            triangle_with_plan(target, *triangle_, &plan, states);
        }
        return;
    }
    let (width, pitch, format) = (target.width, target.pitch, target.format);
    let rows_per_band = ((bottom - top) as u32).div_ceil(BANDS);
    // The rows above the clip are nobody's; the rest go to the bands in turn.
    let skip = (top as u32 - target.first_row) as usize;
    let (_, mut pixels) = target
        .pixels
        .split_at_mut((skip * pitch as usize).min(target.pixels.len()));
    let (_, mut depth) = target
        .depth
        .split_at_mut((skip * width as usize).min(target.depth.len()));
    std::thread::scope(|scope| {
        let mut band_top = top as u32;
        while band_top < bottom as u32 {
            let rows = rows_per_band.min(bottom as u32 - band_top);
            let (band_pixels, rest_pixels) =
                pixels.split_at_mut(((rows * pitch) as usize).min(pixels.len()));
            let (band_depth, rest_depth) =
                depth.split_at_mut(((rows * width) as usize).min(depth.len()));
            pixels = rest_pixels;
            depth = rest_depth;
            let first_row = band_top;
            scope.spawn(move || {
                let mut band = Target {
                    pixels: band_pixels,
                    width,
                    height: rows,
                    pitch,
                    format,
                    depth: band_depth,
                    clip: [left, first_row as i32, right, (first_row + rows) as i32],
                    first_row,
                };
                for triangle_ in list {
                    triangle_with_plan(&mut band, *triangle_, &plan, states);
                }
            });
            band_top += rows;
        }
    });
}

/// Twice the signed area of the triangle from `a` to `b` to the point:
/// positive when the point is on the inside of an edge wound clockwise on the
/// screen, whose y runs down.
fn edge(a: Vertex, b: Vertex, x: f32, y: f32) -> f32 {
    (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
}

/// Whether a pixel centre on an edge's line is the triangle's: only on a top
/// or left edge.
fn covers(w: f32, a: Vertex, b: Vertex) -> bool {
    if w != 0.0 {
        return w > 0.0;
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    // With y running down and the triangle wound clockwise, a top edge runs
    // right along the top and a left edge runs up.
    (dy == 0.0 && dx > 0.0) || dy < 0.0
}

#[cfg(test)]
mod tests {
    use super::span::{DrawPlan, InteriorKernel, TexFormat, U8_TO_F32};
    use super::{
        covers, edge, triangle, triangles, Blend, Channels, Combine, Compare, PixelFormat, States,
        Target, Texture, Vertex, PLANES,
    };

    /// Verbatim unspecialized general rasteriser retained as a test-only
    /// differential oracle for proving every specialized interior-span path
    /// bit-for-bit identical (#254).
    fn ref_across(tex: &Texture<'_>, x: i32) -> usize {
        let w = tex.width.max(1) as i32;
        if (0..w).contains(&x) {
            x as usize
        } else if tex.clamp[0] {
            x.clamp(0, w - 1) as usize
        } else {
            x.rem_euclid(w) as usize
        }
    }

    fn ref_down(tex: &Texture<'_>, y: i32) -> usize {
        let h = tex.height.max(1) as i32;
        if (0..h).contains(&y) {
            y as usize
        } else if tex.clamp[1] {
            y.clamp(0, h - 1) as usize
        } else {
            y.rem_euclid(h) as usize
        }
    }

    fn ref_texel_at(
        tex: &Texture<'_>,
        channels: &Channels,
        x: usize,
        y: usize,
    ) -> ([f32; 4], bool) {
        let at = y * tex.pitch as usize + x * channels.bytes;
        let Some(raw) = tex.bytes.get(at..at + channels.bytes) else {
            return ([0.0; 4], false);
        };
        let value = channels.read(raw);
        let keyed = tex.colour_key == Some(value);
        let colour = match tex.palette {
            Some(palette) => palette[(value & 0xff) as usize].map(|c| f32::from(c) / 255.0),
            None => channels.colour(value),
        };
        (colour, keyed)
    }

    fn ref_bilinear16(tex: &Texture<'_>, channels: &Channels, x: f32, y: f32) -> ([f32; 4], bool) {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, right) = (ref_across(tex, x0), ref_across(tex, x0 + 1));
        let (upper, lower) = (ref_down(tex, y0), ref_down(tex, y0 + 1));
        let pitch = tex.pitch as usize;
        let texel = |x: usize, y: usize| {
            let at = y * pitch + x * 2;
            tex.bytes
                .get(at..at + 2)
                .map_or(0, |b| u32::from(u16::from_le_bytes([b[0], b[1]])))
        };
        let corners = [
            (texel(left, upper), (256 - fx) * (256 - fy)),
            (texel(right, upper), fx * (256 - fy)),
            (texel(left, lower), (256 - fx) * fy),
            (texel(right, lower), fx * fy),
        ];
        let mut sums = [0u32; 4];
        let mut keyed = 0;
        for (value, weight) in corners {
            for (i, sum) in sums.iter_mut().enumerate() {
                *sum += ((value >> channels.shift[i]) & channels.mask[i]) * weight;
            }
            if tex.colour_key == Some(value) {
                keyed += weight;
            }
        }
        let colour = std::array::from_fn(|i| {
            if channels.mask[i] == 0 {
                channels.absent[i]
            } else {
                sums[i] as f32 * channels.step[i] / 65_536.0
            }
        });
        (colour, keyed > 32_768)
    }

    fn ref_sample(
        tex: &Texture<'_>,
        channels: &Channels,
        u: f32,
        v: f32,
        linear: bool,
    ) -> ([f32; 4], bool) {
        let (w, h) = (tex.width as f32, tex.height as f32);
        if linear && channels.bytes == 2 && tex.palette.is_none() {
            return ref_bilinear16(tex, channels, u * w - 0.5, v * h - 0.5);
        }
        if !linear {
            let (x, y) = (
                ref_across(tex, (u * w).floor() as i32),
                ref_down(tex, (v * h).floor() as i32),
            );
            return ref_texel_at(tex, channels, x, y);
        }
        let (x, y) = (u * w - 0.5, v * h - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, right) = (ref_across(tex, x0), ref_across(tex, x0 + 1));
        let (upper, lower) = (ref_down(tex, y0), ref_down(tex, y0 + 1));
        let mut out = [0.0; 4];
        let mut keyed = 0.0;
        for (x, y, weight) in [
            (left, upper, (1.0 - fx) * (1.0 - fy)),
            (right, upper, fx * (1.0 - fy)),
            (left, lower, (1.0 - fx) * fy),
            (right, lower, fx * fy),
        ] {
            let (t, k) = ref_texel_at(tex, channels, x, y);
            for (o, c) in out.iter_mut().zip(t) {
                *o += weight * c;
            }
            if k {
                keyed += weight;
            }
        }
        (out, keyed > 0.5)
    }

    fn triangle_reference(
        target: &mut Target<'_>,
        v: [Vertex; 3],
        texture: Option<&Texture<'_>>,
        states: &States,
    ) {
        let [a, b, c] = v;
        let area = edge(a, b, c.x, c.y);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let (b, c, area) = if area < 0.0 {
            (c, b, -area)
        } else {
            (b, c, area)
        };
        let [clip_left, clip_top, clip_right, clip_bottom] = target.clip;
        let left = (a.x.min(b.x).min(c.x).floor() as i32).max(clip_left).max(0);
        let top = (a.y.min(b.y).min(c.y).floor() as i32)
            .max(clip_top)
            .max(target.first_row as i32);
        let right = (a.x.max(b.x).max(c.x).ceil() as i32)
            .min(clip_right)
            .min(target.width as i32);
        let bottom = (a.y.max(b.y).max(c.y).ceil() as i32)
            .min(clip_bottom)
            .min((target.first_row + target.height) as i32);
        if left >= right || top >= bottom {
            return;
        }
        let edges = [(b, c), (c, a), (a, b)];
        let coefficient = |(p, q): (Vertex, Vertex)| {
            let (dx, dy) = (q.x - p.x, q.y - p.y);
            (-dy, dx, -dx * p.y + dy * p.x)
        };
        let weights = edges.map(coefficient);
        let plane = |values: [f32; 3]| -> (f32, f32, f32) {
            let mut out = (0.0, 0.0, 0.0);
            for ((ex, ey, ek), value) in weights.iter().zip(values) {
                out.0 += ex * value / area;
                out.1 += ey * value / area;
                out.2 += ek * value / area;
            }
            out
        };
        let over_w = |f: fn(&Vertex) -> f32| plane([f(&a) * a.rhw, f(&b) * b.rhw, f(&c) * c.rhw]);
        let depth = if a.z == b.z && b.z == c.z && a.z.is_finite() {
            (0.0, 0.0, a.z)
        } else {
            plane([a.z, b.z, c.z])
        };
        let planes = [
            depth,
            plane([a.rhw, b.rhw, c.rhw]),
            over_w(|v| v.uv[0]),
            over_w(|v| v.uv[1]),
            over_w(|v| v.diffuse[0]),
            over_w(|v| v.diffuse[1]),
            over_w(|v| v.diffuse[2]),
            over_w(|v| v.diffuse[3]),
            over_w(|v| v.specular[0]),
            over_w(|v| v.specular[1]),
            over_w(|v| v.specular[2]),
            over_w(|v| v.fog),
        ];
        let step: [f32; PLANES] = planes.map(|p| p.0);
        let carried = if states.specular || states.fog.is_some() {
            PLANES
        } else if !states.flat {
            8
        } else if texture.is_some() {
            4
        } else {
            2
        };
        let pixel_channels = Channels::of(target.format);
        let texel_channels = texture.map(|t| Channels::of(t.format));
        let bytes = pixel_channels.bytes;
        let flat = (a.diffuse, a.specular);
        let combine = |how: Combine, t: f32, d: f32| match how {
            Combine::Texture => t,
            Combine::Diffuse => d,
            Combine::Modulate => t * d,
            Combine::Add => (t + d).min(1.0),
        };
        for py in top..bottom {
            let y = py as f32;
            let (mut from, mut to) = (left as f32, (right - 1) as f32);
            let mut on_level_edge = false;
            for (ex, ey, ek) in weights {
                let at_row = ey * y + ek;
                if ex > 0.0 {
                    from = from.max(-at_row / ex - 1.0);
                } else if ex < 0.0 {
                    to = to.min(-at_row / ex + 1.0);
                } else if at_row < 0.0 {
                    to = from - 1.0;
                } else if at_row == 0.0 {
                    on_level_edge = true;
                }
            }
            if from.partial_cmp(&to).is_none_or(|order| order.is_gt()) {
                continue;
            }
            let (from, to) = (
                (from.floor() as i32).max(left),
                (to.ceil() as i32).min(right - 1),
            );
            let (inside_from, inside_to) = if on_level_edge {
                (to + 1, to)
            } else {
                (from + 3, to - 3)
            };
            let start = from as f32;
            let mut value: [f32; PLANES] = planes.map(|(px, py_, k)| px * start + py_ * y + k);
            let row = (py as u32 - target.first_row) as usize;
            let row_depth = row * target.width as usize;
            let row_pixels = row * target.pitch as usize;

            for px in from..=to {
                let here = value;
                for (v, s) in value.iter_mut().zip(step).take(carried) {
                    *v += s;
                }
                if px < inside_from || px > inside_to {
                    let x = px as f32;
                    let (w0, w1, w2) = (edge(b, c, x, y), edge(c, a, x, y), edge(a, b, x, y));
                    if !(covers(w0, b, c) && covers(w1, c, a) && covers(w2, a, b)) {
                        continue;
                    }
                }
                let z = here[0];
                let at = row_depth + px as usize;
                if let Some(test) = states.depth_test {
                    if !target.depth.get(at).is_some_and(|d| test.passes(z, *d)) {
                        continue;
                    }
                }
                let near = if here[1] > 0.0 { 1.0 / here[1] } else { 0.0 };
                let (diffuse, specular) = if states.flat {
                    flat
                } else {
                    (
                        [
                            here[4] * near,
                            here[5] * near,
                            here[6] * near,
                            here[7] * near,
                        ],
                        [here[8] * near, here[9] * near, here[10] * near],
                    )
                };
                let (texel, keyed) = match (texture, &texel_channels) {
                    (Some(t), Some(channels)) => {
                        ref_sample(t, channels, here[2] * near, here[3] * near, states.linear)
                    }
                    _ => ([1.0; 4], false),
                };
                if keyed && states.colour_key {
                    continue;
                }
                let mut colour = [
                    combine(states.colour, texel[0], diffuse[0]),
                    combine(states.colour, texel[1], diffuse[1]),
                    combine(states.colour, texel[2], diffuse[2]),
                    combine(states.alpha, texel[3], diffuse[3]),
                ];
                if states.specular {
                    for (c, s) in colour.iter_mut().zip(specular) {
                        *c = (*c + s).min(1.0);
                    }
                }
                if let Some(fog) = states.fog {
                    let f = (here[11] * near).clamp(0.0, 1.0);
                    for (c, air) in colour.iter_mut().zip(fog) {
                        *c = *c * f + air * (1.0 - f);
                    }
                }
                if let Some((test, reference)) = states.alpha_test {
                    let alpha = (colour[3].clamp(0.0, 1.0) * 255.0).round() as u8;
                    if !test.passes(alpha, reference) {
                        continue;
                    }
                }
                let offset = row_pixels + px as usize * bytes;
                let Some(pixel) = target.pixels.get_mut(offset..offset + bytes) else {
                    continue;
                };
                if let Some((source, destination)) = states.blend {
                    let under = pixel_channels.colour(pixel_channels.read(pixel));
                    let (s, d) = (
                        source.factor(colour, under),
                        destination.factor(colour, under),
                    );
                    for i in 0..4 {
                        colour[i] = colour[i] * s[i] + under[i] * d[i];
                    }
                }
                let packed = pixel_channels.value(colour).to_le_bytes();
                match pixel {
                    [lo, hi] => {
                        *lo = packed[0];
                        *hi = packed[1];
                    }
                    _ => pixel.copy_from_slice(&packed[..bytes]),
                }
                if states.depth_write {
                    if let Some(d) = target.depth.get_mut(at) {
                        *d = z;
                    }
                }
            }
        }
    }

    fn at(x: f32, y: f32) -> Vertex {
        Vertex {
            x,
            y,
            z: 0.5,
            rhw: 1.0,
            diffuse: [1.0, 0.0, 0.0, 1.0],
            ..Vertex::default()
        }
    }

    fn drawn(pixels: &[u8], width: usize) -> Vec<String> {
        pixels
            .chunks(width * 2)
            .map(|row| {
                row.chunks(2)
                    .map(|p| if p == [0, 0] { '.' } else { '#' })
                    .collect()
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn assert_bit_exact(
        w: usize,
        h: usize,
        pitch: usize,
        format: PixelFormat,
        init_pixels: &[u8],
        init_depth: &[f32],
        clip: [i32; 4],
        first_row: u32,
        tri: [Vertex; 3],
        texture: Option<&Texture<'_>>,
        states: &States,
        label: &str,
    ) {
        let mut got_pixels = init_pixels.to_vec();
        let mut got_depth = init_depth.to_vec();
        let mut ref_pixels = init_pixels.to_vec();
        let mut ref_depth = init_depth.to_vec();
        let mut got_target = Target {
            pixels: &mut got_pixels,
            width: w as u32,
            height: h as u32,
            pitch: pitch as u32,
            format,
            depth: &mut got_depth,
            clip,
            first_row,
        };
        let mut ref_target = Target {
            pixels: &mut ref_pixels,
            width: w as u32,
            height: h as u32,
            pitch: pitch as u32,
            format,
            depth: &mut ref_depth,
            clip,
            first_row,
        };
        triangle(&mut got_target, tri, texture, states);
        triangle_reference(&mut ref_target, tri, texture, states);
        assert_eq!(got_pixels, ref_pixels, "pixel mismatch for {label}");
        let got_depth_bits: Vec<u32> = got_depth.iter().map(|d| d.to_bits()).collect();
        let ref_depth_bits: Vec<u32> = ref_depth.iter().map(|d| d.to_bits()).collect();
        assert_eq!(got_depth_bits, ref_depth_bits, "depth mismatch for {label}");
    }

    #[test]
    fn u8_to_f32_lut_matches_division_bit_for_bit() {
        for c in 0u8..=255 {
            assert_eq!(
                U8_TO_F32[usize::from(c)].to_bits(),
                (f32::from(c) / 255.0).to_bits(),
                "mismatch at {c}"
            );
        }
    }

    /// Differential proof across every specialized interior kernel: RGB565,
    /// ARGB1555, ARGB4444, RGB555, Paletted8, XRGB8888, ARGB8888; POT wrap,
    /// NPOT wrap, and clamped axes; nearest and bilinear; opaque and blended;
    /// flat, Gouraud, specular, fog, colour key, alpha test, and custom Z;
    /// plus untextured paths and non-RGB565 target fallback.
    #[test]
    fn every_specialized_interior_span_matches_reference_bit_for_bit() {
        const ARGB1555: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x7c00, 0x03e0, 0x001f, 0x8000],
        };
        const ARGB4444: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x0f00, 0x00f0, 0x000f, 0xf000],
        };
        const RGB555: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x7c00, 0x03e0, 0x001f, 0],
        };
        const XRGB8888: PixelFormat = PixelFormat {
            bits: 32,
            masks: [0x00ff_0000, 0xff00, 0xff, 0],
        };
        const ARGB8888: PixelFormat = PixelFormat {
            bits: 32,
            masks: [0x00ff_0000, 0xff00, 0xff, 0xff00_0000],
        };
        const PAL8: PixelFormat = PixelFormat {
            bits: 8,
            masks: [0; 4],
        };

        let mut seed = 0x9e37_79b9u32;
        let mut next_u32 = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        let palette: [[u8; 4]; 256] = std::array::from_fn(|i| {
            [
                (i as u8).wrapping_mul(37),
                (i as u8).wrapping_mul(73).wrapping_add(11),
                (i as u8).wrapping_mul(131).wrapping_add(5),
                if i % 3 == 0 { 255 } else { (i as u8) ^ 0x5a },
            ]
        });

        let (w, h) = (72usize, 24usize);
        // Include extra row padding (pitch > w * 2) to verify row stride math.
        let pitch16 = w * 2 + 8;
        let init_pixels16: Vec<u8> = (0..h * pitch16).map(|_| next_u32() as u8).collect();
        // Mix of near, mid, far, and NaN depths to exercise depth reject/accept.
        let init_depth: Vec<f32> = (0..w * h)
            .map(|i| match i % 11 {
                0 => 0.1,
                1 => f32::NAN,
                2 => 0.5,
                _ => 0.9,
            })
            .collect();

        // Perspective triangle with negative and >1 UVs, varied diffuse/specular/fog.
        let tri = [
            Vertex {
                x: 2.25,
                y: 1.5,
                z: 0.25,
                rhw: 0.8,
                diffuse: [0.91, 0.37, 0.62, 0.73],
                specular: [0.18, 0.44, 0.09],
                uv: [-0.75, -0.4],
                fog: 0.25,
            },
            Vertex {
                x: 69.75,
                y: 4.0,
                z: 0.65,
                rhw: 1.45,
                diffuse: [0.22, 0.88, 0.49, 0.35],
                specular: [0.51, 0.12, 0.33],
                uv: [2.35, 0.85],
                fog: 0.85,
            },
            Vertex {
                x: 11.5,
                y: 22.75,
                z: 0.45,
                rhw: 0.45,
                diffuse: [0.55, 0.61, 0.95, 0.92],
                specular: [0.05, 0.27, 0.66],
                uv: [0.4, 2.6],
                fog: 0.55,
            },
        ];

        // 1. Untextured paths (flat opaque, Gouraud opaque, flat/Gouraud alpha,
        //    plus specular/fog/additive/un-Z-tested).
        let untextured_cases: &[(&str, States, InteriorKernel)] = &[
            (
                "untextured_flat_opaque_z",
                States {
                    flat: true,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    ..States::default()
                },
                InteriorKernel::UntexturedFlatOpaqueZ,
            ),
            (
                "untextured_gouraud_opaque_z",
                States {
                    flat: false,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    ..States::default()
                },
                InteriorKernel::UntexturedGouraudOpaqueZ,
            ),
            (
                "untextured_flat_alpha_z_dw",
                States {
                    flat: true,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    depth_write: true,
                    ..States::default()
                },
                InteriorKernel::UntexturedAlphaZ,
            ),
            (
                "untextured_gouraud_alpha_z_nodw",
                States {
                    flat: false,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    depth_write: false,
                    ..States::default()
                },
                InteriorKernel::UntexturedAlphaZ,
            ),
            (
                "untextured_fast_rgb565_fog_spec_add",
                States {
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    specular: true,
                    fog: Some([0.3, 0.5, 0.7]),
                    blend: Some((Blend::One, Blend::One)),
                    depth_test: None,
                    depth_write: false,
                    ..States::default()
                },
                InteriorKernel::FastRgb565,
            ),
        ];
        for (label, states, expected_kernel) in untextured_cases {
            let plan = DrawPlan::new(PixelFormat::RGB565, None, states);
            assert_eq!(plan.kernel, *expected_kernel, "kernel for {label}");
            assert_bit_exact(
                w,
                h,
                pitch16,
                PixelFormat::RGB565,
                &init_pixels16,
                &init_depth,
                [0, 0, w as i32, h as i32],
                0,
                tri,
                None,
                states,
                label,
            );
        }

        // 2. Textured paths across all formats, POT/NPOT, wrap/clamp, nearest/bilinear.
        let formats: &[(&str, PixelFormat, bool, TexFormat)] = &[
            ("rgb565", PixelFormat::RGB565, false, TexFormat::Rgb565),
            ("argb1555", ARGB1555, false, TexFormat::Argb1555),
            ("argb4444", ARGB4444, false, TexFormat::Argb4444),
            ("rgb555", RGB555, false, TexFormat::Rgb555),
            ("pal8", PAL8, true, TexFormat::Paletted8),
            ("xrgb8888", XRGB8888, false, TexFormat::Xrgb8888),
            ("argb8888", ARGB8888, false, TexFormat::Argb8888),
        ];
        let dims: &[(&str, u32, u32, [bool; 2])] = &[
            ("pot_wrap", 8, 8, [false, false]),
            ("pot_clamp_u", 8, 8, [true, false]),
            ("pot_clamp_both", 8, 8, [true, true]),
            ("npot_wrap", 6, 5, [false, false]),
        ];

        for &(fmt_name, fmt, is_pal, expected_tex_fmt) in formats {
            let bpp = fmt.bits.div_ceil(8) as usize;
            for &(dim_name, tw, th, clamp) in dims {
                let tex_pitch = tw as usize * bpp + 4;
                let tex_bytes: Vec<u8> = (0..th as usize * tex_pitch)
                    .map(|_| next_u32() as u8)
                    .collect();
                let key_val = if bpp == 1 {
                    u32::from(tex_bytes[0])
                } else if bpp == 2 {
                    u32::from(u16::from_le_bytes([tex_bytes[0], tex_bytes[1]]))
                } else {
                    u32::from_le_bytes([tex_bytes[0], tex_bytes[1], tex_bytes[2], tex_bytes[3]])
                };
                let tex = Texture {
                    bytes: &tex_bytes,
                    width: tw,
                    height: th,
                    pitch: tex_pitch as u32,
                    format: fmt,
                    palette: is_pal.then_some(&palette),
                    colour_key: Some(key_val),
                    clamp,
                };
                for linear in [false, true] {
                    let state_variants: Vec<(&str, States)> = vec![
                        (
                            "mod_opaque_gouraud",
                            States {
                                linear,
                                ..States::default()
                            },
                        ),
                        (
                            "mod_opaque_flat",
                            States {
                                linear,
                                flat: true,
                                ..States::default()
                            },
                        ),
                        (
                            "mod_opaque_specular",
                            States {
                                linear,
                                specular: true,
                                ..States::default()
                            },
                        ),
                        (
                            "mod_opaque_fog",
                            States {
                                linear,
                                fog: Some([0.2, 0.4, 0.8]),
                                ..States::default()
                            },
                        ),
                        (
                            "mod_opaque_no_z",
                            States {
                                linear,
                                depth_test: None,
                                depth_write: false,
                                ..States::default()
                            },
                        ),
                        (
                            "copy_opaque_z",
                            States {
                                linear,
                                colour: Combine::Texture,
                                alpha: Combine::Texture,
                                ..States::default()
                            },
                        ),
                        (
                            "copy_opaque_no_z",
                            States {
                                linear,
                                colour: Combine::Texture,
                                alpha: Combine::Texture,
                                depth_test: None,
                                depth_write: false,
                                ..States::default()
                            },
                        ),
                        (
                            "mod_alpha_gouraud_dw",
                            States {
                                linear,
                                blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                                ..States::default()
                            },
                        ),
                        (
                            "mod_alpha_gouraud_specular",
                            States {
                                linear,
                                specular: true,
                                blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                                ..States::default()
                            },
                        ),
                        (
                            "mod_alpha_flat_nodw_texalpha",
                            States {
                                linear,
                                flat: true,
                                alpha: Combine::Texture,
                                blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                                depth_write: false,
                                ..States::default()
                            },
                        ),
                        (
                            "keyed_alphatest_add",
                            States {
                                linear,
                                colour: Combine::Add,
                                alpha: Combine::Modulate,
                                colour_key: true,
                                alpha_test: Some((Compare::GreaterEqual, 96)),
                                blend: Some((Blend::SourceAlpha, Blend::One)),
                                specular: true,
                                fog: Some([0.9, 0.8, 0.7]),
                                ..States::default()
                            },
                        ),
                    ];
                    for (s_name, states) in &state_variants {
                        let label = format!("{fmt_name}/{dim_name}/lin={linear}/{s_name}");
                        let plan = DrawPlan::new(PixelFormat::RGB565, Some(&tex), states);
                        assert_eq!(plan.texture.unwrap().format, expected_tex_fmt, "{label}");
                        assert_bit_exact(
                            w,
                            h,
                            pitch16,
                            PixelFormat::RGB565,
                            &init_pixels16,
                            &init_depth,
                            [0, 0, w as i32, h as i32],
                            0,
                            tri,
                            Some(&tex),
                            states,
                            &label,
                        );
                    }
                }
            }
        }

        // 3. Non-RGB565 target (ARGB8888) exercises the General interior fallback.
        let pitch32 = w * 4;
        let init_pixels32: Vec<u8> = (0..h * pitch32).map(|_| next_u32() as u8).collect();
        let tex_bytes: Vec<u8> = (0..8 * 8 * 2).map(|_| next_u32() as u8).collect();
        let tex = Texture {
            bytes: &tex_bytes,
            width: 8,
            height: 8,
            pitch: 16,
            format: PixelFormat::RGB565,
            palette: None,
            colour_key: None,
            clamp: [false, false],
        };
        let states = States {
            linear: true,
            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
            ..States::default()
        };
        assert_eq!(
            DrawPlan::new(ARGB8888, Some(&tex), &states).kernel,
            InteriorKernel::General
        );
        assert_bit_exact(
            w,
            h,
            pitch32,
            ARGB8888,
            &init_pixels32,
            &init_depth,
            [0, 0, w as i32, h as i32],
            0,
            tri,
            Some(&tex),
            &states,
            "argb8888_target_general_fallback",
        );

        // 4. Truncated target & truncated texture buffers match reference checked behaviour.
        let short_pixels = vec![0x33u8; 40];
        let short_depth = vec![0.8f32; 20];
        let short_tex = Texture {
            bytes: &tex_bytes[..10],
            width: 8,
            height: 8,
            pitch: 16,
            format: PixelFormat::RGB565,
            palette: None,
            colour_key: None,
            clamp: [false, false],
        };
        assert_bit_exact(
            w,
            h,
            pitch16,
            PixelFormat::RGB565,
            &short_pixels,
            &short_depth,
            [0, 0, w as i32, h as i32],
            0,
            tri,
            Some(&short_tex),
            &States::default(),
            "truncated_target_and_texture",
        );
    }

    /// Sweep span widths `0..=65` (hitting zero-interior, 1-pixel interior,
    /// SIMD-aligned multiples of 4, and unaligned remainders) plus non-zero
    /// `first_row` and sub-rect `clip`, asserting bit-exact parity with the
    /// general reference loop.
    #[test]
    fn span_widths_0_to_65_match_reference_bit_for_bit() {
        let (w, h) = (80usize, 12usize);
        let pitch = w * 2;
        let tex_bytes: Vec<u8> = (0..16 * 16 * 2)
            .map(|i| ((i * 53 + 17) & 0xff) as u8)
            .collect();
        let tex = Texture {
            bytes: &tex_bytes,
            width: 16,
            height: 16,
            pitch: 32,
            format: PixelFormat::RGB565,
            palette: None,
            colour_key: None,
            clamp: [false, false],
        };
        let init_pixels: Vec<u8> = (0..h * pitch)
            .map(|i| ((i * 29 + 3) & 0xff) as u8)
            .collect();
        let init_depth = vec![0.75f32; w * h];

        let modes = [
            States::default(),
            States {
                linear: true,
                ..States::default()
            },
            States {
                linear: true,
                blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                ..States::default()
            },
            States {
                flat: true,
                colour: Combine::Diffuse,
                alpha: Combine::Diffuse,
                ..States::default()
            },
        ];

        for span_w in 0..=65usize {
            let x0 = 5.0f32;
            let x1 = x0 + span_w as f32;
            // Two triangles covering a rectangle of width `span_w` across rows 1..9
            // (with sloped edges on one side so interior spans span exact widths).
            let tri = [
                Vertex {
                    x: x0,
                    y: 1.0,
                    z: 0.2,
                    rhw: 0.9,
                    diffuse: [0.8, 0.4, 0.2, 0.6],
                    uv: [0.0, 0.0],
                    ..Vertex::default()
                },
                Vertex {
                    x: x1,
                    y: 1.25,
                    z: 0.6,
                    rhw: 1.3,
                    diffuse: [0.2, 0.9, 0.7, 0.4],
                    uv: [1.75, 0.25],
                    ..Vertex::default()
                },
                Vertex {
                    x: x0 + 0.5,
                    y: 9.5,
                    z: 0.4,
                    rhw: 0.7,
                    diffuse: [0.5, 0.5, 0.9, 0.8],
                    uv: [0.1, 1.5],
                    ..Vertex::default()
                },
            ];
            for (m_idx, states) in modes.iter().enumerate() {
                let use_tex = if m_idx == 3 { None } else { Some(&tex) };
                assert_bit_exact(
                    w,
                    h,
                    pitch,
                    PixelFormat::RGB565,
                    &init_pixels,
                    &init_depth,
                    [2, 2, (w - 2) as i32, (h - 1) as i32],
                    1,
                    tri,
                    use_tex,
                    states,
                    &format!("width={span_w}/mode={m_idx}"),
                );
            }
        }
    }

    /// Edge-case floating-point inputs: zero/negative `rhw`, NaN depth, huge UVs.
    #[test]
    fn edge_case_rhw_and_nan_values_match_reference_bit_for_bit() {
        let (w, h) = (24usize, 12usize);
        let pitch = w * 2;
        let tex_bytes: Vec<u8> = (0..8 * 8 * 2).map(|i| (i * 41) as u8).collect();
        let tex = Texture {
            bytes: &tex_bytes,
            width: 8,
            height: 8,
            pitch: 16,
            format: PixelFormat::RGB565,
            palette: None,
            colour_key: None,
            clamp: [false, false],
        };
        let init_pixels = vec![0x42u8; h * pitch];
        let init_depth = vec![0.8f32; w * h];
        // rhw crosses zero across the triangle so `here[1] <= 0.0` triggers mid-span.
        let tri = [
            Vertex {
                x: 1.0,
                y: 1.0,
                z: 0.3,
                rhw: -0.5,
                diffuse: [0.8, 0.2, 0.5, 0.7],
                uv: [-100.25, 50.75],
                ..Vertex::default()
            },
            Vertex {
                x: 22.0,
                y: 2.0,
                z: 0.5,
                rhw: 1.5,
                diffuse: [0.1, 0.9, 0.3, 0.4],
                uv: [200.5, -75.0],
                ..Vertex::default()
            },
            Vertex {
                x: 4.0,
                y: 11.0,
                z: 0.4,
                rhw: 0.0,
                diffuse: [0.6, 0.4, 0.9, 0.9],
                uv: [0.0, 0.0],
                ..Vertex::default()
            },
        ];
        for linear in [false, true] {
            for blend in [None, Some((Blend::SourceAlpha, Blend::InverseSourceAlpha))] {
                let states = States {
                    linear,
                    blend,
                    ..States::default()
                };
                assert_bit_exact(
                    w,
                    h,
                    pitch,
                    PixelFormat::RGB565,
                    &init_pixels,
                    &init_depth,
                    [0, 0, w as i32, h as i32],
                    0,
                    tri,
                    Some(&tex),
                    &states,
                    &format!("zero_neg_rhw/lin={linear}/blend={}", blend.is_some()),
                );
            }
        }
    }

    /// Quads at one depth, overlapping under `LESSEQUAL`: the later one covers
    /// the earlier wherever on the screen it lies, as on a card (#210). LEGO
    /// Chess's glyphs are such quads, at a third. With an ulp of interpolated
    /// depth across each, 215 of these 270 placements lost pixels. The rows are
    /// near those of the New Player form's lines of text, across the middle of
    /// the screen.
    #[test]
    fn a_quad_at_one_depth_covers_another_at_it_wherever_it_lies() {
        let (w, h) = (640, 480);
        let mut pixels = vec![0u8; w * h * 2];
        let mut depth = vec![1.0f32; w * h];
        let quad = |x: f32, y: f32, blue: bool| {
            let colour = if blue {
                [0.0, 0.0, 1.0, 1.0]
            } else {
                [1.0, 0.0, 0.0, 1.0]
            };
            let v = |x, y| Vertex {
                z: 1.0 / 3.0,
                diffuse: colour,
                ..at(x, y)
            };
            [
                [v(x, y), v(x + 16.0, y), v(x, y + 16.0)],
                [v(x, y + 16.0), v(x + 16.0, y), v(x + 16.0, y + 16.0)],
            ]
        };
        let mut lost = Vec::new();
        for y in [100, 112, 192, 292, 322] {
            for x in (240..400).step_by(3) {
                pixels.fill(0);
                depth.fill(1.0);
                let mut target = Target {
                    pixels: &mut pixels,
                    width: w as u32,
                    height: h as u32,
                    pitch: (w * 2) as u32,
                    format: PixelFormat::RGB565,
                    depth: &mut depth,
                    clip: [0, 0, w as i32, h as i32],
                    first_row: 0,
                };
                for (left, blue) in [(x, false), (x + 8, true)] {
                    for t in quad(left as f32, y as f32, blue) {
                        triangle(&mut target, t, None, &States::default());
                    }
                }
                let covered = (y..y + 16).all(|row| {
                    (x + 8..x + 24).all(|column| {
                        let p = (row * w + column) * 2;
                        pixels[p..p + 2] == [0x1f, 0x00] && depth[row * w + column] == 1.0 / 3.0
                    })
                });
                if !covered {
                    lost.push((x, y));
                }
            }
        }
        assert!(
            lost.is_empty(),
            "{} placements lost pixels: {lost:?}",
            lost.len()
        );
    }

    #[test]
    fn a_pixel_format_narrows_its_channels() {
        let rgb565 = PixelFormat::RGB565;
        assert_eq!(rgb565.encode([255, 255, 255, 255]), 0xffff);
        assert_eq!(rgb565.encode([0, 128, 0, 255]), 0x0400);
    }

    #[test]
    fn two_triangles_sharing_an_edge_cover_a_square_once() {
        let (w, h) = (6, 6);
        let mut pixels = vec![0u8; w * h * 2];
        let mut depth = vec![1.0f32; w * h];
        let mut target = Target {
            pixels: &mut pixels,
            width: w as u32,
            height: h as u32,
            pitch: (w * 2) as u32,
            format: PixelFormat::RGB565,
            depth: &mut depth,
            clip: [0, 0, 6, 6],
            first_row: 0,
        };
        let blend = States {
            // Adding onto what is there would show a pixel drawn twice.
            blend: Some((Blend::One, Blend::One)),
            ..States::default()
        };
        let quarter = Vertex {
            diffuse: [0.5, 0.0, 0.0, 1.0],
            ..at(1.0, 1.0)
        };
        let corner = |x, y| Vertex { x, y, ..quarter };
        triangle(
            &mut target,
            [corner(1.0, 1.0), corner(5.0, 1.0), corner(5.0, 5.0)],
            None,
            &blend,
        );
        triangle(
            &mut target,
            [corner(1.0, 1.0), corner(5.0, 5.0), corner(1.0, 5.0)],
            None,
            &blend,
        );
        assert_eq!(
            drawn(&pixels, w),
            ["......", ".####.", ".####.", ".####.", ".####.", "......"]
        );
        // Every pixel was drawn once: half red, not all of it.
        assert!(pixels.chunks(2).all(|p| p == [0, 0] || p == [0, 0x80]));
    }

    #[test]
    fn a_nearer_triangle_hides_a_farther_one_whichever_is_drawn_first() {
        let mut pixels = vec![0u8; 4 * 4 * 2];
        let mut depth = vec![1.0f32; 16];
        let mut target = Target {
            pixels: &mut pixels,
            width: 4,
            height: 4,
            pitch: 8,
            format: PixelFormat::RGB565,
            depth: &mut depth,
            clip: [0, 0, 4, 4],
            first_row: 0,
        };
        let square = |z: f32, colour: [f32; 4]| {
            let v = |x, y| Vertex {
                z,
                diffuse: colour,
                ..at(x, y)
            };
            [
                [v(0.0, 0.0), v(4.0, 0.0), v(4.0, 4.0)],
                [v(0.0, 0.0), v(4.0, 4.0), v(0.0, 4.0)],
            ]
        };
        for tri in square(0.2, [0.0, 0.0, 1.0, 1.0]) {
            triangle(&mut target, tri, None, &States::default());
        }
        for tri in square(0.7, [1.0, 0.0, 0.0, 1.0]) {
            triangle(&mut target, tri, None, &States::default());
        }
        assert!(
            target.pixels.chunks(2).all(|p| p == [0x1f, 0x00]),
            "near first: all blue"
        );
        // Far first, then near: the near one still wins.
        target.depth.fill(1.0);
        for tri in square(0.7, [1.0, 0.0, 0.0, 1.0]) {
            triangle(&mut target, tri, None, &States::default());
        }
        for tri in square(0.2, [0.0, 0.0, 1.0, 1.0]) {
            triangle(&mut target, tri, None, &States::default());
        }
        assert!(
            pixels.chunks(2).all(|p| p == [0x1f, 0x00]),
            "far first: all blue"
        );
    }

    #[test]
    fn a_texture_is_sampled_where_the_triangle_is_and_its_colour_key_is_not_drawn() {
        // Two texels across: white, then the key, black.
        let texels = [0xff, 0xff, 0x00, 0x00];
        let texture = Texture {
            bytes: &texels,
            width: 2,
            height: 1,
            pitch: 4,
            format: PixelFormat::RGB565,
            palette: None,
            colour_key: Some(0),
            clamp: [false; 2],
        };
        let mut pixels = vec![0x55u8; 4 * 2 * 2];
        let mut depth = vec![1.0f32; 8];
        let mut target = Target {
            pixels: &mut pixels,
            width: 4,
            height: 2,
            pitch: 8,
            format: PixelFormat::RGB565,
            depth: &mut depth,
            clip: [0, 0, 4, 2],
            first_row: 0,
        };
        let v = |x, y, u| Vertex {
            diffuse: [1.0; 4],
            uv: [u, 0.0],
            ..at(x, y)
        };
        let states = States {
            colour_key: true,
            ..States::default()
        };
        triangle(
            &mut target,
            [v(0.0, 0.0, 0.0), v(4.0, 0.0, 1.0), v(4.0, 2.0, 1.0)],
            Some(&texture),
            &states,
        );
        triangle(
            &mut target,
            [v(0.0, 0.0, 0.0), v(4.0, 2.0, 1.0), v(0.0, 2.0, 0.0)],
            Some(&texture),
            &states,
        );
        // The left half is the white texel; the right half is keyed out and
        // what was under it stays.
        for row in pixels.chunks(8) {
            assert_eq!(row, [0xff, 0xff, 0xff, 0xff, 0x55, 0x55, 0x55, 0x55]);
        }
    }

    /// Fog takes a pixel towards the fog's colour by as much as the vertices'
    /// own factor says, and leaves it alone where the factor is one (#61).
    #[test]
    fn a_pixel_is_mixed_with_the_fog_by_the_factor_its_vertices_carry() {
        let mut pixels = [0u8; 4 * 2];
        let mut depth = [1.0f32; 4];
        let drawn_with = |fog: f32, pixels: &mut [u8], depth: &mut [f32]| {
            let mut target = Target {
                pixels,
                width: 4,
                height: 1,
                pitch: 8,
                format: PixelFormat::RGB565,
                depth,
                clip: [0, 0, 4, 1],
                first_row: 0,
            };
            // Red vertices under white fog.
            let v = |x: f32, y: f32| Vertex { fog, ..at(x, y) };
            triangle(
                &mut target,
                [v(0.0, 0.0), v(4.0, 0.0), v(4.0, 4.0)],
                None,
                &States {
                    fog: Some([1.0, 1.0, 1.0]),
                    ..States::default()
                },
            );
        };
        drawn_with(1.0, &mut pixels, &mut depth);
        assert_eq!(pixels[0..2], [0x00, 0xf8], "clear air leaves the red alone");

        let mut pixels = [0u8; 4 * 2];
        let mut depth = [1.0f32; 4];
        drawn_with(0.0, &mut pixels, &mut depth);
        assert_eq!(pixels[0..2], [0xff, 0xff], "no factor at all is the fog");
    }

    #[test]
    fn a_draw_in_bands_is_the_same_as_one_triangle_after_another() {
        let (w, h) = (320usize, 240usize);
        let tex_bytes: Vec<u8> = (0..16 * 16 * 2)
            .map(|i| ((i * 71 + 19) & 0xff) as u8)
            .collect();
        let tex = Texture {
            bytes: &tex_bytes,
            width: 16,
            height: 16,
            pitch: 32,
            format: PixelFormat::RGB565,
            palette: None,
            colour_key: None,
            clamp: [false, false],
        };
        let mut seed = 12_345u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) as f32 / (1 << 24) as f32
        };
        let list: Vec<[Vertex; 3]> = (0..40)
            .map(|_| {
                let colour = [next(), next(), next(), next()];
                std::array::from_fn(|_| Vertex {
                    x: next() * 360.0 - 20.0,
                    y: next() * 280.0 - 20.0,
                    z: next(),
                    rhw: 0.5 + next(),
                    diffuse: colour,
                    uv: [next() * 2.0, next() * 2.0],
                    ..Vertex::default()
                })
            })
            .collect();
        let cases: &[(&str, Option<&Texture<'_>>, States)] = &[
            (
                "untextured_blended",
                None,
                States {
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    depth_test: Some(Compare::LessEqual),
                    ..States::default()
                },
            ),
            (
                "textured_bilinear_blended",
                Some(&tex),
                States {
                    linear: true,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    depth_test: Some(Compare::LessEqual),
                    ..States::default()
                },
            ),
            (
                "textured_bilinear_opaque",
                Some(&tex),
                States {
                    linear: true,
                    depth_test: Some(Compare::LessEqual),
                    ..States::default()
                },
            ),
        ];
        for (name, texture, states) in cases {
            let draw = |banded: bool| {
                let mut pixels = vec![0u8; w * h * 2];
                let mut depth = vec![1.0f32; w * h];
                let mut target = Target {
                    pixels: &mut pixels,
                    width: w as u32,
                    height: h as u32,
                    pitch: (w * 2) as u32,
                    format: PixelFormat::RGB565,
                    depth: &mut depth,
                    clip: [0, 0, w as i32, h as i32],
                    first_row: 0,
                };
                if banded {
                    triangles(&mut target, &list, *texture, states);
                } else {
                    for t in &list {
                        triangle(&mut target, *t, *texture, states);
                    }
                }
                (pixels, depth)
            };
            let (one_by_one, banded) = (draw(false), draw(true));
            assert!(
                one_by_one.0.iter().any(|b| *b != 0),
                "something is drawn ({name})"
            );
            assert!(
                one_by_one == banded,
                "bands draw every pixel the same ({name})"
            );
        }
    }
}
