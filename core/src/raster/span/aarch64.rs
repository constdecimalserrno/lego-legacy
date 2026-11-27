//! 4-pixel `aarch64` NEON interior-span kernels (ADR-0009, #255, #278, #281).
//!
//! Preserves Golden Rule 1 bit-for-bit against [`super::rust`]:
//! - Sequential plane stepping (`v += step`) per pixel via 4-lane vector adds + 4×4 transpose;
//! - Exact IEEE-754 `vdivq_f32` (no approximate reciprocal);
//! - Separate rounded `vmulq_f32` and `vaddq_f32` (no FMA);
//! - Algebraically exact separable 16-bit integer bilinear (`vmlaq_s16` / `vmlal_s16`) with
//!   paired 32-bit non-seam loads;
//! - Exact IEEE-754 `vrndaq_f32` round-half-away-from-zero for `alpha_test == Greater`;
//! - Exact float RGB565 packing matching `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32`;
//! - Early `n < 4` exit and `#[inline(always)]` `0..3` pixel scalar tails delegated to `rust.rs`.

use core::arch::aarch64::{
    float32x4_t, int16x4_t, int16x8_t, int32x4_t, uint16x4_t, uint16x8_t, uint32x4_t, vaddq_f32,
    vaddq_s32, vandq_s32, vandq_u16, vandq_u32, vbsl_u16, vbslq_f32, vceqq_f32, vceqq_s32,
    vceqq_u32, vcgtq_f32, vcgtq_u32, vcleq_f32, vcltq_f32, vcombine_s16, vcvtq_f32_s32,
    vcvtq_f32_u32, vcvtq_s32_f32, vcvtq_u32_f32, vdivq_f32, vdup_n_u16, vdup_n_u64, vdupq_n_f32,
    vdupq_n_s32, vdupq_n_u16, vdupq_n_u32, vget_high_s16, vget_lane_u64, vget_low_s16,
    vgetq_lane_s32, vgetq_lane_u32, vld1q_f32, vmaxnmq_f32, vmaxvq_u32, vminnmq_f32, vminvq_u32,
    vmlal_s16, vmlaq_s16, vmovl_u16, vmovn_u32, vmulq_f32, vmulq_laneq_f32, vmvnq_u32, vorrq_u32,
    vreinterpret_s16_u16, vreinterpret_u16_u64, vreinterpret_u64_u16, vreinterpretq_f32_f64,
    vreinterpretq_f32_u32, vreinterpretq_f64_f32, vreinterpretq_s16_u16, vreinterpretq_u16_u32,
    vreinterpretq_u32_f32, vreinterpretq_u32_s32, vrndaq_f32, vrndmq_f32, vset_lane_u16,
    vsetq_lane_f32, vsetq_lane_u16, vsetq_lane_u32, vshll_n_s16, vshlq_n_s16, vshlq_n_u32,
    vshrq_n_u16, vshrq_n_u32, vst1q_f32, vsub_s16, vsubq_f32, vsubq_s16, vtrn1q_f32, vtrn1q_f64,
    vtrn2q_f32, vtrn2q_f64, vuzp1q_u16, vuzp2q_u16,
};

use super::rust::{self, TexSample, BLEND_ADDITIVE_ALPHA, BLEND_ALPHA, BLEND_INV_SRC_COLOUR};
use super::{
    pack_rgb565_rgb, resolve_xy, BlendMode, DrawPlan, InteriorKernel, PreparedTexture, SpanFn,
    TexFormat, INV_15, INV_31, INV_63,
};
use crate::raster::{Combine, States, PLANES};

const INV_31_Q16: f32 = INV_31 / 65_536.0;
const INV_63_Q16: f32 = INV_63 / 65_536.0;
const INV_15_Q16: f32 = INV_15 / 65_536.0;
const INV_65536: f32 = 1.0 / 65_536.0;

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
        InteriorKernel::TexturedModulatePaletted8KeyedOpaque {
            linear,
            pot_wrap,
            z_test,
            flat,
        } => resolve_paletted8_keyed_opaque(linear, pot_wrap, z_test, flat),
        InteriorKernel::TexturedModulatePaletted8KeyedBlend {
            linear,
            z_le,
            flat,
            additive,
        } => resolve_paletted8_keyed_blend(linear, z_le, flat, additive),
        InteriorKernel::FastRgb565 => rust::span_fast_rgb565,
        InteriorKernel::General => rust::span_general_in_bounds,
    }
}

#[inline(always)]
unsafe fn load_u16x4(ptr: *const u8) -> uint16x4_t {
    let bits = core::ptr::read_unaligned(ptr.cast::<u64>());
    vreinterpret_u16_u64(vdup_n_u64(bits))
}

#[inline(always)]
unsafe fn store_u16x4(ptr: *mut u8, v: uint16x4_t) {
    let bits = vget_lane_u64::<0>(vreinterpret_u64_u16(v));
    core::ptr::write_unaligned(ptr.cast::<u64>(), bits);
}

#[inline(always)]
unsafe fn step4_transpose(
    state: &mut float32x4_t,
    step: float32x4_t,
) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
    let p0 = *state;
    let p1 = vaddq_f32(p0, step);
    let p2 = vaddq_f32(p1, step);
    let p3 = vaddq_f32(p2, step);
    *state = vaddq_f32(p3, step);

    let t0 = vtrn1q_f32(p0, p1);
    let t1 = vtrn2q_f32(p0, p1);
    let t2 = vtrn1q_f32(p2, p3);
    let t3 = vtrn2q_f32(p2, p3);

    let c0 = vreinterpretq_f32_f64(vtrn1q_f64(
        vreinterpretq_f64_f32(t0),
        vreinterpretq_f64_f32(t2),
    ));
    let c1 = vreinterpretq_f32_f64(vtrn1q_f64(
        vreinterpretq_f64_f32(t1),
        vreinterpretq_f64_f32(t3),
    ));
    let c2 = vreinterpretq_f32_f64(vtrn2q_f64(
        vreinterpretq_f64_f32(t0),
        vreinterpretq_f64_f32(t2),
    ));
    let c3 = vreinterpretq_f32_f64(vtrn2q_f64(
        vreinterpretq_f64_f32(t1),
        vreinterpretq_f64_f32(t3),
    ));
    (c0, c1, c2, c3)
}

#[inline(always)]
unsafe fn transpose4(
    p0: float32x4_t,
    p1: float32x4_t,
    p2: float32x4_t,
    p3: float32x4_t,
) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
    let t0 = vtrn1q_f32(p0, p1);
    let t1 = vtrn2q_f32(p0, p1);
    let t2 = vtrn1q_f32(p2, p3);
    let t3 = vtrn2q_f32(p2, p3);
    let c0 = vreinterpretq_f32_f64(vtrn1q_f64(
        vreinterpretq_f64_f32(t0),
        vreinterpretq_f64_f32(t2),
    ));
    let c1 = vreinterpretq_f32_f64(vtrn1q_f64(
        vreinterpretq_f64_f32(t1),
        vreinterpretq_f64_f32(t3),
    ));
    let c2 = vreinterpretq_f32_f64(vtrn2q_f64(
        vreinterpretq_f64_f32(t0),
        vreinterpretq_f64_f32(t2),
    ));
    let c3 = vreinterpretq_f32_f64(vtrn2q_f64(
        vreinterpretq_f64_f32(t1),
        vreinterpretq_f64_f32(t3),
    ));
    (c0, c1, c2, c3)
}

#[inline(always)]
unsafe fn step4_raw(
    state: &mut float32x4_t,
    step: float32x4_t,
) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
    let p0 = *state;
    let p1 = vaddq_f32(p0, step);
    let p2 = vaddq_f32(p1, step);
    let p3 = vaddq_f32(p2, step);
    *state = vaddq_f32(p3, step);
    (p0, p1, p2, p3)
}

#[inline(always)]
unsafe fn persp4(wv: float32x4_t) -> float32x4_t {
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    let pos = vcgtq_f32(wv, zero);
    let inv = vdivq_f32(one, wv);
    vbslq_f32(pos, inv, zero)
}

/// Exact `(sa.clamp(0.0, 1.0) * 255.0).round() as u8 > reference` across 4 lanes.
/// `vrndaq_f32` (`FRINTA`) is exact IEEE-754 round-half-away-from-zero, and
/// `vminnmq_f32(vmaxnmq_f32(sa, 0.0), 1.0)` maps `NaN` to `0.0`, matching Rust `as u8`.
#[inline(always)]
unsafe fn alpha_gt_mask4(sa: float32x4_t, reference: u8) -> uint32x4_t {
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    let v255 = vdupq_n_f32(255.0);
    let clamped = vminnmq_f32(vmaxnmq_f32(sa, zero), one);
    let alpha_u32 = vcvtq_u32_f32(vrndaq_f32(vmulq_f32(clamped, v255)));
    vcgtq_u32(alpha_u32, vdupq_n_u32(u32::from(reference)))
}

#[inline(always)]
unsafe fn unpack_rgb565_4(px: uint16x4_t) -> (float32x4_t, float32x4_t, float32x4_t) {
    let v = vmovl_u16(px);
    let r = vmulq_f32(vcvtq_f32_u32(vshrq_n_u32::<11>(v)), vdupq_n_f32(INV_31));
    let g = vmulq_f32(
        vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<5>(v), vdupq_n_u32(0x3f))),
        vdupq_n_f32(INV_63),
    );
    let b = vmulq_f32(
        vcvtq_f32_u32(vandq_u32(v, vdupq_n_u32(0x1f))),
        vdupq_n_f32(INV_31),
    );
    (r, g, b)
}

#[inline(always)]
unsafe fn pack_rgb565_4(r: float32x4_t, g: float32x4_t, b: float32x4_t) -> uint16x4_t {
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    let v255 = vdupq_n_f32(255.0);
    let half = vdupq_n_f32(0.5);

    let rc = vminnmq_f32(vmaxnmq_f32(r, zero), one);
    let gc = vminnmq_f32(vmaxnmq_f32(g, zero), one);
    let bc = vminnmq_f32(vmaxnmq_f32(b, zero), one);

    // Separate mul then add (ADR-0009: no FMA).
    let ri = vcvtq_u32_f32(vaddq_f32(vmulq_f32(rc, v255), half));
    let gi = vcvtq_u32_f32(vaddq_f32(vmulq_f32(gc, v255), half));
    let bi = vcvtq_u32_f32(vaddq_f32(vmulq_f32(bc, v255), half));

    let packed = vorrq_u32(
        vorrq_u32(
            vshlq_n_u32::<11>(vshrq_n_u32::<3>(ri)),
            vshlq_n_u32::<5>(vshrq_n_u32::<2>(gi)),
        ),
        vshrq_n_u32::<3>(bi),
    );
    vmovn_u32(packed)
}

#[inline(always)]
unsafe fn pack_rgb565_4_unclamped(r: float32x4_t, g: float32x4_t, b: float32x4_t) -> uint16x4_t {
    let v255 = vdupq_n_f32(255.0);
    let half = vdupq_n_f32(0.5);

    // Separate mul then add (ADR-0009: no FMA). Caller guarantees finite inputs in [0.0, 1.0].
    let ri = vcvtq_u32_f32(vaddq_f32(vmulq_f32(r, v255), half));
    let gi = vcvtq_u32_f32(vaddq_f32(vmulq_f32(g, v255), half));
    let bi = vcvtq_u32_f32(vaddq_f32(vmulq_f32(b, v255), half));

    let packed = vorrq_u32(
        vorrq_u32(
            vshlq_n_u32::<11>(vshrq_n_u32::<3>(ri)),
            vshlq_n_u32::<5>(vshrq_n_u32::<2>(gi)),
        ),
        vshrq_n_u32::<3>(bi),
    );
    vmovn_u32(packed)
}

#[inline(always)]
unsafe fn pack_f32x4(a: f32, b: f32, c: f32, d: f32) -> float32x4_t {
    let v = vdupq_n_f32(a);
    let v = vsetq_lane_f32::<1>(b, v);
    let v = vsetq_lane_f32::<2>(c, v);
    vsetq_lane_f32::<3>(d, v)
}

#[inline(always)]
unsafe fn all_zero_f32(v: float32x4_t) -> bool {
    vminvq_u32(vceqq_f32(v, vdupq_n_f32(0.0))) != 0
}

#[inline(always)]
unsafe fn all_finite3(r: float32x4_t, g: float32x4_t, b: float32x4_t) -> bool {
    let s = vaddq_f32(vaddq_f32(vsubq_f32(r, r), vsubq_f32(g, g)), vsubq_f32(b, b));
    vminvq_u32(vceqq_f32(s, vdupq_n_f32(0.0))) != 0
}

