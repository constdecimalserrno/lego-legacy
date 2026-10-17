//! DEFLATE, written here because nothing may be linked (ADR-0001).
//!
//! The pressings keep their data compressed this way, so a title's own files
//! cannot be read without it. RFC 1951 is the whole specification and this is
//! all of it: stored blocks, fixed codes, dynamic codes, and the back
//! references that make it worth doing.

use std::fmt;

/// What the compressed data turned out not to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InflateError {
    /// The stream ended before the data did.
    Truncated,
    /// A block type RFC 1951 does not define.
    BadBlockType(u8),
    /// A stored block whose length and its complement disagree.
    BadStoredLength,
    /// A code that is not in the tree it was read from.
    BadCode,
    /// A back reference pointing before the start of the output.
    BadDistance { distance: usize, have: usize },
}

impl fmt::Display for InflateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "the compressed data ends mid-stream"),
            Self::BadBlockType(t) => write!(f, "block type {t} is not one of the three"),
            Self::BadStoredLength => {
                write!(
                    f,
                    "a stored block's length disagrees with its own complement"
                )
            }
            Self::BadCode => write!(f, "a code that is not in its tree"),
            Self::BadDistance { distance, have } => write!(
                f,
                "a back reference {distance} bytes back, with only {have} bytes written"
            ),
        }
    }
}

impl std::error::Error for InflateError {}

/// Bits, least significant first, which is the order DEFLATE reads them in.
struct Bits<'a> {
    bytes: &'a [u8],
    at: usize,
    bit: u32,
}

impl<'a> Bits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            bit: 0,
        }
    }

    fn bit(&mut self) -> Result<u32, InflateError> {
        let byte = *self.bytes.get(self.at).ok_or(InflateError::Truncated)?;
        let value = (u32::from(byte) >> self.bit) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.at += 1;
        }
        Ok(value)
    }

    fn bits(&mut self, n: u32) -> Result<u32, InflateError> {
        let mut value = 0;
        for i in 0..n {
            value |= self.bit()? << i;
        }
        Ok(value)
    }

    /// Move to the next byte, which is where a stored block starts.
    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.at += 1;
        }
    }
}

/// A canonical Huffman tree, as its code lengths.
struct Tree {
    /// How many codes there are of each length.
    counts: [u16; 16],
    /// The symbols, ordered by code length and then by symbol.
    symbols: Vec<u16>,
}

impl Tree {
    /// Build a tree from one code length per symbol, as DEFLATE stores them.
    fn from_lengths(lengths: &[u8]) -> Self {
        let mut counts = [0u16; 16];
        for &len in lengths {
            counts[len as usize] += 1;
        }
        // Length zero means the symbol is not in the tree at all.
        counts[0] = 0;

        let mut offsets = [0u16; 16];
        for i in 1..15 {
            offsets[i + 1] = offsets[i] + counts[i];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &len) in lengths.iter().enumerate() {
            if len != 0 {
                symbols[offsets[len as usize] as usize] = u16::try_from(symbol).unwrap_or(0);
                offsets[len as usize] += 1;
            }
        }
        Self { counts, symbols }
    }

    /// Read one symbol, walking down the tree a bit at a time.
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, InflateError> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for length in 1..16 {
            code |= i32::try_from(bits.bit()?).unwrap_or(0);
            let count = i32::from(self.counts[length]);
            if code - count < first {
                let at = usize::try_from(index + (code - first)).unwrap_or(0);
                return self.symbols.get(at).copied().ok_or(InflateError::BadCode);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(InflateError::BadCode)
    }
}

/// How much longer a match is, per length symbol, and how many extra bits say
/// the rest.
const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u32; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u32; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Decompress a raw DEFLATE stream: no zlib header, no checksum.
///
/// # Errors
/// When the stream is not a DEFLATE stream, or ends inside one.
pub fn inflate(bytes: &[u8]) -> Result<Vec<u8>, InflateError> {
    let mut bits = Bits::new(bytes);
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() * 4);

