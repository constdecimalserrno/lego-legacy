// SPDX-License-Identifier: LGPL-2.1-or-later
// Derived from FFmpeg's Indeo 5 decoder, Copyright (c) 2009-2011 Maxim
// Poliakovski and the FFmpeg developers. See NOTICE.

//! Indeo Video Interactive 5, the codec the roster's movies are written in.
//!
//! The roster's movies are IV50 (#9, #372). The format: a little-endian
//! bitstream of pictures, each three planes at 4:1:0 (a luma plane and two
//! chroma planes a quarter its size each way). A plane is one band, or four
//! 5/3 wavelet luma bands in scalable mode. A band is tiles of macroblocks of
//! 8x8 (luma) or 4x4 (chroma) blocks: run-value coded coefficients through
//! per-band codebooks, dequantised, inverse slant transformed, and motion
//! compensated at half a pixel against sixteen-bit reference planes kept
//! between pictures.
//!
//! Written against FFmpeg's decoder read as the format's reference; the
//! constants are in `tables`. What a stream asks for that this does not
//! decode is an error that names it, never a picture that is not the movie.

mod tables;

use tables::{HuffDesc, RvMap};

/// Why a packet did not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IV50: {}", self.0)
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// A decoded picture: three planes, tightly packed, luma first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

impl Picture {
    /// The chroma planes' size: a quarter of the picture each way, rounded up.
    #[must_use]
    pub fn chroma_size(&self) -> (usize, usize) {
        (self.width.div_ceil(4), self.height.div_ceil(4))
    }

    /// One pixel's colour, as red, green and blue: what a codec of this era
    /// turned its pictures into for an RGB screen. The one conversion every
    /// movie path takes, DirectShow's (#9) and AVIFile's (#151). Chroma is
    /// taken from the sample its pixel sits in, without interpolation.
    #[must_use]
    pub fn rgb(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let (cw, _) = self.chroma_size();
        let at = (y / 4) * cw + x / 4;
        bt601(
            i32::from(self.y[y * self.width + x]),
            i32::from(self.u[at]),
            i32::from(self.v[at]),
        )
    }
}

/// ITU-R BT.601 at studio range, in the integer form Microsoft documents, in
/// "Converting 8-bit YUV to RGB888" on its page "Recommended 8-Bit YUV
/// Formats for Video Rendering". It is not derived from FFmpeg, whose own
/// converter gives other values (NOTICE).
fn bt601(y: i32, u: i32, v: i32) -> (u8, u8, u8) {
    let c = (y - 16) * 298;
    let (d, e) = (u - 128, v - 128);
    let clip = |x: i32| ((x + 128) >> 8).clamp(0, 255) as u8;
    (
        clip(c + 409 * e),
        clip(c - 100 * d - 208 * e),
        clip(c + 516 * d),
    )
}

const INTRA: u8 = 0;
const INTER: u8 = 1;
const INTER_SCALABLE: u8 = 2;
const NULL: u8 = 4;

/// The bits of a packet, least significant first within each byte.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
}

impl Bits<'_> {
    fn bits(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        if self.at + n as usize > self.data.len() * 8 {
            return Err(Error("the packet ends inside a field"));
        }
        let byte = self.at >> 3;
        let mut word = [0u8; 8];
        let take = (self.data.len() - byte).min(8);
        word[..take].copy_from_slice(&self.data[byte..byte + take]);
        let value = (u64::from_le_bytes(word) >> (self.at & 7)) & ((1u64 << n) - 1);
        self.at += n as usize;
        Ok(value as u32)
    }

    fn bit(&mut self) -> Result<bool> {
        self.bits(1).map(|b| b == 1)
    }

    fn align(&mut self) {
        self.at = (self.at + 7) & !7;
    }

    fn skip_extension(&mut self) -> Result<()> {
        loop {
            let len = self.bits(8)?;
            for _ in 0..len {
                self.bits(8)?;
            }
            if len == 0 {
                return Ok(());
            }
        }
    }
}

/// A codebook: row `i` is `i` one-bits, a zero unless it is the last row,
/// then that row's payload bits, most significant first. No more than 256
/// symbols exist, however many the rows would describe.
#[derive(Clone, Copy)]
struct Codebook {
    rows: usize,
    xbits: [u8; 16],
    base: [u32; 16],
    count: [u32; 16],
}

impl Codebook {
    fn new(rows: usize, xbits: [u8; 16]) -> Result<Self> {
        if rows == 0 || rows > 16 {
            return Err(Error("a codebook with no rows, or more than sixteen"));
        }
        let mut book = Self {
            rows,
            xbits,
            base: [0; 16],
            count: [0; 16],
        };
        let mut symbols = 0u32;
        for (row, &payload) in xbits.iter().enumerate().take(rows) {
            book.base[row] = symbols;
            if symbols >= 256 {
                continue;
            }
            let length = row + usize::from(payload) + usize::from(row + 1 != rows);
            if length > 13 {
                return Err(Error("a code longer than thirteen bits"));
            }
            book.count[row] = (1u32 << payload).min(256 - symbols);
            symbols += book.count[row];
        }
        Ok(book)
    }

    fn predefined(desc: &HuffDesc) -> Result<Self> {
        Self::new(desc.rows, desc.xbits)
    }

    /// A header's choice of codebook: the default when it does not say, one
    /// of seven by number, or one it spells out.
    fn chosen(bits: &mut Bits<'_>, coded: bool, set: &[HuffDesc; 8]) -> Result<Self> {
        if !coded {
            return Self::predefined(&set[7]);
        }
        match bits.bits(3)? {
            7 => {
                let rows = bits.bits(4)? as usize;
                let mut xbits = [0u8; 16];
                for x in xbits.iter_mut().take(rows) {
                    *x = bits.bits(4)? as u8;
                }
                Self::new(rows, xbits)
            }
            n => Self::predefined(&set[n as usize]),
        }
    }

    fn read(&self, bits: &mut Bits<'_>) -> Result<u32> {
        // One row with no payload is the single one-bit code 0.
        if self.rows == 1 && self.xbits[0] == 0 {
            return if bits.bit()? {
                Err(Error("a code the codebook does not have"))
            } else {
                Ok(0)
            };
        }
        let mut row = 0;
        while row + 1 < self.rows && bits.bit()? {
            row += 1;
        }
        let payload = bits.bits(u32::from(self.xbits[row]))?;
        let payload = reverse(payload, u32::from(self.xbits[row]));
        if payload >= self.count[row] {
            return Err(Error("a code past the codebook's 256 symbols"));
        }
        Ok(self.base[row] + payload)
    }
}

/// A field read least significant bit first, turned around.
fn reverse(value: u32, width: u32) -> u32 {
    if width == 0 {
        0
    } else {
        value.reverse_bits() >> (32 - width)
    }
}

/// A value whose sign is its lowest bit.
fn signed(value: u32) -> i32 {
    let v = value as i32;
    -((v >> 1) ^ -(v & 1))
}

fn scale_vector(v: i32, scale: i32) -> i32 {
    (v + i32::from(v > 0) + (scale - 1)) >> scale
}

#[derive(Clone, Copy, Default)]
struct Macroblock {
    x: usize,
    y: usize,
    offset: usize,
    inter: bool,
    cbp: u8,
    q_delta: i32,
    mv_x: i32,
    mv_y: i32,
}

struct Tile {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    blocks: Vec<Macroblock>,
}

/// Which inverse transform a band's coefficients go through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transform {
    Slant8x8,
    RowSlant8x8,
    ColSlant8x8,
    PutPixels8x8,
    Slant4x4,
}

impl Transform {
    const fn is_2d(self) -> bool {
        matches!(self, Self::Slant8x8 | Self::Slant4x4)
    }

    const fn scan(self) -> &'static [u8] {
        match self {
            Self::Slant8x8 => &tables::SCAN_ZIGZAG_8X8,
            Self::RowSlant8x8 => &tables::SCAN_VERTICAL_8X8,
            Self::ColSlant8x8 | Self::PutPixels8x8 => &tables::SCAN_HORIZONTAL_8X8,
            Self::Slant4x4 => &tables::SCAN_DIRECT_4X4,
        }
    }

    const fn size(self) -> usize {
        match self {
            Self::Slant4x4 => 4,
            Self::Slant8x8 | Self::RowSlant8x8 | Self::ColSlant8x8 | Self::PutPixels8x8 => 8,
        }
    }
}

