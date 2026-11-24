//! Specialized interior-span kernels for the software rasteriser (#254).
//!
//! After `triangle_with_plan` trims each scanline to its exact covered pixel
//! interval `[cov_from..=cov_to]` and verifies target bounds once per row,
//! every covered pixel enters this module through [`draw_interior`].
//!
//! Hot `States` × `PixelFormat` combinations resolve once per draw to a
//! single monomorphized span function pointer with compile-time channel bit
//! shifts and power-of-two texture wrap bitmasking, eliminating per-scanline
//! dispatch and per-pixel render-state branching. Uncommon states fall back to
//! the fast RGB565 or arbitrary-format loops. Downstream SIMD backends (#255)
//! plug into this same slice-in/slice-out seam beside the canonical Rust
//! kernels.
//!
//! Determinism invariants preserved bit-for-bit with the general loop:
//! - Sequential plane stepping (`value[k] += step[k]`) on every pixel;
//! - Exact IEEE-754 `1.0 / here[1]` perspective division (no approximate rcp);
//! - Exact Q16 integer weights in `bilinear16`;
//! - Separate multiply and add in blending and fog (no FMA);
//! - Exact RGB565 rounding `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32`.

use super::{Blend, Channels, Combine, Compare, PixelFormat, States, Texture, PLANES};

const INV_15: f32 = 1.0 / 15.0;
const INV_31: f32 = 1.0 / 31.0;
const INV_63: f32 = 1.0 / 63.0;
const INV_255: f32 = 1.0 / 255.0;

/// Exact `f32::from(c) / 255.0` lookup table for 8-bit paletted texels.
/// Avoids four floating-point divisions per texel fetch while remaining
/// bit-for-bit identical to `f32::from(c) / 255.0`.
pub(super) const U8_TO_F32: [f32; 256] = {
    let mut table = [0.0f32; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = i as f32 / 255.0;
        i += 1;
    }
    table
};

/// Uniform signature for every interior-span kernel behind this seam (#254, #255).
type SpanFn = fn(
    &DrawPlan<'_>,
    &mut [f32],
    &mut [u8],
    &mut [f32; PLANES],
    &[f32; PLANES],
    ([f32; 4], [f32; 3]),
    &States,
);

#[inline(always)]
fn unpack_rgb565(lo: u8, hi: u8) -> [f32; 4] {
    let v = u32::from(u16::from_le_bytes([lo, hi]));
    [
        ((v >> 11) & 0x1f) as f32 * INV_31,
        ((v >> 5) & 0x3f) as f32 * INV_63,
        (v & 0x1f) as f32 * INV_31,
        1.0,
    ]
}

