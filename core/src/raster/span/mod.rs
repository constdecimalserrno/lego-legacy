//! Specialized interior-span kernels for the software rasteriser (#254, #255).
//!
//! Co-located per-subsystem seam per ADR-0009:
//! - [`rust`]: canonical pure-Rust oracle and universal fallback;
//! - [`aarch64`]: 4-pixel NEON interior-span adapter;
//! - [`x86_64`]: 4-pixel baseline SSE2 interior-span adapter.
//!
//! Determinism invariants preserved bit-for-bit across all tiers:
//! - Sequential plane stepping (`value[k] += step[k]`) on every pixel;
//! - Exact IEEE-754 `1.0 / here[1]` perspective division (no approximate rcp);
//! - Exact Q16 integer weights in `bilinear16`;
//! - Separate multiply and add in blending, shading, and fog (no FMA);
//! - Exact RGB565 rounding `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32`.

#[cfg(target_arch = "aarch64")]
mod aarch64;
mod rust;
#[cfg(target_arch = "x86_64")]
mod x86_64;

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

#[inline(always)]
fn resolve_xy<const POT: bool>(tex: &PreparedTexture<'_>, x: i32, y: i32) -> (usize, usize) {
    if POT {
        ((x & tex.pot_x_mask) as usize, (y & tex.pot_y_mask) as usize)
    } else {
        (tex.x_wrap.resolve(x), tex.y_wrap.resolve(y))
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
/// Honors `LEGO_LEGACY_ARCH=rust` via `crate::arch::use_rust()` (ADR-0009).
fn resolve_span_fn(kernel: InteriorKernel) -> SpanFn {
    if crate::arch::use_rust() {
        return rust::resolve_span_fn(kernel);
    }
    #[cfg(target_arch = "aarch64")]
    {
        aarch64::resolve_span_fn(kernel)
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::resolve_span_fn(kernel)
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        rust::resolve_span_fn(kernel)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn native_span_fn(kernel: InteriorKernel) -> SpanFn {
        #[cfg(target_arch = "aarch64")]
        {
            aarch64::resolve_span_fn(kernel)
        }
        #[cfg(target_arch = "x86_64")]
        {
            x86_64::resolve_span_fn(kernel)
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        {
            rust::resolve_span_fn(kernel)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn assert_span_parity(
        plan: &DrawPlan<'_>,
        init_depth: &[f32],
        init_pixels: &[u8],
        pixel_byte_offset: usize,
        value: [f32; PLANES],
        step: [f32; PLANES],
        flat: ([f32; 4], [f32; 3]),
        states: &States,
        label: &str,
    ) {
        let rust_fn = rust::resolve_span_fn(plan.kernel);
        let simd_fn = native_span_fn(plan.kernel);

        let mut depth_r = init_depth.to_vec();
        let mut depth_s = init_depth.to_vec();
        let mut pix_r = vec![0xaa; pixel_byte_offset + init_pixels.len() + 3];
        let mut pix_s = vec![0xaa; pixel_byte_offset + init_pixels.len() + 3];
        pix_r[pixel_byte_offset..pixel_byte_offset + init_pixels.len()]
            .copy_from_slice(init_pixels);
        pix_s[pixel_byte_offset..pixel_byte_offset + init_pixels.len()]
            .copy_from_slice(init_pixels);

        let mut vr = value;
        let mut vs = value;
        rust_fn(
            plan,
            &mut depth_r,
            &mut pix_r[pixel_byte_offset..pixel_byte_offset + init_pixels.len()],
            &mut vr,
            &step,
            flat,
            states,
        );
        simd_fn(
            plan,
            &mut depth_s,
            &mut pix_s[pixel_byte_offset..pixel_byte_offset + init_pixels.len()],
            &mut vs,
            &step,
            flat,
            states,
        );

        assert_eq!(pix_r, pix_s, "pixel mismatch ({label})");
        let bits_r: Vec<u32> = depth_r.iter().map(|d| d.to_bits()).collect();
        let bits_s: Vec<u32> = depth_s.iter().map(|d| d.to_bits()).collect();
        assert_eq!(bits_r, bits_s, "depth mismatch ({label})");
    }

    #[test]
    fn bilinear_q16_combined_scales_match_separate_division_bit_for_bit() {
        for &(inv, max_sum) in &[
            (INV_31, 31u32 * 65_536),
            (INV_63, 63u32 * 65_536),
            (INV_15, 15u32 * 65_536),
        ] {
            let combined = inv / 65_536.0;
            // Sample dense edge values plus stride across the entire integer range.
            for s in (0..=1024)
                .chain((max_sum - 1024)..=max_sum)
                .chain((0..=max_sum).step_by(997))
            {
                let a = (s as f32 * inv) / 65_536.0;
                let b = s as f32 * combined;
                assert_eq!(a.to_bits(), b.to_bits(), "sum={s} inv={inv}");
            }
        }
    }

    /// Layer 1 differential oracle: runs the compiled ISA adapter (`aarch64`
    /// NEON or `x86_64` SSE2) side-by-side against `rust.rs` across every
    /// specialized kernel family, texture format, wrap/clamp mode, alpha
    /// combine mode, span widths `0..=65`, and unaligned byte offsets `0..=7`.
    #[test]
    fn simd_matches_rust_oracle_across_widths_offsets_and_kernels() {
        const ARGB1555: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x7c00, 0x03e0, 0x001f, 0x8000],
        };
        const ARGB4444: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x0f00, 0x00f0, 0x000f, 0xf000],
        };
        const PAL8: PixelFormat = PixelFormat {
            bits: 8,
            masks: [0; 4],
        };

        let mut seed = 0x1234_5678u32;
        let mut next_u32 = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        let palette: [[u8; 4]; 256] = std::array::from_fn(|i| {
            [
                (i as u8).wrapping_mul(41),
                (i as u8).wrapping_mul(83).wrapping_add(7),
                (i as u8).wrapping_mul(149).wrapping_add(13),
                if i % 4 == 0 { 255 } else { (i as u8) ^ 0xa5 },
            ]
        });

        let tex16_bytes: Vec<u8> = (0..16 * (16 * 2 + 4)).map(|_| next_u32() as u8).collect();
        let tex8_bytes: Vec<u8> = (0..16 * (16 + 4)).map(|_| next_u32() as u8).collect();

        let base_value: [f32; PLANES] = [
            0.35, 0.85, -0.37, 1.13, 0.72, 0.31, 0.58, 0.64, 0.14, 0.29, 0.08, 0.41,
        ];
        let base_step: [f32; PLANES] = [
            0.004, 0.011, 0.063, -0.047, -0.009, 0.013, 0.007, -0.005, 0.006, -0.004, 0.011, 0.017,
        ];
        let flat = ([0.83, 0.47, 0.61, 0.54], [0.19, 0.33, 0.07]);

        let make_buffers = |len: usize| -> (Vec<f32>, Vec<u8>) {
            let depth: Vec<f32> = (0..len)
                .map(|i| match i % 7 {
                    0 => 0.1,
                    1 => f32::NAN,
                    2 => 0.35,
                    3 => -0.0,
                    _ => 0.9,
                })
                .collect();
            let pixels: Vec<u8> = (0..len * 2)
                .map(|i| ((i * 53 + len * 17 + 9) & 0xff) as u8)
                .collect();
            (depth, pixels)
        };

        // 1. Untextured specialized kernels across all span widths 0..=65 and odd byte offsets.
        let untextured_states: &[(&str, States)] = &[
            (
                "untex_flat_opaque",
                States {
                    flat: true,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    ..States::default()
                },
            ),
            (
                "untex_gouraud_opaque",
                States {
                    flat: false,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    ..States::default()
                },
            ),
            (
                "untex_flat_alpha_dw",
                States {
                    flat: true,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    depth_write: true,
                    ..States::default()
                },
            ),
            (
                "untex_gouraud_alpha_nodw",
                States {
                    flat: false,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    depth_write: false,
                    ..States::default()
                },
            ),
        ];
        for (name, states) in untextured_states {
            let plan = DrawPlan::new(PixelFormat::RGB565, None, states);
            for len in 0..=65 {
                let (depth, pixels) = make_buffers(len);
                let byte_offset = len % 8;
                assert_span_parity(
                    &plan,
                    &depth,
                    &pixels,
                    byte_offset,
                    base_value,
                    base_step,
                    flat,
                    states,
                    &format!("{name}/len={len}/off={byte_offset}"),
                );
            }
        }

        // 2. All specialized textured kernels across formats, wrap/clamp, nearest/bilinear.
        type TexSpec = (&'static str, PixelFormat, bool, u32, u32, [bool; 2]);
        let tex_specs: &[TexSpec] = &[
            (
                "rgb565_pot",
                PixelFormat::RGB565,
                false,
                16,
                16,
                [false, false],
            ),
            (
                "rgb565_clamp",
                PixelFormat::RGB565,
                false,
                16,
                16,
                [true, true],
            ),
            (
                "rgb565_npot",
                PixelFormat::RGB565,
                false,
                6,
                5,
                [false, false],
            ),
            ("argb1555_pot", ARGB1555, false, 16, 16, [false, false]),
            ("argb1555_npot", ARGB1555, false, 7, 5, [false, true]),
            ("argb4444_pot", ARGB4444, false, 16, 16, [false, false]),
            ("argb4444_npot", ARGB4444, false, 5, 7, [true, false]),
            ("pal8_pot", PAL8, true, 16, 16, [false, false]),
            ("pal8_npot", PAL8, true, 6, 5, [false, false]),
        ];

        for &(tname, fmt, is_pal, tw, th, clamp) in tex_specs {
            let bpp = fmt.bits.div_ceil(8);
            let tex = Texture {
                bytes: if is_pal { &tex8_bytes } else { &tex16_bytes },
                width: tw,
                height: th,
                pitch: tw * bpp + 4,
                format: fmt,
                palette: is_pal.then_some(&palette),
                colour_key: None,
                clamp,
            };
            for linear in [false, true] {
                let state_cases: &[(&str, States)] = &[
                    (
                        "mod_opaque_gouraud_z",
                        States {
                            linear,
                            ..States::default()
                        },
                    ),
                    (
                        "mod_opaque_flat_z",
                        States {
                            linear,
                            flat: true,
                            ..States::default()
                        },
                    ),
                    (
                        "mod_opaque_spec_z",
                        States {
                            linear,
                            specular: true,
                            ..States::default()
                        },
                    ),
                    (
                        "mod_opaque_fog_z",
                        States {
                            linear,
                            fog: Some([0.25, 0.55, 0.85]),
                            ..States::default()
                        },
                    ),
                    (
                        "mod_opaque_noz",
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
                        "copy_opaque_noz",
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
                        "alpha_gouraud_mod_dw",
                        States {
                            linear,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            alpha: Combine::Modulate,
                            depth_write: true,
                            ..States::default()
                        },
                    ),
                    (
                        "alpha_gouraud_add_nodw",
                        States {
                            linear,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            alpha: Combine::Add,
                            depth_write: false,
                            ..States::default()
                        },
                    ),
                    (
                        "alpha_flat_tex_nodw",
                        States {
                            linear,
                            flat: true,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            alpha: Combine::Texture,
                            depth_write: false,
                            ..States::default()
                        },
                    ),
                    (
                        "alpha_spec_diff_dw",
                        States {
                            linear,
                            specular: true,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            alpha: Combine::Diffuse,
                            depth_write: true,
                            ..States::default()
                        },
                    ),
                ];

                for (sname, states) in state_cases {
                    let plan = DrawPlan::new(PixelFormat::RGB565, Some(&tex), states);
                    // Sweep full width range 0..=65 on the primary RGB565 POT cases,
                    // and key representative widths (tails + multi-chunk) on all others.
                    let widths: Vec<usize> =
                        if tname == "rgb565_pot" && *sname == "mod_opaque_gouraud_z" {
                            (0..=65).collect()
                        } else {
                            vec![0, 1, 2, 3, 4, 5, 7, 8, 11, 16, 19, 33, 65]
                        };
                    for len in widths {
                        let (depth, pixels) = make_buffers(len);
                        let byte_offset = len % 7;
                        assert_span_parity(
                            &plan,
                            &depth,
                            &pixels,
                            byte_offset,
                            base_value,
                            base_step,
                            flat,
                            states,
                            &format!("{tname}/lin={linear}/{sname}/len={len}/off={byte_offset}"),
                        );
                    }
                }
            }
        }
    }

    /// Layer 1 edge-case oracle: exercises NaN/±inf/negative/zero `rhw`, NaN
    /// and -0.0 depths, extreme out-of-i32-range UVs, out-of-[0,1] and NaN
    /// colours/specular/fog, and partial/all-fail Z masks.
    #[test]
    fn simd_matches_rust_oracle_on_edge_z_uv_colour_alpha_and_nan() {
        let tex_bytes: Vec<u8> = (0..8 * 8 * 2)
            .map(|i| ((i * 73 + 19) & 0xff) as u8)
            .collect();
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
        let tex4444 = Texture {
            format: PixelFormat {
                bits: 16,
                masks: [0x0f00, 0x00f0, 0x000f, 0xf000],
            },
            ..tex
        };

        let edge_values: &[(&str, [f32; PLANES], [f32; PLANES])] = &[
            (
                "zero_neg_rhw",
                [
                    0.2, -0.5, -10.25, 15.75, -0.5, 1.5, 0.5, 1.2, 0.5, -0.2, 1.1, 1.5,
                ],
                [
                    0.01, 0.25, 3.5, -4.25, 0.4, -0.3, 0.2, -0.4, 0.1, 0.2, -0.3, -0.6,
                ],
            ),
            (
                "nan_rhw_and_colour",
                [
                    0.4,
                    f32::NAN,
                    1.0,
                    2.0,
                    f32::NAN,
                    -10.0,
                    10.0,
                    f32::NAN,
                    f32::NAN,
                    2.0,
                    -1.0,
                    f32::NAN,
                ],
                [0.0, 0.0, 0.1, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ),
            (
                "huge_and_nan_uv",
                [
                    0.3, 1.0, 3.0e9, -3.0e9, 0.8, 0.5, 0.2, 0.9, 0.3, 0.4, 0.5, -0.5,
                ],
                [
                    0.0, 0.0, 1.0e9, -1.0e9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.5,
                ],
            ),
            (
                "inf_uv_and_colour",
                [
                    0.1,
                    1.0,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    f32::INFINITY,
                    f32::NEG_INFINITY,
                    0.5,
                    0.5,
                    0.0,
                    0.0,
                    0.0,
                    f32::NAN,
                ],
                [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ),
            (
                "tiny_neg_frac_uv",
                [
                    0.25, 1.0, -1.0e-8, -0.0, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 0.5,
                ],
                [
                    0.0, 0.0, 1.0e-9, -1.0e-8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                ],
            ),
            (
                "nan_depth",
                [
                    f32::NAN,
                    1.0,
                    0.25,
                    0.75,
                    0.5,
                    0.5,
                    0.5,
                    0.5,
                    0.0,
                    0.0,
                    0.0,
                    0.5,
                ],
                [0.0, 0.0, 0.1, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ),
        ];

        let depth_patterns: &[(&str, [f32; 12])] = &[
            ("all_pass", [1.0; 12]),
            ("all_fail", [0.0; 12]),
            (
                "mixed_nan_negzero",
                [
                    0.0,
                    1.0,
                    f32::NAN,
                    0.25,
                    -0.0,
                    0.2,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    1.0,
                    f32::NEG_INFINITY,
                ],
            ),
        ];

        let init_pixels: Vec<u8> = (0..24).map(|i| (i * 29 + 5) as u8).collect();
        let flat = ([1.2, -0.1, f32::NAN, 0.5], [0.2, 0.3, 0.4]);

        let states_list: &[(&str, Option<&Texture<'_>>, States)] = &[
            (
                "untex_flat",
                None,
                States {
                    flat: true,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    ..States::default()
                },
            ),
            (
                "untex_gouraud",
                None,
                States {
                    flat: false,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    ..States::default()
                },
            ),
            (
                "untex_alpha",
                None,
                States {
                    flat: false,
                    colour: Combine::Diffuse,
                    alpha: Combine::Diffuse,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    ..States::default()
                },
            ),
            ("nearest_mod_opaque", Some(&tex), States::default()),
            (
                "bilinear_mod_opaque",
                Some(&tex),
                States {
                    linear: true,
                    ..States::default()
                },
            ),
            (
                "bilinear_copy_opaque",
                Some(&tex),
                States {
                    linear: true,
                    colour: Combine::Texture,
                    alpha: Combine::Texture,
                    ..States::default()
                },
            ),
            (
                "nearest_mod_fog",
                Some(&tex),
                States {
                    fog: Some([0.3, 0.6, 0.9]),
                    ..States::default()
                },
            ),
            (
                "nearest_mod_spec",
                Some(&tex),
                States {
                    specular: true,
                    ..States::default()
                },
            ),
            (
                "bilinear_4444_alpha",
                Some(&tex4444),
                States {
                    linear: true,
                    blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                    ..States::default()
                },
            ),
        ];

        for (sname, t, states) in states_list {
            let plan = DrawPlan::new(PixelFormat::RGB565, *t, states);
            for (ename, val, stp) in edge_values {
                for (dname, depth) in depth_patterns {
                    assert_span_parity(
                        &plan,
                        depth,
                        &init_pixels,
                        3, // unaligned odd byte offset
                        *val,
                        *stp,
                        flat,
                        states,
                        &format!("{sname}/{ename}/{dname}"),
                    );
                }
            }
        }
    }
}
