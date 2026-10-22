//! Video for Windows: the AVIFile library a title reads its movies through
//! by file, and the DrawDib it draws their frames with.
//!
//! Rock Raiders initialises it and plays its movies through DirectShow's
//! multimedia streams instead ([`super::amstream`], #9). LEGO Chess plays
//! all of its movies through its C API (#151): it opens a file, walks its
//! streams and reads their headers and formats, and takes each frame as a
//! DIB, which its DirectX plugin draws with DrawDib on its back buffer's
//! device context. The file is read by [`crate::avi`], its pictures by
//! [`crate::iv50`].
//! Its handles are the COM objects Windows hands out, with
//! every method of their own tables a wall, so a title that calls through
//! one stops at a name; the C API is what is written. Every other entry
//! point is an import nobody has written, and calling one is a wall that
//! names it.

use super::{stdcall, ComState, Ctx, Shim, State};
use crate::com::{wall, Interface};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("AVIFIL32", "AVIFileInit", 0, init),
    stdcall("AVIFIL32", "AVIFileOpenA", 4, open),
    stdcall("AVIFIL32", "AVIFileInfoA", 3, file_info),
    stdcall("AVIFIL32", "AVIFileGetStream", 4, get_stream),
    stdcall("AVIFIL32", "AVIStreamInfoA", 3, stream_info),
    stdcall("AVIFIL32", "AVIStreamAddRef", 1, add_ref),
    stdcall("AVIFIL32", "AVIStreamReadFormat", 4, read_format),
    stdcall("AVIFIL32", "AVIStreamGetFrameOpen", 2, get_frame_open),
    stdcall("AVIFIL32", "AVIStreamGetFrame", 2, get_frame),
    stdcall("MSVFW32", "DrawDibOpen", 0, draw_dib_open),
    stdcall("MSVFW32", "DrawDibDraw", 13, draw_dib_draw),
    stdcall("MSVFW32", "DrawDibClose", 1, draw_dib_close),
];

/// vfw.h's answers: `MAKE_AVIERR(n)`, `0x80044000 + n`.
const AVIERR_OK: u32 = 0;
const AVIERR_BADPARAM: u32 = 0x8004_406a;
const AVIERR_BADSIZE: u32 = 0x8004_406b;
const AVIERR_BADHANDLE: u32 = 0x8004_406c;
const AVIERR_FILEOPEN: u32 = 0x8004_406f;
const AVIERR_NODATA: u32 = 0x8004_4073;
const AVIERR_BUFFERTOOSMALL: u32 = 0x8004_4074;

/// `AVIFILECAPS_CANREAD | AVIFILECAPS_CANWRITE`: what the AVI file handler
/// can do, file and stream alike, as Wine's `avifile.c`, read as
/// documentation, says it.
const CAPS: u32 = 0x3;

/// How long an `AVIFILEINFOA` and an `AVISTREAMINFOA` are.
const FILE_INFO_BYTES: u32 = 108;
const STREAM_INFO_BYTES: u32 = 140;

/// A movie a title opened, held while it is open: its bytes, which every open
/// of the same file shares, its structure, and the streams handed out of it.
pub struct File {
    name: String,
    bytes: std::rc::Rc<[u8]>,
    avi: crate::avi::Avi,
    /// Each stream's object, once `AVIFileGetStream` has handed it out: the
    /// same one every time, as AVIFile hands out the one stream.
    streams: Vec<Option<u32>>,
}

pub static AVI_FILE: Interface = Interface {
    name: "IAVIFile",
    methods: &[
        wall("QueryInterface", 2),
        wall("AddRef", 0),
        wall("Release", 0),
        wall("Info", 2),
        wall("GetStream", 3),
        wall("CreateStream", 2),
        wall("WriteData", 3),
        wall("ReadData", 3),
        wall("EndRecord", 0),
        wall("DeleteStream", 2),
    ],
};

pub static AVI_STREAM: Interface = Interface {
    name: "IAVIStream",
    methods: &[
        wall("QueryInterface", 2),
        wall("AddRef", 0),
        wall("Release", 0),
        wall("Create", 2),
        wall("Info", 2),
        wall("FindSample", 2),
        wall("ReadFormat", 3),
        wall("SetFormat", 3),
        wall("Read", 6),
        wall("Write", 7),
        wall("Delete", 2),
        wall("ReadData", 3),
        wall("WriteData", 3),
        wall("SetInfo", 2),
    ],
};