    loop {
        let last = bits.bit()? == 1;
        let kind = bits.bits(2)?;
        match kind {
            // Stored: the bytes are simply there.
            0 => {
                bits.align();
                let len = usize::try_from(bits.bits(16)?).unwrap_or(0);
                let complement = usize::try_from(bits.bits(16)?).unwrap_or(0);
                if len != !complement & 0xffff {
                    return Err(InflateError::BadStoredLength);
                }
                let start = bits.at;
                let chunk = bytes
                    .get(start..start + len)
                    .ok_or(InflateError::Truncated)?;
                out.extend_from_slice(chunk);
                bits.at += len;
            }
            // Fixed codes: the tree everyone already knows.
            1 => {
                let mut lengths = [0u8; 288];
                for (symbol, len) in lengths.iter_mut().enumerate() {
                    *len = match symbol {
                        0..=143 => 8,
                        144..=255 => 9,
                        256..=279 => 7,
                        _ => 8,
                    };
                }
                let literals = Tree::from_lengths(&lengths);
                let distances = Tree::from_lengths(&[5u8; 30]);
                block(&mut bits, &mut out, &literals, &distances)?;
            }
            // Dynamic codes: the tree is described first.
            2 => {
                let (literals, distances) = read_trees(&mut bits)?;
                block(&mut bits, &mut out, &literals, &distances)?;
            }
            other => return Err(InflateError::BadBlockType(u8::try_from(other).unwrap_or(3))),
        }
        if last {
            break;
        }
    }
    Ok(out)
}

/// The order the code-length code's own lengths are written in.
const LENGTH_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn read_trees(bits: &mut Bits<'_>) -> Result<(Tree, Tree), InflateError> {
    let literal_count = bits.bits(5)? as usize + 257;
    let distance_count = bits.bits(5)? as usize + 1;
    let code_count = bits.bits(4)? as usize + 4;

    let mut code_lengths = [0u8; 19];
    for i in 0..code_count {
        code_lengths[LENGTH_ORDER[i]] = u8::try_from(bits.bits(3)?).unwrap_or(0);
    }
    let code_tree = Tree::from_lengths(&code_lengths);

    // The two trees' lengths are written as one run, with three symbols that
    // repeat what came before or write zeros in bulk.
    let mut lengths = vec![0u8; literal_count + distance_count];
    let mut at = 0;
    while at < lengths.len() {
        let symbol = code_tree.decode(bits)?;
        match symbol {
            0..=15 => {
                lengths[at] = u8::try_from(symbol).unwrap_or(0);
                at += 1;
            }
            16 => {
                let previous = if at > 0 { lengths[at - 1] } else { 0 };
                let repeat = bits.bits(2)? as usize + 3;
                for _ in 0..repeat {
                    if at < lengths.len() {
                        lengths[at] = previous;
                        at += 1;
                    }
                }
            }
            17 => {
                at += bits.bits(3)? as usize + 3;
            }
            _ => {
                at += bits.bits(7)? as usize + 11;
            }
        }
    }
    let literals = Tree::from_lengths(&lengths[..literal_count.min(lengths.len())]);
    let distances = Tree::from_lengths(&lengths[literal_count.min(lengths.len())..]);
    Ok((literals, distances))
}