struct Band {
    width: usize,
    height: usize,
    pitch: usize,
    aligned_height: usize,
    /// The reference planes, kept between pictures: two for standard mode plus
    /// a third used by scalable droppable inter frames.
    planes: [Vec<i16>; 3],
    halfpel: bool,
    mb_size: usize,
    blk_size: usize,
    transform: Transform,
    quant_mat: u8,
    scan: &'static [u8],
    intra_base: &'static [u16],
    inter_base: &'static [u16],
    intra_scale: &'static [u8],
    inter_scale: &'static [u8],
    tiles: Vec<Tile>,
}

impl Band {
    fn new(width: usize, height: usize, align: usize) -> Self {
        Self {
            width,
            height,
            pitch: width.div_ceil(align) * align,
            aligned_height: height.div_ceil(align) * align,
            planes: [Vec::new(), Vec::new(), Vec::new()],
            halfpel: false,
            mb_size: 0,
            blk_size: 0,
            transform: Transform::Slant8x8,
            quant_mat: 0,
            scan: &tables::SCAN_ZIGZAG_8X8,
            intra_base: &tables::BASE_QUANT_8X8_INTRA[0],
            inter_base: &tables::BASE_QUANT_8X8_INTER[0],
            intra_scale: &tables::SCALE_QUANT_8X8_INTRA[0],
            inter_scale: &tables::SCALE_QUANT_8X8_INTER[0],
            tiles: Vec::new(),
        }
    }

    fn apply_tables(&mut self) -> bool {
        self.scan = self.transform.scan();
        if self.blk_size == 8 {
            let idx = usize::from(self.quant_mat);
            if idx >= 5 {
                return false;
            }
            self.intra_base = &tables::BASE_QUANT_8X8_INTRA[idx];
            self.inter_base = &tables::BASE_QUANT_8X8_INTER[idx];
            self.intra_scale = &tables::SCALE_QUANT_8X8_INTRA[idx];
            self.inter_scale = &tables::SCALE_QUANT_8X8_INTER[idx];
        } else {
            self.intra_base = &tables::BASE_QUANT_4X4_INTRA;
            self.inter_base = &tables::BASE_QUANT_4X4_INTER;
            self.intra_scale = &tables::SCALE_QUANT_4X4_INTRA;
            self.inter_scale = &tables::SCALE_QUANT_4X4_INTER;
        }
        true
    }
}

/// What a band header says about this picture's band.
struct BandHeader {
    inherit_mv: bool,
    inherit_q: bool,
    q_present: bool,
    quant: i32,
    map: RvTable,
    book: Codebook,
}

/// A run-value map with the band's corrections applied.
struct RvTable {
    eob: u32,
    esc: u32,
    run: [u8; 256],
    val: [i16; 256],
}

impl RvTable {
    fn from(map: &RvMap) -> Self {
        Self {
            eob: map.eob,
            esc: map.esc,
            run: map.run,
            val: map.val,
        }
    }
}

/// The picture layout a GOP header describes.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Layout {
    width: usize,
    height: usize,
    tile_width: usize,
    tile_height: usize,
    luma_bands: usize,
}

/// One stream's decoder. Pictures depend on the ones before them, so a stream
/// is decoded in order by one of these from its first intra picture.
pub struct Decoder {
    layout: Option<Layout>,
    gop_valid: bool,
    decoded_any: bool,
    is_scalable: bool,
    inter_scal: bool,
    gop_flags: u32,
    frame_flags: u32,
    frame_type: u8,
    previous_type: u8,
    switch: usize,
    dst: usize,
    reference: usize,
    ref2: usize,
    mb_book: Option<Codebook>,
    /// Three planes: luma (1 or 4 bands) and the two chroma planes (1 band
    /// each), once a GOP header has said what they are.
    planes: [Vec<Band>; 3],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::snap::Snap for Layout {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            width,
            height,
            tile_width,
            tile_height,
            luma_bands,
        } = self;
        v.usize(width);
        v.usize(height);
        v.usize(tile_width);
        v.usize(tile_height);
        v.usize(luma_bands);
    }
}

impl crate::snap::Snap for Codebook {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            rows,
            xbits,
            base,
            count,
        } = self;
        v.usize(rows);
        v.bytes(xbits);
        for b in base {
            v.u32(b);
        }
        for c in count {
            v.u32(c);
        }
    }
}

impl crate::snap::Snap for Macroblock {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            x,
            y,
            offset,
            inter,
            cbp,
            q_delta,
            mv_x,
            mv_y,
        } = self;
        v.usize(x);
        v.usize(y);
        v.usize(offset);
        v.bool(inter);
        v.u8(cbp);
        v.i32(q_delta);
        v.i32(mv_x);
        v.i32(mv_y);
    }
}

impl crate::snap::Snap for Tile {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            x,
            y,
            width,
            height,
            blocks,
        } = self;
        v.usize(x);
        v.usize(y);
        v.usize(width);
        v.usize(height);
        crate::snap::snap_vec(v, blocks, |b, v| b.snap(v));
    }
}

impl crate::snap::Snap for Band {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            width,
            height,
            pitch,
            aligned_height,
            planes,
            halfpel,
            mb_size,
            blk_size,
            transform,
            quant_mat,
            scan: _,
            intra_base: _,
            inter_base: _,
            intra_scale: _,
            inter_scale: _,
            tiles,
        } = self;
        v.usize(width);
        v.usize(height);
        v.usize(pitch);
        v.usize(aligned_height);
        for plane in planes {
            crate::snap::snap_i16_vec(v, plane);
        }
        v.bool(halfpel);
        v.usize(mb_size);
        v.usize(blk_size);
        let mut t_tag = match transform {
            Transform::Slant8x8 => 0u8,
            Transform::RowSlant8x8 => 1u8,
            Transform::ColSlant8x8 => 2u8,
            Transform::PutPixels8x8 => 3u8,
            Transform::Slant4x4 => 4u8,
        };
        v.u8(&mut t_tag);
        v.u8(quant_mat);
        crate::snap::snap_vec_with(
            v,
            tiles,
            || Tile {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                blocks: Vec::new(),
            },
            |t, v| t.snap(v),
        );
        if v.is_reading() {
            match t_tag {
                0 => self.transform = Transform::Slant8x8,
                1 => self.transform = Transform::RowSlant8x8,
                2 => self.transform = Transform::ColSlant8x8,
                3 => self.transform = Transform::PutPixels8x8,
                4 => self.transform = Transform::Slant4x4,
                _ => {
                    v.mark_corrupt("iv50::Transform tag");
                    return;
                }
            }
            if (self.blk_size != 0 && self.transform.size() != self.blk_size)
                || !self.apply_tables()
            {
                v.mark_corrupt("iv50::Band quant_mat or blk_size");
            }
        }
    }
}

