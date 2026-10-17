// SPDX-License-Identifier: LGPL-2.1-or-later
// Derived from FFmpeg's Indeo 5 decoder, Copyright (c) 2009-2011 Maxim
// Poliakovski and the FFmpeg developers. See NOTICE.

//! Indeo Video Interactive 5, the codec the roster's movies are written in.
//!
//! Rock Raiders' movies are all IV50 (#9). The format: a little-endian
//! bitstream of pictures, each three planes at 4:1:0 (a luma plane and two
//! chroma planes a quarter its size each way). A plane is one band, or, in the
//! scalable mode, luma is four half-size wavelet bands put back together at
//! output. A band is tiles of macroblocks of 8x8 (luma) or 4x4 (chroma)
//! blocks: run-value coded coefficients through per-band codebooks,
//! dequantised, inverse slant transformed, and motion compensated at half a
//! pixel against sixteen-bit reference planes kept between pictures.
//!
//! Written against FFmpeg's decoder read as the format's reference; the
//! constants are in [`tables`]. What a stream asks for that this does not
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Transform {
    Slant8x8,
    RowSlant8,
    ColumnSlant8,
    Pixels8x8,
    Slant4x4,
}

struct Band {
    width: usize,
    height: usize,
    pitch: usize,
    aligned_height: usize,
    /// The reference planes, kept between pictures: two that trade places,
    /// and a third for the scalable mode.
    planes: [Vec<i16>; 3],
    halfpel: bool,
    mb_size: usize,
    blk_size: usize,
    transform: Transform,
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
            scan: &tables::SCAN_ZIGZAG_8X8,
            intra_base: &tables::BASE_QUANT_8X8_INTRA[0],
            inter_base: &tables::BASE_QUANT_8X8_INTER[0],
            intra_scale: &tables::SCALE_QUANT_8X8_INTRA[0],
            inter_scale: &tables::SCALE_QUANT_8X8_INTER[0],
            tiles: Vec::new(),
        }
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
    chroma_bands: usize,
}

/// One stream's decoder. Pictures depend on the ones before them, so a stream
/// is decoded in order by one of these from its first intra picture.
pub struct Decoder {
    layout: Option<Layout>,
    gop_valid: bool,
    decoded_any: bool,
    gop_flags: u32,
    frame_flags: u32,
    frame_type: u8,
    previous_type: u8,
    switch: usize,
    dst: usize,
    reference: usize,
    second_reference: usize,
    inter_scalable: bool,
    mb_book: Option<Codebook>,
    planes: [Vec<Band>; 3],
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            layout: None,
            gop_valid: false,
            decoded_any: false,
            gop_flags: 0,
            frame_flags: 0,
            frame_type: INTRA,
            previous_type: INTRA,
            switch: 0,
            dst: 0,
            reference: 1,
            second_reference: 0,
            inter_scalable: false,
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
        let scalable = self.layout.is_some_and(|l| l.luma_bands > 1);
        if self.frame_type == INTER_SCALABLE && !scalable {
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
            if scalable || !self.decoded_any {
                return Err(Error("a null picture with nothing to repeat"));
            }
            return Ok(false);
        }

        let book = self.mb_book.ok_or(Error("no macroblock codebook"))?;
        for plane in 0..3 {
            for band in 0..self.planes[plane].len() {
                let reference: Option<Vec<Vec<Macroblock>>> =
                    (plane != 0 || band != 0).then(|| {
                        self.planes[0][0]
                            .tiles
                            .iter()
                            .map(|t| t.blocks.clone())
                            .collect()
                    });
                let context = Context {
                    plane,
                    band,
                    frame_flags: self.frame_flags,
                    frame_type: self.frame_type,
                    base_mb_size: self.planes[0][0].mb_size,
                    dst: self.dst,
                    reference: self.reference,
                    scalable,
                };
                decode_band(
                    &mut bits,
                    &mut self.planes[plane][band],
                    reference.as_deref(),
                    &book,
                    &context,
                )?;
            }
        }
        self.decoded_any = true;

