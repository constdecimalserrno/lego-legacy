//! DirectDraw.
//!
//! What a title draws into is guest memory: a surface is bytes the guest
//! writes and the machine presents (ADR-0002). Nothing is uploaded anywhere,
//! because the frame the guest made is already the frame to show.

use super::{ComState, Convention, Ctx, Shim, State};
use crate::com::{Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::raster::PixelFormat;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "DDRAW",
        name: "DirectDrawCreate",
        args: 3,
        convention: Convention::Stdcall,
        run: create,
    },
    Shim {
        dll: "DDRAW",
        name: "DirectDrawEnumerateA",
        args: 2,
        convention: Convention::Stdcall,
        run: enumerate,
    },
];

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DirectDrawCreate(guid, out, outer).
    let object = State::make_com(ctx, &DIRECT_DRAW, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("ddraw created at {object:#010x}"));
    Ok(S_OK)
}

fn enumerate(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine has one display: the primary one, which is named by
    // a null identifier. A title that enumerates is waiting to be told about
    // it, so the machine calls the guest's own callback with it and finishes
    // this call when that returns.
    let (callback, context) = (args[0], args[1]);
    let description = ctx.intern("Primary Display Driver").map_err(Stop::Fault)?;
    let name = ctx.intern("display").map_err(Stop::Fault)?;
    ctx.log
        .line("ddraw enumerate: one display, and it is the primary");
    if callback != 0 {
        ctx.call_guest = Some((callback, vec![0, description, name, context]));
    }
    Ok(S_OK)
}

/// `IDirectDraw`, in slot order.
pub static DIRECT_DRAW: Interface = Interface {
    name: "IDirectDraw",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Compact",
            args: 0,
            run: ok,
        },
        Method {
            name: "CreateClipper",
            args: 3,
            run: create_clipper,
        },
        Method {
            name: "CreatePalette",
            args: 4,
            run: create_palette,
        },
        Method {
            name: "CreateSurface",
            args: 3,
            run: create_surface,
        },
        Method {
            name: "DuplicateSurface",
            args: 2,
            run: ok,
        },
        Method {
            name: "EnumDisplayModes",
            args: 4,
            run: enum_display_modes,
        },
        Method {
            name: "EnumSurfaces",
            args: 4,
            run: ok,
        },
        Method {
            name: "FlipToGDISurface",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetCaps",
            args: 2,
            run: caps,
        },
        Method {
            name: "GetDisplayMode",
            args: 1,
            run: display_mode,
        },
        Method {
            name: "GetFourCCCodes",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetGDISurface",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetMonitorFrequency",
            args: 1,
            run: monitor_frequency,
        },
        Method {
            name: "GetScanLine",
            args: 1,
            run: scan_line,
        },
        Method {
            name: "GetVerticalBlankStatus",
            args: 1,
            run: vertical_blank,
        },
        Method {
            name: "Initialize",
            args: 1,
            run: ok,
        },
        Method {
            name: "RestoreDisplayMode",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetCooperativeLevel",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetDisplayMode",
            args: 3,
            run: set_display_mode,
        },
        Method {
            name: "WaitForVerticalBlank",
            args: 2,
            run: ok,
        },
    ],
};

/// `IDirectDrawSurface`, in slot order.
pub static DIRECT_DRAW_SURFACE: Interface = Interface {
    name: "IDirectDrawSurface",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "AddAttachedSurface",
            args: 1,
            run: attach,
        },
        Method {
            name: "AddOverlayDirtyRect",
            args: 1,
            run: ok,
        },
        Method {
            name: "Blt",
            args: 5,
            run: blit,
        },
        Method {
            name: "BltBatch",
            args: 3,
            run: ok,
        },
        Method {
            name: "BltFast",
            args: 5,
            run: blit_fast,
        },
        Method {
            name: "DeleteAttachedSurface",
            args: 2,
            run: ok,
        },
        Method {
            name: "EnumAttachedSurfaces",
            args: 2,
            run: ok,
        },
        Method {
            name: "EnumOverlayZOrders",
            args: 3,
            run: ok,
        },
        Method {
            name: "Flip",
            args: 2,
            run: flip,
        },
        Method {
            name: "GetAttachedSurface",
            args: 2,
            run: attached_surface,
        },
        Method {
            name: "GetBltStatus",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetCaps",
            args: 1,
            run: surface_caps,
        },
        Method {
            name: "GetClipper",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetColorKey",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetDC",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetFlipStatus",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetOverlayPosition",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetPalette",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetPixelFormat",
            args: 1,
            run: pixel_format,
        },
        Method {
            name: "GetSurfaceDesc",
            args: 1,
            run: surface_desc,
        },
        Method {
            name: "Initialize",
            args: 2,
            run: ok,
        },
        Method {
            name: "IsLost",
            args: 0,
            run: ok,
        },
        Method {
            name: "Lock",
            args: 4,
            run: lock,
        },
        Method {
            name: "ReleaseDC",
            args: 1,
            run: ok,
        },
        Method {
            name: "Restore",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetClipper",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetColorKey",
            args: 2,
            run: set_color_key,
        },
        Method {
            name: "SetOverlayPosition",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetPalette",
            args: 1,
            run: set_palette,
        },
        Method {
            name: "Unlock",
            args: 1,
            run: unlock,
        },
        Method {
            name: "UpdateOverlay",
            args: 5,
            run: ok,
        },
        Method {
            name: "UpdateOverlayDisplay",
            args: 1,
            run: ok,
        },
        Method {
            name: "UpdateOverlayZOrder",
            args: 2,
            run: ok,
        },
    ],
};