pub static GET_FRAME: Interface = Interface {
    name: "IGetFrame",
    methods: &[
        wall("QueryInterface", 2),
        wall("AddRef", 0),
        wall("Release", 0),
        wall("GetFrame", 1),
        wall("Begin", 3),
        wall("End", 0),
        wall("SetFormat", 6),
    ],
};

pub static INTERFACES: &[&Interface] = &[&AVI_FILE, &AVI_STREAM, &GET_FRAME];

/// What `AVIStreamGetFrameOpen` made: the stream it decodes, its picture so
/// far, and the DIB in guest memory each frame is handed back in.
pub struct Getter {
    file: u32,
    stream: usize,
    /// Where the DIB is: its header, then its bits.
    dib: u32,
    /// Sixteen bits a pixel, which is 555, or 24.
    bits: u16,
    decoder: crate::iv50::Decoder,
    picture: crate::iv50::Picture,
    /// The last frame decoded into the picture, which the DIB holds.
    decoded: Option<usize>,
}

fn init(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

/// What a call was handed that is not an object of the kind it takes, and so
/// not one this machine gave out: a wall that names it.
fn stranger(call: &str, what: &str, at: u32) -> Stop {
    Stop::NotWritten {
        what: format!("AVIFIL32.dll!{call}: {at:#x} is not {what} this machine opened"),
        caller: 0,
    }
}

fn file<'a>(ctx: &'a Ctx<'_>, call: &str, at: u32) -> Result<&'a File, Stop> {
    ctx.state
        .avi_files
        .get(&at)
        .ok_or_else(|| stranger(call, "a movie", at))
}

/// Which movie, by its file object, and which of its streams a stream
/// object is.
fn stream_of(ctx: &Ctx<'_>, call: &str, at: u32) -> Result<(u32, usize), Stop> {
    match ctx.state.com(at).map(|o| &o.state) {
        Some(&ComState::AviStream { file, stream }) if ctx.state.avi_files.contains_key(&file) => {
            Ok((file, stream))
        }
        _ => Err(stranger(call, "a stream of a movie", at)),
    }
}

fn open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIFileOpenA(file out, name, mode, handler).
    const OF_READ: u32 = 0;
    let (out, name, mode, handler) = (args[0], args[1], args[2], args[3]);
    if out == 0 || name == 0 {
        return Ok(AVIERR_BADPARAM);
    }
    let name = ctx.string(name).map_err(Stop::Fault)?;
    // Reading, through the handler the name's extension picks, is the one
    // open this machine makes.
    if mode != OF_READ || handler != 0 {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIFileOpenA: {name} in mode {mode:#x} through handler {handler:#x}"
            ),
            caller: 0,
        });
    }
    ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
    // A file that is not there, or a drive with no disk in it, is a file
    // the handler could not open, which the title can take: LEGO Chess skips
    // the movie (0x0044AFE4).
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(AVIERR_FILEOPEN);
    }
    // ponytail: one copy of each movie file is held for the run, shared by
    // every open of it, as LEGO Chess never lets go of a file (its close
    // releases the streams and not the file): its 22 movies come to 172 MB in
    // a run that plays them all. Let a copy go once no open file holds it,
    // when `AVIFileRelease` is written.
    let path = ctx.absolute(&name).to_uppercase();
    let bytes: std::rc::Rc<[u8]> = if let Some(bytes) = ctx.state.movie_bytes.get(&path) {
        bytes.clone()
    } else {
        let Some(bytes) = ctx.open_guest_file(&name) else {
            ctx.log.line(&format!("movie {name} missing"));
            return Ok(AVIERR_FILEOPEN);
        };
        bytes.into()
    };
    // One that is there and is not a movie is a damaged rip, which stops the
    // run at its name as a damaged archive member does (#169, #190).
    let mut avi = crate::avi::parse(&bytes).map_err(|e| Stop::NotWritten {
        what: format!("AVIFIL32.dll!AVIFileOpenA: {name}: {e}"),
        caller: 0,
    })?;
    // So does one that holds fewer streams than its header says, which Wine's
    // `avifile.c` fails to load. One that holds more is read as Wine reads
    // it: the streams its header counts, the rest kept as chunks nobody asks
    // for.
    let counted = avi.header.streams as usize;
    if avi.streams.len() < counted {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIFileOpenA: {name}: its header says {counted} streams and it holds \
                 {}",
                avi.streams.len()
            ),
            caller: 0,
        });
    }
    avi.streams.truncate(counted);
    name_streams(&mut avi, &name);
    // Only a movie that opens is kept.
    ctx.state
        .movie_bytes
        .entry(path)
        .or_insert_with(|| bytes.clone());
    let object = State::make_com(ctx, &AVI_FILE, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "movie {name} opened through AVIFile: {} streams",
        avi.streams.len()
    ));
    let streams = vec![None; avi.streams.len()];
    ctx.state.avi_files.insert(
        object,
        File {
            name,
            bytes,
            avi,
            streams,
        },
    );
    Ok(AVIERR_OK)
}

