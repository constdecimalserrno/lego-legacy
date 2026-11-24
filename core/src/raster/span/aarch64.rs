//! 4-pixel `aarch64` NEON interior-span kernels (ADR-0009, #255, #278).
//!
//! Preserves Golden Rule 1 bit-for-bit against [`super::rust`]:
//! - Sequential plane stepping (`v += step`) per pixel via 4-lane vector adds + 4×4 transpose;
//! - Exact IEEE-754 `vdivq_f32` (no approximate reciprocal);
//! - Separate rounded `vmulq_f32` and `vaddq_f32` (no FMA);
//! - Exact Q16 integer bilinear weights (`bilinear16`);
//! - Exact IEEE-754 `vrndaq_f32` round-half-away-from-zero for `alpha_test == Greater`;
//! - Clamped float-to-int RGB565 packing matching `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32`;
//! - Tail pixels `0..3` delegated to the canonical pure-Rust oracle.

use core::arch::aarch64::{
    float32x4_t, int32x4_t, uint16x4_t, uint32x4_t, vaddq_f32, vaddq_s32, vaddq_u32, vandq_s32,
    vandq_u32, vbsl_u16, vbslq_f32, vcgtq_f32, vcgtq_u32, vcleq_f32, vcltq_f32, vcvtq_f32_u32,
    vcvtq_s32_f32, vcvtq_u32_f32, vdivq_f32, vdup_n_u16, vdup_n_u64, vdupq_n_f32, vdupq_n_s32,
    vdupq_n_u32, vget_lane_u64, vld1q_f32, vld1q_u32, vmaxnmq_f32, vmaxvq_u32, vminnmq_f32,
    vmovl_u16, vmovn_u32, vmulq_f32, vmulq_u32, vorrq_u32, vreinterpret_u16_u64,
    vreinterpret_u64_u16, vreinterpretq_f32_f64, vreinterpretq_f64_f32, vrndaq_f32, vrndmq_f32,
    vshlq_n_u32, vshrq_n_u32, vst1q_f32, vst1q_s32, vsubq_f32, vsubq_u32, vtrn1q_f32, vtrn1q_f64,
    vtrn2q_f32, vtrn2q_f64,
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
unsafe fn resolve_xy4<const POT: bool>(
    tex: &PreparedTexture<'_>,
    xv: int32x4_t,
    yv: int32x4_t,
) -> ([usize; 4], [usize; 4]) {
    let mut xs = [0i32; 4];
    let mut ys = [0i32; 4];
    if POT {
        let xm = vandq_s32(xv, vdupq_n_s32(tex.pot_x_mask));
        let ym = vandq_s32(yv, vdupq_n_s32(tex.pot_y_mask));
        vst1q_s32(xs.as_mut_ptr(), xm);
        vst1q_s32(ys.as_mut_ptr(), ym);
        (
            [
                xs[0] as usize,
                xs[1] as usize,
                xs[2] as usize,
                xs[3] as usize,
            ],
            [
                ys[0] as usize,
                ys[1] as usize,
                ys[2] as usize,
                ys[3] as usize,
            ],
        )
    } else {
        vst1q_s32(xs.as_mut_ptr(), xv);
        vst1q_s32(ys.as_mut_ptr(), yv);
        let (x0, y0) = resolve_xy::<false>(tex, xs[0], ys[0]);
        let (x1, y1) = resolve_xy::<false>(tex, xs[1], ys[1]);
        let (x2, y2) = resolve_xy::<false>(tex, xs[2], ys[2]);
        let (x3, y3) = resolve_xy::<false>(tex, xs[3], ys[3]);
        ([x0, x1, x2, x3], [y0, y1, y2, y3])
    }
}

#[inline(always)]
unsafe fn read_u16_unchecked(base: *const u8, offset: usize) -> u32 {
    u32::from(u16::from_le(core::ptr::read_unaligned(
        base.add(offset).cast::<u16>(),
    )))
}

#[inline(always)]
unsafe fn gather_u16_4(tex: &PreparedTexture<'_>, xs: [usize; 4], ys: [usize; 4]) -> uint32x4_t {
    debug_assert!(tex.in_bounds);
    let base = tex.bytes.as_ptr();
    let pitch = tex.pitch;
    let raw = [
        read_u16_unchecked(base, ys[0] * pitch + xs[0] * 2),
        read_u16_unchecked(base, ys[1] * pitch + xs[1] * 2),
        read_u16_unchecked(base, ys[2] * pitch + xs[2] * 2),
        read_u16_unchecked(base, ys[3] * pitch + xs[3] * 2),
    ];
    vld1q_u32(raw.as_ptr())
}

#[inline(always)]
unsafe fn bilinear16_setup<const POT: bool>(
    tex: &PreparedTexture<'_>,
    u: float32x4_t,
    v: float32x4_t,
) -> (
    uint32x4_t,
    uint32x4_t,
    uint32x4_t,
    uint32x4_t,
    uint32x4_t,
    uint32x4_t,
    uint32x4_t,
    uint32x4_t,
) {
    debug_assert!(tex.in_bounds);
    let half = vdupq_n_f32(0.5);
    let x = vsubq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f)), half);
    let y = vsubq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f)), half);
    let x0_f = vrndmq_f32(x);
    let y0_f = vrndmq_f32(y);
    let v256_f = vdupq_n_f32(256.0);
    let fx = vcvtq_u32_f32(vmulq_f32(vsubq_f32(x, x0_f), v256_f));
    let fy = vcvtq_u32_f32(vmulq_f32(vsubq_f32(y, y0_f), v256_f));
    let x0 = vcvtq_s32_f32(x0_f);
    let y0 = vcvtq_s32_f32(y0_f);
    let one_i = vdupq_n_s32(1);
    let (left, upper) = resolve_xy4::<POT>(tex, x0, y0);
    let (right, lower) = resolve_xy4::<POT>(tex, vaddq_s32(x0, one_i), vaddq_s32(y0, one_i));

    let v256_u = vdupq_n_u32(256);
    let ifx = vsubq_u32(v256_u, fx);
    let ify = vsubq_u32(v256_u, fy);
    let w00 = vmulq_u32(ifx, ify);
    let w10 = vmulq_u32(fx, ify);
    let w01 = vmulq_u32(ifx, fy);
    let w11 = vmulq_u32(fx, fy);

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
    let t00 = vld1q_u32(
        [
            read_u16_unchecked(base, u0 + left[0] * 2),
            read_u16_unchecked(base, u1 + left[1] * 2),
            read_u16_unchecked(base, u2 + left[2] * 2),
            read_u16_unchecked(base, u3 + left[3] * 2),
        ]
        .as_ptr(),
    );
    let t10 = vld1q_u32(
        [
            read_u16_unchecked(base, u0 + right[0] * 2),
            read_u16_unchecked(base, u1 + right[1] * 2),
            read_u16_unchecked(base, u2 + right[2] * 2),
            read_u16_unchecked(base, u3 + right[3] * 2),
        ]
        .as_ptr(),
    );
    let t01 = vld1q_u32(
        [
            read_u16_unchecked(base, l0 + left[0] * 2),
            read_u16_unchecked(base, l1 + left[1] * 2),
            read_u16_unchecked(base, l2 + left[2] * 2),
            read_u16_unchecked(base, l3 + left[3] * 2),
        ]
        .as_ptr(),
    );
    let t11 = vld1q_u32(
        [
            read_u16_unchecked(base, l0 + right[0] * 2),
            read_u16_unchecked(base, l1 + right[1] * 2),
            read_u16_unchecked(base, l2 + right[2] * 2),
            read_u16_unchecked(base, l3 + right[3] * 2),
        ]
        .as_ptr(),
    );
    (t00, t10, t01, t11, w00, w10, w01, w11)
}

