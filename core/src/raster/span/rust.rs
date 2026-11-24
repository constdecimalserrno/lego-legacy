//! Canonical pure-Rust interior-span kernels (oracle and universal fallback).

use super::{
    combine_channel, pack_rgb565_rgb, resolve_xy, unpack_rgb565, DrawPlan, InteriorKernel,
    PreparedTexture, SpanFn, TexFormat, INV_15, INV_31, INV_63, U8_TO_F32,
};
use crate::raster::{States, PLANES};

/// Compile-time specialized texture sampler for zero-branch interior span loops.
/// All implementations require `tex.in_bounds == true` and no active colour key.
pub(super) trait TexSample {
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4];
}

pub(super) struct NearestRgb565<const POT: bool>;
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

pub(super) struct BilinearRgb565<const POT: bool>;
impl<const POT: bool> TexSample for BilinearRgb565<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0.wrapping_add(1), y0.wrapping_add(1));
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

pub(super) struct NearestArgb1555<const POT: bool>;
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

pub(super) struct BilinearArgb1555<const POT: bool>;
impl<const POT: bool> TexSample for BilinearArgb1555<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0.wrapping_add(1), y0.wrapping_add(1));
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

pub(super) struct NearestArgb4444<const POT: bool>;
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

pub(super) struct BilinearArgb4444<const POT: bool>;
impl<const POT: bool> TexSample for BilinearArgb4444<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (((x - x0) * 256.0) as u32, ((y - y0) * 256.0) as u32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0.wrapping_add(1), y0.wrapping_add(1));
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

pub(super) struct NearestPaletted8<const POT: bool>;
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

pub(super) struct BilinearPaletted8<const POT: bool>;
impl<const POT: bool> TexSample for BilinearPaletted8<POT> {
    #[inline(always)]
    fn sample(tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0.wrapping_add(1), y0.wrapping_add(1));
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

/// Resolve the concrete interior-span function pointer once per draw.
/// Wave 2 SIMD adapters (#255) select their architecture-specific span
/// function pointers at this single seam point.
pub(super) fn resolve_span_fn(kernel: InteriorKernel) -> SpanFn {
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

pub(super) fn span_untextured_flat_opaque_z(
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

pub(super) fn span_untextured_gouraud_opaque_z(
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

pub(super) fn span_untextured_alpha_z(
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

pub(super) fn span_textured_copy_opaque<S: TexSample, const Z_TEST_WRITE: bool>(
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

pub(super) fn span_textured_modulate_opaque<
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

pub(super) fn span_textured_modulate_alpha_z<
    S: TexSample,
    const FLAT: bool,
    const SPECULAR: bool,
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
pub(super) fn span_fast_rgb565(
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
pub(super) fn span_general_in_bounds(
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
