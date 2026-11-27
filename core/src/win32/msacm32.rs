//! The Audio Compression Manager: MSACM32's streams, which turn sound in one
//! wave format into another (#153, #394, #395).
//!
//! Supports Microsoft ADPCM (`2 -> 1`, 16-bit PCM), Microsoft IMA/DVI ADPCM
//! (`0x0011 -> 1`, 16-bit PCM), and Windows' built-in Microsoft PCM Converter
//! (`1 -> 1`, 8/16-bit, 1/2-channel PCM at any positive sample rate).

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::sound::Format;

pub const SHIMS: &[Shim] = &[
    stdcall("MSACM32", "acmStreamOpen", 8, open),
    stdcall("MSACM32", "acmStreamClose", 2, close),
    stdcall("MSACM32", "acmStreamSize", 4, size),
    stdcall("MSACM32", "acmStreamPrepareHeader", 3, prepare),
    stdcall("MSACM32", "acmStreamUnprepareHeader", 3, unprepare),
    stdcall("MSACM32", "acmStreamConvert", 3, convert),
];

/// An open stream: MS ADPCM (`2 -> 1`), IMA ADPCM (`0x0011 -> 1`), or Windows'
/// built-in PCM converter (`1 -> 1`).
#[derive(Clone, Debug)]
pub struct Stream {
    source: Format,
    destination: Format,
}

impl crate::snap::Snap for Stream {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            source,
            destination,
        } = self;
        source.snap(v);
        destination.snap(v);
        if v.is_reading() && !v.failed() {
            let valid = match (source.tag, destination.tag) {
                (1, 1) => {
                    (1..=2).contains(&source.channels)
                        && (1..=2).contains(&destination.channels)
                        && matches!(source.bits, 8 | 16)
                        && matches!(destination.bits, 8 | 16)
                        && source.rate > 0
                        && destination.rate > 0
                }
                (2, 1) => {
                    (1..=2).contains(&source.channels)
                        && source.block_align >= 7 * source.channels
                        && source.rate > 0
                }
                (0x0011, 1) => {
                    (1..=2).contains(&source.channels)
                        && source.bits == 4
                        && source.block_align >= 4 * source.channels
                        && (source.channels == 1 || source.block_align.is_multiple_of(8))
                        && source.rate > 0
                        && destination.rate == source.rate
                        && destination.channels == source.channels
                        && destination.bits == 16
                }
                _ => false,
            };
            if !valid {
                v.mark_corrupt("acm stream format");
            }
        }
    }
}

fn pcm_frame_bytes(format: &Format) -> u32 {
    let computed = u32::from(format.channels) * u32::from(format.bits / 8);
    u32::from(format.block_align).max(computed).max(1)
}

impl Stream {
    #[must_use]
    pub fn from_formats(source: Format, destination: Format) -> Self {
        Self {
            source,
            destination,
        }
    }

    pub(crate) fn empty_for_snap() -> Self {
        let empty = || Format {
            tag: 0,
            channels: 0,
            rate: 0,
            block_align: 0,
            bits: 0,
            coefficients: Vec::new(),
        };
        Self {
            source: empty(),
            destination: empty(),
        }
    }

    fn is_pcm(&self) -> bool {
        self.source.tag == 1 && self.destination.tag == 1
    }

    /// Bytes of source data in one atomic unit (block for ADPCM, frame for PCM).
    fn align(&self) -> u32 {
        if self.is_pcm() {
            pcm_frame_bytes(&self.source)
        } else {
            u32::from(self.source.block_align)
        }
    }

    /// Bytes of 16-bit PCM one ADPCM block decodes to.
    fn block_out(&self) -> u32 {
        samples_per_block(&self.source) * u32::from(self.source.channels) * 2
    }
}

