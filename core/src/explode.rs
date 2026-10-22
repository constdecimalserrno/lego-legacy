//! PKWARE's Data Compression Library, exploded: what InstallShield 3 implodes
//! a title's files with (#143).
//!
//! LEGO Chess keeps every file of its game imploded in `MAIN.Z`. Written here
//! because nothing may be linked (ADR-0001), from Ben Rudiak-Gould's
//! description of the format, posted to comp.compression on 13 August 2001
//! ("Here's the PKWARE Data Compression Library format") and placed by him in
//! the public domain. The three tables below are his, code word by code word,
//! in the notation he wrote them in. His worked example has one wrong offset,
//! as Mark Adler notes in his decoder `blast`; the tests take its corrected
//! bytes from that note, and nothing else of `blast`.
//!
//! A stream is two bytes and then bits, the first of each byte its least
//! significant. The first byte says whether literals are coded, the second
//! how many low bits an offset carries plain: 4, 5 or 6, for a dictionary of
//! one, two or four kilobytes. Then each token is a bit saying which it is,
//! and a literal, or a copy of what was already written: its length coded,
//! then the top six bits of how far back it starts, coded, then the rest
//! plain. A copy 519 long is the end. Every member of both of LEGO Chess's
//! archives has plain literals and the four-kilobyte dictionary.

use std::fmt;
use std::sync::OnceLock;

/// What the stream turned out not to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExplodeError {
    /// The stream ended before its end.
    Truncated,
    /// The first byte, which says whether literals are coded, says neither.
    Literals(u8),
    /// The second byte, the dictionary's size, is not 4, 5 or 6.
    Dictionary(u8),
    /// A code in none of the format's tables.
    BadCode,
    /// A copy from before the first byte written.
    BadDistance { distance: usize, have: usize },
    /// It would expand to more than it was said to.
    TooLong { limit: usize },
}

impl fmt::Display for ExplodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "the imploded data ends mid-stream"),
            Self::Literals(byte) => write!(
                f,
                "its first byte is {byte}, where 0 says literals are plain and 1 that they are coded"
            ),
            Self::Dictionary(byte) => write!(
                f,
                "its dictionary is {byte}, where the format has 4, 5 and 6"
            ),
            Self::BadCode => write!(f, "a code that is in none of the format's tables"),
            Self::BadDistance { distance, have } => write!(
                f,
                "a copy from {distance} bytes back, with only {have} bytes written"
            ),
            Self::TooLong { limit } => {
                write!(f, "it expands past the {limit} bytes it should come to")
            }
        }
    }
}

impl std::error::Error for ExplodeError {}