/// `IDirectDrawPalette`, in slot order.
pub static DIRECT_DRAW_PALETTE: Interface = Interface {
    name: "IDirectDrawPalette",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "GetCaps",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetEntries",
            args: 4,
            run: get_entries,
        },
        Method {
            name: "Initialize",
            args: 3,
            run: ok,
        },
        Method {
            name: "SetEntries",
            args: 4,
            run: set_entries,
        },
    ],
};

/// `IDirectDrawClipper`, in slot order.
pub static DIRECT_DRAW_CLIPPER: Interface = Interface {
    name: "IDirectDrawClipper",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "GetClipList",
            args: 3,
            run: ok,
        },
        Method {
            name: "GetHWnd",
            args: 1,
            run: ok,
        },
        Method {
            name: "Initialize",
            args: 2,
            run: ok,
        },
        Method {
            name: "IsClipListChanged",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetClipList",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetHWnd",
            args: 2,
            run: ok,
        },
    ],
};

/// `IDirectDraw4`: DirectX 6's own DirectDraw, which is what the roster asks
/// for. The same machine underneath; four more slots at the end, and a display
/// mode that takes a refresh rate.
pub static DIRECT_DRAW4: Interface = Interface {
    name: "IDirectDraw4",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Compact",
            args: 0,
            run: ok,
        },
        Method {
            name: "CreateClipper",
            args: 3,
            run: create_clipper,
        },
        Method {
            name: "CreatePalette",
            args: 4,
            run: create_palette,
        },
        Method {
            name: "CreateSurface",
            args: 3,
            run: create_surface,
        },
        Method {
            name: "DuplicateSurface",
            args: 2,
            run: ok,
        },
        Method {
            name: "EnumDisplayModes",
            args: 4,
            run: enum_display_modes4,
        },
        Method {
            name: "EnumSurfaces",
            args: 4,
            run: ok,
        },
        Method {
            name: "FlipToGDISurface",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetCaps",
            args: 2,
            run: caps,
        },
        Method {
            name: "GetDisplayMode",
            args: 1,
            run: display_mode,
        },
        Method {
            name: "GetFourCCCodes",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetGDISurface",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetMonitorFrequency",
            args: 1,
            run: monitor_frequency,
        },
        Method {
            name: "GetScanLine",
            args: 1,
            run: scan_line,
        },
        Method {
            name: "GetVerticalBlankStatus",
            args: 1,
            run: vertical_blank,
        },
        Method {
            name: "Initialize",
            args: 1,
            run: ok,
        },
        Method {
            name: "RestoreDisplayMode",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetCooperativeLevel",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetDisplayMode",
            args: 5,
            run: set_display_mode,
        },
        Method {
            name: "WaitForVerticalBlank",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetAvailableVidMem",
            args: 3,
            run: available_video_memory,
        },
        Method {
            name: "GetSurfaceFromDC",
            args: 2,
            run: ok,
        },
        Method {
            name: "RestoreAllSurfaces",
            args: 0,
            run: ok,
        },
        Method {
            name: "TestCooperativeLevel",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetDeviceIdentifier",
            args: 2,
            run: device_identifier,
        },
    ],
};

