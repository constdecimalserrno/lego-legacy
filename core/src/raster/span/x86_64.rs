//! 4-pixel `x86_64` baseline SSE2 interior-span kernels (ADR-0009, #255, #278).
//!
//! Mirrors [`super::aarch64`] with an identical 4-lane algorithm using only
//! baseline SSE2 (zero runtime feature dispatch, 100% `x86_64` desktop
//! coverage), preserving Golden Rule 1 bit-for-bit against [`super::rust`]:
//! - Sequential plane stepping (`v += step`) per pixel via 4-lane vector adds + 4×4 transpose;
//! - Exact IEEE-754 `_mm_div_ps` (no approximate reciprocal);
//! - Separate rounded `_mm_mul_ps` and `_mm_add_ps` (no FMA);
//! - Exact Q16 integer bilinear weights (`bilinear16`) via baseline `_mm_mul_epu32`;
//! - Exact round-half-away-from-zero (`t + (frac >= 0.5)`) for `alpha_test == Greater`;
//! - Clamped float-to-int RGB565 packing (`_mm_cvttps_epi32` on `[0.0, 1.0]`,
//!   matching saturating `as u32` on every input including `NaN`);
//! - Tail pixels `0..3` delegated to the canonical pure-Rust oracle.

use core::arch::x86_64::{
    __m128, __m128i, _mm_add_epi32, _mm_add_ps, _mm_and_ps, _mm_and_si128, _mm_andnot_ps,
    _mm_andnot_si128, _mm_castps_si128, _mm_castsi128_ps, _mm_cmpge_ps, _mm_cmpgt_epi32,
    _mm_cmpgt_ps, _mm_cmple_ps, _mm_cmplt_ps, _mm_cvtepi32_ps, _mm_cvtsi128_si64,
    _mm_cvtsi64_si128, _mm_cvttps_epi32, _mm_div_ps, _mm_loadu_ps, _mm_loadu_si128, _mm_max_ps,
    _mm_min_ps, _mm_movehl_ps, _mm_movelh_ps, _mm_movemask_ps, _mm_mul_epu32, _mm_mul_ps,
    _mm_or_ps, _mm_or_si128, _mm_packs_epi32, _mm_set1_epi16, _mm_set1_epi32, _mm_set1_ps,
    _mm_setzero_ps, _mm_setzero_si128, _mm_shuffle_epi32, _mm_slli_epi32, _mm_srai_epi32,
    _mm_srli_epi32, _mm_srli_epi64, _mm_storeu_ps, _mm_storeu_si128, _mm_sub_epi32, _mm_sub_ps,
    _mm_unpackhi_ps, _mm_unpacklo_epi16, _mm_unpacklo_epi32, _mm_unpacklo_ps,
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
unsafe fn load_u16x4(ptr: *const u8) -> __m128i {
    let bits = core::ptr::read_unaligned(ptr.cast::<i64>());
    _mm_cvtsi64_si128(bits)
}

#[inline(always)]
unsafe fn store_u16x4(ptr: *mut u8, v: __m128i) {
    let bits = _mm_cvtsi128_si64(v);
    core::ptr::write_unaligned(ptr.cast::<i64>(), bits);
}

#[inline(always)]
unsafe fn select_ps(mask: __m128, a: __m128, b: __m128) -> __m128 {
    _mm_or_ps(_mm_and_ps(mask, a), _mm_andnot_ps(mask, b))
}

#[inline(always)]
unsafe fn select_si128(mask: __m128i, a: __m128i, b: __m128i) -> __m128i {
    _mm_or_si128(_mm_and_si128(mask, a), _mm_andnot_si128(mask, b))
}

#[inline(always)]
unsafe fn step4_raw(state: &mut __m128, step: __m128) -> (__m128, __m128, __m128, __m128) {
    let p0 = *state;
    let p1 = _mm_add_ps(p0, step);
    let p2 = _mm_add_ps(p1, step);
    let p3 = _mm_add_ps(p2, step);
    *state = _mm_add_ps(p3, step);
    (p0, p1, p2, p3)
}

#[inline(always)]
unsafe fn transpose4(
    p0: __m128,
    p1: __m128,
    p2: __m128,
    p3: __m128,
) -> (__m128, __m128, __m128, __m128) {
    let t0 = _mm_unpacklo_ps(p0, p1);
    let t1 = _mm_unpackhi_ps(p0, p1);
    let t2 = _mm_unpacklo_ps(p2, p3);
    let t3 = _mm_unpackhi_ps(p2, p3);
    let c0 = _mm_movelh_ps(t0, t2);
    let c1 = _mm_movehl_ps(t2, t0);
    let c2 = _mm_movelh_ps(t1, t3);
    let c3 = _mm_movehl_ps(t3, t1);
    (c0, c1, c2, c3)
}

#[inline(always)]
unsafe fn step4_transpose(state: &mut __m128, step: __m128) -> (__m128, __m128, __m128, __m128) {
    let (p0, p1, p2, p3) = step4_raw(state, step);
    transpose4(p0, p1, p2, p3)
}

#[inline(always)]
unsafe fn persp4(wv: __m128) -> __m128 {
    let zero = _mm_setzero_ps();
    let one = _mm_set1_ps(1.0);
    let pos = _mm_cmpgt_ps(wv, zero);
    let inv = _mm_div_ps(one, wv);
    _mm_and_ps(pos, inv)
}

/// Exact `(sa.clamp(0.0, 1.0) * 255.0).round() as u8 > reference` across 4 lanes
/// using baseline SSE2. `_mm_max_ps(NaN, 0.0) == 0.0` maps `NaN` to `0`, and
/// `t + (frac >= 0.5)` matches `f32::round()` bit-for-bit on `[0.0, 255.0]`
/// including `0.49999997` (where `(x + 0.5).floor()` would round up).
#[inline(always)]
unsafe fn alpha_gt_mask4(sa: __m128, reference: u8) -> __m128 {
    let zero = _mm_setzero_ps();
    let one = _mm_set1_ps(1.0);
    let v255 = _mm_set1_ps(255.0);
    let half = _mm_set1_ps(0.5);
    let x = _mm_mul_ps(_mm_min_ps(_mm_max_ps(sa, zero), one), v255);
    let t = _mm_cvttps_epi32(x);
    let frac = _mm_sub_ps(x, _mm_cvtepi32_ps(t));
    let ge_half = _mm_castps_si128(_mm_cmpge_ps(frac, half));
    let alpha_i32 = _mm_sub_epi32(t, ge_half);
    _mm_castsi128_ps(_mm_cmpgt_epi32(
        alpha_i32,
        _mm_set1_epi32(i32::from(reference)),
    ))
}

#[inline(always)]
unsafe fn mul_u32(a: __m128i, b: __m128i) -> __m128i {
    let even = _mm_mul_epu32(a, b);
    let odd = _mm_mul_epu32(_mm_srli_epi64::<32>(a), _mm_srli_epi64::<32>(b));
    let e = _mm_shuffle_epi32::<0b00_00_10_00>(even);
    let o = _mm_shuffle_epi32::<0b00_00_10_00>(odd);
    _mm_unpacklo_epi32(e, o)
}

/// Exact `x.floor() as i32` across all 4 lanes using baseline SSE2, with a
/// cold scalar fallback if any lane is `NaN`, infinite, or outside `[-2^31, 2^31)`.
#[inline(always)]
unsafe fn floor_to_i32(x: __m128) -> __m128i {
    let min_i = _mm_set1_ps(-2_147_483_648.0f32);
    let max_i = _mm_set1_ps(2_147_483_648.0f32);
    let in_range = _mm_and_ps(_mm_cmpge_ps(x, min_i), _mm_cmplt_ps(x, max_i));
    if _mm_movemask_ps(in_range) == 0xf {
        let t = _mm_cvttps_epi32(x);
        let tf = _mm_cvtepi32_ps(t);
        let lt = _mm_castps_si128(_mm_cmplt_ps(x, tf));
        _mm_add_epi32(t, lt)
    } else {
        floor_to_i32_slow(x)
    }
}

#[cold]
unsafe fn floor_to_i32_slow(x: __m128) -> __m128i {
    let mut arr = [0.0f32; 4];
    _mm_storeu_ps(arr.as_mut_ptr(), x);
    let out = [
        arr[0].floor() as i32,
        arr[1].floor() as i32,
        arr[2].floor() as i32,
        arr[3].floor() as i32,
    ];
    _mm_loadu_si128(out.as_ptr().cast())
}

/// Exact `(x.floor() as i32, ((x - x.floor()) * 256.0) as u32)` across 4 lanes
/// using baseline SSE2, with a cold scalar fallback for out-of-range/`NaN`.
#[inline(always)]
unsafe fn bilinear_floor_frac(x: __m128) -> (__m128i, __m128i) {
    let min_i = _mm_set1_ps(-2_147_483_648.0f32);
    let max_i = _mm_set1_ps(2_147_483_648.0f32);
    let in_range = _mm_and_ps(_mm_cmpge_ps(x, min_i), _mm_cmplt_ps(x, max_i));
    if _mm_movemask_ps(in_range) == 0xf {
        let t = _mm_cvttps_epi32(x);
        let tf = _mm_cvtepi32_ps(t);
        let lt = _mm_cmplt_ps(x, tf);
        let x0 = _mm_add_epi32(t, _mm_castps_si128(lt));
        let x0_f = _mm_sub_ps(tf, _mm_and_ps(lt, _mm_set1_ps(1.0)));
        let fx = _mm_cvttps_epi32(_mm_mul_ps(_mm_sub_ps(x, x0_f), _mm_set1_ps(256.0)));
        (x0, fx)
    } else {
        bilinear_floor_frac_slow(x)
    }
}

#[cold]
unsafe fn bilinear_floor_frac_slow(x: __m128) -> (__m128i, __m128i) {
    let mut arr = [0.0f32; 4];
    _mm_storeu_ps(arr.as_mut_ptr(), x);
    let mut x0s = [0i32; 4];
    let mut fxs = [0i32; 4];
    for k in 0..4 {
        let f = arr[k].floor();
        x0s[k] = f as i32;
        fxs[k] = (((arr[k] - f) * 256.0) as u32) as i32;
    }
    (
        _mm_loadu_si128(x0s.as_ptr().cast()),
        _mm_loadu_si128(fxs.as_ptr().cast()),
    )
}

#[inline(always)]
unsafe fn unpack_rgb565_4(px: __m128i) -> (__m128, __m128, __m128) {
    let v = _mm_unpacklo_epi16(px, _mm_setzero_si128());
    let r = _mm_mul_ps(
        _mm_cvtepi32_ps(_mm_srli_epi32::<11>(v)),
        _mm_set1_ps(INV_31),
    );
    let g = _mm_mul_ps(
        _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<5>(v), _mm_set1_epi32(0x3f))),
        _mm_set1_ps(INV_63),
    );
    let b = _mm_mul_ps(
        _mm_cvtepi32_ps(_mm_and_si128(v, _mm_set1_epi32(0x1f))),
        _mm_set1_ps(INV_31),
    );
    (r, g, b)
}