#[inline(always)]
unsafe fn blend4_u32(c: [uint32x4_t; 4], w: [uint32x4_t; 4]) -> uint32x4_t {
    vaddq_u32(
        vaddq_u32(vmulq_u32(c[0], w[0]), vmulq_u32(c[1], w[1])),
        vaddq_u32(vmulq_u32(c[2], w[2]), vmulq_u32(c[3], w[3])),
    )
}

trait TexSample4 {
    type Scalar: TexSample;
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t);
}

struct NearestRgb565<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestRgb565<POT> {
    type Scalar = rust::NearestRgb565<POT>;
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
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (t00, t10, t01, t11, w00, w10, w01, w11) = bilinear16_setup::<POT>(tex, u, v);
        let m3f = vdupq_n_u32(0x3f);
        let m1f = vdupq_n_u32(0x1f);
        let r = blend4_u32(
            [
                vshrq_n_u32::<11>(t00),
                vshrq_n_u32::<11>(t10),
                vshrq_n_u32::<11>(t01),
                vshrq_n_u32::<11>(t11),
            ],
            [w00, w10, w01, w11],
        );
        let g = blend4_u32(
            [
                vandq_u32(vshrq_n_u32::<5>(t00), m3f),
                vandq_u32(vshrq_n_u32::<5>(t10), m3f),
                vandq_u32(vshrq_n_u32::<5>(t01), m3f),
                vandq_u32(vshrq_n_u32::<5>(t11), m3f),
            ],
            [w00, w10, w01, w11],
        );
        let b = blend4_u32(
            [
                vandq_u32(t00, m1f),
                vandq_u32(t10, m1f),
                vandq_u32(t01, m1f),
                vandq_u32(t11, m1f),
            ],
            [w00, w10, w01, w11],
        );
        (
            vmulq_f32(vcvtq_f32_u32(r), vdupq_n_f32(INV_31_Q16)),
            vmulq_f32(vcvtq_f32_u32(g), vdupq_n_f32(INV_63_Q16)),
            vmulq_f32(vcvtq_f32_u32(b), vdupq_n_f32(INV_31_Q16)),
            vdupq_n_f32(1.0),
        )
    }
}