/// `IDirectDrawSurface4`: the same surface, with the slots DirectX 6 added.
pub static DIRECT_DRAW_SURFACE4: Interface = Interface {
    name: "IDirectDrawSurface4",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "AddAttachedSurface",
            args: 1,
            run: attach,
        },
        Method {
            name: "AddOverlayDirtyRect",
            args: 1,
            run: ok,
        },
        Method {
            name: "Blt",
            args: 5,
            run: blit,
        },
        Method {
            name: "BltBatch",
            args: 3,
            run: ok,
        },
        Method {
            name: "BltFast",
            args: 5,
            run: blit_fast,
        },
        Method {
            name: "DeleteAttachedSurface",
            args: 2,
            run: ok,
        },
        Method {
            name: "EnumAttachedSurfaces",
            args: 2,
            run: ok,
        },
        Method {
            name: "EnumOverlayZOrders",
            args: 3,
            run: ok,
        },
        Method {
            name: "Flip",
            args: 2,
            run: flip,
        },
        Method {
            name: "GetAttachedSurface",
            args: 2,
            run: attached_surface,
        },
        Method {
            name: "GetBltStatus",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetCaps",
            args: 1,
            run: surface_caps,
        },
        Method {
            name: "GetClipper",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetColorKey",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetDC",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetFlipStatus",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetOverlayPosition",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetPalette",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetPixelFormat",
            args: 1,
            run: pixel_format,
        },
        Method {
            name: "GetSurfaceDesc",
            args: 1,
            run: surface_desc,
        },
        Method {
            name: "Initialize",
            args: 2,
            run: ok,
        },
        Method {
            name: "IsLost",
            args: 0,
            run: ok,
        },
        Method {
            name: "Lock",
            args: 4,
            run: lock,
        },
        Method {
            name: "ReleaseDC",
            args: 1,
            run: ok,
        },
        Method {
            name: "Restore",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetClipper",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetColorKey",
            args: 2,
            run: set_color_key,
        },
        Method {
            name: "SetOverlayPosition",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetPalette",
            args: 1,
            run: set_palette,
        },
        Method {
            name: "Unlock",
            args: 1,
            run: unlock,
        },
        Method {
            name: "UpdateOverlay",
            args: 5,
            run: ok,
        },
        Method {
            name: "UpdateOverlayDisplay",
            args: 1,
            run: ok,
        },
        Method {
            name: "UpdateOverlayZOrder",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetDDInterface",
            args: 1,
            run: ok,
        },
        Method {
            name: "PageLock",
            args: 1,
            run: ok,
        },
        Method {
            name: "PageUnlock",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetSurfaceDesc",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetPrivateData",
            args: 4,
            run: ok,
        },
        Method {
            name: "GetPrivateData",
            args: 3,
            run: ok,
        },
        Method {
            name: "FreePrivateData",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetUniquenessValue",
            args: 1,
            run: uniqueness,
        },
        Method {
            name: "ChangeUniquenessValue",
            args: 0,
            run: ok,
        },
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &DIRECT_DRAW4,
    &DIRECT_DRAW_SURFACE4,
    &DIRECT_DRAW,
    &DIRECT_DRAW_SURFACE,
    &DIRECT_DRAW_PALETTE,
    &DIRECT_DRAW_CLIPPER,
];

fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn caps(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's graphics hardware: it can do what a title needs
    // and claims nothing it cannot.
    for at in [args[0], args[1]] {
        if at == 0 {
            continue;
        }
        let size = ctx.mem.read_u32(at).map_err(Stop::Fault)?.clamp(4, 0x200);
        for offset in (4..size).step_by(4) {
            ctx.mem.write_u32(at + offset, 0).map_err(Stop::Fault)?;
        }
        // DDCAPS_BLT | DDCAPS_BLTCOLORFILL | DDCAPS_PALETTE.
        ctx.mem
            .write_u32(at + 4, 0x0000_5040)
            .map_err(Stop::Fault)?;
        // Four megabytes of video memory, declared.
        ctx.mem
            .write_u32(at + 0x60, 4 * 1024 * 1024)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn display_mode(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (width, height, depth) = ctx.state.display_mode;
    let at = args[0];
    if at != 0 {
        write_surface_desc(ctx, at, width, height, screen_format(depth), 0)?;
    }
    Ok(S_OK)
}

fn set_display_mode(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // What the guest asked its screen to be. A declared machine offers what a
    // title asks for, because refusing would send it down a path nobody has
    // measured.
    ctx.state.display_mode = (args[0], args[1], args[2]);
    ctx.log
        .line(&format!("display mode {}x{}x{}", args[0], args[1], args[2]));
    Ok(S_OK)
}

/// How often the declared display refreshes: the title's own declaration when
/// its flips wait for one, and otherwise sixty a second, what a monitor of the
/// era ran a 640x480 desktop at.
fn refresh_hz(ctx: &Ctx<'_>) -> u64 {
    match ctx.title.flip_cost {
        crate::clock::FlipCost::Refresh(hz) => hz.max(1),
        _ => 60,
    }
}

fn monitor_frequency(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        let hz = u32::try_from(refresh_hz(ctx)).unwrap_or(60);
        ctx.mem.write_u32(args[0], hz).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn scan_line(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Where the beam is, derived from the declared clock so that a title
    // waiting for it sees it move at the declared refresh on every run.
    let (_, height, _) = ctx.state.display_mode;
    let period_micros = 1_000_000 / refresh_hz(ctx);
    let into = (ctx.clock_millis * 1000) % period_micros;
    let line = u32::try_from(into * u64::from(height.max(1)) / period_micros).unwrap_or(0);
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], line).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn vertical_blank(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], 0).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// The pixel format a surface of the screen's depth has: 5-6-5 at sixteen
/// bits, eight bits a channel above that, and indexed at eight.
#[must_use]
pub(crate) fn screen_format(depth: u32) -> PixelFormat {
    PixelFormat {
        bits: depth,
        masks: match depth {
            16 => PixelFormat::RGB565.masks,
            24 | 32 => [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0],
            _ => [0; 4],
        },
    }
}

/// Bytes a surface's pixels are followed by that belong to nobody else.
///
/// A title that converts its textures four pixels at a time, a whole word a
/// pixel, writes a texture whose width is not a multiple of four up to six
/// bytes past its last row; Rock Raiders does. On DirectDraw that lands in the
/// allocator's own slack; here it would land on the next thing allocated. Two
/// bytes more round it to a word.
const SURFACE_SLACK: u32 = 8;

/// How many bytes a row of a surface takes: its pixels, padded to a whole
/// number of four-byte words, as DirectDraw lays out a surface in system
/// memory. Rock Raiders copies its images in padded rows and relies on it.
#[must_use]
pub(crate) fn pitch_for(width: u32, bits: u32) -> u32 {
    (width * bits.div_ceil(8)).next_multiple_of(4)
}

/// `DDPF_ALPHAPIXELS`, `DDPF_PALETTEINDEXED8` and `DDPF_RGB`.
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_PALETTEINDEXED8: u32 = 0x20;
const DDPF_RGB: u32 = 0x40;

/// Write a `DDPIXELFORMAT`: size, flags, no FourCC, bits, and masks.
fn write_pixel_format(ctx: &mut Ctx<'_>, at: u32, format: PixelFormat) -> Result<(), Stop> {
    let indexed = format.bits == 8 && format.masks == [0; 4];
    let flags = DDPF_RGB
        | if indexed { DDPF_PALETTEINDEXED8 } else { 0 }
        | if format.masks[3] != 0 {
            DDPF_ALPHAPIXELS
        } else {
            0
        };
    let [r, g, b, a] = format.masks;
    for (offset, value) in [
        (0, 32),
        (4, flags),
        (8, 0),
        (12, format.bits),
        (16, r),
        (20, g),
        (24, b),
        (28, a),
    ] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(())
}

/// Write a DDSURFACEDESC the way a guest reads one back.
pub(crate) fn write_surface_desc(
    ctx: &mut Ctx<'_>,
    at: u32,
    width: u32,
    height: u32,
    format: PixelFormat,
    pixels: u32,
) -> Result<(), Stop> {
    let pitch = pitch_for(width, format.bits);
    // dwSize, dwFlags, dwHeight, dwWidth, lPitch, ... lpSurface at 0x24.
    for (offset, value) in [
        (0, 108),
        (4, 0x0000_100f),
        (8, height),
        (12, width),
        (16, pitch),
        (0x24, pixels),
    ] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    // The pixel format, at 0x48.
    write_pixel_format(ctx, at + 0x48, format)
}

fn create_surface(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateSurface(descriptor, out, outer). What the guest asked for is in
    // the descriptor; what it gets back is guest memory it can write to.
    let at = args[0];
    let flags = ctx.mem.read_u32(at + 4).map_err(Stop::Fault)?;
    let caps = ctx.mem.read_u32(at + 0x68).map_err(Stop::Fault)?;
    let (screen_w, screen_h, screen_depth) = ctx.state.display_mode;

    // A primary surface is the screen's own size; anything else says its own.
    let primary = caps & 0x0000_0200 != 0;
    let width = if primary || flags & 4 == 0 {
        screen_w
    } else {
        ctx.mem.read_u32(at + 12).map_err(Stop::Fault)?
    };
    let height = if primary || flags & 2 == 0 {
        screen_h
    } else {
        ctx.mem.read_u32(at + 8).map_err(Stop::Fault)?
    };
    // A surface asked for in a pixel format of its own has it: a title's
    // textures and the images it converts them from are not the screen's.
    const DDSD_PIXELFORMAT: u32 = 0x1000;
    let format = if primary || flags & DDSD_PIXELFORMAT == 0 {
        screen_format(screen_depth)
    } else {
        let read = |o: u32| ctx.mem.read_u32(at + 0x48 + o).map_err(Stop::Fault);
        let (pf_flags, bits) = (read(4)?, read(12)?);
        let alpha = if pf_flags & DDPF_ALPHAPIXELS != 0 {
            read(28)?
        } else {
            0
        };
        if pf_flags & DDPF_PALETTEINDEXED8 != 0 && bits == 8 {
            PixelFormat {
                bits: 8,
                masks: [0; 4],
            }
        } else if pf_flags & !(DDPF_RGB | DDPF_ALPHAPIXELS) == 0 && matches!(bits, 16 | 24 | 32) {
            PixelFormat {
                bits,
                masks: [read(16)?, read(20)?, read(24)?, alpha],
            }
        } else {
            return Err(Stop::NotWritten {
                what: format!(
                    "IDirectDraw::CreateSurface: pixel format flags {pf_flags:#x}, {bits} bits"
                ),
                caller: 0,
            });
        }
    };
    let depth = format.bits;

    let (object, pixels) = make_surface_as(ctx, width, height, format, primary)?;
    let pitch = pitch_for(width, depth);

    // A surface asked for with a back buffer gets one, attached to it.
    let back_count = if flags & 0x20 != 0 {
        ctx.mem.read_u32(at + 0x14).map_err(Stop::Fault)?
    } else {
        0
    };
    if back_count > 0 {
        let back_pixels = ctx.state.heap.alloc(pitch * height.max(1) + SURFACE_SLACK);
        let back = State::make_com(
            ctx,
            &DIRECT_DRAW_SURFACE,
            ComState::Surface {
                pixels: back_pixels,
                width,
                height,
                pitch,
                format,
                primary: false,
                back: 0,
                palette: 0,
            },
        )
        .map_err(Stop::Fault)?;
        if let Some(object) = ctx.state.com_mut(object) {
            if let ComState::Surface { back: slot, .. } = &mut object.state {
                *slot = back;
            }
        }
    }

    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    let asked = if flags & DDSD_PIXELFORMAT != 0 && ctx.log.trace() >= crate::cli::Trace::Shims {
        let read = |o: u32| ctx.mem.read_u32(at + 0x48 + o).unwrap_or(0);
        format!(
            " asked for format {:#x} {} bits {:#x}/{:#x}/{:#x}/{:#x} caps {caps:#x}",
            read(4),
            read(12),
            read(16),
            read(20),
            read(24),
            read(28)
        )
    } else {
        String::new()
    };
    ctx.log.line(&format!(
        "surface {width}x{height}x{depth} at {pixels:#010x}{}{asked}",
        if primary { " primary" } else { "" }
    ));
    Ok(S_OK)
}

/// A surface of guest memory, as `CreateSurface` makes one and anything
/// else that hands a guest a surface of its own.
///
/// Answers the object and where its pixels are.
///
/// # Errors
/// When the heap is exhausted or the write faults.
pub(crate) fn make_surface(
    ctx: &mut Ctx<'_>,
    width: u32,
    height: u32,
    depth: u32,
    primary: bool,
) -> Result<(u32, u32), Stop> {
    make_surface_as(ctx, width, height, screen_format(depth), primary)
}

/// A surface in a pixel format of its own.
fn make_surface_as(
    ctx: &mut Ctx<'_>,
    width: u32,
    height: u32,
    format: PixelFormat,
    primary: bool,
) -> Result<(u32, u32), Stop> {
    let pitch = pitch_for(width, format.bits);
    let pixels = ctx.state.heap.alloc(pitch * height.max(1) + SURFACE_SLACK);
    let object = State::make_com(
        ctx,
        &DIRECT_DRAW_SURFACE,
        ComState::Surface {
            pixels,
            width,
            height,
            pitch,
            format,
            primary,
            back: 0,
            palette: 0,
        },
    )
    .map_err(Stop::Fault)?;
    Ok((object, pixels))
}

fn surface_of(ctx: &Ctx<'_>, this: u32) -> Option<(u32, u32, u32, u32, u32, bool, u32)> {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Surface {
            pixels,
            width,
            height,
            pitch,
            format,
            primary,
            back,
            ..
        }) => Some((
            *pixels,
            *width,
            *height,
            *pitch,
            format.bits,
            *primary,
            *back,
        )),
        _ => None,
    }
}

