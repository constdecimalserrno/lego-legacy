//! Canonical pure-Rust presentation pixel conversions (oracle and universal fallback).

/// Copy a 32-bit BGRA surface into a tightly packed destination buffer.
#[inline]
pub(crate) fn copy_bgra32(
    src: &[u8],
    src_pitch: usize,
    dst: &mut [u8],
    width: usize,
    height: usize,
) {
    if width == 0 || height == 0 {
        return;
    }
    let row_bytes = width * 4;
    if src_pitch == row_bytes {
        let total = row_bytes * height;
        dst[..total].copy_from_slice(&src[..total]);
        return;
    }
    for y in 0..height {
        let from = y * src_pitch;
        let to = y * row_bytes;
        dst[to..to + row_bytes].copy_from_slice(&src[from..from + row_bytes]);
    }
}

/// Convert one row of 8-bit paletted indices into BGRA32 using a pre-swizzled LUT.
#[inline]
pub(crate) fn paletted8_row(src: &[u8], dst: &mut [u8], lut: &[[u8; 4]; 256]) {
    let (dst_px, _) = dst.as_chunks_mut::<4>();
    for (out, &index) in dst_px.iter_mut().zip(src) {
        *out = lut[usize::from(index)];
    }
}

/// Convert one row of little-endian 16-bit RGB565 pixels into BGRA32.
///
/// Widens 5-bit and 6-bit channels by repeating the top bits so full-scale
/// white (`0xffff`) maps to `0xff` rather than `0xf8`/`0xfc`.
#[inline]
pub(crate) fn rgb565_row(src: &[u8], dst: &mut [u8]) {
    let (src_px, _) = src.as_chunks::<2>();
    let (dst_px, _) = dst.as_chunks_mut::<4>();
    for (out, &[lo, hi]) in dst_px.iter_mut().zip(src_px) {
        let pixel = u16::from_le_bytes([lo, hi]);
        let r = ((pixel >> 11) & 0x1f) as u8;
        let g = ((pixel >> 5) & 0x3f) as u8;
        let b = (pixel & 0x1f) as u8;
        *out = [
            (b << 3) | (b >> 2),
            (g << 2) | (g >> 4),
            (r << 3) | (r >> 2),
            0xff,
        ];
    }
}
