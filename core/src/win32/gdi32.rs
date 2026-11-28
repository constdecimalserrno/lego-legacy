//! GDI32: fonts, pens, brushes, device contexts, and 2D vector/text rendering
//! onto DirectDraw surface device contexts (#394).
//!
//! All state (registered font file paths, created objects, and per-DC drawing
//! attributes) lives inside [`GdiState`] and is exhaustively snapshotted in
//! `LLSNAP v2`. Parsed TrueType font bytes are rehydrated lazily from `Tree`
//! on cold snapshot restore (matching `movie_bytes`).

use super::{stdcall, ComState, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::raster::PixelFormat;
use crate::snap::{snap_btree_map, snap_option, snap_vec, Snap, Visitor};
use crate::ttf::{self, Font};
use std::collections::BTreeMap;
use std::rc::Rc;

pub const SHIMS: &[Shim] = &[
    stdcall("GDI32", "CreateRectRgn", 4, region),
    stdcall("GDI32", "CreateRectRgnIndirect", 1, region_indirect),
    stdcall("GDI32", "CreateSolidBrush", 1, create_solid_brush),
    stdcall("GDI32", "CreatePen", 3, create_pen),
    stdcall("GDI32", "CreateFontIndirectA", 1, create_font_indirect),
    stdcall("GDI32", "AddFontResourceA", 1, add_font_resource),
    stdcall("GDI32", "RemoveFontResourceA", 1, remove_font_resource),
    stdcall("GDI32", "CreateCompatibleDC", 1, create_compatible_dc),
    stdcall("GDI32", "DeleteDC", 1, delete_dc),
    stdcall("GDI32", "SelectObject", 2, select_object),
    stdcall("GDI32", "DeleteObject", 1, delete_object),
    stdcall("GDI32", "GetStockObject", 1, stock_object),
    stdcall("GDI32", "SetBkMode", 2, set_bk_mode),
    stdcall("GDI32", "SetTextColor", 2, set_text_color),
    stdcall("GDI32", "SetBkColor", 2, set_bk_color),
    stdcall("GDI32", "GetNearestColor", 2, get_nearest_color),
    stdcall("GDI32", "SetTextAlign", 2, set_text_align),
    stdcall("GDI32", "MoveToEx", 4, move_to_ex),
    stdcall("GDI32", "LineTo", 3, line_to),
    stdcall("GDI32", "TextOutA", 5, text_out),
    stdcall(
        "GDI32",
        "GetSystemPaletteEntries",
        4,
        system_palette_entries,
    ),
    stdcall("GDI32", "GetDeviceCaps", 2, device_caps),
    stdcall("GDI32", "GetRegionData", 3, region_data),
];

/// Legacy region pseudo-handle returned by `CreateRectRgn`.
const REGION: u32 = 0x0002_0010;
/// Base handle for stock objects (`GetStockObject`).
const STOCK_BASE: u32 = 0x0002_0100;
/// Base handle for dynamically allocated GDI objects and memory DCs, kept
/// disjoint from `ddraw::SURFACE_DC` (`0x0004_0000`) and solid-brush handles
/// (`0x0100_0000..=0x01ff_ffff`).
const FIRST_GDI_HANDLE: u32 = 0x0200_0000;

/// GDI background modes (`wingdi.h`).
const TRANSPARENT: u32 = 1;
const OPAQUE: u32 = 2;

/// GDI pen styles (`wingdi.h`).
const PS_NULL: u32 = 5;

/// GDI text alignment flags (`wingdi.h`).
const TA_RIGHT: u32 = 2;
const TA_CENTER: u32 = 6;
const TA_BOTTOM: u32 = 8;
const TA_BASELINE: u32 = 24;

/// A GDI object stored in [`GdiState::objects`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GdiObject {
    Font {
        height: i32,
        weight: i32,
        face: String,
    },
    Pen {
        style: u32,
        width: i32,
        color: u32,
    },
    Region {
        rect: [i32; 4],
    },
}

impl Default for GdiObject {
    fn default() -> Self {
        Self::Region { rect: [0; 4] }
    }
}

impl Snap for GdiObject {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let mut tag: u8 = match self {
            Self::Font { .. } => 0,
            Self::Pen { .. } => 1,
            Self::Region { .. } => 2,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            *self = match tag {
                0 => Self::Font {
                    height: 0,
                    weight: 400,
                    face: String::new(),
                },
                1 => Self::Pen {
                    style: 0,
                    width: 1,
                    color: 0,
                },
                2 => Self::Region { rect: [0; 4] },
                _ => {
                    v.mark_corrupt("GdiObject tag");
                    return;
                }
            };
        }
        match self {
            Self::Font {
                height,
                weight,
                face,
            } => {
                v.i32(height);
                v.i32(weight);
                v.string(face);
            }
            Self::Pen {
                style,
                width,
                color,
            } => {
                v.u32(style);
                v.i32(width);
                v.u32(color);
            }
            Self::Region { rect } => {
                for r in rect {
                    v.i32(r);
                }
            }
        }
    }
}

