//! Baseline `x86_64` SSE2 RGB565 -> BGRA32 presentation row kernel (ADR-0009, #271).
//!
//! Processes 8 pixels per iteration using baseline SSE2 only (zero runtime dispatch,
//! unaligned 128-bit loads/stores) and delegates `0..7` tail pixels to
//! [`super::rust::rgb565_row`].

use core::arch::x86_64::{
    __m128i, _mm_and_si128, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi16, _mm_slli_epi16,
    _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi16, _mm_unpacklo_epi16,
};

use super::rust;

#[inline]
pub(crate) fn rgb565_row(src: &[u8], dst: &mut [u8]) {
    let pixels = (src.len() / 2).min(dst.len() / 4);
    let vec_pixels = pixels & !7;
    if vec_pixels > 0 {
        // SAFETY: Baseline SSE2 is always enabled on `x86_64`,
        // `vec_pixels * 2 <= src.len()`, and `vec_pixels * 4 <= dst.len()`.
        unsafe {
            rgb565_row_sse2(src.as_ptr(), dst.as_mut_ptr(), vec_pixels);
        }
    }
    if vec_pixels < pixels {
        rust::rgb565_row(&src[vec_pixels * 2..], &mut dst[vec_pixels * 4..]);
    }
}

#[target_feature(enable = "sse2")]
unsafe fn rgb565_row_sse2(mut src: *const u8, mut dst: *mut u8, pixels: usize) {
    debug_assert_eq!(pixels % 8, 0);
    let mask_00f8 = _mm_set1_epi16(0x00f8_u16.cast_signed());
    let mask_fc00 = _mm_set1_epi16(0xfc00_u16.cast_signed());
    let mask_0300 = _mm_set1_epi16(0x0300_u16.cast_signed());
    let alpha_hi = _mm_set1_epi16(0xff00_u16.cast_signed());
    let end = src.add(pixels * 2);
    while src < end {
        let px = _mm_loadu_si128(src.cast::<__m128i>());

        let b_hi = _mm_and_si128(_mm_slli_epi16::<3>(px), mask_00f8);
        let b = _mm_or_si128(b_hi, _mm_srli_epi16::<5>(b_hi));

        let g_hi = _mm_and_si128(_mm_slli_epi16::<5>(px), mask_fc00);
        let g_lo = _mm_and_si128(_mm_srli_epi16::<1>(px), mask_0300);
        let bg = _mm_or_si128(b, _mm_or_si128(g_hi, g_lo));

        let r_hi = _mm_and_si128(_mm_srli_epi16::<8>(px), mask_00f8);
        let r_lo = _mm_srli_epi16::<13>(px);
        let ra = _mm_or_si128(_mm_or_si128(r_hi, r_lo), alpha_hi);

        let out_lo = _mm_unpacklo_epi16(bg, ra);
        let out_hi = _mm_unpackhi_epi16(bg, ra);
        _mm_storeu_si128(dst.cast::<__m128i>(), out_lo);
        _mm_storeu_si128(dst.add(16).cast::<__m128i>(), out_hi);

        src = src.add(16);
        dst = dst.add(32);
    }
}
