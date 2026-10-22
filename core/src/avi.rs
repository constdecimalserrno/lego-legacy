//! The AVI container: which streams a movie has, and where each one's
//! chunks are.
//!
//! Byte in, structure out. What is inside a stream is its codec's business
//! ([`crate::iv50`] for the roster's video).

use crate::pe::{u16_at, u32_at};
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
///
/// Its header is vfw.h's `AVIStreamHeader`, the first 56 bytes of its `strh`
/// whatever length that is written at, which is what AVIFile reads (#151).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    /// `vids` or `auds`.
    pub kind: [u8; 4],
    /// The codec that wrote it, for video.
    pub handler: [u8; 4],
    pub flags: u32,
    pub priority: u16,
    pub language: u16,
    /// How far ahead of the video an interleaved stream's chunks start.
    pub initial_frames: u32,
    /// Time per unit is `scale / rate` seconds.
    pub scale: u32,
    pub rate: u32,
    /// Where it starts and how long it is, in those units.
    pub start: u32,
    pub length: u32,
    pub suggested_buffer: u32,
    pub quality: u32,
    /// How many bytes one of its units is, for a stream of fixed units such as
    /// sound; zero for one of whole chunks.
    pub sample_size: u32,
    /// Where on the movie's picture it is drawn: left, top, right and bottom,
    /// 16 bits each as vfw.h has them.
    pub frame: [i16; 4],
    /// What its writer named it, in its `strn`, as the bytes it is written
    /// in, or nothing.
    pub name: Vec<u8>,
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
        u32_at(&self.format, at).unwrap_or(0)
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

/// What a movie's main header, its `avih`, says about the whole of it: the
/// fields of vfw.h's `MainAVIHeader` AVIFile answers with (#151).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub max_bytes_per_sec: u32,
    pub flags: u32,
    pub streams: u32,
    pub suggested_buffer: u32,
    pub width: u32,
    pub height: u32,
}

/// A parsed movie.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Avi {
    pub header: Header,
    pub streams: Vec<Stream>,
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
            b"strh" => {
                let stream = avi
                    .streams
                    .last_mut()
                    .ok_or_else(|| Error("a stream header outside a stream list".to_owned()))?;
                let h = &bytes[body..body_end];
                if h.len() < 36 {
                    return Err(Error("a short stream header".to_owned()));
                }
                let at = |offset| u32_at(h, offset).unwrap_or(0);
                let half = |offset| u16_at(h, offset).unwrap_or(0);
                stream.kind.copy_from_slice(&h[0..4]);
                stream.handler.copy_from_slice(&h[4..8]);
                stream.flags = at(8);
                (stream.priority, stream.language) = (half(12), half(14));
                stream.initial_frames = at(16);
                stream.scale = at(20);
                stream.rate = at(24);
                stream.start = at(28);
                stream.length = at(32);
                stream.suggested_buffer = at(36);
                stream.quality = at(40);
                stream.sample_size = at(44);
                stream.frame = std::array::from_fn(|i| half(48 + 2 * i) as i16);
            }
            b"strn" => {
                if let Some(stream) = avi.streams.last_mut() {
                    let name = &bytes[body..body_end];
                    let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
                    stream.name = name[..end].to_vec();
                }
            }
            b"avih" => {
                let at = |offset| u32_at(&bytes[body..body_end], offset).unwrap_or(0);
                avi.header = Header {
                    max_bytes_per_sec: at(4),
                    flags: at(12),
                    streams: at(24),
                    suggested_buffer: at(28),
                    width: at(32),
                    height: at(36),
                };
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
        assert_eq!(avi.streams.len(), 1);
        let video = &avi.streams[0];
        assert!(video.is_video());
        assert_eq!((video.scale, video.rate), (1, 15));
        let bodies: Vec<&[u8]> = video
            .chunks
            .iter()
            .map(|(at, len)| &file[*at..at + len])
            .collect();
        assert_eq!(bodies, [&b"abc"[..], &b"de"[..]]);
    }

    /// What AVIFile answers about a movie is its headers' (#151): the main
    /// header, each stream header as vfw.h lays it out, its edges 16-bit
    /// whatever length the header is written at, and the stream's name.
    #[test]
    fn the_headers_and_the_stream_names_are_read_as_vfw_h_lays_them_out() {
        let words =
            |values: &[u32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
        // MainAVIHeader: microseconds a frame, the most bytes a second,
        // padding, flags, frames, initial frames, streams, buffer, width,
        // height, and four reserved.
        let avih = words(&[
            66_667, 774_506, 0, 0x910, 280, 11, 2, 55_300, 320, 240, 0, 0, 0, 0,
        ]);
        // AVIStreamHeader: type, handler, flags, priority and language, initial
        // frames, scale, rate, start, length, buffer, quality, sample size,
        // then the frame as four 16-bit edges.
        let mut video = b"vidsIV50".to_vec();
        video.extend(words(&[
            0x1,
            0x0002_0001,
            0,
            1,
            15,
            3,
            280,
            52_184,
            10_000,
            0,
        ]));
        video.extend_from_slice(&[0, 0, 0, 0, 0x40, 1, 0xf0, 0]);
        // A sound's, written 64 bytes long as some writers write it, with
        // four 32-bit edges, which read as 16-bit ones are nothing.
        let mut sound = b"auds\0\0\0\0".to_vec();
        sound.extend(words(&[
            0, 0, 11, 1, 22_050, 0, 411_885, 1_477, 0, 1, 0, 0, 0, 0,
        ]));
        let header = list(
            b"hdrl",
            &[
                chunk(b"avih", &avih),
                list(
                    b"strl",
                    &[
                        chunk(b"strh", &video),
                        chunk(b"strf", &[0; 40]),
                        chunk(b"strn", b"LMI.avi Video #1\0"),
                    ],
                ),
                list(b"strl", &[chunk(b"strh", &sound), chunk(b"strf", &[0; 16])]),
            ],
        );
        let mut riff = b"AVI ".to_vec();
        riff.extend_from_slice(&header);
        riff.extend_from_slice(&list(b"movi", &[]));
        let avi = parse(&chunk(b"RIFF", &riff)).expect("a well-formed file");

        let h = &avi.header;
        assert_eq!(
            (h.max_bytes_per_sec, h.flags, h.streams),
            (774_506, 0x910, 2)
        );
        assert_eq!((h.suggested_buffer, h.width, h.height), (55_300, 320, 240));
        let video = &avi.streams[0];
        assert_eq!(&video.handler, b"IV50");
        assert_eq!((video.flags, video.priority, video.language), (1, 1, 2));
        assert_eq!(
            (video.initial_frames, video.start, video.length),
            (0, 3, 280)
        );
        assert_eq!(
            (video.suggested_buffer, video.quality, video.sample_size),
            (52_184, 10_000, 0)
        );
        assert_eq!(video.frame, [0, 0, 320, 240]);
        assert_eq!(video.name, b"LMI.avi Video #1");
        let sound = &avi.streams[1];
        assert_eq!(
            (
                sound.initial_frames,
                sound.rate,
                sound.length,
                sound.sample_size
            ),
            (11, 22_050, 411_885, 1)
        );
        assert_eq!(sound.frame, [0; 4]);
        assert!(sound.name.is_empty());
    }

    #[test]
    fn something_that_is_not_a_movie_is_refused() {
        assert!(parse(b"RIFF\\x04\\x00\\x00\\x00WAVE").is_err());
    }
}