/// One block's worth of literals and back references.
fn block(
    bits: &mut Bits<'_>,
    out: &mut Vec<u8>,
    literals: &Tree,
    distances: &Tree,
) -> Result<(), InflateError> {
    loop {
        let symbol = literals.decode(bits)?;
        match symbol {
            0..=255 => out.push(u8::try_from(symbol).unwrap_or(0)),
            // The end of the block.
            256 => return Ok(()),
            _ => {
                let index = symbol as usize - 257;
                let base = *LENGTH_BASE.get(index).ok_or(InflateError::BadCode)?;
                let extra = LENGTH_EXTRA[index];
                let length = base as usize + bits.bits(extra)? as usize;

                let distance_symbol = distances.decode(bits)? as usize;
                let base = *DISTANCE_BASE
                    .get(distance_symbol)
                    .ok_or(InflateError::BadCode)?;
                let extra = DISTANCE_EXTRA[distance_symbol];
                let distance = base as usize + bits.bits(extra)? as usize;

                if distance > out.len() {
                    return Err(InflateError::BadDistance {
                        distance,
                        have: out.len(),
                    });
                }
                // Copied one byte at a time on purpose: a match may overlap
                // itself, which is how a run of one byte is written.
                let from = out.len() - distance;
                for i in 0..length {
                    let byte = out[from + i];
                    out.push(byte);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{inflate, InflateError};

    #[test]
    fn a_stored_block_comes_back_as_itself() {
        // Final block, stored, five bytes, then the bytes.
        let mut data = vec![0x01, 0x05, 0x00, 0xfa, 0xff];
        data.extend_from_slice(b"hello");
        assert_eq!(inflate(&data).expect("this inflates"), b"hello");
    }

    #[test]
    fn a_stored_block_that_lies_about_its_length_is_refused() {
        let data = vec![0x01, 0x05, 0x00, 0x00, 0x00, b'h'];
        assert_eq!(inflate(&data), Err(InflateError::BadStoredLength));
    }

    #[test]
    fn a_stream_that_ends_early_is_refused_rather_than_guessed_at() {
        assert_eq!(inflate(&[]), Err(InflateError::Truncated));
        assert_eq!(inflate(&[0x01, 0x05]), Err(InflateError::Truncated));
    }

    /// Streams compressed by a reference implementation, with what they came
    /// from. A decoder that agrees with these agrees with everything that ever
    /// wrote a DEFLATE stream.
    fn vectors() -> [(&'static [u8], Vec<u8>); 3] {
        [
            // a run of one byte
            (&[75, 76, 132, 2, 0], b"aaaaaaaaa".to_vec()),
            // text with repeats
            (
                &[
                    43, 201, 72, 85, 40, 202, 79, 206, 86, 40, 74, 204, 76, 73, 45, 42, 6, 211, 96,
                    145, 98, 29, 133, 196, 188, 20, 133, 18, 168, 2, 168, 76, 82, 98, 114, 54, 0,
                ],
                b"the rock raiders raid rocks, and the rocks raid back".to_vec(),
            ),
            // a longer stream with distant matches: "WWAD" and every byte value,
            // forty times over
            (
                &[
                    11, 15, 119, 116, 97, 96, 100, 98, 102, 97, 101, 99, 231, 224, 228, 226, 230,
                    225, 229, 227, 23, 16, 20, 18, 22, 17, 21, 19, 151, 144, 148, 146, 150, 145,
                    149, 147, 87, 80, 84, 82, 86, 81, 85, 83, 215, 208, 212, 210, 214, 209, 213,
                    211, 55, 48, 52, 50, 54, 49, 53, 51, 183, 176, 180, 178, 182, 177, 181, 179,
                    119, 112, 116, 114, 118, 113, 117, 115, 247, 240, 244, 242, 246, 241, 245, 243,
                    15, 8, 12, 10, 14, 9, 13, 11, 143, 136, 140, 138, 142, 137, 141, 139, 79, 72,
                    76, 74, 78, 73, 77, 75, 207, 200, 204, 202, 206, 201, 205, 203, 47, 40, 44, 42,
                    46, 41, 45, 43, 175, 168, 172, 170, 174, 169, 173, 171, 111, 104, 108, 106,
                    110, 105, 109, 107, 239, 232, 236, 234, 238, 233, 237, 235, 159, 48, 113, 210,
                    228, 41, 83, 167, 77, 159, 49, 115, 214, 236, 57, 115, 231, 205, 95, 176, 112,
                    209, 226, 37, 75, 151, 45, 95, 177, 114, 213, 234, 53, 107, 215, 173, 223, 176,
                    113, 211, 230, 45, 91, 183, 109, 223, 177, 115, 215, 238, 61, 123, 247, 237,
                    63, 112, 240, 208, 225, 35, 71, 143, 29, 63, 113, 242, 212, 233, 51, 103, 207,
                    157, 191, 112, 241, 210, 229, 43, 87, 175, 93, 191, 113, 243, 214, 237, 59,
                    119, 239, 221, 127, 240, 240, 209, 227, 39, 79, 159, 61, 127, 241, 242, 213,
                    235, 55, 111, 223, 189, 255, 240, 241, 211, 231, 47, 95, 191, 125, 255, 241,
                    243, 215, 239, 63, 127, 255, 253, 15, 31, 13, 131, 209, 48, 24, 13, 131, 209,
                    48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13,
                    131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48,
                    24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131,
                    209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24,
                    13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 131, 209, 48, 24, 13, 3, 170,
                    134, 1, 0,
                ],
                (0..40)
                    .flat_map(|_| b"WWAD".iter().copied().chain(0..=255))
                    .collect(),
            ),
        ]
    }

    #[test]
    fn what_a_reference_implementation_wrote_comes_back_as_itself() {
        for (compressed, expected) in vectors() {
            let got = inflate(compressed).expect("this inflates");
            assert_eq!(
                got.len(),
                expected.len(),
                "expected {} bytes back, got {}",
                expected.len(),
                got.len()
            );
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn a_run_of_one_byte_is_a_reference_to_what_was_just_written() {
        // The case that catches a copy loop written as a bulk move: the match
        // overlaps itself.
        let [(compressed, expected), ..] = vectors();
        assert_eq!(inflate(compressed).expect("this inflates"), expected);
    }

    #[test]
    fn a_block_type_that_does_not_exist_is_named() {
        assert_eq!(inflate(&[0x07]), Err(InflateError::BadBlockType(3)));
    }
}