/// Name each stream its writer did not as AVIFile names it: the file's name,
/// its kind and how many of that kind it is, counted from one. The evidence
/// is what AVIFile itself named streams on its authors' machines, as two of
/// LEGO Chess's movies kept the names in their `strn`s: `LMI.avi Video #1`,
/// `LMI.avi Audio #1` and `WL1lose.avi Audio #1`. Wine's `avifile.c` shares
/// only the pattern: it writes "video" and "audio", and counts from 0.
fn name_streams(avi: &mut crate::avi::Avi, path: &str) {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let (mut videos, mut sounds) = (0, 0);
    for s in &mut avi.streams {
        let (kind, n) = if s.is_video() {
            videos += 1;
            ("Video", videos)
        } else if s.is_audio() {
            sounds += 1;
            ("Audio", sounds)
        } else {
            // ponytail: a stream that is neither keeps no name; Wine names
            // one by its type, and nothing on the roster has one.
            continue;
        };
        if s.name.is_empty() {
            s.name = format!("{file} {kind} #{n}").into_bytes();
        }
    }
}

/// Write little-endian words into a structure, at their offsets.
fn put(info: &mut [u8], fields: &[(usize, u32)]) {
    for (at, value) in fields {
        info[*at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn file_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIFileInfoA(file, info, size): an AVIFILEINFOA, its flags and stream
    // count the main header's and the rest worked out from the streams as
    // Wine's `AVIFILE_UpdateInfo`, read as documentation, works them out on
    // every call: the time scale the first stream's, the length the longest
    // stream's in the first's units, the most bytes a second the streams'
    // added up, the buffer the biggest stream's, and the size the biggest of
    // the video's frames.
    let (at, size) = (args[1], args[2]);
    if args[0] == 0 {
        return Ok(AVIERR_BADHANDLE);
    }
    if size < FILE_INFO_BYTES {
        return Ok(AVIERR_BADSIZE);
    }
    let file = file(ctx, "AVIFileInfoA", args[0])?;
    let (h, streams) = (&file.avi.header, &file.avi.streams);
    let length = streams.first().map_or(0, |first| {
        streams[1..]
            .iter()
            .map(|s| time_to_sample(first, sample_to_time(s, s.length)))
            .fold(first.length, u32::max)
    });
    let (mut bytes_a_second, mut buffer, mut width, mut height) = (0u32, 0, 0, 0);
    for s in streams {
        let per = if s.sample_bytes() == 0 {
            let [left, top, right, bottom] = frame(s);
            width = width.max(right.wrapping_sub(left) as u32);
            height = height.max(bottom.wrapping_sub(top) as u32);
            largest_chunk(s)
        } else {
            s.sample_bytes()
        };
        bytes_a_second = bytes_a_second.wrapping_add(mul_div(per, s.rate, s.scale));
        buffer = buffer.max(largest_chunk(s));
    }
    let (scale, rate) = streams.first().map_or((0, 0), |s| (s.scale, s.rate));
    // ponytail: szFileType, the handler's own description, is left empty:
    // what Windows' handler calls itself is not documented, and nothing on
    // the roster reads it.
    let mut info = [0u8; FILE_INFO_BYTES as usize];
    put(
        &mut info,
        &[
            (0, bytes_a_second),
            (4, h.flags),
            (8, CAPS),
            (12, h.streams),
            (16, buffer),
            (20, width),
            (24, height),
            (28, scale),
            (32, rate),
            (36, length),
        ],
    );
    ctx.mem.write(at, &info).map_err(Stop::Fault)?;
    Ok(AVIERR_OK)
}

/// What Win32's `MulDiv` answers for counts that are not negative: `a * b /
/// c` rounded to the nearest, and -1 for a `c` of zero.
fn mul_div(a: u32, b: u32, c: u32) -> u32 {
    if c == 0 {
        return u32::MAX;
    }
    let (a, b, c) = (u64::from(a), u64::from(b), u64::from(c));
    u32::try_from((a * b + c / 2) / c).unwrap_or(u32::MAX)
}

/// A stream's sample as milliseconds, as Wine's `AVIStreamSampleToTime`
/// works it out, in single precision: clamped to the stream, truncated, or
/// rounded up for a rate of a thousand a second or more; -1 for none.
fn sample_to_time(s: &crate::avi::Stream, sample: u32) -> i32 {
    if s.rate == 0 || s.scale == 0 {
        return -1;
    }
    let sample = sample.clamp(s.start, s.start.wrapping_add(s.length));
    let ms = sample as f32 * s.scale as f32 * 1000.0;
    if s.rate / s.scale < 1000 {
        (ms / s.rate as f32) as i32
    } else {
        ((ms + (s.rate - 1) as f32) / s.rate as f32) as i32
    }
}

/// Milliseconds as a stream's sample, as Wine's `AVIStreamTimeToSample` works
/// it out: the other way, then clamped to the stream, a sample of -1 being
/// past its end.
fn time_to_sample(s: &crate::avi::Stream, ms: i32) -> u32 {
    if ms < 0 || s.scale == 0 {
        return u32::MAX;
    }
    let per = s.scale as f32 * 1000.0;
    let sample = if s.rate / s.scale < 1000 {
        (s.rate as f32 * ms as f32 / per) as i32
    } else {
        ((s.rate as f32 * ms as f32 + (s.scale * 1000 - 1) as f32) / per) as i32
    } as u32;
    sample.clamp(s.start, s.start.wrapping_add(s.length))
}

/// A stream's biggest chunk: the buffer Windows suggests for reading it, as
/// Wine works it out from the index, and 0 for a stream with none (Wine's
/// conformance test `test_default_data`, which passes on Windows).
fn largest_chunk(s: &crate::avi::Stream) -> u32 {
    s.chunks
        .iter()
        .map(|&(_, len)| u32::try_from(len).unwrap_or(u32::MAX))
        .max()
        .unwrap_or(0)
}

/// Where on the picture a stream is drawn. A video stream's empty frame is its
/// picture, as Wine's `avifile.c`, read as documentation, fills it: twenty of
/// LEGO Chess's twenty-two movies write theirs as 32-bit edges, which as
/// vfw.h's 16-bit ones are empty, and without it the game would size them
/// 0x0.
fn frame(s: &crate::avi::Stream) -> [i32; 4] {
    let [left, top, right, bottom] = s.frame.map(i32::from);
    if s.is_video() && (right <= left || bottom <= top) {
        let (width, height) = s.picture_size();
        [0, 0, width as i32, height as i32]
    } else {
        [left, top, right, bottom]
    }
}

fn get_stream(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIFileGetStream(file, stream out, type, n): the nth stream of that
    // type, or the nth of all of them for type 0, with a reference more.
    let (at, out, kind, n) = (args[0], args[1], args[2], args[3]);
    if at == 0 {
        return Ok(AVIERR_BADHANDLE);
    }
    if out == 0 || (n as i32) < 0 {
        return Ok(AVIERR_BADPARAM);
    }
    let file = file(ctx, "AVIFileGetStream", at)?;
    let found = file
        .avi
        .streams
        .iter()
        .enumerate()
        .filter(|(_, s)| kind == 0 || u32::from_le_bytes(s.kind) == kind)
        .nth(n as usize)
        .map(|(i, _)| (i, file.streams[i]));
    let object = match found {
        None => 0,
        Some((_, Some(object))) => {
            ctx.state.com_ref(object, 1);
            object
        }
        Some((index, None)) => {
            let object = State::make_com(
                ctx,
                &AVI_STREAM,
                ComState::AviStream {
                    file: at,
                    stream: index,
                },
            )
            .map_err(Stop::Fault)?;
            if let Some(file) = ctx.state.avi_files.get_mut(&at) {
                file.streams[index] = Some(object);
            }
            object
        }
    };
    ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
    Ok(if object == 0 {
        AVIERR_NODATA
    } else {
        AVIERR_OK
    })
}

fn stream_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIStreamInfoA(stream, info, size): an AVISTREAMINFOA from the stream's
    // header and name, with three things as Windows answers them rather than
    // as the header writes them: its frame (`frame`), its buffer, its biggest
    // chunk (`largest_chunk`), and a sound's sample size, its format's block
    // (`Stream::sample_bytes`). A sound's time scale is its header's: Wine
    // makes it one sample of its format, which no conformance test observes
    // Windows doing.
    let (at, size) = (args[1], args[2]);
    if args[0] == 0 {
        return Ok(AVIERR_BADHANDLE);
    }
    if size < STREAM_INFO_BYTES {
        return Ok(AVIERR_BADSIZE);
    }
    let (owner, index) = stream_of(ctx, "AVIStreamInfoA", args[0])?;
    let s = &ctx.state.avi_files[&owner].avi.streams[index];
    let frame = frame(s);
    let mut info = [0u8; STREAM_INFO_BYTES as usize];
    put(
        &mut info,
        &[
            (0, u32::from_le_bytes(s.kind)),
            (4, u32::from_le_bytes(s.handler)),
            (8, s.flags),
            (12, CAPS),
            (16, u32::from(s.priority) | u32::from(s.language) << 16),
            (20, s.scale),
            (24, s.rate),
            (28, s.start),
            (32, s.length),
            (36, s.initial_frames),
            (40, largest_chunk(s)),
            (44, s.quality),
            (48, s.sample_bytes()),
            (52, frame[0] as u32),
            (56, frame[1] as u32),
            (60, frame[2] as u32),
            (64, frame[3] as u32),
        ],
    );
    let take = s.name.len().min(63);
    info[76..76 + take].copy_from_slice(&s.name[..take]);
    ctx.mem.write(at, &info).map_err(Stop::Fault)?;
    Ok(AVIERR_OK)
}

fn add_ref(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIStreamAddRef(stream): the references it has now.
    if args[0] == 0 {
        return Ok(0);
    }
    stream_of(ctx, "AVIStreamAddRef", args[0])?;
    Ok(ctx.state.com_ref(args[0], 1))
}

fn read_format(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIStreamReadFormat(stream, position, format, size): how long the
    // format is, asked with no buffer or no room, or as much of it as fits,
    // with too small said when that is not all of it (Wine's `avifile.c`).
    let (at, size_at) = (args[2], args[3]);
    if args[0] == 0 {
        return Ok(AVIERR_BADHANDLE);
    }
    if size_at == 0 {
        return Ok(AVIERR_BADPARAM);
    }
    let (owner, index) = stream_of(ctx, "AVIStreamReadFormat", args[0])?;
    let room = ctx.mem.read_u32(size_at).map_err(Stop::Fault)? as i32;
    let format = &ctx.state.avi_files[&owner].avi.streams[index].format;
    let whole = u32::try_from(format.len()).unwrap_or(u32::MAX);
    ctx.mem.write_u32(size_at, whole).map_err(Stop::Fault)?;
    if at == 0 || room <= 0 {
        return Ok(AVIERR_OK);
    }
    let take = format.len().min(room as usize);
    ctx.mem.write(at, &format[..take]).map_err(Stop::Fault)?;
    Ok(if take < format.len() {
        AVIERR_BUFFERTOOSMALL
    } else {
        AVIERR_OK
    })
}

/// `BI_RGB`: a DIB of plain pixels, through no codec.
const BI_RGB: u32 = 0;
/// How long a `BITMAPINFOHEADER` is.
const DIB_HEADER_BYTES: u32 = 40;

fn get_frame_open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIStreamGetFrameOpen(stream, wanted): a getter of the stream's frames
    // as DIBs in the wanted format, or none for a stream that is not video,
    // as Wine's `getframe.c`, read as documentation, refuses one. The DIB is
    // the wanted header, its image size filled in when it is zero, and the
    // bits after it, in memory the getter keeps (`getframe.c` again).
    let (at, wanted) = (args[0], args[1]);
    let (owner, index) = stream_of(ctx, "AVIStreamGetFrameOpen", at)?;
    let file = &ctx.state.avi_files[&owner];
    let s = &file.avi.streams[index];
    if !s.is_video() {
        return Ok(0);
    }
    let name = file.name.clone();
    let codec = s.compression();
    let (width, height) = s.picture_size();
    if !codec.eq_ignore_ascii_case(b"IV50") {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIStreamGetFrameOpen: {name} is {}, which nothing here decodes",
                String::from_utf8_lossy(&codec)
            ),
            caller: 0,
        });
    }
    // The decompressor's own format, asked for by none or by
    // AVIGETFRAMEF_BESTDISPLAYFMT, is not one this machine chooses.
    if wanted <= 1 {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIStreamGetFrameOpen: {name} in the decompressor's own format"
            ),
            caller: 0,
        });
    }
    let mut header = ctx
        .mem
        .slice(wanted, DIB_HEADER_BYTES as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    let word = |at: usize| {
        u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    let bits = u16::from_le_bytes([header[14], header[15]]);
    // A plain DIB the movie's own size, of one plane, bottom-up, at 16 or 24
    // bits and with no colour table, is what is written: what LEGO Chess asks
    // for, 16 bits for its movies (0x0044B2F2) and 24 for its Advisors (#152,
    // set at 0x004010FD before their open at 0x00401131).
    let plain = word(0) == DIB_HEADER_BYTES
        && word(12) & 0xffff == 1
        && word(16) == BI_RGB
        && matches!(bits, 16 | 24)
        && (word(4), word(8)) == (width, height)
        && word(32) == 0;
    if !plain {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIStreamGetFrameOpen: {name} as a {bits}-bit DIB of \
                 compression {:#x} in {} planes, {}x{} with {} colours, from a \
                 {width}x{height} movie",
                word(16),
                word(12) & 0xffff,
                word(4) as i32,
                word(8) as i32,
                word(32)
            ),
            caller: 0,
        });
    }
    // A picture too big for memory to hold, which only a damaged rip's header
    // could say, stops the run at its size, as a damaged archive member does.
    let too_big = || Stop::NotWritten {
        what: format!(
            "AVIFIL32.dll!AVIStreamGetFrameOpen: {name} is a {width}x{height} picture, more than \
             memory holds as a {bits}-bit DIB"
        ),
        caller: 0,
    };
    let image = width
        .checked_mul(u32::from(bits) / 8)
        .and_then(|row| row.checked_next_multiple_of(4))
        .and_then(|stride| stride.checked_mul(height))
        .filter(|image| image.checked_add(DIB_HEADER_BYTES).is_some())
        .ok_or_else(too_big)?;
    if word(20) == 0 {
        header[20..24].copy_from_slice(&image.to_le_bytes());
    }
    let dib = ctx.state.heap.alloc(DIB_HEADER_BYTES + image);
    if dib == 0 {
        return Err(too_big());
    }
    ctx.mem.write(dib, &header).map_err(Stop::Fault)?;
    let object = State::make_com(ctx, &GET_FRAME, ComState::None).map_err(Stop::Fault)?;
    ctx.state.frame_getters.insert(
        object,
        Getter {
            file: owner,
            stream: index,
            dib,
            bits,
            decoder: crate::iv50::Decoder::new(),
            picture: crate::iv50::Picture::default(),
            decoded: None,
        },
    );
    ctx.log.line(&format!(
        "movie {name} frames as {bits}-bit DIBs at {dib:#010x}"
    ));
    Ok(object)
}