/// Samples a channel in one block of `format`, which must hold at least its
/// header.
fn samples_per_block(format: &Format) -> u32 {
    let ch = u32::from(format.channels);
    let align = u32::from(format.block_align);
    if format.tag == 0x0011 {
        (align - 4 * ch) * 2 / ch + 1
    } else {
        align * 2 / ch - 12
    }
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
    // instance, flags).
    const NONREALTIME: u32 = 0x4;
    let (out, from, to, flags) = (args[0], args[2], args[3], args[7]);
    if args[1] != 0 || args[4] != 0 || args[5] != 0 || flags & !NONREALTIME != 0 {
        return Err(unwritten(format!(
            "acmStreamOpen: driver {:#x}, filter {:#x}, callback {:#x}, flags {flags:#x}",
            args[1], args[4], args[5]
        )));
    }
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
    let summary = match (source.tag, destination.tag) {
        (1, 1) => {
            if !(1..=2).contains(&source.channels)
                || !(1..=2).contains(&destination.channels)
                || !matches!(source.bits, 8 | 16)
                || !matches!(destination.bits, 8 | 16)
                || source.rate == 0
                || destination.rate == 0
            {
                return refused(ctx, ACMERR_NOTPOSSIBLE);
            }
            format!(
                "PCM {} Hz {}ch {}-bit to {} Hz {}ch {}-bit",
                source.rate,
                source.channels,
                source.bits,
                destination.rate,
                destination.channels,
                destination.bits
            )
        }
        (2, 1) => {
            let channels = source.channels;
            if source.bits != 4
                || !(1..=2).contains(&channels)
                || (source.rate, channels) != (destination.rate, destination.channels)
                || !matches!(destination.bits, 8 | 16)
                || (destination.bits == 8
                    && ![8_000, 11_025, 22_050, 44_100].contains(&destination.rate))
            {
                return refused(ctx, ACMERR_NOTPOSSIBLE);
            }
            if source.block_align < 7 * channels
                || source.coefficients != COEFFICIENTS
                || per_block != samples_per_block(&source)
            {
                return Err(unwritten(format!(
                    "acmStreamOpen: MS ADPCM of {per_block} samples a block of {}, predictors {:?}",
                    source.block_align, source.coefficients
                )));
            }
            if destination.bits == 8 {
                return Err(unwritten(format!(
                    "acmStreamOpen: MS ADPCM to eight-bit PCM, which Windows converts a 512-byte \
                     block of 1,012 samples to 1,010 bytes of (test_prepareheader), unmeasured on \
                     a real block; {per_block} samples a block here"
                )));
            }
            format!(
                "MS ADPCM at {} Hz in {channels} channels to {}-bit PCM",
                source.rate, destination.bits
            )
        }
        (0x0011, 1) => {
            let channels = source.channels;
            if source.bits != 4
                || !(1..=2).contains(&channels)
                || (source.rate, channels) != (destination.rate, destination.channels)
                || !matches!(destination.bits, 8 | 16)
                || source.rate == 0
            {
                return refused(ctx, ACMERR_NOTPOSSIBLE);
            }
            let valid_align = source.block_align >= 4 * channels
                && (channels == 1 || source.block_align.is_multiple_of(8));
            if !valid_align || (per_block != 0 && per_block != samples_per_block(&source)) {
                return Err(unwritten(format!(
                    "acmStreamOpen: IMA ADPCM of {per_block} samples a block of {}",
                    source.block_align
                )));
            }
            if destination.bits == 8 {
                return Err(unwritten(format!(
                    "acmStreamOpen: IMA ADPCM to eight-bit PCM ({per_block} samples a block)"
                )));
            }
            format!(
                "IMA ADPCM at {} Hz in {channels} channels to {}-bit PCM",
                source.rate, destination.bits
            )
        }
        (src_tag, dst_tag) => {
            return Err(unwritten(format!(
                "acmStreamOpen: format tag {src_tag} to {dst_tag}"
            )));
        }
    };
    let handle = ctx.state.next_acm_stream;
    ctx.state.next_acm_stream += 1;
    ctx.log.line(&format!("acm stream {handle}: {summary}"));
    ctx.state.acm_streams.insert(
        handle,
        Stream {
            source,
            destination,
        },
    );
    ctx.mem.write_u32(out, handle).map_err(Stop::Fault)?;
    Ok(MMSYSERR_NOERROR)
}

fn close(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamClose(has, fdwClose): `fdwClose` is reserved and must be zero.
    if args[1] != 0 {
        return Ok(MMSYSERR_INVALFLAG);
    }
    if ctx.state.acm_streams.remove(&args[0]).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    Ok(MMSYSERR_NOERROR)
}

