//! The multimedia library: the declared clock as a title of this era reads
//! it, what it asks of sound hardware, and the RIFF reader it loads its sounds
//! with.

use super::{Convention, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "WINMM",
        name: "timeGetTime",
        args: 0,
        convention: Convention::Stdcall,
        run: time_get_time,
    },
    Shim {
        dll: "WINMM",
        name: "mixerGetNumDevs",
        args: 0,
        convention: Convention::Stdcall,
        run: mixer_devices,
    },
    Shim {
        dll: "WINMM",
        name: "mmioOpenA",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_open,
    },
    Shim {
        dll: "WINMM",
        name: "mmioDescend",
        args: 4,
        convention: Convention::Stdcall,
        run: mmio_descend,
    },
    Shim {
        dll: "WINMM",
        name: "mmioAscend",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_ascend,
    },
    Shim {
        dll: "WINMM",
        name: "mmioRead",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_read,
    },
    Shim {
        dll: "WINMM",
        name: "mmioSeek",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_seek,
    },
    Shim {
        dll: "WINMM",
        name: "mmioGetInfo",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_get_info,
    },
    Shim {
        dll: "WINMM",
        name: "mmioSetInfo",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_set_info,
    },
    Shim {
        dll: "WINMM",
        name: "mmioAdvance",
        args: 3,
        convention: Convention::Stdcall,
        run: mmio_advance,
    },
    Shim {
        dll: "WINMM",
        name: "mmioClose",
        args: 2,
        convention: Convention::Stdcall,
        run: mmio_close,
    },
];

fn time_get_time(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Milliseconds on the declared clock, wrapping as the real counter does
    // (ADR-0003).
    Ok((ctx.declared_millis() & 0xffff_ffff) as u32)
}

fn mixer_devices(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // The declared machine has no hardware mixer: what a title plays is mixed
    // on the machine's own clock (ADR-0003), and there are no lines on a card
    // for it to turn up or down. Rock Raiders reads zero as nothing to set.
    Ok(0)
}

/// `FOURCC_MEM`: a file that is a run of the caller's own memory.
const FOURCC_MEM: u32 = u32::from_le_bytes(*b"MEM ");

/// `MMIOERR_CHUNKNOTFOUND` and `MMSYSERR_INVALHANDLE`.
const MMIOERR_CHUNKNOTFOUND: u32 = 265;
const MMSYSERR_INVALHANDLE: u32 = 5;

fn mmio_open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioOpenA(name, info, flags). A title that has its file in memory
    // already opens it through the MEM procedure, with the memory in the
    // MMIOINFO: its size at 20 and where it is at 24. A file opened by name
    // is read whole into guest memory and read from there, which is what
    // reading it through a buffer as large as the file would be.
    const MMIO_READWRITE_MASK: u32 = 0x3;
    let (name, info, flags) = (args[0], args[1], args[2]);
    let procedure = if info == 0 {
        0
    } else {
        ctx.mem.read_u32(info + 4).map_err(Stop::Fault)?
    };
    if name == 0 && procedure == FOURCC_MEM {
        let len = ctx.mem.read_u32(info + 20).map_err(Stop::Fault)?;
        let base = ctx.mem.read_u32(info + 24).map_err(Stop::Fault)?;
        return Ok(ctx.state.give(Object::Mmio {
            base,
            len,
            at: 0,
            owned: false,
        }));
    }
    if name == 0 || procedure != 0 || flags & MMIO_READWRITE_MASK != 0 {
        return Err(Stop::NotWritten {
            what: format!("WINMM.dll!mmioOpenA: flags {flags:#x}, procedure {procedure:#x}"),
            caller: 0,
        });
    }
    let path = ctx.string(name).map_err(Stop::Fault)?;
    let Some(bytes) = ctx.open_guest_file(&path) else {
        ctx.log.line(&format!("file missing {path}"));
        return Ok(0);
    };
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let base = ctx.state.heap.alloc(len.max(1));
    ctx.mem.write(base, &bytes).map_err(Stop::Fault)?;
    ctx.log
        .at(crate::cli::Trace::Shims, &format!("mmio opened {path}"));
    Ok(ctx.state.give(Object::Mmio {
        base,
        len,
        at: 0,
        owned: true,
    }))
}

/// A memory file's run of guest memory, and where it is reading.
fn mmio(ctx: &Ctx<'_>, handle: u32) -> Option<(u32, u32, u32)> {
    match ctx.state.object(handle) {
        Some(Object::Mmio { base, len, at, .. }) => Some((*base, *len, *at)),
        _ => None,
    }
}