fn get_frame(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AVIStreamGetFrame(getter, position): the DIB, with that frame in it.
    // The pictures are decoded in order, a null one keeping the last; the
    // same position again is the DIB as it is; one before the stream's start
    // or past its frames is none, as Wine's `getframe.c` answers, a position
    // being the chunk of that number.
    let (at, position) = (args[0], args[1] as i32);
    if at == 0 {
        return Ok(0);
    }
    let state = &mut *ctx.state;
    let Some(getter) = state.frame_getters.get_mut(&at) else {
        return Err(stranger("AVIStreamGetFrame", "a frame getter", at));
    };
    let file = &state.avi_files[&getter.file];
    let s = &file.avi.streams[getter.stream];
    let chunks = &s.chunks;
    let Ok(position) = usize::try_from(position) else {
        return Ok(0);
    };
    if position < s.start as usize || position >= chunks.len() {
        return Ok(0);
    }
    if getter.decoded == Some(position) {
        return Ok(getter.dib);
    }
    // ponytail: a frame before the last one decoded starts again from the
    // first. #152's Advisors loop back; decoding on from the nearest key
    // frame `idx1` marks is the way past.
    if getter.decoded.is_some_and(|last| position < last) {
        getter.decoder = crate::iv50::Decoder::new();
        getter.picture = crate::iv50::Picture::default();
        getter.decoded = None;
    }
    let from = getter.decoded.map_or(0, |last| last + 1);
    for (n, &(start, len)) in chunks.iter().enumerate().take(position + 1).skip(from) {
        // An empty chunk is a dropped frame, which repeats the last.
        if len > 0 {
            getter
                .decoder
                .decode(&file.bytes[start..start + len], &mut getter.picture)
                .map_err(|e| Stop::NotWritten {
                    what: format!(
                        "AVIFIL32.dll!AVIStreamGetFrame: frame {n} of {}: {e}",
                        file.name
                    ),
                    caller: 0,
                })?;
        }
        getter.decoded = Some(n);
    }
    let (width, height) = s.picture_size();
    let picture = &getter.picture;
    if (picture.width, picture.height) != (width as usize, height as usize) {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIStreamGetFrame: frame {position} of {} is {}x{} in a {width}x{height} movie",
                file.name, picture.width, picture.height
            ),
            caller: 0,
        });
    }
    let bits = dib_bits(picture, getter.bits);
    let dib = getter.dib;
    ctx.mem
        .write(dib + DIB_HEADER_BYTES, &bits)
        .map_err(Stop::Fault)?;
    Ok(dib)
}