#[inline(always)]
unsafe fn clamp_nan_preserving_01(x: float32x4_t) -> float32x4_t {
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    let f = vbslq_f32(vcltq_f32(x, zero), zero, x);
    vbslq_f32(vcgtq_f32(f, one), one, f)
}

#[inline(always)]
unsafe fn combine_4(how: Combine, t: float32x4_t, d: float32x4_t) -> float32x4_t {
    match how {
        Combine::Texture => t,
        Combine::Diffuse => d,
        Combine::Modulate => vmulq_f32(t, d),
        Combine::Add => vminnmq_f32(vaddq_f32(t, d), vdupq_n_f32(1.0)),
    }
}

#[inline(always)]
unsafe fn extract_u32x4_as_usize(v: uint32x4_t) -> [usize; 4] {
    [
        vgetq_lane_u32::<0>(v) as usize,
        vgetq_lane_u32::<1>(v) as usize,
        vgetq_lane_u32::<2>(v) as usize,
        vgetq_lane_u32::<3>(v) as usize,
    ]
}

#[inline(always)]
unsafe fn resolve_xy4<const POT: bool>(
    tex: &PreparedTexture<'_>,
    xv: int32x4_t,
    yv: int32x4_t,
) -> ([usize; 4], [usize; 4]) {
    if POT {
        let xm = vreinterpretq_u32_s32(vandq_s32(xv, vdupq_n_s32(tex.pot_x_mask)));
        let ym = vreinterpretq_u32_s32(vandq_s32(yv, vdupq_n_s32(tex.pot_y_mask)));
        (extract_u32x4_as_usize(xm), extract_u32x4_as_usize(ym))
    } else {
        let (x0, y0) = resolve_xy::<false>(tex, vgetq_lane_s32::<0>(xv), vgetq_lane_s32::<0>(yv));
        let (x1, y1) = resolve_xy::<false>(tex, vgetq_lane_s32::<1>(xv), vgetq_lane_s32::<1>(yv));
        let (x2, y2) = resolve_xy::<false>(tex, vgetq_lane_s32::<2>(xv), vgetq_lane_s32::<2>(yv));
        let (x3, y3) = resolve_xy::<false>(tex, vgetq_lane_s32::<3>(xv), vgetq_lane_s32::<3>(yv));
        ([x0, x1, x2, x3], [y0, y1, y2, y3])
    }
}

#[inline(always)]
unsafe fn read_u16_le(base: *const u8, offset: usize) -> u16 {
    u16::from_le(core::ptr::read_unaligned(base.add(offset).cast::<u16>()))
}

#[inline(always)]
unsafe fn read_u32_le(base: *const u8, offset: usize) -> u32 {
    u32::from_le(core::ptr::read_unaligned(base.add(offset).cast::<u32>()))
}

#[inline(always)]
unsafe fn gather_u16_4(tex: &PreparedTexture<'_>, xs: [usize; 4], ys: [usize; 4]) -> uint32x4_t {
    debug_assert!(tex.in_bounds);
    let base = tex.bytes.as_ptr();
    let pitch = tex.pitch;
    let v = vdup_n_u16(read_u16_le(base, ys[0] * pitch + xs[0] * 2));
    let v = vset_lane_u16::<1>(read_u16_le(base, ys[1] * pitch + xs[1] * 2), v);
    let v = vset_lane_u16::<2>(read_u16_le(base, ys[2] * pitch + xs[2] * 2), v);
    let v = vset_lane_u16::<3>(read_u16_le(base, ys[3] * pitch + xs[3] * 2), v);
    vmovl_u16(v)
}

#[inline(always)]
unsafe fn load_bilinear16_pairs(
    base: *const u8,
    up_rows: [usize; 4],
    lo_rows: [usize; 4],
    left: [usize; 4],
) -> (uint16x8_t, uint16x8_t) {
    let up = vdupq_n_u32(read_u32_le(base, up_rows[0] + left[0] * 2));
    let up = vsetq_lane_u32::<1>(read_u32_le(base, up_rows[1] + left[1] * 2), up);
    let up = vsetq_lane_u32::<2>(read_u32_le(base, up_rows[2] + left[2] * 2), up);
    let up = vsetq_lane_u32::<3>(read_u32_le(base, up_rows[3] + left[3] * 2), up);

    let lo = vdupq_n_u32(read_u32_le(base, lo_rows[0] + left[0] * 2));
    let lo = vsetq_lane_u32::<1>(read_u32_le(base, lo_rows[1] + left[1] * 2), lo);
    let lo = vsetq_lane_u32::<2>(read_u32_le(base, lo_rows[2] + left[2] * 2), lo);
    let lo = vsetq_lane_u32::<3>(read_u32_le(base, lo_rows[3] + left[3] * 2), lo);

    let up16 = vreinterpretq_u16_u32(up);
    let lo16 = vreinterpretq_u16_u32(lo);
    (vuzp1q_u16(up16, lo16), vuzp2q_u16(up16, lo16))
}

#[inline(always)]
unsafe fn load_bilinear16_seam<const POT: bool>(
    tex: &PreparedTexture<'_>,
    x0: int32x4_t,
    y0: int32x4_t,
) -> (uint16x8_t, uint16x8_t) {
    let one_i = vdupq_n_s32(1);
    let (left, upper) = resolve_xy4::<POT>(tex, x0, y0);
    let (right, lower) = resolve_xy4::<POT>(tex, vaddq_s32(x0, one_i), vaddq_s32(y0, one_i));
    let base = tex.bytes.as_ptr();
    let pitch = tex.pitch;
    let u0 = upper[0] * pitch;
    let u1 = upper[1] * pitch;
    let u2 = upper[2] * pitch;
    let u3 = upper[3] * pitch;
    let l0 = lower[0] * pitch;
    let l1 = lower[1] * pitch;
    let l2 = lower[2] * pitch;
    let l3 = lower[3] * pitch;

    let l = vdupq_n_u16(read_u16_le(base, u0 + left[0] * 2));
    let l = vsetq_lane_u16::<1>(read_u16_le(base, u1 + left[1] * 2), l);
    let l = vsetq_lane_u16::<2>(read_u16_le(base, u2 + left[2] * 2), l);
    let l = vsetq_lane_u16::<3>(read_u16_le(base, u3 + left[3] * 2), l);
    let l = vsetq_lane_u16::<4>(read_u16_le(base, l0 + left[0] * 2), l);
    let l = vsetq_lane_u16::<5>(read_u16_le(base, l1 + left[1] * 2), l);
    let l = vsetq_lane_u16::<6>(read_u16_le(base, l2 + left[2] * 2), l);
    let l = vsetq_lane_u16::<7>(read_u16_le(base, l3 + left[3] * 2), l);

    let r = vdupq_n_u16(read_u16_le(base, u0 + right[0] * 2));
    let r = vsetq_lane_u16::<1>(read_u16_le(base, u1 + right[1] * 2), r);
    let r = vsetq_lane_u16::<2>(read_u16_le(base, u2 + right[2] * 2), r);
    let r = vsetq_lane_u16::<3>(read_u16_le(base, u3 + right[3] * 2), r);
    let r = vsetq_lane_u16::<4>(read_u16_le(base, l0 + right[0] * 2), r);
    let r = vsetq_lane_u16::<5>(read_u16_le(base, l1 + right[1] * 2), r);
    let r = vsetq_lane_u16::<6>(read_u16_le(base, l2 + right[2] * 2), r);
    let r = vsetq_lane_u16::<7>(read_u16_le(base, l3 + right[3] * 2), r);

    (l, r)
}

/// Load the 4×(2×2) bilinear corners packed as `left8 = [t00; t01]` and
/// `right8 = [t10; t11]`, plus exact integer weights `fx8 = [fx; fx]` and `fy4`.
/// Non-seam quads (`left != mask_x` / `right == left + 1`) issue 8 paired `u32`
/// loads and deinterleave via `vuzp1q_u16`/`vuzp2q_u16`.
#[inline(always)]
unsafe fn bilinear16_load_pairs<const POT: bool>(
    tex: &PreparedTexture<'_>,
    u: float32x4_t,
    v: float32x4_t,
) -> (uint16x8_t, uint16x8_t, int16x8_t, int16x4_t) {
    debug_assert!(tex.in_bounds);
    let half = vdupq_n_f32(0.5);
    let x = vsubq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f)), half);
    let y = vsubq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f)), half);
    let x0_f = vrndmq_f32(x);
    let y0_f = vrndmq_f32(y);
    let v256_f = vdupq_n_f32(256.0);
    let fx_u32 = vcvtq_u32_f32(vmulq_f32(vsubq_f32(x, x0_f), v256_f));
    let fy_u32 = vcvtq_u32_f32(vmulq_f32(vsubq_f32(y, y0_f), v256_f));
    let fx4 = vreinterpret_s16_u16(vmovn_u32(fx_u32));
    let fy4 = vreinterpret_s16_u16(vmovn_u32(fy_u32));
    let fx8 = vcombine_s16(fx4, fx4);
    let x0 = vcvtq_s32_f32(x0_f);
    let y0 = vcvtq_s32_f32(y0_f);

    let (left8, right8) = if POT {
        let mask_x_v = vdupq_n_s32(tex.pot_x_mask);
        let left_v = vandq_s32(x0, mask_x_v);
        if vmaxvq_u32(vceqq_s32(left_v, mask_x_v)) == 0 {
            let mask_y_v = vdupq_n_s32(tex.pot_y_mask);
            let upper_v = vandq_s32(y0, mask_y_v);
            let lower_v = vandq_s32(vaddq_s32(y0, vdupq_n_s32(1)), mask_y_v);
            let left = extract_u32x4_as_usize(vreinterpretq_u32_s32(left_v));
            let upper = extract_u32x4_as_usize(vreinterpretq_u32_s32(upper_v));
            let lower = extract_u32x4_as_usize(vreinterpretq_u32_s32(lower_v));
            let pitch = tex.pitch;
            load_bilinear16_pairs(
                tex.bytes.as_ptr(),
                [
                    upper[0] * pitch,
                    upper[1] * pitch,
                    upper[2] * pitch,
                    upper[3] * pitch,
                ],
                [
                    lower[0] * pitch,
                    lower[1] * pitch,
                    lower[2] * pitch,
                    lower[3] * pitch,
                ],
                left,
            )
        } else {
            load_bilinear16_seam::<true>(tex, x0, y0)
        }
    } else {
        let one_i = vdupq_n_s32(1);
        let (left, upper) = resolve_xy4::<false>(tex, x0, y0);
        let (right, lower) = resolve_xy4::<false>(tex, vaddq_s32(x0, one_i), vaddq_s32(y0, one_i));
        let non_seam = (left[0] + 1 == right[0])
            & (left[1] + 1 == right[1])
            & (left[2] + 1 == right[2])
            & (left[3] + 1 == right[3]);
        if non_seam {
            let pitch = tex.pitch;
            load_bilinear16_pairs(
                tex.bytes.as_ptr(),
                [
                    upper[0] * pitch,
                    upper[1] * pitch,
                    upper[2] * pitch,
                    upper[3] * pitch,
                ],
                [
                    lower[0] * pitch,
                    lower[1] * pitch,
                    lower[2] * pitch,
                    lower[3] * pitch,
                ],
                left,
            )
        } else {
            load_bilinear16_seam::<false>(tex, x0, y0)
        }
    };
    (left8, right8, fx8, fy4)
}

/// Algebraically bit-exact separable 16-bit integer bilinear interpolation for
/// channels in `0..=63` and weights `fx, fy` in `0..=256`:
/// `top = (c00 << 8) + (c10 - c00) * fx`, `bot = (c01 << 8) + (c11 - c01) * fx`,
/// `res = (top << 8) + (bot - top) * fy` (`0..=4_128_768`).
#[inline(always)]
unsafe fn bilinear_channel_s16(
    c_left: uint16x8_t,
    c_right: uint16x8_t,
    fx8: int16x8_t,
    fy4: int16x4_t,
) -> float32x4_t {
    let cl = vreinterpretq_s16_u16(c_left);
    let cr = vreinterpretq_s16_u16(c_right);
    let tb = vmlaq_s16(vshlq_n_s16::<8>(cl), vsubq_s16(cr, cl), fx8);
    let top = vget_low_s16(tb);
    let bot = vget_high_s16(tb);
    let res = vmlal_s16(vshll_n_s16::<8>(top), vsub_s16(bot, top), fy4);
    vcvtq_f32_s32(res)
}