/// A coded literal: the byte, in hex, and its code word, its bits in the
/// order the stream gives them.
const LITERALS: &str = "
20:1111 45:11101 61:11100 65:11011 69:11010 6c:11001
6e:11000 6f:10111 72:10110 73:10101 74:10100 75:10011
2d:100101 31:100100 41:100011 43:100010 44:100001 49:100000
4c:011111 4e:011110 4f:011101 52:011100 53:011011 54:011010
62:011001 63:011000 64:010111 66:010110 67:010101 68:010100
6d:010011 70:010010 0a:0100011 0d:0100010 28:0100001 29:0100000
2c:0011111 2e:0011110 30:0011101 32:0011100 33:0011011 34:0011010
35:0011001 37:0011000 38:0010111 3d:0010110 42:0010101 46:0010100
4d:0010011 50:0010010 55:0010001 6b:0010000 77:0001111 09:00011101
22:00011100 27:00011011 2a:00011010 2f:00011001 36:00011000 39:00010111
3a:00010110 47:00010101 48:00010100 57:00010011 5b:00010010 5f:00010001
76:00010000 78:00001111 79:00001110 2b:000011011 3e:000011010 4b:000011001
56:000011000 58:000010111 59:000010110 5d:000010101 21:0000101001 24:0000101000
26:0000100111 71:0000100110 7a:0000100101 00:00001001001 3c:00001001000 3f:00001000111
4a:00001000110 51:00001000101 5a:00001000100 5c:00001000011 6a:00001000010 7b:00001000001
7c:00001000000 01:000001111111 02:000001111110 03:000001111101 04:000001111100 05:000001111011
06:000001111010 07:000001111001 08:000001111000 0b:000001110111 0c:000001110110 0e:000001110101
0f:000001110100 10:000001110011 11:000001110010 12:000001110001 13:000001110000 14:000001101111
15:000001101110 16:000001101101 17:000001101100 18:000001101011 19:000001101010 1b:000001101001
1c:000001101000 1d:000001100111 1e:000001100110 1f:000001100101 23:000001100100 25:000001100011
3b:000001100010 40:000001100001 5e:000001100000 60:000001011111 7d:000001011110 7e:000001011101
7f:000001011100 b0:000001011011 b1:000001011010 b2:000001011001 b3:000001011000 b4:000001010111
b5:000001010110 b6:000001010101 b7:000001010100 b8:000001010011 b9:000001010010 ba:000001010001
bb:000001010000 bc:000001001111 bd:000001001110 be:000001001101 bf:000001001100 c0:000001001011
c1:000001001010 c2:000001001001 c3:000001001000 c4:000001000111 c5:000001000110 c6:000001000101
c7:000001000100 c8:000001000011 c9:000001000010 ca:000001000001 cb:000001000000 cc:000000111111
cd:000000111110 ce:000000111101 cf:000000111100 d0:000000111011 d1:000000111010 d2:000000111001
d3:000000111000 d4:000000110111 d5:000000110110 d6:000000110101 d7:000000110100 d8:000000110011
d9:000000110010 da:000000110001 db:000000110000 dc:000000101111 dd:000000101110 de:000000101101
df:000000101100 e1:000000101011 e5:000000101010 e9:000000101001 ee:000000101000 f2:000000100111
f3:000000100110 f4:000000100101 1a:0000001001001 80:0000001001000 81:0000001000111 82:0000001000110
83:0000001000101 84:0000001000100 85:0000001000011 86:0000001000010 87:0000001000001 88:0000001000000
89:0000000111111 8a:0000000111110 8b:0000000111101 8c:0000000111100 8d:0000000111011 8e:0000000111010
8f:0000000111001 90:0000000111000 91:0000000110111 92:0000000110110 93:0000000110101 94:0000000110100
95:0000000110011 96:0000000110010 97:0000000110001 98:0000000110000 99:0000000101111 9a:0000000101110
9b:0000000101101 9c:0000000101100 9d:0000000101011 9e:0000000101010 9f:0000000101001 a0:0000000101000
a1:0000000100111 a2:0000000100110 a3:0000000100101 a4:0000000100100 a5:0000000100011 a6:0000000100010
a7:0000000100001 a8:0000000100000 a9:0000000011111 aa:0000000011110 ab:0000000011101 ac:0000000011100
ad:0000000011011 ae:0000000011010 af:0000000011001 e0:0000000011000 e2:0000000010111 e3:0000000010110
e4:0000000010101 e6:0000000010100 e7:0000000010011 e8:0000000010010 ea:0000000010001 eb:0000000010000
ec:0000000001111 ed:0000000001110 ef:0000000001101 f0:0000000001100 f1:0000000001011 f5:0000000001010
f6:0000000001001 f7:0000000001000 f8:0000000000111 f9:0000000000110 fa:0000000000101 fb:0000000000100
fc:0000000000011 fd:0000000000010 fe:0000000000001 ff:0000000000000
";

/// A copy's length: the shortest length a code word stands for, and the code
/// word, each `x` one more bit after it, least significant first, to add.
const LENGTHS: &str = "
2:101 3:11 4:100 5:011
6:0101 7:0100 8:0011 9:00101
10:00100x 12:00011xx 16:00010xxx 24:000011xxxx
40:000010xxxxx 72:000001xxxxxx 136:0000001xxxxxxx 264:0000000xxxxxxxx
";

/// The top six bits of a copy's offset, in hex, and the code word.
const OFFSETS: &str = "
00:11 01:1011 02:1010 03:10011 04:10010 05:10001 06:10000 07:011111
08:011110 09:011101 0a:011100 0b:011011 0c:011010 0d:011001 0e:011000 0f:010111
10:010110 11:010101 12:010100 13:010011 14:010010 15:010001 16:0100001 17:0100000
18:0011111 19:0011110 1a:0011101 1b:0011100 1c:0011011 1d:0011010 1e:0011001 1f:0011000
20:0010111 21:0010110 22:0010101 23:0010100 24:0010011 25:0010010 26:0010001 27:0010000
28:0001111 29:0001110 2a:0001101 2b:0001100 2c:0001011 2d:0001010 2e:0001001 2f:0001000
30:00001111 31:00001110 32:00001101 33:00001100 34:00001011 35:00001010 36:00001001 37:00001000
38:00000111 39:00000110 3a:00000101 3b:00000100 3c:00000011 3d:00000010 3e:00000001 3f:00000000
";

/// The length that is not a copy but the end of the stream.
const END: usize = 519;

/// The stream's bits, least significant first.
struct Bits<'a> {
    bytes: &'a [u8],
    /// How many have been read.
    at: usize,
}