/// A picture as the bits of a bottom-up DIB of plain pixels, its rows each
/// padded to a whole number of words: 555 at 16 bits, which is a 16-bit
/// `BI_RGB` pixel by Microsoft's `BITMAPINFOHEADER` page, each channel
/// narrowed by dropping its low bits; or blue, green and red at 24.
fn dib_bits(picture: &crate::iv50::Picture, bits: u16) -> Vec<u8> {
    const RGB555: crate::raster::PixelFormat = crate::raster::PixelFormat {
        bits: 16,
        masks: [0x7c00, 0x03e0, 0x001f, 0],
    };
    let bytes = usize::from(bits / 8);
    let stride = (picture.width * bytes).next_multiple_of(4);
    let mut out = vec![0u8; stride * picture.height];
    for y in 0..picture.height {
        let row = (picture.height - 1 - y) * stride;
        let row = &mut out[row..row + picture.width * bytes];
        for (x, pixel) in row.chunks_exact_mut(bytes).enumerate() {
            let (r, g, b) = picture.rgb(x, y);
            if bytes == 2 {
                let value = RGB555.encode([r, g, b, 0]) as u16;
                pixel.copy_from_slice(&value.to_le_bytes());
            } else {
                pixel.copy_from_slice(&[b, g, r]);
            }
        }
    }
    out
}