/// A surface's pixel format.
fn format_of(ctx: &Ctx<'_>, this: u32) -> PixelFormat {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Surface { format, .. }) => *format,
        _ => screen_format(16),
    }
}

fn surface_desc(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some((pixels, width, height, ..)) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let format = format_of(ctx, this);
    write_surface_desc(ctx, args[0], width, height, format, pixels)?;
    Ok(S_OK)
}

fn surface_caps(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let primary = surface_of(ctx, this).is_some_and(|s| s.5);
    if args[0] != 0 {
        // DDSCAPS_PRIMARYSURFACE or DDSCAPS_OFFSCREENPLAIN.
        ctx.mem
            .write_u32(args[0], if primary { 0x200 } else { 0x40 })
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn pixel_format(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let format = format_of(ctx, this);
    write_pixel_format(ctx, args[0], format)?;
    Ok(S_OK)
}

fn lock(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Lock(rect, descriptor, flags, event). The guest is given the address of
    // its own pixels: the surface is guest memory and always was.
    let Some((pixels, width, height, ..)) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let format = format_of(ctx, this);
    write_surface_desc(ctx, args[1], width, height, format, pixels)?;
    Ok(S_OK)
}

fn unlock(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn attach(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn attached_surface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // What a title asks for right after making a flipping chain: the buffer
    // behind the one on screen.
    let back = surface_of(ctx, this).map_or(0, |s| s.6);
    ctx.mem.write_u32(args[1], back).map_err(Stop::Fault)?;
    if back == 0 {
        // DDERR_NOTFOUND.
        return Ok(0x8878_0033);
    }
    Ok(S_OK)
}

fn flip(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The back buffer becomes the screen. This is the moment a frame exists,
    // and it is what the machine presents.
    let Some((pixels, width, height, pitch, depth, _, back)) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let source = surface_of(ctx, back).map_or(pixels, |s| s.0);
    if source != pixels {
        let bytes = ctx
            .mem
            .slice(source, (pitch * height) as usize)
            .map_err(Stop::Fault)?
            .to_vec();
        ctx.mem.write(pixels, &bytes).map_err(Stop::Fault)?;
    }
    ctx.state.present = Some(super::Present {
        pixels,
        width,
        height,
        pitch,
        depth,
    });
    Ok(S_OK)
}

fn blit(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Blt(destination rect, source, source rect, flags, effects).
    const DDBLT_ASYNC: u32 = 0x0000_0200;
    const DDBLT_COLORFILL: u32 = 0x0000_0400;
    const DDBLT_KEYSRC: u32 = 0x0000_8000;
    const DDBLT_WAIT: u32 = 0x0100_0000;
    let flags = args[3];
    if flags & !(DDBLT_ASYNC | DDBLT_COLORFILL | DDBLT_KEYSRC | DDBLT_WAIT) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface::Blt: flags {flags:#x}"),
            caller: 0,
        });
    }
    let Some((pixels, width, height, pitch, depth, _, _)) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let to = rect(ctx, args[0], width, height)?;
    // DDBLT_COLORFILL: the fill value is the effects block's own first word.
    if flags & DDBLT_COLORFILL != 0 {
        let colour = if args[4] == 0 {
            0
        } else {
            ctx.mem.read_u32(args[4] + 0x14).map_err(Stop::Fault)?
        };
        fill(ctx, pixels, pitch, depth, to, colour)?;
        return Ok(S_OK);
    }
    let Some((source, source_w, source_h, source_pitch, source_depth, _, _)) =
        surface_of(ctx, args[1])
    else {
        return Ok(S_OK);
    };
    let (from_format, to_format) = (format_of(ctx, args[1]), format_of(ctx, this));
    if from_format != to_format {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirectDrawSurface::Blt: from {source_depth} bits {:x?} to {depth} bits {:x?}",
                from_format.masks, to_format.masks
            ),
            caller: 0,
        });
    }
    let from = rect(ctx, args[2], source_w, source_h)?;
    let key = source_key(ctx, source, flags & DDBLT_KEYSRC != 0);
    transfer(
        ctx,
        Pixels {
            at: source,
            pitch: source_pitch,
        },
        from,
        Pixels { at: pixels, pitch },
        to,
        depth,
        key,
    )?;
    Ok(S_OK)
}