fn size(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // acmStreamSize(stream, bytes, out, flags).
    const SOURCE: u32 = 0;
    const DESTINATION: u32 = 1;
    let Some(stream) = ctx.state.acm_streams.get(&args[0]).cloned() else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    if args[2] == 0 {
        return Ok(MMSYSERR_INVALPARAM);
    }
    if stream.is_pcm() {
        let (in_align, out_align, in_rate, out_rate) = match args[3] {
            SOURCE => (
                u64::from(pcm_frame_bytes(&stream.source)),
                u64::from(pcm_frame_bytes(&stream.destination)),
                u64::from(stream.source.rate),
                u64::from(stream.destination.rate),
            ),
            DESTINATION => (
                u64::from(pcm_frame_bytes(&stream.destination)),
                u64::from(pcm_frame_bytes(&stream.source)),
                u64::from(stream.destination.rate),
                u64::from(stream.source.rate),
            ),
            _ => return Ok(MMSYSERR_INVALFLAG),
        };
        ctx.mem.write_u32(args[2], 0).map_err(Stop::Fault)?;
        let input = u64::from(args[1]);
        if input < in_align {
            return Ok(ACMERR_NOTPOSSIBLE);
        }
        let out_frames = ((input / in_align) * out_rate).div_ceil(in_rate);
        let Some(out_bytes) = out_frames
            .checked_mul(out_align)
            .and_then(|v| u32::try_from(v).ok())
        else {
            return Ok(ACMERR_NOTPOSSIBLE);
        };
        ctx.mem.write_u32(args[2], out_bytes).map_err(Stop::Fault)?;
        return Ok(MMSYSERR_NOERROR);
    }
    let (align, block_out) = (stream.align(), stream.block_out());
    let (min_in, unit_in, unit_out) = match args[3] {
        SOURCE => (align, align, block_out),
        DESTINATION => (pcm_frame_bytes(&stream.destination), block_out, align),
        _ => return Ok(MMSYSERR_INVALFLAG),
    };
    ctx.mem.write_u32(args[2], 0).map_err(Stop::Fault)?;
    if args[1] < min_in {
        return Ok(ACMERR_NOTPOSSIBLE);
    }
    let Some(bytes) = args[1].div_ceil(unit_in).checked_mul(unit_out) else {
        return Ok(ACMERR_NOTPOSSIBLE);
    };
    ctx.mem.write_u32(args[2], bytes).map_err(Stop::Fault)?;
    Ok(MMSYSERR_NOERROR)
}

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

fn buffers(ctx: &Ctx<'_>, header: u32) -> Result<[u32; 4], Stop> {
    let read = |o: u32| ctx.mem.read_u32(header + o).map_err(Stop::Fault);
    Ok([read(0xc)?, read(0x10)?, read(0x1c)?, read(0x20)?])
}

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

fn decoded(ctx: &Ctx<'_>, format: &Format, at: u32, bytes: u32) -> Result<Vec<i16>, Stop> {
    let adpcm = ctx.mem.slice(at, bytes as usize).map_err(Stop::Fault)?;
    let result = if format.tag == 0x0011 {
        format.ima_adpcm(adpcm)
    } else {
        format.ms_adpcm(adpcm)
    };
    result.map_err(|e| unwritten(format!("acmStreamConvert: {e}")))
}

/// Convert PCM frames from `src_fmt` to `dst_fmt`, bounded by `max_dst_bytes`.
/// Returns `(src_bytes_used, dst_bytes)`.
fn convert_pcm(
    src_fmt: &Format,
    dst_fmt: &Format,
    src: &[u8],
    max_dst_bytes: usize,
) -> (u32, Vec<u8>) {
    let src_align = pcm_frame_bytes(src_fmt) as usize;
    let dst_align = pcm_frame_bytes(dst_fmt) as usize;
    let avail_src_frames = src.len() / src_align;
    let max_dst_frames = max_dst_bytes / dst_align;
    if avail_src_frames == 0 || max_dst_frames == 0 {
        return (0, Vec::new());
    }
    let src_rate = u64::from(src_fmt.rate);
    let dst_rate = u64::from(dst_fmt.rate);

    let full_dst_frames = ((avail_src_frames as u64) * dst_rate / src_rate) as usize;
    let (use_src_frames, out_frames) = if full_dst_frames <= max_dst_frames {
        (avail_src_frames, full_dst_frames.max(1).min(max_dst_frames))
    } else {
        let s = ((max_dst_frames as u64) * src_rate / dst_rate) as usize;
        (s.min(avail_src_frames), max_dst_frames)
    };
    if use_src_frames == 0 {
        return (0, Vec::new());
    }

    let mut frames = Vec::with_capacity(use_src_frames);
    for i in 0..use_src_frames {
        let off = i * src_align;
        let sample = |ch: usize| -> i16 {
            if src_fmt.bits == 8 {
                (i16::from(src[off + ch]) - 128) << 8
            } else {
                i16::from_le_bytes([src[off + ch * 2], src[off + ch * 2 + 1]])
            }
        };
        let l = sample(0);
        let r = if src_fmt.channels == 2 { sample(1) } else { l };
        frames.push((l, r));
    }

    let mut out = Vec::with_capacity(out_frames * dst_align);
    for i in 0..out_frames {
        let (l, r) = if src_rate == dst_rate {
            frames[i.min(frames.len() - 1)]
        } else {
            let num = (i as u64) * src_rate;
            let idx = (num / dst_rate) as usize;
            let rem = (num % dst_rate) as i64;
            let (l0, r0) = frames[idx.min(frames.len() - 1)];
            let (l1, r1) = frames[(idx + 1).min(frames.len() - 1)];
            let lerp = |a: i16, b: i16| -> i16 {
                (i64::from(a) + ((i64::from(b) - i64::from(a)) * rem) / (dst_rate as i64)) as i16
            };
            (lerp(l0, l1), lerp(r0, r1))
        };
        let mut push_sample = |s: i16| {
            if dst_fmt.bits == 8 {
                out.push(((s >> 8) + 128) as u8);
            } else {
                out.extend_from_slice(&s.to_le_bytes());
            }
        };
        if dst_fmt.channels == 1 {
            let mono = ((i32::from(l) + i32::from(r)) / 2) as i16;
            push_sample(mono);
        } else {
            push_sample(l);
            push_sample(r);
        }
    }
    ((use_src_frames * src_align) as u32, out)
}

