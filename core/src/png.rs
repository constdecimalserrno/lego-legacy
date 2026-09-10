//! A frame as a PNG file, stored rather than compressed: a picture a person,
//! or anything that reads pictures, can open, of exactly what was presented.
//!
//! ponytail: stored, a 640x480 frame is about 922 KB and its checksum is
//! taken a bit at a time; a run writing every flip wants DEFLATE's fixed codes
//! and a table-driven CRC.

/// A PNG of a frame given as blue, green, red and one more byte a pixel, row
/// by row from the top.
#[must_use]
pub fn encode(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width * 3 + 1) * height) as usize);
    for row in bgra
        .chunks_exact((width * 4).max(1) as usize)
        .take(height as usize)
    {
        // Each row says how it is filtered: not at all.
        raw.push(0);
        for pixel in row.as_chunks::<4>().0 {
            raw.extend([pixel[2], pixel[1], pixel[0]]);
        }
    }
    // A zlib stream of stored DEFLATE blocks, each at most 65,535 bytes.
    let mut zlib = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (n, block) in blocks.iter().enumerate() {
        zlib.push(u8::from(n + 1 == blocks.len()));
        let len = block.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend(adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    // Eight bits a channel, truecolour, and the only compression, filter and
    // interlace methods there are.
    header.extend([8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    let mut crc = !0u32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    out.extend((!crc).to_be_bytes());
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::encode;

    #[test]
    fn a_frame_comes_back_out_of_its_png_as_it_went_in() {
        // Two by two: blue, green, red, white.
        let bgra = [
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = encode(2, 2, &bgra);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        let idat_len = u32::from_be_bytes([png[33], png[34], png[35], png[36]]) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let zlib = &png[41..41 + idat_len];
        // Past the zlib header, the stream is DEFLATE; before the checksum.
        let raw = crate::inflate::inflate(&zlib[2..zlib.len() - 4]).expect("it inflates");
        assert_eq!(raw, [0, 0, 0, 255, 0, 255, 0, 0, 255, 0, 0, 255, 255, 255]);
        // The IHDR's CRC, as zlib's crc32 gives it for these bytes.
        assert_eq!(&png[29..33], &[0xfd, 0xd4, 0x9a, 0x73]);
    }

    #[test]
    fn a_frame_larger_than_one_stored_block_comes_back_whole_and_checksummed() {
        // 200 by 120 is 72,120 bytes of rows: two stored blocks.
        let bgra: Vec<u8> = (0..200u32 * 120 * 4).map(|i| (i % 251) as u8).collect();
        let png = encode(200, 120, &bgra);
        let idat_len = u32::from_be_bytes([png[33], png[34], png[35], png[36]]) as usize;
        let zlib = &png[41..41 + idat_len];
        let raw = crate::inflate::inflate(&zlib[2..zlib.len() - 4]).expect("it inflates");
        assert_eq!(raw.len(), (200 * 3 + 1) * 120);
        assert_eq!(&raw[1..4], [bgra[2], bgra[1], bgra[0]]);
        assert_eq!(
            zlib[zlib.len() - 4..],
            super::adler32(&raw).to_be_bytes(),
            "the stream ends with its checksum"
        );
    }
}