/// Per-device-context GDI attributes, tracked for both DirectDraw surface DCs
/// and `CreateCompatibleDC` memory DCs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DcState {
    pub font: u32,
    pub pen: u32,
    pub brush: u32,
    pub text_color: u32,
    pub bk_color: u32,
    pub bk_mode: u32,
    pub text_align: u32,
    pub pos: (i32, i32),
    pub clip_rect: Option<[i32; 4]>,
}

impl Default for DcState {
    fn default() -> Self {
        Self {
            font: STOCK_BASE + 13, // SYSTEM_FONT
            pen: STOCK_BASE + 7,   // BLACK_PEN
            brush: STOCK_BASE,     // WHITE_BRUSH
            text_color: 0x0000_0000,
            bk_color: 0x00ff_ffff,
            bk_mode: OPAQUE,
            text_align: 0,
            pos: (0, 0),
            clip_rect: None,
        }
    }
}

impl Snap for DcState {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            font,
            pen,
            brush,
            text_color,
            bk_color,
            bk_mode,
            text_align,
            pos,
            clip_rect,
        } = self;
        v.u32(font);
        v.u32(pen);
        v.u32(brush);
        v.u32(text_color);
        v.u32(bk_color);
        v.u32(bk_mode);
        v.u32(text_align);
        v.i32(&mut pos.0);
        v.i32(&mut pos.1);
        snap_option(v, clip_rect, |rect, v| {
            for r in rect {
                v.i32(r);
            }
        });
    }
}

/// GDI subsystem state owned by `win32::State`.
#[derive(Clone, Debug)]
pub struct GdiState {
    /// Resolved guest tree paths registered via `AddFontResourceA` (in order).
    pub font_files: Vec<String>,
    /// Next dynamically allocated handle (incremented by 4).
    pub next_handle: u32,
    /// Created GDI objects keyed by handle.
    pub objects: BTreeMap<u32, GdiObject>,
    /// Per-DC drawing attributes keyed by DC handle.
    pub dcs: BTreeMap<u32, DcState>,
    /// Runtime-only cache of parsed TrueType fonts in `font_files` order;
    /// rehydrated deterministically from `Tree` on cold snapshot restore.
    pub loaded_fonts: Vec<(String, Rc<Font>)>,
}

impl Default for GdiState {
    fn default() -> Self {
        Self {
            font_files: Vec::new(),
            next_handle: FIRST_GDI_HANDLE,
            objects: BTreeMap::new(),
            dcs: BTreeMap::new(),
            loaded_fonts: Vec::new(),
        }
    }
}

impl GdiState {
    fn alloc_handle(&mut self) -> u32 {
        let h = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(4);
        h
    }

    fn dc_mut(&mut self, dc: u32) -> &mut DcState {
        self.dcs.entry(dc).or_default()
    }

    fn find_font(&self, face: &str) -> Option<Rc<Font>> {
        if face.is_empty() {
            return self.loaded_fonts.first().map(|(_, f)| f.clone());
        }
        self.loaded_fonts
            .iter()
            .find(|(_, f)| f.family.eq_ignore_ascii_case(face))
            .map(|(_, f)| f.clone())
    }

    /// Rehydrate `loaded_fonts` in exact `font_files` order from `read` (called
    /// on `LLSNAP v2` snapshot restore).
    ///
    /// # Errors
    /// Returns a static error tag when a registered font is missing or unparseable.
    pub fn rehydrate_fonts(
        &mut self,
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Result<(), &'static str> {
        let existing = std::mem::take(&mut self.loaded_fonts);
        let mut rebuilt = Vec::with_capacity(self.font_files.len());
        for path in &self.font_files {
            if let Some((_, font)) = existing.iter().find(|(p, _)| p.eq_ignore_ascii_case(path)) {
                rebuilt.push((path.clone(), font.clone()));
                continue;
            }
            let bytes = read(path).ok_or("gdi font missing from tree")?;
            let font = Font::parse(bytes.into()).ok_or("gdi font corrupt")?;
            rebuilt.push((path.clone(), Rc::new(font)));
        }
        self.loaded_fonts = rebuilt;
        Ok(())
    }
}

impl Snap for GdiState {
    fn snap(&mut self, v: &mut dyn Visitor) {
        let Self {
            font_files,
            next_handle,
            objects,
            dcs,
            loaded_fonts: _,
        } = self;
        snap_vec(v, font_files, |p, v| v.string(p));
        v.u32(next_handle);
        snap_btree_map(v, objects, |k, v| v.u32(k), |o, v| o.snap(v));
        snap_btree_map(v, dcs, |k, v| v.u32(k), |d, v| d.snap(v));
    }
}

fn region(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(REGION)
}

fn region_indirect(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(0);
    }
    let rect = super::user32::read_rect(ctx, args[0])?;
    let h = ctx.state.gdi.alloc_handle();
    ctx.state.gdi.objects.insert(h, GdiObject::Region { rect });
    Ok(h)
}

fn create_solid_brush(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(0x0100_0000 | (args[0] & 0x00ff_ffff))
}

