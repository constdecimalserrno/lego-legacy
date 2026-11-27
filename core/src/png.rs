//! A frame as a PNG file, compressed with RFC 1951 fixed-Huffman DEFLATE and
//! adaptive None/Sub scanline filtering: a picture a person, or anything that
//! reads pictures, can open, of exactly what was presented (#309).

const ADLER_MOD: u32 = 65_521;
const ADLER_NMAX: usize = 5_552;

/// Compile-time IEEE 802.3 CRC-32 lookup table (polynomial `0xedb8_8320`).
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut n = 0usize;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = (c >> 1) ^ (0xedb8_8320 & (c & 1).wrapping_neg());
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
};

/// RFC 1951 length base values for symbols `257..=285`.
const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];

/// RFC 1951 extra bits for length symbols `257..=285`.
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];

/// RFC 1951 distance base values for codes `0..=29`.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];

/// RFC 1951 extra bits for distance codes `0..=29`.
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Reverse the low `bits` bits of `val` (for writing MSB-first Huffman codes
/// into RFC 1951's LSB-first bitstream).
const fn bit_reverse(mut val: u16, bits: u8) -> u16 {
    let mut rev = 0u16;
    let mut i = 0;
    while i < bits {
        rev = (rev << 1) | (val & 1);
        val >>= 1;
        i += 1;
    }
    rev
}

/// Pre-reversed RFC 1951 fixed literal/length Huffman codes `(code_lsb, bit_len)`.
const FIXED_LIT_LEN: [(u16, u8); 286] = {
    let mut table = [(0u16, 0u8); 286];
    let mut s = 0usize;
    while s <= 143 {
        table[s] = (bit_reverse(0x0030 + s as u16, 8), 8);
        s += 1;
    }
    while s <= 255 {
        table[s] = (bit_reverse(0x0190 + (s as u16 - 144), 9), 9);
        s += 1;
    }
    while s <= 279 {
        table[s] = (bit_reverse(s as u16 - 256, 7), 7);
        s += 1;
    }
    while s <= 285 {
        table[s] = (bit_reverse(0x00c0 + (s as u16 - 280), 8), 8);
        s += 1;
    }
    table
};

/// Pre-reversed 5-bit fixed distance codes.
const FIXED_DIST: [u16; 30] = {
    let mut table = [0u16; 30];
    let mut d = 0usize;
    while d < 30 {
        table[d] = bit_reverse(d as u16, 5);
        d += 1;
    }
    table
};

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    bits: u32,
}