impl crate::snap::Snap for Decoder {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            layout,
            gop_valid,
            decoded_any,
            is_scalable,
            inter_scal,
            gop_flags,
            frame_flags,
            frame_type,
            previous_type,
            switch,
            dst,
            reference,
            ref2,
            mb_book,
            planes,
        } = self;
        crate::snap::snap_option(v, layout, |l, v| l.snap(v));
        v.bool(gop_valid);
        v.bool(decoded_any);
        v.bool(is_scalable);
        v.bool(inter_scal);
        v.u32(gop_flags);
        v.u32(frame_flags);
        v.u8(frame_type);
        v.u8(previous_type);
        v.usize(switch);
        v.usize(dst);
        v.usize(reference);
        v.usize(ref2);
        crate::snap::snap_option_with(
            v,
            mb_book,
            || Codebook {
                rows: 0,
                xbits: [0; 16],
                base: [0; 16],
                count: [0; 16],
            },
            |b, v| b.snap(v),
        );
        for plane in planes {
            crate::snap::snap_vec_with(v, plane, || Band::new(0, 0, 1), |b, v| b.snap(v));
        }
        if v.is_reading() && (*dst >= 3 || *reference >= 3 || *ref2 >= 3) {
            v.mark_corrupt("iv50::Decoder buffer index");
        }
    }
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            layout: None,
            gop_valid: false,
            decoded_any: false,
            is_scalable: false,
            inter_scal: false,
            gop_flags: 0,
            frame_flags: 0,
            frame_type: INTRA,
            previous_type: INTRA,
            switch: 0,
            dst: 0,
            reference: 1,
            ref2: 0,
            mb_book: None,
            planes: [Vec::new(), Vec::new(), Vec::new()],
        }
    }

    /// Decode one packet into `out`.
    ///
    /// Answers whether `out` holds a new picture: a null packet repeats the
    /// last one and leaves `out` as it was.
    ///
    /// # Errors
    /// When the packet is not IV50 this decoder can read, or is corrupt.
    pub fn decode(&mut self, packet: &[u8], out: &mut Picture) -> Result<bool> {
        let mut bits = Bits {
            data: packet,
            at: 0,
        };
        if bits.bits(5)? != 0x1f {
            return Err(Error("no picture start code"));
        }
        self.previous_type = self.frame_type;
        self.frame_type = bits.bits(3)? as u8;
        if self.frame_type > NULL {
            self.frame_type = INTRA;
            return Err(Error("a picture type that does not exist"));
        }
        let _number = bits.bits(8)?;
        if self.frame_type == INTRA {
            self.gop_valid = false;
            self.gop_header(&mut bits)?;
            self.gop_valid = true;
        }
        if self.frame_type == INTER_SCALABLE && !self.is_scalable {
            self.frame_type = INTER;
            return Err(Error("a scalable picture in a stream that is not"));
        }
        if self.frame_type != NULL {
            self.frame_flags = bits.bits(8)?;
            if self.frame_flags & 0x01 != 0 {
                bits.bits(24)?;
            }
            if self.frame_flags & 0x10 != 0 {
                bits.bits(16)?;
            }
            if self.frame_flags & 0x20 != 0 {
                bits.skip_extension()?;
            }
            self.mb_book = Some(Codebook::chosen(
                &mut bits,
                self.frame_flags & 0x40 != 0,
                &tables::MB_HUFF,
            )?);
            bits.bits(3)?;
        }
        bits.align();

        let Some(layout) = self.layout.filter(|_| self.gop_valid) else {
            return Err(Error("no intra picture has set the stream up yet"));
        };
        if self.gop_flags & 0x20 != 0 {
            return Err(Error("a password-protected stream"));
        }
        self.switch_buffers();

        if self.frame_type == NULL {
            if self.is_scalable {
                return Err(Error("a null picture in scalable mode"));
            }
            if !self.decoded_any {
                return Err(Error("a null picture with nothing to repeat"));
            }
            return Ok(false);
        }

        let book = self.mb_book.ok_or(Error("no macroblock codebook"))?;
        let base_mb_size = self.planes[0][0].mb_size;
        for plane in 0..3 {
            let num_bands = self.planes[plane].len();
            for band_idx in 0..num_bands {
                let context = Context {
                    plane,
                    band: band_idx,
                    is_scalable: self.is_scalable,
                    frame_flags: self.frame_flags,
                    frame_type: self.frame_type,
                    base_mb_size,
                    dst: self.dst,
                    reference: self.reference,
                };
                if plane == 0 && band_idx == 0 {
                    decode_band(&mut bits, &mut self.planes[0][0], None, &book, &context)?;
                } else if plane == 0 {
                    let (head, tail) = self.planes[0].split_at_mut(band_idx);
                    decode_band(
                        &mut bits,
                        &mut tail[0],
                        Some(&head[0].tiles),
                        &book,
                        &context,
                    )?;
                } else {
                    let (head, tail) = self.planes.split_at_mut(plane);
                    decode_band(
                        &mut bits,
                        &mut tail[0][band_idx],
                        Some(&head[0][0].tiles),
                        &book,
                        &context,
                    )?;
                }
            }
        }
        self.decoded_any = true;

        out.width = layout.width;
        out.height = layout.height;
        let (cw, ch) = out.chroma_size();
        out.y.resize(layout.width * layout.height, 0);
        out.u.resize(cw * ch, 0);
        out.v.resize(cw * ch, 0);
        if self.is_scalable {
            recompose53(
                &self.planes[0],
                self.dst,
                &mut out.y,
                layout.width,
                layout.height,
            );
        } else {
            copy_plane(
                &self.planes[0][0],
                self.dst,
                &mut out.y,
                layout.width,
                layout.height,
            );
        }
        // The first chroma plane in the stream is V, the second U.
        copy_plane(&self.planes[2][0], self.dst, &mut out.u, cw, ch);
        copy_plane(&self.planes[1][0], self.dst, &mut out.v, cw, ch);
        Ok(true)
    }

    fn switch_buffers(&mut self) {
        match self.previous_type {
            INTRA | INTER => {
                self.switch ^= 1;
                self.dst = self.switch;
                self.reference = self.switch ^ 1;
            }
            INTER_SCALABLE => {
                if !self.inter_scal {
                    self.ref2 = 2;
                    self.inter_scal = true;
                }
                std::mem::swap(&mut self.dst, &mut self.ref2);
                self.reference = self.ref2;
            }
            _ => {}
        }
        match self.frame_type {
            INTRA => {
                self.switch = 0;
                self.inter_scal = false;
                self.dst = self.switch;
                self.reference = self.switch ^ 1;
            }
            INTER => {
                self.inter_scal = false;
                self.dst = self.switch;
                self.reference = self.switch ^ 1;
            }
            _ => {}
        }
    }

    fn gop_header(&mut self, bits: &mut Bits<'_>) -> Result<()> {
        self.gop_flags = bits.bits(8)?;
        if self.gop_flags & 0x01 != 0 {
            bits.bits(16)?;
        }
        if self.gop_flags & 0x20 != 0 {
            bits.bits(32)?;
        }
        let tile_size = if self.gop_flags & 0x40 != 0 {
            64usize << bits.bits(2)?
        } else {
            0
        };
        if tile_size > 256 {
            return Err(Error("a tile larger than 256"));
        }
        let luma_bands = bits.bits(2)? as usize * 3 + 1;
        let chroma_bands = bits.bits(1)? as usize * 3 + 1;
        let is_scalable = luma_bands != 1 || chroma_bands != 1;
        if is_scalable && (luma_bands != 4 || chroma_bands != 1) {
            return Err(Error("a band subdivision other than the scalable one"));
        }
        let size = bits.bits(4)? as usize;
        let (height, width) = if size == 15 {
            (bits.bits(13)? as usize, bits.bits(13)? as usize)
        } else {
            (
                usize::from(tables::COMMON_PIC_SIZES[size * 2 + 1]) << 2,
                usize::from(tables::COMMON_PIC_SIZES[size * 2]) << 2,
            )
        };
        if width == 0 || height == 0 {
            return Err(Error("a picture with no size"));
        }
        if self.gop_flags & 0x02 != 0 {
            return Err(Error("the YV12 layout"));
        }
        let layout = Layout {
            width,
            height,
            tile_width: if tile_size == 0 { width } else { tile_size },
            tile_height: if tile_size == 0 { height } else { tile_size },
            luma_bands,
        };
        let reconfigured = self.layout != Some(layout);
        if reconfigured {
            let (lw, lh) = if luma_bands == 1 {
                (width, height)
            } else {
                (width.div_ceil(2), height.div_ceil(2))
            };
            let (cw, ch) = (width.div_ceil(4), height.div_ceil(4));
            self.planes = [
                (0..luma_bands).map(|_| Band::new(lw, lh, 16)).collect(),
                vec![Band::new(cw, ch, 8)],
                vec![Band::new(cw, ch, 8)],
            ];
            self.layout = Some(layout);
            self.is_scalable = is_scalable;
        }

        let mut rebuild = reconfigured;
        for plane in 0..2 {
            let count = if plane == 0 { luma_bands } else { 1 };
            for i in 0..count {
                let band = &mut self.planes[plane][i];
                band.halfpel = bits.bit()?;
                let mb_bit = bits.bits(1)?;
                let blk_size = 8usize >> bits.bits(1)?;
                let mb_size = blk_size << (mb_bit ^ 1);
                if plane == 0 && blk_size == 4 {
                    return Err(Error("4x4 luma blocks"));
                }
                rebuild |=
                    mb_size != band.mb_size || blk_size != band.blk_size || band.tiles.is_empty();
                band.mb_size = mb_size;
                band.blk_size = blk_size;
                if bits.bit()? {
                    return Err(Error("extended transform information"));
                }
                let transform = match (plane, i) {
                    (0, 0) => Transform::Slant8x8,
                    (0, 1) => Transform::RowSlant8x8,
                    (0, 2) => Transform::ColSlant8x8,
                    (0, 3) => Transform::PutPixels8x8,
                    (1, 0) => Transform::Slant4x4,
                    _ => return Err(Error("unsupported plane/band transform")),
                };
                if transform.size() != blk_size {
                    return Err(Error("a transform the size of no block"));
                }
                band.transform = transform;
                band.quant_mat = if plane == 0 {
                    if luma_bands > 1 {
                        (i + 1) as u8
                    } else {
                        0
                    }
                } else {
                    5
                };
                if !band.apply_tables() {
                    return Err(Error("quantisation matrix out of range"));
                }
                if bits.bits(2)? != 0 {
                    return Err(Error("a band description without its end marker"));
                }
            }
        }
        // The second chroma plane is described by the first.
        let (first, second) = self.planes.split_at_mut(2);
        let (from, to) = (&first[1][0], &mut second[0][0]);
        to.halfpel = from.halfpel;
        to.mb_size = from.mb_size;
        to.blk_size = from.blk_size;
        to.transform = from.transform;
        to.quant_mat = from.quant_mat;
        to.scan = from.scan;
        to.intra_base = from.intra_base;
        to.inter_base = from.inter_base;
        to.intra_scale = from.intra_scale;
        to.inter_scale = from.inter_scale;
        if rebuild {
            self.build_tiles(layout)?;
        }
        if self.gop_flags & 0x08 != 0 {
            if bits.bits(3)? != 0 {
                return Err(Error("alignment bits that are not zero"));
            }
            if bits.bit()? {
                bits.bits(24)?;
            }
        }
        bits.align();
        bits.bits(23)?;
        if bits.bit()? {
            while bits.bits(16)? & 0x8000 != 0 {}
        }
        bits.align();
        Ok(())
    }

    fn build_tiles(&mut self, layout: Layout) -> Result<()> {
        for plane in 0..3 {
            let (mut tile_width, mut tile_height) = if plane == 0 {
                (layout.tile_width, layout.tile_height)
            } else {
                (
                    layout.tile_width.div_ceil(4),
                    layout.tile_height.div_ceil(4),
                )
            };
            if plane == 0 && layout.luma_bands == 4 {
                if !tile_width.is_multiple_of(2) || !tile_height.is_multiple_of(2) {
                    return Err(Error("odd tile dimensions in scalable mode"));
                }
                tile_width >>= 1;
                tile_height >>= 1;
            }
            if tile_width == 0 || tile_height == 0 {
                return Err(Error("zero tile dimensions"));
            }
            let num_bands = self.planes[plane].len();
            for b in 0..num_bands {
                let band = &self.planes[plane][b];
                let mut tiles = Vec::new();
                for y in (0..band.height).step_by(tile_height) {
                    for x in (0..band.width).step_by(tile_width) {
                        let width = tile_width.min(band.width - x);
                        let height = tile_height.min(band.height - y);
                        let count = width.div_ceil(band.mb_size) * height.div_ceil(band.mb_size);
                        tiles.push(Tile {
                            x,
                            y,
                            width,
                            height,
                            blocks: vec![Macroblock::default(); count],
                        });
                    }
                }
                if plane != 0 || b != 0 {
                    let base = &self.planes[0][0].tiles;
                    if base.len() != tiles.len()
                        || base
                            .iter()
                            .zip(&tiles)
                            .any(|(a, b)| a.blocks.len() != b.blocks.len())
                    {
                        return Err(Error("a band whose tiles do not line up with luma's"));
                    }
                }
                self.planes[plane][b].tiles = tiles;
            }
        }
        Ok(())
    }
}