#[inline(always)]
unsafe fn pack_rgb565_4(r: __m128, g: __m128, b: __m128) -> __m128i {
    let zero = _mm_setzero_ps();
    let one = _mm_set1_ps(1.0);
    let v255 = _mm_set1_ps(255.0);
    let half = _mm_set1_ps(0.5);

    // `_mm_max_ps(NaN, 0.0) == 0.0` and `_mm_min_ps(0.0, 1.0) == 0.0`,
    // matching Rust `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32` bit-for-bit on NaN.
    let rc = _mm_min_ps(_mm_max_ps(r, zero), one);
    let gc = _mm_min_ps(_mm_max_ps(g, zero), one);
    let bc = _mm_min_ps(_mm_max_ps(b, zero), one);

    // Separate mul then add (ADR-0009: no FMA).
    let ri = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(rc, v255), half));
    let gi = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(gc, v255), half));
    let bi = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(bc, v255), half));

    let packed = _mm_or_si128(
        _mm_or_si128(
            _mm_slli_epi32::<11>(_mm_srli_epi32::<3>(ri)),
            _mm_slli_epi32::<5>(_mm_srli_epi32::<2>(gi)),
        ),
        _mm_srli_epi32::<3>(bi),
    );
    // Sign-extend low 16 bits so signed `_mm_packs_epi32` preserves all 0..=65535.
    let sign_ext = _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(packed));
    _mm_packs_epi32(sign_ext, sign_ext)
}