        out.width = layout.width;
        out.height = layout.height;
        let (cw, ch) = out.chroma_size();
        out.y.resize(layout.width * layout.height, 0);
        out.u.resize(cw * ch, 0);
        out.v.resize(cw * ch, 0);
        if scalable {
            return Err(Error("the scalable mode's wavelet recomposition"));
        }
        copy_plane(
            &self.planes[0][0],
            self.dst,
            &mut out.y,
            layout.width,
            layout.height,
        );
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
                if !self.inter_scalable {
                    self.second_reference = 2;
                    self.inter_scalable = true;
                }
                std::mem::swap(&mut self.dst, &mut self.second_reference);
                self.reference = self.second_reference;
            }
            _ => {}
        }
        if matches!(self.frame_type, INTRA | INTER) {
            if self.frame_type == INTRA {
                self.switch = 0;
            }
            self.inter_scalable = false;
            self.dst = self.switch;
            self.reference = self.switch ^ 1;
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
        if (luma_bands, chroma_bands) != (1, 1) && (luma_bands, chroma_bands) != (4, 1) {
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
            chroma_bands,
        };
        let reconfigured = self.layout != Some(layout);
        if reconfigured {
            self.planes = [
                bands(width, height, luma_bands, 16),
                bands(width.div_ceil(4), height.div_ceil(4), chroma_bands, 8),
                bands(width.div_ceil(4), height.div_ceil(4), chroma_bands, 8),
            ];
            self.layout = Some(layout);
        }

        // As the reference does: whether the tiles are rebuilt follows the
        // last band read, and a new layout always rebuilds them.
        let mut rebuild = reconfigured;
        for plane in 0..2 {
            for index in 0..self.planes[plane].len() {
                let band = &mut self.planes[plane][index];
                band.halfpel = bits.bit()?;
                let mb_bit = bits.bits(1)?;
                let blk_size = 8usize >> bits.bits(1)?;
                let mb_size = blk_size << (mb_bit ^ 1);
                if plane == 0 && blk_size == 4 {
                    return Err(Error("4x4 luma blocks"));
                }
                rebuild = mb_size != band.mb_size || blk_size != band.blk_size;
                band.mb_size = mb_size;
                band.blk_size = blk_size;
                if bits.bit()? {
                    return Err(Error("extended transform information"));
                }
                let (transform, scan, size): (Transform, &'static [u8], usize) =
                    match (plane, index) {
                        (0, 0) => (Transform::Slant8x8, &tables::SCAN_ZIGZAG_8X8, 8),
                        (0, 1) => (Transform::RowSlant8, &tables::SCAN_VERTICAL_8X8, 8),
                        (0, 2) => (Transform::ColumnSlant8, &tables::SCAN_HORIZONTAL_8X8, 8),
                        (0, 3) => (Transform::Pixels8x8, &tables::SCAN_HORIZONTAL_8X8, 8),
                        _ => (Transform::Slant4x4, &tables::SCAN_DIRECT_4X4, 4),
                    };
                if size != blk_size {
                    return Err(Error("a transform the size of no block"));
                }
                band.transform = transform;
                band.scan = scan;
                let matrix = match plane {
                    0 if luma_bands > 1 => index + 1,
                    0 => 0,
                    _ => 5,
                };
                if blk_size == 8 {
                    let matrix = matrix.min(4);
                    band.intra_base = &tables::BASE_QUANT_8X8_INTRA[matrix];
                    band.inter_base = &tables::BASE_QUANT_8X8_INTER[matrix];
                    band.intra_scale = &tables::SCALE_QUANT_8X8_INTRA[matrix];
                    band.inter_scale = &tables::SCALE_QUANT_8X8_INTER[matrix];
                } else {
                    band.intra_base = &tables::BASE_QUANT_4X4_INTRA;
                    band.inter_base = &tables::BASE_QUANT_4X4_INTER;
                    band.intra_scale = &tables::SCALE_QUANT_4X4_INTRA;
                    band.inter_scale = &tables::SCALE_QUANT_4X4_INTER;
                }
                if bits.bits(2)? != 0 {
                    return Err(Error("a band description without its end marker"));
                }
            }
        }
        // The second chroma plane is described by the first.
        for index in 0..chroma_bands {
            let (first, second) = self.planes.split_at_mut(2);
            let from = &first[1][index];
            let to = &mut second[0][index];
            to.halfpel = from.halfpel;
            to.mb_size = from.mb_size;
            to.blk_size = from.blk_size;
            to.transform = from.transform;
            to.scan = from.scan;
            to.intra_base = from.intra_base;
            to.inter_base = from.inter_base;
            to.intra_scale = from.intra_scale;
            to.inter_scale = from.inter_scale;
        }
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
                if tile_width % 2 != 0 || tile_height % 2 != 0 {
                    return Err(Error("odd tiles in the scalable mode"));
                }
                tile_width >>= 1;
                tile_height >>= 1;
            }
            for index in 0..self.planes[plane].len() {
                let band = &self.planes[plane][index];
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
                if plane != 0 || index != 0 {
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
                self.planes[plane][index].tiles = tiles;
            }
        }
        Ok(())
    }
}