#[inline(always)]
fn pack_rgb565_rgb(r: f32, g: f32, b: f32) -> [u8; 2] {
    let r = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let g = (g.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    let b = (b.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    ((((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)) as u16).to_le_bytes()
}

#[inline(always)]
pub(super) fn combine_channel(how: Combine, t: f32, d: f32) -> f32 {
    match how {
        Combine::Texture => t,
        Combine::Diffuse => d,
        Combine::Modulate => t * d,
        Combine::Add => (t + d).min(1.0),
    }
}

/// Texture pixel layout classified once per draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TexFormat {
    Rgb565,
    Argb1555,
    Argb4444,
    Rgb555,
    Paletted8,
    Xrgb8888,
    Argb8888,
    General,
}

impl TexFormat {
    fn classify(tex: &Texture<'_>, channels: &Channels) -> Self {
        if tex.palette.is_some() {
            return if channels.bytes == 1 {
                Self::Paletted8
            } else {
                Self::General
            };
        }
        match (tex.format.bits, tex.format.masks) {
            (16, [0xf800, 0x07e0, 0x001f, 0]) => Self::Rgb565,
            (16, [0x7c00, 0x03e0, 0x001f, 0x8000]) => Self::Argb1555,
            (16, [0x0f00, 0x00f0, 0x000f, 0xf000]) => Self::Argb4444,
            (16, [0x7c00, 0x03e0, 0x001f, 0]) => Self::Rgb555,
            (32, [0x00ff_0000, 0xff00, 0xff, 0]) => Self::Xrgb8888,
            (32, [0x00ff_0000, 0xff00, 0xff, 0xff00_0000]) => Self::Argb8888,
            _ => Self::General,
        }
    }
}

/// Per-axis texture coordinate resolution classified once per draw.
#[derive(Clone, Copy, Debug)]
pub(super) enum AxisWrap {
    /// Power-of-two dimension with wrapping: `(c & mask) as usize`.
    PotWrap(i32),
    /// Clamped to `0..=max`.
    Clamp(i32),
    /// Non-power-of-two wrapping.
    NpotWrap(i32),
}

impl AxisWrap {
    fn of(dim: u32, clamp: bool) -> Self {
        let n = dim.max(1);
        if clamp {
            Self::Clamp((n - 1) as i32)
        } else if n.is_power_of_two() && n <= (1 << 30) {
            Self::PotWrap((n - 1) as i32)
        } else {
            Self::NpotWrap(n as i32)
        }
    }

    #[inline(always)]
    fn resolve(self, c: i32) -> usize {
        match self {
            Self::PotWrap(mask) => (c & mask) as usize,
            Self::Clamp(max) => c.clamp(0, max) as usize,
            Self::NpotWrap(n) => {
                if (0..n).contains(&c) {
                    c as usize
                } else {
                    c.rem_euclid(n) as usize
                }
            }
        }
    }
}

/// Texture metadata and bounds proof precomputed once per draw.
#[derive(Clone, Copy)]
pub(super) struct PreparedTexture<'a> {
    pub bytes: &'a [u8],
    pub width_f: f32,
    pub height_f: f32,
    pub pitch: usize,
    pub x_wrap: AxisWrap,
    pub y_wrap: AxisWrap,
    pub pot_x_mask: i32,
    pub pot_y_mask: i32,
    pub both_pot_wrap: bool,
    pub in_bounds: bool,
    pub format: TexFormat,
    pub palette: Option<&'a [[u8; 4]; 256]>,
    pub colour_key: Option<u32>,
    pub active_key: Option<u32>,
    pub channels: Channels,
}

impl<'a> PreparedTexture<'a> {
    pub(super) fn new(tex: &'a Texture<'a>, colour_key_enabled: bool) -> Self {
        let channels = Channels::of(tex.format);
        let format = TexFormat::classify(tex, &channels);
        let x_wrap = AxisWrap::of(tex.width, tex.clamp[0]);
        let y_wrap = AxisWrap::of(tex.height, tex.clamp[1]);
        let (both_pot_wrap, pot_x_mask, pot_y_mask) = match (x_wrap, y_wrap) {
            (AxisWrap::PotWrap(xm), AxisWrap::PotWrap(ym)) => (true, xm, ym),
            _ => (false, 0, 0),
        };
        let w = tex.width.max(1) as usize;
        let h = tex.height.max(1) as usize;
        let pitch = tex.pitch as usize;
        let max_end = (h - 1)
            .checked_mul(pitch)
            .and_then(|row| row.checked_add(w.checked_mul(channels.bytes)?));
        let in_bounds = channels.bytes > 0 && max_end.is_some_and(|end| end <= tex.bytes.len());
        let active_key = if colour_key_enabled {
            tex.colour_key
        } else {
            None
        };
        Self {
            bytes: tex.bytes,
            width_f: tex.width as f32,
            height_f: tex.height as f32,
            pitch,
            x_wrap,
            y_wrap,
            pot_x_mask,
            pot_y_mask,
            both_pot_wrap,
            in_bounds,
            format,
            palette: tex.palette,
            colour_key: tex.colour_key,
            active_key,
            channels,
        }
    }

    #[inline(always)]
    fn read_u16_in_bounds(&self, x: usize, y: usize) -> u32 {
        let at = y * self.pitch + x * 2;
        u32::from(u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]]))
    }

    #[inline(always)]
    fn read_u16_checked(&self, x: usize, y: usize) -> u32 {
        let at = y * self.pitch + x * 2;
        self.bytes
            .get(at..at + 2)
            .map_or(0, |b| u32::from(u16::from_le_bytes([b[0], b[1]])))
    }

    fn texel_at_fast(&self, x: usize, y: usize) -> ([f32; 4], bool) {
        if !self.in_bounds {
            return self.texel_at_checked(x, y);
        }
        match self.format {
            TexFormat::Rgb565 => {
                let v = self.read_u16_in_bounds(x, y);
                let keyed = self.active_key == Some(v);
                (
                    [
                        ((v >> 11) & 0x1f) as f32 * INV_31,
                        ((v >> 5) & 0x3f) as f32 * INV_63,
                        (v & 0x1f) as f32 * INV_31,
                        1.0,
                    ],
                    keyed,
                )
            }
            TexFormat::Argb1555 => {
                let v = self.read_u16_in_bounds(x, y);
                let keyed = self.active_key == Some(v);
                (
                    [
                        ((v >> 10) & 0x1f) as f32 * INV_31,
                        ((v >> 5) & 0x1f) as f32 * INV_31,
                        (v & 0x1f) as f32 * INV_31,
                        ((v >> 15) & 1) as f32,
                    ],
                    keyed,
                )
            }
            TexFormat::Argb4444 => {
                let v = self.read_u16_in_bounds(x, y);
                let keyed = self.active_key == Some(v);
                (
                    [
                        ((v >> 8) & 0xf) as f32 * INV_15,
                        ((v >> 4) & 0xf) as f32 * INV_15,
                        (v & 0xf) as f32 * INV_15,
                        ((v >> 12) & 0xf) as f32 * INV_15,
                    ],
                    keyed,
                )
            }
            TexFormat::Rgb555 => {
                let v = self.read_u16_in_bounds(x, y);
                let keyed = self.active_key == Some(v);
                (
                    [
                        ((v >> 10) & 0x1f) as f32 * INV_31,
                        ((v >> 5) & 0x1f) as f32 * INV_31,
                        (v & 0x1f) as f32 * INV_31,
                        1.0,
                    ],
                    keyed,
                )
            }
            TexFormat::Paletted8 => {
                let idx = self.bytes[y * self.pitch + x];
                let keyed = self.active_key == Some(u32::from(idx));
                let entry = self.palette.unwrap()[usize::from(idx)];
                (
                    [
                        U8_TO_F32[usize::from(entry[0])],
                        U8_TO_F32[usize::from(entry[1])],
                        U8_TO_F32[usize::from(entry[2])],
                        U8_TO_F32[usize::from(entry[3])],
                    ],
                    keyed,
                )
            }
            TexFormat::Xrgb8888 => {
                let at = y * self.pitch + x * 4;
                let v = u32::from_le_bytes([
                    self.bytes[at],
                    self.bytes[at + 1],
                    self.bytes[at + 2],
                    self.bytes[at + 3],
                ]);
                let keyed = self.active_key == Some(v);
                (
                    [
                        ((v >> 16) & 0xff) as f32 * INV_255,
                        ((v >> 8) & 0xff) as f32 * INV_255,
                        (v & 0xff) as f32 * INV_255,
                        1.0,
                    ],
                    keyed,
                )
            }
            TexFormat::Argb8888 => {
                let at = y * self.pitch + x * 4;
                let v = u32::from_le_bytes([
                    self.bytes[at],
                    self.bytes[at + 1],
                    self.bytes[at + 2],
                    self.bytes[at + 3],
                ]);
                let keyed = self.active_key == Some(v);
                (
                    [
                        ((v >> 16) & 0xff) as f32 * INV_255,
                        ((v >> 8) & 0xff) as f32 * INV_255,
                        (v & 0xff) as f32 * INV_255,
                        ((v >> 24) & 0xff) as f32 * INV_255,
                    ],
                    keyed,
                )
            }
            TexFormat::General => self.texel_at_checked(x, y),
        }
    }

    fn texel_at_checked(&self, x: usize, y: usize) -> ([f32; 4], bool) {
        let at = y * self.pitch + x * self.channels.bytes;
        let Some(raw) = self.bytes.get(at..at + self.channels.bytes) else {
            return ([0.0; 4], false);
        };
        let value = self.channels.read(raw);
        let keyed = self.colour_key == Some(value);
        let colour = match self.palette {
            Some(palette) => palette[(value & 0xff) as usize].map(|c| U8_TO_F32[usize::from(c)]),
            None => self.channels.colour(value),
        };
        (colour, keyed)
    }

    fn bilinear16_fast(&self, x: f32, y: f32) -> ([f32; 4], bool) {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, right) = (self.x_wrap.resolve(x0), self.x_wrap.resolve(x0 + 1));
        let (upper, lower) = (self.y_wrap.resolve(y0), self.y_wrap.resolve(y0 + 1));
        let w00 = (256 - fx) * (256 - fy);
        let w10 = fx * (256 - fy);
        let w01 = (256 - fx) * fy;
        let w11 = fx * fy;
        let (t00, t10, t01, t11) = if self.in_bounds {
            (
                self.read_u16_in_bounds(left, upper),
                self.read_u16_in_bounds(right, upper),
                self.read_u16_in_bounds(left, lower),
                self.read_u16_in_bounds(right, lower),
            )
        } else {
            (
                self.read_u16_checked(left, upper),
                self.read_u16_checked(right, upper),
                self.read_u16_checked(left, lower),
                self.read_u16_checked(right, lower),
            )
        };
        let keyed = if let Some(key) = self.active_key {
            let mut k = 0u32;
            if t00 == key {
                k += w00;
            }
            if t10 == key {
                k += w10;
            }
            if t01 == key {
                k += w01;
            }
            if t11 == key {
                k += w11;
            }
            k > 32_768
        } else {
            false
        };
        let colour = match self.format {
            TexFormat::Rgb565 => {
                let r = ((t00 >> 11) & 0x1f) * w00
                    + ((t10 >> 11) & 0x1f) * w10
                    + ((t01 >> 11) & 0x1f) * w01
                    + ((t11 >> 11) & 0x1f) * w11;
                let g = ((t00 >> 5) & 0x3f) * w00
                    + ((t10 >> 5) & 0x3f) * w10
                    + ((t01 >> 5) & 0x3f) * w01
                    + ((t11 >> 5) & 0x3f) * w11;
                let b = (t00 & 0x1f) * w00
                    + (t10 & 0x1f) * w10
                    + (t01 & 0x1f) * w01
                    + (t11 & 0x1f) * w11;
                [
                    r as f32 * INV_31 / 65_536.0,
                    g as f32 * INV_63 / 65_536.0,
                    b as f32 * INV_31 / 65_536.0,
                    1.0,
                ]
            }
            TexFormat::Argb1555 => {
                let r = ((t00 >> 10) & 0x1f) * w00
                    + ((t10 >> 10) & 0x1f) * w10
                    + ((t01 >> 10) & 0x1f) * w01
                    + ((t11 >> 10) & 0x1f) * w11;
                let g = ((t00 >> 5) & 0x1f) * w00
                    + ((t10 >> 5) & 0x1f) * w10
                    + ((t01 >> 5) & 0x1f) * w01
                    + ((t11 >> 5) & 0x1f) * w11;
                let b = (t00 & 0x1f) * w00
                    + (t10 & 0x1f) * w10
                    + (t01 & 0x1f) * w01
                    + (t11 & 0x1f) * w11;
                let a = ((t00 >> 15) & 1) * w00
                    + ((t10 >> 15) & 1) * w10
                    + ((t01 >> 15) & 1) * w01
                    + ((t11 >> 15) & 1) * w11;
                [
                    r as f32 * INV_31 / 65_536.0,
                    g as f32 * INV_31 / 65_536.0,
                    b as f32 * INV_31 / 65_536.0,
                    a as f32 / 65_536.0,
                ]
            }
            TexFormat::Argb4444 => {
                let r = ((t00 >> 8) & 0xf) * w00
                    + ((t10 >> 8) & 0xf) * w10
                    + ((t01 >> 8) & 0xf) * w01
                    + ((t11 >> 8) & 0xf) * w11;
                let g = ((t00 >> 4) & 0xf) * w00
                    + ((t10 >> 4) & 0xf) * w10
                    + ((t01 >> 4) & 0xf) * w01
                    + ((t11 >> 4) & 0xf) * w11;
                let b =
                    (t00 & 0xf) * w00 + (t10 & 0xf) * w10 + (t01 & 0xf) * w01 + (t11 & 0xf) * w11;
                let a = ((t00 >> 12) & 0xf) * w00
                    + ((t10 >> 12) & 0xf) * w10
                    + ((t01 >> 12) & 0xf) * w01
                    + ((t11 >> 12) & 0xf) * w11;
                [
                    r as f32 * INV_15 / 65_536.0,
                    g as f32 * INV_15 / 65_536.0,
                    b as f32 * INV_15 / 65_536.0,
                    a as f32 * INV_15 / 65_536.0,
                ]
            }
            TexFormat::Rgb555 => {
                let r = ((t00 >> 10) & 0x1f) * w00
                    + ((t10 >> 10) & 0x1f) * w10
                    + ((t01 >> 10) & 0x1f) * w01
                    + ((t11 >> 10) & 0x1f) * w11;
                let g = ((t00 >> 5) & 0x1f) * w00
                    + ((t10 >> 5) & 0x1f) * w10
                    + ((t01 >> 5) & 0x1f) * w01
                    + ((t11 >> 5) & 0x1f) * w11;
                let b = (t00 & 0x1f) * w00
                    + (t10 & 0x1f) * w10
                    + (t01 & 0x1f) * w01
                    + (t11 & 0x1f) * w11;
                [
                    r as f32 * INV_31 / 65_536.0,
                    g as f32 * INV_31 / 65_536.0,
                    b as f32 * INV_31 / 65_536.0,
                    1.0,
                ]
            }
            _ => {
                let corners = [(t00, w00), (t10, w10), (t01, w01), (t11, w11)];
                let mut sums = [0u32; 4];
                for (value, weight) in corners {
                    for (i, sum) in sums.iter_mut().enumerate() {
                        *sum +=
                            ((value >> self.channels.shift[i]) & self.channels.mask[i]) * weight;
                    }
                }
                std::array::from_fn(|i| {
                    if self.channels.mask[i] == 0 {
                        self.channels.absent[i]
                    } else {
                        sums[i] as f32 * self.channels.step[i] / 65_536.0
                    }
                })
            }
        };
        (colour, keyed)
    }

    pub(super) fn sample_fast(&self, u: f32, v: f32, linear: bool) -> ([f32; 4], bool) {
        if linear && self.channels.bytes == 2 && self.palette.is_none() {
            return self.bilinear16_fast(u * self.width_f - 0.5, v * self.height_f - 0.5);
        }
        if !linear {
            let x = self.x_wrap.resolve((u * self.width_f).floor() as i32);
            let y = self.y_wrap.resolve((v * self.height_f).floor() as i32);
            return self.texel_at_fast(x, y);
        }
        let (x, y) = (u * self.width_f - 0.5, v * self.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, right) = (self.x_wrap.resolve(x0), self.x_wrap.resolve(x0 + 1));
        let (upper, lower) = (self.y_wrap.resolve(y0), self.y_wrap.resolve(y0 + 1));
        let mut out = [0.0; 4];
        let mut keyed = 0.0;
        for (tx, ty, weight) in [
            (left, upper, (1.0 - fx) * (1.0 - fy)),
            (right, upper, fx * (1.0 - fy)),
            (left, lower, (1.0 - fx) * fy),
            (right, lower, fx * fy),
        ] {
            let (t, k) = self.texel_at_fast(tx, ty);
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

/// Compile-time specialized texture sampler for zero-branch interior span loops.
/// All implementations require `tex.in_bounds == true` and no active colour key.
trait TexSample {
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4];
}

#[inline(always)]
fn resolve_xy<const POT: bool>(tex: &PreparedTexture<'_>, x: i32, y: i32) -> (usize, usize) {
    if POT {
        ((x & tex.pot_x_mask) as usize, (y & tex.pot_y_mask) as usize)
    } else {
        (tex.x_wrap.resolve(x), tex.y_wrap.resolve(y))
    }
}

struct NearestRgb565<const POT: bool>;
impl<const POT: bool> TexSample for NearestRgb565<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = resolve_xy::<POT>(
            tex,
            (u * tex.width_f).floor() as i32,
            (v * tex.height_f).floor() as i32,
        );
        let raw = tex.read_u16_in_bounds(x, y);
        [
            ((raw >> 11) & 0x1f) as f32 * INV_31,
            ((raw >> 5) & 0x3f) as f32 * INV_63,
            (raw & 0x1f) as f32 * INV_31,
            1.0,
        ]
    }
}

