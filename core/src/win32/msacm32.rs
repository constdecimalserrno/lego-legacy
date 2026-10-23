//! The Audio Compression Manager: MSACM32's streams, which turn sound in one
//! wave format into another (#153).
//!
//! What is written is what LEGO Chess's sample loader asks of it (0x0044F950):
//! Microsoft's ADPCM into sixteen-bit PCM of the same rate and channels, a
//! block as many samples as the format's `wSamplesPerBlock` says (mmreg.h's
//! `ADPCMWAVEFORMAT`, as Microsoft's ADPCM format description counts them).
//! The decoding is `sound.rs`'s, which Rock Raiders' movies already use. The
//! stream calls answer as Microsoft's pages and Wine's conformance tests, which
//! pass on Windows, have them answer, and otherwise as Wine's `msacm32` and
//! `msadp32`, read as documentation. Eight-bit PCM, any other conversion, a
//! driver, a filter or a callback is a wall: the one measurement of Windows' own
//! codec into eight bits, Wine's `test_prepareheader`, does not answer what
//! that count gives.

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::sound::Format;

pub const SHIMS: &[Shim] = &[
    stdcall("MSACM32", "acmStreamOpen", 8, open),
    stdcall("MSACM32", "acmStreamSize", 4, size),
    stdcall("MSACM32", "acmStreamPrepareHeader", 3, prepare),
    stdcall("MSACM32", "acmStreamUnprepareHeader", 3, unprepare),
    stdcall("MSACM32", "acmStreamConvert", 3, convert),
];

/// An open stream: the ADPCM it reads into sixteen-bit PCM.
#[derive(Clone, Debug)]
pub struct Stream {
    source: Format,
}

impl Stream {
    /// Bytes of ADPCM in a block.
    fn align(&self) -> u32 {
        u32::from(self.source.block_align)
    }

    /// Bytes of PCM one block decodes to: the header's two samples a channel,
    /// and two a byte after it, as `ADPCMWAVEFORMAT`'s page and `msadp32`
    /// count them.
    fn block_out(&self) -> u32 {
        samples_per_block(&self.source) * u32::from(self.source.channels) * 2
    }
}

/// Samples a channel in one block of `format`, which must hold at least its
/// header.
fn samples_per_block(format: &Format) -> u32 {
    u32::from(format.block_align) * 2 / u32::from(format.channels) - 12
}

/// mmsystem.h's and msacm.h's answers.
const MMSYSERR_NOERROR: u32 = 0;
const MMSYSERR_INVALHANDLE: u32 = 5;
const MMSYSERR_INVALFLAG: u32 = 10;
const MMSYSERR_INVALPARAM: u32 = 11;
const ACMERR_NOTPOSSIBLE: u32 = 512;
const ACMERR_UNPREPARED: u32 = 514;

/// An `ACMSTREAMHEADER`'s size and its status flags; and the reserved words
/// in it where ACM keeps what the header was prepared with, as msacmdrv.h's
/// `ACMDRVSTREAMHEADER` lays them out from 0x2C.
const HEADER_BYTES: u32 = 0x54;
const STATUSF_DONE: u32 = 0x0001_0000;
const STATUSF_PREPARED: u32 = 0x0002_0000;
const STATUSF_INQUEUE: u32 = 0x0010_0000;
const RESERVED: u32 = 0x2c;
/// Where among them the four buffers it was prepared with are kept:
/// `ACMDRVSTREAMHEADER`'s `pbPreparedSrc` and the three after it.
const PREPARED_BUFFERS: u32 = RESERVED + 0x18;

/// Microsoft's seven predictor pairs, the ones its codec decodes with.
const COEFFICIENTS: [(i32, i32); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];

fn unwritten(what: String) -> Stop {
    Stop::NotWritten {
        what: format!("MSACM32.dll!{what}"),
        caller: 0,
    }
}

/// A format as `msacm32` reads one: PCM's 16-byte `PCMWAVEFORMAT`, and for
/// anything else a `WAVEFORMATEX` and as much after it as its `cbSize` says;
/// and an ADPCM format's own samples a block, which follow its `cbSize`.
fn wave_format(ctx: &Ctx<'_>, at: u32) -> Result<(Format, u32), Stop> {
    let tag = ctx.mem.slice(at, 2).map_err(Stop::Fault)?;
    let len = if u16::from_le_bytes([tag[0], tag[1]]) == 1 {
        16
    } else {
        let head = ctx.mem.slice(at, 18).map_err(Stop::Fault)?;
        18 + usize::from(u16::from_le_bytes([head[16], head[17]]))
    };
    let bytes = ctx.mem.slice(at, len).map_err(Stop::Fault)?;
    let format = Format::parse(bytes).map_err(|e| unwritten(format!("acmStreamOpen: {e}")))?;
    let per_block = bytes
        .get(18..20)
        .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
    Ok((format, u32::from(per_block)))
}

