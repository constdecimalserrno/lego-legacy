//! The AVI container: which streams a movie has, and where each one's
//! chunks are.
//!
//! Byte in, structure out. What is inside a stream is its codec's business
//! ([`crate::iv50`] for the roster's video).

use std::fmt;

/// Why a file is not a movie this reader understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AVI: {}", self.0)
    }
}

impl std::error::Error for Error {}

/// One stream of a movie.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    /// `vids` or `auds`.
    pub kind: [u8; 4],
    /// The codec, as the stream header names it.
    pub handler: [u8; 4],
    /// Time per unit is `scale / rate` seconds.
    pub scale: u32,
    pub rate: u32,
    /// How many units the stream holds: frames for video, blocks for audio.
    pub length: u32,
    /// The stream's format block: a `BITMAPINFOHEADER` for video, a
    /// `WAVEFORMATEX` for audio.
    pub format: Vec<u8>,
    /// Where each of the stream's chunks is in the file, and how long.
    pub chunks: Vec<(usize, usize)>,
}

impl Stream {
    #[must_use]
    pub fn is_video(&self) -> bool {
        &self.kind == b"vids"
    }

    #[must_use]
    pub fn is_audio(&self) -> bool {
        &self.kind == b"auds"
    }

    fn format_u32(&self, at: usize) -> u32 {
        self.format
            .get(at..at + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A video stream's picture size, from its `BITMAPINFOHEADER`.
    #[must_use]
    pub fn picture_size(&self) -> (u32, u32) {
        (self.format_u32(4), self.format_u32(8))
    }

    /// A video stream's codec, from its `BITMAPINFOHEADER`.
    #[must_use]
    pub fn compression(&self) -> [u8; 4] {
        self.format_u32(16).to_le_bytes()
    }
}

/// A parsed movie.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Avi {
    /// Microseconds per frame, as the main header declares it.
    pub micros_per_frame: u32,
    pub streams: Vec<Stream>,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read a movie's structure.
///
/// # Errors
/// When the bytes are not a RIFF AVI file.
pub fn parse(bytes: &[u8]) -> Result<Avi, Error> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"AVI ") {
        return Err(Error("not a RIFF AVI file".to_owned()));
    }
    let end = (u32_at(bytes, 4).unwrap_or(0) as usize + 8).min(bytes.len());
    let mut avi = Avi::default();
    walk(bytes, 12, end, &mut avi)?;
    if avi.streams.is_empty() {
        return Err(Error("no streams".to_owned()));
    }
    Ok(avi)
}

fn walk(bytes: &[u8], mut at: usize, end: usize, avi: &mut Avi) -> Result<(), Error> {
    while at + 8 <= end {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4).unwrap_or(0) as usize;
        let body = at + 8;
        let body_end = (body + size).min(end);
        match id {
            b"LIST" => {
                let kind = bytes.get(body..body + 4).unwrap_or(&[]);
                if kind == b"strl" {
                    avi.streams.push(Stream::default());
                }
                // `movi` and `rec ` hold chunks; everything else holds headers.
                walk(bytes, body + 4, body_end, avi)?;
            }
            b"avih" => avi.micros_per_frame = u32_at(bytes, body).unwrap_or(0),
            b"strh" => {
                let stream = avi
                    .streams
                    .last_mut()
                    .ok_or_else(|| Error("a stream header outside a stream list".to_owned()))?;
                let h = &bytes[body..body_end];
                if h.len() < 36 {
                    return Err(Error("a short stream header".to_owned()));
                }
                stream.kind.copy_from_slice(&h[0..4]);
                stream.handler.copy_from_slice(&h[4..8]);
                stream.scale = u32_at(h, 20).unwrap_or(0);
                stream.rate = u32_at(h, 24).unwrap_or(0);
                stream.length = u32_at(h, 32).unwrap_or(0);
            }
            b"strf" => {
                let stream = avi
                    .streams
                    .last_mut()
                    .ok_or_else(|| Error("a stream format outside a stream list".to_owned()))?;
                stream.format = bytes[body..body_end].to_vec();
            }
            _ => {
                // A stream's chunk is named by its two-digit number.
                if let (Some(tens), Some(ones)) = (digit(id[0]), digit(id[1])) {
                    if let Some(stream) = avi.streams.get_mut(tens * 10 + ones) {
                        stream.chunks.push((body, body_end - body));
                    }
                }
            }
        }
        at = body + size + (size & 1);
    }
    Ok(())
}

fn digit(byte: u8) -> Option<usize> {
    byte.is_ascii_digit().then(|| usize::from(byte - b'0'))
}

#[cfg(test)]
mod tests {
    use super::parse;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend_from_slice(p);
        }
        chunk(b"LIST", &body)
    }

    #[test]
    fn streams_and_their_chunks_are_found_in_file_order() {
        let mut strh = vec![0u8; 56];
        strh[0..4].copy_from_slice(b"vids");
        strh[4..8].copy_from_slice(b"IV50");
        strh[20..24].copy_from_slice(&1u32.to_le_bytes());
        strh[24..28].copy_from_slice(&15u32.to_le_bytes());
        let header = list(
            b"hdrl",
            &[
                chunk(b"avih", &66_666u32.to_le_bytes()),
                list(
                    b"strl",
                    &[chunk(b"strh", &strh), chunk(b"strf", &[1, 2, 3])],
                ),
            ],
        );
        let movi = list(b"movi", &[chunk(b"00dc", b"abc"), chunk(b"00dc", b"de")]);
        let mut riff = b"AVI ".to_vec();
        riff.extend_from_slice(&header);
        riff.extend_from_slice(&movi);
        let file = chunk(b"RIFF", &riff);

        let avi = parse(&file).expect("a well-formed file");
        assert_eq!(avi.micros_per_frame, 66_666);
        assert_eq!(avi.streams.len(), 1);
        let video = &avi.streams[0];
        assert!(video.is_video());
        assert_eq!(&video.handler, b"IV50");
        assert_eq!((video.scale, video.rate), (1, 15));
        let bodies: Vec<&[u8]> = video
            .chunks
            .iter()
            .map(|(at, len)| &file[*at..at + len])
            .collect();
        assert_eq!(bodies, [&b"abc"[..], &b"de"[..]]);
    }

    #[test]
    fn something_that_is_not_a_movie_is_refused() {
        assert!(parse(b"RIFF\\x04\\x00\\x00\\x00WAVE").is_err());
    }
}