/// What a band is decoded against.
struct Context {
    plane: usize,
    band: usize,
    is_scalable: bool,
    frame_flags: u32,
    frame_type: u8,
    base_mb_size: usize,
    dst: usize,
    reference: usize,
}

impl Context {
    fn is_base(&self) -> bool {
        self.plane == 0 && self.band == 0
    }
}

fn decode_band(
    bits: &mut Bits<'_>,
    band: &mut Band,
    reference: Option<&[Tile]>,
    mb_book: &Codebook,
    cx: &Context,
) -> Result<()> {
    if cx.dst == cx.reference {
        return Err(Error("destination and reference buffers alias"));
    }
    if !cx.is_scalable && (cx.dst == 2 || cx.reference == 2) {
        return Err(Error("scalable buffer in non-scalable stream"));
    }
    for i in [cx.dst, cx.reference] {
        if band.planes[i].is_empty() {
            band.planes[i] = vec![0; band.pitch * band.aligned_height];
        }
    }

    let flags = bits.bits(8)?;
    if flags & 0x01 != 0 {
        return Err(Error("an empty band"));
    }
    if cx.frame_flags & 0x80 != 0 {
        bits.bits(24)?;
    }
    let q_present = flags & 0x04 != 0;
    let mut corrections = Vec::new();
    if flags & 0x10 != 0 {
        let count = bits.bits(8)?;
        if count > 61 {
            return Err(Error("more than 61 run-value corrections"));
        }
        for _ in 0..count * 2 {
            corrections.push(bits.bits(8)? as usize);
        }
    }
    let map = if flags & 0x40 != 0 {
        bits.bits(3)? as usize
    } else {
        8
    };
    let book = Codebook::chosen(bits, flags & 0x80 != 0, &tables::BLK_HUFF)?;
    if bits.bit()? {
        bits.bits(16)?;
    }
    let quant = bits.bits(5)? as i32;
    if flags & 0x20 != 0 {
        bits.align();
        bits.skip_extension()?;
    }
    bits.align();

    let mut table = RvTable::from(&tables::RV_MAPS[map]);
    for pair in corrections.chunks(2) {
        let (a, b) = (pair[0], pair[1]);
        table.run.swap(a, b);
        table.val.swap(a, b);
        let delta = (a ^ b) as u32;
        if a as u32 == table.eob || b as u32 == table.eob {
            table.eob ^= delta;
        }
        if a as u32 == table.esc || b as u32 == table.esc {
            table.esc ^= delta;
        }
    }
    let header = BandHeader {
        inherit_mv: flags & 0x02 != 0,
        inherit_q: flags & 0x08 != 0 || !q_present,
        q_present,
        quant,
        map: table,
        book,
    };

    let scale = (cx.base_mb_size >> 3) as i32 - (band.mb_size >> 3) as i32;
    let mut start = bits.at;
    for t in 0..band.tiles.len() {
        if cx.base_mb_size < band.mb_size {
            return Err(Error("a band with larger macroblocks than luma's"));
        }
        let reference = match reference {
            Some(r) if r[t].blocks.len() != band.tiles[t].blocks.len() => {
                return Err(Error("a tile unlike its reference"));
            }
            Some(r) => Some(&r[t].blocks[..]),
            None => None,
        };
        if bits.bit()? {
            empty_tile(band, t, reference, &header, scale, cx)?;
        } else {
            let mut size = 0;
            if bits.bit()? {
                size = bits.bits(8)? as usize;
                if size == 255 {
                    size = bits.bits(24)? as usize;
                }
            }
            bits.align();
            if size == 0 {
                return Err(Error("a tile with no data"));
            }
            macroblocks(bits, band, t, reference, mb_book, &header, scale, cx)?;
            blocks(bits, band, t, &header, cx)?;
            if (bits.at - start) >> 3 != size {
                return Err(Error("a tile whose data is not the size it says"));
            }
            start += size << 3;
        }
    }
    bits.align();
    Ok(())
}

/// The shape of a band, apart from its planes.
#[derive(Clone, Copy)]
struct Shape {
    pitch: usize,
    aligned_height: usize,
    mb_size: usize,
    halfpel: bool,
}

impl Band {
    fn shape(&self) -> Shape {
        Shape {
            pitch: self.pitch,
            aligned_height: self.aligned_height,
            mb_size: self.mb_size,
            halfpel: self.halfpel,
        }
    }
}