#[inline(always)]
unsafe fn clamp_nan_preserving_01(x: __m128) -> __m128 {
    let zero = _mm_setzero_ps();
    let one = _mm_set1_ps(1.0);
    let f = select_ps(_mm_cmplt_ps(x, zero), zero, x);
    select_ps(_mm_cmpgt_ps(f, one), one, f)
}

#[inline(always)]
unsafe fn combine_4(how: Combine, t: __m128, d: __m128) -> __m128 {
    match how {
        Combine::Texture => t,
        Combine::Diffuse => d,
        Combine::Modulate => _mm_mul_ps(t, d),
        Combine::Add => _mm_min_ps(_mm_add_ps(t, d), _mm_set1_ps(1.0)),
    }
}

#[inline(always)]
unsafe fn resolve_xy4<const POT: bool>(
    tex: &PreparedTexture<'_>,
    xv: __m128i,
    yv: __m128i,
) -> ([usize; 4], [usize; 4]) {
    let mut xs = [0i32; 4];
    let mut ys = [0i32; 4];
    if POT {
        let xm = _mm_and_si128(xv, _mm_set1_epi32(tex.pot_x_mask));
        let ym = _mm_and_si128(yv, _mm_set1_epi32(tex.pot_y_mask));
        _mm_storeu_si128(xs.as_mut_ptr().cast(), xm);
        _mm_storeu_si128(ys.as_mut_ptr().cast(), ym);
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
        _mm_storeu_si128(xs.as_mut_ptr().cast(), xv);
        _mm_storeu_si128(ys.as_mut_ptr().cast(), yv);
        let (x0, y0) = resolve_xy::<false>(tex, xs[0], ys[0]);
        let (x1, y1) = resolve_xy::<false>(tex, xs[1], ys[1]);
        let (x2, y2) = resolve_xy::<false>(tex, xs[2], ys[2]);
        let (x3, y3) = resolve_xy::<false>(tex, xs[3], ys[3]);
        ([x0, x1, x2, x3], [y0, y1, y2, y3])
    }
}

#[inline(always)]
unsafe fn read_u16_unchecked(base: *const u8, offset: usize) -> i32 {
    i32::from(u16::from_le(core::ptr::read_unaligned(
        base.add(offset).cast::<u16>(),
    )))
}

#[inline(always)]
unsafe fn gather_u16_4(tex: &PreparedTexture<'_>, xs: [usize; 4], ys: [usize; 4]) -> __m128i {
    debug_assert!(tex.in_bounds);
    let base = tex.bytes.as_ptr();
    let pitch = tex.pitch;
    let raw = [
        read_u16_unchecked(base, ys[0] * pitch + xs[0] * 2),
        read_u16_unchecked(base, ys[1] * pitch + xs[1] * 2),
        read_u16_unchecked(base, ys[2] * pitch + xs[2] * 2),
        read_u16_unchecked(base, ys[3] * pitch + xs[3] * 2),
    ];
    _mm_loadu_si128(raw.as_ptr().cast())
}

#[inline(always)]
unsafe fn bilinear16_setup<const POT: bool>(
    tex: &PreparedTexture<'_>,
    u: __m128,
    v: __m128,
) -> (
    __m128i,
    __m128i,
    __m128i,
    __m128i,
    __m128i,
    __m128i,
    __m128i,
    __m128i,
) {
    debug_assert!(tex.in_bounds);
    let half = _mm_set1_ps(0.5);
    let x = _mm_sub_ps(_mm_mul_ps(u, _mm_set1_ps(tex.width_f)), half);
    let y = _mm_sub_ps(_mm_mul_ps(v, _mm_set1_ps(tex.height_f)), half);
    let (x0, fx) = bilinear_floor_frac(x);
    let (y0, fy) = bilinear_floor_frac(y);
    let one_i = _mm_set1_epi32(1);
    let (left, upper) = resolve_xy4::<POT>(tex, x0, y0);
    let (right, lower) =
        resolve_xy4::<POT>(tex, _mm_add_epi32(x0, one_i), _mm_add_epi32(y0, one_i));

    let v256 = _mm_set1_epi32(256);
    let ifx = _mm_sub_epi32(v256, fx);
    let ify = _mm_sub_epi32(v256, fy);
    let w00 = mul_u32(ifx, ify);
    let w10 = mul_u32(fx, ify);
    let w01 = mul_u32(ifx, fy);
    let w11 = mul_u32(fx, fy);

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
    let t00 = _mm_loadu_si128(
        [
            read_u16_unchecked(base, u0 + left[0] * 2),
            read_u16_unchecked(base, u1 + left[1] * 2),
            read_u16_unchecked(base, u2 + left[2] * 2),
            read_u16_unchecked(base, u3 + left[3] * 2),
        ]
        .as_ptr()
        .cast(),
    );
    let t10 = _mm_loadu_si128(
        [
            read_u16_unchecked(base, u0 + right[0] * 2),
            read_u16_unchecked(base, u1 + right[1] * 2),
            read_u16_unchecked(base, u2 + right[2] * 2),
            read_u16_unchecked(base, u3 + right[3] * 2),
        ]
        .as_ptr()
        .cast(),
    );
    let t01 = _mm_loadu_si128(
        [
            read_u16_unchecked(base, l0 + left[0] * 2),
            read_u16_unchecked(base, l1 + left[1] * 2),
            read_u16_unchecked(base, l2 + left[2] * 2),
            read_u16_unchecked(base, l3 + left[3] * 2),
        ]
        .as_ptr()
        .cast(),
    );
    let t11 = _mm_loadu_si128(
        [
            read_u16_unchecked(base, l0 + right[0] * 2),
            read_u16_unchecked(base, l1 + right[1] * 2),
            read_u16_unchecked(base, l2 + right[2] * 2),
            read_u16_unchecked(base, l3 + right[3] * 2),
        ]
        .as_ptr()
        .cast(),
    );
    (t00, t10, t01, t11, w00, w10, w01, w11)
}

#[inline(always)]
unsafe fn blend4_u32(c: [__m128i; 4], w: [__m128i; 4]) -> __m128i {
    _mm_add_epi32(
        _mm_add_epi32(mul_u32(c[0], w[0]), mul_u32(c[1], w[1])),
        _mm_add_epi32(mul_u32(c[2], w[2]), mul_u32(c[3], w[3])),
    )
}

trait TexSample4 {
    type Scalar: TexSample;
    unsafe fn sample4(
        plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128);
}