fn open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamOpen(out, driver, source, destination, filter, callback,
    // instance, flags). A stream it does not open is no handle.
    const NONREALTIME: u32 = 0x4;
    let (out, from, to, flags) = (args[0], args[2], args[3], args[7]);
    if args[1] != 0 || args[4] != 0 || args[5] != 0 || flags & !NONREALTIME != 0 {
        return Err(unwritten(format!(
            "acmStreamOpen: driver {:#x}, filter {:#x}, callback {:#x}, flags {flags:#x}",
            args[1], args[4], args[5]
        )));
    }
    // No format, or no handle to hand back: what Windows answers has no
    // source, and Wine reads through the first and opens without the second.
    if out == 0 || from == 0 || to == 0 {
        return Err(unwritten(format!(
            "acmStreamOpen: handle at {out:#x}, formats at {from:#x} and {to:#x}"
        )));
    }
    let refused = |ctx: &mut Ctx<'_>, answer: u32| -> Result<u32, Stop> {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
        Ok(answer)
    };
    let ((source, per_block), (destination, _)) = (wave_format(ctx, from)?, wave_format(ctx, to)?);
    if (source.tag, destination.tag) != (2, 1) {
        return Err(unwritten(format!(
            "acmStreamOpen: format tag {} to {}",
            source.tag, destination.tag
        )));
    }
    // The ADPCM codec changes neither the rate nor the channels, and writes
    // eight or sixteen bits, eight only at the rates of its own table, as
    // `msadp32`'s `ADPCM_StreamOpen` and `ADPCM_GetFormatIndex` take them.
    // Nothing on the machine converts the rest, which is `ACMERR_NOTPOSSIBLE`.
    let channels = source.channels;
    if source.bits != 4
        || !(1..=2).contains(&channels)
        || (source.rate, channels) != (destination.rate, destination.channels)
        || !matches!(destination.bits, 8 | 16)
        || (destination.bits == 8 && ![8_000, 11_025, 22_050, 44_100].contains(&destination.rate))
    {
        return refused(ctx, ACMERR_NOTPOSSIBLE);
    }
    // The codec decodes with its own seven predictor pairs, a block holding at
    // least its header and as many samples as its size holds: a format that
    // says otherwise is not written, `msadp32` checking none of it.
    if source.block_align < 7 * channels
        || source.coefficients != COEFFICIENTS
        || per_block != samples_per_block(&source)
    {
        return Err(unwritten(format!(
            "acmStreamOpen: MS ADPCM of {per_block} samples a block of {}, predictors {:?}",
            source.block_align, source.coefficients
        )));
    }
    // Into eight bits, the one measurement of Windows, Wine's
    // `test_prepareheader`, has a 512-byte block of 1,012 samples convert to
    // 1,010 bytes: unmeasured on a real block, so not written.
    if destination.bits == 8 {
        return Err(unwritten(format!(
            "acmStreamOpen: MS ADPCM to eight-bit PCM, which Windows converts a 512-byte block of \
             1,012 samples to 1,010 bytes of (test_prepareheader), unmeasured on a real block; \
             {} samples a block here",
            per_block
        )));
    }
    let handle = ctx.state.next_acm_stream;
    ctx.state.next_acm_stream += 1;
    ctx.log.line(&format!(
        "acm stream {handle}: MS ADPCM at {} Hz in {channels} channels to {}-bit PCM",
        source.rate, destination.bits
    ));
    ctx.state.acm_streams.insert(handle, Stream { source });
    ctx.mem.write_u32(out, handle).map_err(Stop::Fault)?;
    Ok(MMSYSERR_NOERROR)
}

