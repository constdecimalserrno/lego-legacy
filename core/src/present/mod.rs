//! Presentation surface conversion to tightly packed host BGRA32 (ADR-0009, #271).
//!
//! Co-located per-subsystem seam:
//! - [`rust`]: canonical pure-Rust oracle and universal fallback (32-bit copy,
//!   8-bit paletted LUT conversion, and 16-bit RGB565 conversion);
//! - [`aarch64`]: baseline NEON RGB565 -> BGRA32 row kernel (8 pixels/iter);
//! - [`x86_64`]: baseline SSE2 RGB565 -> BGRA32 row kernel (8 pixels/iter).

#[cfg(all(target_arch = "aarch64", target_endian = "little"))]
mod aarch64;
mod rust;
#[cfg(target_arch = "x86_64")]
mod x86_64;

use crate::guest::{Arena, GuestMem};

/// Copy a 32-bit BGRA surface into tightly packed BGRA32 output.
pub fn copy_bgra32(src: &[u8], src_pitch: usize, dst: &mut [u8], width: usize, height: usize) {
    rust::copy_bgra32(src, src_pitch, dst, width, height);
}

/// Build a 256-entry pre-swizzled BGRA stack LUT from a guest `PALETTEENTRY`
/// table once per flip.
///
/// A null palette address (`0`) maps every entry to opaque black `[0, 0, 0, 0xff]`.
/// When the full 1,024-byte table is in bounds, all 256 entries are swizzled
/// from a single `arena.slice(palette, 1024)`; otherwise each entry falls back
/// to `arena.read_u32` (defaulting to `0` on fault), matching per-pixel reads
/// bit-for-bit.
#[must_use]
pub fn build_palette_lut(palette: u32, arena: &Arena) -> [[u8; 4]; 256] {
    let mut lut = [[0u8, 0, 0, 0xff]; 256];
    if palette == 0 {
        return lut;
    }
    if let Ok(bytes) = arena.slice(palette, 1024) {
        let (entries, _) = bytes.as_chunks::<4>();
        for (slot, &[r, g, b, _]) in lut.iter_mut().zip(entries) {
            *slot = [b, g, r, 0xff];
        }
        return lut;
    }
    for (i, slot) in lut.iter_mut().enumerate() {
        let entry = palette
            .checked_add((i as u32) * 4)
            .and_then(|addr| arena.read_u32(addr).ok())
            .unwrap_or(0);
        let [r, g, b, _] = entry.to_le_bytes();
        *slot = [b, g, r, 0xff];
    }
    lut
}

/// Convert an 8-bit paletted surface into tightly packed BGRA32 output.
pub fn paletted8_to_bgra(
    src: &[u8],
    src_pitch: usize,
    dst: &mut [u8],
    width: usize,
    height: usize,
    palette: u32,
    arena: &Arena,
) {
    if width == 0 || height == 0 {
        return;
    }
    let lut = build_palette_lut(palette, arena);
    if src_pitch == width {
        let pixels = width * height;
        rust::paletted8_row(&src[..pixels], &mut dst[..pixels * 4], &lut);
        return;
    }
    let dst_stride = width * 4;
    for y in 0..height {
        let src_row = &src[y * src_pitch..y * src_pitch + width];
        let dst_row = &mut dst[y * dst_stride..(y + 1) * dst_stride];
        rust::paletted8_row(src_row, dst_row, &lut);
    }
}

/// Convert a 16-bit RGB565 surface into tightly packed BGRA32 output.
pub fn rgb565_to_bgra(src: &[u8], src_pitch: usize, dst: &mut [u8], width: usize, height: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let row_fn: fn(&[u8], &mut [u8]) = if crate::arch::use_rust() {
        rust::rgb565_row
    } else {
        native_rgb565_row
    };
    let src_row_bytes = width * 2;
    let dst_row_bytes = width * 4;
    if src_pitch == src_row_bytes {
        row_fn(
            &src[..src_row_bytes * height],
            &mut dst[..dst_row_bytes * height],
        );
        return;
    }
    for y in 0..height {
        let src_row = &src[y * src_pitch..y * src_pitch + src_row_bytes];
        let dst_row = &mut dst[y * dst_row_bytes..(y + 1) * dst_row_bytes];
        row_fn(src_row, dst_row);
    }
}

