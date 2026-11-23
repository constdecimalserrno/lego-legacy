//! Microsoft Video 1 (`CRAM` / `MSVC` / `WHAM`): 16-bit 4×4 block video
//! decoder, written from its public format description (#152).
//!
//! A frame is divided into 4×4 blocks walked bottom-to-top and left-to-right.
//! Each block is skipped (keeping the previous frame's pixels), filled with
//! one 15-bit RGB555 colour, or painted by a 16-bit mask from two colours
//! across the block or two per 2×2 quadrant (eight in all).

use std::fmt;

/// Why a frame or picture size is not valid 16-bit Microsoft Video 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MS Video 1: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// A decoded 16-bit Microsoft Video 1 frame: top-down 15-bit RGB555 pixels
/// (`0..=0x7fff`), held across frames so skip blocks keep what came before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    width: usize,
    height: usize,
    pixels: Vec<u16>,
}

impl Picture {
    /// Make a black picture of `width` by `height`, each a non-zero multiple
    /// of four.
    ///
    /// # Errors
    /// When either side is zero, not a multiple of four, or overflows memory.
    pub fn new(width: usize, height: usize) -> Result<Self, Error> {
        if width == 0 || height == 0 || !width.is_multiple_of(4) || !height.is_multiple_of(4) {
            return Err(Error(format!(
                "{width}x{height} is not a non-zero multiple of four"
            )));
        }
        // Bound pixel count so both the u16 frame buffer and a 24-bit DIB fit
        // within Rust's allocation limit (`isize::MAX` bytes).
        let count = width
            .checked_mul(height)
            .filter(|c| c.checked_mul(3).is_some_and(|b| b <= isize::MAX as usize))
            .ok_or_else(|| Error(format!("{width}x{height} overflows memory")))?;
        Ok(Self {
            width,
            height,
            pixels: vec![0; count],
        })
    }

    /// Clear every pixel to black, for a seek back to the start of a stream.
    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    /// Decode one 16-bit Video 1 chunk into this picture in place.
    ///
    /// # Errors
    /// When a block is truncated or a skip count is zero or runs past the
    /// last block.
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        let blocks_x = self.width / 4;
        let blocks_y = self.height / 4;
        let total = blocks_x * blocks_y;
        let mut pos = 0;
        let mut blk = 0;
        let read_u16 = |pos: &mut usize| -> Result<u16, Error> {
            let bytes = data
                .get(*pos..*pos + 2)
                .ok_or_else(|| Error("truncated block".to_owned()))?;
            *pos += 2;
            Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
        };