fn size(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamSize(stream, bytes, out, flags): how much PCM `bytes` of ADPCM
    // decodes to at most, a short last block counted whole, in a `DWORD` as
    // `msadp32` counts it; less than a block is `ACMERR_NOTPOSSIBLE`. Sized
    // from the destination is a wall.
    const SOURCE: u32 = 0;
    const DESTINATION: u32 = 1;
    let Some(stream) = ctx.state.acm_streams.get(&args[0]) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    let (align, block_out) = (stream.align(), stream.block_out());
    match args[3] {
        SOURCE => {}
        DESTINATION => return Err(unwritten("acmStreamSize from the destination".to_owned())),
        _ => return Ok(MMSYSERR_INVALFLAG),
    }
    ctx.mem.write_u32(args[2], 0).map_err(Stop::Fault)?;
    if args[1] < align {
        return Ok(ACMERR_NOTPOSSIBLE);
    }
    let bytes = args[1].div_ceil(align).wrapping_mul(block_out);
    ctx.mem.write_u32(args[2], bytes).map_err(Stop::Fault)?;
    Ok(MMSYSERR_NOERROR)
}

/// The stream a call names and the header it hands, checked as `msacm32`
/// checks them: an unknown stream is `MMSYSERR_INVALHANDLE`, and no header or
/// one too small `MMSYSERR_INVALPARAM`. Answers the stream's block and the
/// header's status.
fn stream_and_header(ctx: &Ctx<'_>, args: &[u32]) -> Result<Result<(Stream, u32), u32>, Stop> {
    let Some(stream) = ctx.state.acm_streams.get(&args[0]).cloned() else {
        return Ok(Err(MMSYSERR_INVALHANDLE));
    };
    let header = args[1];
    if header == 0 || ctx.mem.read_u32(header).map_err(Stop::Fault)? < HEADER_BYTES {
        return Ok(Err(MMSYSERR_INVALPARAM));
    }
    let status = ctx.mem.read_u32(header + 4).map_err(Stop::Fault)?;
    Ok(Ok((stream, status)))
}

/// A header's ADPCM and PCM, where they are and how long: `pbSrc`,
/// `cbSrcLength`, `pbDst` and `cbDstLength`.
fn buffers(ctx: &Ctx<'_>, header: u32) -> Result<[u32; 4], Stop> {
    let read = |o: u32| ctx.mem.read_u32(header + o).map_err(Stop::Fault);
    Ok([read(0xc)?, read(0x10)?, read(0x1c)?, read(0x20)?])
}

/// Whether a header still has the buffers it was prepared with, neither moved
/// nor longer, as Wine's `ACM_ValidatePointers` asks before it converts or
/// lets one go.
fn as_prepared(ctx: &Ctx<'_>, header: u32) -> Result<bool, Stop> {
    let [from, from_bytes, to, to_bytes] = buffers(ctx, header)?;
    let read = |i: u32| {
        ctx.mem
            .read_u32(header + PREPARED_BUFFERS + 4 * i)
            .map_err(Stop::Fault)
    };
    Ok(read(0)? == from && read(1)? >= from_bytes && read(2)? == to && read(3)? >= to_bytes)
}

fn prepare(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamPrepareHeader(stream, header, reserved): ready to convert once
    // it holds a block of ADPCM; with none, `MMSYSERR_INVALPARAM`, and less
    // than a block, `ACMERR_NOTPOSSIBLE`, as `test_prepareheader` finds. What
    // it was prepared with is kept in the header's reserved words.
    let (stream, status) = match stream_and_header(ctx, args)? {
        Ok(found) => found,
        Err(refused) => return Ok(refused),
    };
    if args[2] != 0 {
        return Ok(MMSYSERR_INVALFLAG);
    }
    let header = args[1];
    let [from, from_bytes, to, to_bytes] = buffers(ctx, header)?;
    if from_bytes < stream.align() {
        return Ok(if from_bytes == 0 {
            MMSYSERR_INVALPARAM
        } else {
            ACMERR_NOTPOSSIBLE
        });
    }
    let status = status & !STATUSF_INQUEUE | STATUSF_PREPARED;
    ctx.mem.write_u32(header + 4, status).map_err(Stop::Fault)?;
    // `fdwConvert`, the next header and the driver's words, then
    // `fdwPrepared` and `dwPrepared`; and the four buffers.
    for (i, value) in (0..).zip([0, 0, 0, 0, status, 0]) {
        ctx.mem
            .write_u32(header + RESERVED + 4 * i, value)
            .map_err(Stop::Fault)?;
    }
    for (i, value) in (0..).zip([from, from_bytes, to, to_bytes]) {
        ctx.mem
            .write_u32(header + PREPARED_BUFFERS + 4 * i, value)
            .map_err(Stop::Fault)?;
    }
    Ok(MMSYSERR_NOERROR)
}