impl BitWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            out: Vec::with_capacity(cap),
            acc: 0,
            bits: 0,
        }
    }

    #[inline(always)]
    fn write_bits(&mut self, value: u32, count: u32) {
        self.acc |= u64::from(value) << self.bits;
        self.bits += count;
        while self.bits >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.bits -= 8;
        }
    }

    #[inline(always)]
    fn write_lit(&mut self, byte: u8) {
        let (code, len) = FIXED_LIT_LEN[usize::from(byte)];
        self.write_bits(u32::from(code), u32::from(len));
    }

    #[inline(always)]
    fn write_match(&mut self, length: usize, distance: usize) {
        debug_assert!((3..=258).contains(&length));
        debug_assert!((1..=32_768).contains(&distance));
        let len_u16 = length as u16;
        let len_idx = if len_u16 == 258 {
            28
        } else {
            let mut idx = 0usize;
            while idx + 1 < 28 && LEN_BASE[idx + 1] <= len_u16 {
                idx += 1;
            }
            idx
        };
        let (lcode, lbits) = FIXED_LIT_LEN[257 + len_idx];
        self.write_bits(u32::from(lcode), u32::from(lbits));
        let lextra = LEN_EXTRA[len_idx];
        if lextra > 0 {
            self.write_bits(u32::from(len_u16 - LEN_BASE[len_idx]), u32::from(lextra));
        }

        let dist_u16 = distance as u16;
        let mut lo = 0usize;
        let mut hi = 29usize;
        while lo < hi {
            let mid = (lo + hi + 1) >> 1;
            if DIST_BASE[mid] <= dist_u16 {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let d_idx = lo;
        self.write_bits(u32::from(FIXED_DIST[d_idx]), 5);
        let dextra = DIST_EXTRA[d_idx];
        if dextra > 0 {
            self.write_bits(u32::from(dist_u16 - DIST_BASE[d_idx]), u32::from(dextra));
        }
    }

    fn finish(mut self) -> Vec<u8> {
        let (eob_code, eob_len) = FIXED_LIT_LEN[256];
        self.write_bits(u32::from(eob_code), u32::from(eob_len));
        if self.bits > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

#[inline(always)]
fn hash3(data: &[u8], pos: usize) -> usize {
    let v =
        u32::from(data[pos]) | (u32::from(data[pos + 1]) << 8) | (u32::from(data[pos + 2]) << 16);
    ((v.wrapping_mul(0x1e35_a7bd)) >> 19) as usize & 0x1fff
}

#[inline(always)]
fn match_len(data: &[u8], a: usize, b: usize, max_len: usize) -> usize {
    let mut n = 0usize;
    while n + 8 <= max_len && data[a + n..a + n + 8] == data[b + n..b + n + 8] {
        n += 8;
    }
    while n < max_len && data[a + n] == data[b + n] {
        n += 1;
    }
    n
}

/// Compress `raw` into one RFC 1951 fixed-Huffman DEFLATE block (with a fast
/// integer hash-chain LZ77 matcher), falling back to stored DEFLATE blocks if
/// incompressible input would otherwise expand.
fn deflate_fixed(raw: &[u8]) -> Vec<u8> {
    const HASH_SIZE: usize = 8_192;
    const WINDOW_MASK: usize = 32_767;
    const MAX_CHAIN: usize = 12;
    const NONE_POS: u32 = u32::MAX;

    let mut bw = BitWriter::with_capacity(raw.len() / 2 + 16);
    // BFINAL = 1, BTYPE = 01 (fixed Huffman).
    bw.write_bits(0b011, 3);

    let len = raw.len();
    if len < 3 {
        for &b in raw {
            bw.write_lit(b);
        }
    } else {
        let mut head = vec![NONE_POS; HASH_SIZE];
        let mut prev = vec![NONE_POS; 32_768];
        let mut pos = 0usize;

        while pos < len {
            if pos + 3 > len {
                bw.write_lit(raw[pos]);
                pos += 1;
                continue;
            }
            let h = hash3(raw, pos);
            let mut best_len = 0usize;
            let mut best_dist = 0usize;
            let max_len = (len - pos).min(258);

            // Fast RLE check at dist = 1 and dist = 3 (very frequent after PNG Sub filter).
            for d in [1usize, 3usize] {
                if pos >= d
                    && raw[pos - d] == raw[pos]
                    && raw[pos - d + 1] == raw[pos + 1]
                    && raw[pos - d + 2] == raw[pos + 2]
                {
                    let ml = match_len(raw, pos - d, pos, max_len);
                    if ml > best_len {
                        best_len = ml;
                        best_dist = d;
                        if best_len == max_len {
                            break;
                        }
                    }
                }
            }

            let mut cand = head[h];
            let mut steps = 0usize;
            while cand != NONE_POS && steps < MAX_CHAIN && best_len < max_len {
                let cpos = cand as usize;
                let dist = pos - cpos;
                if dist == 0 || dist > 32_768 {
                    break;
                }
                if best_len < 3 || raw[cpos + best_len] == raw[pos + best_len] {
                    let ml = match_len(raw, cpos, pos, max_len);
                    if ml > best_len {
                        best_len = ml;
                        best_dist = dist;
                    }
                }
                cand = prev[cpos & WINDOW_MASK];
                steps += 1;
            }

            prev[pos & WINDOW_MASK] = head[h];
            head[h] = pos as u32;

            if best_len >= 3 {
                bw.write_match(best_len, best_dist);
                // Insert skipped positions into the hash chain for better matches.
                let end = (pos + best_len).min(len.saturating_sub(2));
                let mut p = pos + 1;
                while p < end {
                    let ph = hash3(raw, p);
                    prev[p & WINDOW_MASK] = head[ph];
                    head[ph] = p as u32;
                    p += 1;
                }
                pos += best_len;
            } else {
                bw.write_lit(raw[pos]);
                pos += 1;
            }
        }
    }

    let compressed = bw.finish();
    let stored_size = raw.len() + raw.len().div_ceil(65_535).max(1) * 5;
    if compressed.len() < stored_size {
        compressed
    } else {
        deflate_stored(raw)
    }
}

fn deflate_stored(raw: &[u8]) -> Vec<u8> {
    if raw.is_empty() {
        return vec![0x01, 0x00, 0x00, 0xff, 0xff];
    }
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    let mut out = Vec::with_capacity(raw.len() + blocks.len() * 5);
    for (n, block) in blocks.iter().enumerate() {
        out.push(u8::from(n + 1 == blocks.len()));
        let len = block.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out
}

/// Filter BGRX rows into PNG RGB24 scanlines, choosing None (0) or Sub (1) per
/// scanline by minimum sum of signed byte magnitudes.
fn build_filtered_scanlines(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let row_rgb = w * 3;
    let mut raw = Vec::with_capacity((row_rgb + 1) * h);
    let mut rgb_row = vec![0u8; row_rgb];
    let mut sub_row = vec![0u8; row_rgb];

    for row in bgra.chunks_exact((w * 4).max(1)).take(h) {
        let mut none_score = 0u32;
        let mut sub_score = 0u32;
        let mut pr = 0u8;
        let mut pg = 0u8;
        let mut pb = 0u8;

        for (i, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
            let r = pixel[2];
            let g = pixel[1];
            let b = pixel[0];
            let base = i * 3;
            rgb_row[base] = r;
            rgb_row[base + 1] = g;
            rgb_row[base + 2] = b;

            let sr = r.wrapping_sub(pr);
            let sg = g.wrapping_sub(pg);
            let sb = b.wrapping_sub(pb);
            sub_row[base] = sr;
            sub_row[base + 1] = sg;
            sub_row[base + 2] = sb;

            none_score += u32::from((r as i8).unsigned_abs())
                + u32::from((g as i8).unsigned_abs())
                + u32::from((b as i8).unsigned_abs());
            sub_score += u32::from((sr as i8).unsigned_abs())
                + u32::from((sg as i8).unsigned_abs())
                + u32::from((sb as i8).unsigned_abs());

            pr = r;
            pg = g;
            pb = b;
        }

        if sub_score < none_score {
            raw.push(1);
            raw.extend_from_slice(&sub_row);
        } else {
            raw.push(0);
            raw.extend_from_slice(&rgb_row);
        }
    }
    raw
}

/// A PNG of a frame given as blue, green, red and one more byte a pixel, row
/// by row from the top.
#[must_use]
pub fn encode(width: u32, height: u32, bgra: &[u8]) -> Vec<u8> {
    let raw = build_filtered_scanlines(width, height, bgra);
    let deflated = deflate_fixed(&raw);

    let mut zlib = Vec::with_capacity(2 + deflated.len() + 4);
    zlib.extend_from_slice(&[0x78, 0x01]);
    zlib.extend_from_slice(&deflated);
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut header = Vec::with_capacity(13);
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    // Eight bits a channel, truecolour, and the only compression, filter and
    // interlace methods there are.
    header.extend([8, 2, 0, 0, 0]);
    let mut out = Vec::with_capacity(8 + 25 + 12 + zlib.len() + 12);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn crc32(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in kind.iter().chain(data) {
        crc = CRC_TABLE[((crc ^ u32::from(byte)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    out.extend(crc32(kind, data).to_be_bytes());
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(ADLER_NMAX) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b << 16) | a
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = i32::from(a);
    let b = i32::from(b);
    let c = i32::from(c);
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// Decode an 8-bit non-interlaced RGB or RGBA PNG into `(width, height, bgra)`
/// using pure `std` and [`crate::inflate::inflate`].
///
/// Supports all five RFC 2083 scanline filters (`None`, `Sub`, `Up`, `Average`,
/// and `Paeth`), validates chunk CRC-32s and the zlib Adler-32 checksum, and
/// returns dense top-down BGRA32 pixels (with alpha `255` for RGB24 inputs).
///
/// # Errors
/// Returns a descriptive error if the PNG header, chunk structure, CRC-32,
/// color format, zlib stream, Adler-32 checksum, or scanline filters are
/// invalid or unsupported.
pub fn decode_to_bgra(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let Some(mut rest) = png.strip_prefix(b"\x89PNG\r\n\x1a\n") else {
        return Err("invalid PNG signature".to_owned());
    };

    let mut first_chunk = true;
    let mut dims: Option<(u32, u32, usize)> = None;
    let mut idat_started = false;
    let mut idat_ended = false;
    let mut zlib = Vec::new();
    let mut saw_iend = false;

    while !rest.is_empty() {
        if rest.len() < 12 {
            return Err("truncated PNG chunk header".to_owned());
        }
        let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
        let kind: [u8; 4] = [rest[4], rest[5], rest[6], rest[7]];
        if len > rest.len() - 12 {
            return Err("truncated PNG chunk payload".to_owned());
        }
        let data = &rest[8..8 + len];
        let crc_off = 8 + len;
        let expected_crc = u32::from_be_bytes([
            rest[crc_off],
            rest[crc_off + 1],
            rest[crc_off + 2],
            rest[crc_off + 3],
        ]);
        let actual_crc = crc32(&kind, data);
        if actual_crc != expected_crc {
            return Err(format!(
                "PNG chunk {:?} CRC mismatch (expected {expected_crc:#010x}, got {actual_crc:#010x})",
                std::str::from_utf8(&kind).unwrap_or("????")
            ));
        }

        match &kind {
            b"IHDR" => {
                if !first_chunk || dims.is_some() {
                    return Err("IHDR must appear once as the first PNG chunk".to_owned());
                }
                if data.len() != 13 {
                    return Err(format!("invalid IHDR length {}", data.len()));
                }
                let w = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                let h = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                if w == 0 || h == 0 || w > i32::MAX as u32 || h > i32::MAX as u32 {
                    return Err(format!("invalid PNG dimensions {w}x{h}"));
                }
                let (bit_depth, color_type, compression, filter_method, interlace) =
                    (data[8], data[9], data[10], data[11], data[12]);
                if bit_depth != 8 || compression != 0 || filter_method != 0 || interlace != 0 {
                    return Err(format!(
                        "unsupported PNG format (depth={bit_depth}, color={color_type}, comp={compression}, filter={filter_method}, interlace={interlace})"
                    ));
                }
                let bpp = match color_type {
                    2 => 3usize,
                    6 => 4usize,
                    _ => {
                        return Err(format!(
                            "unsupported PNG color type {color_type} (expected 2=RGB or 6=RGBA)"
                        ));
                    }
                };
                dims = Some((w, h, bpp));
            }
            b"IDAT" => {
                if dims.is_none() {
                    return Err("IDAT appeared before IHDR".to_owned());
                }
                if idat_ended {
                    return Err("non-consecutive IDAT chunks".to_owned());
                }
                idat_started = true;
                zlib.extend_from_slice(data);
            }
            b"IEND" => {
                if !data.is_empty() {
                    return Err(format!("non-empty IEND chunk ({} bytes)", data.len()));
                }
                saw_iend = true;
                break;
            }
            _ => {
                if idat_started {
                    idat_ended = true;
                }
                // RFC 2083 §3.3: bit 5 clear in byte 0 marks a critical chunk.
                if (kind[0] & 0x20) == 0 && &kind != b"PLTE" {
                    return Err(format!(
                        "unsupported critical PNG chunk {:?}",
                        std::str::from_utf8(&kind).unwrap_or("????")
                    ));
                }
            }
        }
        first_chunk = false;
        rest = &rest[crc_off + 4..];
    }

    let (w, h, bpp) = dims.ok_or_else(|| "missing IHDR chunk".to_owned())?;
    if !saw_iend {
        return Err("missing IEND chunk".to_owned());
    }
    if zlib.len() < 6 {
        return Err("missing or truncated IDAT zlib stream".to_owned());
    }

    let (cmf, flg) = (zlib[0], zlib[1]);
    if (cmf & 0x0f) != 8
        || (cmf >> 4) > 7
        || !(u16::from(cmf) * 256 + u16::from(flg)).is_multiple_of(31)
    {
        return Err(format!("invalid zlib header [{cmf:#04x}, {flg:#04x}]"));
    }
    if (flg & 0x20) != 0 {
        return Err("unsupported zlib preset dictionary".to_owned());
    }

    let raw = crate::inflate::inflate(&zlib[2..zlib.len() - 4])
        .map_err(|e| format!("PNG inflate failed: {e}"))?;
    let expected_adler = u32::from_be_bytes([
        zlib[zlib.len() - 4],
        zlib[zlib.len() - 3],
        zlib[zlib.len() - 2],
        zlib[zlib.len() - 1],
    ]);
    let actual_adler = adler32(&raw);
    if actual_adler != expected_adler {
        return Err(format!(
            "PNG Adler-32 mismatch (expected {expected_adler:#010x}, got {actual_adler:#010x})"
        ));
    }

    let width = w as usize;
    let height = h as usize;
    let (row_bytes, stride_in, expected_raw_len, out_len) = width
        .checked_mul(bpp)
        .and_then(|rb| {
            let si = rb.checked_add(1)?;
            let raw_len = si.checked_mul(height)?;
            let bgra_len = width.checked_mul(height)?.checked_mul(4)?;
            Some((rb, si, raw_len, bgra_len))
        })
        .ok_or_else(|| "PNG dimensions overflow".to_owned())?;
    if raw.len() != expected_raw_len {
        return Err(format!(
            "PNG inflated length {} != expected {expected_raw_len}",
            raw.len()
        ));
    }

    let mut prev_row = vec![0u8; row_bytes];
    let mut curr_row = vec![0u8; row_bytes];
    let mut bgra = Vec::with_capacity(out_len);

    for y in 0..height {
        let line = &raw[y * stride_in..(y + 1) * stride_in];
        let filter = line[0];
        let data = &line[1..];
        match filter {
            0 => curr_row.copy_from_slice(data),
            1 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { curr_row[i - bpp] } else { 0 };
                    curr_row[i] = data[i].wrapping_add(a);
                }
            }
            2 => {
                for i in 0..row_bytes {
                    curr_row[i] = data[i].wrapping_add(prev_row[i]);
                }
            }
            3 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { curr_row[i - bpp] } else { 0 };
                    let b = prev_row[i];
                    let avg = ((u16::from(a) + u16::from(b)) >> 1) as u8;
                    curr_row[i] = data[i].wrapping_add(avg);
                }
            }
            4 => {
                for i in 0..row_bytes {
                    let a = if i >= bpp { curr_row[i - bpp] } else { 0 };
                    let b = prev_row[i];
                    let c = if i >= bpp { prev_row[i - bpp] } else { 0 };
                    curr_row[i] = data[i].wrapping_add(paeth_predictor(a, b, c));
                }
            }
            other => return Err(format!("unsupported PNG scanline filter {other}")),
        }

        if bpp == 3 {
            for px in curr_row.as_chunks::<3>().0 {
                bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        } else {
            for px in curr_row.as_chunks::<4>().0 {
                bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    Ok((w, h, bgra))
}

#[cfg(test)]
pub(crate) fn decode_png_to_rgb(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w, h, bgra) = decode_to_bgra(png).expect("valid PNG");
    let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
    for px in bgra.as_chunks::<4>().0 {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    (w, h, rgb)
}

#[cfg(test)]
mod tests {
    use super::{
        adler32, chunk, decode_png_to_rgb, decode_to_bgra, deflate_stored, encode, paeth_predictor,
    };

    /// Encode a synthetic PNG where each scanline uses the filter type given in
    /// `row_filters` (cycled if shorter than `height`), supporting RGB (`2`)
    /// and RGBA (`6`).
    fn encode_with_filters(
        width: u32,
        height: u32,
        bgra: &[u8],
        color_type: u8,
        row_filters: &[u8],
    ) -> Vec<u8> {
        let w = width as usize;
        let h = height as usize;
        let bpp = if color_type == 6 { 4usize } else { 3usize };
        let row_bytes = w * bpp;
        let mut raw = Vec::with_capacity((row_bytes + 1) * h);
        let mut prev = vec![0u8; row_bytes];
        let mut curr = vec![0u8; row_bytes];

        for y in 0..h {
            for x in 0..w {
                let si = (y * w + x) * 4;
                let di = x * bpp;
                curr[di] = bgra[si + 2];
                curr[di + 1] = bgra[si + 1];
                curr[di + 2] = bgra[si];
                if bpp == 4 {
                    curr[di + 3] = bgra[si + 3];
                }
            }
            let filter = row_filters[y % row_filters.len()];
            raw.push(filter);
            for i in 0..row_bytes {
                let a = if i >= bpp { curr[i - bpp] } else { 0 };
                let b = prev[i];
                let c = if i >= bpp { prev[i - bpp] } else { 0 };
                let encoded = match filter {
                    0 => curr[i],
                    1 => curr[i].wrapping_sub(a),
                    2 => curr[i].wrapping_sub(b),
                    3 => curr[i].wrapping_sub(((u16::from(a) + u16::from(b)) >> 1) as u8),
                    4 => curr[i].wrapping_sub(paeth_predictor(a, b, c)),
                    other => panic!("bad test filter {other}"),
                };
                raw.push(encoded);
            }
            prev.copy_from_slice(&curr);
        }

        let deflated = deflate_stored(&raw);
        let mut zlib = Vec::with_capacity(2 + deflated.len() + 4);
        zlib.extend_from_slice(&[0x78, 0x9c]);
        zlib.extend_from_slice(&deflated);
        zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut ihdr = Vec::with_capacity(13);
        ihdr.extend(width.to_be_bytes());
        ihdr.extend(height.to_be_bytes());
        ihdr.extend([8, color_type, 0, 0, 0]);

        let mut out = Vec::new();
        out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        chunk(&mut out, b"IHDR", &ihdr);
        let mid = zlib.len() / 2;
        chunk(&mut out, b"IDAT", &zlib[..mid]);
        chunk(&mut out, b"IDAT", &zlib[mid..]);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn a_frame_comes_back_out_of_its_png_as_it_went_in() {
        let bgra = [
            255, 0, 0, 255, 0, 255, 0, 255, //
            0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = encode(2, 2, &bgra);
        assert_eq!(&png[29..33], &[0xfd, 0xd4, 0x9a, 0x73]);
        let (w, h, rgb) = decode_png_to_rgb(&png);
        assert_eq!((w, h), (2, 2));
        assert_eq!(rgb, [0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255]);
        let (bw, bh, decoded) = decode_to_bgra(&png).expect("decode_to_bgra");
        assert_eq!((bw, bh), (2, 2));
        assert_eq!(decoded, bgra);
    }

    #[test]
    fn a_frame_larger_than_one_stored_block_comes_back_whole_and_checksummed() {
        let mut bgra: Vec<u8> = (0..200u32 * 120 * 4).map(|i| (i % 251) as u8).collect();
        for px in bgra.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let png = encode(200, 120, &bgra);
        let (w, h, decoded) = decode_to_bgra(&png).expect("decode_to_bgra");
        assert_eq!((w, h), (200, 120));
        assert_eq!(decoded, bgra);
    }

    #[test]
    fn fixed_huffman_shrinks_640x480_frames_substantially_and_roundtrips_bit_exact() {
        let (w, h) = (640u32, 480u32);
        let stored_baseline = (w * 3 + 1) * h;

        let mut scene = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let idx = ((y * w + x) * 4) as usize;
                let (r, g, b) = if y < 60 {
                    let block = ((x / 40) % 4) as u8;
                    (30 + block * 20, 40 + block * 15, 70 + block * 10)
                } else if y < 260 {
                    (40, (80 + y / 2) as u8, (180 + y / 4) as u8)
                } else {
                    let tx = (x % 16) as u8;
                    let ty = (y % 16) as u8;
                    let check = ((x / 16 + y / 16) & 1) as u8;
                    (
                        50 + check * 25 + tx / 2,
                        90 + check * 20 + ty / 2,
                        40 + ((tx ^ ty) & 7) * 4,
                    )
                };
                scene[idx] = b;
                scene[idx + 1] = g;
                scene[idx + 2] = r;
                scene[idx + 3] = 255;
            }
        }
        let png = encode(w, h, &scene);
        assert!(
            png.len() < 120_000,
            "640x480 scene should shrink well below 120 KB (was {} vs stored ~{stored_baseline})",
            png.len()
        );
        let (dw, dh, decoded) = decode_to_bgra(&png).expect("decode_to_bgra scene");
        assert_eq!((dw, dh), (w, h));
        assert_eq!(decoded, scene);

        let mut noise = vec![0u8; (w * h * 4) as usize];
        let mut rng = 0x1234_5678_9abc_def0u64;
        for px in noise.as_chunks_mut::<4>().0 {
            rng = rng.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            px[0] = (rng >> 24) as u8;
            px[1] = (rng >> 32) as u8;
            px[2] = (rng >> 40) as u8;
            px[3] = 255;
        }
        let noise_png = encode(w, h, &noise);
        let (nw, nh, ndecoded) = decode_to_bgra(&noise_png).expect("decode_to_bgra noise");
        assert_eq!((nw, nh), (w, h));
        assert_eq!(ndecoded, noise);
    }

    #[test]
    fn decode_to_bgra_roundtrips_all_five_png_filter_types_and_rgba() {
        let (w, h) = (17u32, 13u32);
        let mut rgb_bgra = vec![0u8; (w * h * 4) as usize];
        let mut rgba_bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let b = ((x * 19 + y * 37) % 256) as u8;
                let g = (((x * 7) ^ (y * 13)).wrapping_add(101) % 256) as u8;
                let r = ((x * x + y * 53) % 256) as u8;
                let a = ((x * 11 + y * 29) % 256) as u8;
                rgb_bgra[i..i + 4].copy_from_slice(&[b, g, r, 255]);
                rgba_bgra[i..i + 4].copy_from_slice(&[b, g, r, a]);
            }
        }

        for filter in 0u8..=4 {
            let png = encode_with_filters(w, h, &rgb_bgra, 2, &[filter]);
            let (dw, dh, out) = decode_to_bgra(&png)
                .unwrap_or_else(|e| panic!("RGB filter {filter} decode failed: {e}"));
            assert_eq!((dw, dh), (w, h));
            assert_eq!(out, rgb_bgra, "RGB filter {filter} mismatch");

            let rgba_png = encode_with_filters(w, h, &rgba_bgra, 6, &[filter]);
            let (rw, rh, rout) = decode_to_bgra(&rgba_png)
                .unwrap_or_else(|e| panic!("RGBA filter {filter} decode failed: {e}"));
            assert_eq!((rw, rh), (w, h));
            assert_eq!(rout, rgba_bgra, "RGBA filter {filter} mismatch");
        }

        let mixed = encode_with_filters(w, h, &rgb_bgra, 2, &[0, 1, 2, 3, 4]);
        let (mw, mh, mout) = decode_to_bgra(&mixed).expect("mixed filters decode");
        assert_eq!((mw, mh), (w, h));
        assert_eq!(mout, rgb_bgra);
    }

    #[test]
    fn decode_to_bgra_rejects_malformed_inputs_without_panicking() {
        assert!(decode_to_bgra(&[]).is_err());
        assert!(decode_to_bgra(b"not a png").is_err());

        let valid = encode(
            2,
            2,
            &[
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 1, 2, 3, 255,
            ],
        );
        assert!(decode_to_bgra(&valid[..valid.len() - 5]).is_err());

        let mut bad_crc = valid.clone();
        bad_crc[32] ^= 0x01;
        assert!(decode_to_bgra(&bad_crc).is_err());

        let build_1x1 =
            |raw: &[u8], adler_xor: u32, extra_chunk: Option<(&[u8; 4], &[u8])>, iend: &[u8]| {
                let mut ihdr = Vec::with_capacity(13);
                ihdr.extend(1u32.to_be_bytes());
                ihdr.extend(1u32.to_be_bytes());
                ihdr.extend([8, 2, 0, 0, 0]);
                let mut zlib = vec![0x78, 0x01];
                zlib.extend_from_slice(&deflate_stored(raw));
                zlib.extend_from_slice(&(adler32(raw) ^ adler_xor).to_be_bytes());
                let mut out = Vec::new();
                out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
                chunk(&mut out, b"IHDR", &ihdr);
                if let Some((kind, payload)) = extra_chunk {
                    chunk(&mut out, kind, payload);
                }
                chunk(&mut out, b"IDAT", &zlib);
                chunk(&mut out, b"IEND", iend);
                out
            };

        assert!(decode_to_bgra(&build_1x1(&[0, 10, 20, 30], 1, None, &[])).is_err());
        assert!(decode_to_bgra(&build_1x1(&[5, 10, 20, 30], 0, None, &[])).is_err());
        assert!(
            decode_to_bgra(&build_1x1(&[0, 10, 20, 30], 0, Some((b"CRIT", b"x")), &[])).is_err()
        );
        assert!(decode_to_bgra(&build_1x1(&[0, 10, 20, 30], 0, None, b"x")).is_err());
    }
}