fn create_pen(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreatePen(fnPenStyle, nWidth, crColor).
    let h = ctx.state.gdi.alloc_handle();
    ctx.state.gdi.objects.insert(
        h,
        GdiObject::Pen {
            style: args[0],
            width: args[1] as i32,
            color: args[2] & 0x00ff_ffff,
        },
    );
    Ok(h)
}

fn create_font_indirect(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateFontIndirectA(const LOGFONTA *lplf):
    // +0 lfHeight (i32), +16 lfWeight (i32), +28 lfFaceName (32 bytes).
    let at = args[0];
    if at == 0 {
        return Ok(0);
    }
    let height = ctx.mem.read_u32(at).map_err(Stop::Fault)? as i32;
    let weight = ctx.mem.read_u32(at + 16).map_err(Stop::Fault)? as i32;
    let face_raw = ctx.mem.slice(at + 28, 32).map_err(Stop::Fault)?;
    let nul = face_raw.iter().position(|&b| b == 0).unwrap_or(32);
    let face = String::from_utf8_lossy(&face_raw[..nul]).into_owned();
    let h = ctx.state.gdi.alloc_handle();
    ctx.log
        .line(&format!("gdi font {h:#x}: {face:?} h={height} w={weight}"));
    ctx.state.gdi.objects.insert(
        h,
        GdiObject::Font {
            height,
            weight,
            face,
        },
    );
    Ok(h)
}

fn add_font_resource(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // AddFontResourceA(lpszFilename): returns number of fonts added (0 on failure).
    if args[0] == 0 {
        return Ok(0);
    }
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(0);
    }
    let Some((resolved, bytes)) = ctx.open_guest_file_or_install(&name) else {
        ctx.log
            .line(&format!("gdi AddFontResourceA: {name} missing"));
        return Ok(0);
    };
    let Some(font) = Font::parse(bytes.into()) else {
        ctx.log
            .line(&format!("gdi AddFontResourceA: {name} is not a valid TTF"));
        return Ok(0);
    };
    ctx.log.line(&format!(
        "gdi AddFontResourceA: {name} -> family {:?}",
        font.family
    ));
    if !ctx
        .state
        .gdi
        .font_files
        .iter()
        .any(|p| p.eq_ignore_ascii_case(&resolved))
    {
        ctx.state.gdi.font_files.push(resolved.clone());
        ctx.state.gdi.loaded_fonts.push((resolved, Rc::new(font)));
    }
    Ok(1)
}

fn remove_font_resource(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(0);
    }
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    let abs = ctx.absolute(&name);
    let fallback = (!name.contains(['\\', '/', ':']))
        .then(|| format!("{}\\{name}", ctx.title.install_dir.trim_end_matches('\\')));
    let matches_path = |p: &String| {
        p.eq_ignore_ascii_case(&abs)
            || fallback
                .as_ref()
                .is_some_and(|fb| p.eq_ignore_ascii_case(fb))
    };
    let before = ctx.state.gdi.font_files.len();
    ctx.state.gdi.font_files.retain(|p| !matches_path(p));
    ctx.state.gdi.loaded_fonts.retain(|(p, _)| !matches_path(p));
    Ok(u32::from(ctx.state.gdi.font_files.len() < before))
}

fn create_compatible_dc(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    let dc = ctx.state.gdi.alloc_handle();
    ctx.state.gdi.dcs.insert(dc, DcState::default());
    Ok(dc)
}

fn delete_dc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let dc = args[0];
    if dc == 0 {
        return Ok(0);
    }
    Ok(u32::from(ctx.state.gdi.dcs.remove(&dc).is_some()))
}

fn select_object(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // SelectObject(hdc, hgdiobj): replaces the DC's selected object of matching
    // kind and returns the previous handle.
    let (dc, obj) = (args[0], args[1]);
    if dc == 0 || obj == 0 {
        return Ok(0);
    }
    if let Some(kind) = ctx.state.gdi.objects.get(&obj).cloned() {
        let state = ctx.state.gdi.dc_mut(dc);
        return Ok(match kind {
            GdiObject::Font { .. } => std::mem::replace(&mut state.font, obj),
            GdiObject::Pen { .. } => std::mem::replace(&mut state.pen, obj),
            GdiObject::Region { rect } => {
                state.clip_rect = Some(rect);
                2 // SIMPLEREGION
            }
        });
    }
    // Stock objects: brushes (0..=5), pens (6..=8), fonts (10..=14, 16..=17).
    if let Some(idx) = obj.checked_sub(STOCK_BASE) {
        let state = ctx.state.gdi.dc_mut(dc);
        return Ok(match idx {
            0..=5 => std::mem::replace(&mut state.brush, obj),
            6..=8 => std::mem::replace(&mut state.pen, obj),
            10..=14 | 16..=17 => std::mem::replace(&mut state.font, obj),
            _ => 0,
        });
    }
    if obj & 0xff00_0000 == 0x0100_0000 {
        let state = ctx.state.gdi.dc_mut(dc);
        return Ok(std::mem::replace(&mut state.brush, obj));
    }
    Ok(0)
}

fn delete_object(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let obj = args[0];
    if obj == 0 {
        return Ok(0);
    }
    ctx.state.gdi.objects.remove(&obj);
    Ok(1)
}