struct BilinearRgb565<const POT: bool>;
impl<const POT: bool> TexSample for BilinearRgb565<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0 + 1, y0 + 1);
        let w00 = (256 - fx) * (256 - fy);
        let w10 = fx * (256 - fy);
        let w01 = (256 - fx) * fy;
        let w11 = fx * fy;
        let t00 = tex.read_u16_in_bounds(left, upper);
        let t10 = tex.read_u16_in_bounds(right, upper);
        let t01 = tex.read_u16_in_bounds(left, lower);
        let t11 = tex.read_u16_in_bounds(right, lower);
        let r = ((t00 >> 11) & 0x1f) * w00
            + ((t10 >> 11) & 0x1f) * w10
            + ((t01 >> 11) & 0x1f) * w01
            + ((t11 >> 11) & 0x1f) * w11;
        let g = ((t00 >> 5) & 0x3f) * w00
            + ((t10 >> 5) & 0x3f) * w10
            + ((t01 >> 5) & 0x3f) * w01
            + ((t11 >> 5) & 0x3f) * w11;
        let b = (t00 & 0x1f) * w00 + (t10 & 0x1f) * w10 + (t01 & 0x1f) * w01 + (t11 & 0x1f) * w11;
        [
            r as f32 * INV_31 / 65_536.0,
            g as f32 * INV_63 / 65_536.0,
            b as f32 * INV_31 / 65_536.0,
            1.0,
        ]
    }
}

