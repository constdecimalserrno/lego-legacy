//! The multimedia library: the declared clock as a title of this era reads
//! it, what it asks of sound hardware, the RIFF reader it loads its sounds
//! with, and the drive it plays its Red Book audio from.

use super::{stdcall, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("WINMM", "timeGetTime", 0, time_get_time),
    stdcall("WINMM", "mixerGetNumDevs", 0, mixer_devices),
    stdcall("WINMM", "mmioOpenA", 3, mmio_open),
    stdcall("WINMM", "mmioDescend", 4, mmio_descend),
    stdcall("WINMM", "mmioAscend", 3, mmio_ascend),
    stdcall("WINMM", "mmioRead", 3, mmio_read),
    stdcall("WINMM", "mmioSeek", 3, mmio_seek),
    stdcall("WINMM", "mmioGetInfo", 3, mmio_get_info),
    stdcall("WINMM", "mmioSetInfo", 3, mmio_set_info),
    stdcall("WINMM", "mmioAdvance", 3, mmio_advance),
    stdcall("WINMM", "mmioClose", 2, mmio_close),
    stdcall("WINMM", "mciSendStringA", 4, mci_send_string),
    stdcall("WINMM", "mciGetErrorStringA", 3, mci_error_string),
];

/// The `cdaudio` device: the title's Red Book pressing, once it is open;
/// whether it counts positions in tracks; and the sector it started playing
/// from and the declared millisecond it started at, while it plays.
#[derive(Default)]
pub struct CdAudio {
    disc: Option<crate::disc::RedBook>,
    tracks: bool,
    playing: Option<(u64, u64)>,
}

impl CdAudio {
    /// The sector the drive has played to at a declared millisecond, while it
    /// is playing: past the end of the disc, it has stopped.
    fn at(&self, millis: u64) -> Option<u64> {
        let (disc, (sector, started)) = (self.disc.as_ref()?, self.playing?);
        let at = sector + millis.saturating_sub(started) * crate::disc::SECTORS_PER_SECOND / 1000;
        (at < disc.sectors).then_some(at)
    }
}

/// `MCIERR_HARDWARE`, `_DEVICE_NOT_READY` and `_OUTOFRANGE`.
const MCIERR_HARDWARE: u32 = 262;
const MCIERR_DEVICE_NOT_READY: u32 = 276;
const MCIERR_OUTOFRANGE: u32 = 282;

/// Which voice of the mix the drive plays in: no COM object is at address
/// zero.
const CD_VOICE: u32 = 0;

fn mci_send_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mciSendStringA(command, answer, answer length, window). The commands a
    // title of this era plays its Red Book audio with: open the drive, count
    // in tracks, play from a track to the end of the disc, ask whether it is
    // still playing and which track it is on, stop, close. The tracks are the
    // title's Red Book pressing's (ADR-0006), mixed on the declared clock
    // like every other sound, and where the drive has got to follows from
    // when it started.
    let command = ctx.string(args[0]).map_err(Stop::Fault)?;
    let lower = command.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let millis = ctx.clock_millis;
    let nanos = millis * 1_000_000;
    let answer = match words[..] {
        ["open", "cdaudio"] => {
            let red_book = ctx
                .title
                .red_book
                .map(|pressing| crate::disc::open_red_book(ctx.title.image.disc_dir, pressing));
            match red_book {
                // A drive with no disc of audio in it is not ready; one whose
                // rip cannot be read has a problem.
                None | Some(Err(crate::disc::DiscError::Missing(_))) => {
                    Err(MCIERR_DEVICE_NOT_READY)
                }
                Some(Err(_)) => Err(MCIERR_HARDWARE),
                Some(Ok(disc)) => {
                    ctx.state.cd.disc = Some(disc);
                    Ok(String::new())
                }
            }
        }
        // Tracks, minutes, seconds and frames: a position is a track.
        ["set", "cdaudio", "time", "format", "tmsf"] => {
            ctx.state.cd.tracks = true;
            Ok(String::new())
        }
        ["play", "cdaudio", "from", _] if !ctx.state.cd.tracks => {
            return Err(Stop::NotWritten {
                what: format!("mciSendStringA(\"{command}\"): a position that is not a track"),
                caller: 0,
            })
        }
        ["play", "cdaudio", "from", track] => {
            let wanted: Option<u32> = track.parse().ok();
            let cd = &ctx.state.cd;
            match (cd.disc.as_ref(), wanted) {
                (None, _) => Err(MCIERR_DEVICE_NOT_READY),
                (Some(disc), Some(n)) => match disc.tracks.iter().find(|t| t.number == n) {
                    None => Err(MCIERR_OUTOFRANGE),
                    // ponytail: everything from the track to the end of the
                    // disc is read and decoded when it is asked for, a pause
                    // of a fraction of a second on the host at each track; a
                    // host that stutters there wants the mix to read the rip
                    // as it goes.
                    Some(track) => match disc.samples_from(track.start) {
                        Err(_) => Err(MCIERR_HARDWARE),
                        Ok(samples) => {
                            ctx.state.cd.playing = Some((track.start, millis));
                            ctx.state.mixer.play(CD_VOICE, samples, nanos);
                            Ok(String::new())
                        }
                    },
                },
                (Some(_), None) => Err(MCIERR_OUTOFRANGE),
            }
        }
        ["status", "cdaudio", "mode"] => match (&ctx.state.cd.disc, ctx.state.cd.at(millis)) {
            (None, _) => Err(MCIERR_DEVICE_NOT_READY),
            (Some(_), Some(_)) => Ok("playing".to_owned()),
            (Some(_), None) => Ok("stopped".to_owned()),
        },
        ["status", "cdaudio", "current", "track"] => {
            let cd = &ctx.state.cd;
            match &cd.disc {
                None => Err(MCIERR_DEVICE_NOT_READY),
                // A drive that is not playing is on the first track.
                Some(disc) => Ok(cd
                    .at(millis)
                    .and_then(|at| disc.track_at(at))
                    .unwrap_or(1)
                    .to_string()),
            }
        }
        ["stop", "cdaudio"] | ["close", "cdaudio"] => {
            ctx.state.cd.playing = None;
            ctx.state.mixer.stop(CD_VOICE, nanos);
            if words[0] == "close" {
                ctx.state.cd = CdAudio::default();
            }
            Ok(String::new())
        }
        _ => {
            return Err(Stop::NotWritten {
                what: format!("mciSendStringA(\"{command}\")"),
                caller: 0,
            })
        }
    };
    // Few enough to keep in every run's log, and what a person asks first
    // when what is heard is wrong.
    ctx.log
        .line(&format!("mci \"{command}\" answered {answer:?}"));
    match answer {
        Ok(text) => {
            ctx.write_string(args[1], args[2], &text)
                .map_err(Stop::Fault)?;
            Ok(0)
        }
        Err(code) => Ok(code),
    }
}

