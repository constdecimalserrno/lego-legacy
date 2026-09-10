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

    fn read(self, at: &[u8]) -> u32 {
        at.iter()
            .take(self.bytes())
            .rev()
            .fold(0, |v, b| (v << 8) | u32::from(*b))
    }

    fn write(self, at: &mut [u8], value: u32) {
        for (i, b) in at.iter_mut().take(self.bytes()).enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
    }

    /// A pixel's channels, each widened to eight bits.
    #[must_use]
    pub fn decode(self, value: u32) -> [u8; 4] {
        let channel = |mask: u32, absent: u8| {
            if mask == 0 {
                return absent;
            }
            let v = u64::from((value & mask) >> mask.trailing_zeros());
            let full = (1u64 << mask.count_ones()) - 1;
            // Full scale in the format is full scale in eight bits.
            ((v * 255 + full / 2) / full) as u8
        };
        [
            channel(self.masks[0], 0),
            channel(self.masks[1], 0),
            channel(self.masks[2], 0),
            channel(self.masks[3], 0xff),
        ]
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
}

impl Texture<'_> {
    /// The texel at a position, wrapped, and whether it is the colour key.
    fn texel(&self, x: i32, y: i32) -> ([u8; 4], bool) {
        let (w, h) = (self.width.max(1) as i32, self.height.max(1) as i32);
        let (x, y) = (x.rem_euclid(w) as usize, y.rem_euclid(h) as usize);
        let bytes = self.format.bytes();
        let at = y * self.pitch as usize + x * bytes;
        let Some(raw) = self.bytes.get(at..at + bytes) else {
            return ([0; 4], false);
        };
        let value = self.format.read(raw);
        let keyed = self.colour_key == Some(value);
        let rgba = match self.palette {
            Some(palette) => palette[(value & 0xff) as usize],
            None => self.format.decode(value),
        };
        (rgba, keyed)
    }

    /// Sample at texture coordinates, nearest or bilinear.
    fn sample(&self, u: f32, v: f32, linear: bool) -> ([f32; 4], bool) {
        let (w, h) = (self.width as f32, self.height as f32);
        if !linear {
            let (t, keyed) = self.texel((u * w).floor() as i32, (v * h).floor() as i32);
            return (t.map(|c| f32::from(c) / 255.0), keyed);
        }
        let (x, y) = (u * w - 0.5, v * h - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let mut out = [0.0; 4];
        let mut keyed = 0.0;
        for (dx, dy, weight) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let (t, k) = self.texel(x0 + dx, y0 + dy);
            for (o, c) in out.iter_mut().zip(t) {
                *o += weight * f32::from(c) / 255.0;
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
        }
    }
}

/// Draw one triangle.
///
/// Pixels whose centres fall inside it are drawn; an edge shared by two
/// triangles belongs to whichever it is the top or left edge of, so neither
/// draws it twice nor leaves a gap.
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
    let top = (a.y.min(b.y).min(c.y).floor() as i32).max(clip_top).max(0);
    let right = (a.x.max(b.x).max(c.x).ceil() as i32)
        .min(clip_right)
        .min(target.width as i32);
    let bottom = (a.y.max(b.y).max(c.y).ceil() as i32)
        .min(clip_bottom)
        .min(target.height as i32);

    let bytes = target.format.bytes();
    let flat = (a.diffuse, a.specular);
    for py in top..bottom {
        for px in left..right {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let (w0, w1, w2) = (edge(b, c, x, y), edge(c, a, x, y), edge(a, b, x, y));
            if !(covers(w0, b, c) && covers(w1, c, a) && covers(w2, a, b)) {
                continue;
            }
            let (l0, l1, l2) = (w0 / area, w1 / area, w2 / area);
            let z = l0 * a.z + l1 * b.z + l2 * c.z;
            let at = (py as u32 * target.width + px as u32) as usize;
            if let Some(test) = states.depth_test {
                if !target.depth.get(at).is_some_and(|d| test.passes(z, *d)) {
                    continue;
                }
            }

            // Colours and texture coordinates go straight in the scene, not
            // on the screen: weighted by each vertex's nearness.
            let (p0, p1, p2) = (l0 * a.rhw, l1 * b.rhw, l2 * c.rhw);
            let sum = p0 + p1 + p2;
            let (p0, p1, p2) = if sum > 0.0 {
                (p0 / sum, p1 / sum, p2 / sum)
            } else {
                (l0, l1, l2)
            };
            let mix = |f: fn(&Vertex) -> f32| p0 * f(&a) + p1 * f(&b) + p2 * f(&c);
            let (diffuse, specular) = if states.flat {
                flat
            } else {
                (
                    [
                        mix(|v| v.diffuse[0]),
                        mix(|v| v.diffuse[1]),
                        mix(|v| v.diffuse[2]),
                        mix(|v| v.diffuse[3]),
                    ],
                    [
                        mix(|v| v.specular[0]),
                        mix(|v| v.specular[1]),
                        mix(|v| v.specular[2]),
                    ],
                )
            };

            let (texel, keyed) = match texture {
                Some(t) => t.sample(mix(|v| v.uv[0]), mix(|v| v.uv[1]), states.linear),
                None => ([1.0; 4], false),
            };
            if keyed && states.colour_key {
                continue;
            }
            let combine = |how: Combine, t: f32, d: f32| match how {
                Combine::Texture => t,
                Combine::Diffuse => d,
                Combine::Modulate => t * d,
                Combine::Add => (t + d).min(1.0),
            };
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
            if let Some((test, reference)) = states.alpha_test {
                let alpha = (colour[3].clamp(0.0, 1.0) * 255.0).round() as u8;
                if !test.passes(alpha, reference) {
                    continue;
                }
            }

            let offset = py as usize * target.pitch as usize + px as usize * bytes;
            let Some(pixel) = target.pixels.get_mut(offset..offset + bytes) else {
                continue;
            };
            if let Some((source, destination)) = states.blend {
                let under = target
                    .format
                    .decode(target.format.read(pixel))
                    .map(|c| f32::from(c) / 255.0);
                let (s, d) = (
                    source.factor(colour, under),
                    destination.factor(colour, under),
                );
                for i in 0..4 {
                    colour[i] = colour[i] * s[i] + under[i] * d[i];
                }
            }
            let rgba = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            target.format.write(pixel, target.format.encode(rgba));
            if states.depth_write {
                if let Some(d) = target.depth.get_mut(at) {
                    *d = z;
                }
            }
        }
    }
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
    fn a_pixel_format_widens_and_narrows_its_channels() {
        let rgb565 = PixelFormat::RGB565;
        assert_eq!(rgb565.decode(0xffff), [255, 255, 255, 255]);
        assert_eq!(rgb565.decode(0xf800), [255, 0, 0, 255]);
        assert_eq!(rgb565.encode([255, 255, 255, 255]), 0xffff);
        assert_eq!(rgb565.encode([0, 128, 0, 255]), 0x0400);
        let argb1555 = PixelFormat {
            bits: 16,
            masks: [0x7c00, 0x03e0, 0x001f, 0x8000],
        };
        assert_eq!(argb1555.decode(0x7fff), [255, 255, 255, 0]);
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
        assert!(pixels.chunks(2).all(|p| p == [0x1f, 0x00]), "all blue");
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
}
