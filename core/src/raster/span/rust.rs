//! Canonical pure-Rust interior-span kernels (oracle and universal fallback).

use super::{
    combine_channel, pack_rgb565_rgb, resolve_xy, unpack_rgb565, BlendMode, DrawPlan,
    InteriorKernel, PreparedTexture, SpanFn, TexFormat, INV_15, INV_31, INV_63,
};
use crate::raster::{Blend, Combine, Compare, States, PLANES};

pub(super) const BLEND_ALPHA: u8 = 0;
pub(super) const BLEND_ADDITIVE_ALPHA: u8 = 1;
pub(super) const BLEND_INV_SRC_COLOUR: u8 = 2;

/// Compile-time specialized texture sampler for zero-branch interior span loops.
/// All implementations require `tex.in_bounds == true` and no active colour key.
pub(super) trait TexSample {
    fn sample(plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4];
}

pub(super) struct NearestRgb565<const POT: bool>;
impl<const POT: bool> TexSample for NearestRgb565<POT> {
    #[inline(always)]
    fn sample(_plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
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
    fn sample(_plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
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
    fn sample(_plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
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
    fn sample(_plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
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
    fn sample(_plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
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
    fn sample(_plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
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
    fn sample(plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = resolve_xy::<POT>(
            tex,
            (u * tex.width_f).floor() as i32,
            (v * tex.height_f).floor() as i32,
        );
        let idx = tex.bytes[y * tex.pitch + x];
        plan.palette_f32()[usize::from(idx)]
    }
}

pub(super) struct BilinearPaletted8<const POT: bool>;
impl<const POT: bool> TexSample for BilinearPaletted8<POT> {
    #[inline(always)]
    fn sample(plan: &DrawPlan<'_>, tex: &PreparedTexture<'_>, u: f32, v: f32) -> [f32; 4] {
        let (x, y) = (u * tex.width_f - 0.5, v * tex.height_f - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (left, upper) = resolve_xy::<POT>(tex, x0, y0);
        let (right, lower) = resolve_xy::<POT>(tex, x0.wrapping_add(1), y0.wrapping_add(1));
        let pal = plan.palette_f32();
        let at = |tx: usize, ty: usize| pal[usize::from(tex.bytes[ty * tex.pitch + tx])];
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
pub(super) fn resolve_span_fn(kernel: InteriorKernel) -> SpanFn {
    match kernel {
        InteriorKernel::UntexturedFlatOpaqueZ => span_untextured_flat_opaque::<true, false>,
        InteriorKernel::UntexturedGouraudOpaqueZ => span_untextured_gouraud_opaque::<true, false>,
        InteriorKernel::UntexturedAlphaZ => span_untextured_alpha::<true, false>,
        InteriorKernel::UntexturedOpaque {
            flat,
            specular,
            z_test,
        } => match (flat, z_test, specular) {
            (true, true, false) => span_untextured_flat_opaque::<true, false>,
            (true, true, true) => span_untextured_flat_opaque::<true, true>,
            (true, false, false) => span_untextured_flat_opaque::<false, false>,
            (true, false, true) => span_untextured_flat_opaque::<false, true>,
            (false, true, false) => span_untextured_gouraud_opaque::<true, false>,
            (false, true, true) => span_untextured_gouraud_opaque::<true, true>,
            (false, false, false) => span_untextured_gouraud_opaque::<false, false>,
            (false, false, true) => span_untextured_gouraud_opaque::<false, true>,
        },
        InteriorKernel::UntexturedAlpha { z_le, alpha_gt } => match (z_le, alpha_gt) {
            (true, false) => span_untextured_alpha::<true, false>,
            (true, true) => span_untextured_alpha::<true, true>,
            (false, false) => span_untextured_alpha::<false, false>,
            (false, true) => span_untextured_alpha::<false, true>,
        },
        InteriorKernel::TexturedModulateOpaque {
            format,
            linear,
            pot_wrap,
            z_test,
            flat,
            specular,
            fog,
        } => {
            resolve_textured_modulate_opaque(format, linear, pot_wrap, z_test, flat, specular, fog)
        }
        InteriorKernel::TexturedCopyOpaque {
            format,
            linear,
            pot_wrap,
            z_test,
        } => resolve_textured_copy_opaque(format, linear, pot_wrap, z_test),
        InteriorKernel::TexturedModulateBlend {
            format,
            linear,
            pot_wrap,
            z_le,
            alpha_gt,
            blend,
            flat,
            specular,
        } => resolve_textured_modulate_blend(
            format, linear, pot_wrap, z_le, alpha_gt, blend, flat, specular,
        ),
        InteriorKernel::FastRgb565 => span_fast_rgb565,
        InteriorKernel::General => span_general_in_bounds,
    }
}

#[inline(always)]
pub(super) fn span_untextured_flat_opaque<const Z_TEST: bool, const SPECULAR: bool>(
    _plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let (r, g, b) = if SPECULAR {
        (
            (flat.0[0] + flat.1[0]).min(1.0),
            (flat.0[1] + flat.1[1]).min(1.0),
            (flat.0[2] + flat.1[2]).min(1.0),
        )
    } else {
        (flat.0[0], flat.0[1], flat.0[2])
    };
    let packed = pack_rgb565_rgb(r, g, b);
    if !Z_TEST {
        for px in pixels.as_chunks_mut::<2>().0.iter_mut().take(depth.len()) {
            *px = packed;
        }
        return;
    }
    let depth_write = states.depth_write;
    let mut z_val = value[0];
    let z_step = step[0];
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let z = z_val;
        z_val += z_step;
        if z <= *d {
            *px = packed;
            if depth_write {
                *d = z;
            }
        }
    }
}

#[inline(always)]
pub(super) fn span_untextured_gouraud_opaque<const Z_TEST: bool, const SPECULAR: bool>(
    _plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    _flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let depth_write = states.depth_write;
    let mut z_val = value[0];
    let mut rhw = value[1];
    let mut r_w = value[4];
    let mut g_w = value[5];
    let mut b_w = value[6];
    let mut sr_w = value[8];
    let mut sg_w = value[9];
    let mut sb_w = value[10];
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let (z, w, cr, cg, cb) = (z_val, rhw, r_w, g_w, b_w);
        if Z_TEST {
            z_val += step[0];
        }
        rhw += step[1];
        r_w += step[4];
        g_w += step[5];
        b_w += step[6];
        let (sr, sg, sb) = (sr_w, sg_w, sb_w);
        if SPECULAR {
            sr_w += step[8];
            sg_w += step[9];
            sb_w += step[10];
        }
        if !Z_TEST || z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let mut r = cr * near;
            let mut g = cg * near;
            let mut b = cb * near;
            if SPECULAR {
                r = (r + sr * near).min(1.0);
                g = (g + sg * near).min(1.0);
                b = (b + sb * near).min(1.0);
            }
            *px = pack_rgb565_rgb(r, g, b);
            if Z_TEST && depth_write {
                *d = z;
            }
        }
    }
}

#[inline(always)]
pub(super) fn span_untextured_alpha<const Z_LE: bool, const ALPHA_GT: bool>(
    _plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let depth_write = states.depth_write;
    let alpha_ref = states.alpha_test.map_or(0, |(_, r)| r);
    if states.flat {
        let sa = flat.0[3];
        if ALPHA_GT {
            let alpha = (sa.clamp(0.0, 1.0) * 255.0).round() as u8;
            if alpha <= alpha_ref {
                return;
            }
        }
        let isa = 1.0 - sa;
        let sr_sa = flat.0[0] * sa;
        let sg_sa = flat.0[1] * sa;
        let sb_sa = flat.0[2] * sa;
        let mut z_val = value[0];
        for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
            let z = z_val;
            if Z_LE {
                z_val += step[0];
            }
            if !Z_LE || z <= *d {
                let under = unpack_rgb565(px[0], px[1]);
                *px = pack_rgb565_rgb(
                    sr_sa + under[0] * isa,
                    sg_sa + under[1] * isa,
                    sb_sa + under[2] * isa,
                );
                if Z_LE && depth_write {
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
            if Z_LE {
                z_val += step[0];
            }
            rhw += step[1];
            r_w += step[4];
            g_w += step[5];
            b_w += step[6];
            a_w += step[7];
            if !Z_LE || z <= *d {
                let near = if w > 0.0 { 1.0 / w } else { 0.0 };
                let sa = ca * near;
                if ALPHA_GT {
                    let alpha = (sa.clamp(0.0, 1.0) * 255.0).round() as u8;
                    if alpha <= alpha_ref {
                        continue;
                    }
                }
                let r = cr * near;
                let g = cg * near;
                let b = cb * near;
                let isa = 1.0 - sa;
                let under = unpack_rgb565(px[0], px[1]);
                *px = pack_rgb565_rgb(
                    r * sa + under[0] * isa,
                    g * sa + under[1] * isa,
                    b * sa + under[2] * isa,
                );
                if Z_LE && depth_write {
                    *d = z;
                }
            }
        }
    }
}

#[inline(always)]
pub(super) fn span_textured_copy_opaque<S: TexSample, const Z_TEST: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    _flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
    let mut z_val = value[0];
    let mut rhw = value[1];
    let mut u_w = value[2];
    let mut v_w = value[3];
    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let (z, w, u, v) = (z_val, rhw, u_w, v_w);
        if Z_TEST {
            z_val += step[0];
        }
        rhw += step[1];
        u_w += step[2];
        v_w += step[3];
        if !Z_TEST || z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let texel = S::sample(plan, tex, u * near, v * near);
            *px = pack_rgb565_rgb(texel[0], texel[1], texel[2]);
            if Z_TEST && depth_write {
                *d = z;
            }
        }
    }
}

fn resolve_textured_copy_opaque(
    format: TexFormat,
    linear: bool,
    pot_wrap: bool,
    z_test: bool,
) -> SpanFn {
    macro_rules! go {
        ($sampler:ty) => {
            if z_test {
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

#[inline(always)]
pub(super) fn span_textured_modulate_opaque<
    S: TexSample,
    const Z_TEST: bool,
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
    let flat_specular = flat.1;
    let fog_colour = states.fog.unwrap_or([0.0; 3]);
    let depth_write = states.depth_write;
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
        if Z_TEST {
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
        if !FLAT && SPECULAR {
            sr_w += step[8];
            sg_w += step[9];
            sb_w += step[10];
        }
        let fw = f_w;
        if FOG {
            f_w += step[11];
        }
        if !Z_TEST || z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let texel = S::sample(plan, tex, u * near, v * near);
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
                if FLAT {
                    r = (r + flat_specular[0]).min(1.0);
                    g = (g + flat_specular[1]).min(1.0);
                    b = (b + flat_specular[2]).min(1.0);
                } else {
                    r = (r + sr * near).min(1.0);
                    g = (g + sg * near).min(1.0);
                    b = (b + sb * near).min(1.0);
                }
            }
            if FOG {
                let f = (fw * near).clamp(0.0, 1.0);
                let inv_f = 1.0 - f;
                r = r * f + fog_colour[0] * inv_f;
                g = g * f + fog_colour[1] * inv_f;
                b = b * f + fog_colour[2] * inv_f;
            }
            *px = pack_rgb565_rgb(r, g, b);
            if Z_TEST && depth_write {
                *d = z;
            }
        }
    }
}

fn resolve_opaque_pot_sampler<S: TexSample>(
    z_test: bool,
    flat: bool,
    specular: bool,
    fog: bool,
) -> SpanFn {
    match (z_test, flat, specular, fog) {
        (true, false, false, false) => {
            span_textured_modulate_opaque::<S, true, false, false, false>
        }
        (true, true, false, false) => span_textured_modulate_opaque::<S, true, true, false, false>,
        (true, false, true, false) => span_textured_modulate_opaque::<S, true, false, true, false>,
        (true, true, true, false) => span_textured_modulate_opaque::<S, true, true, true, false>,
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
    z_test: bool,
    flat: bool,
    specular: bool,
    fog: bool,
) -> SpanFn {
    if pot_wrap {
        macro_rules! go_pot {
            ($sampler:ty) => {
                resolve_opaque_pot_sampler::<$sampler>(z_test, flat, specular, fog)
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
                if z_test {
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

#[inline(always)]
pub(super) fn span_textured_modulate_blend<
    S: TexSample,
    const Z_LE: bool,
    const ALPHA_GT: bool,
    const BLEND: u8,
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
    let flat_specular = flat.1;
    let alpha_combine = states.alpha;
    let alpha_ref = states.alpha_test.map_or(0, |(_, r)| r);
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
        if Z_LE {
            z_val += step[0];
        }
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
        if !FLAT && SPECULAR {
            sr_w += step[8];
            sg_w += step[9];
            sb_w += step[10];
        }
        if !Z_LE || z <= *d {
            let near = if w > 0.0 { 1.0 / w } else { 0.0 };
            let texel = S::sample(plan, tex, u * near, v * near);
            let sa = if alpha_combine == Combine::Texture {
                texel[3]
            } else {
                let da = if FLAT { flat_diffuse[3] } else { ca * near };
                combine_channel(alpha_combine, texel[3], da)
            };
            if ALPHA_GT {
                let alpha = (sa.clamp(0.0, 1.0) * 255.0).round() as u8;
                if alpha <= alpha_ref {
                    continue;
                }
            }
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
                if FLAT {
                    r = (r + flat_specular[0]).min(1.0);
                    g = (g + flat_specular[1]).min(1.0);
                    b = (b + flat_specular[2]).min(1.0);
                } else {
                    r = (r + sr * near).min(1.0);
                    g = (g + sg * near).min(1.0);
                    b = (b + sb * near).min(1.0);
                }
            }
            let under = unpack_rgb565(px[0], px[1]);
            let (out_r, out_g, out_b) = match BLEND {
                BLEND_ALPHA => {
                    let isa = 1.0 - sa;
                    (
                        r * sa + under[0] * isa,
                        g * sa + under[1] * isa,
                        b * sa + under[2] * isa,
                    )
                }
                BLEND_ADDITIVE_ALPHA => (r * sa + under[0], g * sa + under[1], b * sa + under[2]),
                BLEND_INV_SRC_COLOUR => (
                    r * 0.0 + under[0] * (1.0 - r),
                    g * 0.0 + under[1] * (1.0 - g),
                    b * 0.0 + under[2] * (1.0 - b),
                ),
                _ => unreachable!(),
            };
            *px = pack_rgb565_rgb(out_r, out_g, out_b);
            if Z_LE && depth_write {
                *d = z;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_blend_pot_sampler<S: TexSample>(
    z_le: bool,
    alpha_gt: bool,
    blend: BlendMode,
    flat: bool,
    specular: bool,
) -> SpanFn {
    match (blend, z_le, alpha_gt, flat, specular) {
        (BlendMode::Alpha, true, false, false, false) => {
            span_textured_modulate_blend::<S, true, false, BLEND_ALPHA, false, false>
        }
        (BlendMode::Alpha, true, false, true, false) => {
            span_textured_modulate_blend::<S, true, false, BLEND_ALPHA, true, false>
        }
        (BlendMode::Alpha, true, false, false, true) => {
            span_textured_modulate_blend::<S, true, false, BLEND_ALPHA, false, true>
        }
        (BlendMode::Alpha, true, false, true, true) => {
            span_textured_modulate_blend::<S, true, false, BLEND_ALPHA, true, true>
        }
        (BlendMode::Alpha, true, true, false, false) => {
            span_textured_modulate_blend::<S, true, true, BLEND_ALPHA, false, false>
        }
        (BlendMode::Alpha, false, true, false, false) => {
            span_textured_modulate_blend::<S, false, true, BLEND_ALPHA, false, false>
        }
        (BlendMode::Alpha, false, false, false, false) => {
            span_textured_modulate_blend::<S, false, false, BLEND_ALPHA, false, false>
        }
        (BlendMode::AdditiveAlpha, true, false, false, false) => {
            span_textured_modulate_blend::<S, true, false, BLEND_ADDITIVE_ALPHA, false, false>
        }
        (BlendMode::InvSrcColour, true, false, false, false) => {
            span_textured_modulate_blend::<S, true, false, BLEND_INV_SRC_COLOUR, false, false>
        }
        _ => unreachable!("select_kernel only emits supported blend combinations"),
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_textured_modulate_blend(
    format: TexFormat,
    linear: bool,
    pot_wrap: bool,
    z_le: bool,
    alpha_gt: bool,
    blend: BlendMode,
    flat: bool,
    specular: bool,
) -> SpanFn {
    if pot_wrap {
        macro_rules! go_pot {
            ($sampler:ty) => {
                resolve_blend_pot_sampler::<$sampler>(z_le, alpha_gt, blend, flat, specular)
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
                span_textured_modulate_blend::<$sampler, true, false, BLEND_ALPHA, false, false>
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

#[inline(always)]
fn blend_factor_rgb(factor: Blend, src: [f32; 4], dst: [f32; 4]) -> [f32; 3] {
    match factor {
        Blend::Zero => [0.0, 0.0, 0.0],
        Blend::One => [1.0, 1.0, 1.0],
        Blend::SourceColour => [src[0], src[1], src[2]],
        Blend::InverseSourceColour => [1.0 - src[0], 1.0 - src[1], 1.0 - src[2]],
        Blend::SourceAlpha => [src[3], src[3], src[3]],
        Blend::InverseSourceAlpha => {
            let v = 1.0 - src[3];
            [v, v, v]
        }
        Blend::DestinationAlpha => [dst[3], dst[3], dst[3]],
        Blend::InverseDestinationAlpha => {
            let v = 1.0 - dst[3];
            [v, v, v]
        }
        Blend::DestinationColour => [dst[0], dst[1], dst[2]],
        Blend::InverseDestinationColour => [1.0 - dst[0], 1.0 - dst[1], 1.0 - dst[2]],
    }
}

fn span_fast_rgb565_inner<const HAS_TEX: bool, const HAS_BLEND: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let tex = if HAS_TEX { plan.texture.as_ref() } else { None };
    let carried = plan.carried;
    let depth_test = match states.depth_test {
        Some(Compare::Always) => None,
        other => other,
    };
    let alpha_test = match states.alpha_test {
        Some((Compare::Always, _)) => None,
        other => other,
    };
    let depth_write = states.depth_write;
    let flat_mode = states.flat;
    let specular_on = states.specular;
    let fog_colour = states.fog;
    let colour_combine = states.colour;
    let alpha_combine = states.alpha;
    let linear = states.linear;
    let check_key = HAS_TEX && states.colour_key;
    let blend_pair = if HAS_BLEND { states.blend } else { None };
    let flat_folded_rgb = if !HAS_TEX && flat_mode && specular_on {
        Some([
            (combine_channel(colour_combine, 1.0, flat.0[0]) + flat.1[0]).min(1.0),
            (combine_channel(colour_combine, 1.0, flat.0[1]) + flat.1[1]).min(1.0),
            (combine_channel(colour_combine, 1.0, flat.0[2]) + flat.1[2]).min(1.0),
        ])
    } else {
        None
    };

    for (d, px) in depth.iter_mut().zip(pixels.as_chunks_mut::<2>().0) {
        let here = *value;
        for (v, s) in value.iter_mut().zip(step).take(carried) {
            *v += *s;
        }
        let z = here[0];
        if let Some(test) = depth_test {
            if !test.passes(z, *d) {
                continue;
            }
        }
        let near = if here[1] > 0.0 { 1.0 / here[1] } else { 0.0 };
        let (diffuse, specular) = if flat_mode {
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
        let mut colour = if HAS_TEX {
            let (texel, keyed) =
                plan.sample_fast_tex(tex.unwrap(), here[2] * near, here[3] * near, linear);
            if check_key && keyed {
                continue;
            }
            let mut c = [
                combine_channel(colour_combine, texel[0], diffuse[0]),
                combine_channel(colour_combine, texel[1], diffuse[1]),
                combine_channel(colour_combine, texel[2], diffuse[2]),
                combine_channel(alpha_combine, texel[3], diffuse[3]),
            ];
            if specular_on {
                c[0] = (c[0] + specular[0]).min(1.0);
                c[1] = (c[1] + specular[1]).min(1.0);
                c[2] = (c[2] + specular[2]).min(1.0);
            }
            c
        } else if let Some(rgb) = flat_folded_rgb {
            [
                rgb[0],
                rgb[1],
                rgb[2],
                combine_channel(alpha_combine, 1.0, diffuse[3]),
            ]
        } else {
            let mut c = [
                combine_channel(colour_combine, 1.0, diffuse[0]),
                combine_channel(colour_combine, 1.0, diffuse[1]),
                combine_channel(colour_combine, 1.0, diffuse[2]),
                combine_channel(alpha_combine, 1.0, diffuse[3]),
            ];
            if specular_on {
                c[0] = (c[0] + specular[0]).min(1.0);
                c[1] = (c[1] + specular[1]).min(1.0);
                c[2] = (c[2] + specular[2]).min(1.0);
            }
            c
        };
        if let Some(fog) = fog_colour {
            let f = (here[11] * near).clamp(0.0, 1.0);
            let inv_f = 1.0 - f;
            colour[0] = colour[0] * f + fog[0] * inv_f;
            colour[1] = colour[1] * f + fog[1] * inv_f;
            colour[2] = colour[2] * f + fog[2] * inv_f;
        }
        if let Some((test, reference)) = alpha_test {
            let alpha = (colour[3].clamp(0.0, 1.0) * 255.0).round() as u8;
            if !test.passes(alpha, reference) {
                continue;
            }
        }
        if HAS_BLEND {
            let (source, destination) = blend_pair.unwrap();
            let under = unpack_rgb565(px[0], px[1]);
            let s = blend_factor_rgb(source, colour, under);
            let dst = blend_factor_rgb(destination, colour, under);
            colour[0] = colour[0] * s[0] + under[0] * dst[0];
            colour[1] = colour[1] * s[1] + under[1] * dst[1];
            colour[2] = colour[2] * s[2] + under[2] * dst[2];
        }
        let packed = pack_rgb565_rgb(colour[0], colour[1], colour[2]);
        px[0] = packed[0];
        px[1] = packed[1];
        if depth_write {
            *d = z;
        }
    }
}

/// Fast RGB565 interior span covering all remaining render-state combinations:
/// zero edge tests, hoisted target slices, compile-time RGB565 pack/unpack,
/// hoisted texture/blend dispatch (no per-pixel `[1.0f32; 4]` memset), and
/// POT-wrapped fast texture sampling with per-draw float palette LUT.
pub(super) fn span_fast_rgb565(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    match (plan.texture.is_some(), states.blend.is_some()) {
        (false, false) => {
            span_fast_rgb565_inner::<false, false>(plan, depth, pixels, value, step, flat, states)
        }
        (false, true) => {
            span_fast_rgb565_inner::<false, true>(plan, depth, pixels, value, step, flat, states)
        }
        (true, false) => {
            span_fast_rgb565_inner::<true, false>(plan, depth, pixels, value, step, flat, states)
        }
        (true, true) => {
            span_fast_rgb565_inner::<true, true>(plan, depth, pixels, value, step, flat, states)
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
            Some(t) => plan.sample_fast_tex(t, here[2] * near, here[3] * near, states.linear),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::span::U8_TO_F32;

    #[test]
    fn rgb565_unpack_pack_round_trips_all_65536_values() {
        for v in 0u32..=0xffff {
            let bytes = (v as u16).to_le_bytes();
            let c = unpack_rgb565(bytes[0], bytes[1]);
            assert_eq!(
                pack_rgb565_rgb(c[0], c[1], c[2]),
                bytes,
                "RGB565 round-trip mismatch at {v:#06x}"
            );
        }
    }

    #[test]
    fn unit_range_texel_scales_stay_strictly_within_0_to_1() {
        for r in 0u32..=31 {
            let f = r as f32 * INV_31;
            assert!((0.0..=1.0).contains(&f), "r={r} f={f}");
        }
        for g in 0u32..=63 {
            let f = g as f32 * INV_63;
            assert!((0.0..=1.0).contains(&f), "g={g} f={f}");
        }
        for c in 0u32..=15 {
            let f = c as f32 * INV_15;
            assert!((0.0..=1.0).contains(&f), "c={c} f={f}");
        }
        for &(inv, max_sum) in &[
            (INV_31 / 65_536.0, 31u32 * 65_536),
            (INV_63 / 65_536.0, 63u32 * 65_536),
            (INV_15 / 65_536.0, 15u32 * 65_536),
        ] {
            let lo = 0.0f32 * inv;
            let hi = (max_sum as f32) * inv;
            assert!((0.0..=1.0).contains(&lo), "lo={lo}");
            assert!((0.0..=1.0).contains(&hi), "hi={hi}");
        }
        for (i, &f) in U8_TO_F32.iter().enumerate() {
            assert!((0.0..=1.0).contains(&f), "u8={i} f={f}");
        }
    }
}