fn convert(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    const BLOCKALIGN: u32 = 0x4;
    const START: u32 = 0x10;
    const END: u32 = 0x20;
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
    let allowed = BLOCKALIGN | START | END;
    if args[2] & !allowed != 0 {
        return Err(unwritten(format!("acmStreamConvert: flags {:#x}", args[2])));
    }
    let [from, from_bytes, to, to_bytes] = buffers(ctx, header)?;
    let (used, pcm) = if stream.is_pcm() {
        let src = ctx
            .mem
            .slice(from, from_bytes as usize)
            .map_err(Stop::Fault)?;
        convert_pcm(&stream.source, &stream.destination, src, to_bytes as usize)
    } else {
        let (align, block_out) = (stream.align(), stream.block_out());
        if to_bytes < block_out {
            return Err(unwritten(format!(
                "acmStreamConvert: {to_bytes} bytes of PCM for blocks of {block_out}, which \
                 Windows fills from the whole source (test_prepareheader), unmeasured at sixteen \
                 bits"
            )));
        }
        let blocks = (from_bytes / align).min(to_bytes / block_out);
        let mut used = blocks * align;
        let mut samples = decoded(ctx, &stream.source, from, used)?;
        let rest = from_bytes - used;
        let channels = u32::from(stream.source.channels);
        let min_short = if stream.source.tag == 0x0011 {
            4 * channels
        } else {
            7 * channels
        };
        let short_len = if stream.source.tag == 0x0011 && channels == 2 {
            rest - (rest % 8)
        } else {
            rest
        };
        if args[2] & BLOCKALIGN == 0 && blocks == from_bytes / align && short_len >= min_short {
            let short = Format {
                block_align: u16::try_from(short_len).expect("shorter than a block"),
                ..stream.source.clone()
            };
            if 2 * channels * samples_per_block(&short) <= to_bytes - blocks * block_out {
                samples.extend(decoded(ctx, &short, from + used, short_len)?);
                used += short_len;
            }
        }
        (used, samples.iter().flat_map(|s| s.to_le_bytes()).collect())
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::tests::with_test_ctx;

    fn write_pcm_fmt(mem: &mut crate::guest::Arena, at: u32, ch: u16, rate: u32, bits: u16) {
        let align = ch * (bits / 8);
        let avg = rate * u32::from(align);
        let mut buf = [0u8; 18];
        buf[0..2].copy_from_slice(&1u16.to_le_bytes());
        buf[2..4].copy_from_slice(&ch.to_le_bytes());
        buf[4..8].copy_from_slice(&rate.to_le_bytes());
        buf[8..12].copy_from_slice(&avg.to_le_bytes());
        buf[12..14].copy_from_slice(&align.to_le_bytes());
        buf[14..16].copy_from_slice(&bits.to_le_bytes());
        mem.write(at, &buf).unwrap();
    }

    #[test]
    fn pcm_converter_converts_8bit_to_16bit_handles_small_upsample_dst_and_closes() {
        // Upsampling 11,025 -> 44,100 Hz into a 2-frame dst buffer must not panic.
        let src_fmt = Format {
            tag: 1,
            channels: 1,
            rate: 11_025,
            block_align: 2,
            bits: 16,
            coefficients: Vec::new(),
        };
        let dst_fmt = Format {
            tag: 1,
            channels: 1,
            rate: 44_100,
            block_align: 2,
            bits: 16,
            coefficients: Vec::new(),
        };
        assert_eq!(
            convert_pcm(&src_fmt, &dst_fmt, &[0u8; 8], 4),
            (0, Vec::new())
        );

        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let out_h = ctx.state.heap.alloc(4);
            let src_fmt = ctx.state.heap.alloc(18);
            let dst_fmt = ctx.state.heap.alloc(18);
            write_pcm_fmt(ctx.mem, src_fmt, 1, 22_050, 8);
            write_pcm_fmt(ctx.mem, dst_fmt, 1, 22_050, 16);
            assert_eq!(
                open(ctx, &[out_h, 0, src_fmt, dst_fmt, 0, 0, 0, 0]).unwrap(),
                MMSYSERR_NOERROR
            );
            let h = ctx.mem.read_u32(out_h).unwrap();
            assert_ne!(h, 0);

            let sz_out = ctx.state.heap.alloc(4);
            assert_eq!(size(ctx, &[h, 4, sz_out, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(ctx.mem.read_u32(sz_out).unwrap(), 8);

            let src_buf = ctx.state.heap.alloc(4);
            let dst_buf = ctx.state.heap.alloc(8);
            ctx.mem.write(src_buf, &[128, 255, 0, 192]).unwrap();
            let hdr = ctx.state.heap.alloc(HEADER_BYTES);
            ctx.mem.write(hdr, &[0u8; HEADER_BYTES as usize]).unwrap();
            ctx.mem.write_u32(hdr, HEADER_BYTES).unwrap();
            ctx.mem.write_u32(hdr + 0x0c, src_buf).unwrap();
            ctx.mem.write_u32(hdr + 0x10, 4).unwrap();
            ctx.mem.write_u32(hdr + 0x1c, dst_buf).unwrap();
            ctx.mem.write_u32(hdr + 0x20, 8).unwrap();

            assert_eq!(prepare(ctx, &[h, hdr, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(convert(ctx, &[h, hdr, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(ctx.mem.read_u32(hdr + 0x14).unwrap(), 4);
            assert_eq!(ctx.mem.read_u32(hdr + 0x24).unwrap(), 8);
            let out = ctx.mem.slice(dst_buf, 8).unwrap();
            assert_eq!(i16::from_le_bytes([out[0], out[1]]), 0);
            assert_eq!(i16::from_le_bytes([out[2], out[3]]), 32512);
            assert_eq!(i16::from_le_bytes([out[4], out[5]]), -32768);

            assert_eq!(unprepare(ctx, &[h, hdr, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(close(ctx, &[h, 1]).unwrap(), MMSYSERR_INVALFLAG);
            assert_eq!(close(ctx, &[h, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(close(ctx, &[h, 0]).unwrap(), MMSYSERR_INVALHANDLE);
        });
    }

    fn write_ima_fmt(
        mem: &mut crate::guest::Arena,
        at: u32,
        ch: u16,
        rate: u32,
        align: u16,
        samples_per_blk: u16,
    ) {
        let avg = rate * u32::from(align) / u32::from(samples_per_blk.max(1));
        let mut buf = [0u8; 20];
        buf[0..2].copy_from_slice(&0x0011u16.to_le_bytes());
        buf[2..4].copy_from_slice(&ch.to_le_bytes());
        buf[4..8].copy_from_slice(&rate.to_le_bytes());
        buf[8..12].copy_from_slice(&avg.to_le_bytes());
        buf[12..14].copy_from_slice(&align.to_le_bytes());
        buf[14..16].copy_from_slice(&4u16.to_le_bytes());
        buf[16..18].copy_from_slice(&2u16.to_le_bytes());
        buf[18..20].copy_from_slice(&samples_per_blk.to_le_bytes());
        mem.write(at, &buf).unwrap();
    }

    #[test]
    fn ima_adpcm_stream_opens_sizes_both_ways_converts_and_snaps() {
        use crate::snap::{Reader, Snap, Visitor, Writer};

        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let out_h = ctx.state.heap.alloc(4);
            let src_fmt = ctx.state.heap.alloc(20);
            let dst_fmt = ctx.state.heap.alloc(18);
            // Stereo 22,050 Hz, block_align = 1024 -> 1017 samples/block (4068 PCM bytes/block).
            write_ima_fmt(ctx.mem, src_fmt, 2, 22_050, 1024, 1017);
            write_pcm_fmt(ctx.mem, dst_fmt, 2, 22_050, 16);
            assert_eq!(
                open(ctx, &[out_h, 0, src_fmt, dst_fmt, 0, 0, 0, 0x4]).unwrap(),
                MMSYSERR_NOERROR
            );
            let h = ctx.mem.read_u32(out_h).unwrap();
            assert_ne!(h, 0);

            // acmStreamSize SOURCE (0) and DESTINATION (1).
            let sz_out = ctx.state.heap.alloc(4);
            assert_eq!(size(ctx, &[h, 1024, sz_out, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(ctx.mem.read_u32(sz_out).unwrap(), 4068);
            assert_eq!(
                size(ctx, &[h, 1023, sz_out, 0]).unwrap(),
                ACMERR_NOTPOSSIBLE
            );
            assert_eq!(size(ctx, &[h, 5880, sz_out, 1]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(ctx.mem.read_u32(sz_out).unwrap(), 2048);
            assert_eq!(size(ctx, &[h, 4068, sz_out, 1]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(ctx.mem.read_u32(sz_out).unwrap(), 1024);
            assert_eq!(size(ctx, &[h, 0, sz_out, 1]).unwrap(), ACMERR_NOTPOSSIBLE);

            // Convert 1 block (2048 src offered, 5880 dst offered -> 1 block = 1024 -> 4068).
            let src_buf = ctx.state.heap.alloc(2048);
            let dst_buf = ctx.state.heap.alloc(5880);
            let mut block = vec![0u8; 2048];
            block[0..2].copy_from_slice(&1000i16.to_le_bytes());
            block[4..6].copy_from_slice(&(-1000i16).to_le_bytes());
            block[8] = 0x04;
            block[12] = 0x0c;
            ctx.mem.write(src_buf, &block).unwrap();

            let hdr = ctx.state.heap.alloc(HEADER_BYTES);
            ctx.mem.write(hdr, &[0u8; HEADER_BYTES as usize]).unwrap();
            ctx.mem.write_u32(hdr, HEADER_BYTES).unwrap();
            ctx.mem.write_u32(hdr + 0x0c, src_buf).unwrap();
            ctx.mem.write_u32(hdr + 0x10, 2048).unwrap();
            ctx.mem.write_u32(hdr + 0x1c, dst_buf).unwrap();
            ctx.mem.write_u32(hdr + 0x20, 5880).unwrap();

            assert_eq!(prepare(ctx, &[h, hdr, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(convert(ctx, &[h, hdr, 0x10]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(ctx.mem.read_u32(hdr + 0x14).unwrap(), 1024);
            assert_eq!(ctx.mem.read_u32(hdr + 0x24).unwrap(), 4068);
            let out = ctx.mem.slice(dst_buf, 8).unwrap();
            assert_eq!(i16::from_le_bytes([out[0], out[1]]), 1000);
            assert_eq!(i16::from_le_bytes([out[2], out[3]]), -1000);
            assert_eq!(i16::from_le_bytes([out[4], out[5]]), 1007);
            assert_eq!(i16::from_le_bytes([out[6], out[7]]), -1007);

            // LLSNAP v2 Snap round-trip for IMA ADPCM Stream.
            let mut stream = ctx.state.acm_streams[&h].clone();
            let mut w = Writer::new();
            stream.snap(&mut w);
            let mut restored = Stream::empty_for_snap();
            let mut r = Reader::new(&w.out);
            restored.snap(&mut r);
            assert!(!r.failed());
            assert_eq!(restored.source, stream.source);
            assert_eq!(restored.destination, stream.destination);

            // Corrupt IMA ADPCM stream snapshot (misaligned stereo block_align) is refused.
            let mut bad = stream;
            bad.source.block_align = 12;
            let mut w_bad = Writer::new();
            bad.snap(&mut w_bad);
            let mut restored_bad = Stream::empty_for_snap();
            let mut r_bad = Reader::new(&w_bad.out);
            restored_bad.snap(&mut r_bad);
            assert!(r_bad.failed());

            assert_eq!(unprepare(ctx, &[h, hdr, 0]).unwrap(), MMSYSERR_NOERROR);
            assert_eq!(close(ctx, &[h, 0]).unwrap(), MMSYSERR_NOERROR);
        });
    }
}