/// Where a motion vector lands must be inside the reference plane.
fn vector_fits(band: Shape, mb: &Macroblock) -> bool {
    let s = i32::from(band.halfpel);
    let (x, y) = (mb.x as i32, mb.y as i32);
    let size = band.mb_size as i32;
    let (dx, dy) = (mb.mv_x >> s, mb.mv_y >> s);
    let (cx, cy) = (mb.mv_x & s, mb.mv_y & s);
    x + dx >= 0
        && x + dx + size + cx <= band.pitch as i32
        && y + dy >= 0
        && y + dy + size + cy <= band.aligned_height as i32
}

fn inherit(mb: &mut Macroblock, from: &Macroblock, scale: i32) {
    if scale == 0 {
        mb.mv_x = from.mv_x;
        mb.mv_y = from.mv_y;
    } else {
        mb.mv_x = scale_vector(from.mv_x, scale);
        mb.mv_y = scale_vector(from.mv_y, scale);
    }
}

#[allow(clippy::too_many_arguments)]
fn macroblocks(
    bits: &mut Bits<'_>,
    band: &mut Band,
    t: usize,
    reference: Option<&[Macroblock]>,
    book: &Codebook,
    header: &BandHeader,
    scale: i32,
    cx: &Context,
) -> Result<()> {
    if reference.is_none() && ((header.q_present && header.inherit_q) || header.inherit_mv) {
        return Err(Error("inheritance with nothing to inherit from"));
    }
    let (pitch, mb_size, blk_size) = (band.pitch, band.mb_size, band.blk_size);
    let s = i32::from(band.halfpel);
    let limit = (band.pitch * band.aligned_height) as i32;
    let per_mb = if mb_size == blk_size { 1 } else { 4 };
    let qdelta_everywhere = cx.is_base() && cx.frame_flags & 0x08 != 0;
    let tile = &mut band.tiles[t];
    let (mut mv_x, mut mv_y) = (0, 0);
    let mut n = 0;
    for y in (tile.y..tile.y + tile.height).step_by(mb_size) {
        for x in (tile.x..tile.x + tile.width).step_by(mb_size) {
            let from = reference.map(|r| r[n]);
            let mb = &mut tile.blocks[n];
            *mb = Macroblock {
                x,
                y,
                offset: y * pitch + x,
                ..Macroblock::default()
            };
            if bits.bit()? {
                if cx.frame_type == INTRA {
                    return Err(Error("an empty macroblock in an intra picture"));
                }
                mb.inter = true;
                if qdelta_everywhere {
                    mb.q_delta = signed(book.read(bits)?);
                }
                if let (true, Some(from)) = (header.inherit_mv, from) {
                    inherit(mb, &from, scale);
                }
            } else {
                mb.inter = match (header.inherit_mv, from) {
                    (true, Some(from)) => from.inter,
                    _ if cx.frame_type == INTRA => false,
                    _ => bits.bit()?,
                };
                mb.cbp = bits.bits(per_mb)? as u8;
                if header.q_present {
                    if header.inherit_q {
                        mb.q_delta = from.map_or(0, |f| f.q_delta);
                    } else if mb.cbp != 0 || qdelta_everywhere {
                        mb.q_delta = signed(book.read(bits)?);
                    }
                }
                if mb.inter {
                    if let (true, Some(from)) = (header.inherit_mv, from) {
                        inherit(mb, &from, scale);
                    } else {
                        mv_y += signed(book.read(bits)?);
                        mv_x += signed(book.read(bits)?);
                        mb.mv_x = mv_x;
                        mb.mv_y = mv_y;
                    }
                }
            }
            if mb.inter {
                let (x, y, size) = (x as i32, y as i32, mb_size as i32);
                let low = x + (mb.mv_x >> s) + (y + (mb.mv_y >> s)) * pitch as i32;
                let high = x + ((mb.mv_x + s) >> s) + size - 1
                    + (y + size - 1 + ((mb.mv_y + s) >> s)) * pitch as i32;
                if low < 0 || high > limit - 1 {
                    return Err(Error("a motion vector outside the reference"));
                }
            }
            n += 1;
        }
    }
    bits.align();
    Ok(())
}

fn empty_tile(
    band: &mut Band,
    t: usize,
    reference: Option<&[Macroblock]>,
    header: &BandHeader,
    scale: i32,
    cx: &Context,
) -> Result<()> {
    let clear_first = !header.q_present && cx.is_base();
    let (pitch, mb_size) = (band.pitch, band.mb_size);
    let mut moved = false;
    let mut n = 0;
    let tile = &band.tiles[t];
    let (tx, ty, tw, th) = (tile.x, tile.y, tile.width, tile.height);
    for y in (ty..ty + th).step_by(mb_size) {
        for x in (tx..tx + tw).step_by(mb_size) {
            let mut mb = band.tiles[t].blocks[n];
            mb.x = x;
            mb.y = y;
            mb.offset = y * pitch + x;
            mb.inter = true;
            mb.cbp = 0;
            if clear_first {
                mb.q_delta = header.quant;
                mb.mv_x = 0;
                mb.mv_y = 0;
            }
            if let Some(from) = reference.map(|r| r[n]) {
                if header.inherit_q {
                    mb.q_delta = from.q_delta;
                }
                if header.inherit_mv {
                    inherit(&mut mb, &from, scale);
                    moved |= mb.mv_x != 0 || mb.mv_y != 0;
                    if !vector_fits(band.shape(), &mb) {
                        return Err(Error("an inherited motion vector out of bounds"));
                    }
                }
            }
            band.tiles[t].blocks[n] = mb;
            n += 1;
        }
    }

    let (dst, src) = two_planes(&mut band.planes, cx.dst, cx.reference);
    if header.inherit_mv && moved {
        let per_mb = if mb_size == band.blk_size { 1 } else { 4 };
        for mb in &band.tiles[t].blocks {
            let (mv_x, mv_y, kind) = halfpel_split(band.halfpel, mb.mv_x, mb.mv_y);
            for blk in 0..per_mb {
                let offset =
                    mb.offset + band.blk_size * ((blk & 1) + usize::from(blk & 2 != 0) * pitch);
                motion(
                    dst,
                    src,
                    offset,
                    mv_x,
                    mv_y,
                    kind,
                    pitch,
                    band.aligned_height,
                    band.blk_size,
                    false,
                )?;
            }
        }
    } else {
        for row in 0..th {
            let at = (ty + row) * pitch + tx;
            dst[at..at + tw].copy_from_slice(&src[at..at + tw]);
        }
    }
    Ok(())
}

fn halfpel_split(halfpel: bool, mv_x: i32, mv_y: i32) -> (i32, i32, i32) {
    if halfpel {
        (mv_x >> 1, mv_y >> 1, ((mv_y & 1) << 1) | (mv_x & 1))
    } else {
        (mv_x, mv_y, 0)
    }
}

fn two_planes(planes: &mut [Vec<i16>; 3], dst: usize, src: usize) -> (&mut [i16], &[i16]) {
    let (low, high) = planes.split_at_mut(dst.max(src));
    if dst < src {
        (&mut low[dst], &high[0])
    } else {
        (&mut high[0], &low[src])
    }
}