fn draw_dib_open(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // DrawDibOpen(): a handle to draw with, counted up from one past any still
    // open, as Wine's `drawdib.c`, read as documentation, counts them, so a
    // handle let go of is not handed out again and drawing with it is
    // refused. They are DrawDib's own, not the machine's handles: LEGO Chess
    // opens one for each frame it draws.
    while ctx.state.drawdibs.contains(&ctx.state.next_drawdib) {
        ctx.state.next_drawdib = ctx.state.next_drawdib.wrapping_add(1);
    }
    let handle = ctx.state.next_drawdib;
    ctx.state.next_drawdib = handle.wrapping_add(1);
    ctx.state.drawdibs.insert(handle);
    Ok(handle)
}

fn draw_dib_close(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DrawDibClose(handle): TRUE, or FALSE for a handle it never gave (Wine).
    Ok(u32::from(ctx.state.drawdibs.remove(&args[0])))
}

fn draw_dib_draw(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DrawDibDraw(handle, dc, x, y, width, height, header, bits, source x,
    // source y, source width, source height, flags): a plain bottom-up DIB of
    // 16 bits a pixel drawn whole onto a surface's DC, each pixel repeated as
    // many times over as the destination is the DIB's size, and what falls off
    // the surface left out. FALSE for a handle it never gave or has let go of
    // (Wine's `drawdib.c`, read as documentation), and for a DIB that is
    // top-down or empty: Microsoft's DrawDibDraw page says its height must be
    // positive, as DrawDibDraw "will not draw inverted DIBs", and Wine's
    // conformance test `test_DrawDib_sizeimage`, which passes on Windows,
    // finds an empty one refused too. Bits of none are the ones after the
    // header, as Wine reads them. Each channel is widened by repeating its top
    // bits and then narrowed to the surface's, as Wine's DIB engine does,
    // which gives a 555 DIB's green its sixth bit on a 565 surface from its
    // own top one. A flag, a part of a DIB, a scale that is not whole, a
    // codec, a colour table, another depth or an indexed surface is a wall:
    // LEGO Chess draws its movies' 16-bit frames whole and doubled, and its
    // Advisors read their 24-bit ones without DrawDib.
    let [hdd, dc, x, y, width, height, header, bits, sx, sy, sw, sh, flags] = args else {
        unreachable!("DrawDibDraw takes thirteen arguments");
    };
    if !ctx.state.drawdibs.contains(hdd) {
        return Ok(0);
    }
    let wall = |what: String| Stop::NotWritten {
        what: format!("MSVFW32.dll!DrawDibDraw: {what}"),
        caller: 0,
    };
    if *flags != 0 {
        return Err(wall(format!("flags {flags:#x}")));
    }
    let Some(&surface) = ctx.state.surface_dcs.get(dc) else {
        return Err(wall(format!("{dc:#x}, which is no surface's DC")));
    };
    let read = |at: u32| ctx.mem.read_u32(header + at).map_err(Stop::Fault);
    let (size, dib_width, dib_height) = (read(0)?, read(4)? as i32, read(8)? as i32);
    let (depth, compression, colours) = (read(12)? >> 16, read(16)?, read(32)?);
    if dib_width <= 0 || dib_height <= 0 {
        return Ok(0);
    }
    if compression != BI_RGB || depth != 16 || colours != 0 {
        return Err(wall(format!(
            "a {depth}-bit DIB of compression {compression:#x} with {colours} colours"
        )));
    }
    let signed = |v: u32| i64::from(v as i32);
    let dib = (i64::from(dib_width), i64::from(dib_height));
    let (sw, sh, width, height) = (signed(*sw), signed(*sh), signed(*width), signed(*height));
    let (x, y) = (signed(*x), signed(*y));
    // The source is the whole DIB, so not empty, before it divides anything.
    if (*sx, *sy) != (0, 0)
        || (sw, sh) != dib
        || width <= 0
        || height <= 0
        || width % sw != 0
        || height % sh != 0
    {
        return Err(wall(format!(
            "{sw}x{sh} from {},{} of a {dib_width}x{dib_height} DIB onto {width}x{height}",
            signed(*sx),
            signed(*sy)
        )));
    }
    let Some(ComState::Surface {
        pixels,
        width: surface_width,
        height: surface_height,
        pitch,
        format,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Err(wall(format!("{dc:#x}, which is no surface's DC")));
    };
    if format.bits < 16 {
        return Err(wall(format!("onto a {}-bit surface", format.bits)));
    }
    let (x0, x1) = (x.max(0), (x + width).min(i64::from(surface_width)));
    let (y0, y1) = (y.max(0), (y + height).min(i64::from(surface_height)));
    // Wholly off the surface: nothing to draw, and drawn.
    if x0 >= x1 || y0 >= y1 {
        return Ok(1);
    }
    let stride = (dib_width as usize * 2).next_multiple_of(4);
    let at = if *bits == 0 { header + size } else { *bits };
    let image = ctx
        .mem
        .slice(at, stride * dib_height as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    let colour = |p: &[u8; 2]| -> [u8; 4] {
        let v = u16::from_le_bytes(*p);
        let widen = |c: u16| ((c << 3) | (c >> 2)) as u8;
        [
            widen((v >> 10) & 31),
            widen((v >> 5) & 31),
            widen(v & 31),
            0,
        ]
    };
    let (scale_x, scale_y) = (width / sw, height / sh);
    let out = format.bits.div_ceil(8) as usize;
    let mut row = vec![0u8; (x1 - x0) as usize * out];
    let mut drawn = None;
    for dy in y0..y1 {
        // A positive height is bottom-up: the DIB's first row is the bottom.
        // Each of its rows is put into the surface's format once, however
        // many times over it is drawn.
        let line = (dib.1 - 1 - (dy - y) / scale_y) as usize * stride;
        if drawn != Some(line) {
            let pixels: Vec<u32> = image[line..line + dib_width as usize * 2]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|p| format.encode(colour(p)))
                .collect();
            for (dx, pixel) in (x0..x1).zip(row.chunks_exact_mut(out)) {
                let value = pixels[((dx - x) / scale_x) as usize];
                pixel.copy_from_slice(&value.to_le_bytes()[..out]);
            }
            drawn = Some(line);
        }
        let to = pixels + dy as u32 * pitch + x0 as u32 * out as u32;
        ctx.mem.write(to, &row).map_err(Stop::Fault)?;
    }
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::dib_bits;
    use crate::iv50::Picture;

    /// Studio red at full and at no light, a 2x2 picture with one chroma
    /// sample: the bottom row comes first, a 555 pixel keeps red at its top,
    /// a 24-bit one is blue, green, red, and its rows of six bytes are padded
    /// to eight.
    #[test]
    fn a_picture_is_a_bottom_up_dib_at_16_or_24_bits() {
        let picture = Picture {
            width: 2,
            height: 2,
            y: vec![81, 16, 16, 81],
            u: vec![90],
            v: vec![240],
        };
        // Full red is 255, 0, 0; with no light, 179, 0, 0.
        assert_eq!(
            (picture.rgb(0, 0), picture.rgb(1, 0)),
            ((255, 0, 0), (179, 0, 0))
        );
        assert_eq!(
            dib_bits(&picture, 16),
            [0x00, 0x58, 0x00, 0x7c, 0x00, 0x7c, 0x00, 0x58]
        );
        assert_eq!(
            dib_bits(&picture, 24),
            [0, 0, 179, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 179, 0, 0]
        );
    }
}