struct NearestArgb1555<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestArgb1555<POT> {
    type Scalar = rust::NearestArgb1555<POT>;
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
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (t00, t10, t01, t11, w00, w10, w01, w11) = bilinear16_setup::<POT>(tex, u, v);
        let m1f = vdupq_n_u32(0x1f);
        let r = blend4_u32(
            [
                vandq_u32(vshrq_n_u32::<10>(t00), m1f),
                vandq_u32(vshrq_n_u32::<10>(t10), m1f),
                vandq_u32(vshrq_n_u32::<10>(t01), m1f),
                vandq_u32(vshrq_n_u32::<10>(t11), m1f),
            ],
            [w00, w10, w01, w11],
        );
        let g = blend4_u32(
            [
                vandq_u32(vshrq_n_u32::<5>(t00), m1f),
                vandq_u32(vshrq_n_u32::<5>(t10), m1f),
                vandq_u32(vshrq_n_u32::<5>(t01), m1f),
                vandq_u32(vshrq_n_u32::<5>(t11), m1f),
            ],
            [w00, w10, w01, w11],
        );
        let b = blend4_u32(
            [
                vandq_u32(t00, m1f),
                vandq_u32(t10, m1f),
                vandq_u32(t01, m1f),
                vandq_u32(t11, m1f),
            ],
            [w00, w10, w01, w11],
        );
        let a = blend4_u32(
            [
                vshrq_n_u32::<15>(t00),
                vshrq_n_u32::<15>(t10),
                vshrq_n_u32::<15>(t01),
                vshrq_n_u32::<15>(t11),
            ],
            [w00, w10, w01, w11],
        );
        let inv31 = vdupq_n_f32(INV_31_Q16);
        (
            vmulq_f32(vcvtq_f32_u32(r), inv31),
            vmulq_f32(vcvtq_f32_u32(g), inv31),
            vmulq_f32(vcvtq_f32_u32(b), inv31),
            vmulq_f32(vcvtq_f32_u32(a), vdupq_n_f32(INV_65536)),
        )
    }
}