struct NearestArgb1555<const POT: bool>;
impl<const POT: bool> TexSample for NearestArgb1555<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = resolve_xy::<POT>(
            tex,
            (u * tex.width_f).floor() as i32,
            (v * tex.height_f).floor() as i32,
        );
        let raw = tex.read_u16_in_bounds(x, y);
        [
            ((raw >> 10) & 0x1f) as f32 * INV_31,
            ((raw >> 5) & 0x1f) as f32 * INV_31,
            (raw & 0x1f) as f32 * INV_31,
            ((raw >> 15) & 1) as f32,
        ]
    }
}

struct BilinearArgb1555<const POT: bool>;
impl<const POT: bool> TexSample for BilinearArgb1555<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0 + 1, y0 + 1);
        let w00 = (256 - fx) * (256 - fy);
        let w10 = fx * (256 - fy);
        let w01 = (256 - fx) * fy;
        let w11 = fx * fy;
        let t00 = tex.read_u16_in_bounds(left, upper);
        let t10 = tex.read_u16_in_bounds(right, upper);
        let t01 = tex.read_u16_in_bounds(left, lower);
        let t11 = tex.read_u16_in_bounds(right, lower);
        let r = ((t00 >> 10) & 0x1f) * w00
            + ((t10 >> 10) & 0x1f) * w10
            + ((t01 >> 10) & 0x1f) * w01
            + ((t11 >> 10) & 0x1f) * w11;
        let g = ((t00 >> 5) & 0x1f) * w00
            + ((t10 >> 5) & 0x1f) * w10
            + ((t01 >> 5) & 0x1f) * w01
            + ((t11 >> 5) & 0x1f) * w11;
        let b = (t00 & 0x1f) * w00 + (t10 & 0x1f) * w10 + (t01 & 0x1f) * w01 + (t11 & 0x1f) * w11;
        let a = ((t00 >> 15) & 1) * w00
            + ((t10 >> 15) & 1) * w10
            + ((t01 >> 15) & 1) * w01
            + ((t11 >> 15) & 1) * w11;
        [
            r as f32 * INV_31 / 65_536.0,
            g as f32 * INV_31 / 65_536.0,
            b as f32 * INV_31 / 65_536.0,
            a as f32 / 65_536.0,
        ]
    }
}

struct NearestArgb4444<const POT: bool>;
impl<const POT: bool> TexSample for NearestArgb4444<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = resolve_xy::<POT>(
            tex,
            (u * tex.width_f).floor() as i32,
            (v * tex.height_f).floor() as i32,
        );
        let raw = tex.read_u16_in_bounds(x, y);
        [
            ((raw >> 8) & 0xf) as f32 * INV_15,
            ((raw >> 4) & 0xf) as f32 * INV_15,
            (raw & 0xf) as f32 * INV_15,
            ((raw >> 12) & 0xf) as f32 * INV_15,
        ]
    }
}

struct BilinearArgb4444<const POT: bool>;
impl<const POT: bool> TexSample for BilinearArgb4444<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0 + 1, y0 + 1);
        let w00 = (256 - fx) * (256 - fy);
        let w10 = fx * (256 - fy);
        let w01 = (256 - fx) * fy;
        let w11 = fx * fy;
        let t00 = tex.read_u16_in_bounds(left, upper);
        let t10 = tex.read_u16_in_bounds(right, upper);
        let t01 = tex.read_u16_in_bounds(left, lower);
        let t11 = tex.read_u16_in_bounds(right, lower);
        let r = ((t00 >> 8) & 0xf) * w00
            + ((t10 >> 8) & 0xf) * w10
            + ((t01 >> 8) & 0xf) * w01
            + ((t11 >> 8) & 0xf) * w11;
        let g = ((t00 >> 4) & 0xf) * w00
            + ((t10 >> 4) & 0xf) * w10
            + ((t01 >> 4) & 0xf) * w01
            + ((t11 >> 4) & 0xf) * w11;
        let b = (t00 & 0xf) * w00 + (t10 & 0xf) * w10 + (t01 & 0xf) * w01 + (t11 & 0xf) * w11;
        let a = ((t00 >> 12) & 0xf) * w00
            + ((t10 >> 12) & 0xf) * w10
            + ((t01 >> 12) & 0xf) * w01
            + ((t11 >> 12) & 0xf) * w11;
        [
            r as f32 * INV_15 / 65_536.0,
            g as f32 * INV_15 / 65_536.0,
            b as f32 * INV_15 / 65_536.0,
            a as f32 * INV_15 / 65_536.0,
        ]
    }
}

struct NearestPaletted8<const POT: bool>;
impl<const POT: bool> TexSample for NearestPaletted8<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = resolve_xy::<POT>(
            tex,
            (u * tex.width_f).floor() as i32,
            (v * tex.height_f).floor() as i32,
        );
        let idx = tex.bytes[y * tex.pitch + x];
        let entry = tex.palette.unwrap()[usize::from(idx)];
        [
            U8_TO_F32[usize::from(entry[0])],
            U8_TO_F32[usize::from(entry[1])],
            U8_TO_F32[usize::from(entry[2])],
            U8_TO_F32[usize::from(entry[3])],
        ]
    }
}