fn stock_object(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(STOCK_BASE + args[0])
}

fn set_bk_mode(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (dc, mode) = (args[0], args[1]);
    if dc == 0 || (mode != TRANSPARENT && mode != OPAQUE) {
        return Ok(0);
    }
    Ok(std::mem::replace(
        &mut ctx.state.gdi.dc_mut(dc).bk_mode,
        mode,
    ))
}

fn set_text_color(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (dc, color) = (args[0], args[1] & 0x00ff_ffff);
    if dc == 0 {
        return Ok(0xffff_ffff); // CLR_INVALID
    }
    Ok(std::mem::replace(
        &mut ctx.state.gdi.dc_mut(dc).text_color,
        color,
    ))
}

fn set_bk_color(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (dc, color) = (args[0], args[1] & 0x00ff_ffff);
    if dc == 0 {
        return Ok(0xffff_ffff); // CLR_INVALID
    }
    Ok(std::mem::replace(
        &mut ctx.state.gdi.dc_mut(dc).bk_color,
        color,
    ))
}

fn get_nearest_color(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(0xffff_ffff); // CLR_INVALID
    }
    Ok(args[1] & 0x00ff_ffff)
}

fn set_text_align(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (dc, align) = (args[0], args[1]);
    if dc == 0 {
        return Ok(0xffff_ffff); // GDI_ERROR
    }
    Ok(std::mem::replace(
        &mut ctx.state.gdi.dc_mut(dc).text_align,
        align,
    ))
}

fn move_to_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // MoveToEx(hdc, x, y, lppt).
    let (dc, x, y, prev_out) = (args[0], args[1] as i32, args[2] as i32, args[3]);
    if dc == 0 {
        return Ok(0);
    }
    let old = std::mem::replace(&mut ctx.state.gdi.dc_mut(dc).pos, (x, y));
    if prev_out != 0 {
        ctx.mem
            .write_u32(prev_out, old.0 as u32)
            .map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(prev_out + 4, old.1 as u32)
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

struct SurfaceTarget {
    pixels: u32,
    width: i32,
    height: i32,
    pitch: u32,
    format: PixelFormat,
}

fn surface_target(ctx: &Ctx<'_>, dc: u32, call: &str) -> Result<Option<SurfaceTarget>, Stop> {
    let Some(&surface) = ctx.state.surface_dcs.get(&dc) else {
        return Ok(None);
    };
    let Some(ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format,
        ..
    }) = ctx.state.com.get(surface).map(|o| o.state.clone())
    else {
        return Ok(None);
    };
    if format.bits < 16 {
        return Err(Stop::NotWritten {
            what: format!(
                "{call}: drawing onto a {}-bit DirectDraw surface DC",
                format.bits
            ),
            caller: 0,
        });
    }
    Ok(Some(SurfaceTarget {
        pixels,
        width: width as i32,
        height: height as i32,
        pitch,
        format,
    }))
}

fn encode_colorref(format: PixelFormat, colorref: u32) -> u32 {
    let r = (colorref & 0xff) as u8;
    let g = ((colorref >> 8) & 0xff) as u8;
    let b = ((colorref >> 16) & 0xff) as u8;
    format.encode([r, g, b, 255])
}

fn put_pixel(
    mem: &mut crate::guest::Arena,
    target: &SurfaceTarget,
    clip: Option<[i32; 4]>,
    x: i32,
    y: i32,
    encoded: u32,
) -> Result<(), Stop> {
    if x < 0 || y < 0 || x >= target.width || y >= target.height {
        return Ok(());
    }
    if let Some([cl, ct, cr, cb]) = clip {
        if x < cl || x >= cr || y < ct || y >= cb {
            return Ok(());
        }
    }
    let bpp = target.format.bits / 8;
    if bpp == 0 {
        return Ok(());
    }
    let addr = target
        .pixels
        .wrapping_add((y as u32).wrapping_mul(target.pitch))
        .wrapping_add((x as u32).wrapping_mul(bpp));
    match bpp {
        2 => mem
            .write(addr, &(encoded as u16).to_le_bytes())
            .map_err(Stop::Fault)?,
        3 => {
            let b = encoded.to_le_bytes();
            mem.write(addr, &b[..3]).map_err(Stop::Fault)?;
        }
        4 => mem.write_u32(addr, encoded).map_err(Stop::Fault)?,
        _ => {}
    }
    Ok(())
}

fn pen_info(gdi: &GdiState, pen_handle: u32) -> (u32, i32, u32) {
    if let Some(GdiObject::Pen {
        style,
        width,
        color,
    }) = gdi.objects.get(&pen_handle)
    {
        return (*style, (*width).clamp(1, 32), *color);
    }
    match pen_handle.checked_sub(STOCK_BASE) {
        Some(6) => (0, 1, 0x00ff_ffff), // WHITE_PEN
        Some(8) => (PS_NULL, 0, 0),     // NULL_PEN
        _ => (0, 1, 0x0000_0000),       // BLACK_PEN
    }
}