struct NearestArgb4444<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestArgb4444<POT> {
    type Scalar = rust::NearestArgb4444<POT>;
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
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let (t00, t10, t01, t11, w00, w10, w01, w11) = bilinear16_setup::<POT>(tex, u, v);
        let mf = vdupq_n_u32(0xf);
        let r = blend4_u32(
            [
                vandq_u32(vshrq_n_u32::<8>(t00), mf),
                vandq_u32(vshrq_n_u32::<8>(t10), mf),
                vandq_u32(vshrq_n_u32::<8>(t01), mf),
                vandq_u32(vshrq_n_u32::<8>(t11), mf),
            ],
            [w00, w10, w01, w11],
        );
        let g = blend4_u32(
            [
                vandq_u32(vshrq_n_u32::<4>(t00), mf),
                vandq_u32(vshrq_n_u32::<4>(t10), mf),
                vandq_u32(vshrq_n_u32::<4>(t01), mf),
                vandq_u32(vshrq_n_u32::<4>(t11), mf),
            ],
            [w00, w10, w01, w11],
        );
        let b = blend4_u32(
            [
                vandq_u32(t00, mf),
                vandq_u32(t10, mf),
                vandq_u32(t01, mf),
                vandq_u32(t11, mf),
            ],
            [w00, w10, w01, w11],
        );
        let a = blend4_u32(
            [
                vshrq_n_u32::<12>(t00),
                vshrq_n_u32::<12>(t10),
                vshrq_n_u32::<12>(t01),
                vshrq_n_u32::<12>(t11),
            ],
            [w00, w10, w01, w11],
        );
        let inv15 = vdupq_n_f32(INV_15_Q16);
        (
            vmulq_f32(vcvtq_f32_u32(r), inv15),
            vmulq_f32(vcvtq_f32_u32(g), inv15),
            vmulq_f32(vcvtq_f32_u32(b), inv15),
            vmulq_f32(vcvtq_f32_u32(a), inv15),
        )
    }
}

struct NearestPaletted8<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestPaletted8<POT> {
    type Scalar = rust::NearestPaletted8<POT>;
    #[inline(always)]
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        debug_assert!(tex.in_bounds);
        let xi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(u, vdupq_n_f32(tex.width_f))));
        let yi = vcvtq_s32_f32(vrndmq_f32(vmulq_f32(v, vdupq_n_f32(tex.height_f))));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let pal = plan.palette_f32();
        let base = tex.bytes.as_ptr();
        let pitch = tex.pitch;
        let c0 = vld1q_f32(pal[usize::from(*base.add(ys[0] * pitch + xs[0]))].as_ptr());
        let c1 = vld1q_f32(pal[usize::from(*base.add(ys[1] * pitch + xs[1]))].as_ptr());
        let c2 = vld1q_f32(pal[usize::from(*base.add(ys[2] * pitch + xs[2]))].as_ptr());
        let c3 = vld1q_f32(pal[usize::from(*base.add(ys[3] * pitch + xs[3]))].as_ptr());
        transpose4(c0, c1, c2, c3)
    }
}

