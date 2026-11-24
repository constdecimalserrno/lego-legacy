//! Baseline `aarch64` NEON RGB565 -> BGRA32 presentation row kernel (ADR-0009, #271).
//!
//! Processes 16 pixels (one 64-byte cache line of BGRA32 output) per iteration
//! using full-width 128-bit NEON (`vld1q_u8`, `vsriq_n_u8`, `vst4q_u8`), then
//! an optional 8-pixel pass (`vst4_u8`), delegating `0..7` tail pixels to
//! [`super::rust::rgb565_row`].

use core::arch::aarch64::{
    uint8x16x4_t, uint8x8x4_t, vdup_n_u8, vdupq_n_u8, vld1q_u8, vmovn_high_u16, vmovn_u16,
    vreinterpretq_u16_u8, vshl_n_u8, vshlq_n_u8, vshrn_high_n_u16, vshrn_n_u16, vsri_n_u8,
    vsriq_n_u8, vst4_u8, vst4q_u8,
};

use super::rust;

#[inline]
pub(crate) fn rgb565_row(src: &[u8], dst: &mut [u8]) {
    let pixels = (src.len() / 2).min(dst.len() / 4);
    let vec_pixels = pixels & !7;
    if vec_pixels > 0 {
        // SAFETY: Baseline NEON is always enabled on `aarch64` desktop targets,
        // `vec_pixels * 2 <= src.len()`, and `vec_pixels * 4 <= dst.len()`.
        unsafe {
            rgb565_row_neon(src.as_ptr(), dst.as_mut_ptr(), vec_pixels);
        }
    }
    if vec_pixels < pixels {
        rust::rgb565_row(&src[vec_pixels * 2..], &mut dst[vec_pixels * 4..]);
    }
}

#[target_feature(enable = "neon")]
unsafe fn rgb565_row_neon(mut src: *const u8, mut dst: *mut u8, pixels: usize) {
    debug_assert_eq!(pixels % 8, 0);
    let vec16 = pixels & !15;
    if vec16 > 0 {
        let alpha16 = vdupq_n_u8(0xff);
        let end16 = src.add(vec16 * 2);
        while src < end16 {
            // `vld1q_u8` requires only 1-byte alignment, so odd byte offsets are sound.
            let px0 = vreinterpretq_u16_u8(vld1q_u8(src));
            let px1 = vreinterpretq_u16_u8(vld1q_u8(src.add(16)));

            let lo = vmovn_high_u16(vmovn_u16(px0), px1);
            let b_hi = vshlq_n_u8::<3>(lo);
            let b = vsriq_n_u8::<5>(b_hi, b_hi);

            let g_hi = vshrn_high_n_u16::<3>(vshrn_n_u16::<3>(px0), px1);
            let g = vsriq_n_u8::<6>(g_hi, g_hi);

            let r_hi = vshrn_high_n_u16::<8>(vshrn_n_u16::<8>(px0), px1);
            let r = vsriq_n_u8::<5>(r_hi, r_hi);

            vst4q_u8(dst, uint8x16x4_t(b, g, r, alpha16));
            src = src.add(32);
            dst = dst.add(64);
        }
    }
    if vec16 < pixels {
        let alpha8 = vdup_n_u8(0xff);
        let px = vreinterpretq_u16_u8(vld1q_u8(src));
        let b_hi = vshl_n_u8::<3>(vmovn_u16(px));
        let b = vsri_n_u8::<5>(b_hi, b_hi);
        let g_hi = vshrn_n_u16::<3>(px);
        let g = vsri_n_u8::<6>(g_hi, g_hi);
        let r_hi = vshrn_n_u16::<8>(px);
        let r = vsri_n_u8::<5>(r_hi, r_hi);
        vst4_u8(dst, uint8x8x4_t(b, g, r, alpha8));
    }
}