fn mci_error_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mciGetErrorStringA(error, buffer, length): what an error code says, in
    // the words Windows uses for it. A title asks when a command failed, as
    // `open cdaudio` does with no Red Book pressing to play from.
    let text = match args[0] {
        MCIERR_HARDWARE => "There is a problem with your media device.",
        MCIERR_DEVICE_NOT_READY => "The device is not ready.",
        MCIERR_OUTOFRANGE => "The specified parameter is out of range for the specified command.",
        _ => return Ok(0),
    };
    ctx.write_string(args[1], args[2], text)
        .map_err(Stop::Fault)?;
    Ok(1)
}

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
    // ponytail: a file opened by name is copied into the guest's heap whole,
    // which moves where later allocations land; a title that streams files
    // too large for its heap needs reads from the file as it goes.
    const MMIO_ALLOCBUF: u32 = 0x0001_0000;
    let (name, info, flags) = (args[0], args[1], args[2]);
    let (procedure, custom) = if info == 0 {
        (0, 0)
    } else {
        (
            ctx.mem.read_u32(info + 4).map_err(Stop::Fault)?,
            ctx.mem.read_u32(info + 8).map_err(Stop::Fault)?,
        )
    };
    // Reading is all this opens: creating, deleting, parsing a name, asking
    // whether a file exists, or a title's own I/O procedure, is a wall.
    if flags & !MMIO_ALLOCBUF != 0 || custom != 0 {
        return Err(Stop::NotWritten {
            what: format!("WINMM.dll!mmioOpenA: flags {flags:#x}, I/O procedure {custom:#x}"),
            caller: 0,
        });
    }
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
    if name == 0 || procedure != 0 {
        return Err(Stop::NotWritten {
            what: format!("WINMM.dll!mmioOpenA: procedure {procedure:#x}"),
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
        .at(crate::cli::Trace::Shims, || format!("mmio opened {path}"));
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
pub(crate) enum Find {
    Next,
    Chunk(u32),
    Form(u32, u32),
}

/// A chunk found: its id, size, form type, and where its data starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub(crate) id: u32,
    pub(crate) size: u32,
    pub(crate) form: u32,
    pub(crate) data: u32,
}

/// Find a chunk in RIFF bytes, from `at` to `end`, stepping over each chunk
/// that is not it, padded to an even size. A DLS bank is walked with it too
/// ([`crate::dls`]).
pub(crate) fn find_chunk(bytes: &[u8], mut at: u32, end: u32, find: Find) -> Option<Chunk> {
    let word = |o: u32| {
        bytes
            .get(o as usize..o as usize + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    while at.checked_add(8).is_some_and(|header| header <= end) {
        let (id, size) = (word(at)?, word(at + 4)?);
        let is_form = id == u32::from_le_bytes(*b"RIFF") || id == u32::from_le_bytes(*b"LIST");
        let form = if is_form {
            word(at.checked_add(8)?)?
        } else {
            0
        };
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
        at = at
            .checked_add(8)?
            .checked_add(size)?
            .checked_add(size % 2)?;
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
        data.saturating_add(size).min(len)
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
    let into = found
        .data
        .saturating_add(if found.form != 0 { 4 } else { 0 });
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
    move_to(
        ctx,
        handle,
        data.saturating_add(size).saturating_add(size % 2),
    );
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