fn unprepare(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamUnprepareHeader(stream, header, reserved): done stays done, as
    // `test_prepareheader` finds. A header moved since it was prepared is
    // `MMSYSERR_INVALPARAM`; a reserved word set is not written.
    let (_, status) = match stream_and_header(ctx, args)? {
        Ok(found) => found,
        Err(refused) => return Ok(refused),
    };
    if status & STATUSF_PREPARED == 0 {
        return Ok(ACMERR_UNPREPARED);
    }
    if !as_prepared(ctx, args[1])? {
        return Ok(MMSYSERR_INVALPARAM);
    }
    if args[2] != 0 {
        return Err(unwritten(format!(
            "acmStreamUnprepareHeader: reserved {:#x}",
            args[2]
        )));
    }
    let status = status & !(STATUSF_INQUEUE | STATUSF_PREPARED);
    ctx.mem
        .write_u32(args[1] + 4, status)
        .map_err(Stop::Fault)?;
    Ok(MMSYSERR_NOERROR)
}

/// `bytes` of ADPCM at `at`, in blocks of `format`'s, decoded.
fn decoded(ctx: &Ctx<'_>, format: &Format, at: u32, bytes: u32) -> Result<Vec<i16>, Stop> {
    let adpcm = ctx.mem.slice(at, bytes as usize).map_err(Stop::Fault)?;
    format
        .ms_adpcm(adpcm)
        .map_err(|e| unwritten(format!("acmStreamConvert: {e}")))
}

fn convert(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamConvert(stream, header, flags): as many whole blocks as there
    // are and the destination holds, decoded, and how much of each side was
    // used, as `msadp32`'s converters do; and a short last block after them,
    // at least its header long, unless `ACM_STREAMCONVERTF_BLOCKALIGN` asks
    // for whole blocks only, which Microsoft's `acmStreamConvert` page says
    // it alone does. The header is then done. A destination shorter than one
    // block's PCM is a wall: Windows, unlike `msadp32`, uses the whole source
    // and fills such a one (`test_prepareheader`), measured only into eight
    // bits.
    const BLOCKALIGN: u32 = 0x4;
    const START: u32 = 0x10;
    let (stream, status) = match stream_and_header(ctx, args)? {
        Ok(found) => found,
        Err(refused) => return Ok(refused),
    };
    if status & STATUSF_PREPARED == 0 {
        return Ok(ACMERR_UNPREPARED);
    }
    let header = args[1];
    for used in [0x14, 0x24] {
        ctx.mem.write_u32(header + used, 0).map_err(Stop::Fault)?;
    }
    if !as_prepared(ctx, header)? {
        return Ok(MMSYSERR_INVALPARAM);
    }
    if args[2] & !(BLOCKALIGN | START) != 0 {
        return Err(unwritten(format!("acmStreamConvert: flags {:#x}", args[2])));
    }
    let [from, from_bytes, to, to_bytes] = buffers(ctx, header)?;
    let (align, block_out) = (stream.align(), stream.block_out());
    if to_bytes < block_out {
        return Err(unwritten(format!(
            "acmStreamConvert: {to_bytes} bytes of PCM for blocks of {block_out}, which \
             Windows fills from the whole source (test_prepareheader), unmeasured at sixteen bits"
        )));
    }
    let blocks = (from_bytes / align).min(to_bytes / block_out);
    let mut used = blocks * align;
    let mut samples = decoded(ctx, &stream.source, from, used)?;
    // After the last whole block, a short one at least its header long,
    // decoded as a block its own length, when the destination holds it.
    let rest = from_bytes - used;
    let channels = u32::from(stream.source.channels);
    if args[2] & BLOCKALIGN == 0 && blocks == from_bytes / align && rest >= 7 * channels {
        let short = Format {
            block_align: u16::try_from(rest).expect("shorter than a block"),
            ..stream.source.clone()
        };
        if blocks * block_out + 2 * channels * samples_per_block(&short) <= to_bytes {
            samples.extend(decoded(ctx, &short, from + used, rest)?);
            used = from_bytes;
        }
    }
    let pcm: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    ctx.mem.write(to, &pcm).map_err(Stop::Fault)?;
    for (offset, value) in [
        (0x14, used),
        (0x24, pcm.len() as u32),
        (RESERVED, args[2]),
        (4, status | STATUSF_DONE),
    ] {
        ctx.mem
            .write_u32(header + offset, value)
            .map_err(Stop::Fault)?;
    }
    Ok(MMSYSERR_NOERROR)
}