struct BilinearPaletted8<const POT: bool>;
impl<const POT: bool> TexSample for BilinearPaletted8<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0 + 1, y0 + 1);
        let pal = tex.palette.unwrap();
        let at = |tx: usize, ty: usize| {
            let e = pal[usize::from(tex.bytes[ty * tex.pitch + tx])];
            [
                U8_TO_F32[usize::from(e[0])],
                U8_TO_F32[usize::from(e[1])],
                U8_TO_F32[usize::from(e[2])],
                U8_TO_F32[usize::from(e[3])],
            ]
        };
        let mut out = [0.0f32; 4];
        for (tx, ty, weight) in [
            (left, upper, (1.0 - fx) * (1.0 - fy)),
            (right, upper, fx * (1.0 - fy)),
            (left, lower, (1.0 - fx) * fy),
            (right, lower, fx * fy),
        ] {
            let c = at(tx, ty);
            out[0] += weight * c[0];
            out[1] += weight * c[1];
            out[2] += weight * c[2];
            out[3] += weight * c[3];
        }
        out
    }
}

/// Which interior-span kernel handles a draw. Selected once per draw call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InteriorKernel {
    UntexturedFlatOpaqueZ,
    UntexturedGouraudOpaqueZ,
    UntexturedAlphaZ,
    TexturedModulateOpaque {
        format: TexFormat,
        linear: bool,
        pot_wrap: bool,
        z_test_write: bool,
        flat: bool,
        specular: bool,
        fog: bool,
    },
    TexturedCopyOpaque {
        format: TexFormat,
        linear: bool,
        pot_wrap: bool,
        z_test_write: bool,
    },
    TexturedModulateAlphaZ {
        format: TexFormat,
        linear: bool,
        pot_wrap: bool,
        flat: bool,
        specular: bool,
    },
    FastRgb565,
    General,
}

/// Per-draw precomputed state shared across every triangle in `triangles()`.
#[derive(Clone, Copy)]
pub(super) struct DrawPlan<'a> {
    pub pixel_channels: Channels,
    pub texture: Option<PreparedTexture<'a>>,
    pub carried: usize,
    #[cfg_attr(not(test), allow(dead_code))]
    pub kernel: InteriorKernel,
    draw_span: SpanFn,
}

impl<'a> DrawPlan<'a> {
    pub(super) fn new(
        target_format: PixelFormat,
        texture: Option<&'a Texture<'a>>,
        states: &States,
    ) -> Self {
        let pixel_channels = Channels::of(target_format);
        let target_rgb565 = target_format == PixelFormat::RGB565;
        let prepared_tex = texture.map(|t| PreparedTexture::new(t, states.colour_key));
        let carried = if states.specular || states.fog.is_some() {
            PLANES
        } else if !states.flat {
            8
        } else if texture.is_some() {
            4
        } else {
            2
        };
        let kernel = select_kernel(target_rgb565, prepared_tex.as_ref(), states);
        let draw_span = resolve_span_fn(kernel);
        Self {
            pixel_channels,
            texture: prepared_tex,
            carried,
            kernel,
            draw_span,
        }
    }
}

fn is_specialized_tex_format(format: TexFormat) -> bool {
    matches!(
        format,
        TexFormat::Rgb565 | TexFormat::Argb1555 | TexFormat::Argb4444 | TexFormat::Paletted8
    )
}

fn select_kernel(
    target_rgb565: bool,
    tex: Option<&PreparedTexture<'_>>,
    states: &States,
) -> InteriorKernel {
    if !target_rgb565 {
        return InteriorKernel::General;
    }
    let z_le = states.depth_test == Some(Compare::LessEqual);
    let z_le_write = z_le && states.depth_write;
    let z_none = states.depth_test.is_none() && !states.depth_write;
    let no_alpha_test = states.alpha_test.is_none();
    let has_fog = states.fog.is_some();

    match tex {
        None => {
            if z_le
                && no_alpha_test
                && !states.specular
                && !has_fog
                && states.colour == Combine::Diffuse
            {
                if states.blend.is_none() && states.depth_write {
                    return if states.flat {
                        InteriorKernel::UntexturedFlatOpaqueZ
                    } else {
                        InteriorKernel::UntexturedGouraudOpaqueZ
                    };
                }
                if states.blend == Some((Blend::SourceAlpha, Blend::InverseSourceAlpha))
                    && states.alpha == Combine::Diffuse
                {
                    return InteriorKernel::UntexturedAlphaZ;
                }
            }
            InteriorKernel::FastRgb565
        }
        Some(t) => {
            if !no_alpha_test
                || !t.in_bounds
                || t.active_key.is_some()
                || !is_specialized_tex_format(t.format)
            {
                return InteriorKernel::FastRgb565;
            }
            if states.blend.is_none() && (z_le_write || z_none) {
                if states.colour == Combine::Texture && !states.specular && !has_fog {
                    return InteriorKernel::TexturedCopyOpaque {
                        format: t.format,
                        linear: states.linear,
                        pot_wrap: t.both_pot_wrap,
                        z_test_write: z_le_write,
                    };
                }
                if states.colour == Combine::Modulate {
                    let supported_shade = if t.both_pot_wrap && z_le_write {
                        (!states.flat && !(states.specular && has_fog))
                            || (states.flat && !states.specular && !has_fog)
                    } else {
                        !states.flat && !states.specular && !has_fog
                    };
                    if supported_shade {
                        return InteriorKernel::TexturedModulateOpaque {
                            format: t.format,
                            linear: states.linear,
                            pot_wrap: t.both_pot_wrap,
                            z_test_write: z_le_write,
                            flat: states.flat,
                            specular: states.specular,
                            fog: has_fog,
                        };
                    }
                }
            } else if z_le
                && states.colour == Combine::Modulate
                && states.blend == Some((Blend::SourceAlpha, Blend::InverseSourceAlpha))
                && !has_fog
            {
                let supported_shade = if t.both_pot_wrap {
                    !(states.flat && states.specular)
                } else {
                    !states.flat && !states.specular
                };
                if supported_shade {
                    return InteriorKernel::TexturedModulateAlphaZ {
                        format: t.format,
                        linear: states.linear,
                        pot_wrap: t.both_pot_wrap,
                        flat: states.flat,
                        specular: states.specular,
                    };
                }
            }
            InteriorKernel::FastRgb565
        }
    }
}