impl Bits<'_> {
    fn bit(&mut self) -> Result<usize, ExplodeError> {
        let byte = self.bytes.get(self.at / 8).ok_or(ExplodeError::Truncated)?;
        let bit = (byte >> (self.at % 8)) & 1;
        self.at += 1;
        Ok(usize::from(bit))
    }

    /// `n` bits written plain, the first of them the least significant.
    fn plain(&mut self, n: u32) -> Result<usize, ExplodeError> {
        let mut value = 0;
        for i in 0..n {
            value |= self.bit()? << i;
        }
        Ok(value)
    }
}

/// One of the format's tables: what each code word stands for, and how many
/// plain bits follow it.
///
/// A code word is kept under its bits as a number, the first bit read the
/// most significant, behind a leading 1 that tells `01` from `1`.
struct Code(Vec<Option<(u16, u32)>>);

impl Code {
    /// Read a table as it is written above: `value:word`, the value in
    /// `radix`.
    fn new(table: &str, radix: u32) -> Self {
        let entries: Vec<(usize, (u16, u32))> = table
            .split_whitespace()
            .map(|entry| {
                let (value, word) = entry.split_once(':').expect("value:word");
                let bits = word.trim_end_matches('x');
                assert!(bits.bytes().all(|b| b == b'0' || b == b'1'), "{entry}");
                let key = bits
                    .bytes()
                    .fold(1, |key, bit| (key << 1) | usize::from(bit == b'1'));
                let extra = (word.len() - bits.len()) as u32;
                let value = u16::from_str_radix(value, radix).expect("a value");
                (key, (value, extra))
            })
            .collect();
        let top = entries.iter().map(|&(key, _)| key).max().expect("a table");
        let mut words = vec![None; top + 1];
        for (key, found) in entries {
            words[key] = Some(found);
        }
        Self(words)
    }

    /// Read one code word, a bit at a time until the bits so far are one, or
    /// longer than any.
    fn read(&self, bits: &mut Bits<'_>) -> Result<(u16, u32), ExplodeError> {
        let mut key = 1;
        loop {
            key = (key << 1) | bits.bit()?;
            match self.0.get(key) {
                Some(Some(found)) => return Ok(*found),
                Some(None) => {}
                None => return Err(ExplodeError::BadCode),
            }
        }
    }
}

/// The literal, length and offset tables, read once.
fn codes() -> &'static [Code; 3] {
    static CODES: OnceLock<[Code; 3]> = OnceLock::new();
    CODES.get_or_init(|| {
        [
            Code::new(LITERALS, 16),
            Code::new(LENGTHS, 10),
            Code::new(OFFSETS, 16),
        ]
    })
}