#[inline]
fn native_rgb565_row(src: &[u8], dst: &mut [u8]) {
    #[cfg(all(target_arch = "aarch64", target_endian = "little"))]
    {
        aarch64::rgb565_row(src, dst);
    }
    #[cfg(target_arch = "x86_64")]
    {
        x86_64::rgb565_row(src, dst);
    }
    #[cfg(not(any(
        all(target_arch = "aarch64", target_endian = "little"),
        target_arch = "x86_64"
    )))]
    {
        rust::rgb565_row(src, dst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::headless::Headless;

    fn scalar_rgb565_reference(src: &[u8], dst: &mut [u8]) {
        let pixels = (src.len() / 2).min(dst.len() / 4);
        for x in 0..pixels {
            let at = x * 2;
            let pixel = u16::from_le_bytes([src[at], src[at + 1]]);
            let r = ((pixel >> 11) & 0x1f) as u8;
            let g = ((pixel >> 5) & 0x3f) as u8;
            let b = (pixel & 0x1f) as u8;
            let out = x * 4;
            dst[out] = (b << 3) | (b >> 2);
            dst[out + 1] = (g << 2) | (g >> 4);
            dst[out + 2] = (r << 3) | (r >> 2);
            dst[out + 3] = 0xff;
        }
    }

    #[test]
    fn rgb565_exhaustive_all_65536_values_match_oracle_and_native() {
        let mut src = Vec::with_capacity(65_536 * 2);
        for v in 0u16..=u16::MAX {
            src.extend_from_slice(&v.to_le_bytes());
        }
        let mut expected = vec![0xaa; 65_536 * 4];
        let mut rust_out = vec![0x55; 65_536 * 4];
        let mut native_out = vec![0x33; 65_536 * 4];

        scalar_rgb565_reference(&src, &mut expected);
        rust::rgb565_row(&src, &mut rust_out);
        native_rgb565_row(&src, &mut native_out);

        assert_eq!(rust_out, expected, "pure-Rust oracle drifted from formula");
        assert_eq!(
            native_out, rust_out,
            "native SIMD kernel drifted from pure-Rust oracle across 65,536 RGB565 values"
        );

        // Pin key anchor values (black, white, pure R/G/B).
        assert_eq!(&native_out[0..4], &[0x00, 0x00, 0x00, 0xff]);
        assert_eq!(
            &native_out[0xffff * 4..0xffff * 4 + 4],
            &[0xff, 0xff, 0xff, 0xff]
        );
        assert_eq!(
            &native_out[0xf800 * 4..0xf800 * 4 + 4],
            &[0x00, 0x00, 0xff, 0xff]
        );
        assert_eq!(
            &native_out[0x07e0 * 4..0x07e0 * 4 + 4],
            &[0x00, 0xff, 0x00, 0xff]
        );
        assert_eq!(
            &native_out[0x001f * 4..0x001f * 4 + 4],
            &[0xff, 0x00, 0x00, 0xff]
        );
    }

    #[test]
    fn rgb565_widths_0_through_65_and_odd_offsets_match_oracle() {
        let mut raw_src = [0u8; 65 * 2 + 16];
        for (i, byte) in raw_src.iter_mut().enumerate() {
            *byte = ((i * 73 + 19) & 0xff) as u8;
        }
        const GUARD: u8 = 0xa5;

        for src_off in 0..8usize {
            for dst_off in 0..8usize {
                for width in 0..=65usize {
                    let src_slice = &raw_src[src_off..src_off + width * 2];
                    let mut rust_buf = vec![GUARD; dst_off + width * 4 + 8];
                    let mut native_buf = vec![GUARD; dst_off + width * 4 + 8];

                    rust::rgb565_row(src_slice, &mut rust_buf[dst_off..dst_off + width * 4]);
                    native_rgb565_row(src_slice, &mut native_buf[dst_off..dst_off + width * 4]);

                    assert_eq!(
                        native_buf, rust_buf,
                        "mismatch at src_off={src_off} dst_off={dst_off} width={width}"
                    );
                    assert!(
                        native_buf[..dst_off].iter().all(|&b| b == GUARD),
                        "leading guard overwritten"
                    );
                    assert!(
                        native_buf[dst_off + width * 4..]
                            .iter()
                            .all(|&b| b == GUARD),
                        "trailing guard overwritten"
                    );
                }
            }
        }
    }

    #[test]
    fn rgb565_to_bgra_handles_dense_padded_and_empty_surfaces() {
        let width = 13usize;
        let height = 5usize;
        let pitch = width * 2 + 6;
        let mut src = vec![0u8; pitch * height];
        for y in 0..height {
            for x in 0..width {
                let px = (((y * 37 + x * 257) as u16) ^ 0x5a5a).to_le_bytes();
                src[y * pitch + x * 2] = px[0];
                src[y * pitch + x * 2 + 1] = px[1];
            }
        }
        let mut actual = vec![0u8; width * height * 4];
        rgb565_to_bgra(&src, pitch, &mut actual, width, height);

        let mut expected = vec![0u8; width * height * 4];
        for y in 0..height {
            scalar_rgb565_reference(
                &src[y * pitch..y * pitch + width * 2],
                &mut expected[y * width * 4..(y + 1) * width * 4],
            );
        }
        assert_eq!(actual, expected);

        // Dense surface path (`pitch == width * 2`).
        let mut dense_src = vec![0u8; width * height * 2];
        for y in 0..height {
            dense_src[y * width * 2..(y + 1) * width * 2]
                .copy_from_slice(&src[y * pitch..y * pitch + width * 2]);
        }
        let mut dense_out = vec![0u8; width * height * 4];
        rgb565_to_bgra(&dense_src, width * 2, &mut dense_out, width, height);
        assert_eq!(dense_out, expected);

        // Zero dimensions do not touch output.
        let mut empty = vec![0x77; 4];
        rgb565_to_bgra(&[], 0, &mut empty, 0, 4);
        rgb565_to_bgra(&[], 0, &mut empty, 4, 0);
        assert_eq!(empty, [0x77; 4]);
    }

    #[test]
    fn paletted8_matches_legacy_scalar_on_full_partial_and_null_palettes() {
        let mut host = Headless::new();
        let mut arena = Arena::new(&mut host, 0x4000).unwrap();

        // Populate a full 256-entry palette at 0x1000.
        let pal_addr = 0x1000u32;
        for i in 0u32..256 {
            let r = (i as u8).wrapping_mul(3);
            let g = (i as u8).wrapping_mul(5).wrapping_add(17);
            let b = (i as u8).wrapping_mul(7).wrapping_add(91);
            let flags = (i & 0xff) as u8;
            arena.write(pal_addr + i * 4, &[r, g, b, flags]).unwrap();
        }

        // Populate a partial palette near the top of the arena so `slice(partial, 1024)` faults.
        let partial_addr = 0x4000 - 400; // only 100 entries fit in bounds
        for i in 0u32..100 {
            let r = (i as u8).wrapping_add(10);
            let g = (i as u8).wrapping_add(20);
            let b = (i as u8).wrapping_add(30);
            arena.write(partial_addr + i * 4, &[r, g, b, 0xab]).unwrap();
        }

        let width = 16usize;
        let height = 16usize;
        let pitch = 20usize;
        let mut src = vec![0u8; pitch * height];
        for y in 0..height {
            for x in 0..width {
                src[y * pitch + x] = (y * 16 + x) as u8;
            }
        }

        for palette in [0u32, pal_addr, partial_addr, u32::MAX - 8] {
            let mut actual = vec![0u8; width * height * 4];
            paletted8_to_bgra(&src, pitch, &mut actual, width, height, palette, &arena);

            let mut expected = vec![0u8; width * height * 4];
            for y in 0..height {
                for x in 0..width {
                    let idx = src[y * pitch + x];
                    let entry = if palette == 0 {
                        0
                    } else {
                        palette
                            .checked_add(u32::from(idx) * 4)
                            .and_then(|a| arena.read_u32(a).ok())
                            .unwrap_or(0)
                    };
                    let [r, g, b, _] = entry.to_le_bytes();
                    let out = (y * width + x) * 4;
                    expected[out] = b;
                    expected[out + 1] = g;
                    expected[out + 2] = r;
                    expected[out + 3] = 0xff;
                }
            }
            assert_eq!(actual, expected, "palette={palette:#x}");
        }

        // Dense 8-bit surface (`pitch == width`).
        let dense_src: Vec<u8> = (0..=255u8).collect();
        let mut dense_out = vec![0u8; 256 * 4];
        paletted8_to_bgra(&dense_src, 16, &mut dense_out, 16, 16, pal_addr, &arena);
        let mut padded_out = vec![0u8; 256 * 4];
        paletted8_to_bgra(&src, pitch, &mut padded_out, 16, 16, pal_addr, &arena);
        assert_eq!(dense_out, padded_out);
    }

    #[test]
    fn copy_bgra32_handles_dense_and_padded_pitches() {
        let width = 3usize;
        let height = 2usize;
        let pitch = 16usize;
        let mut src = vec![0u8; pitch * height];
        for y in 0..height {
            for x in 0..width * 4 {
                src[y * pitch + x] = (y * 32 + x + 1) as u8;
            }
        }
        let mut padded_out = vec![0u8; width * height * 4];
        copy_bgra32(&src, pitch, &mut padded_out, width, height);

        let mut dense_src = vec![0u8; width * height * 4];
        for y in 0..height {
            dense_src[y * width * 4..(y + 1) * width * 4]
                .copy_from_slice(&src[y * pitch..y * pitch + width * 4]);
        }
        let mut dense_out = vec![0u8; width * height * 4];
        copy_bgra32(&dense_src, width * 4, &mut dense_out, width, height);

        assert_eq!(padded_out, dense_src);
        assert_eq!(dense_out, dense_src);
    }
}