trait TexSample4 {
    type Scalar: TexSample;
    /// True when `sample4` always returns finite RGB values strictly in `[0.0, 1.0]`
    /// for all UV inputs (including `NaN` / `±inf`), permitting unclamped RGB565 pack.
    const UNIT_RANGE: bool;
    const MAY_KEY: bool = false;
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t);
    #[inline(always)]
    unsafe fn sample4_keyed(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (
        float32x4_t,
        float32x4_t,
        float32x4_t,
        float32x4_t,
        uint32x4_t,
    ) {
        let (r, g, b, a) = Self::sample4(plan, tex, u, v);
        (r, g, b, a, vdupq_n_u32(u32::MAX))
    }
}

struct NearestRgb565<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestRgb565<POT> {
    type Scalar = rust::NearestRgb565<POT>;
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let xi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f))));
        let yi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f))));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let raw = gather_u16_4(tex, xs, ys);
        let r = vmulq_f32(vcvtq_f32_u32(vshrq_n_u32::<11>(raw)), vdupq_n_f32(INV_31));
        let g = vmulq_f32(
            vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<5>(raw), vdupq_n_u32(0x3f))),
            vdupq_n_f32(INV_63),
        );
        let b = vmulq_f32(
            vcvtq_f32_u32(vandq_u32(raw, vdupq_n_u32(0x1f))),
            vdupq_n_f32(INV_31),
        );
        (r, g, b, vdupq_n_f32(1.0))
    }
}

struct BilinearRgb565<const POT: bool>;
impl<const POT: bool> TexSample4 for BilinearRgb565<POT> {
    type Scalar = rust::BilinearRgb565<POT>;
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (left8, right8, fx8, fy4) = bilinear16_load_pairs::<POT>(tex, u, v);
        let m3f = vdupq_n_u16(0x3f);
        let m1f = vdupq_n_u16(0x1f);
        let r = bilinear_channel_s16(
            vshrq_n_u16::<11>(left8),
            vshrq_n_u16::<11>(right8),
            fx8,
            fy4,
        );
        let g = bilinear_channel_s16(
            vandq_u16(vshrq_n_u16::<5>(left8), m3f),
            vandq_u16(vshrq_n_u16::<5>(right8), m3f),
            fx8,
            fy4,
        );
        let b = bilinear_channel_s16(vandq_u16(left8, m1f), vandq_u16(right8, m1f), fx8, fy4);
        let inv31 = vdupq_n_f32(INV_31_Q16);
        (
            vmulq_f32(r, inv31),
            vmulq_f32(g, vdupq_n_f32(INV_63_Q16)),
            vmulq_f32(b, inv31),
            vdupq_n_f32(1.0),
        )
    }
}

struct NearestArgb1555<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestArgb1555<POT> {
    type Scalar = rust::NearestArgb1555<POT>;
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let xi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f))));
        let yi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f))));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let raw = gather_u16_4(tex, xs, ys);
        let m1f = vdupq_n_u32(0x1f);
        let inv31 = vdupq_n_f32(INV_31);
        let r = vmulq_f32(vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<10>(raw), m1f)), inv31);
        let g = vmulq_f32(vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<5>(raw), m1f)), inv31);
        let b = vmulq_f32(vcvtq_f32_u32(vandq_u32(raw, m1f)), inv31);
        let a = vcvtq_f32_u32(vshrq_n_u32::<15>(raw));
        (r, g, b, a)
    }
}

struct BilinearArgb1555<const POT: bool>;
impl<const POT: bool> TexSample4 for BilinearArgb1555<POT> {
    type Scalar = rust::BilinearArgb1555<POT>;
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (left8, right8, fx8, fy4) = bilinear16_load_pairs::<POT>(tex, u, v);
        let m1f = vdupq_n_u16(0x1f);
        let r = bilinear_channel_s16(
            vandq_u16(vshrq_n_u16::<10>(left8), m1f),
            vandq_u16(vshrq_n_u16::<10>(right8), m1f),
            fx8,
            fy4,
        );
        let g = bilinear_channel_s16(
            vandq_u16(vshrq_n_u16::<5>(left8), m1f),
            vandq_u16(vshrq_n_u16::<5>(right8), m1f),
            fx8,
            fy4,
        );
        let b = bilinear_channel_s16(vandq_u16(left8, m1f), vandq_u16(right8, m1f), fx8, fy4);
        let a = bilinear_channel_s16(
            vshrq_n_u16::<15>(left8),
            vshrq_n_u16::<15>(right8),
            fx8,
            fy4,
        );
        let inv31 = vdupq_n_f32(INV_31_Q16);
        (
            vmulq_f32(r, inv31),
            vmulq_f32(g, inv31),
            vmulq_f32(b, inv31),
            vmulq_f32(a, vdupq_n_f32(INV_65536)),
        )
    }
}

struct NearestArgb4444<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestArgb4444<POT> {
    type Scalar = rust::NearestArgb4444<POT>;
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let xi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f))));
        let yi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f))));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let raw = gather_u16_4(tex, xs, ys);
        let mf = vdupq_n_u32(0xf);
        let inv15 = vdupq_n_f32(INV_15);
        let r = vmulq_f32(vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<8>(raw), mf)), inv15);
        let g = vmulq_f32(vcvtq_f32_u32(vandq_u32(vshrq_n_u32::<4>(raw), mf)), inv15);
        let b = vmulq_f32(vcvtq_f32_u32(vandq_u32(raw, mf)), inv15);
        let a = vmulq_f32(vcvtq_f32_u32(vshrq_n_u32::<12>(raw)), inv15);
        (r, g, b, a)
    }
}

struct BilinearArgb4444<const POT: bool>;
impl<const POT: bool> TexSample4 for BilinearArgb4444<POT> {
    type Scalar = rust::BilinearArgb4444<POT>;
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (left8, right8, fx8, fy4) = bilinear16_load_pairs::<POT>(tex, u, v);
        let mf = vdupq_n_u16(0xf);
        let r = bilinear_channel_s16(
            vandq_u16(vshrq_n_u16::<8>(left8), mf),
            vandq_u16(vshrq_n_u16::<8>(right8), mf),
            fx8,
            fy4,
        );
        let g = bilinear_channel_s16(
            vandq_u16(vshrq_n_u16::<4>(left8), mf),
            vandq_u16(vshrq_n_u16::<4>(right8), mf),
            fx8,
            fy4,
        );
        let b = bilinear_channel_s16(vandq_u16(left8, mf), vandq_u16(right8, mf), fx8, fy4);
        let a = bilinear_channel_s16(
            vshrq_n_u16::<12>(left8),
            vshrq_n_u16::<12>(right8),
            fx8,
            fy4,
        );
        let inv15 = vdupq_n_f32(INV_15_Q16);
        (
            vmulq_f32(r, inv15),
            vmulq_f32(g, inv15),
            vmulq_f32(b, inv15),
            vmulq_f32(a, inv15),
        )
    }
}

#[inline(always)]
unsafe fn pack_u8x4_u32(arr: [u8; 4]) -> uint32x4_t {
    let v = vdupq_n_u32(u32::from(arr[0]));
    let v = vsetq_lane_u32::<1>(u32::from(arr[1]), v);
    let v = vsetq_lane_u32::<2>(u32::from(arr[2]), v);
    vsetq_lane_u32::<3>(u32::from(arr[3]), v)
}

struct NearestPaletted8<const POT: bool, const KEYED: bool = false>;
impl<const POT: bool, const KEYED: bool> TexSample4 for NearestPaletted8<POT, KEYED> {
    type Scalar = rust::NearestPaletted8<POT, KEYED>;
    const UNIT_RANGE: bool = true;
    const MAY_KEY: bool = KEYED;
    #[inline(always)]
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (r, g, b, a, _) = Self::sample4_keyed(plan, tex, u, v);
        (r, g, b, a)
    }
    #[inline(always)]
    unsafe fn sample4_keyed(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (
        float32x4_t,
        float32x4_t,
        float32x4_t,
        float32x4_t,
        uint32x4_t,
    ) {
        debug_assert!(tex.in_bounds);
        let xi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f))));
        let yi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f))));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let pal = plan.palette_f32();
        let base = tex.bytes.as_ptr();
        let pitch = tex.pitch;
        let idx = [
            *base.add(ys[0] * pitch + xs[0]),
            *base.add(ys[1] * pitch + xs[1]),
            *base.add(ys[2] * pitch + xs[2]),
            *base.add(ys[3] * pitch + xs[3]),
        ];
        let alive = if KEYED {
            vmvnq_u32(vceqq_u32(
                pack_u8x4_u32(idx),
                vdupq_n_u32(tex.active_key.unwrap()),
            ))
        } else {
            vdupq_n_u32(u32::MAX)
        };
        let c0 = vld1q_f32(pal[usize::from(idx[0])].as_ptr());
        let c1 = vld1q_f32(pal[usize::from(idx[1])].as_ptr());
        let c2 = vld1q_f32(pal[usize::from(idx[2])].as_ptr());
        let c3 = vld1q_f32(pal[usize::from(idx[3])].as_ptr());
        let (r, g, b, a) = transpose4(c0, c1, c2, c3);
        (r, g, b, a, alive)
    }
}

/// Vectorized 4-pixel bilinear `Paletted8` sampler (and optional colour-key
/// test) matching `rust::BilinearPaletted8` bit-for-bit: exact IEEE-754
/// `floor`, paired `u16` non-seam byte loads, separate `vmulq_laneq_f32` +
/// `vaddq_f32` in `(00, 10, 01, 11)` order (zero FMA), and exact `keyed > 0.5`.
#[inline(always)]
unsafe fn bilinear_paletted8_sample4<const POT: bool, const KEYED: bool>(
    plan: &DrawPlan<'_>,
    tex: &PreparedTexture<'_>,
    u: float32x4_t,
    v: float32x4_t,
) -> (
    float32x4_t,
    float32x4_t,
    float32x4_t,
    float32x4_t,
    uint32x4_t,
) {
    debug_assert!(tex.in_bounds);
    let half = vdupq_n_f32(0.5);
    let one = vdupq_n_f32(1.0);
    let zero = vdupq_n_f32(0.0);

    let x = vsubq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f)), half);
    let y = vsubq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f)), half);
    let x0_f = vrndmq_f32(x);
    let y0_f = vrndmq_f32(y);
    let fx = vsubq_f32(x, x0_f);
    let fy = vsubq_f32(y, y0_f);
    let inv_fx = vsubq_f32(one, fx);
    let inv_fy = vsubq_f32(one, fy);
    let w00 = vmulq_f32(inv_fx, inv_fy);
    let w10 = vmulq_f32(fx, inv_fy);
    let w01 = vmulq_f32(inv_fx, fy);
    let w11 = vmulq_f32(fx, fy);

    let x0 = vcvtq_s32_f32(x0_f);
    let y0 = vcvtq_s32_f32(y0_f);
    let base = tex.bytes.as_ptr();
    let pitch = tex.pitch;

    let pot_non_seam = if POT {
        let mask_x_v = vdupq_n_s32(tex.pot_x_mask);
        let left_v = vandq_s32(x0, mask_x_v);
        if vmaxvq_u32(vceqq_s32(left_v, mask_x_v)) == 0 {
            let mask_y_v = vdupq_n_s32(tex.pot_y_mask);
            let upper_v = vandq_s32(y0, mask_y_v);
            let lower_v = vandq_s32(vaddq_s32(y0, vdupq_n_s32(1)), mask_y_v);
            let left = extract_u32x4_as_usize(vreinterpretq_u32_s32(left_v));
            let upper = extract_u32x4_as_usize(vreinterpretq_u32_s32(upper_v));
            let lower = extract_u32x4_as_usize(vreinterpretq_u32_s32(lower_v));
            let up = [0, 1, 2, 3].map(|k| read_u16_le(base, upper[k] * pitch + left[k]));
            let lo = [0, 1, 2, 3].map(|k| read_u16_le(base, lower[k] * pitch + left[k]));
            Some((
                up.map(|v| v as u8),
                up.map(|v| (v >> 8) as u8),
                lo.map(|v| v as u8),
                lo.map(|v| (v >> 8) as u8),
            ))
        } else {
            None
        }
    } else {
        None
    };

    let (i00, i10, i01, i11) = match pot_non_seam {
        Some(quad) => quad,
        None => {
            let one_i = vdupq_n_s32(1);
            let (left, upper) = resolve_xy4::<POT>(tex, x0, y0);
            let (right, lower) =
                resolve_xy4::<POT>(tex, vaddq_s32(x0, one_i), vaddq_s32(y0, one_i));
            (
                [0, 1, 2, 3].map(|k| *base.add(upper[k] * pitch + left[k])),
                [0, 1, 2, 3].map(|k| *base.add(upper[k] * pitch + right[k])),
                [0, 1, 2, 3].map(|k| *base.add(lower[k] * pitch + left[k])),
                [0, 1, 2, 3].map(|k| *base.add(lower[k] * pitch + right[k])),
            )
        }
    };

    let pal = plan.palette_f32();
    macro_rules! accum_px {
        ($lane:literal) => {{
            let c00 = vld1q_f32(pal[usize::from(i00[$lane])].as_ptr());
            let c10 = vld1q_f32(pal[usize::from(i10[$lane])].as_ptr());
            let c01 = vld1q_f32(pal[usize::from(i01[$lane])].as_ptr());
            let c11 = vld1q_f32(pal[usize::from(i11[$lane])].as_ptr());
            let mut acc = vaddq_f32(zero, vmulq_laneq_f32::<$lane>(c00, w00));
            acc = vaddq_f32(acc, vmulq_laneq_f32::<$lane>(c10, w10));
            acc = vaddq_f32(acc, vmulq_laneq_f32::<$lane>(c01, w01));
            vaddq_f32(acc, vmulq_laneq_f32::<$lane>(c11, w11))
        }};
    }
    let o0 = accum_px!(0);
    let o1 = accum_px!(1);
    let o2 = accum_px!(2);
    let o3 = accum_px!(3);
    let (r, g, b, a) = transpose4(o0, o1, o2, o3);

    let alive = if KEYED {
        let key_v = vdupq_n_u32(tex.active_key.unwrap());
        let m00 = vceqq_u32(pack_u8x4_u32(i00), key_v);
        let m10 = vceqq_u32(pack_u8x4_u32(i10), key_v);
        let m01 = vceqq_u32(pack_u8x4_u32(i01), key_v);
        let m11 = vceqq_u32(pack_u8x4_u32(i11), key_v);
        let mut k_acc = vaddq_f32(
            zero,
            vreinterpretq_f32_u32(vandq_u32(m00, vreinterpretq_u32_f32(w00))),
        );
        k_acc = vaddq_f32(
            k_acc,
            vreinterpretq_f32_u32(vandq_u32(m10, vreinterpretq_u32_f32(w10))),
        );
        k_acc = vaddq_f32(
            k_acc,
            vreinterpretq_f32_u32(vandq_u32(m01, vreinterpretq_u32_f32(w01))),
        );
        k_acc = vaddq_f32(
            k_acc,
            vreinterpretq_f32_u32(vandq_u32(m11, vreinterpretq_u32_f32(w11))),
        );
        vmvnq_u32(vcgtq_f32(k_acc, half))
    } else {
        vdupq_n_u32(u32::MAX)
    };
    (r, g, b, a, alive)
}

