//! Video for Windows: the AVIFile library a title reads its movies through
//! by file.
//!
//! Rock Raiders initialises it and plays its movies through DirectShow's
//! multimedia streams instead ([`super::amstream`], #9). LEGO Chess plays
//! all of its movies through its C API (#151): it opens a file, walks its
//! streams and reads their headers and formats. The file is read by
//! [`crate::avi`]. Its handles are the COM objects Windows hands out, with
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

/// A movie a title opened, held while it is open: its structure, and the
/// streams handed out of it.
pub struct File {
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

pub static INTERFACES: &[&Interface] = &[&AVI_FILE, &AVI_STREAM];

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
    let Some(bytes) = ctx.open_guest_file(&name) else {
        ctx.log.line(&format!("movie {name} missing"));
        return Ok(AVIERR_FILEOPEN);
    };
    // One that is there and is not a movie is a damaged rip, which stops the
    // run at its name as a damaged archive member does (#169, #190).
    let mut avi = crate::avi::parse(&bytes).map_err(|e| Stop::NotWritten {
        what: format!("AVIFIL32.dll!AVIFileOpenA: {name}: {e}"),
        caller: 0,
    })?;
    // So does one that holds another number of streams than its header says,
    // which Wine's `avifile.c` fails to load.
    if avi.header.streams as usize != avi.streams.len() {
        return Err(Stop::NotWritten {
            what: format!(
                "AVIFIL32.dll!AVIFileOpenA: {name}: its header says {} streams and it holds {}",
                avi.header.streams,
                avi.streams.len()
            ),
            caller: 0,
        });
    }
    name_streams(&mut avi, &name);
    let object = State::make_com(ctx, &AVI_FILE, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "movie {name} opened through AVIFile: {} streams",
        avi.streams.len()
    ));
    let streams = vec![None; avi.streams.len()];
    ctx.state.avi_files.insert(object, File { avi, streams });
    Ok(AVIERR_OK)
}

/// Name each stream its writer did not as AVIFile names it: the file's name,
/// its kind and how many of that kind it is, counted from one. Two of LEGO
/// Chess's movies were written with the names their authors' AVIFile gave
/// them, `LMI.avi Video #1` and `WL1lose.avi Audio #1`, and Wine's
/// `avifile.c` names them so too.
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
    // AVIFileInfoA(file, info, size): an AVIFILEINFOA from the main header.
    // Its time scale is its first stream's, as Microsoft's page for the
    // structure says a handler's typically is, and so is its length, in
    // those units, as Wine's `avifile.c` keeps them together.
    let (at, size) = (args[1], args[2]);
    if args[0] == 0 {
        return Ok(AVIERR_BADHANDLE);
    }
    if size < FILE_INFO_BYTES {
        return Ok(AVIERR_BADSIZE);
    }
    let file = file(ctx, "AVIFileInfoA", args[0])?;
    let h = &file.avi.header;
    let (scale, rate, length) = file
        .avi
        .streams
        .first()
        .map_or((0, 0, 0), |s| (s.scale, s.rate, s.length));
    // ponytail: szFileType, the handler's own description, is left empty:
    // what Windows' handler calls itself is not documented, and nothing on
    // the roster reads it.
    let mut info = [0u8; FILE_INFO_BYTES as usize];
    put(
        &mut info,
        &[
            (0, h.max_bytes_per_sec),
            (4, h.flags),
            (8, CAPS),
            (12, h.streams),
            (16, h.suggested_buffer),
            (20, h.width),
            (24, h.height),
            (28, scale),
            (32, rate),
            (36, length),
        ],
    );
    ctx.mem.write(at, &info).map_err(Stop::Fault)?;
    Ok(AVIERR_OK)
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
    // header and name. A video stream's empty frame is its picture, as Wine's
    // `avifile.c`, read as documentation, fills it: twenty of LEGO Chess's
    // twenty-two movies write theirs as 32-bit edges, which as vfw.h's 16-bit
    // ones are empty, and without it the game would size them 0x0.
    let (at, size) = (args[1], args[2]);
    if args[0] == 0 {
        return Ok(AVIERR_BADHANDLE);
    }
    if size < STREAM_INFO_BYTES {
        return Ok(AVIERR_BADSIZE);
    }
    let (owner, index) = stream_of(ctx, "AVIStreamInfoA", args[0])?;
    let s = &ctx.state.avi_files[&owner].avi.streams[index];
    let [left, top, right, bottom] = s.frame.map(i32::from);
    let frame = if s.is_video() && (right <= left || bottom <= top) {
        let (width, height) = s.picture_size();
        [0, 0, width as i32, height as i32]
    } else {
        [left, top, right, bottom]
    };
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
            (40, s.suggested_buffer),
            (44, s.quality),
            (48, s.sample_size),
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