        while blk < total && pos < data.len() {
            let Some(&[a, b]) = data.get(pos..pos + 2) else {
                return Err(Error("truncated block header".to_owned()));
            };
            pos += 2;

            if (0x84..=0x87).contains(&b) {
                let skip = (usize::from(b - 0x84) << 8) | usize::from(a);
                if skip == 0 {
                    return Err(Error(format!("zero skip count at block {blk}")));
                }
                if blk + skip > total {
                    return Err(Error(format!(
                        "skip {skip} from block {blk} past {total} blocks"
                    )));
                }
                blk += skip;
                continue;
            }

            let bx = (blk % blocks_x) * 4;
            let by = (blocks_y - 1 - blk / blocks_x) * 4;

            if b >= 0x80 {
                let color = u16::from_le_bytes([a, b]) & 0x7fff;
                for row in 0..4 {
                    let start = (by + row) * self.width + bx;
                    self.pixels[start..start + 4].fill(color);
                }
                blk += 1;
                continue;
            }

            if a == 0 && b == 0 && pos == data.len() {
                break;
            }

            let mut mask = u16::from_le_bytes([a, b]);
            let c0 = read_u16(&mut pos)?;
            let c1 = read_u16(&mut pos)?;
            let (p0, p1) = (c0 & 0x7fff, c1 & 0x7fff);
            let mut colors = [p0, p1, p0, p1, p0, p1, p0, p1];
            if c0 & 0x8000 != 0 {
                for slot in &mut colors[2..] {
                    *slot = read_u16(&mut pos)? & 0x7fff;
                }
            }
            for row in (0..4).rev() {
                let base = (by + row) * self.width + bx;
                let q_row = usize::from(row < 2) * 2;
                for col in 0..4 {
                    let q = q_row + usize::from(col >= 2);
                    self.pixels[base + col] = colors[q * 2 + usize::from(mask & 1 == 0)];
                    mask >>= 1;
                }
            }
            blk += 1;
        }
        Ok(())
    }

    /// This picture as the bits of a bottom-up `BI_RGB` DIB, rows padded to
    /// four bytes: 15-bit RGB555 little-endian at 16 bits, or BGR at 24 bits
    /// with each 5-bit channel shifted left three bits (`c << 3`), as Windows'
    /// `msvidc32` expands 16-bit Video 1 to 24 bits.
    ///
    /// # Panics
    /// When `bits` is neither 16 nor 24.
    #[must_use]
    pub fn to_dib_bits(&self, bits: u16) -> Vec<u8> {
        assert!(
            matches!(bits, 16 | 24),
            "Video 1 DIB depth must be 16 or 24, got {bits}"
        );
        let bytes = usize::from(bits / 8);
        let stride = (self.width * bytes).next_multiple_of(4);
        let mut out = vec![0u8; stride * self.height];
        for y in 0..self.height {
            let dst_row = (self.height - 1 - y) * stride;
            let dst = &mut out[dst_row..dst_row + self.width * bytes];
            let src = &self.pixels[y * self.width..(y + 1) * self.width];
            for (&p, pixel) in src.iter().zip(dst.chunks_exact_mut(bytes)) {
                if bytes == 2 {
                    pixel.copy_from_slice(&p.to_le_bytes());
                } else {
                    let b = ((p & 31) << 3) as u8;
                    let g = (((p >> 5) & 31) << 3) as u8;
                    let r = (((p >> 10) & 31) << 3) as u8;
                    pixel.copy_from_slice(&[b, g, r]);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Picture;

    fn pack_u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn dimensions_must_be_non_zero_multiples_of_four() {
        for (w, h) in [(0, 4), (4, 0), (6, 4), (4, 6)] {
            assert!(Picture::new(w, h).is_err(), "{w}x{h} must be refused");
        }
        for (w, h) in [(usize::MAX & !3, 4), (1usize << (usize::BITS - 4), 4)] {
            let err = Picture::new(w, h).expect_err("overflow must be refused");
            assert!(err.0.contains("overflows"), "{w}x{h}: {err}");
        }
        let pic = Picture::new(8, 4).expect("8x4 is valid");
        assert_eq!((pic.width, pic.height, pic.pixels.len()), (8, 4, 32));
        assert!(pic.pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn solid_blocks_cover_every_high_byte_outside_skip_range_bottom_to_top() {
        // 8x8 = 2x2 blocks, ordered BL, BR, TL, TR.
        // Exercise 0x80..0x83, 0x88..0x8f, and 0x90..0xff high bytes.
        let mut pic = Picture::new(8, 8).expect("8x8");
        let chunk = [pack_u16s(&[0x8001, 0x83e0, 0x8c40, 0xfc00]), vec![0, 0]].concat();
        pic.decode(&chunk).expect("decodes");

        // Top-left (y=0..4, x=0..4) = block 2 = 0x0c40
        // Top-right (y=0..4, x=4..8) = block 3 = 0x7c00
        // Bottom-left (y=4..8, x=0..4) = block 0 = 0x0001
        // Bottom-right (y=4..8, x=4..8) = block 1 = 0x03e0
        for y in 0..8 {
            for x in 0..8 {
                let expected = match (y < 4, x < 4) {
                    (true, true) => 0x0c40,
                    (true, false) => 0x7c00,
                    (false, true) => 0x0001,
                    (false, false) => 0x03e0,
                };
                assert_eq!(pic.pixels[y * 8 + x], expected, "pixel ({x},{y})");
            }
        }
        pic.clear();
        assert!(pic.pixels.iter().all(|&p| p == 0));
    }

    #[test]
    fn two_color_block_walks_bits_bottom_to_top_left_to_right_and_accepts_zero_flags() {
        // 8x4 = 2 blocks:
        // Block 0: flags = 0x0000 (all bits 0 -> every pixel takes c1 = 0x03e0,
        //          with bit 15 set in c1 masked off). Must NOT trigger EOF mid-stream.
        // Block 1: flags = 0x1005 = bits 0 (row 3, col 0), 2 (row 3, col 2),
        //          12 (row 0, col 0) set -> c0 = 0x7c00, others -> c1 = 0x001f.
        let mut pic = Picture::new(8, 4).expect("8x4");
        let chunk = [
            pack_u16s(&[0x0000, 0x7c00, 0x83e0]),
            pack_u16s(&[0x1005, 0x7c00, 0x001f]),
            vec![0, 0],
        ]
        .concat();
        pic.decode(&chunk).expect("decodes");

        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(pic.pixels[y * 8 + x], 0x03e0, "block 0 ({x},{y})");
            }
        }
        for y in 0..4 {
            for x in 0..4 {
                let expected = if (x, y) == (0, 3) || (x, y) == (2, 3) || (x, y) == (0, 0) {
                    0x7c00
                } else {
                    0x001f
                };
                assert_eq!(pic.pixels[y * 8 + 4 + x], expected, "block 1 ({x},{y})");
            }
        }
    }

    #[test]
    fn eight_color_block_splits_into_four_2x2_quadrants() {
        // 4x4 = 1 block.
        // Quadrant 0 (bottom-left: y=2..4, x=0..2): pair (0x0001, 0x0002), bit 15 of c0 set.
        // Quadrant 1 (bottom-right: y=2..4, x=2..4): pair (0x0003, 0x0004).
        // Quadrant 2 (top-left: y=0..2, x=0..2): pair (0x0005, 0x0006).
        // Quadrant 3 (top-right: y=0..2, x=2..4): pair (0x0007, 0x0008), bit 15 on c7 masked.
        // Set bit 0 (y=3,x=0), bit 6 (y=2,x=2), bit 9 (y=1,x=1), bit 14 (y=0,x=2).
        // Bit 15 is always 0 in 2-colour/8-colour mode (byte_b < 0x80).
        let flags: u16 = (1 << 0) | (1 << 6) | (1 << 9) | (1 << 14);
        let mut pic = Picture::new(4, 4).expect("4x4");
        let chunk = [
            pack_u16s(&[
                flags, 0x8001, 0x0002, 0x0003, 0x0004, 0x0005, 0x0006, 0x0007, 0x8008,
            ]),
            vec![0, 0],
        ]
        .concat();
        pic.decode(&chunk).expect("decodes");

        let expected: [[u16; 4]; 4] = [
            [6, 6, 7, 8], // y=0 (top): Q2 bits 12,13; Q3 bits 14,15
            [6, 5, 8, 8], // y=1:       Q2 bits 8,9;   Q3 bits 10,11
            [2, 2, 3, 4], // y=2:       Q0 bits 4,5;   Q1 bits 6,7
            [1, 2, 4, 4], // y=3 (bot): Q0 bits 0,1;   Q1 bits 2,3
        ];
        for (y, row) in expected.iter().enumerate() {
            for (x, &color) in row.iter().enumerate() {
                assert_eq!(pic.pixels[y * 4 + x], color, "({x},{y})");
            }
        }
    }

    #[test]
    fn skip_blocks_and_early_eof_preserve_previous_frame_pixels() {
        let mut pic = Picture::new(8, 8).expect("8x8");
        let frame0 = [pack_u16s(&[0x8001, 0x8820, 0x9040, 0xfc00]), vec![0, 0]].concat();
        pic.decode(&frame0).expect("frame 0");

        // Skip 2 blocks (BL and BR kept), paint TL solid 0x03e0, then early EOF 0x00 0x00 (TR kept).
        let frame1 = [vec![2, 0x84], pack_u16s(&[0x83e0]), vec![0, 0]].concat();
        pic.decode(&frame1).expect("frame 1");

        assert_eq!(pic.pixels[4 * 8], 0x0001, "BL preserved");
        assert_eq!(pic.pixels[4 * 8 + 4], 0x0820, "BR preserved");
        assert_eq!(pic.pixels[0], 0x03e0, "TL updated");
        assert_eq!(pic.pixels[4], 0x7c00, "TR preserved by early EOF");

        // Exact skip to end (4 blocks of 4).
        pic.decode(&[4, 0x84, 0, 0]).expect("skip all 4");
        assert_eq!(pic.pixels[0], 0x03e0);

        // Max skip (0xff, 0x87 = 1023 blocks) on a 128x128 frame (1024 blocks):
        // paints only the final block (top-right: y=0..4, x=124..128).
        let mut big = Picture::new(128, 128).expect("128x128");
        big.decode(&[vec![0xff, 0x87], pack_u16s(&[0xffff]), vec![0, 0]].concat())
            .expect("skip 1023");
        assert_eq!(big.pixels[124], 0x7fff);
        assert_eq!(big.pixels[120], 0x0000);
    }

    #[test]
    fn malformed_chunks_are_refused() {
        let mut pic = Picture::new(8, 4).expect("8x4");
        // Skip count 0 (0x84, 0x00).
        let zero = pic.decode(&[0, 0x84]).expect_err("zero skip");
        assert!(zero.0.contains("zero skip"), "{zero}");
        // Skip past end (3 blocks in a 2-block picture).
        assert!(pic.decode(&[3, 0x84]).is_err());
        // Truncated 2-color block (0 colors or missing c1), including flags=0x0000.
        assert!(pic.decode(&[0x01, 0x00]).is_err());
        assert!(pic.decode(&[0x01, 0x00, 0x10, 0x00]).is_err());
        assert!(pic.decode(&[0x00, 0x00, 0x10, 0x00]).is_err());
        // Truncated 8-color block (only 2 of 8 colors).
        assert!(pic.decode(&[0x01, 0x00, 0x10, 0x80, 0x20, 0x00]).is_err());
        // Single stray byte when blocks remain.
        assert!(pic.decode(&[0x80]).is_err());
    }

    #[test]
    fn dib_bits_are_bottom_up_padded_and_shift_rgb555_left_three_for_24_bit() {
        // 4x4 picture: top-left block with Advisor chroma-key 0x4212 (r=16,g=16,b=18)
        // on the top half and pure white 0x7fff (r=31,g=31,b=31) on the bottom half.
        let mut pic = Picture::new(4, 4).expect("4x4");
        // flags = 0x00ff -> bottom two rows (bits 0..8) = c0 (0x7fff), top two rows = c1 (0x4212).
        let chunk = [pack_u16s(&[0x00ff, 0x7fff, 0x4212]), vec![0, 0]].concat();
        pic.decode(&chunk).expect("decodes");

        let bits16 = pic.to_dib_bits(16);
        assert_eq!(bits16.len(), 4 * 2 * 4);
        // Bottom row comes first in DIB: 0x7fff.
        assert_eq!(&bits16[0..2], &0x7fffu16.to_le_bytes());
        // Top row comes last in DIB: 0x4212.
        assert_eq!(&bits16[3 * 8..3 * 8 + 2], &0x4212u16.to_le_bytes());

        let bits24 = pic.to_dib_bits(24);
        assert_eq!(bits24.len(), 4 * 3 * 4);
        // Bottom row: white 31<<3 = 0xf8.
        assert_eq!(&bits24[0..3], &[0xf8, 0xf8, 0xf8]);
        // Top row: 0x4212 -> B=18<<3=0x90, G=16<<3=0x80, R=16<<3=0x80 (Lego Chess 0x808090 key!).
        assert_eq!(&bits24[3 * 12..3 * 12 + 3], &[0x90, 0x80, 0x80]);
    }
}