fn blit_fast(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // BltFast(x, y, source, source rect, flags).
    const DDBLTFAST_SRCCOLORKEY: u32 = 0x1;
    const DDBLTFAST_WAIT: u32 = 0x10;
    let flags = args[4];
    if flags & !(DDBLTFAST_SRCCOLORKEY | DDBLTFAST_WAIT) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface::BltFast: flags {flags:#x}"),
            caller: 0,
        });
    }
    let Some((pixels, width, height, pitch, depth, _, _)) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let Some((source, source_w, source_h, source_pitch, _, _, _)) = surface_of(ctx, args[2]) else {
        return Ok(S_OK);
    };
    let (from_format, to_format) = (format_of(ctx, args[2]), format_of(ctx, this));
    if from_format != to_format {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirectDrawSurface::BltFast: from {} bits {:x?} to {} bits {:x?}",
                from_format.bits, from_format.masks, to_format.bits, to_format.masks
            ),
            caller: 0,
        });
    }
    let from = rect(ctx, args[3], source_w, source_h)?;
    let (x, y) = (args[0].min(width), args[1].min(height));
    // What lands off the destination's edge is clipped away.
    let w = (from.2 - from.0).min(width - x);
    let h = (from.3 - from.1).min(height - y);
    let key = source_key(ctx, source, flags & DDBLTFAST_SRCCOLORKEY != 0);
    transfer(
        ctx,
        Pixels {
            at: source,
            pitch: source_pitch,
        },
        (from.0, from.1, from.0 + w, from.1 + h),
        Pixels { at: pixels, pitch },
        (x, y, x + w, y + h),
        depth,
        key,
    )?;
    Ok(S_OK)
}

