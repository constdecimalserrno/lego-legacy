//! 4-pixel `x86_64` baseline SSE2 interior-span kernels (ADR-0009, #255, #278, #281).
//!
//! Mirrors [`super::aarch64`] with an identical 4-lane algorithm using only
//! baseline SSE2 (zero runtime feature dispatch, 100% `x86_64` desktop
//! coverage), preserving Golden Rule 1 bit-for-bit against [`super::rust`]:
//! - Sequential plane stepping (`v += step`) per pixel via 4-lane vector adds + 4×4 transpose;
//! - Exact IEEE-754 `_mm_div_ps` (no approximate reciprocal);
//! - Separate rounded `_mm_mul_ps` and `_mm_add_ps` (no FMA);
//! - Algebraically exact separable 16-bit integer bilinear (`_mm_mullo_epi16` / `_mm_madd_epi16`)
//!   with paired 32-bit non-seam loads;
//! - Exact round-half-away-from-zero (`t + (frac >= 0.5)`) for `alpha_test == Greater`;
//! - Exact float RGB565 packing (`_mm_cvttps_epi32` on `[0.0, 1.0]`,
//!   matching saturating `as u32` on every input including `NaN`);
//! - Early `n < 4` exit and `#[inline(always)]` `0..3` pixel scalar tails delegated to `rust.rs`.

use core::arch::x86_64::{
    __m128, __m128i, _mm_add_epi16, _mm_add_epi32, _mm_add_ps, _mm_and_ps, _mm_and_si128,
    _mm_andnot_ps, _mm_andnot_si128, _mm_castps_si128, _mm_castsi128_ps, _mm_cmpeq_epi32,
    _mm_cmpeq_ps, _mm_cmpge_ps, _mm_cmpgt_epi32, _mm_cmpgt_ps, _mm_cmple_ps, _mm_cmplt_ps,
    _mm_cvtepi32_ps, _mm_cvtsi128_si64, _mm_cvtsi32_si128, _mm_cvtsi64_si128, _mm_cvttps_epi32,
    _mm_div_ps, _mm_insert_epi16, _mm_loadu_ps, _mm_loadu_si128, _mm_madd_epi16, _mm_max_ps,
    _mm_min_ps, _mm_movehl_ps, _mm_movelh_ps, _mm_movemask_ps, _mm_mul_ps, _mm_mullo_epi16,
    _mm_or_ps, _mm_or_si128, _mm_packs_epi32, _mm_set1_epi16, _mm_set1_epi32, _mm_set1_ps,
    _mm_set_epi64x, _mm_set_ps, _mm_setzero_ps, _mm_setzero_si128, _mm_slli_epi16, _mm_slli_epi32,
    _mm_srai_epi32, _mm_srli_epi16, _mm_srli_epi32, _mm_srli_si128, _mm_storeu_ps, _mm_sub_epi16,
    _mm_sub_epi32, _mm_sub_ps, _mm_unpackhi_ps, _mm_unpacklo_epi16, _mm_unpacklo_ps,
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
unsafe fn pack_rgb565_4_unclamped(r: __m128, g: __m128, b: __m128) -> __m128i {
    let v255 = _mm_set1_ps(255.0);
    let half = _mm_set1_ps(0.5);

    // Separate mul then add (ADR-0009: no FMA). Caller guarantees finite inputs in [0.0, 1.0].
    let ri = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(r, v255), half));
    let gi = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(g, v255), half));
    let bi = _mm_cvttps_epi32(_mm_add_ps(_mm_mul_ps(b, v255), half));

    let packed = _mm_or_si128(
        _mm_or_si128(
            _mm_slli_epi32::<11>(_mm_srli_epi32::<3>(ri)),
            _mm_slli_epi32::<5>(_mm_srli_epi32::<2>(gi)),
        ),
        _mm_srli_epi32::<3>(bi),
    );
    let sign_ext = _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(packed));
    _mm_packs_epi32(sign_ext, sign_ext)
}

#[inline(always)]
unsafe fn all_zero_f32(v: __m128) -> bool {
    _mm_movemask_ps(_mm_cmpeq_ps(v, _mm_setzero_ps())) == 0xf
}