/// Expand one imploded stream to the `expanded` bytes it should come to.
///
/// # Errors
/// When the bytes are not an imploded stream, end inside one, or would
/// expand past `expanded`.
pub fn explode(bytes: &[u8], expanded: usize) -> Result<Vec<u8>, ExplodeError> {
    let &[coded, dictionary, ..] = bytes else {
        return Err(ExplodeError::Truncated);
    };
    if coded > 1 {
        return Err(ExplodeError::Literals(coded));
    }
    if !(4..=6).contains(&dictionary) {
        return Err(ExplodeError::Dictionary(dictionary));
    }
    let [literals, lengths, offsets] = codes();
    let mut bits = Bits {
        bytes: &bytes[2..],
        at: 0,
    };
    let too_long = ExplodeError::TooLong { limit: expanded };
    // What the table says, but no more than this stream could come to at
    // most, a byte of it being nearly 200 at the longest copies: a damaged
    // size must not take the memory before a bit is read.
    let mut out = Vec::with_capacity(expanded.min(bytes.len().saturating_mul(200)));
    loop {
        if bits.bit()? == 0 {
            let literal = if coded == 1 {
                literals.read(&mut bits)?.0 as u8
            } else {
                bits.plain(8)? as u8
            };
            if out.len() == expanded {
                return Err(too_long);
            }
            out.push(literal);
            continue;
        }
        let (shortest, extra) = lengths.read(&mut bits)?;
        let length = usize::from(shortest) + bits.plain(extra)?;
        if length == END {
            return Ok(out);
        }
        // A copy two long says where it starts in fewer bits.
        let low = if length == 2 {
            2
        } else {
            u32::from(dictionary)
        };
        let top = usize::from(offsets.read(&mut bits)?.0);
        // Zero is the last byte written.
        let distance = ((top << low) | bits.plain(low)?) + 1;
        let Some(from) = out.len().checked_sub(distance) else {
            return Err(ExplodeError::BadDistance {
                distance,
                have: out.len(),
            });
        };
        if out.len() + length > expanded {
            return Err(too_long);
        }
        // A byte at a time on purpose: a copy may overlap what it writes,
        // which is how a run of one byte is written.
        for at in from..from + length {
            out.push(out[at]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{codes, explode, ExplodeError};

    /// The longest code word in any table, a literal's, without its plain
    /// bits.
    const LONGEST: u32 = 13;

    /// Bits written as the description writes them, first first, packed
    /// least significant first; spaces are for reading.
    fn pack(bits: &str) -> Vec<u8> {
        let bits: Vec<u8> = bits.bytes().filter(|b| *b != b' ').collect();
        bits.chunks(8)
            .map(|byte| {
                byte.iter()
                    .enumerate()
                    .fold(0, |acc, (i, b)| acc | (u8::from(*b == b'1') << i))
            })
            .collect()
    }

    #[test]
    fn adler_s_vector_explodes_to_what_he_says() {
        // Rudiak-Gould's example as Adler corrected it in `blast`: two plain
        // literals and a copy of eleven from two back, then the end.
        let vector = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8f, 0x80, 0x7f];
        assert_eq!(explode(&vector, 13), Ok(b"AIAIAIAIAIAIA".to_vec()));
        // And it is the description's own breakdown of it, offset corrected.
        let mut packed = vec![0x00, 0x04];
        packed.extend(pack(
            "0 10000010  0 10010010  1 001001 11 1000  1 0000000 11111111  0",
        ));
        assert_eq!(packed, vector);
    }

    #[test]
    fn coded_literals_are_read_by_the_description_s_table() {
        // No stream on the roster codes its literals, so this one is made
        // from the table: 'A' is 100011 and 'I' 100000, then the same copy.
        let mut stream = vec![0x01, 0x04];
        stream.extend(pack(
            "0 100011  0 100000  1 001001 11 1000  1 0000000 11111111",
        ));
        assert_eq!(explode(&stream, 13), Ok(b"AIAIAIAIAIAIA".to_vec()));
    }

    #[test]
    fn each_table_is_a_whole_code_for_every_symbol() {
        // Every symbol has one code word, no word begins another, and the
        // words fill the whole space of bits, which a word mistyped in a
        // table would break.
        for (code, symbols) in codes().iter().zip([256, 16, 64]) {
            let words: Vec<usize> = (0..code.0.len()).filter(|&k| code.0[k].is_some()).collect();
            assert_eq!(words.len(), symbols);
            let mut values: Vec<u16> = words.iter().map(|&k| code.0[k].unwrap().0).collect();
            values.sort_unstable();
            values.dedup();
            assert_eq!(values.len(), symbols, "no value twice");
            let mut room = 0u64;
            for &key in &words {
                let len = key.ilog2();
                room += 1 << (LONGEST - len);
                for shorter in 1..len {
                    assert!(code.0[key >> shorter].is_none(), "a word begins another");
                }
            }
            assert_eq!(room, 1 << LONGEST);
        }
    }

    #[test]
    fn a_stream_that_is_not_one_is_refused_by_its_first_two_bytes() {
        assert_eq!(explode(&[2, 4, 0], 1), Err(ExplodeError::Literals(2)));
        assert_eq!(explode(&[0, 3, 0], 1), Err(ExplodeError::Dictionary(3)));
        assert_eq!(explode(&[0, 7, 0], 1), Err(ExplodeError::Dictionary(7)));
    }

    #[test]
    fn a_stream_that_ends_early_is_refused_rather_than_guessed_at() {
        assert_eq!(explode(&[], 13), Err(ExplodeError::Truncated));
        assert_eq!(explode(&[0x00], 13), Err(ExplodeError::Truncated));
        assert_eq!(
            explode(&[0x00, 0x04, 0x82, 0x24, 0x25], 13),
            Err(ExplodeError::Truncated)
        );
    }

    #[test]
    fn a_copy_from_before_the_start_is_refused() {
        // A copy first of all: length 3 is 11, the top offset bits 0 are 11,
        // and four plain zeros, one back.
        assert_eq!(
            explode(&[0x00, 0x04, 0x1f, 0x00], 13),
            Err(ExplodeError::BadDistance {
                distance: 1,
                have: 0
            })
        );
    }

    #[test]
    fn a_stream_is_stopped_where_it_would_pass_its_size() {
        // Refused as the copy would pass it, not after it has been written:
        // a damaged member cannot grow the machine's memory past its size.
        let vector = [0x00, 0x04, 0x82, 0x24, 0x25, 0x8f, 0x80, 0x7f];
        for limit in [0, 1, 2, 12] {
            assert_eq!(
                explode(&vector, limit),
                Err(ExplodeError::TooLong { limit }),
                "{limit}"
            );
        }
    }
}
