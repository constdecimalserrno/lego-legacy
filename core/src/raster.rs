//! Triangles into a surface's own bytes (ADR-0004: one thread, no host GPU).
//!
//! What Direct3D's fixed pipeline does once a triangle is on the screen:
//! which pixels it covers, how deep each is, what colour and texel each takes,
//! and how that lands in a surface laid out however the guest asked for it.
//! Nothing here knows about the guest: the caller hands over the surface's
//! bytes, the depth buffer, and a texture's bytes.

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

impl Texture<'_> {
    /// The texel at a position, wrapped or clamped, from 0 to 1, and whether
    /// it is the colour key.
    fn texel(&self, channels: &Channels, x: i32, y: i32) -> ([f32; 4], bool) {
        let (x, y) = (self.across(x), self.down(y));
        self.texel_at(channels, x, y)
    }

    /// A column, wrapped round or clamped to the texture.
    fn across(&self, x: i32) -> usize {
        let w = self.width.max(1) as i32;
        if (0..w).contains(&x) {
            x as usize
        } else if self.clamp[0] {
            x.clamp(0, w - 1) as usize
        } else {
            x.rem_euclid(w) as usize
        }
    }

    /// A row, wrapped round or clamped to the texture.
    fn down(&self, y: i32) -> usize {
        let h = self.height.max(1) as i32;
        if (0..h).contains(&y) {
            y as usize
        } else if self.clamp[1] {
            y.clamp(0, h - 1) as usize
        } else {
            y.rem_euclid(h) as usize
        }
    }

    /// Bilinear filtering of a sixteen-bit texture with no palette, weighed in
    /// whole numbers: the same four texels and weights as the general path,
    /// in less work.
    fn bilinear16(&self, channels: &Channels, x: f32, y: f32) -> ([f32; 4], bool) {
        let (x0, y0) = (x.floor(), y.floor());
        // Weights in 256ths.
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, right) = (self.across(x0), self.across(x0 + 1));
        let (upper, lower) = (self.down(y0), self.down(y0 + 1));
        let pitch = self.pitch as usize;
        let texel = |x: usize, y: usize| {
            let at = y * pitch + x * 2;
            self.bytes
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
            if self.colour_key == Some(value) {
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

    /// The texel at a column and row already in the texture.
    fn texel_at(&self, channels: &Channels, x: usize, y: usize) -> ([f32; 4], bool) {
        let at = y * self.pitch as usize + x * channels.bytes;
        let Some(raw) = self.bytes.get(at..at + channels.bytes) else {
            return ([0.0; 4], false);
        };
        let value = channels.read(raw);
        let keyed = self.colour_key == Some(value);
        let colour = match self.palette {
            Some(palette) => palette[(value & 0xff) as usize].map(|c| f32::from(c) / 255.0),
            None => channels.colour(value),
        };
        (colour, keyed)
    }

    /// Sample at texture coordinates, nearest or bilinear.
    fn sample(&self, channels: &Channels, u: f32, v: f32, linear: bool) -> ([f32; 4], bool) {
        let (w, h) = (self.width as f32, self.height as f32);
        if linear && channels.bytes == 2 && self.palette.is_none() {
            return self.bilinear16(channels, u * w - 0.5, v * h - 0.5);
        }
        if !linear {
            return self.texel(channels, (u * w).floor() as i32, (v * h).floor() as i32);
        }
        let (x, y) = (u * w - 0.5, v * h - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, right) = (self.across(x0), self.across(x0 + 1));
        let (upper, lower) = (self.down(y0), self.down(y0 + 1));
        let mut out = [0.0; 4];
        let mut keyed = 0.0;
        for (x, y, weight) in [
            (left, upper, (1.0 - fx) * (1.0 - fy)),
            (right, upper, fx * (1.0 - fy)),
            (left, lower, (1.0 - fx) * fy),
            (right, lower, fx * fy),
        ] {
            let (t, k) = self.texel_at(channels, x, y);
            for (o, c) in out.iter_mut().zip(t) {
                *o += weight * c;
            }
            if k {
                keyed += weight;
            }
        }
        (out, keyed > 0.5)
    }
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
/// Each row is walked only across the span the triangle can cover, and what
/// is interpolated steps along the row: depth, the reciprocal of distance,
/// and the colours and texture coordinates weighted by it, so that they are
/// straight in the scene rather than on the screen.
///
/// **#73 asked for that shape and this already had it**, and the measurements
/// say the rest is not worth changing. The three edge functions are evaluated
/// only at the three pixels either end of a span, not per pixel; the
/// texture's channels are worked out once per draw, and a sixteen-bit
/// bilinear sample has a path of its own. What is left per pixel is the
/// perspective divide, the attribute stepping, the texel fetch and the blend.
///
/// Measured over Alpha Team's menu, an eight-second `sample`, main thread:
/// this function and everything inlined into it is **10.4%**, `texel_at`
/// another **1.6%**, and the banded path 4.9% on top. Of that, the attribute
/// stepping — the one thing left that a different shape could remove — is
/// **1.8% of the whole run**, measured by making the loop step twice. A
/// rewrite that has to leave every golden hash unchanged is not worth two
/// percent.
pub fn triangle(
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
    // Depth, the reciprocal of distance, texture coordinates, diffuse and
    // specular colour.
    let planes = [
        plane([a.z, b.z, c.z]),
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
    const PLANES: usize = 12;
    let step: [f32; PLANES] = planes.map(|p| p.0);
    // Only the planes this draw reads are carried along a row: depth and
    // distance always; texture coordinates when there is a texture; colour
    // when it is not flat; and specular when it is added.
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
                    t.sample(channels, here[2] * near, here[3] * near, states.linear)
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
            // Fog, over the colour the texture and the lights made and under
            // the alpha test, which is where Direct3D puts it.
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
    if area < WORTH_BANDS || bottom - top < BANDS as i32 * 2 {
        for triangle_ in list {
            triangle(target, *triangle_, texture, states);
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
                    triangle(&mut band, *triangle_, texture, states);
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
    use super::{triangle, Blend, PixelFormat, States, Target, Texture, Vertex};

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
        use super::{triangles, Blend, Compare};
        let (w, h) = (320usize, 240usize);
        // Blended, depth-tested and overlapping, so the order each pixel's
        // triangles are drawn in shows in the result.
        let states = States {
            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
            depth_test: Some(Compare::LessEqual),
            ..States::default()
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
                    rhw: 1.0,
                    diffuse: colour,
                    ..Vertex::default()
                })
            })
            .collect();
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
                triangles(&mut target, &list, None, &states);
            } else {
                for t in &list {
                    triangle(&mut target, *t, None, &states);
                }
            }
            (pixels, depth)
        };
        let (one_by_one, banded) = (draw(false), draw(true));
        assert!(one_by_one.0.iter().any(|b| *b != 0), "something is drawn");
        assert!(one_by_one == banded, "bands draw every pixel the same");
    }
}