/// Resolve the concrete interior-span function pointer once per draw.
/// Wave 2 SIMD adapters (#255) select their architecture-specific span
/// function pointers at this single seam point.
fn resolve_span_fn(kernel: InteriorKernel) -> SpanFn {
    match kernel {
        InteriorKernel::UntexturedFlatOpaqueZ => span_untextured_flat_opaque_z,
        InteriorKernel::UntexturedGouraudOpaqueZ => span_untextured_gouraud_opaque_z,
        InteriorKernel::UntexturedAlphaZ => span_untextured_alpha_z,
        InteriorKernel::TexturedModulateOpaque {
            format,
            linear,
            pot_wrap,
            z_test_write,
            flat,
            specular,
            fog,
        } => resolve_textured_modulate_opaque(
            format,
            linear,
            pot_wrap,
            z_test_write,
            flat,
            specular,
            fog,
        ),
        InteriorKernel::TexturedCopyOpaque {
            format,
            linear,
            pot_wrap,
            z_test_write,
        } => resolve_textured_copy_opaque(format, linear, pot_wrap, z_test_write),
        InteriorKernel::TexturedModulateAlphaZ {
            format,
            linear,
            pot_wrap,
            flat,
            specular,
        } => resolve_textured_modulate_alpha_z(format, linear, pot_wrap, flat, specular),
        InteriorKernel::FastRgb565 => span_fast_rgb565,
        InteriorKernel::General => span_general_in_bounds,
    }
}

/// Dispatch the scanline's covered pixel span through the draw's pre-resolved
/// interior-span kernel.
///
/// Preconditions:
/// - `pixels.len() == depth.len() * plan.pixel_channels.bytes`
/// - Every pixel in `0..depth.len()` is covered by the triangle.
/// - `value` holds the exact plane values at the first pixel of the span;
///   callers do not read `value` after this call returns.
#[inline(always)]
pub(super) fn draw_interior(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    (plan.draw_span)(plan, depth, pixels, value, step, flat, states);
}

fn span_untextured_flat_opaque_z(
    _plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    _states: &States,
) {
    let packed = pack_rgb565_rgb(flat.0[0], flat.0[1], flat.0[2]);
    let mut z_val = value[0];
    let z_step = step[0];
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let z = z_val;
        z_val += z_step;
        if z <= *d {
            *px = packed;
            *d = z;
        }
    }
}

fn span_untextured_gouraud_opaque_z(
    _plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    _flat: ([f32; 4], [f32; 3]),
    _states: &States,
) {
    let mut z_val = value[0];
    let mut rhw = value[1];
    let mut r_w = value[4];
    let mut g_w = value[5];
    let mut b_w = value[6];
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let (z, w, cr, cg, cb) = (z_val, rhw, r_w, g_w, b_w);
        z_val += step[0];
        rhw += step[1];
        r_w += step[4];
        g_w += step[5];
        b_w += step[6];
        if z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            *px = pack_rgb565_rgb(cr * near, cg * near, cb * near);
            *d = z;
        }
    }
}

fn span_untextured_alpha_z(
    _plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let depth_write = states.depth_write;
    if states.flat {
        let sa = flat.0[3];
        let isa = 1.0 - sa;
        let sr_sa = flat.0[0] * sa;
        let sg_sa = flat.0[1] * sa;
        let sb_sa = flat.0[2] * sa;
        let mut z_val = value[0];
        for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
            let z = z_val;
            z_val += step[0];
            if z <= *d {
                let under = unpack_rgb565(px[0], px[1]);
                *px = pack_rgb565_rgb(
                    sr_sa + under[0] * isa,
                    sg_sa + under[1] * isa,
                    sb_sa + under[2] * isa,
                );
                if depth_write {
                    *d = z;
                }
            }
        }
    } else {
        let mut z_val = value[0];
        let mut rhw = value[1];
        let mut r_w = value[4];
        let mut g_w = value[5];
        let mut b_w = value[6];
        let mut a_w = value[7];
        for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
            let (z, w, cr, cg, cb, ca) = (z_val, rhw, r_w, g_w, b_w, a_w);
            z_val += step[0];
            rhw += step[1];
            r_w += step[4];
            g_w += step[5];
            b_w += step[6];
            a_w += step[7];
            if z <= *d {
                let near = if w > 0.0 { 1.0 / w } else { 0.0 };
                let r = cr * near;
                let g = cg * near;
                let b = cb * near;
                let sa = ca * near;
                let isa = 1.0 - sa;
                let under = unpack_rgb565(px[0], px[1]);
                *px = pack_rgb565_rgb(
                    r * sa + under[0] * isa,
                    g * sa + under[1] * isa,
                    b * sa + under[2] * isa,
                );
                if depth_write {
                    *d = z;
                }
            }
        }
    }
}

fn span_textured_copy_opaque<S: TexSample, const Z_TEST_WRITE: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    _flat: ([f32; 4], [f32; 3]),
    _states: &States,
) {
    let tex = plan.texture.as_ref().unwrap();
    let mut z_val = value[0];
    let mut rhw = value[1];
    let mut u_w = value[2];
    let mut v_w = value[3];
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let (z, w, u, v) = (z_val, rhw, u_w, v_w);
        if Z_TEST_WRITE {
            z_val += step[0];
        }
        rhw += step[1];
        u_w += step[2];
        v_w += step[3];
        if !Z_TEST_WRITE || z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let texel = S::sample(tex, u * near, v * near);
            *px = pack_rgb565_rgb(texel[0], texel[1], texel[2]);
            if Z_TEST_WRITE {
                *d = z;
            }
        }
    }
}

fn resolve_textured_copy_opaque(
    format: TexFormat,
    linear: bool,
    pot_wrap: bool,
    z_test_write: bool,
) -> SpanFn {
    macro_rules! go {
        ($sampler:ty) => {
            if z_test_write {
                span_textured_copy_opaque::<$sampler, true>
            } else {
                span_textured_copy_opaque::<$sampler, false>
            }
        };
    }
    match (format, linear, pot_wrap) {
        (TexFormat::Rgb565, false, true) => go!(NearestRgb565<true>),
        (TexFormat::Rgb565, false, false) => go!(NearestRgb565<false>),
        (TexFormat::Rgb565, true, true) => go!(BilinearRgb565<true>),
        (TexFormat::Rgb565, true, false) => go!(BilinearRgb565<false>),
        (TexFormat::Argb1555, false, true) => go!(NearestArgb1555<true>),
        (TexFormat::Argb1555, false, false) => go!(NearestArgb1555<false>),
        (TexFormat::Argb1555, true, true) => go!(BilinearArgb1555<true>),
        (TexFormat::Argb1555, true, false) => go!(BilinearArgb1555<false>),
        (TexFormat::Argb4444, false, true) => go!(NearestArgb4444<true>),
        (TexFormat::Argb4444, false, false) => go!(NearestArgb4444<false>),
        (TexFormat::Argb4444, true, true) => go!(BilinearArgb4444<true>),
        (TexFormat::Argb4444, true, false) => go!(BilinearArgb4444<false>),
        (TexFormat::Paletted8, false, true) => go!(NearestPaletted8<true>),
        (TexFormat::Paletted8, false, false) => go!(NearestPaletted8<false>),
        (TexFormat::Paletted8, true, true) => go!(BilinearPaletted8<true>),
        (TexFormat::Paletted8, true, false) => go!(BilinearPaletted8<false>),
        _ => unreachable!("select_kernel only emits specialized texture formats"),
    }
}

