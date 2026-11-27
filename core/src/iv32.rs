// SPDX-License-Identifier: LGPL-2.1-or-later
// Derived from FFmpeg's Indeo 3 decoder (libavcodec/indeo3.c),
// Copyright (c) 2009-2011 Maxim Poliakovski and the FFmpeg developers.
// See NOTICE.

//! Intel Indeo Video 3 (`IV31` / `IV32`) decoder.
//!
//! Decodes 4:1:0 planar YUV frames (`crate::iv50::Picture`) using binary space
//! partition trees, motion compensation, and vector-quantised delta tables.
//! Written against FFmpeg's `libavcodec/indeo3.c` and `indeo3data.h`; see
//! `NOTICE`.

mod tables;

use crate::iv50::Picture;
use tables::{delta_m10, REQUANT_TAB, VQ_TAB};

/// Why an Indeo 3 packet did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IV32: {}", self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

const OS_HDR_ID: u32 = u32::from_be_bytes(*b"FRMH");
const BS_8BIT_PEL: u16 = 1 << 1;
const BS_MV_Y_HALF: u16 = 1 << 4;
const BS_MV_X_HALF: u16 = 1 << 5;
const BS_BUFFER: u16 = 9;

const CELL_STACK_MAX: usize = 20;

const H_SPLIT: u32 = 0;
const V_SPLIT: u32 = 1;
const INTRA_NULL: u32 = 2;
const INTER_DATA: u32 = 3;

const RLE_ESC_F9: u8 = 249;
const RLE_ESC_FA: u8 = 250;
const RLE_ESC_FB: u8 = 251;
const RLE_ESC_FC: u8 = 252;
const RLE_ESC_FD: u8 = 253;
const RLE_ESC_FE: u8 = 254;
const RLE_ESC_FF: u8 = 255;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Plane {
    width: usize,
    height: usize,
    pitch: usize,
    /// Two reference buffers; row 0 of each is the `0x40` INTRA prediction
    /// sentinel line, so pixel row `y` starts at `(y + 1) * pitch`.
    buffers: [Vec<u8>; 2],
}

impl Plane {
    fn allocate(width: usize, height: usize) -> Self {
        let pitch = width.next_multiple_of(16);
        let size = pitch * (height + 1);
        let make = || {
            let mut buf = vec![0u8; size];
            buf[..pitch].fill(0x40);
            buf
        };
        Self {
            width,
            height,
            pitch,
            buffers: [make(), make()],
        }
    }

    fn is_valid_for_snap(&self) -> bool {
        if self.width == 0 && self.height == 0 {
            return self.pitch == 0 && self.buffers[0].is_empty() && self.buffers[1].is_empty();
        }
        if !(4..=640).contains(&self.width)
            || !(4..=480).contains(&self.height)
            || (self.width & 3) != 0
            || (self.height & 3) != 0
            || self.pitch != self.width.next_multiple_of(16)
        {
            return false;
        }
        let expected = self.pitch * (self.height + 1);
        self.buffers[0].len() == expected && self.buffers[1].len() == expected
    }
}

impl crate::snap::Snap for Plane {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            width,
            height,
            pitch,
            buffers,
        } = self;
        v.usize(width);
        v.usize(height);
        v.usize(pitch);
        for buf in buffers {
            v.byte_vec(buf);
        }
        if v.is_reading() && !self.is_valid_for_snap() {
            v.mark_corrupt("iv32::Plane dimensions or buffer size");
        }
    }
}

/// Stateful Indeo 3 (`IV31`/`IV32`) stream decoder holding dual reference
/// frame buffers across inter frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decoder {
    width: usize,
    height: usize,
    planes: [Plane; 3],
}