#[inline(always)]
unsafe fn all_finite3(r: __m128, g: __m128, b: __m128) -> bool {
    let s = _mm_add_ps(
        _mm_add_ps(_mm_sub_ps(r, r), _mm_sub_ps(g, g)),
        _mm_sub_ps(b, b),
    );
    _mm_movemask_ps(_mm_cmpeq_ps(s, _mm_setzero_ps())) == 0xf
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
unsafe fn extract_u32x4_as_usize(v: __m128i) -> [usize; 4] {
    let lo = _mm_cvtsi128_si64(v) as u64;
    let hi = _mm_cvtsi128_si64(_mm_srli_si128::<8>(v)) as u64;
    [
        (lo as u32) as usize,
        (lo >> 32) as usize,
        (hi as u32) as usize,
        (hi >> 32) as usize,
    ]
}

#[inline(always)]
unsafe fn extract_i32x4(v: __m128i) -> [i32; 4] {
    let lo = _mm_cvtsi128_si64(v) as u64;
    let hi = _mm_cvtsi128_si64(_mm_srli_si128::<8>(v)) as u64;
    [lo as i32, (lo >> 32) as i32, hi as i32, (hi >> 32) as i32]
}

#[inline(always)]
unsafe fn resolve_xy4<const POT: bool>(
    tex: &PreparedTexture<'_>,
    xv: __m128i,
    yv: __m128i,
) -> ([usize; 4], [usize; 4]) {
    if POT {
        let xm = _mm_and_si128(xv, _mm_set1_epi32(tex.pot_x_mask));
        let ym = _mm_and_si128(yv, _mm_set1_epi32(tex.pot_y_mask));
        (extract_u32x4_as_usize(xm), extract_u32x4_as_usize(ym))
    } else {
        let xs = extract_i32x4(xv);
        let ys = extract_i32x4(yv);
        let (x0, y0) = resolve_xy::<false>(tex, xs[0], ys[0]);
        let (x1, y1) = resolve_xy::<false>(tex, xs[1], ys[1]);
        let (x2, y2) = resolve_xy::<false>(tex, xs[2], ys[2]);
        let (x3, y3) = resolve_xy::<false>(tex, xs[3], ys[3]);
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
unsafe fn gather_u16_4(tex: &PreparedTexture<'_>, xs: [usize; 4], ys: [usize; 4]) -> __m128i {
    debug_assert!(tex.in_bounds);
    let base = tex.bytes.as_ptr();
    let pitch = tex.pitch;
    let v = _mm_cvtsi32_si128(i32::from(read_u16_le(base, ys[0] * pitch + xs[0] * 2)));
    let v = _mm_insert_epi16::<1>(v, i32::from(read_u16_le(base, ys[1] * pitch + xs[1] * 2)));
    let v = _mm_insert_epi16::<2>(v, i32::from(read_u16_le(base, ys[2] * pitch + xs[2] * 2)));
    let v = _mm_insert_epi16::<3>(v, i32::from(read_u16_le(base, ys[3] * pitch + xs[3] * 2)));
    _mm_unpacklo_epi16(v, _mm_setzero_si128())
}

#[inline(always)]
unsafe fn load_bilinear16_pairs(
    base: *const u8,
    up_rows: [usize; 4],
    lo_rows: [usize; 4],
    left: [usize; 4],
) -> (__m128i, __m128i) {
    let pu0 = u64::from(read_u32_le(base, up_rows[0] + left[0] * 2));
    let pu1 = u64::from(read_u32_le(base, up_rows[1] + left[1] * 2));
    let pu2 = u64::from(read_u32_le(base, up_rows[2] + left[2] * 2));
    let pu3 = u64::from(read_u32_le(base, up_rows[3] + left[3] * 2));
    let up = _mm_set_epi64x(((pu3 << 32) | pu2) as i64, ((pu1 << 32) | pu0) as i64);

    let pl0 = u64::from(read_u32_le(base, lo_rows[0] + left[0] * 2));
    let pl1 = u64::from(read_u32_le(base, lo_rows[1] + left[1] * 2));
    let pl2 = u64::from(read_u32_le(base, lo_rows[2] + left[2] * 2));
    let pl3 = u64::from(read_u32_le(base, lo_rows[3] + left[3] * 2));
    let lo = _mm_set_epi64x(((pl3 << 32) | pl2) as i64, ((pl1 << 32) | pl0) as i64);

    let up_l = _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(up));
    let lo_l = _mm_srai_epi32::<16>(_mm_slli_epi32::<16>(lo));
    let left8 = _mm_packs_epi32(up_l, lo_l);

    let up_r = _mm_srai_epi32::<16>(up);
    let lo_r = _mm_srai_epi32::<16>(lo);
    let right8 = _mm_packs_epi32(up_r, lo_r);

    (left8, right8)
}

#[inline(always)]
unsafe fn load_bilinear16_seam<const POT: bool>(
    tex: &PreparedTexture<'_>,
    x0: __m128i,
    y0: __m128i,
) -> (__m128i, __m128i) {
    let one_i = _mm_set1_epi32(1);
    let (left, upper) = resolve_xy4::<POT>(tex, x0, y0);
    let (right, lower) =
        resolve_xy4::<POT>(tex, _mm_add_epi32(x0, one_i), _mm_add_epi32(y0, one_i));
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

    let l = _mm_cvtsi32_si128(i32::from(read_u16_le(base, u0 + left[0] * 2)));
    let l = _mm_insert_epi16::<1>(l, i32::from(read_u16_le(base, u1 + left[1] * 2)));
    let l = _mm_insert_epi16::<2>(l, i32::from(read_u16_le(base, u2 + left[2] * 2)));
    let l = _mm_insert_epi16::<3>(l, i32::from(read_u16_le(base, u3 + left[3] * 2)));
    let l = _mm_insert_epi16::<4>(l, i32::from(read_u16_le(base, l0 + left[0] * 2)));
    let l = _mm_insert_epi16::<5>(l, i32::from(read_u16_le(base, l1 + left[1] * 2)));
    let l = _mm_insert_epi16::<6>(l, i32::from(read_u16_le(base, l2 + left[2] * 2)));
    let l = _mm_insert_epi16::<7>(l, i32::from(read_u16_le(base, l3 + left[3] * 2)));

    let r = _mm_cvtsi32_si128(i32::from(read_u16_le(base, u0 + right[0] * 2)));
    let r = _mm_insert_epi16::<1>(r, i32::from(read_u16_le(base, u1 + right[1] * 2)));
    let r = _mm_insert_epi16::<2>(r, i32::from(read_u16_le(base, u2 + right[2] * 2)));
    let r = _mm_insert_epi16::<3>(r, i32::from(read_u16_le(base, u3 + right[3] * 2)));
    let r = _mm_insert_epi16::<4>(r, i32::from(read_u16_le(base, l0 + right[0] * 2)));
    let r = _mm_insert_epi16::<5>(r, i32::from(read_u16_le(base, l1 + right[1] * 2)));
    let r = _mm_insert_epi16::<6>(r, i32::from(read_u16_le(base, l2 + right[2] * 2)));
    let r = _mm_insert_epi16::<7>(r, i32::from(read_u16_le(base, l3 + right[3] * 2)));

    (l, r)
}

/// Load the 4×(2×2) bilinear corners packed as `left8 = [t00; t01]` and
/// `right8 = [t10; t11]`, plus exact integer weights `fx8 = [fx; fx]` and
/// `fy_pair = [256-fy0, fy0, ...]`. Non-seam quads issue 8 paired `u32` loads.
#[inline(always)]
unsafe fn bilinear16_load_pairs<const POT: bool>(
    tex: &PreparedTexture<'_>,
    u: __m128,
    v: __m128,
) -> (__m128i, __m128i, __m128i, __m128i) {
    debug_assert!(tex.in_bounds);
    let half = _mm_set1_ps(0.5);
    let x = _mm_sub_ps(_mm_mul_ps(u, _mm_set1_ps(tex.width_f)), half);
    let y = _mm_sub_ps(_mm_mul_ps(v, _mm_set1_ps(tex.height_f)), half);
    let (x0, fx_i32) = bilinear_floor_frac(x);
    let (y0, fy_i32) = bilinear_floor_frac(y);
    let fx8 = _mm_packs_epi32(fx_i32, fx_i32);
    let fy16 = _mm_packs_epi32(fy_i32, fy_i32);
    let ify16 = _mm_sub_epi16(_mm_set1_epi16(256), fy16);
    let fy_pair = _mm_unpacklo_epi16(ify16, fy16);

    let (left8, right8) = if POT {
        let mask_x_v = _mm_set1_epi32(tex.pot_x_mask);
        let left_v = _mm_and_si128(x0, mask_x_v);
        let at_seam = _mm_cmpeq_epi32(left_v, mask_x_v);
        if _mm_movemask_ps(_mm_castsi128_ps(at_seam)) == 0 {
            let mask_y_v = _mm_set1_epi32(tex.pot_y_mask);
            let upper_v = _mm_and_si128(y0, mask_y_v);
            let lower_v = _mm_and_si128(_mm_add_epi32(y0, _mm_set1_epi32(1)), mask_y_v);
            let left = extract_u32x4_as_usize(left_v);
            let upper = extract_u32x4_as_usize(upper_v);
            let lower = extract_u32x4_as_usize(lower_v);
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
        let one_i = _mm_set1_epi32(1);
        let (left, upper) = resolve_xy4::<false>(tex, x0, y0);
        let (right, lower) =
            resolve_xy4::<false>(tex, _mm_add_epi32(x0, one_i), _mm_add_epi32(y0, one_i));
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
    (left8, right8, fx8, fy_pair)
}

/// Algebraically bit-exact separable 16-bit integer bilinear interpolation for
/// channels in `0..=63` and weights `fx, fy` in `0..=256`:
/// `top = (c00 << 8) + (c10 - c00) * fx`, `bot = (c01 << 8) + (c11 - c01) * fx`,
/// `res = top * (256 - fy) + bot * fy == (top << 8) + (bot - top) * fy`.
#[inline(always)]
unsafe fn bilinear_channel_s16(
    c_left: __m128i,
    c_right: __m128i,
    fx8: __m128i,
    fy_pair: __m128i,
) -> __m128 {
    let diff = _mm_sub_epi16(c_right, c_left);
    let tb = _mm_add_epi16(_mm_slli_epi16::<8>(c_left), _mm_mullo_epi16(diff, fx8));
    let bot = _mm_srli_si128::<8>(tb);
    let tb_interleaved = _mm_unpacklo_epi16(tb, bot);
    let res = _mm_madd_epi16(tb_interleaved, fy_pair);
    _mm_cvtepi32_ps(res)
}

trait TexSample4 {
    type Scalar: TexSample;
    /// True when `sample4` always returns finite RGB values strictly in `[0.0, 1.0]`
    /// for all UV inputs (including `NaN` / `±inf`), permitting unclamped RGB565 pack.
    const UNIT_RANGE: bool;
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
    const UNIT_RANGE: bool = true;
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
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let (left8, right8, fx8, fy_pair) = bilinear16_load_pairs::<POT>(tex, u, v);
        let m3f = _mm_set1_epi16(0x3f);
        let m1f = _mm_set1_epi16(0x1f);
        let r = bilinear_channel_s16(
            _mm_srli_epi16::<11>(left8),
            _mm_srli_epi16::<11>(right8),
            fx8,
            fy_pair,
        );
        let g = bilinear_channel_s16(
            _mm_and_si128(_mm_srli_epi16::<5>(left8), m3f),
            _mm_and_si128(_mm_srli_epi16::<5>(right8), m3f),
            fx8,
            fy_pair,
        );
        let b = bilinear_channel_s16(
            _mm_and_si128(left8, m1f),
            _mm_and_si128(right8, m1f),
            fx8,
            fy_pair,
        );
        let inv31 = _mm_set1_ps(INV_31_Q16);
        (
            _mm_mul_ps(r, inv31),
            _mm_mul_ps(g, _mm_set1_ps(INV_63_Q16)),
            _mm_mul_ps(b, inv31),
            _mm_set1_ps(1.0),
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
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let (left8, right8, fx8, fy_pair) = bilinear16_load_pairs::<POT>(tex, u, v);
        let m1f = _mm_set1_epi16(0x1f);
        let r = bilinear_channel_s16(
            _mm_and_si128(_mm_srli_epi16::<10>(left8), m1f),
            _mm_and_si128(_mm_srli_epi16::<10>(right8), m1f),
            fx8,
            fy_pair,
        );
        let g = bilinear_channel_s16(
            _mm_and_si128(_mm_srli_epi16::<5>(left8), m1f),
            _mm_and_si128(_mm_srli_epi16::<5>(right8), m1f),
            fx8,
            fy_pair,
        );
        let b = bilinear_channel_s16(
            _mm_and_si128(left8, m1f),
            _mm_and_si128(right8, m1f),
            fx8,
            fy_pair,
        );
        let a = bilinear_channel_s16(
            _mm_srli_epi16::<15>(left8),
            _mm_srli_epi16::<15>(right8),
            fx8,
            fy_pair,
        );
        let inv31 = _mm_set1_ps(INV_31_Q16);
        (
            _mm_mul_ps(r, inv31),
            _mm_mul_ps(g, inv31),
            _mm_mul_ps(b, inv31),
            _mm_mul_ps(a, _mm_set1_ps(INV_65536)),
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
    const UNIT_RANGE: bool = true;
    #[inline(always)]
    unsafe fn sample4(
        _plan: &DrawPlan<'_>,
        tex: &PreparedTexture<'_>,
        u: __m128,
        v: __m128,
    ) -> (__m128, __m128, __m128, __m128) {
        let (left8, right8, fx8, fy_pair) = bilinear16_load_pairs::<POT>(tex, u, v);
        let mf = _mm_set1_epi16(0xf);
        let r = bilinear_channel_s16(
            _mm_and_si128(_mm_srli_epi16::<8>(left8), mf),
            _mm_and_si128(_mm_srli_epi16::<8>(right8), mf),
            fx8,
            fy_pair,
        );
        let g = bilinear_channel_s16(
            _mm_and_si128(_mm_srli_epi16::<4>(left8), mf),
            _mm_and_si128(_mm_srli_epi16::<4>(right8), mf),
            fx8,
            fy_pair,
        );
        let b = bilinear_channel_s16(
            _mm_and_si128(left8, mf),
            _mm_and_si128(right8, mf),
            fx8,
            fy_pair,
        );
        let a = bilinear_channel_s16(
            _mm_srli_epi16::<12>(left8),
            _mm_srli_epi16::<12>(right8),
            fx8,
            fy_pair,
        );
        let inv15 = _mm_set1_ps(INV_15_Q16);
        (
            _mm_mul_ps(r, inv15),
            _mm_mul_ps(g, inv15),
            _mm_mul_ps(b, inv15),
            _mm_mul_ps(a, inv15),
        )
    }
}

struct NearestPaletted8<const POT: bool>;
impl<const POT: bool> TexSample4 for NearestPaletted8<POT> {
    type Scalar = rust::NearestPaletted8<POT>;
    const UNIT_RANGE: bool = true;
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
    const UNIT_RANGE: bool = false;
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
        span_untextured_flat_opaque_sse2::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
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
            let zv = _mm_set_ps(z3, z2, z1, z0);
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            let z_bits = _mm_movemask_ps(z_mask);
            if z_bits != 0 {
                if z_bits == 0xf {
                    if depth_write {
                        _mm_storeu_ps(d_ptr.add(i), zv);
                    }
                    store_u16x4(p_ptr.add(i * 2), packed4);
                } else {
                    if depth_write {
                        _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
                    }
                    let old_px = load_u16x4(p_ptr.add(i * 2));
                    let z_mask_i = _mm_castps_si128(z_mask);
                    let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
                    store_u16x4(p_ptr.add(i * 2), select_si128(mask16, packed4, old_px));
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
        span_untextured_gouraud_opaque_sse2::<Z_TEST, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
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
        let (zv, dv, z_mask, z_bits) = if Z_TEST {
            let zv = _mm_movelh_ps(t0, t2);
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            let z_bits = _mm_movemask_ps(z_mask);
            if z_bits == 0 {
                i += 4;
                continue;
            }
            (zv, dv, z_mask, z_bits)
        } else {
            (p0, p0, _mm_castsi128_ps(_mm_set1_epi32(-1)), 0xf)
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
            if z_bits == 0xf {
                if depth_write {
                    _mm_storeu_ps(d_ptr.add(i), zv);
                }
                store_u16x4(p_ptr.add(i * 2), new_px);
            } else {
                if depth_write {
                    _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
                }
                let old_px = load_u16x4(p_ptr.add(i * 2));
                let z_mask_i = _mm_castps_si128(z_mask);
                let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
                store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
            }
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }
    if n4 == n {
        return;
    }
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
        span_untextured_alpha_sse2::<Z_LE, ALPHA_GT>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
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
        if !ALPHA_GT
            && !depth_write
            && sa == 0.0
            && flat.0[0].is_finite()
            && flat.0[1].is_finite()
            && flat.0[2].is_finite()
        {
            return;
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
                let zv = _mm_set_ps(z3, z2, z1, z0);
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
        if n4 == n {
            return;
        }
        if Z_LE {
            value[0] = z_val;
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
            if !ALPHA_GT && !depth_write && all_zero_f32(sa) && all_finite3(r, g, b) {
                i += 4;
                continue;
            }
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
        if n4 == n {
            return;
        }
        _mm_storeu_ps(value.as_mut_ptr(), s0);
        _mm_storeu_ps(value.as_mut_ptr().add(4), s1);
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
        span_textured_copy_opaque_sse2::<S, Z_TEST>(plan, depth, pixels, value, step, flat, states);
    }
}

#[inline(never)]
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
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
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
            let z_bits = _mm_movemask_ps(z_mask);
            if z_bits != 0 {
                let wv = _mm_movehl_ps(t2, t0);
                let t1 = _mm_unpackhi_ps(p0, p1);
                let t3 = _mm_unpackhi_ps(p2, p3);
                let uv = _mm_movelh_ps(t1, t3);
                let vv = _mm_movehl_ps(t3, t1);
                let near = persp4(wv);
                let (tr, tg, tb, _) =
                    S::sample4(plan, tex, _mm_mul_ps(uv, near), _mm_mul_ps(vv, near));
                let new_px = if S::UNIT_RANGE {
                    pack_rgb565_4_unclamped(tr, tg, tb)
                } else {
                    pack_rgb565_4(tr, tg, tb)
                };
                if z_bits == 0xf {
                    if depth_write {
                        _mm_storeu_ps(d_ptr.add(i), zv);
                    }
                    store_u16x4(p_ptr.add(i * 2), new_px);
                } else {
                    if depth_write {
                        _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
                    }
                    let old_px = load_u16x4(p_ptr.add(i * 2));
                    let z_mask_i = _mm_castps_si128(z_mask);
                    let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
                    store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
                }
            }
        } else {
            let (_, wv, uv, vv) = step4_transpose(&mut s0, d0);
            let near = persp4(wv);
            let (tr, tg, tb, _) = S::sample4(plan, tex, _mm_mul_ps(uv, near), _mm_mul_ps(vv, near));
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
        span_textured_modulate_opaque_sse2::<S, Z_TEST, FLAT, SPECULAR, FOG>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
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
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let tex = plan.texture.as_ref().unwrap();
    let depth_write = states.depth_write;
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
        let (dv, z_mask, z_bits) = if Z_TEST {
            let dv = _mm_loadu_ps(d_ptr.add(i));
            let z_mask = _mm_cmple_ps(zv, dv);
            let z_bits = _mm_movemask_ps(z_mask);
            if z_bits == 0 {
                i += 4;
                continue;
            }
            (dv, z_mask, z_bits)
        } else {
            (zv, _mm_castsi128_ps(_mm_set1_epi32(-1)), 0xf)
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
            if z_bits == 0xf {
                if depth_write {
                    _mm_storeu_ps(d_ptr.add(i), zv);
                }
                store_u16x4(p_ptr.add(i * 2), new_px);
            } else {
                if depth_write {
                    _mm_storeu_ps(d_ptr.add(i), select_ps(z_mask, zv, dv));
                }
                let old_px = load_u16x4(p_ptr.add(i * 2));
                let z_mask_i = _mm_castps_si128(z_mask);
                let mask16 = _mm_packs_epi32(z_mask_i, z_mask_i);
                store_u16x4(p_ptr.add(i * 2), select_si128(mask16, new_px, old_px));
            }
        } else {
            store_u16x4(p_ptr.add(i * 2), new_px);
        }
        i += 4;
    }

    if n4 == n {
        return;
    }
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
        span_textured_modulate_blend_sse2::<S, Z_LE, ALPHA_GT, BLEND, FLAT, SPECULAR>(
            plan, depth, pixels, value, step, flat, states,
        );
    }
}

#[inline(never)]
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
    let n = depth.len().min(pixels.len() / 2);
    let n4 = n & !3;
    let tex = plan.texture.as_ref().unwrap();
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
    let can_skip_transparent =
        !ALPHA_GT && !depth_write && (BLEND == BLEND_ALPHA || BLEND == BLEND_ADDITIVE_ALPHA);
    let flat_rgb_always_finite = FLAT
        && S::UNIT_RANGE
        && flat.0[0].is_finite()
        && flat.0[1].is_finite()
        && flat.0[2].is_finite()
        && (!SPECULAR || (flat.1[0].is_finite() && flat.1[1].is_finite() && flat.1[2].is_finite()));

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

        if can_skip_transparent && flat_rgb_always_finite && all_zero_f32(sa) {
            i += 4;
            continue;
        }

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

    if n4 == n {
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::{Blend, Compare, PixelFormat, Texture};

    #[allow(clippy::too_many_arguments)]
    fn assert_sse2_matches_rust(
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
        let sse2_fn = resolve_span_fn(plan.kernel);

        let mut dr = init_depth.to_vec();
        let mut ds = init_depth.to_vec();
        let mut pr = vec![0x5a; byte_offset + init_pixels.len() + 3];
        let mut ps = vec![0x5a; byte_offset + init_pixels.len() + 3];
        pr[byte_offset..byte_offset + init_pixels.len()].copy_from_slice(init_pixels);
        ps[byte_offset..byte_offset + init_pixels.len()].copy_from_slice(init_pixels);

        let mut vr = value;
        let mut vs = value;
        rust_fn(
            plan,
            &mut dr,
            &mut pr[byte_offset..byte_offset + init_pixels.len()],
            &mut vr,
            &step,
            flat,
            states,
        );
        sse2_fn(
            plan,
            &mut ds,
            &mut ps[byte_offset..byte_offset + init_pixels.len()],
            &mut vs,
            &step,
            flat,
            states,
        );
        assert_eq!(pr, ps, "pixel mismatch ({label})");
        let br: Vec<u32> = dr.iter().map(|d| d.to_bits()).collect();
        let bs: Vec<u32> = ds.iter().map(|d| d.to_bits()).collect();
        assert_eq!(br, bs, "depth mismatch ({label})");
    }

    #[test]
    fn separable_bilinear_s16_and_unclamped_pack_match_oracle_bit_for_bit() {
        #[target_feature(enable = "sse2")]
        unsafe fn run() {
            let fys: [i16; 5] = [0, 1, 128, 255, 256];
            for c00 in 0i16..=63 {
                let c01 = 63 - c00;
                for c10 in 0i16..=63 {
                    let c11 = 63 - c10;
                    let mut cl = _mm_set1_epi16(c00);
                    cl = _mm_insert_epi16::<4>(cl, i32::from(c01));
                    cl = _mm_insert_epi16::<5>(cl, i32::from(c01));
                    cl = _mm_insert_epi16::<6>(cl, i32::from(c01));
                    cl = _mm_insert_epi16::<7>(cl, i32::from(c01));
                    let mut cr = _mm_set1_epi16(c10);
                    cr = _mm_insert_epi16::<4>(cr, i32::from(c11));
                    cr = _mm_insert_epi16::<5>(cr, i32::from(c11));
                    cr = _mm_insert_epi16::<6>(cr, i32::from(c11));
                    cr = _mm_insert_epi16::<7>(cr, i32::from(c11));
                    for fx_base in (0i16..=256).step_by(4) {
                        let fxs = [
                            fx_base,
                            (fx_base + 1).min(256),
                            (fx_base + 2).min(256),
                            (fx_base + 3).min(256),
                        ];
                        let fx_i32 = _mm_loadu_si128(
                            [
                                i32::from(fxs[0]),
                                i32::from(fxs[1]),
                                i32::from(fxs[2]),
                                i32::from(fxs[3]),
                            ]
                            .as_ptr()
                            .cast(),
                        );
                        let fx8 = _mm_packs_epi32(fx_i32, fx_i32);
                        for &fy in &fys {
                            let fy16 = _mm_set1_epi16(fy);
                            let ify16 = _mm_sub_epi16(_mm_set1_epi16(256), fy16);
                            let fy_pair = _mm_unpacklo_epi16(ify16, fy16);
                            let got = bilinear_channel_s16(cl, cr, fx8, fy_pair);
                            let mut lanes = [0.0f32; 4];
                            _mm_storeu_ps(lanes.as_mut_ptr(), got);
                            for k in 0..4 {
                                let fx = fxs[k] as u32;
                                let fyu = fy as u32;
                                let expected = ((c00 as u32) * (256 - fx) * (256 - fyu)
                                    + (c10 as u32) * fx * (256 - fyu)
                                    + (c01 as u32) * (256 - fx) * fyu
                                    + (c11 as u32) * fx * fyu)
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

            let cs: &[i16] = &[0, 1, 15, 16, 31, 32, 62, 63];
            let ws: &[i16] = &[0, 1, 127, 128, 255, 256];
            for &c00 in cs {
                for &c10 in cs {
                    for &c01 in cs {
                        for &c11 in cs {
                            let cl = _mm_insert_epi16::<4>(_mm_set1_epi16(c00), i32::from(c01));
                            let cr = _mm_insert_epi16::<4>(_mm_set1_epi16(c10), i32::from(c11));
                            for &fx in ws {
                                let fx8 = _mm_set1_epi16(fx);
                                for &fy in ws {
                                    let fy16 = _mm_set1_epi16(fy);
                                    let ify16 = _mm_sub_epi16(_mm_set1_epi16(256), fy16);
                                    let fy_pair = _mm_unpacklo_epi16(ify16, fy16);
                                    let got = bilinear_channel_s16(cl, cr, fx8, fy_pair);
                                    let mut lanes = [0.0f32; 4];
                                    _mm_storeu_ps(lanes.as_mut_ptr(), got);
                                    let fxu = fx as u32;
                                    let fyu = fy as u32;
                                    let expected = ((c00 as u32) * (256 - fxu) * (256 - fyu)
                                        + (c10 as u32) * fxu * (256 - fyu)
                                        + (c01 as u32) * (256 - fxu) * fyu
                                        + (c11 as u32) * fxu * fyu)
                                        as f32;
                                    assert_eq!(lanes[0].to_bits(), expected.to_bits());
                                }
                            }
                        }
                    }
                }
            }

            let max_g = 63u32 * 65_536;
            for g_base in (0..=max_g).step_by(4) {
                let gs = [
                    g_base,
                    (g_base + 1).min(max_g),
                    (g_base + 2).min(max_g),
                    (g_base + 3).min(max_g),
                ];
                let gf = _mm_set_ps(
                    gs[3] as f32 * INV_63_Q16,
                    gs[2] as f32 * INV_63_Q16,
                    gs[1] as f32 * INV_63_Q16,
                    gs[0] as f32 * INV_63_Q16,
                );
                let rf = _mm_set_ps(
                    (gs[3] & 31) as f32 * INV_31,
                    (gs[2] & 31) as f32 * INV_31,
                    (gs[1] & 31) as f32 * INV_31,
                    (gs[0] & 31) as f32 * INV_31,
                );
                let bf = _mm_set_ps(
                    ((gs[3] >> 5) & 31) as f32 * INV_31,
                    ((gs[2] >> 5) & 31) as f32 * INV_31,
                    ((gs[1] >> 5) & 31) as f32 * INV_31,
                    ((gs[0] >> 5) & 31) as f32 * INV_31,
                );
                let unclamped = pack_rgb565_4_unclamped(rf, gf, bf);
                let clamped = pack_rgb565_4(rf, gf, bf);
                assert_eq!(
                    _mm_cvtsi128_si64(unclamped),
                    _mm_cvtsi128_si64(clamped),
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
        #[target_feature(enable = "sse2")]
        unsafe fn check_sampler<S: TexSample4>(
            plan: &DrawPlan<'_>,
            tex: &PreparedTexture<'_>,
            us: [f32; 4],
            vs: [f32; 4],
            label: &str,
        ) {
            let uv = _mm_set_ps(us[3], us[2], us[1], us[0]);
            let vv = _mm_set_ps(vs[3], vs[2], vs[1], vs[0]);
            let (r, g, b, a) = S::sample4(plan, tex, uv, vv);
            let mut rs = [0.0f32; 4];
            let mut gs = [0.0f32; 4];
            let mut bs = [0.0f32; 4];
            let mut as_ = [0.0f32; 4];
            _mm_storeu_ps(rs.as_mut_ptr(), r);
            _mm_storeu_ps(gs.as_mut_ptr(), g);
            _mm_storeu_ps(bs.as_mut_ptr(), b);
            _mm_storeu_ps(as_.as_mut_ptr(), a);
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
                        assert_sse2_matches_rust(
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
                        assert_sse2_matches_rust(
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
                for len in 0..=65 {
                    let depth = vec![1.0f32; len];
                    let pixels: Vec<u8> = (0..len * 2).map(|i| (i * 23 + 9) as u8).collect();
                    assert_sse2_matches_rust(
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