fn span_textured_modulate_opaque<
    S: TexSample,
    const Z_TEST_WRITE: bool,
    const FLAT: bool,
    const SPECULAR: bool,
    const FOG: bool,
>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let tex = plan.texture.as_ref().unwrap();
    let flat_diffuse = flat.0;
    let fog_colour = states.fog.unwrap_or([0.0; 3]);
    let mut z_val = value[0];
    let mut rhw = value[1];
    let mut u_w = value[2];
    let mut v_w = value[3];
    let mut r_w = value[4];
    let mut g_w = value[5];
    let mut b_w = value[6];
    let mut sr_w = value[8];
    let mut sg_w = value[9];
    let mut sb_w = value[10];
    let mut f_w = value[11];

    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let (z, w, u, v) = (z_val, rhw, u_w, v_w);
        if Z_TEST_WRITE {
            z_val += step[0];
        }
        rhw += step[1];
        u_w += step[2];
        v_w += step[3];
        let (cr, cg, cb) = (r_w, g_w, b_w);
        if !FLAT {
            r_w += step[4];
            g_w += step[5];
            b_w += step[6];
        }
        let (sr, sg, sb) = (sr_w, sg_w, sb_w);
        if SPECULAR {
            sr_w += step[8];
            sg_w += step[9];
            sb_w += step[10];
        }
        let fw = f_w;
        if FOG {
            f_w += step[11];
        }
        if !Z_TEST_WRITE || z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let texel = S::sample(tex, u * near, v * near);
            let (mut r, mut g, mut b) = if FLAT {
                (
                    texel[0] * flat_diffuse[0],
                    texel[1] * flat_diffuse[1],
                    texel[2] * flat_diffuse[2],
                )
            } else {
                (
                    texel[0] * (cr * near),
                    texel[1] * (cg * near),
                    texel[2] * (cb * near),
                )
            };
            if SPECULAR {
                r = (r + sr * near).min(1.0);
                g = (g + sg * near).min(1.0);
                b = (b + sb * near).min(1.0);
            }
            if FOG {
                let f = (fw * near).clamp(0.0, 1.0);
                let inv_f = 1.0 - f;
                r = r * f + fog_colour[0] * inv_f;
                g = g * f + fog_colour[1] * inv_f;
                b = b * f + fog_colour[2] * inv_f;
            }
            *px = pack_rgb565_rgb(r, g, b);
            if Z_TEST_WRITE {
                *d = z;
            }
        }
    }
}

fn resolve_opaque_pot_sampler<S: TexSample>(
    z_test_write: bool,
    flat: bool,
    specular: bool,
    fog: bool,
) -> SpanFn {
    match (z_test_write, flat, specular, fog) {
        (true, false, false, false) => {
            span_textured_modulate_opaque::<S, true, false, false, false>
        }
        (true, true, false, false) => span_textured_modulate_opaque::<S, true, true, false, false>,
        (true, false, true, false) => span_textured_modulate_opaque::<S, true, false, true, false>,
        (true, false, false, true) => span_textured_modulate_opaque::<S, true, false, false, true>,
        (false, false, false, false) => {
            span_textured_modulate_opaque::<S, false, false, false, false>
        }
        _ => unreachable!("select_kernel only emits supported opaque shade combinations"),
    }
}

fn resolve_textured_modulate_opaque(
    format: TexFormat,
    linear: bool,
    pot_wrap: bool,
    z_test_write: bool,
    flat: bool,
    specular: bool,
    fog: bool,
) -> SpanFn {
    if pot_wrap {
        macro_rules! go_pot {
            ($sampler:ty) => {
                resolve_opaque_pot_sampler::<$sampler>(z_test_write, flat, specular, fog)
            };
        }
        match (format, linear) {
            (TexFormat::Rgb565, false) => go_pot!(NearestRgb565<true>),
            (TexFormat::Rgb565, true) => go_pot!(BilinearRgb565<true>),
            (TexFormat::Argb1555, false) => go_pot!(NearestArgb1555<true>),
            (TexFormat::Argb1555, true) => go_pot!(BilinearArgb1555<true>),
            (TexFormat::Argb4444, false) => go_pot!(NearestArgb4444<true>),
            (TexFormat::Argb4444, true) => go_pot!(BilinearArgb4444<true>),
            (TexFormat::Paletted8, false) => go_pot!(NearestPaletted8<true>),
            (TexFormat::Paletted8, true) => go_pot!(BilinearPaletted8<true>),
            _ => unreachable!("select_kernel only emits specialized texture formats"),
        }
    } else {
        macro_rules! go_gen {
            ($sampler:ty) => {
                if z_test_write {
                    span_textured_modulate_opaque::<$sampler, true, false, false, false>
                } else {
                    span_textured_modulate_opaque::<$sampler, false, false, false, false>
                }
            };
        }
        match (format, linear) {
            (TexFormat::Rgb565, false) => go_gen!(NearestRgb565<false>),
            (TexFormat::Rgb565, true) => go_gen!(BilinearRgb565<false>),
            (TexFormat::Argb1555, false) => go_gen!(NearestArgb1555<false>),
            (TexFormat::Argb1555, true) => go_gen!(BilinearArgb1555<false>),
            (TexFormat::Argb4444, false) => go_gen!(NearestArgb4444<false>),
            (TexFormat::Argb4444, true) => go_gen!(BilinearArgb4444<false>),
            (TexFormat::Paletted8, false) => go_gen!(NearestPaletted8<false>),
            (TexFormat::Paletted8, true) => go_gen!(BilinearPaletted8<false>),
            _ => unreachable!("select_kernel only emits specialized texture formats"),
        }
    }
}