impl crate::snap::Snap for Decoder {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            width,
            height,
            planes,
        } = self;
        v.usize(width);
        v.usize(height);
        for plane in planes.iter_mut() {
            plane.snap(v);
        }
        if v.is_reading() && !v.failed() {
            let valid = if *width == 0 && *height == 0 {
                planes.iter().all(|p| p.width == 0 && p.height == 0)
            } else if (16..=640).contains(width)
                && (16..=480).contains(height)
                && (*width & 3) == 0
                && (*height & 3) == 0
            {
                let cw = (*width >> 2).next_multiple_of(4);
                let ch = (*height >> 2).next_multiple_of(4);
                (planes[0].width, planes[0].height) == (*width, *height)
                    && (planes[1].width, planes[1].height) == (cw, ch)
                    && (planes[2].width, planes[2].height) == (cw, ch)
            } else {
                false
            };
            if !valid {
                v.mark_corrupt("iv32::Decoder dimensions or plane layout");
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Cell {
    xpos: usize,
    ypos: usize,
    width: usize,
    height: usize,
    tree: u8,
    mv: Option<(i8, i8)>,
}

struct PlaneBits<'a> {
    data: &'a [u8],
    pos: usize,
    skip_bits: usize,
    need_resync: bool,
    next_cell_offset: usize,
}

impl<'a> PlaneBits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            skip_bits: 0,
            need_resync: false,
            next_cell_offset: 0,
        }
    }

    fn bits_left(&self) -> usize {
        (self.data.len() * 8).saturating_sub(self.pos)
    }

    fn resync(&mut self) {
        if self.need_resync && (self.pos & 7) == 0 {
            self.pos = self.pos.saturating_add(self.skip_bits);
            self.skip_bits = 0;
            self.need_resync = false;
        }
    }

    fn update_bitpos(&mut self, nbits: usize) {
        self.skip_bits = self.skip_bits.saturating_add(nbits);
        self.need_resync = true;
    }

    fn get_2bits(&mut self) -> Result<u32> {
        if self.pos + 2 > self.data.len() * 8 {
            return Err(Error("bitstream ended inside tree code"));
        }
        let byte = self.data[self.pos >> 3];
        let shift = 6 - (self.pos & 7);
        self.pos += 2;
        Ok(u32::from((byte >> shift) & 3))
    }

    fn sync_cell_offset(&mut self) {
        if !self.need_resync {
            self.next_cell_offset = (self.pos + 7) >> 3;
        }
    }
}

struct FrameParams<'a> {
    cb_offset: usize,
    buf_sel: usize,
    alt_quant: &'a [u8],
}

fn read_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn write_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn write_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn read_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

fn write_u64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn replicate32(mut a: u32) -> u32 {
    a &= 0x00ff_00ff;
    a |= a << 8;
    a
}

fn replicate64(mut a: u64) -> u64 {
    a &= 0x00ff_00ff_00ff_00ff;
    a |= a << 8;
    a
}

fn avg_u32(a: u32, b: u32) -> u32 {
    (a.wrapping_add(b) >> 1) & 0x7f7f_7f7f
}

fn avg_u64(a: u64, b: u64) -> u64 {
    (a.wrapping_add(b) >> 1) & 0x7f7f_7f7f_7f7f_7f7f
}

fn copy_block4(
    dst_buf: &mut [u8],
    ref_buf: Option<&[u8]>,
    mut dst: usize,
    mut src: usize,
    pitch: usize,
    h: usize,
) {
    for _ in 0..h {
        let v = read_u32(ref_buf.unwrap_or(dst_buf), src);
        write_u32(dst_buf, dst, v);
        dst += pitch;
        src += pitch;
    }
}

#[allow(clippy::too_many_arguments)]
fn rle_fill(
    mode: u8,
    is_intra: bool,
    is_top: bool,
    dst_buf: &mut [u8],
    ref_buf: Option<&[u8]>,
    dst_at: usize,
    ref_at: usize,
    pitch: usize,
    lines: usize,
    v_zoom: usize,
) {
    if mode <= 4 {
        copy_block4(dst_buf, ref_buf, dst_at, ref_at, pitch, lines << v_zoom);
    } else if mode == 10 && is_intra {
        let pix64 = read_u64(ref_buf.unwrap_or(dst_buf), ref_at);
        let count = lines << 1;
        if is_top {
            let rep = replicate64(pix64);
            for r in 1..count {
                write_u64(dst_buf, dst_at + r * pitch, rep);
            }
            write_u64(dst_buf, dst_at, avg_u64(pix64, rep));
        } else {
            for r in 0..count {
                write_u64(dst_buf, dst_at + r * pitch, pix64);
            }
        }
    }
}

fn split_cell(size: usize) -> usize {
    if size > 2 {
        ((size + 2) >> 2) << 1
    } else {
        1
    }
}

fn mv_in_bounds(plane: &Plane, cell: &Cell, mv_y: i8, mv_x: i8) -> bool {
    let y0 = (cell.ypos as isize) * 4 + isize::from(mv_y);
    let x0 = (cell.xpos as isize) * 4 + isize::from(mv_x);
    let y1 = ((cell.ypos + cell.height) as isize) * 4 + isize::from(mv_y);
    let x1 = ((cell.xpos + cell.width) as isize) * 4 + isize::from(mv_x);
    y0 >= -1 && x0 >= 0 && y1 <= plane.height as isize && x1 <= plane.width as isize
}

fn ref_offset(plane: &Plane, cell: &Cell, mv_y: i8, mv_x: i8) -> usize {
    let row = ((cell.ypos as isize) * 4 + isize::from(mv_y) + 1) as usize;
    let col = ((cell.xpos as isize) * 4 + isize::from(mv_x)) as usize;
    row * plane.pitch + col
}

fn dst_offset(plane: &Plane, cell: &Cell) -> usize {
    (cell.ypos * 4 + 1) * plane.pitch + cell.xpos * 4
}