fn set_color_key(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetColorKey(flags, key). The source key is what a keyed blit leaves
    // out; it belongs to the pixels, which every face of a surface shares.
    const DDCKEY_SRCBLT: u32 = 0x8;
    let Some((pixels, ..)) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    if args[0] & !DDCKEY_SRCBLT != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface::SetColorKey: flags {:#x}", args[0]),
            caller: 0,
        });
    }
    if args[1] == 0 {
        ctx.state.color_keys.remove(&pixels);
    } else {
        let low = ctx.mem.read_u32(args[1]).map_err(Stop::Fault)?;
        let high = ctx.mem.read_u32(args[1] + 4).map_err(Stop::Fault)?;
        if low != high {
            return Err(Stop::NotWritten {
                what: format!("IDirectDrawSurface::SetColorKey: a range {low:#x}..{high:#x}"),
                caller: 0,
            });
        }
        ctx.state.color_keys.insert(pixels, low);
    }
    Ok(S_OK)
}

/// The colour a keyed blit leaves out, when the blit asks for one.
fn source_key(ctx: &Ctx<'_>, source: u32, keyed: bool) -> Option<u32> {
    keyed
        .then(|| ctx.state.color_keys.get(&source).copied())
        .flatten()
}

/// A rectangle the guest passed, or the whole surface when it passed none.
fn rect(ctx: &Ctx<'_>, at: u32, width: u32, height: u32) -> Result<(u32, u32, u32, u32), Stop> {
    if at == 0 {
        return Ok((0, 0, width, height));
    }
    let left = ctx.mem.read_u32(at).map_err(Stop::Fault)?;
    let top = ctx.mem.read_u32(at + 4).map_err(Stop::Fault)?;
    let right = ctx.mem.read_u32(at + 8).map_err(Stop::Fault)?;
    let bottom = ctx.mem.read_u32(at + 12).map_err(Stop::Fault)?;
    let right = right.min(width);
    let bottom = bottom.min(height);
    Ok((left.min(right), top.min(bottom), right, bottom))
}

fn fill(
    ctx: &mut Ctx<'_>,
    pixels: u32,
    pitch: u32,
    depth: u32,
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    colour: u32,
) -> Result<(), Stop> {
    let bytes = depth.div_ceil(8);
    let row: Vec<u8> = (x0..x1)
        .flat_map(|_| colour.to_le_bytes().into_iter().take(bytes as usize))
        .collect();
    for y in y0..y1 {
        ctx.mem
            .write(pixels + y * pitch + x0 * bytes, &row)
            .map_err(Stop::Fault)?;
    }
    Ok(())
}

/// Where a surface's pixels are, and how long its rows are.
#[derive(Clone, Copy)]
struct Pixels {
    at: u32,
    pitch: u32,
}

/// Copy one rectangle into another, stretching when their sizes differ.
///
/// A stretch takes the nearest source pixel, which is what DirectDraw's own
/// software blitter does for a surface in system memory. A source colour key
/// leaves the pixels that match it untouched.
fn transfer(
    ctx: &mut Ctx<'_>,
    source: Pixels,
    (sx0, sy0, sx1, sy1): (u32, u32, u32, u32),
    destination: Pixels,
    (dx0, dy0, dx1, dy1): (u32, u32, u32, u32),
    depth: u32,
    key: Option<u32>,
) -> Result<(), Stop> {
    let bytes = depth.div_ceil(8) as usize;
    let (sw, sh) = (sx1 - sx0, sy1 - sy0);
    let (dw, dh) = (dx1 - dx0, dy1 - dy0);
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return Ok(());
    }
    let key = key.map(|k| k.to_le_bytes());
    let mut row = vec![0u8; dw as usize * bytes];
    for y in 0..dh {
        let sy = sy0 + y * sh / dh;
        let line = ctx
            .mem
            .slice(source.at + sy * source.pitch, (sx1 as usize) * bytes)
            .map_err(Stop::Fault)?
            .to_vec();
        let at = destination.at + (dy0 + y) * destination.pitch + dx0 * bytes as u32;
        if key.is_some() {
            let len = row.len();
            row.copy_from_slice(ctx.mem.slice(at, len).map_err(Stop::Fault)?);
        }
        for x in 0..dw as usize {
            let sx = (sx0 as usize + x * sw as usize / dw as usize) * bytes;
            let pixel = &line[sx..sx + bytes];
            if key.is_some_and(|k| k[..bytes] == *pixel) {
                continue;
            }
            row[x * bytes..(x + 1) * bytes].copy_from_slice(pixel);
        }
        ctx.mem.write(at, &row).map_err(Stop::Fault)?;
    }
    Ok(())
}