fn span_textured_modulate_alpha_z<S: TexSample, const FLAT: bool, const SPECULAR: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let tex = plan.texture.as_ref().unwrap();
    let flat_diffuse = flat.0;
    let alpha_combine = states.alpha;
    let depth_write = states.depth_write;
    let mut z_val = value[0];
    let mut rhw = value[1];
    let mut u_w = value[2];
    let mut v_w = value[3];
    let mut r_w = value[4];
    let mut g_w = value[5];
    let mut b_w = value[6];
    let mut a_w = value[7];
    let mut sr_w = value[8];
    let mut sg_w = value[9];
    let mut sb_w = value[10];

    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let (z, w, u, v) = (z_val, rhw, u_w, v_w);
        z_val += step[0];
        rhw += step[1];
        u_w += step[2];
        v_w += step[3];
        let (cr, cg, cb, ca) = (r_w, g_w, b_w, a_w);
        if !FLAT {
            r_w += step[4];
            g_w += step[5];
            b_w += step[6];
            a_w += step[7];
        }
        let (sr, sg, sb) = (sr_w, sg_w, sb_w);
        if SPECULAR {
            sr_w += step[8];
            sg_w += step[9];
            sb_w += step[10];
        }
        if z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let texel = S::sample(tex, u * near, v * near);
            let (mut r, mut g, mut b, da) = if FLAT {
                (
                    texel[0] * flat_diffuse[0],
                    texel[1] * flat_diffuse[1],
                    texel[2] * flat_diffuse[2],
                    flat_diffuse[3],
                )
            } else {
                (
                    texel[0] * (cr * near),
                    texel[1] * (cg * near),
                    texel[2] * (cb * near),
                    ca * near,
                )
            };
            if SPECULAR {
                r = (r + sr * near).min(1.0);
                g = (g + sg * near).min(1.0);
                b = (b + sb * near).min(1.0);
            }
            let sa = combine_channel(alpha_combine, texel[3], da);
            let isa = 1.0 - sa;
            let under = unpack_rgb565(px[0], px[1]);
            *px = pack_rgb565_rgb(
                r * sa + under[0] * isa,
                g * sa + under[1] * isa,
                b * sa + under[2] * isa,
            );
            if depth_write {
                *d = z;
            }
        }
    }
}

fn resolve_alpha_pot_sampler<S: TexSample>(flat: bool, specular: bool) -> SpanFn {
    match (flat, specular) {
        (false, false) => span_textured_modulate_alpha_z::<S, false, false>,
        (true, false) => span_textured_modulate_alpha_z::<S, true, false>,
        (false, true) => span_textured_modulate_alpha_z::<S, false, true>,
        _ => unreachable!("select_kernel only emits supported alpha shade combinations"),
    }
}

fn resolve_textured_modulate_alpha_z(
    format: TexFormat,
    linear: bool,
    pot_wrap: bool,
    flat: bool,
    specular: bool,
) -> SpanFn {
    if pot_wrap {
        macro_rules! go_pot {
            ($sampler:ty) => {
                resolve_alpha_pot_sampler::<$sampler>(flat, specular)
            };
        }
        match (format, linear) {
            (TexFormat::Rgb565, false) => go_pot!(NearestRgb565<true>),
            (TexFormat::Rgb565, true) => go_pot!(BilinearRgb565<true>),
            (TexFormat::Argb1555, false) => go_pot!(NearestArgb1555<true>),
            (TexFormat::Argb1555, true) => go_pot!(BilinearArgb1555<true>),
            (TexFormat::Argb4444, false) => go_pot!(NearestArgb4444<true>),
            (TexFormat::Argb4444, true) => go_pot!(BilinearArgb4444<true>),
            (TexFormat::Paletted8, false) => go_pot!(NearestPaletted8<true>),
            (TexFormat::Paletted8, true) => go_pot!(BilinearPaletted8<true>),
            _ => unreachable!("select_kernel only emits specialized texture formats"),
        }
    } else {
        macro_rules! go_gen {
            ($sampler:ty) => {
                span_textured_modulate_alpha_z::<$sampler, false, false>
            };
        }
        match (format, linear) {
            (TexFormat::Rgb565, false) => go_gen!(NearestRgb565<false>),
            (TexFormat::Rgb565, true) => go_gen!(BilinearRgb565<false>),
            (TexFormat::Argb1555, false) => go_gen!(NearestArgb1555<false>),
            (TexFormat::Argb1555, true) => go_gen!(BilinearArgb1555<false>),
            (TexFormat::Argb4444, false) => go_gen!(NearestArgb4444<false>),
            (TexFormat::Argb4444, true) => go_gen!(BilinearArgb4444<false>),
            (TexFormat::Paletted8, false) => go_gen!(NearestPaletted8<false>),
            (TexFormat::Paletted8, true) => go_gen!(BilinearPaletted8<false>),
            _ => unreachable!("select_kernel only emits specialized texture formats"),
        }
    }
}

/// Fast RGB565 interior span covering all remaining render-state combinations:
/// zero edge tests, hoisted target slices, compile-time RGB565 pack/unpack,
/// and POT-wrapped fast texture sampling.
fn span_fast_rgb565(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let texture = plan.texture.as_ref();
    let carried = plan.carried;
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let here = *value;
        for (v, s) in value.iter_mut().zip(step).take(carried) {
            *v += *s;
        }
        let z = here[0];
        if let Some(test) = states.depth_test {
            if !test.passes(z, *d) {
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
        let (texel, keyed) = match texture {
            Some(t) => t.sample_fast(here[2] * near, here[3] * near, states.linear),
            None => ([1.0; 4], false),
        };
        if keyed && states.colour_key {
            continue;
        }
        let mut colour = [
            combine_channel(states.colour, texel[0], diffuse[0]),
            combine_channel(states.colour, texel[1], diffuse[1]),
            combine_channel(states.colour, texel[2], diffuse[2]),
            combine_channel(states.alpha, texel[3], diffuse[3]),
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
        if let Some((source, destination)) = states.blend {
            let under = unpack_rgb565(px[0], px[1]);
            let (s, dst) = (
                source.factor(colour, under),
                destination.factor(colour, under),
            );
            for i in 0..3 {
                colour[i] = colour[i] * s[i] + under[i] * dst[i];
            }
        }
        let packed = pack_rgb565_rgb(colour[0], colour[1], colour[2]);
        px[0] = packed[0];
        px[1] = packed[1];
        if states.depth_write {
            *d = z;
        }
    }
}

/// Arbitrary-format interior span fallback when target format is not RGB565.
fn span_general_in_bounds(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let pixel_channels = &plan.pixel_channels;
    let texture = plan.texture.as_ref();
    let carried = plan.carried;
    let bytes = pixel_channels.bytes;
    if bytes == 0 {
        return;
    }
    for (d, pixel) in depth.iter_mut().zip(pixels.chunks_exact_mut(bytes)) {
        let here = *value;
        for (v, s) in value.iter_mut().zip(step).take(carried) {
            *v += *s;
        }
        let z = here[0];
        if let Some(test) = states.depth_test {
            if !test.passes(z, *d) {
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
        let (texel, keyed) = match texture {
            Some(t) => t.sample_fast(here[2] * near, here[3] * near, states.linear),
            None => ([1.0; 4], false),
        };
        if keyed && states.colour_key {
            continue;
        }
        let mut colour = [
            combine_channel(states.colour, texel[0], diffuse[0]),
            combine_channel(states.colour, texel[1], diffuse[1]),
            combine_channel(states.colour, texel[2], diffuse[2]),
            combine_channel(states.alpha, texel[3], diffuse[3]),
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
        if let Some((source, destination)) = states.blend {
            let under = pixel_channels.colour(pixel_channels.read(pixel));
            let (s, dst) = (
                source.factor(colour, under),
                destination.factor(colour, under),
            );
            for i in 0..4 {
                colour[i] = colour[i] * s[i] + under[i] * dst[i];
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
            *d = z;
        }
    }
}