fn split_plane_buffers(plane: &mut Plane, buf_sel: usize) -> (&mut [u8], &mut [u8]) {
    let (b0, b1) = plane.buffers.split_at_mut(1);
    if buf_sel == 0 {
        (&mut b0[0], &mut b1[0])
    } else {
        (&mut b1[0], &mut b0[0])
    }
}

fn copy_cell(plane: &mut Plane, buf_sel: usize, cell: &Cell) -> Result<()> {
    let (mv_y, mv_x) = cell.mv.unwrap_or((0, 0));
    if !mv_in_bounds(plane, cell, mv_y, mv_x) {
        return Err(Error("motion vector out of frame"));
    }
    let src_start = ref_offset(plane, cell, mv_y, mv_x);
    let dst_start = dst_offset(plane, cell);
    let w = cell.width * 4;
    let h = cell.height * 4;
    let pitch = plane.pitch;
    let (dst_buf, ref_buf) = split_plane_buffers(plane, buf_sel);
    for r in 0..h {
        let s = src_start + r * pitch;
        let d = dst_start + r * pitch;
        dst_buf[d..d + w].copy_from_slice(&ref_buf[s..s + w]);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_cell_data(
    cell: &Cell,
    dst_buf: &mut [u8],
    ref_buf: Option<&[u8]>,
    mut block: usize,
    mut ref_block: usize,
    row_offset: usize,
    h_zoom: usize,
    v_zoom: usize,
    mode: u8,
    delta: [(&[u16], u8); 2],
    swap_quads: [bool; 2],
    data: &[u8],
    pos: &mut usize,
) -> Result<()> {
    if (cell.height & v_zoom) != 0 || (cell.width & h_zoom) != 0 {
        return Err(Error("cell dimensions incompatible with zoom mode"));
    }

    let blk_row_offset = (row_offset << (2 + v_zoom)) - (cell.width << 2);
    let line_offset = if v_zoom != 0 { row_offset } else { 0 };
    let is_intra = cell.mv.is_none();
    let mut rle_blocks: i32 = 0;
    let mut skip_flag = false;
    let mut is_first_row = true;

    let mut y = 0;
    while y < cell.height {
        let mut x = 0;
        while x < cell.width {
            let mut ref_at = ref_block;
            let mut dst_at = block;

            if rle_blocks > 0 {
                if mode >= 10 || !is_intra || !skip_flag {
                    rle_fill(
                        mode,
                        is_intra,
                        is_first_row,
                        dst_buf,
                        ref_buf,
                        dst_at,
                        ref_at,
                        row_offset,
                        4,
                        v_zoom,
                    );
                }
                rle_blocks -= 1;
            } else {
                let mut line = 0usize;
                while line < 4 {
                    let mut num_lines = 1usize;
                    let is_top_of_cell = is_first_row && line == 0;
                    let (deltas, quad_exp) = if mode <= 4 { delta[line & 1] } else { delta[1] };
                    let num_dyads = deltas.len() as u8;
                    if *pos >= data.len() {
                        return Err(Error("cell data truncated"));
                    }
                    let mut code = data[*pos];
                    *pos += 1;

                    if code < 248 {
                        let (dyad1, dyad2) = if code < num_dyads {
                            if *pos >= data.len() {
                                return Err(Error("cell dyad truncated"));
                            }
                            let d1 = data[*pos];
                            *pos += 1;
                            if d1 >= num_dyads {
                                return Err(Error("invalid dyad index"));
                            }
                            (usize::from(d1), usize::from(code))
                        } else {
                            let q = usize::from(code - num_dyads);
                            let exp = usize::from(quad_exp);
                            let mut d1 = q / exp;
                            let mut d2 = q % exp;
                            if swap_quads[line & 1] {
                                std::mem::swap(&mut d1, &mut d2);
                            }
                            if d1 >= deltas.len() || d2 >= deltas.len() {
                                return Err(Error("quad index out of range"));
                            }
                            (d1, d2)
                        };

                        if mode <= 4 {
                            let src = ref_buf.unwrap_or(dst_buf);
                            let r0 = read_u16(src, ref_at);
                            let r1 = read_u16(src, ref_at + 2);
                            write_u16(
                                dst_buf,
                                dst_at + line_offset,
                                r0.wrapping_add(deltas[dyad1]) & 0x7f7f,
                            );
                            write_u16(
                                dst_buf,
                                dst_at + line_offset + 2,
                                r1.wrapping_add(deltas[dyad2]) & 0x7f7f,
                            );
                            if mode >= 3 {
                                let below = read_u32(dst_buf, dst_at + row_offset);
                                if is_top_of_cell && cell.ypos == 0 {
                                    write_u32(dst_buf, dst_at, below);
                                } else {
                                    let r = read_u32(ref_buf.unwrap_or(dst_buf), ref_at);
                                    write_u32(dst_buf, dst_at, avg_u32(r, below));
                                }
                            }
                        } else if mode == 10 && is_intra {
                            let src = ref_buf.unwrap_or(dst_buf);
                            let r0 = read_u32(src, ref_at);
                            let r1 = read_u32(src, ref_at + 4);
                            let (base0, base1) = if is_top_of_cell {
                                (replicate32(r0), replicate32(r1))
                            } else {
                                (r0, r1)
                            };
                            write_u32(
                                dst_buf,
                                dst_at + row_offset,
                                base0.wrapping_add(delta_m10(deltas[dyad1])) & 0x7f7f_7f7f,
                            );
                            write_u32(
                                dst_buf,
                                dst_at + row_offset + 4,
                                base1.wrapping_add(delta_m10(deltas[dyad2])) & 0x7f7f_7f7f,
                            );
                            let below = read_u64(dst_buf, dst_at + row_offset);
                            if is_top_of_cell && cell.ypos == 0 {
                                write_u64(dst_buf, dst_at, below);
                            } else {
                                let r = read_u64(ref_buf.unwrap_or(dst_buf), ref_at);
                                write_u64(dst_buf, dst_at, avg_u64(r, below));
                            }
                        } else if mode == 10 {
                            let d1 = delta_m10(deltas[dyad1]);
                            let d2 = delta_m10(deltas[dyad2]);
                            for off in [dst_at, dst_at + row_offset] {
                                let v0 = read_u32(dst_buf, off).wrapping_add(d1) & 0x7f7f_7f7f;
                                let v1 = read_u32(dst_buf, off + 4).wrapping_add(d2) & 0x7f7f_7f7f;
                                write_u32(dst_buf, off, v0);
                                write_u32(dst_buf, off + 4, v1);
                            }
                        } else {
                            let d1 = deltas[dyad1];
                            let d2 = deltas[dyad2];
                            for off in [dst_at, dst_at + row_offset] {
                                let v0 = read_u16(dst_buf, off).wrapping_add(d1) & 0x7f7f;
                                let v1 = read_u16(dst_buf, off + 2).wrapping_add(d2) & 0x7f7f;
                                write_u16(dst_buf, off, v0);
                                write_u16(dst_buf, off + 2, v1);
                            }
                        }
                    } else {
                        match code {
                            RLE_ESC_FC | RLE_ESC_FF | RLE_ESC_FE | RLE_ESC_FD => {
                                if code == RLE_ESC_FC {
                                    skip_flag = false;
                                    rle_blocks = 1;
                                    code = RLE_ESC_FD;
                                }
                                let nl = 257isize - isize::from(code) - line as isize;
                                if nl <= 0 {
                                    return Err(Error("invalid RLE line count"));
                                }
                                num_lines = nl as usize;
                                rle_fill(
                                    mode,
                                    is_intra,
                                    is_top_of_cell,
                                    dst_buf,
                                    ref_buf,
                                    dst_at,
                                    ref_at,
                                    row_offset,
                                    num_lines,
                                    v_zoom,
                                );
                            }
                            RLE_ESC_FB => {
                                if *pos >= data.len() {
                                    return Err(Error("RLE-FB counter truncated"));
                                }
                                let cnt = data[*pos];
                                *pos += 1;
                                rle_blocks = i32::from(cnt & 0x1f) - 1;
                                if cnt >= 64 || rle_blocks < 0 {
                                    return Err(Error("invalid RLE-FB counter"));
                                }
                                skip_flag = (cnt & 0x20) != 0;
                                num_lines = 4 - line;
                                if mode >= 10 || !is_intra || !skip_flag {
                                    rle_fill(
                                        mode,
                                        is_intra,
                                        is_top_of_cell,
                                        dst_buf,
                                        ref_buf,
                                        dst_at,
                                        ref_at,
                                        row_offset,
                                        num_lines,
                                        v_zoom,
                                    );
                                }
                            }
                            RLE_ESC_F9 | RLE_ESC_FA => {
                                if code == RLE_ESC_F9 {
                                    skip_flag = true;
                                    rle_blocks = 1;
                                }
                                if line != 0 {
                                    return Err(Error("RLE-F9/FA mid-block"));
                                }
                                num_lines = 4;
                                if !is_intra && mode <= 4 {
                                    copy_block4(
                                        dst_buf,
                                        ref_buf,
                                        dst_at,
                                        ref_at,
                                        row_offset,
                                        num_lines << v_zoom,
                                    );
                                }
                            }
                            _ => return Err(Error("unsupported RLE opcode")),
                        }
                    }

                    line += num_lines;
                    ref_at += row_offset * (num_lines << v_zoom);
                    dst_at += row_offset * (num_lines << v_zoom);
                }
            }

            block += 4 << h_zoom;
            ref_block += 4 << h_zoom;
            x += 1 + h_zoom;
        }
        ref_block += blk_row_offset;
        block += blk_row_offset;
        is_first_row = false;
        y += 1 + v_zoom;
    }
    Ok(())
}

fn decode_cell(
    params: &FrameParams<'_>,
    plane: &mut Plane,
    cell: &Cell,
    data: &[u8],
) -> Result<usize> {
    if data.is_empty() {
        return Err(Error("attempt to read past end of cell data"));
    }
    let desc = data[0];
    let mode = desc >> 4;
    let mut vq_index = usize::from(desc & 0x0f);
    let mut pos = 1usize;

    let (prim_indx, second_indx) = if mode == 1 || mode == 4 {
        let alt = params.alt_quant[vq_index];
        (
            usize::from(alt >> 4) + params.cb_offset,
            usize::from(alt & 0x0f) + params.cb_offset,
        )
    } else {
        vq_index += params.cb_offset;
        (vq_index, vq_index)
    };
    if prim_indx >= 24 || second_indx >= 24 {
        return Err(Error("VQ table index out of range"));
    }
    let delta = [VQ_TAB[second_indx], VQ_TAB[prim_indx]];
    let swap_quads = [second_indx >= 16, prim_indx >= 16];

    // Validate (mv, mode) before mutating any reference or destination pixels.
    let (h_zoom, v_zoom, inter_copy, other_buf_mv) = match (cell.mv, mode) {
        (None, 0 | 1) => (0, 0, false, None),
        (None, 3 | 4) => (0, 1, false, None),
        (None, 10) => (1, 1, false, None),
        (None, 11) => return Err(Error("mode 11 used on intra cell")),
        (Some(mv), 0 | 1) => (0, 0, false, Some(mv)),
        (Some(_), 3 | 4) => return Err(Error("mode 3/4 used on inter cell")),
        (Some(_), 10) => (1, 1, true, None),
        (Some(_), 11) => (0, 1, true, None),
        _ => return Err(Error("unsupported coding mode")),
    };

    let pitch = plane.pitch;
    let block_off = dst_offset(plane, cell);

    if inter_copy {
        copy_cell(plane, params.buf_sel, cell)?;
        let dst_buf = &mut plane.buffers[params.buf_sel];
        decode_cell_data(
            cell, dst_buf, None, block_off, block_off, pitch, h_zoom, v_zoom, mode, delta,
            swap_quads, data, &mut pos,
        )?;
        return Ok(pos);
    }

    let (ref_off, use_other) = if let Some((mv_y, mv_x)) = other_buf_mv {
        if !mv_in_bounds(plane, cell, mv_y, mv_x) {
            return Err(Error("motion vector out of frame"));
        }
        (ref_offset(plane, cell, mv_y, mv_x), true)
    } else {
        (block_off - pitch, false)
    };

    let (dst_buf, ref_buf) = split_plane_buffers(plane, params.buf_sel);
    if vq_index >= 8 {
        let tab = &REQUANT_TAB[vq_index & 7];
        let req_buf = if use_other {
            &mut *ref_buf
        } else {
            &mut *dst_buf
        };
        for px in &mut req_buf[ref_off..ref_off + (cell.width << 2)] {
            *px = tab[usize::from(*px & 127)];
        }
    }
    let ref_slice = use_other.then_some(&*ref_buf);
    decode_cell_data(
        cell, dst_buf, ref_slice, block_off, ref_off, pitch, h_zoom, v_zoom, mode, delta,
        swap_quads, data, &mut pos,
    )?;
    Ok(pos)
}

#[allow(clippy::too_many_arguments)]
fn parse_bintree(
    params: &FrameParams<'_>,
    bits: &mut PlaneBits<'_>,
    mc_vectors: &[u8],
    plane: &mut Plane,
    code: u32,
    ref_cell: &mut Cell,
    depth: usize,
    strip_width: usize,
) -> Result<()> {
    if depth == 0 {
        return Err(Error("binary tree depth exceeded"));
    }
    let mut curr_cell = *ref_cell;
    if code == H_SPLIT {
        curr_cell.height = split_cell(ref_cell.height);
        if curr_cell.height == 0 || curr_cell.height >= ref_cell.height {
            return Err(Error("invalid horizontal split"));
        }
        ref_cell.ypos += curr_cell.height;
        ref_cell.height -= curr_cell.height;
    } else if code == V_SPLIT {
        curr_cell.width = if curr_cell.width > strip_width {
            if curr_cell.width <= strip_width * 2 {
                strip_width
            } else {
                strip_width * 2
            }
        } else {
            split_cell(ref_cell.width)
        };
        if curr_cell.width == 0 || curr_cell.width >= ref_cell.width {
            return Err(Error("invalid vertical split"));
        }
        ref_cell.xpos += curr_cell.width;
        ref_cell.width -= curr_cell.width;
    }

    let check_cell = |c: &Cell, p: &Plane| -> Result<()> {
        if c.width == 0
            || c.height == 0
            || c.xpos + c.width > (p.width >> 2)
            || c.ypos + c.height > (p.height >> 2)
        {
            return Err(Error("cell outside plane bounds"));
        }
        Ok(())
    };

    while bits.bits_left() >= 2 {
        bits.resync();
        match bits.get_2bits()? {
            next @ (H_SPLIT | V_SPLIT) => {
                parse_bintree(
                    params,
                    bits,
                    mc_vectors,
                    plane,
                    next,
                    &mut curr_cell,
                    depth - 1,
                    strip_width,
                )?;
            }
            INTRA_NULL => {
                if curr_cell.tree == 0 {
                    curr_cell.mv = None;
                    curr_cell.tree = 1;
                } else {
                    bits.resync();
                    let null_code = bits.get_2bits()?;
                    if null_code >= 2 {
                        return Err(Error("invalid VQ null code"));
                    }
                    check_cell(&curr_cell, plane)?;
                    if curr_cell.mv.is_none() {
                        return Err(Error("VQ null code on intra cell"));
                    }
                    return copy_cell(plane, params.buf_sel, &curr_cell);
                }
            }
            INTER_DATA => {
                if curr_cell.tree == 0 {
                    bits.sync_cell_offset();
                    if bits.next_cell_offset >= bits.data.len() {
                        return Err(Error("motion vector index past end of plane"));
                    }
                    let mv_idx = usize::from(bits.data[bits.next_cell_offset]);
                    bits.next_cell_offset += 1;
                    if mv_idx * 2 + 1 >= mc_vectors.len() {
                        return Err(Error("motion vector index out of range"));
                    }
                    curr_cell.mv = Some((
                        mc_vectors[mv_idx * 2] as i8,
                        mc_vectors[mv_idx * 2 + 1] as i8,
                    ));
                    curr_cell.tree = 1;
                    bits.update_bitpos(8);
                } else {
                    bits.sync_cell_offset();
                    check_cell(&curr_cell, plane)?;
                    if bits.next_cell_offset > bits.data.len() {
                        return Err(Error("cell payload offset past end of plane"));
                    }
                    let used = decode_cell(
                        params,
                        plane,
                        &curr_cell,
                        &bits.data[bits.next_cell_offset..],
                    )?;
                    bits.update_bitpos(used << 3);
                    bits.next_cell_offset += used;
                    return Ok(());
                }
            }
            _ => unreachable!("2-bit value is 0..=3"),
        }
    }

    Err(Error("unexpected end of plane bitstream"))
}

fn decode_plane(
    params: &FrameParams<'_>,
    plane: &mut Plane,
    data: &[u8],
    strip_width: usize,
) -> Result<()> {
    if data.len() < 4 {
        return Err(Error("plane data shorter than vector count"));
    }
    let num_vectors = read_u32(data, 0) as usize;
    let body = &data[4..];
    if num_vectors > 256 || num_vectors * 2 > body.len() {
        return Err(Error("invalid motion vector count"));
    }
    let mc_vectors = &body[..num_vectors * 2];
    let mut bits = PlaneBits::new(&body[num_vectors * 2..]);
    let mut root = Cell {
        xpos: 0,
        ypos: 0,
        width: plane.width >> 2,
        height: plane.height >> 2,
        tree: 0,
        mv: None,
    };
    parse_bintree(
        params,
        &mut bits,
        mc_vectors,
        plane,
        INTRA_NULL,
        &mut root,
        CELL_STACK_MAX,
        strip_width,
    )
}

fn output_plane(
    plane: &Plane,
    buf_sel: usize,
    dst: &mut [u8],
    dst_width: usize,
    dst_height: usize,
) {
    let buf = &plane.buffers[buf_sel];
    let rows = dst_height.min(plane.height);
    let cols = dst_width.min(plane.width);
    for y in 0..rows {
        let src_row = (y + 1) * plane.pitch;
        let dst_row = y * dst_width;
        for x in 0..cols {
            dst[dst_row + x] = (buf[src_row + x] & 0x7f) << 1;
        }
    }
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn allocate_buffers(&mut self, width: usize, height: usize) -> Result<()> {
        if !(16..=640).contains(&width)
            || !(16..=480).contains(&height)
            || (width & 3) != 0
            || (height & 3) != 0
        {
            return Err(Error("invalid picture dimensions"));
        }
        self.width = width;
        self.height = height;
        let chroma_w = (width >> 2).next_multiple_of(4);
        let chroma_h = (height >> 2).next_multiple_of(4);
        self.planes = [
            Plane::allocate(width, height),
            Plane::allocate(chroma_w, chroma_h),
            Plane::allocate(chroma_w, chroma_h),
        ];
        Ok(())
    }

    /// Decode one Indeo 3 (`IV31`/`IV32`) packet into `out`.
    ///
    /// Returns `Ok(true)` when a new picture was written to `out`, or
    /// `Ok(false)` on a 16-byte sync/null packet that repeats the prior frame.
    ///
    /// # Errors
    /// When the packet header, bitstream, or cell payload is corrupt or uses
    /// an unsupported feature (8-bit pixels or halfpel motion vectors).
    pub fn decode(&mut self, packet: &[u8], out: &mut Picture) -> Result<bool> {
        if packet.len() < 16 {
            return Err(Error("packet shorter than OS header"));
        }
        let frame_num = read_u32(packet, 0);
        let word2 = read_u32(packet, 4);
        let checksum = read_u32(packet, 8);
        let os_data_size = read_u32(packet, 12);
        if (frame_num ^ word2 ^ os_data_size ^ OS_HDR_ID) != checksum {
            return Err(Error("OS header checksum mismatch"));
        }

        let bs = &packet[16..];
        if bs.len() < 16 {
            return Err(Error("packet shorter than bitstream header"));
        }
        if read_u16(bs, 0) != 32 {
            return Err(Error("unsupported codec version"));
        }
        let frame_flags = read_u16(bs, 2);
        let raw_bits = u64::from(read_u32(bs, 4));
        let data_size = ((raw_bits + 7) >> 3) as usize;
        let cb_offset = usize::from(bs[8]);
        if data_size == 16 {
            return Ok(false);
        }
        let data_size = data_size.min(bs.len());
        if data_size < 48 {
            return Err(Error("bitstream header truncated"));
        }
        let height = usize::from(read_u16(bs, 12));
        let width = usize::from(read_u16(bs, 14));
        if width != self.width || height != self.height {
            self.allocate_buffers(width, height)?;
        }

        let y_off = read_u32(bs, 16) as usize;
        let v_off = read_u32(bs, 20) as usize;
        let u_off = read_u32(bs, 24) as usize;
        let starts = [y_off, v_off, u_off];
        let mut ends = [data_size; 3];
        for j in 0..3 {
            for &s in starts.iter().rev() {
                if s < ends[j] && s > starts[j] {
                    ends[j] = s;
                }
            }
        }
        let min_off = y_off.min(v_off).min(u_off);
        let max_off = y_off.max(v_off).max(u_off);
        if min_off < 48
            || max_off.saturating_add(16) >= data_size
            || ends[0] <= starts[0]
            || ends[1] <= starts[1]
            || ends[2] <= starts[2]
        {
            return Err(Error("invalid plane offset"));
        }
        if frame_flags & BS_8BIT_PEL != 0 {
            return Err(Error("8-bit pixel format unsupported"));
        }
        if frame_flags & (BS_MV_X_HALF | BS_MV_Y_HALF) != 0 {
            return Err(Error("halfpel motion vectors unsupported"));
        }

        let params = FrameParams {
            cb_offset,
            buf_sel: usize::from((frame_flags >> BS_BUFFER) & 1),
            alt_quant: &bs[32..48],
        };
        decode_plane(&params, &mut self.planes[0], &bs[starts[0]..ends[0]], 40)?;
        decode_plane(&params, &mut self.planes[1], &bs[starts[2]..ends[2]], 10)?;
        decode_plane(&params, &mut self.planes[2], &bs[starts[1]..ends[1]], 10)?;

        out.width = self.width;
        out.height = self.height;
        let (cw, ch) = out.chroma_size();
        out.y.resize(self.width * self.height, 0);
        out.u.resize(cw * ch, 0);
        out.v.resize(cw * ch, 0);
        output_plane(
            &self.planes[0],
            params.buf_sel,
            &mut out.y,
            self.width,
            self.height,
        );
        output_plane(&self.planes[1], params.buf_sel, &mut out.u, cw, ch);
        output_plane(&self.planes[2], params.buf_sel, &mut out.v, cw, ch);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snap::{Reader, Snap, Visitor, Writer};

    fn make_solid_frame(frame_num: u32, width: u16, height: u16) -> Vec<u8> {
        let make_plane = |blocks: u8| -> Vec<u8> {
            let mut p = Vec::new();
            p.extend_from_slice(&0u32.to_le_bytes());
            p.push(0xb0);
            p.push(0x00);
            p.push(RLE_ESC_FB);
            p.push(blocks);
            p
        };
        let y_plane = make_plane(((width / 4) * (height / 4)) as u8);
        let u_plane = make_plane(1);
        let v_plane = make_plane(1);

        let y_off = 48u32;
        let v_off = y_off + y_plane.len() as u32;
        let u_off = v_off + v_plane.len() as u32;
        let bs_len = u_off + u_plane.len() as u32 + 17;

        let mut bs = vec![0u8; bs_len as usize];
        bs[0..2].copy_from_slice(&32u16.to_le_bytes());
        bs[2..4].copy_from_slice(&0x0004u16.to_le_bytes());
        bs[4..8].copy_from_slice(&(bs_len * 8).to_le_bytes());
        bs[12..14].copy_from_slice(&height.to_le_bytes());
        bs[14..16].copy_from_slice(&width.to_le_bytes());
        bs[16..20].copy_from_slice(&y_off.to_le_bytes());
        bs[20..24].copy_from_slice(&v_off.to_le_bytes());
        bs[24..28].copy_from_slice(&u_off.to_le_bytes());
        bs[y_off as usize..y_off as usize + y_plane.len()].copy_from_slice(&y_plane);
        bs[v_off as usize..v_off as usize + v_plane.len()].copy_from_slice(&v_plane);
        bs[u_off as usize..u_off as usize + u_plane.len()].copy_from_slice(&u_plane);

        let mut pkt = Vec::with_capacity(16 + bs.len());
        let data_size = bs.len() as u32;
        let checksum = frame_num ^ data_size ^ OS_HDR_ID;
        pkt.extend_from_slice(&frame_num.to_le_bytes());
        pkt.extend_from_slice(&0u32.to_le_bytes());
        pkt.extend_from_slice(&checksum.to_le_bytes());
        pkt.extend_from_slice(&data_size.to_le_bytes());
        pkt.extend_from_slice(&bs);
        pkt
    }

    #[test]
    fn decodes_synthetic_intra_frame_handles_sync_packet_and_round_trips_snap() {
        let mut dec = Decoder::new();
        let mut pic = Picture::default();
        let pkt = make_solid_frame(0, 16, 16);
        assert_eq!(dec.decode(&pkt, &mut pic), Ok(true));
        assert_eq!((pic.width, pic.height), (16, 16));
        assert!(pic.y.iter().all(|&p| p == 128));
        assert!(pic.u.iter().all(|&p| p == 128));
        assert!(pic.v.iter().all(|&p| p == 128));

        let mut sync_pkt = Vec::new();
        let sync_csum = 1u32 ^ 16u32 ^ OS_HDR_ID;
        sync_pkt.extend_from_slice(&1u32.to_le_bytes());
        sync_pkt.extend_from_slice(&0u32.to_le_bytes());
        sync_pkt.extend_from_slice(&sync_csum.to_le_bytes());
        sync_pkt.extend_from_slice(&16u32.to_le_bytes());
        let mut sync_bs = [0u8; 16];
        sync_bs[0..2].copy_from_slice(&32u16.to_le_bytes());
        sync_bs[4..8].copy_from_slice(&(16u32 * 8).to_le_bytes());
        sync_pkt.extend_from_slice(&sync_bs);
        assert_eq!(dec.decode(&sync_pkt, &mut pic), Ok(false));

        let mut w = Writer::new();
        dec.snap(&mut w);
        let mut restored = Decoder::new();
        let mut r = Reader::new(&w.out);
        restored.snap(&mut r);
        assert!(!r.failed());
        assert_eq!(restored, dec);
    }

    #[test]
    fn rejects_bad_checksum_unsupported_version_and_corrupt_snaps() {
        let mut dec = Decoder::new();
        let mut pic = Picture::default();
        let mut bad = make_solid_frame(0, 16, 16);
        bad[8] ^= 0xff;
        assert!(dec.decode(&bad, &mut pic).is_err());

        let mut bad_ver = make_solid_frame(0, 16, 16);
        bad_ver[16] = 31;
        assert!(dec.decode(&bad_ver, &mut pic).is_err());

        // Corrupt snap: mismatched buffer length on a plane.
        let mut bogus_buf = Decoder {
            width: 16,
            height: 16,
            planes: [
                Plane {
                    width: 16,
                    height: 16,
                    pitch: 16,
                    buffers: [vec![0; 4], vec![0; 4]],
                },
                Plane::default(),
                Plane::default(),
            ],
        };
        // Corrupt snap: non-zero Decoder dimensions with empty planes.
        let mut bogus_planes = Decoder {
            width: 320,
            height: 200,
            planes: [Plane::default(), Plane::default(), Plane::default()],
        };
        for bogus in [&mut bogus_buf, &mut bogus_planes] {
            let mut w = Writer::new();
            bogus.snap(&mut w);
            let mut restored = Decoder::new();
            let mut r = Reader::new(&w.out);
            restored.snap(&mut r);
            assert!(r.failed());
        }
    }
}