struct NearestRgb565<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestRgb565<POT> {
    type Scalar = rust::NearestRgb565<POT>;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let xi = floor_to_i32(_mm_mul_ps(u, _mm_set1_ps(tex.width_f)));
        let yi = floor_to_i32(_mm_mul_ps(v, _mm_set1_ps(tex.height_f)));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let raw = gather_u16_4(tex, xs, ys);
        let r = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_srli_epi32::<11>(raw)),
            _mm_set1_ps(INV_31),
        );
        let g = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_and_si128(
                _mm_srli_epi32::<5>(raw),
                _mm_set1_epi32(0x3f),
            )),
            _mm_set1_ps(INV_63),
        );
        let b = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_and_si128(raw, _mm_set1_epi32(0x1f))),
            _mm_set1_ps(INV_31),
        );
        (r, g, b, _mm_set1_ps(1.0))
    }
}

struct BilinearRgb565<const POT: bool>;
impl<const POT: bool> TexSample4 for BilinearRgb565<POT> {
    type Scalar = rust::BilinearRgb565<POT>;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let (t00, t10, t01, t11, w00, w10, w01, w11) = bilinear16_setup::<POT>(tex, u, v);
        let m3f = _mm_set1_epi32(0x3f);
        let m1f = _mm_set1_epi32(0x1f);
        let r = blend4_u32(
            [
                _mm_srli_epi32::<11>(t00),
                _mm_srli_epi32::<11>(t10),
                _mm_srli_epi32::<11>(t01),
                _mm_srli_epi32::<11>(t11),
            ],
            [w00, w10, w01, w11],
        );
        let g = blend4_u32(
            [
                _mm_and_si128(_mm_srli_epi32::<5>(t00), m3f),
                _mm_and_si128(_mm_srli_epi32::<5>(t10), m3f),
                _mm_and_si128(_mm_srli_epi32::<5>(t01), m3f),
                _mm_and_si128(_mm_srli_epi32::<5>(t11), m3f),
            ],
            [w00, w10, w01, w11],
        );
        let b = blend4_u32(
            [
                _mm_and_si128(t00, m1f),
                _mm_and_si128(t10, m1f),
                _mm_and_si128(t01, m1f),
                _mm_and_si128(t11, m1f),
            ],
            [w00, w10, w01, w11],
        );
        (
            _mm_mul_ps(_mm_cvtepi32_ps(r), _mm_set1_ps(INV_31_Q16)),
            _mm_mul_ps(_mm_cvtepi32_ps(g), _mm_set1_ps(INV_63_Q16)),
            _mm_mul_ps(_mm_cvtepi32_ps(b), _mm_set1_ps(INV_31_Q16)),
            _mm_set1_ps(1.0),
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
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let xi = floor_to_i32(_mm_mul_ps(u, _mm_set1_ps(tex.width_f)));
        let yi = floor_to_i32(_mm_mul_ps(v, _mm_set1_ps(tex.height_f)));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let raw = gather_u16_4(tex, xs, ys);
        let m1f = _mm_set1_epi32(0x1f);
        let inv31 = _mm_set1_ps(INV_31);
        let r = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<10>(raw), m1f)),
            inv31,
        );
        let g = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<5>(raw), m1f)),
            inv31,
        );
        let b = _mm_mul_ps(_mm_cvtepi32_ps(_mm_and_si128(raw, m1f)), inv31);
        let a = _mm_cvtepi32_ps(_mm_srli_epi32::<15>(raw));
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
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let (t00, t10, t01, t11, w00, w10, w01, w11) = bilinear16_setup::<POT>(tex, u, v);
        let m1f = _mm_set1_epi32(0x1f);
        let r = blend4_u32(
            [
                _mm_and_si128(_mm_srli_epi32::<10>(t00), m1f),
                _mm_and_si128(_mm_srli_epi32::<10>(t10), m1f),
                _mm_and_si128(_mm_srli_epi32::<10>(t01), m1f),
                _mm_and_si128(_mm_srli_epi32::<10>(t11), m1f),
            ],
            [w00, w10, w01, w11],
        );
        let g = blend4_u32(
            [
                _mm_and_si128(_mm_srli_epi32::<5>(t00), m1f),
                _mm_and_si128(_mm_srli_epi32::<5>(t10), m1f),
                _mm_and_si128(_mm_srli_epi32::<5>(t01), m1f),
                _mm_and_si128(_mm_srli_epi32::<5>(t11), m1f),
            ],
            [w00, w10, w01, w11],
        );
        let b = blend4_u32(
            [
                _mm_and_si128(t00, m1f),
                _mm_and_si128(t10, m1f),
                _mm_and_si128(t01, m1f),
                _mm_and_si128(t11, m1f),
            ],
            [w00, w10, w01, w11],
        );
        let a = blend4_u32(
            [
                _mm_srli_epi32::<15>(t00),
                _mm_srli_epi32::<15>(t10),
                _mm_srli_epi32::<15>(t01),
                _mm_srli_epi32::<15>(t11),
            ],
            [w00, w10, w01, w11],
        );
        let inv31 = _mm_set1_ps(INV_31_Q16);
        (
            _mm_mul_ps(_mm_cvtepi32_ps(r), inv31),
            _mm_mul_ps(_mm_cvtepi32_ps(g), inv31),
            _mm_mul_ps(_mm_cvtepi32_ps(b), inv31),
            _mm_mul_ps(_mm_cvtepi32_ps(a), _mm_set1_ps(INV_65536)),
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
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let xi = floor_to_i32(_mm_mul_ps(u, _mm_set1_ps(tex.width_f)));
        let yi = floor_to_i32(_mm_mul_ps(v, _mm_set1_ps(tex.height_f)));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let raw = gather_u16_4(tex, xs, ys);
        let mf = _mm_set1_epi32(0xf);
        let inv15 = _mm_set1_ps(INV_15);
        let r = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<8>(raw), mf)),
            inv15,
        );
        let g = _mm_mul_ps(
            _mm_cvtepi32_ps(_mm_and_si128(_mm_srli_epi32::<4>(raw), mf)),
            inv15,
        );
        let b = _mm_mul_ps(_mm_cvtepi32_ps(_mm_and_si128(raw, mf)), inv15);
        let a = _mm_mul_ps(_mm_cvtepi32_ps(_mm_srli_epi32::<12>(raw)), inv15);
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
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let (t00, t10, t01, t11, w00, w10, w01, w11) = bilinear16_setup::<POT>(tex, u, v);
        let mf = _mm_set1_epi32(0xf);
        let r = blend4_u32(
            [
                _mm_and_si128(_mm_srli_epi32::<8>(t00), mf),
                _mm_and_si128(_mm_srli_epi32::<8>(t10), mf),
                _mm_and_si128(_mm_srli_epi32::<8>(t01), mf),
                _mm_and_si128(_mm_srli_epi32::<8>(t11), mf),
            ],
            [w00, w10, w01, w11],
        );
        let g = blend4_u32(
            [
                _mm_and_si128(_mm_srli_epi32::<4>(t00), mf),
                _mm_and_si128(_mm_srli_epi32::<4>(t10), mf),
                _mm_and_si128(_mm_srli_epi32::<4>(t01), mf),
                _mm_and_si128(_mm_srli_epi32::<4>(t11), mf),
            ],
            [w00, w10, w01, w11],
        );
        let b = blend4_u32(
            [
                _mm_and_si128(t00, mf),
                _mm_and_si128(t10, mf),
                _mm_and_si128(t01, mf),
                _mm_and_si128(t11, mf),
            ],
            [w00, w10, w01, w11],
        );
        let a = blend4_u32(
            [
                _mm_srli_epi32::<12>(t00),
                _mm_srli_epi32::<12>(t10),
                _mm_srli_epi32::<12>(t01),
                _mm_srli_epi32::<12>(t11),
            ],
            [w00, w10, w01, w11],
        );
        let inv15 = _mm_set1_ps(INV_15_Q16);
        (
            _mm_mul_ps(_mm_cvtepi32_ps(r), inv15),
            _mm_mul_ps(_mm_cvtepi32_ps(g), inv15),
            _mm_mul_ps(_mm_cvtepi32_ps(b), inv15),
            _mm_mul_ps(_mm_cvtepi32_ps(a), inv15),
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
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        debug_assert!(tex.in_bounds);
        let xi = floor_to_i32(_mm_mul_ps(u, _mm_set1_ps(tex.width_f)));
        let yi = floor_to_i32(_mm_mul_ps(v, _mm_set1_ps(tex.height_f)));
        let (xs, ys) = resolve_xy4::<POT>(tex, xi, yi);
        let pal = plan.palette_f32();
        let base = tex.bytes.as_ptr();
        let pitch = tex.pitch;
        let c0 = _mm_loadu_ps(pal[usize::from(*base.add(ys[0] * pitch + xs[0]))].as_ptr());
        let c1 = _mm_loadu_ps(pal[usize::from(*base.add(ys[1] * pitch + xs[1]))].as_ptr());
        let c2 = _mm_loadu_ps(pal[usize::from(*base.add(ys[2] * pitch + xs[2]))].as_ptr());
        let c3 = _mm_loadu_ps(pal[usize::from(*base.add(ys[3] * pitch + xs[3]))].as_ptr());
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
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let mut us = [0.0f32; 4];
        let mut vs = [0.0f32; 4];
        _mm_storeu_ps(us.as_mut_ptr(), u);
        _mm_storeu_ps(vs.as_mut_ptr(), v);
        let c0 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[0], vs[0]);
        let c1 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[1], vs[1]);
        let c2 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[2], vs[2]);
        let c3 = rust::BilinearPaletted8::<POT>::sample(plan, tex, us[3], vs[3]);
        transpose4(
            _mm_loadu_ps(c0.as_ptr()),
            _mm_loadu_ps(c1.as_ptr()),
            _mm_loadu_ps(c2.as_ptr()),
            _mm_loadu_ps(c3.as_ptr()),
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
        span_untextured_flat_opaque_sse2::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[target_feature(enable = "sse2")]
unsafe fn span_untextured_flat_opaque_sse2<const Z_TEST: bool, const SPECULAR: bool>(
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
    let packed_i16 = u16::from_le_bytes(pack_rgb565_rgb(r, g, b)) as i16;
    let packed4 = _mm_set1_epi16(packed_i16);
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
        let zv = _mm_loadu_ps([z0, z1, z2, z3].as_ptr());
        let dv = _mm_loadu_ps(d_ptr.add(i));
        let z_mask = _mm_cmple_ps(zv, dv);
        if _mm_movemask_ps(z_mask) != 0 {
            if depth_write {
                _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
            }
            let old_px = load_u16x4(p_ptr.add(i * 2));
            let z_mask_i = _mm_castps_si128(z_mask);
            let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
            store_u16x4(p_ptr.add(i * 2), select_si128(mask16, packed4, old_px));
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
        span_untextured_gouraud_opaque_sse2::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[target_feature(enable = "sse2")]
unsafe fn span_untextured_gouraud_opaque_sse2<const Z_TEST: bool, const SPECULAR: bool>(
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
    let one = _mm_set1_ps(1.0);
    let mut s0 = _mm_loadu_ps(value.as_ptr());
    let d0 = _mm_loadu_ps(step.as_ptr());
    let mut s1 = _mm_loadu_ps(value.as_ptr().add(4));
    let d1 = _mm_loadu_ps(step.as_ptr().add(4));
    let mut s2 = _mm_loadu_ps(value.as_ptr().add(8));
    let d2 = _mm_loadu_ps(step.as_ptr().add(8));
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
        let t0 = _mm_unpacklo_ps(p0, p1);
        let t2 = _mm_unpacklo_ps(p2, p3);
        let (zv, dv, z_mask) = if Z_TEST {
            let zv = _mm_movelh_ps(t0, t2);
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            if _mm_movemask_ps(z_mask) == 0 {
                i += 4;
                continue;
            }
            (zv, dv, z_mask)
        } else {
            (p0, p0, _mm_castsi128_ps(_mm_set1_epi32(-1)))
        };
        let wv = _mm_movehl_ps(t2, t0);
        let (rv, gv, bv, _) = transpose4(q0, q1, q2, q3);
        let near = persp4(wv);
        let mut r = _mm_mul_ps(rv, near);
        let mut g = _mm_mul_ps(gv, near);
        let mut b = _mm_mul_ps(bv, near);
        if SPECULAR {
            let (m0, m1, m2, m3) = r2.unwrap();
            let (sr, sg, sb, _) = transpose4(m0, m1, m2, m3);
            r = _mm_min_ps(_mm_add_ps(r, _mm_mul_ps(sr, near)), one);
            g = _mm_min_ps(_mm_add_ps(g, _mm_mul_ps(sg, near)), one);
            b = _mm_min_ps(_mm_add_ps(b, _mm_mul_ps(sb, near)), one);
        }
        let new_px = pack_rgb565_4(r, g, b);
        if Z_TEST {
            if depth_write {
                _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
            }
            let old_px = load_u16x4(p_ptr.add(i * 2));
            let z_mask_i = _mm_castps_si128(z_mask);
            let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
            store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 < n {
        _mm_storeu_ps(value.as_mut_ptr(), s0);
        _mm_storeu_ps(value.as_mut_ptr().add(4), s1);
        if SPECULAR {
            _mm_storeu_ps(value.as_mut_ptr().add(8), s2);
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
        span_untextured_alpha_sse2::<Z_LE, ALPHA_GT>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[target_feature(enable = "sse2")]
unsafe fn span_untextured_alpha_sse2<const Z_LE: bool, const ALPHA_GT: bool>(
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
        let isa = _mm_set1_ps(1.0 - sa);
        let sr_sa = _mm_set1_ps(flat.0[0] * sa);
        let sg_sa = _mm_set1_ps(flat.0[1] * sa);
        let sb_sa = _mm_set1_ps(flat.0[2] * sa);
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
                let zv = _mm_loadu_ps([z0, z1, z2, z3].as_ptr());
                let dv = _mm_loadu_ps(d_ptr.add(i));
                let z_mask = _mm_cmple_ps(zv, dv);
                if _mm_movemask_ps(z_mask) != 0 {
                    let old_px = load_u16x4(p_ptr.add(i * 2));
                    let (ur, ug, ub) = unpack_rgb565_4(old_px);
                    let out_r = _mm_add_ps(sr_sa, _mm_mul_ps(ur, isa));
                    let out_g = _mm_add_ps(sg_sa, _mm_mul_ps(ug, isa));
                    let out_b = _mm_add_ps(sb_sa, _mm_mul_ps(ub, isa));
                    let new_px = pack_rgb565_4(out_r, out_g, out_b);
                    let z_mask_i = _mm_castps_si128(z_mask);
                    let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
                    store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
                    if depth_write {
                        _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
                    }
                }
            } else {
                let old_px = load_u16x4(p_ptr.add(i * 2));
                let (ur, ug, ub) = unpack_rgb565_4(old_px);
                let out_r = _mm_add_ps(sr_sa, _mm_mul_ps(ur, isa));
                let out_g = _mm_add_ps(sg_sa, _mm_mul_ps(ug, isa));
                let out_b = _mm_add_ps(sb_sa, _mm_mul_ps(ub, isa));
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
        let one = _mm_set1_ps(1.0);
        let mut s0 = _mm_loadu_ps(value.as_ptr());
        let d0 = _mm_loadu_ps(step.as_ptr());
        let mut s1 = _mm_loadu_ps(value.as_ptr().add(4));
        let d1 = _mm_loadu_ps(step.as_ptr().add(4));
        let mut i = 0;
        while i < n4 {
            let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
            let (q0, q1, q2, q3) = step4_raw(&mut s1, d1);
            let t0 = _mm_unpacklo_ps(p0, p1);
            let t2 = _mm_unpacklo_ps(p2, p3);
            let (zv, dv, mut write_mask) = if Z_LE {
                let zv = _mm_movelh_ps(t0, t2);
                let dv = _mm_loadu_ps(d_ptr.add(i));
                let z_mask = _mm_cmple_ps(zv, dv);
                if _mm_movemask_ps(z_mask) == 0 {
                    i += 4;
                    continue;
                }
                (zv, dv, z_mask)
            } else {
                (p0, p0, _mm_castsi128_ps(_mm_set1_epi32(-1)))
            };
            let wv = _mm_movehl_ps(t2, t0);
            let (cr, cg, cb, ca) = transpose4(q0, q1, q2, q3);
            let near = persp4(wv);
            let sa = _mm_mul_ps(ca, near);
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(sa, alpha_ref);
                write_mask = if Z_LE {
                    _mm_and_ps(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if _mm_movemask_ps(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            let r = _mm_mul_ps(cr, near);
            let g = _mm_mul_ps(cg, near);
            let b = _mm_mul_ps(cb, near);
            let isa = _mm_sub_ps(one, sa);
            let old_px = load_u16x4(p_ptr.add(i * 2));
            let (ur, ug, ub) = unpack_rgb565_4(old_px);
            let out_r = _mm_add_ps(_mm_mul_ps(r, sa), _mm_mul_ps(ur, isa));
            let out_g = _mm_add_ps(_mm_mul_ps(g, sa), _mm_mul_ps(ug, isa));
            let out_b = _mm_add_ps(_mm_mul_ps(b, sa), _mm_mul_ps(ub, isa));
            let new_px = pack_rgb565_4(out_r, out_g, out_b);
            if Z_LE || ALPHA_GT {
                let wm_i = _mm_castps_si128(write_mask);
                let mask16 = _mm_packs_epi32(wm_i, wm_i);
                store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
                if Z_LE && depth_write {
                    _mm_storeu_ps(d_ptr.add(i), select_ps(write_mask, zv, dv));
                }
            } else {
                store_u16x4(p_ptr.add(i * 2), new_px);
            }
            i += 4;
        }
        if n4 < n {
            _mm_storeu_ps(value.as_mut_ptr(), s0);
            _mm_storeu_ps(value.as_mut_ptr().add(4), s1);
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
        span_textured_copy_opaque_sse2::<S, Z_TEST>(plan, depth, pixels, value, step, flat, states);
    }
}

#[target_feature(enable = "sse2")]
unsafe fn span_textured_copy_opaque_sse2<S: TexSample4, const Z_TEST: bool>(
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
    let mut s0 = _mm_loadu_ps(value.as_ptr());
    let d0 = _mm_loadu_ps(step.as_ptr());
    let d_ptr = depth.as_mut_ptr();
    let p_ptr = pixels.as_mut_ptr();
    let mut i = 0;
    while i < n4 {
        if Z_TEST {
            let (p0, p1, p2, p3) = step4_raw(&mut s0, d0);
            let t0 = _mm_unpacklo_ps(p0, p1);
            let t2 = _mm_unpacklo_ps(p2, p3);
            let zv = _mm_movelh_ps(t0, t2);
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            if _mm_movemask_ps(z_mask) != 0 {
                let wv = _mm_movehl_ps(t2, t0);
                let t1 = _mm_unpackhi_ps(p0, p1);
                let t3 = _mm_unpackhi_ps(p2, p3);
                let uv = _mm_movelh_ps(t1, t3);
                let vv = _mm_movehl_ps(t3, t1);
                let near = persp4(wv);
                let (tr, tg, tb, _) =
                    S::sample4(plan, tex, _mm_mul_ps(uv, near), _mm_mul_ps(vv, near));
                let new_px = pack_rgb565_4(tr, tg, tb);
                if depth_write {
                    _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
                }
                let old_px = load_u16x4(p_ptr.add(i * 2));
                let z_mask_i = _mm_castps_si128(z_mask);
                let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
                store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
            }
        } else {
            let (_, wv, uv, vv) = step4_transpose(&mut s0, d0);
            let near = persp4(wv);
            let (tr, tg, tb, _) = S::sample4(plan, tex, _mm_mul_ps(uv, near), _mm_mul_ps(vv, near));
            let new_px = pack_rgb565_4(tr, tg, tb);
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 < n {
        _mm_storeu_ps(value.as_mut_ptr(), s0);
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
        span_textured_modulate_opaque_sse2::<S, Z_TEST, FLAT, SPECULAR, FOG>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[target_feature(enable = "sse2")]
unsafe fn span_textured_modulate_opaque_sse2<
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
    let flat_r = _mm_set1_ps(flat.0[0]);
    let flat_g = _mm_set1_ps(flat.0[1]);
    let flat_b = _mm_set1_ps(flat.0[2]);
    let flat_sr = _mm_set1_ps(flat.1[0]);
    let flat_sg = _mm_set1_ps(flat.1[1]);
    let flat_sb = _mm_set1_ps(flat.1[2]);
    let fog_colour = states.fog.unwrap_or([0.0; 3]);
    let fog_r = _mm_set1_ps(fog_colour[0]);
    let fog_g = _mm_set1_ps(fog_colour[1]);
    let fog_b = _mm_set1_ps(fog_colour[2]);
    let one = _mm_set1_ps(1.0);

    let mut s0 = _mm_loadu_ps(value.as_ptr());
    let d0 = _mm_loadu_ps(step.as_ptr());
    let mut s1 = _mm_loadu_ps(value.as_ptr().add(4));
    let d1 = _mm_loadu_ps(step.as_ptr().add(4));
    let mut s2 = _mm_loadu_ps(value.as_ptr().add(8));
    let d2 = _mm_loadu_ps(step.as_ptr().add(8));

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

        let t0 = _mm_unpacklo_ps(p0, p1);
        let t2 = _mm_unpacklo_ps(p2, p3);
        let zv = _mm_movelh_ps(t0, t2);
        let (dv, z_mask) = if Z_TEST {
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            if _mm_movemask_ps(z_mask) == 0 {
                i += 4;
                continue;
            }
            (dv, z_mask)
        } else {
            (zv, _mm_castsi128_ps(_mm_set1_epi32(-1)))
        };

        let wv = _mm_movehl_ps(t2, t0);
        let t1 = _mm_unpackhi_ps(p0, p1);
        let t3 = _mm_unpackhi_ps(p2, p3);
        let uv = _mm_movelh_ps(t1, t3);
        let vv = _mm_movehl_ps(t3, t1);

        let near = persp4(wv);
        let (tr, tg, tb, _) = S::sample4(plan, tex, _mm_mul_ps(uv, near), _mm_mul_ps(vv, near));

        let (mut r, mut g, mut b) = if FLAT {
            (
                _mm_mul_ps(tr, flat_r),
                _mm_mul_ps(tg, flat_g),
                _mm_mul_ps(tb, flat_b),
            )
        } else {
            let (q0, q1, q2, q3) = q.unwrap();
            let (cr, cg, cb, _) = transpose4(q0, q1, q2, q3);
            (
                _mm_mul_ps(tr, _mm_mul_ps(cr, near)),
                _mm_mul_ps(tg, _mm_mul_ps(cg, near)),
                _mm_mul_ps(tb, _mm_mul_ps(cb, near)),
            )
        };

        if FLAT && SPECULAR {
            r = _mm_min_ps(_mm_add_ps(r, flat_sr), one);
            g = _mm_min_ps(_mm_add_ps(g, flat_sg), one);
            b = _mm_min_ps(_mm_add_ps(b, flat_sb), one);
        }

        if (!FLAT && SPECULAR) || FOG {
            let (m0, m1, m2, m3) = r2.unwrap();
            let (sr, sg, sb, fw) = transpose4(m0, m1, m2, m3);
            if !FLAT && SPECULAR {
                r = _mm_min_ps(_mm_add_ps(r, _mm_mul_ps(sr, near)), one);
                g = _mm_min_ps(_mm_add_ps(g, _mm_mul_ps(sg, near)), one);
                b = _mm_min_ps(_mm_add_ps(b, _mm_mul_ps(sb, near)), one);
            }
            if FOG {
                let f = clamp_nan_preserving_01(_mm_mul_ps(fw, near));
                let inv_f = _mm_sub_ps(one, f);
                r = _mm_add_ps(_mm_mul_ps(r, f), _mm_mul_ps(fog_r, inv_f));
                g = _mm_add_ps(_mm_mul_ps(g, f), _mm_mul_ps(fog_g, inv_f));
                b = _mm_add_ps(_mm_mul_ps(b, f), _mm_mul_ps(fog_b, inv_f));
            }
        }

        let new_px = pack_rgb565_4(r, g, b);
        if Z_TEST {
            if depth_write {
                _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
            }
            let old_px = load_u16x4(p_ptr.add(i * 2));
            let z_mask_i = _mm_castps_si128(z_mask);
            let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
            store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }

    if n4 < n {
        _mm_storeu_ps(value.as_mut_ptr(), s0);
        if !FLAT {
            _mm_storeu_ps(value.as_mut_ptr().add(4), s1);
        }
        if (!FLAT && SPECULAR) || FOG {
            _mm_storeu_ps(value.as_mut_ptr().add(8), s2);
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
        span_textured_modulate_blend_sse2::<S, Z_LE, ALPHA_GT, BLEND, FLAT, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[target_feature(enable = "sse2")]
unsafe fn span_textured_modulate_blend_sse2<
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
    let flat_r = _mm_set1_ps(flat.0[0]);
    let flat_g = _mm_set1_ps(flat.0[1]);
    let flat_b = _mm_set1_ps(flat.0[2]);
    let flat_a = _mm_set1_ps(flat.0[3]);
    let flat_sr = _mm_set1_ps(flat.1[0]);
    let flat_sg = _mm_set1_ps(flat.1[1]);
    let flat_sb = _mm_set1_ps(flat.1[2]);
    let zero = _mm_setzero_ps();
    let one = _mm_set1_ps(1.0);
    let alpha_combine = states.alpha;
    let alpha_ref = states.alpha_test.map_or(0, |(_, r)| r);
    let depth_write = states.depth_write;

    let mut s0 = _mm_loadu_ps(value.as_ptr());
    let d0 = _mm_loadu_ps(step.as_ptr());
    let mut s1 = _mm_loadu_ps(value.as_ptr().add(4));
    let d1 = _mm_loadu_ps(step.as_ptr().add(4));
    let mut s2 = _mm_loadu_ps(value.as_ptr().add(8));
    let d2 = _mm_loadu_ps(step.as_ptr().add(8));

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

        let t0 = _mm_unpacklo_ps(p0, p1);
        let t2 = _mm_unpacklo_ps(p2, p3);
        let (zv, dv, mut write_mask) = if Z_LE {
            let zv = _mm_movelh_ps(t0, t2);
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            if _mm_movemask_ps(z_mask) == 0 {
                i += 4;
                continue;
            }
            (zv, dv, z_mask)
        } else {
            (p0, p0, _mm_castsi128_ps(_mm_set1_epi32(-1)))
        };

        let wv = _mm_movehl_ps(t2, t0);
        let t1 = _mm_unpackhi_ps(p0, p1);
        let t3 = _mm_unpackhi_ps(p2, p3);
        let uv = _mm_movelh_ps(t1, t3);
        let vv = _mm_movehl_ps(t3, t1);

        let near = persp4(wv);
        let (tr, tg, tb, ta) = S::sample4(plan, tex, _mm_mul_ps(uv, near), _mm_mul_ps(vv, near));

        let (sa, diffuse_opt) = if alpha_combine == Combine::Texture {
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(ta, alpha_ref);
                write_mask = if Z_LE {
                    _mm_and_ps(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if _mm_movemask_ps(write_mask) == 0 {
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
                    _mm_and_ps(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if _mm_movemask_ps(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            (sa, None)
        } else {
            let (q0, q1, q2, q3) = q.unwrap();
            let (cr, cg, cb, ca) = transpose4(q0, q1, q2, q3);
            let sa = combine_4(alpha_combine, ta, _mm_mul_ps(ca, near));
            if ALPHA_GT {
                let alpha_mask = alpha_gt_mask4(sa, alpha_ref);
                write_mask = if Z_LE {
                    _mm_and_ps(write_mask, alpha_mask)
                } else {
                    alpha_mask
                };
                if _mm_movemask_ps(write_mask) == 0 {
                    i += 4;
                    continue;
                }
            }
            (sa, Some((cr, cg, cb)))
        };

        let (mut r, mut g, mut b) = if FLAT {
            (
                _mm_mul_ps(tr, flat_r),
                _mm_mul_ps(tg, flat_g),
                _mm_mul_ps(tb, flat_b),
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
                _mm_mul_ps(tr, _mm_mul_ps(cr, near)),
                _mm_mul_ps(tg, _mm_mul_ps(cg, near)),
                _mm_mul_ps(tb, _mm_mul_ps(cb, near)),
            )
        };

        if SPECULAR {
            if FLAT {
                r = _mm_min_ps(_mm_add_ps(r, flat_sr), one);
                g = _mm_min_ps(_mm_add_ps(g, flat_sg), one);
                b = _mm_min_ps(_mm_add_ps(b, flat_sb), one);
            } else {
                let (m0, m1, m2, m3) = r2.unwrap();
                let (sr, sg, sb, _) = transpose4(m0, m1, m2, m3);
                r = _mm_min_ps(_mm_add_ps(r, _mm_mul_ps(sr, near)), one);
                g = _mm_min_ps(_mm_add_ps(g, _mm_mul_ps(sg, near)), one);
                b = _mm_min_ps(_mm_add_ps(b, _mm_mul_ps(sb, near)), one);
            }
        }

        let old_px = load_u16x4(p_ptr.add(i * 2));
        let (ur, ug, ub) = unpack_rgb565_4(old_px);
        let (out_r, out_g, out_b) = match BLEND {
            BLEND_ALPHA => {
                let isa = _mm_sub_ps(one, sa);
                (
                    _mm_add_ps(_mm_mul_ps(r, sa), _mm_mul_ps(ur, isa)),
                    _mm_add_ps(_mm_mul_ps(g, sa), _mm_mul_ps(ug, isa)),
                    _mm_add_ps(_mm_mul_ps(b, sa), _mm_mul_ps(ub, isa)),
                )
            }
            BLEND_ADDITIVE_ALPHA => (
                _mm_add_ps(_mm_mul_ps(r, sa), ur),
                _mm_add_ps(_mm_mul_ps(g, sa), ug),
                _mm_add_ps(_mm_mul_ps(b, sa), ub),
            ),
            BLEND_INV_SRC_COLOUR => (
                _mm_add_ps(_mm_mul_ps(r, zero), _mm_mul_ps(ur, _mm_sub_ps(one, r))),
                _mm_add_ps(_mm_mul_ps(g, zero), _mm_mul_ps(ug, _mm_sub_ps(one, g))),
                _mm_add_ps(_mm_mul_ps(b, zero), _mm_mul_ps(ub, _mm_sub_ps(one, b))),
            ),
            _ => unreachable!(),
        };
        let new_px = pack_rgb565_4(out_r, out_g, out_b);
        if Z_LE || ALPHA_GT {
            let wm_i = _mm_castps_si128(write_mask);
            let mask16 = _mm_packs_epi32(wm_i, wm_i);
            store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
            if Z_LE && depth_write {
                _mm_storeu_ps(d_ptr.add(i), select_ps(write_mask, zv, dv));
            }
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }

    if n4 < n {
        _mm_storeu_ps(value.as_mut_ptr(), s0);
        if !FLAT {
            _mm_storeu_ps(value.as_mut_ptr().add(4), s1);
        }
        if !FLAT && SPECULAR {
            _mm_storeu_ps(value.as_mut_ptr().add(8), s2);
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