struct BilinearPaletted8<const POT: bool>;
impl<const POT: bool> TexSample4 for BilinearPaletted8<POT> {
    type Scalar = rust::BilinearPaletted8<POT>;
    #[inline(always)]
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: float32x4_t,
        v: float32x4_t,
    ) -> (float32x4_t, float32x4_t, float32x4_t, float32x4_t) {
        let mut us = [0.0f32; 4];
        let mut vs = [0.0f32; 4];
        vst1q_f32(us.as_mut_ptr(), u);
        vst1q_f32(vs.as_mut_ptr(), v);
        let c0 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[0], vs[0]);
        let c1 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[1], vs[1]);
        let c2 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[2], vs[2]);
        let c3 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[3], vs[3]);
        transpose4(
            vld1q_f32(c0.as_ptr()),
            vld1q_f32(c1.as_ptr()),
            vld1q_f32(c2.as_ptr()),
            vld1q_f32(c3.as_ptr()),
        )
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
    unsafe {
        span_untextured_flat_opaque_neon::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

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
        if n4 < n {
            rust::span_untextured_flat_opaque::<false, SPECULAR>(
                plan,
                &mut depth[n4..n],
                &mut pixels[n4 * 2..n * 2],
                value,
                step,
                flat,
                states,
            );
        }
        return;
    }
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
        let zv = vld1q_f32([z0, z1, z2, z3].as_ptr());
        let dv = vld1q_f32(d_ptr.add(i));
        let z_mask = vcleq_f32(zv, dv);
        if vmaxvq_u32(z_mask) != 0 {
            if depth_write {
                vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
            }
            let old_px = load_u16x4(p_ptr.add(i * 2));
            let mask16 = vmovn_u32(z_mask);
            store_u16x4(p_ptr.add(i * 2), vbsl_u16(mask16, packed4, old_px));
        }
        i += 4;
    }
    if n4 < n {
        value[0] = z_val;
        rust::span_untextured_flat_opaque::<true, SPECULAR>(
            plan,
            &mut depth[n4..n],
            &mut pixels[n4 * 2..n * 2],
            value,
            step,
            flat,
            states,
        );
    }
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
    unsafe {
        span_untextured_gouraud_opaque_neon::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

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
            if depth_write {
                vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
            }
            let old_px = load_u16x4(p_ptr.add(i * 2));
            store_u16x4(
                p_ptr.add(i * 2),
                vbsl_u16(vmovn_u32(z_mask), new_px, old_px),
            );
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 < n {
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
    unsafe {
        span_untextured_alpha_neon::<Z_LE, ALPHA_GT>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

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
                let zv = vld1q_f32([z0, z1, z2, z3].as_ptr());
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
        if n4 < n {
            if Z_LE {
                value[0] = z_val;
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
        if n4 < n {
            vst1q_f32(value.as_mut_ptr(), s0);
            vst1q_f32(value.as_mut_ptr().add(4), s1);
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
    }
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
    unsafe {
        span_textured_copy_opaque_neon::<S, Z_TEST>(plan, depth, pixels, value, step, flat, states);
    }
}

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
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
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
                let new_px = pack_rgb565_4(tr, tg, tb);
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
            let (_, wv, uv, vv) = step4_transpose(&mut s0, d0);
            let near = persp4(wv);
            let (tr, tg, tb, _) = S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));
            let new_px = pack_rgb565_4(tr, tg, tb);
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 < n {
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
    unsafe {
        span_textured_modulate_opaque_neon::<S, Z_TEST, FLAT, SPECULAR, FOG>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

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
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
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
        let (tr, tg, tb, _) = S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));

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
        if Z_TEST {
            if depth_write {
                vst1q_f32(d_ptr.add(i), vbslq_f32(z_mask, zv, dv));
            }
            let old_px = load_u16x4(p_ptr.add(i * 2));
            store_u16x4(
                p_ptr.add(i * 2),
                vbsl_u16(vmovn_u32(z_mask), new_px, old_px),
            );
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }

    if n4 < n {
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
    unsafe {
        span_textured_modulate_blend_neon::<S, Z_LE, ALPHA_GT, BLEND, FLAT, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

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
    let tex = plan.texture.as_ref().unwrap();
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
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
        let (tr, tg, tb, ta) = S::sample4(plan, tex, vmulq_f32(uv, near), vmulq_f32(vv, near));

        // When alpha comes solely from the texture and alpha_test is active, test
        // alpha before transposing diffuse RGB or loading the framebuffer so fully
        // transparent 4-pixel quads exit early.
        let (sa, diffuse_opt) = if alpha_combine == Combine::Texture {
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(ta, alpha_ref);
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
            (ta, None)
        } else if FLAT {
            let sa = combine_4(alpha_combine, ta, flat_a);
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
            (sa, None)
        } else {
            let (q0, q1, q2, q3) = q.unwrap();
            let (cr, cg, cb, ca) = transpose4(q0, q1, q2, q3);
            let sa = combine_4(alpha_combine, ta, vmulq_f32(ca, near));
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
            (sa, Some((cr, cg, cb)))
        };

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

    if n4 < n {
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