fn bands(width: usize, height: usize, count: usize, align: usize) -> Vec<Band> {
    let (w, h) = if count == 1 {
        (width, height)
    } else {
        (width.div_ceil(2), height.div_ceil(2))
    };
    (0..count).map(|_| Band::new(w, h, align)).collect()
}

/// What a band is decoded against.
struct Context {
    plane: usize,
    band: usize,
    frame_flags: u32,
    frame_type: u8,
    base_mb_size: usize,
    dst: usize,
    reference: usize,
    scalable: bool,
}

impl Context {
    fn is_base(&self) -> bool {
        self.plane == 0 && self.band == 0
    }
}

fn decode_band(
    bits: &mut Bits<'_>,
    band: &mut Band,
    reference: Option<&[Vec<Macroblock>]>,
    mb_book: &Codebook,
    cx: &Context,
) -> Result<()> {
    for i in [cx.dst, cx.reference] {
        if i == 2 && !cx.scalable {
            return Err(Error("a third reference plane outside the scalable mode"));
        }
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
            Some(r) if r[t].len() != band.tiles[t].blocks.len() => {
                return Err(Error("a tile unlike its reference"));
            }
            Some(r) => Some(&r[t][..]),
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
    let two_d = matches!(band.transform, Transform::Slant8x8 | Transform::Slant4x4);
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
                if intra && two_d {
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
        Transform::RowSlant8 => {
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
        Transform::ColumnSlant8 => {
            for i in 0..8 {
                let column = if columns[i] {
                    slant8(std::array::from_fn(|k| c[i + 8 * k]), true)
                } else {
                    [0; 8]
                };
                for (k, v) in column.into_iter().enumerate() {
                    out[i + k * pitch] = v as i16;
                }
            }
        }
        Transform::Pixels8x8 => {
            for row in 0..8 {
                for k in 0..8 {
                    out[row * pitch + k] = c[8 * row + k] as i16;
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

/// An intra block with no coefficients: the running DC, spread the way the
/// band's transform would have spread it.
fn dc_only(transform: Transform, dc: i32, out: &mut [i16], pitch: usize, size: usize) {
    let half = ((dc + 1) >> 1) as i16;
    for row in 0..size {
        let line = &mut out[row * pitch..row * pitch + size];
        match transform {
            Transform::Slant8x8 | Transform::Slant4x4 => line.fill(half),
            Transform::RowSlant8 => line.fill(if row == 0 { half } else { 0 }),
            Transform::ColumnSlant8 => {
                line.fill(0);
                line[0] = half;
            }
            Transform::Pixels8x8 => {
                line.fill(0);
                if row == 0 {
                    line[0] = dc as i16;
                }
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

#[cfg(test)]
mod tests {
    use super::{signed, Bits, Codebook, Decoder, Picture};

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
}