fn line_to(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // LineTo(hdc, xEnd, yEnd): draws from current position up to (but not
    // including) the end point using the DC's selected pen, then updates pos.
    let (dc, x1, y1) = (args[0], args[1] as i32, args[2] as i32);
    if dc == 0 {
        return Ok(0);
    }
    let dc_state = ctx.state.gdi.dcs.get(&dc).cloned().unwrap_or_default();
    let (x0, y0) = dc_state.pos;
    ctx.state.gdi.dc_mut(dc).pos = (x1, y1);

    let Some(target) = surface_target(ctx, dc, "GDI32.dll!LineTo")? else {
        return Ok(1);
    };
    let (style, width, color) = pen_info(&ctx.state.gdi, dc_state.pen);
    if style == PS_NULL {
        return Ok(1);
    }
    let encoded = encode_colorref(target.format, color);
    let radius = (width - 1) / 2;

    // 64-bit integer Bresenham excluding endpoint (x1, y1) per Win32 LineTo.
    let (mut cx, mut cy) = (i64::from(x0), i64::from(y0));
    let (tx, ty) = (i64::from(x1), i64::from(y1));
    let dx = (tx - cx).abs();
    let dy = -(ty - cy).abs();
    let sx: i64 = if cx < tx { 1 } else { -1 };
    let sy: i64 = if cy < ty { 1 } else { -1 };
    let mut err = dx + dy;
    let max_steps = (dx.max(-dy) + 1).min(4096);
    for _ in 0..max_steps {
        if cx == tx && cy == ty {
            break;
        }
        if let (Ok(px), Ok(py)) = (i32::try_from(cx), i32::try_from(cy)) {
            for oy in -radius..=radius {
                for ox in -radius..=radius {
                    put_pixel(
                        ctx.mem,
                        &target,
                        dc_state.clip_rect,
                        px.saturating_add(ox),
                        py.saturating_add(oy),
                        encoded,
                    )?;
                }
            }
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
    Ok(1)
}

/// Resolved typeface and line metrics for a GDI text draw call.
struct TextFace {
    font: Option<Rc<Font>>,
    lf_height: i32,
    bold: bool,
    ascent: i32,
    line_height: i32,
}

impl TextFace {
    fn resolve(gdi: &GdiState, font_handle: u32) -> Self {
        let (font, lf_height, bold) = match gdi.objects.get(&font_handle) {
            Some(GdiObject::Font {
                height,
                weight,
                face,
            }) => (gdi.find_font(face), *height, *weight >= 700),
            _ => (gdi.find_font(""), -16, false),
        };
        let (ascent, line_height) = if let Some(ref f) = font {
            f.line_metrics(lf_height)
        } else {
            let scale = ttf::fallback_scale(lf_height);
            (7 * scale, 8 * scale)
        };
        Self {
            font,
            lf_height,
            bold,
            ascent,
            line_height,
        }
    }

    fn measure(&self, text: &str) -> i32 {
        if let Some(ref f) = self.font {
            text.chars()
                .map(|ch| f.advance_px(ch, self.lf_height) + i32::from(self.bold))
                .sum()
        } else {
            let step = 6 * ttf::fallback_scale(self.lf_height);
            (text.chars().count() as i32).saturating_mul(step)
        }
    }

    fn render_line(
        &self,
        mem: &mut crate::guest::Arena,
        target: &SurfaceTarget,
        clip: Option<[i32; 4]>,
        origin: (i32, i32),
        text: &str,
        colors: (u32, Option<u32>),
    ) -> Result<(), Stop> {
        let (start_x, top_y) = origin;
        let (fg, bg) = colors;
        if let Some(bg_col) = bg {
            let w = self.measure(text);
            let mut x0 = start_x.max(0);
            let mut y0 = top_y.max(0);
            let mut x1 = start_x.saturating_add(w).min(target.width);
            let mut y1 = top_y.saturating_add(self.line_height).min(target.height);
            if let Some([cl, ct, cr, cb]) = clip {
                x0 = x0.max(cl);
                y0 = y0.max(ct);
                x1 = x1.min(cr);
                y1 = y1.min(cb);
            }
            for py in y0..y1 {
                for px in x0..x1 {
                    put_pixel(mem, target, None, px, py, bg_col)?;
                }
            }
        }
        let mut pen_x = start_x;
        let baseline_y = top_y.saturating_add(self.ascent);
        for ch in text.chars() {
            let glyph = if let Some(ref f) = self.font {
                f.rasterize(ch, self.lf_height, self.bold)
            } else {
                ttf::fallback_glyph(ch, self.lf_height)
            };
            for gy in 0..glyph.height {
                let py = baseline_y.saturating_add(glyph.y_off).saturating_add(gy);
                let row = (gy * glyph.width) as usize;
                for gx in 0..glyph.width {
                    if glyph.mask[row + gx as usize] != 0 {
                        let px = pen_x.saturating_add(glyph.x_off).saturating_add(gx);
                        put_pixel(mem, target, clip, px, py, fg)?;
                    }
                }
            }
            pen_x = pen_x.saturating_add(glyph.advance);
        }
        Ok(())
    }
}

/// Read an ANSI string (`u8 -> char` Latin-1). When `stop_at_nul` is true (or
/// `count < 0`), stops at the first NUL byte.
fn read_ansi_text(ctx: &Ctx<'_>, ptr: u32, count: i32, stop_at_nul: bool) -> Result<String, Stop> {
    if ptr == 0 || count == 0 {
        return Ok(String::new());
    }
    if count < 0 {
        let mut out = String::new();
        let mut at = ptr;
        for _ in 0..4096 {
            let b = ctx.mem.slice(at, 1).map_err(Stop::Fault)?[0];
            if b == 0 {
                break;
            }
            out.push(char::from(b));
            at = at.wrapping_add(1);
        }
        return Ok(out);
    }
    let slice = ctx.mem.slice(ptr, count as usize).map_err(Stop::Fault)?;
    let end = if stop_at_nul {
        slice.iter().position(|&b| b == 0).unwrap_or(slice.len())
    } else {
        slice.len()
    };
    Ok(slice[..end].iter().map(|&b| char::from(b)).collect())
}

fn text_out(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // TextOutA(hdc, nXStart, nYStart, lpString, cchString).
    let (dc, x, y, ptr, count) = (
        args[0],
        args[1] as i32,
        args[2] as i32,
        args[3],
        args[4] as i32,
    );
    if dc == 0 || count < 0 {
        return Ok(0);
    }
    let text = read_ansi_text(ctx, ptr, count, false)?;
    let Some(target) = surface_target(ctx, dc, "GDI32.dll!TextOutA")? else {
        return Ok(1);
    };
    let dc_state = ctx.state.gdi.dcs.get(&dc).cloned().unwrap_or_default();
    let face = TextFace::resolve(&ctx.state.gdi, dc_state.font);
    let text_w = face.measure(&text);

    let start_x = match dc_state.text_align & 0x6 {
        TA_CENTER => x.saturating_sub(text_w / 2),
        TA_RIGHT => x.saturating_sub(text_w),
        _ => x,
    };
    let top_y = match dc_state.text_align & 0x18 {
        TA_BOTTOM => y.saturating_sub(face.line_height),
        TA_BASELINE => y.saturating_sub(face.ascent),
        _ => y,
    };
    let fg = encode_colorref(target.format, dc_state.text_color);
    let bg =
        (dc_state.bk_mode == OPAQUE).then(|| encode_colorref(target.format, dc_state.bk_color));
    face.render_line(
        ctx.mem,
        &target,
        dc_state.clip_rect,
        (start_x, top_y),
        &text,
        (fg, bg),
    )?;
    Ok(1)
}

fn strip_mnemonic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn wrap_lines(
    face: &TextFace,
    text: &str,
    max_width: i32,
    word_break: bool,
    single_line: bool,
) -> Vec<String> {
    if single_line {
        return vec![text.replace(['\r', '\n'], " ")];
    }
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if !word_break || max_width <= 0 || line.is_empty() {
            lines.push(line.to_owned());
            continue;
        }
        let mut current = String::new();
        for word in line.split(' ') {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && face.measure(&candidate) > max_width {
                lines.push(current);
                current = word.to_owned();
            } else {
                current = candidate;
            }
        }
        lines.push(current);
    }
    lines
}

/// Implementation of `USER32.dll!DrawTextA(hdc, lpchText, cchText, lprc, format)`.
pub(super) fn draw_text(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    const DT_CENTER: u32 = 0x0001;
    const DT_RIGHT: u32 = 0x0002;
    const DT_VCENTER: u32 = 0x0004;
    const DT_BOTTOM: u32 = 0x0008;
    const DT_WORDBREAK: u32 = 0x0010;
    const DT_SINGLELINE: u32 = 0x0020;
    const DT_NOCLIP: u32 = 0x0100;
    const DT_CALCRECT: u32 = 0x0400;
    const DT_NOPREFIX: u32 = 0x0800;

    let (dc, text_ptr, count, rect_ptr, format) =
        (args[0], args[1], args[2] as i32, args[3], args[4]);
    if dc == 0 || rect_ptr == 0 {
        return Ok(0);
    }
    let raw_text = read_ansi_text(ctx, text_ptr, count, true)?;
    let text = if format & DT_NOPREFIX != 0 {
        raw_text
    } else {
        strip_mnemonic(&raw_text)
    };
    let [left, top, right, bottom] = super::user32::read_rect(ctx, rect_ptr)?;

    let dc_state = ctx.state.gdi.dcs.get(&dc).cloned().unwrap_or_default();
    let face = TextFace::resolve(&ctx.state.gdi, dc_state.font);

    let single_line = format & DT_SINGLELINE != 0;
    let word_break = format & DT_WORDBREAK != 0;
    let max_width = right.saturating_sub(left).max(0);
    let lines = wrap_lines(&face, &text, max_width, word_break, single_line);
    let total_height = (lines.len() as i32).max(1).saturating_mul(face.line_height);
    let max_line_w = lines.iter().map(|l| face.measure(l)).max().unwrap_or(0);

    if format & DT_CALCRECT != 0 {
        if single_line || !word_break || lines.len() == 1 {
            ctx.mem
                .write_u32(rect_ptr + 8, left.saturating_add(max_line_w) as u32)
                .map_err(Stop::Fault)?;
        }
        ctx.mem
            .write_u32(rect_ptr + 12, top.saturating_add(total_height) as u32)
            .map_err(Stop::Fault)?;
        return Ok(total_height as u32);
    }

    let Some(target) = surface_target(ctx, dc, "USER32.dll!DrawTextA")? else {
        return Ok(total_height as u32);
    };

    let clip = if format & DT_NOCLIP != 0 {
        dc_state.clip_rect
    } else {
        let r = [left, top, right, bottom];
        Some(match dc_state.clip_rect {
            Some([cl, ct, cr, cb]) => [cl.max(r[0]), ct.max(r[1]), cr.min(r[2]), cb.min(r[3])],
            None => r,
        })
    };

    let start_y = if single_line && format & DT_VCENTER != 0 {
        top.saturating_add(bottom.saturating_sub(top).saturating_sub(total_height) / 2)
    } else if single_line && format & DT_BOTTOM != 0 {
        bottom.saturating_sub(total_height)
    } else {
        top
    };

    let fg = encode_colorref(target.format, dc_state.text_color);
    let bg =
        (dc_state.bk_mode == OPAQUE).then(|| encode_colorref(target.format, dc_state.bk_color));

    for (idx, line) in lines.iter().enumerate() {
        let lw = face.measure(line);
        let start_x = if format & DT_CENTER != 0 {
            left.saturating_add(right.saturating_sub(left).saturating_sub(lw) / 2)
        } else if format & DT_RIGHT != 0 {
            right.saturating_sub(lw)
        } else {
            left
        };
        let y = start_y.saturating_add((idx as i32).saturating_mul(face.line_height));
        face.render_line(ctx.mem, &target, clip, (start_x, y), line, (fg, bg))?;
    }

    Ok(total_height as u32)
}

fn brush_color(brush: u32) -> Option<u32> {
    if brush & 0xff00_0000 == 0x0100_0000 {
        return Some(brush & 0x00ff_ffff);
    }
    match brush.checked_sub(STOCK_BASE) {
        Some(0) => Some(0x00ff_ffff), // WHITE_BRUSH
        Some(1) => Some(0x00c0_c0c0), // LTGRAY_BRUSH
        Some(2) => Some(0x0080_8080), // GRAY_BRUSH
        Some(3) => Some(0x0040_4040), // DKGRAY_BRUSH
        Some(4) => Some(0x0000_0000), // BLACK_BRUSH
        Some(5) => None,              // NULL_BRUSH / HOLLOW_BRUSH
        _ => Some(0x0000_0000),
    }
}

/// Implementation of `USER32.dll!FillRect(hDC, lprc, hbr)`.
/// Paints onto DirectDraw surface DCs; acts as a validating no-op on window DCs.
pub(super) fn fill_rect(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (dc, rect_ptr, brush) = (args[0], args[1], args[2]);
    if rect_ptr == 0 || brush == 0 {
        return Ok(0);
    }
    let Some(target) = surface_target(ctx, dc, "USER32.dll!FillRect")? else {
        return Ok(1);
    };
    let Some(color) = brush_color(brush) else {
        return Ok(1);
    };
    let [left, top, right, bottom] = super::user32::read_rect(ctx, rect_ptr)?;
    let clip = ctx.state.gdi.dcs.get(&dc).and_then(|d| d.clip_rect);
    let (mut x0, mut y0, mut x1, mut y1) = (
        left.max(0),
        top.max(0),
        right.min(target.width),
        bottom.min(target.height),
    );
    if let Some([cl, ct, cr, cb]) = clip {
        x0 = x0.max(cl);
        y0 = y0.max(ct);
        x1 = x1.min(cr);
        y1 = y1.min(cb);
    }
    if x0 >= x1 || y0 >= y1 {
        return Ok(1);
    }
    let encoded = encode_colorref(target.format, color);
    for y in y0..y1 {
        for x in x0..x1 {
            put_pixel(ctx.mem, &target, None, x, y, encoded)?;
        }
    }
    Ok(1)
}

fn system_palette_entries(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

fn device_caps(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (width, height, depth) = ctx.state.display_mode;
    Ok(match args[1] {
        // HORZRES, VERTRES, BITSPIXEL, PLANES, LOGPIXELSX, LOGPIXELSY.
        8 => width,
        10 => height,
        12 => depth,
        14 => 1,
        88 | 90 => 96,
        _ => 0,
    })
}

fn region_data(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetRegionData(region, room, data).
    let (room, at) = (args[1], args[2]);
    let (width, height, _) = ctx.state.display_mode;
    let needed = 32 + 16;
    if at == 0 || room < needed {
        return Ok(needed);
    }
    let rect = match ctx.state.gdi.objects.get(&args[0]) {
        Some(GdiObject::Region { rect }) => [
            rect[0] as u32,
            rect[1] as u32,
            rect[2] as u32,
            rect[3] as u32,
        ],
        _ => [0, 0, width, height],
    };
    for (offset, value) in [
        (0, 32),
        (4, 1),
        (8, 1),
        (12, 16),
        (16, rect[0]),
        (20, rect[1]),
        (24, rect[2]),
        (28, rect[3]),
        (32, rect[0]),
        (36, rect[1]),
        (40, rect[2]),
        (44, rect[3]),
    ] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(needed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::tests::with_test_ctx;

    #[test]
    fn solid_brush_and_delete_object_behave_like_gdi() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let black = create_solid_brush(ctx, &[0]).unwrap();
            let red = create_solid_brush(ctx, &[0x0000_00ff]).unwrap();
            assert_ne!(black, 0);
            assert_ne!(black, red);
            assert_eq!(delete_object(ctx, &[black]).unwrap(), 1);
            assert_eq!(delete_object(ctx, &[0]).unwrap(), 0);
        });
    }

    #[test]
    fn draw_text_text_out_fill_rect_and_line_to_render_onto_rgb565_surface_dc() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x20_0000, |ctx| {
            let pixels = ctx.state.heap.alloc(64 * 32 * 2);
            let surface = crate::com::create(
                ctx,
                &crate::win32::ddraw::DIRECT_DRAW_SURFACE,
                ComState::Surface {
                    pixels,
                    owned_pixels: true,
                    width: 64,
                    height: 32,
                    pitch: 128,
                    format: PixelFormat::RGB565,
                    primary: false,
                    back: 0,
                    caps: 0,
                    palette: 0,
                },
            )
            .unwrap();
            let dc = 0x0004_0000;
            ctx.state.surface_dcs.insert(dc, surface);

            let ttf_bytes = crate::ttf::tests::synthetic_square_ttf("Lego");
            let font = Font::parse(ttf_bytes.into()).unwrap();
            ctx.state
                .gdi
                .loaded_fonts
                .push(("C:\\Lego.TTF".to_owned(), Rc::new(font)));

            let lf = ctx.state.heap.alloc(60);
            ctx.mem.write(lf, &[0u8; 60]).unwrap();
            ctx.mem.write_u32(lf, (-16i32) as u32).unwrap();
            ctx.mem.write_u32(lf + 16, 400).unwrap();
            ctx.mem.write(lf + 28, b"Lego\0").unwrap();
            let hfont = create_font_indirect(ctx, &[lf]).unwrap();
            assert!(hfont >= FIRST_GDI_HANDLE);
            let prev = select_object(ctx, &[dc, hfont]).unwrap();
            assert_ne!(prev, 0);

            let hbr = create_solid_brush(ctx, &[0x00ff_ffff]).unwrap();
            let rect = ctx.state.heap.alloc(16);
            for (i, v) in [0u32, 0, 64, 32].iter().enumerate() {
                ctx.mem.write_u32(rect + (i as u32) * 4, *v).unwrap();
            }
            assert_eq!(fill_rect(ctx, &[dc, rect, hbr]).unwrap(), 1);
            assert_eq!(delete_object(ctx, &[hbr]).unwrap(), 1);

            set_bk_mode(ctx, &[dc, TRANSPARENT]).unwrap();
            set_text_color(ctx, &[dc, 0x0000_00ff]).unwrap();
            let text_ptr = ctx.intern("A").unwrap();
            let h = draw_text(ctx, &[dc, text_ptr, 0xffff_ffff, rect, 0]).unwrap();
            assert!(h > 0);

            // TextOutA renders blue glyph at (32, 0) and rejects negative count.
            set_text_color(ctx, &[dc, 0x00ff_0000]).unwrap();
            assert_eq!(
                text_out(ctx, &[dc, 32, 0, text_ptr, 0xffff_ffff]).unwrap(),
                0
            );
            assert_eq!(text_out(ctx, &[dc, 32, 0, text_ptr, 1]).unwrap(), 1);

            let hpen = create_pen(ctx, &[0, 1, 0x0000_ff00]).unwrap();
            select_object(ctx, &[dc, hpen]).unwrap();
            move_to_ex(ctx, &[dc, 2, 28, 0]).unwrap();
            line_to(ctx, &[dc, 20, 28]).unwrap();

            let raw = ctx.mem.slice(pixels, 64 * 32 * 2).unwrap();
            let count = |want: u16| {
                raw.as_chunks::<2>()
                    .0
                    .iter()
                    .filter(|c| u16::from_le_bytes(**c) == want)
                    .count()
            };
            let red_pixels = count(0xf800);
            let blue_pixels = count(0x001f);
            let green_pixels = count(0x07e0);
            let white_pixels = count(0xffff);
            assert!(red_pixels >= 40, "DrawTextA red glyph pixels: {red_pixels}");
            assert!(
                blue_pixels >= 40,
                "TextOutA blue glyph pixels: {blue_pixels}"
            );
            assert_eq!(green_pixels, 18, "LineTo excludes endpoint (2..20)");
            assert_eq!(
                red_pixels + blue_pixels + green_pixels + white_pixels,
                64 * 32
            );
        });
    }
}