struct BilinearPaletted8<const POT: bool, const KEYED: bool = false>;
impl<const POT: bool, const KEYED: bool> TexSample4 for BilinearPaletted8<POT, KEYED> {
    type Scalar = rust::BilinearPaletted8<POT, KEYED>;
    const UNIT_RANGE: bool = false;
    const MAY_KEY: bool = KEYED;
    #[inline(always)]
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (r, g, b, a, _) = bilinear_paletted8_sample4::<POT, KEYED>(plan, tex, u, v);
        (r, g, b, a)
    }
    #[inline(always)]
    unsafe fn sample4_keyed(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (
        float32x4_t,
        float32x4_t,
        float32x4_t,
        float32x4_t,
        uint32x4_t,
    ) {
        bilinear_paletted8_sample4::<POT, KEYED>(plan, tex, u, v)
    }
}

fn span_untextured_flat_opaque<const Z_TEST: bool, const SPECULAR: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    if n < 4 {
        if n != 0 {
            rust::span_untextured_flat_opaque::<Z_TEST, SPECULAR>(
                plan,
                &mut depth[..n],
                &mut pixels[..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
    unsafe {
        span_untextured_flat_opaque_neon::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
unsafe fn span_untextured_flat_opaque_neon<const Z_TEST: bool, const SPECULAR: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let (r, g, b) = if SPECULAR {
        (
            (flat.0[0] + flat.1[0]).min(1.0),
            (flat.0[1] + flat.1[1]).min(1.0),
            (flat.0[2] + flat.1[2]).min(1.0),
        )
    } else {
        (flat.0[0], flat.0[1], flat.0[2])
    };
    let packed_u16 = u16::from_le_bytes(pack_rgb565_rgb(r, g, b));
    let packed4 = vdup_n_u16(packed_u16);
    let p_ptr = pixels.as_mut_ptr();
    if !Z_TEST {
        let mut i = 0;
        while i < n4 {
            store_u16x4(p_ptr.add(i * 2), packed4);
            i += 4;
        }
        if n4 == n {
            return;
        }
    } else {
        let depth_write = states.depth_write;
        let mut z_val = value[0];
        let z_step = step[0];
        let d_ptr = depth.as_mut_ptr();
        let mut i = 0;
        while i < n4 {
            let z0 = z_val;
            z_val += z_step;
            let z1 = z_val;
            z_val += z_step;
            let z2 = z_val;
            z_val += z_step;
            let z3 = z_val;
            z_val += z_step;
            let zv = pack_f32x4(z0, z1, z2, z3);
            let dv = vld1q_f32(d_ptr.add(i));
            let z_mask = vcleq_f32(zv, dv);
            if vmaxvq_u32(z_mask) != 0 {
                if vminvq_u32(z_mask) != 0 {
                    if depth_write {
                        vst1q_f32(d_ptr.add(i), zv);
                    }
                    store_u16x4(p_ptr.add(i * 2), packed4);
                } else {
                    if depth_write {
                        vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
                    }
                    let old_px = load_u16x4(p_ptr.add(i * 2));
                    let mask16 = vmovn_u32(z_mask);
                    store_u16x4(p_ptr.add(i * 2), vbsl_u16(mask16, packed4, old_px));
                }
            }
            i += 4;
        }
        if n4 == n {
            return;
        }
        value[0] = z_val;
    }
    rust::span_untextured_flat_opaque::<Z_TEST, SPECULAR>(
        plan,
        &mut depth[n4..n],
        &mut pixels[n4 * 2..n * 2],
        value,
        step,
        flat,
        states,
    );
}

fn span_untextured_gouraud_opaque<const Z_TEST: bool, const SPECULAR: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    if n < 4 {
        if n != 0 {
            rust::span_untextured_gouraud_opaque::<Z_TEST, SPECULAR>(
                plan,
                &mut depth[..n],
                &mut pixels[..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
    unsafe {
        span_untextured_gouraud_opaque_neon::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
unsafe fn span_untextured_gouraud_opaque_neon<const Z_TEST: bool, const SPECULAR: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let depth_write = states.depth_write;
    let one = vdupq_n_f32(1.0);
    let mut s0 = vld1q_f32(value.as_ptr());
    let d0 = vld1q_f32(step.as_ptr());
    let mut s1 = vld1q_f32(value.as_ptr().add(4));
    let d1 = vld1q_f32(step.as_ptr().add(4));
    let mut s2 = vld1q_f32(value.as_ptr().add(8));
    let d2 = vld1q_f32(step.as_ptr().add(8));
    let d_ptr = depth.as_mut_ptr();
    let p_ptr = pixels.as_mut_ptr();
    let mut i = 0;
    while i < n4 {
        let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
        let (q0, q1, q2, q3) = step4_raw(&mut s1, d1);
        let r2 = if SPECULAR {
            Some(step4_raw(&mut s2, d2))
        } else {
            None
        };
        let (zv, dv, z_mask) = if Z_TEST {
            let t0 = vtrn1q_f32(p0, p1);
            let t2 = vtrn1q_f32(p2, p3);
            let zv = vreinterpretq_f32_f64(vtrn1q_f64(
                vreinterpretq_f64_f32(t0),
                vreinterpretq_f64_f32(t2),
            ));
            let dv = vld1q_f32(d_ptr.add(i));
            let z_mask = vcleq_f32(zv, dv);
            if vmaxvq_u32(z_mask) == 0 {
                i += 4;
                continue;
            }
            (zv, dv, z_mask)
        } else {
            (p0, p0, vdupq_n_u32(u32::MAX))
        };
        let t1 = vtrn2q_f32(p0, p1);
        let t3 = vtrn2q_f32(p2, p3);
        let wv = vreinterpretq_f32_f64(vtrn1q_f64(
            vreinterpretq_f64_f32(t1),
            vreinterpretq_f64_f32(t3),
        ));
        let (rv, gv, bv, _) = transpose4(q0, q1, q2, q3);
        let near = persp4(wv);
        let mut r = vmulq_f32(rv, near);
        let mut g = vmulq_f32(gv, near);
        let mut b = vmulq_f32(bv, near);
        if SPECULAR {
            let (m0, m1, m2, m3) = r2.unwrap();
            let (sr, sg, sb, _) = transpose4(m0, m1, m2, m3);
            r = vminnmq_f32(vaddq_f32(r, vmulq_f32(sr, near)), one);
            g = vminnmq_f32(vaddq_f32(g, vmulq_f32(sg, near)), one);
            b = vminnmq_f32(vaddq_f32(b, vmulq_f32(sb, near)), one);
        }
        let new_px = pack_rgb565_4(r, g, b);
        if Z_TEST {
            if vminvq_u32(z_mask) != 0 {
                if depth_write {
                    vst1q_f32(d_ptr.add(i), zv);
                }
                store_u16x4(p_ptr.add(i * 2), new_px);
            } else {
                if depth_write {
                    vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
                }
                let old_px = load_u16x4(p_ptr.add(i * 2));
                store_u16x4(
                    p_ptr.add(i * 2),
                    vbsl_u16(vmovn_u32(z_mask), new_px, old_px),
                );
            }
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 == n {
        return;
    }
    vst1q_f32(value.as_mut_ptr(), s0);
    vst1q_f32(value.as_mut_ptr().add(4), s1);
    if SPECULAR {
        vst1q_f32(value.as_mut_ptr().add(8), s2);
    }
    rust::span_untextured_gouraud_opaque::<Z_TEST, SPECULAR>(
        plan,
        &mut depth[n4..n],
        &mut pixels[n4 * 2..n * 2],
        value,
        step,
        flat,
        states,
    );
}

fn span_untextured_alpha<const Z_LE: bool, const ALPHA_GT: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    if n < 4 {
        if n != 0 {
            rust::span_untextured_alpha::<Z_LE, ALPHA_GT>(
                plan,
                &mut depth[..n],
                &mut pixels[..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
    unsafe {
        span_untextured_alpha_neon::<Z_LE, ALPHA_GT>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
unsafe fn span_untextured_alpha_neon<const Z_LE: bool, const ALPHA_GT: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let depth_write = states.depth_write;
    let alpha_ref = states.alpha_test.map_or(0, |(_, r)| r);
    let d_ptr = depth.as_mut_ptr();
    let p_ptr = pixels.as_mut_ptr();
    if states.flat {
        let sa = flat.0[3];
        if ALPHA_GT {
            let alpha = (sa.clamp(0.0, 1.0) * 255.0).round() as u8;
            if alpha <= alpha_ref {
                return;
            }
        }
        if !ALPHA_GT
            && !depth_write
            && sa == 0.0
            && flat.0[0].is_finite()
            && flat.0[1].is_finite()
            && flat.0[2].is_finite()
        {
            return;
        }
        let isa = vdupq_n_f32(1.0 - sa);
        let sr_sa = vdupq_n_f32(flat.0[0] * sa);
        let sg_sa = vdupq_n_f32(flat.0[1] * sa);
        let sb_sa = vdupq_n_f32(flat.0[2] * sa);
        let mut z_val = value[0];
        let z_step = step[0];
        let mut i = 0;
        while i < n4 {
            if Z_LE {
                let z0 = z_val;
                z_val += z_step;
                let z1 = z_val;
                z_val += z_step;
                let z2 = z_val;
                z_val += z_step;
                let z3 = z_val;
                z_val += z_step;
                let zv = pack_f32x4(z0, z1, z2, z3);
                let dv = vld1q_f32(d_ptr.add(i));
                let z_mask = vcleq_f32(zv, dv);
                if vmaxvq_u32(z_mask) != 0 {
                    let old_px = load_u16x4(p_ptr.add(i * 2));
                    let (ur, ug, ub) = unpack_rgb565_4(old_px);
                    let out_r = vaddq_f32(sr_sa, vmulq_f32(ur, isa));
                    let out_g = vaddq_f32(sg_sa, vmulq_f32(ug, isa));
                    let out_b = vaddq_f32(sb_sa, vmulq_f32(ub, isa));
                    let new_px = pack_rgb565_4(out_r, out_g, out_b);
                    store_u16x4(
                        p_ptr.add(i * 2),
                        vbsl_u16(vmovn_u32(z_mask), new_px, old_px),
                    );
                    if depth_write {
                        vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
                    }
                }
            } else {
                let old_px = load_u16x4(p_ptr.add(i * 2));
                let (ur, ug, ub) = unpack_rgb565_4(old_px);
                let out_r = vaddq_f32(sr_sa, vmulq_f32(ur, isa));
                let out_g = vaddq_f32(sg_sa, vmulq_f32(ug, isa));
                let out_b = vaddq_f32(sb_sa, vmulq_f32(ub, isa));
                store_u16x4(p_ptr.add(i * 2), pack_rgb565_4(out_r, out_g, out_b));
            }
            i += 4;
        }
        if n4 == n {
            return;
        }
        if Z_LE {
            value[0] = z_val;
        }
    } else {
        let one = vdupq_n_f32(1.0);
        let mut s0 = vld1q_f32(value.as_ptr());
        let d0 = vld1q_f32(step.as_ptr());
        let mut s1 = vld1q_f32(value.as_ptr().add(4));
        let d1 = vld1q_f32(step.as_ptr().add(4));
        let mut i = 0;
        while i < n4 {
            let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
            let (q0, q1, q2, q3) = step4_raw(&mut s1, d1);
            let (zv, dv, mut write_mask) = if Z_LE {
                let t0 = vtrn1q_f32(p0, p1);
                let t2 = vtrn1q_f32(p2, p3);
                let zv = vreinterpretq_f32_f64(vtrn1q_f64(
                    vreinterpretq_f64_f32(t0),
                    vreinterpretq_f64_f32(t2),
                ));
                let dv = vld1q_f32(d_ptr.add(i));
                let z_mask = vcleq_f32(zv, dv);
                if vmaxvq_u32(z_mask) == 0 {
                    i += 4;
                    continue;
                }
                (zv, dv, z_mask)
            } else {
                (p0, p0, vdupq_n_u32(u32::MAX))
            };
            let t1 = vtrn2q_f32(p0, p1);
            let t3 = vtrn2q_f32(p2, p3);
            let wv = vreinterpretq_f32_f64(vtrn1q_f64(
                vreinterpretq_f64_f32(t1),
                vreinterpretq_f64_f32(t3),
            ));
            let (cr, cg, cb, ca) = transpose4(q0, q1, q2, q3);
            let near = persp4(wv);
            let sa = vmulq_f32(ca, near);
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(sa, alpha_ref);
                write_mask = if Z_LE {
                    vandq_u32(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if vmaxvq_u32(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            let r = vmulq_f32(cr, near);
            let g = vmulq_f32(cg, near);
            let b = vmulq_f32(cb, near);
            if !ALPHA_GT && !depth_write && all_zero_f32(sa) && all_finite3(r, g, b) {
                i += 4;
                continue;
            }
            let isa = vsubq_f32(one, sa);
            let old_px = load_u16x4(p_ptr.add(i * 2));
            let (ur, ug, ub) = unpack_rgb565_4(old_px);
            let out_r = vaddq_f32(vmulq_f32(r, sa), vmulq_f32(ur, isa));
            let out_g = vaddq_f32(vmulq_f32(g, sa), vmulq_f32(ug, isa));
            let out_b = vaddq_f32(vmulq_f32(b, sa), vmulq_f32(ub, isa));
            let new_px = pack_rgb565_4(out_r, out_g, out_b);
            if Z_LE || ALPHA_GT {
                store_u16x4(
                    p_ptr.add(i * 2),
                    vbsl_u16(vmovn_u32(write_mask), new_px, old_px),
                );
                if Z_LE && depth_write {
                    vst1q_f32(d_ptr.add(i), vbslq_f32(write_mask, zv, dv));
                }
            } else {
                store_u16x4(p_ptr.add(i * 2), new_px);
            }
            i += 4;
        }
        if n4 == n {
            return;
        }
        vst1q_f32(value.as_mut_ptr(), s0);
        vst1q_f32(value.as_mut_ptr().add(4), s1);
    }
    rust::span_untextured_alpha::<Z_LE, ALPHA_GT>(
        plan,
        &mut depth[n4..n],
        &mut pixels[n4 * 2..n * 2],
        value,
        step,
        flat,
        states,
    );
}

fn span_textured_copy_opaque<S: TexSample4, const Z_TEST: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    if n < 4 {
        if n != 0 {
            rust::span_textured_copy_opaque::<S::Scalar, Z_TEST>(
                plan,
                &mut depth[..n],
                &mut pixels[..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
    unsafe {
        span_textured_copy_opaque_neon::<S, Z_TEST>(plan, depth, pixels, value, step, flat, states);
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
unsafe fn span_textured_copy_opaque_neon<S: TexSample4, const Z_TEST: bool>(
    plan: &DrawPlan<'_>,
    depth: &mut [f32],
    pixels: &mut [u8],
    value: &mut [f32; PLANES],
    step: &[f32; PLANES],
    flat: ([f32; 4], [f32; 3]),
    states: &States,
) {
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
    let mut s0 = vld1q_f32(value.as_ptr());
    let d0 = vld1q_f32(step.as_ptr());
    let d_ptr = depth.as_mut_ptr();
    let p_ptr = pixels.as_mut_ptr();
    let mut i = 0;
    while i < n4 {
        if Z_TEST {
            let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
            let t0 = vtrn1q_f32(p0, p1);
            let t2 = vtrn1q_f32(p2, p3);
            let zv = vreinterpretq_f32_f64(vtrn1q_f64(
                vreinterpretq_f64_f32(t0),
                vreinterpretq_f64_f32(t2),
            ));
            let dv = vld1q_f32(d_ptr.add(i));
            let z_mask = vcleq_f32(zv, dv);
            if vmaxvq_u32(z_mask) != 0 {
                let t1 = vtrn2q_f32(p0, p1);
                let t3 = vtrn2q_f32(p2, p3);
                let wv = vreinterpretq_f32_f64(vtrn1q_f64(
                    vreinterpretq_f64_f32(t1),
                    vreinterpretq_f64_f32(t3),
                ));
                let uv = vreinterpretq_f32_f64(vtrn2q_f64(
                    vreinterpretq_f64_f32(t0),
                    vreinterpretq_f64_f32(t2),
                ));
                let vv = vreinterpretq_f32_f64(vtrn2q_f64(
                    vreinterpretq_f64_f32(t1),
                    vreinterpretq_f64_f32(t3),
                ));
                let near = persp4(wv);
                let (tr, tg, tb, _) =
                    S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));
                let new_px = if S::UNIT_RANGE {
                    pack_rgb565_4_unclamped(tr, tg, tb)
                } else {
                    pack_rgb565_4(tr, tg, tb)
                };
                if vminvq_u32(z_mask) != 0 {
                    if depth_write {
                        vst1q_f32(d_ptr.add(i), zv);
                    }
                    store_u16x4(p_ptr.add(i * 2), new_px);
                } else {
                    if depth_write {
                        vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
                    }
                    let old_px = load_u16x4(p_ptr.add(i * 2));
                    store_u16x4(
                        p_ptr.add(i * 2),
                        vbsl_u16(vmovn_u32(z_mask), new_px, old_px),
                    );
                }
            }
        } else {
            let (_, wv, uv, vv) = step4_transpose(&mut s0, d0);
            let near = persp4(wv);
            let (tr, tg, tb, _) = S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));
            let new_px = if S::UNIT_RANGE {
                pack_rgb565_4_unclamped(tr, tg, tb)
            } else {
                pack_rgb565_4(tr, tg, tb)
            };
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 == n {
        return;
    }
    vst1q_f32(value.as_mut_ptr(), s0);
    rust::span_textured_copy_opaque::<S::Scalar, Z_TEST>(
        plan,
        &mut depth[n4..n],
        &mut pixels[n4 * 2..n * 2],
        value,
        step,
        flat,
        states,
    );
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

fn span_textured_modulate_opaque<
    S: TexSample4,
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
    let n = depth.len().min(pixels.len() / 2);
    if n < 4 {
        if n != 0 {
            rust::span_textured_modulate_opaque::<S::Scalar, Z_TEST, FLAT, SPECULAR, FOG>(
                plan,
                &mut depth[..n],
                &mut pixels[..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
    unsafe {
        span_textured_modulate_opaque_neon::<S, Z_TEST, FLAT, SPECULAR, FOG>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
unsafe fn span_textured_modulate_opaque_neon<
    S: TexSample4,
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
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
    let flat_r = vdupq_n_f32(flat.0[0]);
    let flat_g = vdupq_n_f32(flat.0[1]);
    let flat_b = vdupq_n_f32(flat.0[2]);
    let flat_sr = vdupq_n_f32(flat.1[0]);
    let flat_sg = vdupq_n_f32(flat.1[1]);
    let flat_sb = vdupq_n_f32(flat.1[2]);
    let fog_colour = states.fog.unwrap_or([0.0; 3]);
    let fog_r = vdupq_n_f32(fog_colour[0]);
    let fog_g = vdupq_n_f32(fog_colour[1]);
    let fog_b = vdupq_n_f32(fog_colour[2]);
    let one = vdupq_n_f32(1.0);

    let mut s0 = vld1q_f32(value.as_ptr());
    let d0 = vld1q_f32(step.as_ptr());
    let mut s1 = vld1q_f32(value.as_ptr().add(4));
    let d1 = vld1q_f32(step.as_ptr().add(4));
    let mut s2 = vld1q_f32(value.as_ptr().add(8));
    let d2 = vld1q_f32(step.as_ptr().add(8));

    let d_ptr = depth.as_mut_ptr();
    let p_ptr = pixels.as_mut_ptr();
    let mut i = 0;
    while i < n4 {
        let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
        let q = if !FLAT {
            Some(step4_raw(&mut s1, d1))
        } else {
            None
        };
        let r2 = if (!FLAT && SPECULAR) || FOG {
            Some(step4_raw(&mut s2, d2))
        } else {
            None
        };

        let t0 = vtrn1q_f32(p0, p1);
        let t2 = vtrn1q_f32(p2, p3);
        let zv = vreinterpretq_f32_f64(vtrn1q_f64(
            vreinterpretq_f64_f32(t0),
            vreinterpretq_f64_f32(t2),
        ));
        let (dv, z_mask) = if Z_TEST {
            let dv = vld1q_f32(d_ptr.add(i));
            let z_mask = vcleq_f32(zv, dv);
            if vmaxvq_u32(z_mask) == 0 {
                i += 4;
                continue;
            }
            (dv, z_mask)
        } else {
            (zv, vdupq_n_u32(u32::MAX))
        };

        let t1 = vtrn2q_f32(p0, p1);
        let t3 = vtrn2q_f32(p2, p3);
        let wv = vreinterpretq_f32_f64(vtrn1q_f64(
            vreinterpretq_f64_f32(t1),
            vreinterpretq_f64_f32(t3),
        ));
        let uv = vreinterpretq_f32_f64(vtrn2q_f64(
            vreinterpretq_f64_f32(t0),
            vreinterpretq_f64_f32(t2),
        ));
        let vv = vreinterpretq_f32_f64(vtrn2q_f64(
            vreinterpretq_f64_f32(t1),
            vreinterpretq_f64_f32(t3),
        ));

        let near = persp4(wv);
        let (tr, tg, tb, write_mask) = if S::MAY_KEY {
            let (tr, tg, tb, _, alive) =
                S::sample4_keyed(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));
            let wm = if Z_TEST {
                vandq_u32(z_mask, alive)
            } else {
                alive
            };
            if vmaxvq_u32(wm) == 0 {
                i += 4;
                continue;
            }
            (tr, tg, tb, wm)
        } else {
            let (tr, tg, tb, _) = S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));
            (tr, tg, tb, z_mask)
        };

        let (mut r, mut g, mut b) = if FLAT {
            (
                vmulq_f32(tr, flat_r),
                vmulq_f32(tg, flat_g),
                vmulq_f32(tb, flat_b),
            )
        } else {
            let (q0, q1, q2, q3) = q.unwrap();
            let (cr, cg, cb, _) = transpose4(q0, q1, q2, q3);
            (
                vmulq_f32(tr, vmulq_f32(cr, near)),
                vmulq_f32(tg, vmulq_f32(cg, near)),
                vmulq_f32(tb, vmulq_f32(cb, near)),
            )
        };

        if FLAT && SPECULAR {
            r = vminnmq_f32(vaddq_f32(r, flat_sr), one);
            g = vminnmq_f32(vaddq_f32(g, flat_sg), one);
            b = vminnmq_f32(vaddq_f32(b, flat_sb), one);
        }

        if (!FLAT && SPECULAR) || FOG {
            let (m0, m1, m2, m3) = r2.unwrap();
            let (sr, sg, sb, fw) = transpose4(m0, m1, m2, m3);
            if !FLAT && SPECULAR {
                r = vminnmq_f32(vaddq_f32(r, vmulq_f32(sr, near)), one);
                g = vminnmq_f32(vaddq_f32(g, vmulq_f32(sg, near)), one);
                b = vminnmq_f32(vaddq_f32(b, vmulq_f32(sb, near)), one);
            }
            if FOG {
                let f = clamp_nan_preserving_01(vmulq_f32(fw, near));
                let inv_f = vsubq_f32(one, f);
                r = vaddq_f32(vmulq_f32(r, f), vmulq_f32(fog_r, inv_f));
                g = vaddq_f32(vmulq_f32(g, f), vmulq_f32(fog_g, inv_f));
                b = vaddq_f32(vmulq_f32(b, f), vmulq_f32(fog_b, inv_f));
            }
        }

        let new_px = pack_rgb565_4(r, g, b);
        if Z_TEST || S::MAY_KEY {
            if vminvq_u32(write_mask) != 0 {
                if Z_TEST && depth_write {
                    vst1q_f32(d_ptr.add(i), zv);
                }
                store_u16x4(p_ptr.add(i * 2), new_px);
            } else {
                if Z_TEST && depth_write {
                    vst1q_f32(d_ptr.add(i), vbslq_f32(write_mask, zv, dv));
                }
                let old_px = load_u16x4(p_ptr.add(i * 2));
                store_u16x4(
                    p_ptr.add(i * 2),
                    vbsl_u16(vmovn_u32(write_mask), new_px, old_px),
                );
            }
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }

    if n4 == n {
        return;
    }
    vst1q_f32(value.as_mut_ptr(), s0);
    if !FLAT {
        vst1q_f32(value.as_mut_ptr().add(4), s1);
    }
    if (!FLAT && SPECULAR) || FOG {
        vst1q_f32(value.as_mut_ptr().add(8), s2);
    }
    rust::span_textured_modulate_opaque::<S::Scalar, Z_TEST, FLAT, SPECULAR, FOG>(
        plan,
        &mut depth[n4..n],
        &mut pixels[n4 * 2..n * 2],
        value,
        step,
        flat,
        states,
    );
}

fn resolve_opaque_pot_sampler<S: TexSample4>(
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

fn span_textured_modulate_blend<
    S: TexSample4,
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
    let n = depth.len().min(pixels.len() / 2);
    if n < 4 {
        if n != 0 {
            rust::span_textured_modulate_blend::<S::Scalar, Z_LE, ALPHA_GT, BLEND, FLAT, SPECULAR>(
                plan,
                &mut depth[..n],
                &mut pixels[..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
    unsafe {
        span_textured_modulate_blend_neon::<S, Z_LE, ALPHA_GT, BLEND, FLAT, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
#[target_feature(enable = "neon")]
unsafe fn span_textured_modulate_blend_neon<
    S: TexSample4,
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
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let tex = plan.texture.as_ref().unwrap();
    let flat_r = vdupq_n_f32(flat.0[0]);
    let flat_g = vdupq_n_f32(flat.0[1]);
    let flat_b = vdupq_n_f32(flat.0[2]);
    let flat_a = vdupq_n_f32(flat.0[3]);
    let flat_sr = vdupq_n_f32(flat.1[0]);
    let flat_sg = vdupq_n_f32(flat.1[1]);
    let flat_sb = vdupq_n_f32(flat.1[2]);
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    let alpha_combine = states.alpha;
    let alpha_ref = states.alpha_test.map_or(0, |(_, r)| r);
    let depth_write = states.depth_write;
    let can_skip_transparent =
        !ALPHA_GT && !depth_write && (BLEND == BLEND_ALPHA || BLEND == BLEND_ADDITIVE_ALPHA);
    let flat_rgb_always_finite = FLAT
        && S::UNIT_RANGE
        && flat.0[0].is_finite()
        && flat.0[1].is_finite()
        && flat.0[2].is_finite()
        && (!SPECULAR || (flat.1[0].is_finite() && flat.1[1].is_finite() && flat.1[2].is_finite()));

    let mut s0 = vld1q_f32(value.as_ptr());
    let d0 = vld1q_f32(step.as_ptr());
    let mut s1 = vld1q_f32(value.as_ptr().add(4));
    let d1 = vld1q_f32(step.as_ptr().add(4));
    let mut s2 = vld1q_f32(value.as_ptr().add(8));
    let d2 = vld1q_f32(step.as_ptr().add(8));

    let d_ptr = depth.as_mut_ptr();
    let p_ptr = pixels.as_mut_ptr();
    let mut i = 0;
    while i < n4 {
        let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
        let q = if !FLAT {
            Some(step4_raw(&mut s1, d1))
        } else {
            None
        };
        let r2 = if !FLAT && SPECULAR {
            Some(step4_raw(&mut s2, d2))
        } else {
            None
        };

        let t0 = vtrn1q_f32(p0, p1);
        let t2 = vtrn1q_f32(p2, p3);
        let (zv, dv, mut write_mask) = if Z_LE {
            let zv = vreinterpretq_f32_f64(vtrn1q_f64(
                vreinterpretq_f64_f32(t0),
                vreinterpretq_f64_f32(t2),
            ));
            let dv = vld1q_f32(d_ptr.add(i));
            let z_mask = vcleq_f32(zv, dv);
            if vmaxvq_u32(z_mask) == 0 {
                i += 4;
                continue;
            }
            (zv, dv, z_mask)
        } else {
            (p0, p0, vdupq_n_u32(u32::MAX))
        };

        let t1 = vtrn2q_f32(p0, p1);
        let t3 = vtrn2q_f32(p2, p3);
        let wv = vreinterpretq_f32_f64(vtrn1q_f64(
            vreinterpretq_f64_f32(t1),
            vreinterpretq_f64_f32(t3),
        ));
        let uv = vreinterpretq_f32_f64(vtrn2q_f64(
            vreinterpretq_f64_f32(t0),
            vreinterpretq_f64_f32(t2),
        ));
        let vv = vreinterpretq_f32_f64(vtrn2q_f64(
            vreinterpretq_f64_f32(t1),
            vreinterpretq_f64_f32(t3),
        ));

        let near = persp4(wv);
        let (tr, tg, tb, ta) = if S::MAY_KEY {
            let (tr, tg, tb, ta, alive) =
                S::sample4_keyed(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));
            write_mask = if Z_LE {
                vandq_u32(write_mask, alive)
            } else {
                alive
            };
            if vmaxvq_u32(write_mask) == 0 {
                i += 4;
                continue;
            }
            (tr, tg, tb, ta)
        } else {
            S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near))
        };

        // When alpha comes solely from the texture and alpha_test is active, test
        // alpha before transposing diffuse RGB or loading the framebuffer so fully
        // transparent 4-pixel quads exit early.
        let (sa, diffuse_opt) = if alpha_combine == Combine::Texture {
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(ta, alpha_ref);
                write_mask = if Z_LE || S::MAY_KEY {
                    vandq_u32(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if vmaxvq_u32(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            (ta, None)
        } else if FLAT {
            let sa = combine_4(alpha_combine, ta, flat_a);
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(sa, alpha_ref);
                write_mask = if Z_LE || S::MAY_KEY {
                    vandq_u32(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if vmaxvq_u32(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            (sa, None)
        } else {
            let (q0, q1, q2, q3) = q.unwrap();
            let (cr, cg, cb, ca) = transpose4(q0, q1, q2, q3);
            let sa = combine_4(alpha_combine, ta, vmulq_f32(ca, near));
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(sa, alpha_ref);
                write_mask = if Z_LE || S::MAY_KEY {
                    vandq_u32(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if vmaxvq_u32(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            (sa, Some((cr, cg, cb)))
        };

        if can_skip_transparent && flat_rgb_always_finite && all_zero_f32(sa) {
            i += 4;
            continue;
        }

        let (mut r, mut g, mut b) = if FLAT {
            (
                vmulq_f32(tr, flat_r),
                vmulq_f32(tg, flat_g),
                vmulq_f32(tb, flat_b),
            )
        } else {
            let (cr, cg, cb) = match diffuse_opt {
                Some(rgb) => rgb,
                None => {
                    let (q0, q1, q2, q3) = q.unwrap();
                    let (cr, cg, cb, _) = transpose4(q0, q1, q2, q3);
                    (cr, cg, cb)
                }
            };
            (
                vmulq_f32(tr, vmulq_f32(cr, near)),
                vmulq_f32(tg, vmulq_f32(cg, near)),
                vmulq_f32(tb, vmulq_f32(cb, near)),
            )
        };

        if SPECULAR {
            if FLAT {
                r = vminnmq_f32(vaddq_f32(r, flat_sr), one);
                g = vminnmq_f32(vaddq_f32(g, flat_sg), one);
                b = vminnmq_f32(vaddq_f32(b, flat_sb), one);
            } else {
                let (m0, m1, m2, m3) = r2.unwrap();
                let (sr, sg, sb, _) = transpose4(m0, m1, m2, m3);
                r = vminnmq_f32(vaddq_f32(r, vmulq_f32(sr, near)), one);
                g = vminnmq_f32(vaddq_f32(g, vmulq_f32(sg, near)), one);
                b = vminnmq_f32(vaddq_f32(b, vmulq_f32(sb, near)), one);
            }
        }

        if can_skip_transparent
            && !flat_rgb_always_finite
            && all_zero_f32(sa)
            && all_finite3(r, g, b)
        {
            i += 4;
            continue;
        }

        let old_px = load_u16x4(p_ptr.add(i * 2));
        let (ur, ug, ub) = unpack_rgb565_4(old_px);
        let (out_r, out_g, out_b) = match BLEND {
            BLEND_ALPHA => {
                let isa = vsubq_f32(one, sa);
                (
                    vaddq_f32(vmulq_f32(r, sa), vmulq_f32(ur, isa)),
                    vaddq_f32(vmulq_f32(g, sa), vmulq_f32(ug, isa)),
                    vaddq_f32(vmulq_f32(b, sa), vmulq_f32(ub, isa)),
                )
            }
            BLEND_ADDITIVE_ALPHA => (
                vaddq_f32(vmulq_f32(r, sa), ur),
                vaddq_f32(vmulq_f32(g, sa), ug),
                vaddq_f32(vmulq_f32(b, sa), ub),
            ),
            BLEND_INV_SRC_COLOUR => (
                vaddq_f32(vmulq_f32(r, zero), vmulq_f32(ur, vsubq_f32(one, r))),
                vaddq_f32(vmulq_f32(g, zero), vmulq_f32(ug, vsubq_f32(one, g))),
                vaddq_f32(vmulq_f32(b, zero), vmulq_f32(ub, vsubq_f32(one, b))),
            ),
            _ => unreachable!(),
        };
        let new_px = pack_rgb565_4(out_r, out_g, out_b);
        if Z_LE || ALPHA_GT || S::MAY_KEY {
            store_u16x4(
                p_ptr.add(i * 2),
                vbsl_u16(vmovn_u32(write_mask), new_px, old_px),
            );
            if Z_LE && depth_write {
                vst1q_f32(d_ptr.add(i), vbslq_f32(write_mask, zv, dv));
            }
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }

    if n4 == n {
        return;
    }
    vst1q_f32(value.as_mut_ptr(), s0);
    if !FLAT {
        vst1q_f32(value.as_mut_ptr().add(4), s1);
    }
    if !FLAT && SPECULAR {
        vst1q_f32(value.as_mut_ptr().add(8), s2);
    }
    rust::span_textured_modulate_blend::<S::Scalar, Z_LE, ALPHA_GT, BLEND, FLAT, SPECULAR>(
        plan,
        &mut depth[n4..n],
        &mut pixels[n4 * 2..n * 2],
        value,
        step,
        flat,
        states,
    );
}

#[allow(clippy::too_many_arguments)]
fn resolve_blend_pot_sampler<S: TexSample4>(
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

fn resolve_paletted8_keyed_opaque(
    linear: bool,
    pot_wrap: bool,
    z_test: bool,
    flat: bool,
) -> SpanFn {
    macro_rules! go {
        ($sampler:ty) => {
            match (z_test, flat) {
                (true, false) => {
                    span_textured_modulate_opaque::<$sampler, true, false, false, false>
                }
                (true, true) => span_textured_modulate_opaque::<$sampler, true, true, false, false>,
                (false, false) => {
                    span_textured_modulate_opaque::<$sampler, false, false, false, false>
                }
                (false, true) => {
                    span_textured_modulate_opaque::<$sampler, false, true, false, false>
                }
            }
        };
    }
    match (linear, pot_wrap) {
        (false, true) => go!(NearestPaletted8<true, true>),
        (false, false) => go!(NearestPaletted8<false, true>),
        (true, true) => go!(BilinearPaletted8<true, true>),
        (true, false) => go!(BilinearPaletted8<false, true>),
    }
}

fn resolve_paletted8_keyed_blend(linear: bool, z_le: bool, flat: bool, additive: bool) -> SpanFn {
    macro_rules! go {
        ($sampler:ty) => {
            match (additive, z_le, flat) {
                (false, true, false) => {
                    span_textured_modulate_blend::<$sampler, true, false, BLEND_ALPHA, false, false>
                }
                (false, true, true) => {
                    span_textured_modulate_blend::<$sampler, true, false, BLEND_ALPHA, true, false>
                }
                (false, false, false) => {
                    span_textured_modulate_blend::<$sampler, false, false, BLEND_ALPHA, false, false>
                }
                (false, false, true) => {
                    span_textured_modulate_blend::<$sampler, false, false, BLEND_ALPHA, true, false>
                }
                (true, true, false) => span_textured_modulate_blend::<
                    $sampler,
                    true,
                    false,
                    BLEND_ADDITIVE_ALPHA,
                    false,
                    false,
                >,
                _ => unreachable!("select_kernel only emits supported keyed blend combinations"),
            }
        };
    }
    if linear {
        go!(BilinearPaletted8<true, true>)
    } else {
        go!(NearestPaletted8<true, true>)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Blend, Compare, PixelFormat, Texture};
    use core::arch::aarch64::vgetq_lane_f32;

    #[allow(clippy::too_many_arguments)]
    fn assert_neon_matches_rust(
        plan: &DrawPlan<'_>,
        init_depth: &[f32],
        init_pixels: &[u8],
        byte_offset: usize,
        value: [f32; PLANES],
        step: [f32; PLANES],
        flat: ([f32; 4], [f32; 3]),
        states: &States,
        label: &str,
    ) {
        let rust_fn = rust::resolve_span_fn(plan.kernel);
        let neon_fn = resolve_span_fn(plan.kernel);

        let mut dr = init_depth.to_vec();
        let mut dn = init_depth.to_vec();
        let mut pr = vec![0x5a; byte_offset + init_pixels.len() + 3];
        let mut pn = vec![0x5a; byte_offset + init_pixels.len() + 3];
        pr[byte_offset..byte_offset + init_pixels.len()].copy_from_slice(init_pixels);
        pn[byte_offset..byte_offset + init_pixels.len()].copy_from_slice(init_pixels);

        let mut vr = value;
        let mut vn = value;
        rust_fn(
            plan,
            &mut dr,
            &mut pr[byte_offset..byte_offset + init_pixels.len()],
            &mut vr,
            &step,
            flat,
            states,
        );
        neon_fn(
            plan,
            &mut dn,
            &mut pn[byte_offset..byte_offset + init_pixels.len()],
            &mut vn,
            &step,
            flat,
            states,
        );
        assert_eq!(pr, pn, "pixel mismatch ({label})");
        let br: Vec<u32> = dr.iter().map(|d| d.to_bits()).collect();
        let bn: Vec<u32> = dn.iter().map(|d| d.to_bits()).collect();
        assert_eq!(br, bn, "depth mismatch ({label})");
    }

    #[test]
    fn separable_bilinear_s16_and_unclamped_pack_match_oracle_bit_for_bit() {
        #[target_feature(enable = "neon")]
        unsafe fn run() {
            // 1. Exhaustive horizontal sweep across all (c00, c10, fx) in 0..=63 x 0..=63 x 0..=256.
            let fys: [i16; 5] = [0, 1, 128, 255, 256];
            for c00 in 0u16..=63 {
                let c01 = 63 - c00;
                for c10 in 0u16..=63 {
                    let c11 = 63 - c10;
                    let cl = vsetq_lane_u16::<4>(
                        c01,
                        vsetq_lane_u16::<5>(
                            c01,
                            vsetq_lane_u16::<6>(c01, vsetq_lane_u16::<7>(c01, vdupq_n_u16(c00))),
                        ),
                    );
                    let cr = vsetq_lane_u16::<4>(
                        c11,
                        vsetq_lane_u16::<5>(
                            c11,
                            vsetq_lane_u16::<6>(c11, vsetq_lane_u16::<7>(c11, vdupq_n_u16(c10))),
                        ),
                    );
                    for fx_base in (0i16..=256).step_by(4) {
                        let fxs = [
                            fx_base,
                            (fx_base + 1).min(256),
                            (fx_base + 2).min(256),
                            (fx_base + 3).min(256),
                        ];
                        let mut fx4 = vdup_n_u16(fxs[0] as u16);
                        fx4 = vset_lane_u16::<1>(fxs[1] as u16, fx4);
                        fx4 = vset_lane_u16::<2>(fxs[2] as u16, fx4);
                        fx4 = vset_lane_u16::<3>(fxs[3] as u16, fx4);
                        let fx4 = vreinterpret_s16_u16(fx4);
                        let fx8 = vcombine_s16(fx4, fx4);
                        for &fy in &fys {
                            let fy4 = vreinterpret_s16_u16(vdup_n_u16(fy as u16));
                            let got = bilinear_channel_s16(cl, cr, fx8, fy4);
                            let lanes = [
                                vgetq_lane_f32::<0>(got),
                                vgetq_lane_f32::<1>(got),
                                vgetq_lane_f32::<2>(got),
                                vgetq_lane_f32::<3>(got),
                            ];
                            for k in 0..4 {
                                let fx = fxs[k] as u32;
                                let fyu = fy as u32;
                                let expected = (u32::from(c00) * (256 - fx) * (256 - fyu)
                                    + u32::from(c10) * fx * (256 - fyu)
                                    + u32::from(c01) * (256 - fx) * fyu
                                    + u32::from(c11) * fx * fyu)
                                    as f32;
                                assert_eq!(
                                    lanes[k].to_bits(),
                                    expected.to_bits(),
                                    "c=({c00},{c10},{c01},{c11}) fx={fx} fy={fyu}"
                                );
                            }
                        }
                    }
                }
            }

            // 2. Full 4-corner boundary grid across all (c00, c10, c01, c11, fx, fy) corners.
            let cs: &[u16] = &[0, 1, 15, 16, 31, 32, 62, 63];
            let ws: &[i16] = &[0, 1, 127, 128, 255, 256];
            for &c00 in cs {
                for &c10 in cs {
                    for &c01 in cs {
                        for &c11 in cs {
                            let cl = vsetq_lane_u16::<4>(c01, vdupq_n_u16(c00));
                            let cr = vsetq_lane_u16::<4>(c11, vdupq_n_u16(c10));
                            for &fx in ws {
                                let fx4 = vreinterpret_s16_u16(vdup_n_u16(fx as u16));
                                let fx8 = vcombine_s16(fx4, fx4);
                                for &fy in ws {
                                    let fy4 = vreinterpret_s16_u16(vdup_n_u16(fy as u16));
                                    let got =
                                        vgetq_lane_f32::<0>(bilinear_channel_s16(cl, cr, fx8, fy4));
                                    let fxu = fx as u32;
                                    let fyu = fy as u32;
                                    let expected = (u32::from(c00) * (256 - fxu) * (256 - fyu)
                                        + u32::from(c10) * fxu * (256 - fyu)
                                        + u32::from(c01) * (256 - fxu) * fyu
                                        + u32::from(c11) * fxu * fyu)
                                        as f32;
                                    assert_eq!(got.to_bits(), expected.to_bits());
                                }
                            }
                        }
                    }
                }
            }

            // 3. Exact float RGB565 pack (unclamped vs clamped vs scalar) across all green Q16 values.
            let max_g = 63u32 * 65_536;
            for g_base in (0..=max_g).step_by(4) {
                let gs = [
                    g_base,
                    (g_base + 1).min(max_g),
                    (g_base + 2).min(max_g),
                    (g_base + 3).min(max_g),
                ];
                let gf = pack_f32x4(
                    gs[0] as f32 * INV_63_Q16,
                    gs[1] as f32 * INV_63_Q16,
                    gs[2] as f32 * INV_63_Q16,
                    gs[3] as f32 * INV_63_Q16,
                );
                let rf = pack_f32x4(
                    (gs[0] & 31) as f32 * INV_31,
                    (gs[1] & 31) as f32 * INV_31,
                    (gs[2] & 31) as f32 * INV_31,
                    (gs[3] & 31) as f32 * INV_31,
                );
                let bf = pack_f32x4(
                    ((gs[0] >> 5) & 31) as f32 * INV_31,
                    ((gs[1] >> 5) & 31) as f32 * INV_31,
                    ((gs[2] >> 5) & 31) as f32 * INV_31,
                    ((gs[3] >> 5) & 31) as f32 * INV_31,
                );
                let unclamped = pack_rgb565_4_unclamped(rf, gf, bf);
                let clamped = pack_rgb565_4(rf, gf, bf);
                assert_eq!(
                    vget_lane_u64::<0>(vreinterpret_u64_u16(unclamped)),
                    vget_lane_u64::<0>(vreinterpret_u64_u16(clamped)),
                    "unclamped pack mismatch at g_base={g_base}"
                );
                let mut out = [0u8; 8];
                store_u16x4(out.as_mut_ptr(), unclamped);
                for k in 0..4 {
                    let expected = pack_rgb565_rgb(
                        (gs[k] & 31) as f32 * INV_31,
                        gs[k] as f32 * INV_63 / 65_536.0,
                        ((gs[k] >> 5) & 31) as f32 * INV_31,
                    );
                    assert_eq!(&out[k * 2..k * 2 + 2], &expected, "g={}", gs[k]);
                }
            }
        }
        unsafe { run() };
    }

    #[test]
    fn bilinear_seams_1x1_unaligned_and_special_uv_match_rust() {
        #[target_feature(enable = "neon")]
        unsafe fn check_sampler<S: TexSample4>(
            plan: &DrawPlan<'_>,
            tex: &PreparedTexture<'_>,
            us: [f32; 4],
            vs: [f32; 4],
            label: &str,
        ) {
            let uv = pack_f32x4(us[0], us[1], us[2], us[3]);
            let vv = pack_f32x4(vs[0], vs[1], vs[2], vs[3]);
            let (r, g, b, a) = S::sample4(plan, tex, uv, vv);
            let rs = [
                vgetq_lane_f32::<0>(r),
                vgetq_lane_f32::<1>(r),
                vgetq_lane_f32::<2>(r),
                vgetq_lane_f32::<3>(r),
            ];
            let gs = [
                vgetq_lane_f32::<0>(g),
                vgetq_lane_f32::<1>(g),
                vgetq_lane_f32::<2>(g),
                vgetq_lane_f32::<3>(g),
            ];
            let bs = [
                vgetq_lane_f32::<0>(b),
                vgetq_lane_f32::<1>(b),
                vgetq_lane_f32::<2>(b),
                vgetq_lane_f32::<3>(b),
            ];
            let as_ = [
                vgetq_lane_f32::<0>(a),
                vgetq_lane_f32::<1>(a),
                vgetq_lane_f32::<2>(a),
                vgetq_lane_f32::<3>(a),
            ];
            for k in 0..4 {
                let exp = S::Scalar::sample(plan, tex, us[k], vs[k]);
                assert_eq!(rs[k].to_bits(), exp[0].to_bits(), "{label} r lane {k}");
                assert_eq!(gs[k].to_bits(), exp[1].to_bits(), "{label} g lane {k}");
                assert_eq!(bs[k].to_bits(), exp[2].to_bits(), "{label} b lane {k}");
                assert_eq!(as_[k].to_bits(), exp[3].to_bits(), "{label} a lane {k}");
            }
        }

        const ARGB1555: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x7c00, 0x03e0, 0x001f, 0x8000],
        };
        const ARGB4444: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x0f00, 0x00f0, 0x000f, 0xf000],
        };
        // Backing buffer with 1-byte leading pad to prove unaligned texture slices work.
        let backing: Vec<u8> = (0..1 + 16 * 36)
            .map(|i| ((i * 97 + 13) & 0xff) as u8)
            .collect();
        let unaligned = &backing[1..];

        let uv_cases: &[(&str, [f32; 4], [f32; 4])] = &[
            ("all_non_seam", [0.1, 0.25, 0.4, 0.6], [0.2, 0.35, 0.5, 0.7]),
            ("one_seam", [0.1, 0.99, 0.4, 0.6], [0.2, 0.99, 0.5, 0.7]),
            (
                "all_seam",
                [0.99, -0.01, 1.99, -1.01],
                [0.99, -0.01, 0.5, 1.99],
            ),
            (
                "fx_eq_256_tiny_neg",
                [0.5 / 16.0 - 1.0e-8, 0.5 / 16.0 - 1.0e-9, -1.0e-8, -0.0],
                [0.5 / 16.0 - 1.0e-8, 0.25, -1.0e-8, 0.9999999],
            ),
            (
                "nan_inf_huge",
                [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 3.0e9],
                [f32::NEG_INFINITY, f32::NAN, -3.0e9, f32::INFINITY],
            ),
        ];

        for &(label, w, h, clamp, fmt) in &[
            (
                "1x1_pot_565",
                1u32,
                1u32,
                [false, false],
                PixelFormat::RGB565,
            ),
            ("2x2_pot_565", 2, 2, [false, false], PixelFormat::RGB565),
            ("16x16_pot_565", 16, 16, [false, false], PixelFormat::RGB565),
            ("7x5_npot_565", 7, 5, [false, false], PixelFormat::RGB565),
            ("7x5_clamp_565", 7, 5, [true, true], PixelFormat::RGB565),
            ("16x16_pot_1555", 16, 16, [false, false], ARGB1555),
            ("7x5_clamp_1555", 7, 5, [true, false], ARGB1555),
            ("16x16_pot_4444", 16, 16, [false, false], ARGB4444),
            ("5x7_npot_4444", 5, 7, [false, true], ARGB4444),
        ] {
            let pitch = w * 2 + if w == 1 { 0 } else { 3 };
            let need_bytes = ((h - 1) * pitch + w * 2) as usize;
            let tex = Texture {
                bytes: &unaligned[..need_bytes],
                width: w,
                height: h,
                pitch,
                format: fmt,
                palette: None,
                colour_key: None,
                clamp,
            };
            let states = States {
                linear: true,
                colour: Combine::Texture,
                alpha: Combine::Texture,
                ..States::default()
            };
            let plan = DrawPlan::new(PixelFormat::RGB565, Some(&tex), &states);
            let prep = plan.texture.as_ref().unwrap();
            for &(uv_name, us, vs) in uv_cases {
                let tag = format!("{label}/{uv_name}");
                unsafe {
                    match (prep.format, prep.both_pot_wrap) {
                        (TexFormat::Rgb565, true) => {
                            check_sampler::<BilinearRgb565<true>>(&plan, prep, us, vs, &tag)
                        }
                        (TexFormat::Rgb565, false) => {
                            check_sampler::<BilinearRgb565<false>>(&plan, prep, us, vs, &tag)
                        }
                        (TexFormat::Argb1555, true) => {
                            check_sampler::<BilinearArgb1555<true>>(&plan, prep, us, vs, &tag)
                        }
                        (TexFormat::Argb1555, false) => {
                            check_sampler::<BilinearArgb1555<false>>(&plan, prep, us, vs, &tag)
                        }
                        (TexFormat::Argb4444, true) => {
                            check_sampler::<BilinearArgb4444<true>>(&plan, prep, us, vs, &tag)
                        }
                        (TexFormat::Argb4444, false) => {
                            check_sampler::<BilinearArgb4444<false>>(&plan, prep, us, vs, &tag)
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    #[test]
    fn transparent_alpha_quad_skip_and_nonfinite_rgb_guard_match_rust() {
        const ARGB4444: PixelFormat = PixelFormat {
            bits: 16,
            masks: [0x0f00, 0x00f0, 0x000f, 0xf000],
        };
        const PAL8: PixelFormat = PixelFormat {
            bits: 8,
            masks: [0; 4],
        };
        let tex_zero_alpha: Vec<u8> = (0..8 * 8 * 2)
            .map(|i| if i % 2 == 1 { 0x07 } else { 0x55 })
            .collect();
        let tex4444 = Texture {
            bytes: &tex_zero_alpha,
            width: 8,
            height: 8,
            pitch: 16,
            format: ARGB4444,
            palette: None,
            colour_key: None,
            clamp: [false, false],
        };
        let palette: [[u8; 4]; 256] = std::array::from_fn(|i| [(i as u8), 128, 255, 0]);
        let pal_bytes = vec![7u8; 64];
        let tex_pal = Texture {
            bytes: &pal_bytes,
            width: 8,
            height: 8,
            pitch: 8,
            format: PAL8,
            palette: Some(&palette),
            colour_key: None,
            clamp: [false, false],
        };

        let init_depth = [1.0f32; 9];
        let init_pixels: Vec<u8> = (0..18).map(|i| (i * 41 + 3) as u8).collect();
        let zero_step = [0.0f32; PLANES];

        type RgbCase = (&'static str, [f32; PLANES], ([f32; 4], [f32; 3]));
        let rgb_cases: &[RgbCase] = &[
            (
                "finite_pos_zero_alpha",
                [0.2, 1.0, 0.3, 0.4, 0.8, 0.5, 0.2, 0.0, 0.1, 0.2, 0.3, 0.0],
                ([0.8, 0.5, 0.2, 0.0], [0.1, 0.2, 0.3]),
            ),
            (
                "finite_neg_zero_alpha",
                [0.2, 1.0, 0.3, 0.4, 0.8, 0.5, 0.2, -0.0, 0.1, 0.2, 0.3, 0.0],
                ([0.8, 0.5, 0.2, -0.0], [0.1, 0.2, 0.3]),
            ),
            (
                "inf_rgb_zero_alpha",
                [
                    0.2,
                    1.0,
                    0.3,
                    0.4,
                    f32::INFINITY,
                    0.5,
                    0.2,
                    0.0,
                    0.1,
                    0.2,
                    0.3,
                    0.0,
                ],
                ([f32::INFINITY, 0.5, 0.2, 0.0], [0.1, 0.2, 0.3]),
            ),
            (
                "neginf_rgb_zero_alpha",
                [
                    0.2,
                    1.0,
                    0.3,
                    0.4,
                    0.8,
                    f32::NEG_INFINITY,
                    0.2,
                    0.0,
                    0.1,
                    0.2,
                    0.3,
                    0.0,
                ],
                ([0.8, f32::NEG_INFINITY, 0.2, 0.0], [0.1, 0.2, 0.3]),
            ),
            (
                "nan_rgb_zero_alpha",
                [
                    0.2,
                    1.0,
                    0.3,
                    0.4,
                    0.8,
                    0.5,
                    f32::NAN,
                    0.0,
                    0.1,
                    0.2,
                    0.3,
                    0.0,
                ],
                ([0.8, 0.5, f32::NAN, 0.0], [0.1, 0.2, 0.3]),
            ),
            (
                "inf_uv_pal8_zero_alpha",
                [
                    0.2,
                    1.0,
                    f32::INFINITY,
                    0.4,
                    0.8,
                    0.5,
                    0.2,
                    0.0,
                    0.1,
                    0.2,
                    0.3,
                    0.0,
                ],
                ([0.8, 0.5, 0.2, 0.0], [0.1, 0.2, 0.3]),
            ),
        ];

        for depth_write in [false, true] {
            for flat_mode in [false, true] {
                let state_cases: &[(&str, Option<&Texture<'_>>, States)] = &[
                    (
                        "untex_alpha",
                        None,
                        States {
                            flat: flat_mode,
                            colour: Combine::Diffuse,
                            alpha: Combine::Diffuse,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            depth_write,
                            ..States::default()
                        },
                    ),
                    (
                        "tex4444_alpha_tex",
                        Some(&tex4444),
                        States {
                            flat: flat_mode,
                            alpha: Combine::Texture,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            depth_write,
                            ..States::default()
                        },
                    ),
                    (
                        "tex4444_alpha_mod",
                        Some(&tex4444),
                        States {
                            flat: flat_mode,
                            alpha: Combine::Modulate,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            depth_write,
                            ..States::default()
                        },
                    ),
                    (
                        "tex4444_additive",
                        Some(&tex4444),
                        States {
                            flat: false,
                            alpha: Combine::Texture,
                            blend: Some((Blend::SourceAlpha, Blend::One)),
                            depth_write,
                            ..States::default()
                        },
                    ),
                    (
                        "pal8_lin_alpha",
                        Some(&tex_pal),
                        States {
                            flat: flat_mode,
                            linear: true,
                            alpha: Combine::Texture,
                            blend: Some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
                            depth_write,
                            ..States::default()
                        },
                    ),
                ];
                for (sname, t, states) in state_cases {
                    let plan = DrawPlan::new(PixelFormat::RGB565, *t, states);
                    for &(rname, val, fl) in rgb_cases {
                        assert_neon_matches_rust(
                            &plan,
                            &init_depth,
                            &init_pixels,
                            1,
                            val,
                            zero_step,
                            fl,
                            states,
                            &format!("{sname}/dw={depth_write}/flat={flat_mode}/{rname}"),
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn opaque_z_all_pass_masks_and_inlined_tails_match_rust() {
        let tex_bytes: Vec<u8> = (0..16 * 16 * 2)
            .map(|i| ((i * 59 + 11) & 0xff) as u8)
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
        let val: [f32; PLANES] = [0.5, 1.0, 0.15, 0.25, 0.7, 0.6, 0.5, 0.4, 0.1, 0.2, 0.3, 0.5];
        let step: [f32; PLANES] = [
            0.0, 0.01, 0.09, 0.07, -0.01, 0.01, -0.005, 0.0, 0.005, -0.005, 0.005, 0.0,
        ];
        let flat = ([0.75, 0.45, 0.85, 1.0], [0.2, 0.1, 0.3]);

        for depth_write in [false, true] {
            let kernels: &[(&str, Option<&Texture<'_>>, States)] = &[
                (
                    "untex_flat_z",
                    None,
                    States {
                        flat: true,
                        colour: Combine::Diffuse,
                        alpha: Combine::Diffuse,
                        depth_test: Some(Compare::LessEqual),
                        depth_write,
                        ..States::default()
                    },
                ),
                (
                    "untex_gouraud_z",
                    None,
                    States {
                        colour: Combine::Diffuse,
                        alpha: Combine::Diffuse,
                        depth_test: Some(Compare::LessEqual),
                        depth_write,
                        ..States::default()
                    },
                ),
                (
                    "tex_copy_bilin_z",
                    Some(&tex),
                    States {
                        linear: true,
                        colour: Combine::Texture,
                        alpha: Combine::Texture,
                        depth_test: Some(Compare::LessEqual),
                        depth_write,
                        ..States::default()
                    },
                ),
                (
                    "tex_mod_bilin_z",
                    Some(&tex),
                    States {
                        linear: true,
                        depth_test: Some(Compare::LessEqual),
                        depth_write,
                        ..States::default()
                    },
                ),
            ];
            for (kname, t, states) in kernels {
                let plan = DrawPlan::new(PixelFormat::RGB565, *t, states);
                // Every 4-bit Z mask 0..=15 (all-fail, partial, all-pass) with tail lengths 0..=3.
                for mask in 0u8..=15 {
                    for extra in 0..=3 {
                        let len = 4 + extra;
                        let depth: Vec<f32> = (0..len)
                            .map(|i| {
                                if ((mask >> (i & 3)) & 1) != 0 {
                                    0.5
                                } else {
                                    0.1
                                }
                            })
                            .collect();
                        let pixels: Vec<u8> = (0..len * 2).map(|i| (i * 37 + 5) as u8).collect();
                        assert_neon_matches_rust(
                            &plan,
                            &depth,
                            &pixels,
                            usize::from(mask & 7),
                            val,
                            step,
                            flat,
                            states,
                            &format!("{kname}/dw={depth_write}/mask={mask}/len={len}"),
                        );
                    }
                }
                // Widths 0..=65 and odd byte offsets 0..=7 on all-pass depths.
                for len in 0..=65 {
                    let depth = vec![1.0f32; len];
                    let pixels: Vec<u8> = (0..len * 2).map(|i| (i * 23 + 9) as u8).collect();
                    assert_neon_matches_rust(
                        &plan,
                        &depth,
                        &pixels,
                        len % 8,
                        val,
                        step,
                        flat,
                        states,
                        &format!("{kname}/dw={depth_write}/all_pass/len={len}"),
                    );
                }
            }
        }
    }
}