/// How long a `DDSURFACEDESC` is, and DirectX 6's `DDSURFACEDESC2`.
const SURFACE_DESC_BYTES: u32 = 108;
const SURFACE_DESC2_BYTES: u32 = 124;

fn enum_display_modes(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    enumerate_modes(ctx, args, SURFACE_DESC_BYTES)
}

/// `IDirectDraw4`'s spelling, whose callback is handed the larger descriptor.
fn enum_display_modes4(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    enumerate_modes(ctx, args, SURFACE_DESC2_BYTES)
}

/// EnumDisplayModes(flags, filter, context, callback): the declared machine's
/// screen modes, which are the title crate's own, handed to the guest's
/// callback.
fn enumerate_modes(ctx: &mut Ctx<'_>, args: &[u32], size: u32) -> Result<u32, Stop> {
    let (context, callback) = (args[2], args[3]);
    let modes = ctx.title.declared.modes;
    // ponytail: one callback per enumeration, which is all a machine that
    // declares one mode needs. A second declared mode needs the callback's
    // answer read between calls, the way a dialog's turns are.
    let [(width, height, depth)] = modes else {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirectDraw::EnumDisplayModes: {} declared modes, and only one is handed out",
                modes.len()
            ),
            caller: 0,
        });
    };
    let desc = ctx.state.heap.alloc(size);
    ctx.mem
        .write(desc, &vec![0; size as usize])
        .map_err(Stop::Fault)?;
    write_surface_desc(ctx, desc, *width, *height, screen_format(*depth), 0)?;
    ctx.mem.write_u32(desc, size).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "ddraw modes: {width}x{height}x{depth}, the only one declared"
    ));
    if callback != 0 {
        ctx.call_guest = Some((callback, vec![desc, context]));
    }
    Ok(S_OK)
}

fn create_palette(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreatePalette(caps, entries, out, outer).
    let store = ctx.state.heap.alloc(256 * 4);
    if args[1] != 0 {
        let entries = ctx
            .mem
            .slice(args[1], 256 * 4)
            .map_err(Stop::Fault)?
            .to_vec();
        ctx.mem.write(store, &entries).map_err(Stop::Fault)?;
    }
    let object = State::make_com(ctx, &DIRECT_DRAW_PALETTE, ComState::Palette { store })
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_entries(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetEntries(flags, start, count, entries).
    let Some(ComState::Palette { store }) = ctx.state.com(this).map(|o| o.state.clone()) else {
        return Ok(S_OK);
    };
    let (start, count) = (args[1].min(256), args[2].min(256));
    let bytes = ctx
        .mem
        .slice(args[3], (count * 4) as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    ctx.mem
        .write(store + start * 4, &bytes)
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn get_entries(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(ComState::Palette { store }) = ctx.state.com(this).map(|o| o.state.clone()) else {
        return Ok(S_OK);
    };
    let (start, count) = (args[1].min(256), args[2].min(256));
    let bytes = ctx
        .mem
        .slice(store + start * 4, (count * 4) as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    ctx.mem.write(args[3], &bytes).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_palette(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The surface's pixels are looked up in the palette from now on; the
    // primary's is what the screen shows through.
    let Some(ComState::Palette { store }) = ctx.state.com(args[0]).map(|o| o.state.clone()) else {
        return Ok(S_OK);
    };
    if let Some(ComState::Surface {
        palette, primary, ..
    }) = ctx.state.com_mut(this).map(|o| &mut o.state)
    {
        *palette = args[0];
        if *primary {
            ctx.state.palette = store;
        }
    }
    Ok(S_OK)
}

fn create_clipper(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &DIRECT_DRAW_CLIPPER, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn available_video_memory(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's video memory, whole and free. A title sizes its
    // surfaces from this, so it must be the same on every run.
    let four_megabytes = 4 * 1024 * 1024;
    for out in [args[1], args[2]] {
        if out != 0 {
            ctx.mem
                .write_u32(out, four_megabytes)
                .map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn device_identifier(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DDDEVICEIDENTIFIER: two strings, then version and vendor numbers. The
    // declared machine's own graphics card, named rather than measured.
    let at = args[0];
    if at == 0 {
        return Ok(S_OK);
    }
    ctx.write_string(at, 512, "lego-legacy.drv")
        .map_err(Stop::Fault)?;
    ctx.write_string(at + 512, 512, "lego-legacy declared display")
        .map_err(Stop::Fault)?;
    for offset in (1024..1024 + 40).step_by(4) {
        ctx.mem.write_u32(at + offset, 0).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn uniqueness(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], 1).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}