fn blocks(
    bits: &mut Bits<'_>,
    band: &mut Band,
    t: usize,
    header: &BandHeader,
    cx: &Context,
) -> Result<()> {
    let (pitch, height, blk_size) = (band.pitch, band.aligned_height, band.blk_size);
    let per_mb = if band.mb_size == blk_size { 1 } else { 4 };
    let shape = band.shape();
    let (transform, scan) = (band.transform, band.scan);
    let mut prev_dc = 0i32;
    let (dst, src) = two_planes(&mut band.planes, cx.dst, cx.reference);
    for mb in &band.tiles[t].blocks {
        let intra = !mb.inter;
        let q = (header.quant + mb.q_delta).clamp(0, 23) as usize;
        let quant = u32::from(if intra {
            band.intra_scale[q]
        } else {
            band.inter_scale[q]
        });
        let base = if intra {
            band.intra_base
        } else {
            band.inter_base
        };
        let (mv_x, mv_y, kind) = if intra {
            (0, 0, 0)
        } else {
            if !vector_fits(shape, mb) {
                return Err(Error("a block's motion vector out of bounds"));
            }
            halfpel_split(band.halfpel, mb.mv_x, mb.mv_y)
        };
        let mut offset = mb.offset;
        let mut cbp = mb.cbp;
        for blk in 0..per_mb {
            if blk & 1 != 0 {
                offset += blk_size;
            } else if blk == 2 {
                offset = offset - blk_size + blk_size * pitch;
            }
            if (blk_size - 1) * pitch + blk_size > pitch * height - offset {
                return Err(Error("a block outside its band"));
            }
            if cbp & 1 != 0 {
                let mut coeffs = [0i32; 64];
                let mut columns = [false; 8];
                coefficients(
                    bits,
                    blk_size,
                    scan,
                    header,
                    base,
                    quant,
                    &mut coeffs,
                    &mut columns,
                )?;
                if intra && transform.is_2d() {
                    prev_dc += coeffs[0];
                    coeffs[0] = prev_dc;
                    columns[0] |= prev_dc != 0;
                }
                inverse(transform, &coeffs, &mut dst[offset..], pitch, &columns);
                if !intra {
                    motion(
                        dst, src, offset, mv_x, mv_y, kind, pitch, height, blk_size, true,
                    )?;
                }
            } else if intra {
                dc_only(transform, prev_dc, &mut dst[offset..], pitch, blk_size);
            } else {
                motion(
                    dst, src, offset, mv_x, mv_y, kind, pitch, height, blk_size, false,
                )?;
            }
            cbp >>= 1;
        }
    }
    bits.align();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn coefficients(
    bits: &mut Bits<'_>,
    blk_size: usize,
    scan: &[u8],
    header: &BandHeader,
    base: &[u16],
    quant: u32,
    coeffs: &mut [i32; 64],
    columns: &mut [bool; 8],
) -> Result<()> {
    let count = (blk_size * blk_size) as i32;
    let mask = blk_size - 1;
    let map = &header.map;
    let mut at = -1i32;
    let mut symbol;
    loop {
        symbol = header.book.read(bits)?;
        if symbol == map.eob {
            break;
        }
        let (run, value) = if symbol == map.esc {
            let run = header.book.read(bits)? as i32 + 1;
            let low = header.book.read(bits)?;
            let high = header.book.read(bits)?;
            (run, signed((high << 6) | low))
        } else {
            if symbol >= 256 {
                return Err(Error("a run-value symbol past the map"));
            }
            (
                i32::from(map.run[symbol as usize]),
                i32::from(map.val[symbol as usize]),
            )
        };
        at += run;
        if at >= count || at < 0 {
            break;
        }
        let pos = usize::from(scan[at as usize]);
        let q = (u32::from(base[pos]) * quant) >> 9;
        let mut v = value;
        if q > 1 {
            let sign = if v > 0 { 1 } else { -1 };
            v = v * q as i32 + sign * (((q ^ 1) as i32 - 1) >> 1);
        }
        coeffs[pos] = v;
        columns[pos & mask] |= v != 0;
    }
    if at < 0 || (at >= count && symbol != map.eob) {
        return Err(Error("corrupt block data"));
    }
    Ok(())
}

/// The butterfly every slant transform is built from.
fn butterfly(a: i32, b: i32) -> (i32, i32) {
    (a + b, a - b)
}

/// The slant transform's reflection by a half and five quarters.
fn reflect(a: i32, b: i32) -> (i32, i32) {
    (((a + b * 2 + 2) >> 2) + a, ((a * 2 - b + 2) >> 2) - b)
}

/// The slant transform's reflection by a half and seven eighths.
fn reflect4(a: i32, b: i32) -> (i32, i32) {
    (b + ((a * 4 - b + 4) >> 3), a + ((-a - b * 4 + 4) >> 3))
}

/// One inverse eight-point slant. `round` halves the result, which only the
/// last pass of a transform does.
fn slant8(c: [i32; 8], round: bool) -> [i32; 8] {
    let [s1, s4, s8, s5, s2, s6, s3, s7] = c;
    let (t4, t5) = reflect4(s4, s5);
    let (t1, t5) = butterfly(s1, t5);
    let (t2, t6) = butterfly(s2, s6);
    let (t7, t3) = butterfly(s7, s3);
    let (t4, t8) = butterfly(t4, s8);
    let (t1, t2) = butterfly(t1, t2);
    let (t4, t3) = reflect(t4, t3);
    let (t5, t6) = butterfly(t5, t6);
    let (t8, t7) = reflect(t8, t7);
    let (t1, t4) = butterfly(t1, t4);
    let (t2, t3) = butterfly(t2, t3);
    let (t5, t8) = butterfly(t5, t8);
    let (t6, t7) = butterfly(t6, t7);
    [t1, t2, t3, t4, t5, t6, t7, t8].map(|t| if round { (t + 1) >> 1 } else { t })
}

fn slant4(c: [i32; 4], round: bool) -> [i32; 4] {
    let [s1, s4, s2, s3] = c;
    let (t1, t2) = butterfly(s1, s2);
    let (t4, t3) = reflect(s4, s3);
    let (t1, t4) = butterfly(t1, t4);
    let (t2, t3) = butterfly(t2, t3);
    [t1, t2, t3, t4].map(|t| if round { (t + 1) >> 1 } else { t })
}

fn inverse(
    transform: Transform,
    c: &[i32; 64],
    out: &mut [i16],
    pitch: usize,
    columns: &[bool; 8],
) {
    match transform {
        Transform::Slant8x8 => {
            let mut tmp = [0i32; 64];
            for i in (0..8).filter(|i| columns[*i]) {
                let column = slant8(std::array::from_fn(|k| c[i + 8 * k]), false);
                for (k, v) in column.into_iter().enumerate() {
                    tmp[i + 8 * k] = v;
                }
            }
            for row in 0..8 {
                let line: [i32; 8] = std::array::from_fn(|k| tmp[8 * row + k]);
                let at = row * pitch;
                if line == [0; 8] {
                    out[at..at + 8].fill(0);
                } else {
                    for (k, v) in slant8(line, true).into_iter().enumerate() {
                        out[at + k] = v as i16;
                    }
                }
            }
        }
        Transform::RowSlant8x8 => {
            for row in 0..8 {
                let line: [i32; 8] = std::array::from_fn(|k| c[8 * row + k]);
                let at = row * pitch;
                if line == [0; 8] {
                    out[at..at + 8].fill(0);
                } else {
                    for (k, v) in slant8(line, true).into_iter().enumerate() {
                        out[at + k] = v as i16;
                    }
                }
            }
        }
        Transform::ColSlant8x8 => {
            for i in 0..8 {
                if columns[i] {
                    let column = slant8(std::array::from_fn(|k| c[i + 8 * k]), true);
                    for (k, v) in column.into_iter().enumerate() {
                        out[k * pitch + i] = v as i16;
                    }
                } else {
                    for k in 0..8 {
                        out[k * pitch + i] = 0;
                    }
                }
            }
        }
        Transform::PutPixels8x8 => {
            for row in 0..8 {
                let at = row * pitch;
                for k in 0..8 {
                    out[at + k] = c[8 * row + k] as i16;
                }
            }
        }
        Transform::Slant4x4 => {
            let mut tmp = [0i32; 16];
            for i in (0..4).filter(|i| columns[*i]) {
                let column = slant4(std::array::from_fn(|k| c[i + 4 * k]), false);
                for (k, v) in column.into_iter().enumerate() {
                    tmp[i + 4 * k] = v;
                }
            }
            for row in 0..4 {
                let line: [i32; 4] = std::array::from_fn(|k| tmp[4 * row + k]);
                let at = row * pitch;
                if line == [0; 4] {
                    out[at..at + 4].fill(0);
                } else {
                    for (k, v) in slant4(line, true).into_iter().enumerate() {
                        out[at + k] = v as i16;
                    }
                }
            }
        }
    }
}

/// An intra block with no coefficients: the running DC, spread over the
/// block as the band's transform spreads it.
fn dc_only(transform: Transform, dc: i32, out: &mut [i16], pitch: usize, size: usize) {
    let half = ((dc + 1) >> 1) as i16;
    match transform {
        Transform::Slant8x8 | Transform::Slant4x4 => {
            for row in 0..size {
                out[row * pitch..row * pitch + size].fill(half);
            }
        }
        Transform::RowSlant8x8 => {
            out[..size].fill(half);
            for row in 1..size {
                out[row * pitch..row * pitch + size].fill(0);
            }
        }
        Transform::ColSlant8x8 => {
            for row in 0..size {
                let at = row * pitch;
                out[at] = half;
                out[at + 1..at + size].fill(0);
            }
        }
        Transform::PutPixels8x8 => {
            out[0] = dc as i16;
            out[1..size].fill(0);
            for row in 1..size {
                out[row * pitch..row * pitch + size].fill(0);
            }
        }
    }
}

/// Take a block from the reference plane, at a whole or half pixel, and put
/// it in place or add it to the residual already there.
#[allow(clippy::too_many_arguments)]
fn motion(
    dst: &mut [i16],
    src: &[i16],
    offset: usize,
    mv_x: i32,
    mv_y: i32,
    kind: i32,
    pitch: usize,
    height: usize,
    size: usize,
    add: bool,
) -> Result<()> {
    let from = offset as i64 + i64::from(mv_y) * pitch as i64 + i64::from(mv_x);
    let total = (pitch * height) as i64;
    let span = (pitch * (size - 1) + size) as i64;
    let reach = i64::from(kind > 1) * pitch as i64 + i64::from(kind & 1);
    if from < 0 || total - span < offset as i64 || total - span - reach < from {
        return Err(Error("motion compensation outside the reference"));
    }
    let from = from as usize;
    for row in 0..size {
        let (d, r) = (offset + row * pitch, from + row * pitch);
        for j in 0..size {
            let a = i32::from(src[r + j]);
            let v = match kind {
                0 => a,
                1 => (a + i32::from(src[r + j + 1])) >> 1,
                2 => (a + i32::from(src[r + pitch + j])) >> 1,
                _ => {
                    (a + i32::from(src[r + j + 1])
                        + i32::from(src[r + pitch + j])
                        + i32::from(src[r + pitch + j + 1]))
                        >> 2
                }
            } as i16;
            dst[d + j] = if add { dst[d + j].wrapping_add(v) } else { v };
        }
    }
    Ok(())
}

/// A band's plane as pixels: the encoder took 128 away, and this puts it
/// back.
fn copy_plane(band: &Band, dst: usize, out: &mut [u8], width: usize, height: usize) {
    let plane = &band.planes[dst];
    for y in 0..height {
        let row = &plane[y * band.pitch..y * band.pitch + width];
        for (o, v) in out[y * width..(y + 1) * width].iter_mut().zip(row) {
            *o = (i32::from(*v) + 128).clamp(0, 255) as u8;
        }
    }
}

/// Recompose four 5/3 wavelet luma bands (LL, HL, LH, HH) into full-resolution
/// 8-bit pixels (`ff_ivi_recompose53` in FFmpeg's `ivi_dsp.c`).
fn recompose53(bands: &[Band], dst: usize, out: &mut [u8], width: usize, height: usize) {
    let pitch = bands[0].pitch;
    let b0 = &bands[0].planes[dst];
    let b1 = &bands[1].planes[dst];
    let b2 = &bands[2].planes[dst];
    let b3 = &bands[3].planes[dst];

    for y in (0..height).step_by(2) {
        let row_idx = y / 2;
        let y_cur = row_idx * pitch;
        let y_prev = if row_idx == 0 { y_cur } else { y_cur - pitch };
        let y_next = if y + 2 >= height {
            y_cur
        } else {
            y_cur + pitch
        };

        let mut b0_1 = i32::from(b0[y_cur]);
        let mut b0_2 = i32::from(b0[y_next]);

        let mut b1_1 = i32::from(b1[y_prev]);
        let mut b1_2 = i32::from(b1[y_cur]);
        let mut b1_3 = b1_1 - b1_2 * 6 + i32::from(b1[y_next]);

        let mut b2_2 = i32::from(b2[y_cur]);
        let mut b2_3 = b2_2;
        let mut b2_5 = i32::from(b2[y_next]);
        let mut b2_6 = b2_5;

        let mut b3_2 = i32::from(b3[y_prev]);
        let mut b3_3 = b3_2;
        let mut b3_5 = i32::from(b3[y_cur]);
        let mut b3_6 = b3_5;
        let mut b3_8 = b3_2 - b3_5 * 6 + i32::from(b3[y_next]);
        let mut b3_9 = b3_8;

        for x in (0..width).step_by(2) {
            let col = x / 2;
            let next_col = if x + 2 >= width { col } else { col + 1 };

            let b2_1 = b2_2;
            b2_2 = b2_3;
            let b2_4 = b2_5;
            b2_5 = b2_6;
            let b3_1 = b3_2;
            b3_2 = b3_3;
            let b3_4 = b3_5;
            b3_5 = b3_6;
            let b3_7 = b3_8;
            b3_8 = b3_9;

            // LL band: LPF vertically and horizontally.
            let tmp0 = b0_1;
            let tmp2 = b0_2;
            b0_1 = i32::from(b0[y_cur + next_col]);
            b0_2 = i32::from(b0[y_next + next_col]);
            let tmp1 = tmp0 + b0_1;

            let mut p0 = tmp0 * 16;
            let mut p1 = tmp1 * 8;
            let mut p2 = (tmp0 + tmp2) * 8;
            let mut p3 = (tmp1 + tmp2 + b0_2) * 4;

            // HL band: HPF vertically, LPF horizontally.
            let tmp0 = b1_2;
            let tmp1 = b1_1;
            b1_2 = i32::from(b1[y_cur + next_col]);
            b1_1 = i32::from(b1[y_prev + next_col]);
            let tmp2 = b1_3;
            b1_3 = b1_1 - b1_2 * 6 + i32::from(b1[y_next + next_col]);

            p0 += (tmp0 + tmp1) * 8;
            p1 += (tmp0 + tmp1 + b1_1 + b1_2) * 4;
            p2 += tmp2 * 4;
            p3 += (tmp2 + b1_3) * 2;

            // LH band: LPF vertically, HPF horizontally.
            b2_3 = i32::from(b2[y_cur + next_col]);
            b2_6 = i32::from(b2[y_next + next_col]);
            let tmp0 = b2_1 + b2_2;
            let tmp1 = b2_1 - b2_2 * 6 + b2_3;

            p0 += tmp0 * 8;
            p1 += tmp1 * 4;
            p2 += (tmp0 + b2_4 + b2_5) * 4;
            p3 += (tmp1 + b2_4 - b2_5 * 6 + b2_6) * 2;

            // HH band: HPF vertically and horizontally.
            b3_6 = i32::from(b3[y_cur + next_col]);
            b3_3 = i32::from(b3[y_prev + next_col]);
            let tmp0 = b3_1 + b3_4;
            let tmp1 = b3_2 + b3_5;
            let tmp2 = b3_3 + b3_6;
            b3_9 = b3_3 - b3_6 * 6 + i32::from(b3[y_next + next_col]);

            p0 += (tmp0 + tmp1) * 4;
            p1 += (tmp0 - tmp1 * 6 + tmp2) * 2;
            p2 += (b3_7 + b3_8) * 2;
            p3 += b3_7 - b3_8 * 6 + b3_9;

            let row0 = y * width + x;
            out[row0] = ((p0 >> 6) + 128).clamp(0, 255) as u8;
            if x + 1 < width {
                out[row0 + 1] = ((p1 >> 6) + 128).clamp(0, 255) as u8;
            }
            if y + 1 < height {
                let row1 = (y + 1) * width + x;
                out[row1] = ((p2 >> 6) + 128).clamp(0, 255) as u8;
                if x + 1 < width {
                    out[row1 + 1] = ((p3 >> 6) + 128).clamp(0, 255) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bt601, signed, Band, Bits, Codebook, Decoder, Picture};

    #[test]
    fn studio_black_and_white_are_the_ends_of_the_range_and_grey_has_no_colour() {
        assert_eq!(bt601(16, 128, 128), (0, 0, 0));
        assert_eq!(bt601(235, 128, 128), (255, 255, 255));
        let (r, g, b) = bt601(126, 128, 128);
        assert!(r == g && g == b, "grey came out as {r},{g},{b}");
    }

    #[test]
    fn full_red_and_blue_chroma_lean_the_way_bt601_says() {
        // Studio red is Y 81, U 90, V 240; studio blue is Y 41, U 240, V 110.
        let (r, g, b) = bt601(81, 90, 240);
        assert!(r > 250 && g < 5 && b < 5, "red came out as {r},{g},{b}");
        let (r, g, b) = bt601(41, 240, 110);
        assert!(r < 5 && g < 5 && b > 250, "blue came out as {r},{g},{b}");
    }

    /// A pixel's colour takes its chroma from the four by four block it is
    /// in: an 8x4 picture has two, side by side.
    #[test]
    fn a_pixel_takes_its_chroma_from_its_own_block() {
        let picture = Picture {
            width: 8,
            height: 4,
            y: vec![81; 32],
            u: vec![90, 128],
            v: vec![240, 128],
        };
        assert_eq!(picture.rgb(3, 3), bt601(81, 90, 240));
        assert_eq!(picture.rgb(4, 0), bt601(81, 128, 128));
    }

    #[test]
    fn fields_are_read_least_significant_bit_first() {
        let mut bits = Bits {
            data: &[0x1f, 0x00, 0x81],
            at: 0,
        };
        assert_eq!(bits.bits(5), Ok(0x1f));
        assert_eq!(bits.bits(3), Ok(0));
        assert_eq!(bits.bits(8), Ok(0));
        assert_eq!(bits.bits(8), Ok(0x81));
        assert!(bits.bits(1).is_err());
    }

    #[test]
    fn a_sign_lives_in_the_lowest_bit() {
        assert_eq!([0, 1, 2, 3, 4].map(signed), [0, 1, -1, 2, -2]);
    }

    #[test]
    fn a_code_is_its_row_in_ones_then_its_payload() {
        let mut xbits = [0u8; 16];
        xbits[0] = 1;
        xbits[1] = 2;
        let book = Codebook::new(2, xbits).expect("a valid codebook");
        // 0 then payload 1 is symbol 1; 1 then payload 10 is row 1's third.
        // Read in order 0,1,1,1,0, which packs as 0b01110.
        let mut bits = Bits {
            data: &[0x0e],
            at: 0,
        };
        assert_eq!(book.read(&mut bits), Ok(1));
        assert_eq!(book.read(&mut bits), Ok(4));
    }

    #[test]
    fn a_null_picture_before_any_picture_is_refused() {
        let mut decoder = Decoder::new();
        let mut out = Picture::default();
        assert!(decoder.decode(&[0x1f | (4 << 5), 0], &mut out).is_err());
    }

    #[test]
    fn scalable_recompose53_reconstructs_dc_and_detail_bands() {
        let mut bands: Vec<Band> = (0..4).map(|_| Band::new(2, 2, 16)).collect();
        for b in &mut bands {
            b.planes[0] = vec![0; b.pitch * b.aligned_height];
        }
        // Uniform LL = 16 with zero high-frequency bands: p0..p3 = 16*16 = 256 -> (256 >> 6) + 128 = 132.
        for row in 0..2 {
            let at = row * bands[0].pitch;
            bands[0].planes[0][at] = 16;
            bands[0].planes[0][at + 1] = 16;
        }
        let mut out = vec![0u8; 16];
        super::recompose53(&bands, 0, &mut out, 4, 4);
        assert_eq!(out, vec![132u8; 16]);

        // Constant HL = 16 with zero LL/LH/HH must be zero-DC vertically:
        // even rows +4 (132), odd rows -4 (124) across every column.
        for b in &mut bands {
            b.planes[0].fill(0);
        }
        for row in 0..2 {
            let at = row * bands[1].pitch;
            bands[1].planes[0][at] = 16;
            bands[1].planes[0][at + 1] = 16;
        }
        super::recompose53(&bands, 0, &mut out, 4, 4);
        assert_eq!(
            out,
            vec![132, 132, 132, 132, 124, 124, 124, 124, 132, 132, 132, 132, 124, 124, 124, 124]
        );
    }

    #[test]
    fn scalable_1d_transforms_and_dc_spread_match_band_orientation() {
        let mut coeffs = [0i32; 64];
        coeffs[0] = 4;
        let mut cols = [false; 8];
        cols[0] = true;

        let mut row_out = [0i16; 64];
        super::inverse(
            super::Transform::RowSlant8x8,
            &coeffs,
            &mut row_out,
            8,
            &cols,
        );
        assert!(row_out[..8].iter().all(|&v| v == 2));
        assert!(row_out[8..].iter().all(|&v| v == 0));

        let mut row_dc = [0i16; 64];
        super::dc_only(super::Transform::RowSlant8x8, 4, &mut row_dc, 8, 8);
        assert_eq!(row_out, row_dc);

        let mut col_out = [0i16; 64];
        super::inverse(
            super::Transform::ColSlant8x8,
            &coeffs,
            &mut col_out,
            8,
            &cols,
        );
        for r in 0..8 {
            assert_eq!(col_out[r * 8], 2);
            assert!(col_out[r * 8 + 1..r * 8 + 8].iter().all(|&v| v == 0));
        }
        let mut col_dc = [0i16; 64];
        super::dc_only(super::Transform::ColSlant8x8, 4, &mut col_dc, 8, 8);
        assert_eq!(col_out, col_dc);

        let mut pix_out = [0i16; 64];
        super::inverse(
            super::Transform::PutPixels8x8,
            &coeffs,
            &mut pix_out,
            8,
            &cols,
        );
        assert_eq!(pix_out[0], 4);
        assert!(pix_out[1..].iter().all(|&v| v == 0));
        let mut pix_dc = [0i16; 64];
        super::dc_only(super::Transform::PutPixels8x8, 4, &mut pix_dc, 8, 8);
        assert_eq!(pix_out, pix_dc);
    }

    #[test]
    fn scalable_inter_buffer_switching_uses_third_buffer_and_snaps_cleanly() {
        use crate::snap::{Reader, Snap, Writer};

        let mut dec = Decoder::new();
        // INTRA -> dst=0, ref=1
        dec.previous_type = super::INTRA;
        dec.frame_type = super::INTRA;
        dec.switch_buffers();
        assert_eq!((dec.dst, dec.reference, dec.inter_scal), (0, 1, false));

        // First INTER_SCALABLE -> dst=1, ref=0
        dec.previous_type = super::INTRA;
        dec.frame_type = super::INTER_SCALABLE;
        dec.switch_buffers();
        assert_eq!((dec.dst, dec.reference, dec.inter_scal), (1, 0, false));

        // Second INTER_SCALABLE -> swaps into buffer 2 (dst=2, ref=1, inter_scal=true)
        dec.previous_type = super::INTER_SCALABLE;
        dec.frame_type = super::INTER_SCALABLE;
        dec.switch_buffers();
        assert_eq!((dec.dst, dec.reference, dec.inter_scal), (2, 1, true));

        // Third INTER_SCALABLE -> swaps back (dst=1, ref=2)
        dec.previous_type = super::INTER_SCALABLE;
        dec.frame_type = super::INTER_SCALABLE;
        dec.switch_buffers();
        assert_eq!((dec.dst, dec.reference, dec.inter_scal), (1, 2, true));

        // Populate 4 scalable luma bands with all 4 transforms + snap round-trip.
        dec.is_scalable = true;
        dec.planes[0] = [
            super::Transform::Slant8x8,
            super::Transform::RowSlant8x8,
            super::Transform::ColSlant8x8,
            super::Transform::PutPixels8x8,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let mut b = Band::new(16, 16, 16);
            b.blk_size = 8;
            b.mb_size = 16;
            b.transform = t;
            b.quant_mat = (i + 1) as u8;
            assert!(b.apply_tables());
            b.planes[2] = vec![(i as i16) + 7; 4];
            b
        })
        .collect();

        let mut writer = Writer::new();
        dec.snap(&mut writer);

        let mut restored = Decoder::new();
        let mut reader = Reader::new(&writer.out);
        restored.snap(&mut reader);
        reader.finish("iv50").expect("clean scalable iv50 snapshot");

        assert!(restored.is_scalable);
        assert!(restored.inter_scal);
        assert_eq!((restored.dst, restored.reference, restored.ref2), (1, 2, 2));
        assert_eq!(restored.planes[0].len(), 4);
        assert_eq!(
            restored.planes[0][1].transform,
            super::Transform::RowSlant8x8
        );
        assert_eq!(
            restored.planes[0][2].transform,
            super::Transform::ColSlant8x8
        );
        assert_eq!(
            restored.planes[0][3].transform,
            super::Transform::PutPixels8x8
        );
        assert_eq!(restored.planes[0][3].quant_mat, 4);
        assert_eq!(restored.planes[0][3].planes[2], vec![10; 4]);
    }
}