fn move_to(ctx: &mut Ctx<'_>, handle: u32, to: u32) {
    if let Some(Object::Mmio { at, .. }) = ctx.state.object_mut(handle) {
        *at = to;
    }
}

/// What `mmioDescend` looks for: the next chunk, a chunk by its id, or a RIFF
/// or LIST chunk by its form type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Find {
    Next,
    Chunk(u32),
    Form(u32, u32),
}

/// A chunk found: its id, size, form type, and where its data starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chunk {
    id: u32,
    size: u32,
    form: u32,
    data: u32,
}

/// Find a chunk in RIFF bytes, from `at` to `end`, stepping over each chunk
/// that is not it, padded to an even size.
fn find_chunk(bytes: &[u8], mut at: u32, end: u32, find: Find) -> Option<Chunk> {
    let word = |o: u32| {
        bytes
            .get(o as usize..o as usize + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    while at + 8 <= end {
        let (id, size) = (word(at)?, word(at + 4)?);
        let is_form = id == u32::from_le_bytes(*b"RIFF") || id == u32::from_le_bytes(*b"LIST");
        let form = if is_form { word(at + 8)? } else { 0 };
        let found = match find {
            Find::Next => true,
            Find::Chunk(want) => id == want,
            Find::Form(kind, want) => id == kind && form == want,
        };
        if found {
            return Some(Chunk {
                id,
                size,
                form,
                data: at + 8,
            });
        }
        at = at.checked_add(8 + size + size % 2)?;
    }
    None
}

fn mmio_descend(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioDescend(file, chunk, parent, flags). An MMCKINFO: id, size, form
    // type, data offset, flags.
    const MMIO_FINDCHUNK: u32 = 0x10;
    const MMIO_FINDRIFF: u32 = 0x20;
    const MMIO_FINDLIST: u32 = 0x40;
    let (handle, chunk_at, parent, flags) = (args[0], args[1], args[2], args[3]);
    let Some((base, len, at)) = mmio(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    let read = |o: u32| ctx.mem.read_u32(chunk_at + o).map_err(Stop::Fault);
    let find = match flags & 0x70 {
        0 => Find::Next,
        MMIO_FINDCHUNK => Find::Chunk(read(0)?),
        MMIO_FINDRIFF => Find::Form(u32::from_le_bytes(*b"RIFF"), read(8)?),
        MMIO_FINDLIST => Find::Form(u32::from_le_bytes(*b"LIST"), read(8)?),
        other => {
            return Err(Stop::NotWritten {
                what: format!("WINMM.dll!mmioDescend: flags {other:#x}"),
                caller: 0,
            })
        }
    };
    let end = if parent == 0 {
        len
    } else {
        let data = ctx.mem.read_u32(parent + 12).map_err(Stop::Fault)?;
        let size = ctx.mem.read_u32(parent + 4).map_err(Stop::Fault)?;
        (data + size).min(len)
    };
    let bytes = ctx.mem.slice(base, len as usize).map_err(Stop::Fault)?;
    let Some(found) = find_chunk(bytes, at, end, find) else {
        return Ok(MMIOERR_CHUNKNOTFOUND);
    };
    for (offset, value) in [
        (0, found.id),
        (4, found.size),
        (8, found.form),
        (12, found.data),
        (16, 0),
    ] {
        ctx.mem
            .write_u32(chunk_at + offset, value)
            .map_err(Stop::Fault)?;
    }
    // Into the chunk's data: past its form type, for a RIFF or LIST chunk.
    let into = found.data + if found.form != 0 { 4 } else { 0 };
    move_to(ctx, handle, into);
    Ok(0)
}

fn mmio_ascend(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioAscend(file, chunk, flags): out past the chunk, padded to even.
    let (handle, chunk_at) = (args[0], args[1]);
    if mmio(ctx, handle).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    let size = ctx.mem.read_u32(chunk_at + 4).map_err(Stop::Fault)?;
    let data = ctx.mem.read_u32(chunk_at + 12).map_err(Stop::Fault)?;
    move_to(ctx, handle, data + size + size % 2);
    Ok(0)
}

fn mmio_read(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioRead(file, into, count): how many bytes were read, or -1.
    let (handle, into, count) = (args[0], args[1], args[2]);
    let Some((base, len, at)) = mmio(ctx, handle) else {
        return Ok(u32::MAX);
    };
    let take = count.min(len.saturating_sub(at));
    let bytes = ctx
        .mem
        .slice(base + at, take as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    ctx.mem.write(into, &bytes).map_err(Stop::Fault)?;
    move_to(ctx, handle, at + take);
    Ok(take)
}

fn mmio_seek(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioSeek(file, offset, origin): from the start, here, or the end; the
    // new position, or -1.
    let (handle, offset, origin) = (args[0], args[1] as i32, args[2]);
    let Some((_, len, at)) = mmio(ctx, handle) else {
        return Ok(u32::MAX);
    };
    let from = match origin {
        0 => 0,
        1 => i64::from(at),
        2 => i64::from(len),
        _ => return Ok(u32::MAX),
    };
    let to = from + i64::from(offset);
    if !(0..=i64::from(len)).contains(&to) {
        return Ok(u32::MAX);
    }
    move_to(ctx, handle, to as u32);
    Ok(to as u32)
}

/// Write a memory file's `MMIOINFO`: its whole memory is its buffer, read to
/// its end, with the next byte where the file is reading.
fn write_info(ctx: &mut Ctx<'_>, handle: u32, info: u32) -> Result<u32, Stop> {
    let Some((base, len, at)) = mmio(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    for (offset, value) in [
        (0, 0),
        (4, FOURCC_MEM),
        (20, len),
        (24, base),
        (28, base + at),
        (32, base + len),
        (36, base + len),
        (40, 0),
        (44, len),
        (68, handle),
    ] {
        ctx.mem
            .write_u32(info + offset, value)
            .map_err(Stop::Fault)?;
    }
    Ok(0)
}

/// Where a title's `MMIOINFO` says it has read to.
fn read_next(ctx: &mut Ctx<'_>, handle: u32, info: u32) -> Result<u32, Stop> {
    let Some((base, len, _)) = mmio(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    let next = ctx.mem.read_u32(info + 28).map_err(Stop::Fault)?;
    move_to(ctx, handle, next.saturating_sub(base).min(len));
    Ok(0)
}

fn mmio_get_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioGetInfo(file, info, flags): the buffer, for a title to read directly.
    write_info(ctx, args[0], args[1])
}

fn mmio_set_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioSetInfo(file, info, flags): how far the title read directly.
    read_next(ctx, args[0], args[1])
}

fn mmio_advance(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioAdvance(file, info, flags): the next buffer. A memory file's one
    // buffer is all of it, so past its end there is nothing more, and the
    // title finds its next byte still at the end.
    let (handle, info) = (args[0], args[1]);
    if read_next(ctx, handle, info)? != 0 {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    write_info(ctx, handle, info)
}

fn mmio_close(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if let Some(Object::Mmio {
        base, owned: true, ..
    }) = ctx.state.object(args[0])
    {
        let base = *base;
        ctx.state.heap.free(base);
    }
    Ok(if ctx.state.close(args[0]) {
        0
    } else {
        MMSYSERR_INVALHANDLE
    })
}

#[cfg(test)]
mod tests {
    use super::{find_chunk, Chunk, Find};

    #[test]
    fn a_wave_files_chunks_are_found_by_form_and_by_id() {
        let mut wave = Vec::new();
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&30u32.to_le_bytes());
        wave.extend_from_slice(b"WAVE");
        wave.extend_from_slice(b"fmt ");
        wave.extend_from_slice(&3u32.to_le_bytes());
        wave.extend_from_slice(&[1, 2, 3, 0]); // three bytes, padded to four
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&2u32.to_le_bytes());
        wave.extend_from_slice(&[9, 9]);
        let fourcc = |s: &[u8; 4]| u32::from_le_bytes(*s);
        let riff = find_chunk(&wave, 0, 38, Find::Form(fourcc(b"RIFF"), fourcc(b"WAVE")));
        assert_eq!(
            riff,
            Some(Chunk {
                id: fourcc(b"RIFF"),
                size: 30,
                form: fourcc(b"WAVE"),
                data: 8
            })
        );
        // Inside the form, past the odd-sized format chunk, to the data.
        let data = find_chunk(&wave, 12, 38, Find::Chunk(fourcc(b"data")));
        assert_eq!(data.map(|c| (c.size, c.data)), Some((2, 32)));
        assert_eq!(
            find_chunk(&wave, 12, 38, Find::Chunk(fourcc(b"cue "))),
            None
        );
    }
}
