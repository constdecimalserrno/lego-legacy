//! DirectDraw.
//!
//! What a title draws into is guest memory: a surface is bytes the guest
//! writes and the machine presents (ADR-0002). Nothing is uploaded anywhere,
//! because the frame the guest made is already the frame to show.

use super::{stdcall, ComState, Ctx, Shim, State};
use crate::com::{method, ok, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use crate::raster::PixelFormat;

pub const SHIMS: &[Shim] = &[
    stdcall("DDRAW", "DirectDrawCreate", 3, create),
    stdcall("DDRAW", "DirectDrawCreateEx", 4, create_ex),
    stdcall("DDRAW", "DirectDrawEnumerateA", 2, enumerate),
];

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DirectDrawCreate(guid, out, outer).
    let object = State::make_com(ctx, &DIRECT_DRAW, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("ddraw created at {object:#010x}"));
    Ok(S_OK)
}

fn create_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DirectDrawCreateEx(guid, out, interface, outer). DirectX 7's way in: the
    // caller names the face it wants rather than asking for it afterwards, and
    // the only one this call is allowed to hand back is IDirectDraw7 (#57).
    let interface = crate::com::iid_text(&crate::com::read_iid(ctx, args[2])?);
    if interface != "15e65ec0-3b9c-11d2-b92f-00609797ea5b" {
        return Err(Stop::NotWritten {
            what: format!(
                "DDRAW.dll!DirectDrawCreateEx: asked for {interface}, and this call only makes \
                 IDirectDraw7"
            ),
            caller: 0,
        });
    }
    let object = State::make_com(ctx, &DIRECT_DRAW7, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("ddraw7 created at {object:#010x}"));
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
        method("Compact", 0, ok),
        method("CreateClipper", 3, create_clipper),
        method("CreatePalette", 4, create_palette),
        method("CreateSurface", 3, create_surface),
        method("DuplicateSurface", 2, ok),
        method("EnumDisplayModes", 4, enum_display_modes),
        method("EnumSurfaces", 4, ok),
        method("FlipToGDISurface", 0, ok),
        method("GetCaps", 2, caps),
        method("GetDisplayMode", 1, display_mode),
        method("GetFourCCCodes", 2, ok),
        method("GetGDISurface", 1, ok),
        method("GetMonitorFrequency", 1, monitor_frequency),
        method("GetScanLine", 1, scan_line),
        method("GetVerticalBlankStatus", 1, vertical_blank),
        method("Initialize", 1, ok),
        method("RestoreDisplayMode", 0, ok),
        method("SetCooperativeLevel", 2, ok),
        method("SetDisplayMode", 3, set_display_mode),
        method("WaitForVerticalBlank", 2, ok),
    ],
};

/// `IDirectDrawSurface`, in slot order.
pub static DIRECT_DRAW_SURFACE: Interface = Interface {
    name: "IDirectDrawSurface",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("AddAttachedSurface", 1, ok),
        method("AddOverlayDirtyRect", 1, ok),
        method("Blt", 5, blit),
        method("BltBatch", 3, ok),
        method("BltFast", 5, blit_fast),
        method("DeleteAttachedSurface", 2, ok),
        method("EnumAttachedSurfaces", 2, ok),
        method("EnumOverlayZOrders", 3, ok),
        method("Flip", 2, flip),
        method("GetAttachedSurface", 2, attached_surface),
        method("GetBltStatus", 1, ok),
        method("GetCaps", 1, surface_caps),
        method("GetClipper", 1, ok),
        method("GetColorKey", 2, get_color_key),
        method("GetDC", 1, get_dc),
        method("GetFlipStatus", 1, ok),
        method("GetOverlayPosition", 2, ok),
        method("GetPalette", 1, ok),
        method("GetPixelFormat", 1, pixel_format),
        method("GetSurfaceDesc", 1, surface_desc),
        method("Initialize", 2, ok),
        method("IsLost", 0, ok),
        method("Lock", 4, lock),
        method("ReleaseDC", 1, release_dc),
        method("Restore", 0, ok),
        method("SetClipper", 1, set_clipper),
        method("SetColorKey", 2, set_color_key),
        method("SetOverlayPosition", 2, ok),
        method("SetPalette", 1, set_palette),
        method("Unlock", 1, unlock),
        method("UpdateOverlay", 5, ok),
        method("UpdateOverlayDisplay", 1, ok),
        method("UpdateOverlayZOrder", 2, ok),
    ],
};

/// `IDirectDrawPalette`, in slot order.
pub static DIRECT_DRAW_PALETTE: Interface = Interface {
    name: "IDirectDrawPalette",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetCaps", 1, ok),
        method("GetEntries", 4, get_entries),
        method("Initialize", 3, ok),
        method("SetEntries", 4, set_entries),
    ],
};

/// `IDirectDrawClipper`, in slot order.
pub static DIRECT_DRAW_CLIPPER: Interface = Interface {
    name: "IDirectDrawClipper",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetClipList", 3, ok),
        method("GetHWnd", 1, ok),
        method("Initialize", 2, ok),
        method("IsClipListChanged", 1, ok),
        method("SetClipList", 2, set_clip_list),
        method("SetHWnd", 2, set_hwnd),
    ],
};

/// `IDirectDraw4`: DirectX 6's own DirectDraw, which is what Rock Raiders and
/// Hot Wheels ask for. The same machine underneath; four more slots at the
/// end, and a display mode that takes a refresh rate.
pub static DIRECT_DRAW4: Interface = Interface {
    name: "IDirectDraw4",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("Compact", 0, ok),
        method("CreateClipper", 3, create_clipper),
        method("CreatePalette", 4, create_palette),
        method("CreateSurface", 3, create_surface),
        method("DuplicateSurface", 2, ok),
        method("EnumDisplayModes", 4, enum_display_modes4),
        method("EnumSurfaces", 4, ok),
        method("FlipToGDISurface", 0, ok),
        method("GetCaps", 2, caps),
        method("GetDisplayMode", 1, display_mode),
        method("GetFourCCCodes", 2, ok),
        method("GetGDISurface", 1, ok),
        method("GetMonitorFrequency", 1, monitor_frequency),
        method("GetScanLine", 1, scan_line),
        method("GetVerticalBlankStatus", 1, vertical_blank),
        method("Initialize", 1, ok),
        method("RestoreDisplayMode", 0, ok),
        method("SetCooperativeLevel", 2, ok),
        method("SetDisplayMode", 5, set_display_mode),
        method("WaitForVerticalBlank", 2, ok),
        method("GetAvailableVidMem", 3, available_video_memory),
        method("GetSurfaceFromDC", 2, ok),
        method("RestoreAllSurfaces", 0, ok),
        method("TestCooperativeLevel", 0, ok),
        method("GetDeviceIdentifier", 2, device_identifier),
    ],
};

/// `IDirectDrawSurface4`: the same surface, with the slots DirectX 6 added.
pub static DIRECT_DRAW_SURFACE4: Interface = Interface {
    name: "IDirectDrawSurface4",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("AddAttachedSurface", 1, ok),
        method("AddOverlayDirtyRect", 1, ok),
        method("Blt", 5, blit),
        method("BltBatch", 3, ok),
        method("BltFast", 5, blit_fast),
        method("DeleteAttachedSurface", 2, ok),
        method("EnumAttachedSurfaces", 2, ok),
        method("EnumOverlayZOrders", 3, ok),
        method("Flip", 2, flip),
        method("GetAttachedSurface", 2, attached_surface),
        method("GetBltStatus", 1, ok),
        method("GetCaps", 1, surface_caps),
        method("GetClipper", 1, ok),
        method("GetColorKey", 2, get_color_key),
        method("GetDC", 1, get_dc),
        method("GetFlipStatus", 1, ok),
        method("GetOverlayPosition", 2, ok),
        method("GetPalette", 1, ok),
        method("GetPixelFormat", 1, pixel_format),
        method("GetSurfaceDesc", 1, surface_desc),
        method("Initialize", 2, ok),
        method("IsLost", 0, ok),
        method("Lock", 4, lock),
        method("ReleaseDC", 1, release_dc),
        method("Restore", 0, ok),
        method("SetClipper", 1, set_clipper),
        method("SetColorKey", 2, set_color_key),
        method("SetOverlayPosition", 2, ok),
        method("SetPalette", 1, set_palette),
        method("Unlock", 1, unlock),
        method("UpdateOverlay", 5, ok),
        method("UpdateOverlayDisplay", 1, ok),
        method("UpdateOverlayZOrder", 2, ok),
        method("GetDDInterface", 1, ok),
        method("PageLock", 1, ok),
        method("PageUnlock", 1, ok),
        method("SetSurfaceDesc", 2, set_surface_desc),
        method("SetPrivateData", 4, ok),
        method("GetPrivateData", 3, ok),
        method("FreePrivateData", 1, ok),
        method("GetUniquenessValue", 1, uniqueness),
        method("ChangeUniquenessValue", 0, ok),
    ],
};

/// `IDirectDraw7`: DirectX 7's DirectDraw, which is what Alpha Team asks its
/// DirectDraw object for before it will draw anything (#57).
///
/// The same machine underneath and the same table as DirectX 6's as far as
/// that one goes: every slot through `GetDeviceIdentifier` is in the same
/// place and means the same thing, and DirectX 7 adds two after it.
pub static DIRECT_DRAW7: Interface = Interface {
    name: "IDirectDraw7",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("Compact", 0, ok),
        method("CreateClipper", 3, create_clipper),
        method("CreatePalette", 4, create_palette),
        method("CreateSurface", 3, create_surface),
        method("DuplicateSurface", 2, ok),
        method("EnumDisplayModes", 4, enum_display_modes4),
        method("EnumSurfaces", 4, ok),
        method("FlipToGDISurface", 0, ok),
        method("GetCaps", 2, caps),
        method("GetDisplayMode", 1, display_mode),
        method("GetFourCCCodes", 2, ok),
        method("GetGDISurface", 1, ok),
        method("GetMonitorFrequency", 1, monitor_frequency),
        method("GetScanLine", 1, scan_line),
        method("GetVerticalBlankStatus", 1, vertical_blank),
        method("Initialize", 1, ok),
        method("RestoreDisplayMode", 0, ok),
        method("SetCooperativeLevel", 2, ok),
        method("SetDisplayMode", 5, set_display_mode),
        method("WaitForVerticalBlank", 2, ok),
        method("GetAvailableVidMem", 3, available_video_memory),
        method("GetSurfaceFromDC", 2, ok),
        method("RestoreAllSurfaces", 0, ok),
        method("TestCooperativeLevel", 0, ok),
        method("GetDeviceIdentifier", 2, device_identifier),
        wall("StartModeTest", 3),
        wall("EvaluateMode", 2),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &DIRECT_DRAW7,
    &DIRECT_DRAW4,
    &DIRECT_DRAW_SURFACE4,
    &DIRECT_DRAW,
    &DIRECT_DRAW_SURFACE,
    &DIRECT_DRAW_PALETTE,
    &DIRECT_DRAW_CLIPPER,
];

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
        // DDCAPS_3D | DDCAPS_BLT | DDCAPS_BLTCOLORFILL | DDCAPS_PALETTE.
        // The three-dimensional bit is this machine's own rasteriser saying it
        // is there: a title that does not see it here does not go looking for
        // a device at all (#57).
        ctx.mem
            .write_u32(at + 4, 0x0400_8041)
            .map_err(Stop::Fault)?;
        // The declared video memory, as GetAvailableVidMem says it, at
        // dwVidMemTotal and dwVidMemFree (#93).
        for offset in [0x3c, 0x40] {
            ctx.mem
                .write_u32(at + offset, VIDEO_MEMORY)
                .map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn display_mode(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (width, height, depth) = ctx.state.display_mode;
    let at = args[0];
    if at != 0 {
        let pitch = pitch_for(width, depth);
        write_surface_desc(ctx, at, (width, height, pitch), screen_format(depth), 0)?;
    }
    Ok(S_OK)
}

fn set_display_mode(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // What the guest asked its screen to be. A declared machine offers what a
    // title asks for, because refusing would send it down a path nobody has
    // measured.
    ctx.state.display_mode = (args[0], args[1], args[2]);
    // A new mode puts the pointer in the middle of it, as Windows does, and a
    // title that draws its own pointer starts that in the middle too: the
    // mouse's first move is measured from there (#48).
    let middle = (args[0] as i32 / 2, args[1] as i32 / 2);
    ctx.state.pointer = middle;
    ctx.state.last_move = middle;
    // The refresh rate the title asked for is recorded because whether it asks
    // at all is what settles the declared display's own rate (#82): a zero is
    // "whatever the machine runs at", which is a declared value.
    ctx.log.line(&format!(
        "display mode {}x{}x{} refresh {}",
        args[0],
        args[1],
        args[2],
        args.get(3).copied().unwrap_or(0)
    ));
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
pub(crate) fn write_pixel_format(
    ctx: &mut Ctx<'_>,
    at: u32,
    format: PixelFormat,
) -> Result<(), Stop> {
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

/// Write a DDSURFACEDESC the way a guest reads one back. Its `dwFlags` claim
/// `DDSD_CAPS`, and only `GetSurfaceDesc` writes the caps after it; that every
/// other caller writes them, or does not claim them, is #227's.
pub(crate) fn write_surface_desc(
    ctx: &mut Ctx<'_>,
    at: u32,
    (width, height, pitch): (u32, u32, u32),
    format: PixelFormat,
    pixels: u32,
) -> Result<(), Stop> {
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
        // A depth buffer is a surface like any other as far as DirectDraw is
        // concerned: so many bits a pixel, and the device writes them rather
        // than the title (#57).
        if pf_flags & DDPF_ZBUFFER != 0 {
            PixelFormat {
                bits,
                masks: [0; 4],
            }
        } else if pf_flags & DDPF_PALETTEINDEXED8 != 0 && bits == 8 {
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

    // A surface asked for with a back buffer gets one, attached to it.
    let back_count = if flags & 0x20 != 0 {
        ctx.mem.read_u32(at + 0x14).map_err(Stop::Fault)?
    } else {
        0
    };
    // What it says it is: what it was asked to be, in video memory unless
    // asked for system memory, and on the screen if it is the screen, its
    // front buffer if it flips; as Wine's conformance tests `test_rt_caps`,
    // `test_primary_caps` and `test_flip` find Windows answering.
    let mut given = caps;
    if given & ddscaps::SYSTEMMEMORY == 0 {
        given |= ddscaps::VIDEOMEMORY;
        if given & ddscaps::NONLOCALVIDMEM == 0 {
            given |= ddscaps::LOCALVIDMEM;
        }
    }
    if primary {
        given |= ddscaps::VISIBLE;
    }
    if back_count > 0 {
        given |= ddscaps::FRONTBUFFER;
    }

    let (object, pixels) = make_surface_as(ctx, width, height, format, given)?;
    let pitch = pitch_for(width, depth);
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
                // The buffer behind the screen, as `test_flip` finds it.
                caps: given & !(ddscaps::VISIBLE | ddscaps::PRIMARYSURFACE | ddscaps::FRONTBUFFER)
                    | ddscaps::BACKBUFFER,
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

/// A surface of guest memory, as anything that hands a guest a surface of
/// its own makes one: a plain one in system memory, as a movie stream makes
/// the surface of a sample a title gave none (Wine's `ddrawstream.c`).
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
) -> Result<(u32, u32), Stop> {
    let caps = ddscaps::SYSTEMMEMORY | ddscaps::OFFSCREENPLAIN;
    make_surface_as(ctx, width, height, screen_format(depth), caps)
}

/// A surface in a pixel format of its own, which is what `caps` says.
fn make_surface_as(
    ctx: &mut Ctx<'_>,
    width: u32,
    height: u32,
    format: PixelFormat,
    caps: u32,
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
            primary: caps & ddscaps::PRIMARYSURFACE != 0,
            caps,
            back: 0,
            palette: 0,
        },
    )
    .map_err(Stop::Fault)?;
    Ok((object, pixels))
}

/// `DDSCAPS_*`, as ddraw.h numbers them.
pub(crate) mod ddscaps {
    pub const BACKBUFFER: u32 = 0x0000_0004;
    pub const FRONTBUFFER: u32 = 0x0000_0020;
    pub const OFFSCREENPLAIN: u32 = 0x0000_0040;
    pub const PRIMARYSURFACE: u32 = 0x0000_0200;
    pub const SYSTEMMEMORY: u32 = 0x0000_0800;
    pub const VIDEOMEMORY: u32 = 0x0000_4000;
    pub const VISIBLE: u32 = 0x0000_8000;
    pub const MIPMAP: u32 = 0x0040_0000;
    pub const ALLOCONLOAD: u32 = 0x0400_0000;
    pub const LOCALVIDMEM: u32 = 0x1000_0000;
    pub const NONLOCALVIDMEM: u32 = 0x2000_0000;
}

#[derive(Clone, Copy)]
struct SurfaceInfo {
    pixels: u32,
    width: u32,
    height: u32,
    pitch: u32,
    format: PixelFormat,
    primary: bool,
    caps: u32,
    back: u32,
}

fn surface_of(ctx: &Ctx<'_>, this: u32) -> Option<SurfaceInfo> {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Surface {
            pixels,
            width,
            height,
            pitch,
            format,
            primary,
            caps,
            back,
            ..
        }) => Some(SurfaceInfo {
            pixels: *pixels,
            width: *width,
            height: *height,
            pitch: *pitch,
            format: *format,
            primary: *primary,
            caps: *caps,
            back: *back,
        }),
        _ => None,
    }
}

/// Where a surface's device contexts start, a word apart: a range no other
/// handle here is handed out from, USER32's dialog controls having all of
/// `0x0002_xxxx`.
const SURFACE_DC: u32 = 0x0004_0000;

/// `DDERR_INVALIDPARAMS`, `DDERR_DCALREADYCREATED`, `DDERR_NODC` and
/// `DDERR_SURFACEBUSY`.
const DDERR_INVALIDPARAMS: u32 = 0x8007_0057;
const DDERR_DCALREADYCREATED: u32 = 0x8876_026c;
const DDERR_NODC: u32 = 0x8876_00cb;
const DDERR_SURFACEBUSY: u32 = 0x8876_01ae;

/// Whether a surface has its device context out, which locks it until the DC
/// is let go of, as Microsoft's GetDC page says: what a DC draws, DrawDib
/// (#151), draws on the surface's own pixels.
fn dc_out(ctx: &Ctx<'_>, surface: u32) -> bool {
    if ctx.state.surface_dcs.is_empty() {
        return false;
    }
    let surface = ctx.state.identity(surface);
    ctx.state.surface_dcs.values().any(|s| *s == surface)
}

/// A blit or a flip touching a surface whose DC is out: what DirectDraw
/// answers for one is not documented, so it is a wall.
fn busy(ctx: &Ctx<'_>, this: u32, call: &str, surfaces: &[u32]) -> Result<(), Stop> {
    if ctx.state.surface_dcs.is_empty() {
        return Ok(());
    }
    match surfaces.iter().find(|s| **s != 0 && dc_out(ctx, **s)) {
        None => Ok(()),
        Some(s) => Err(Stop::NotWritten {
            what: format!(
                "{}::{call}: {s:#x} while its DC is out",
                ctx.state.interface_of(this).unwrap_or("IDirectDrawSurface")
            ),
            caller: 0,
        }),
    }
}

fn get_dc(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDC(dc out): a device context drawing onto the surface's own pixels,
    // one at a time, as ddraw.h's DDERR_DCALREADYCREATED says and Wine's
    // `surface.c`, read as documentation, answers a second without touching
    // what it was handed. What draws on one is DrawDib (#151); GDI's own
    // drawing calls are walls.
    if args[0] == 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    let surface = ctx.state.identity(this);
    if dc_out(ctx, this) {
        return Ok(DDERR_DCALREADYCREATED);
    }
    let mut dc = SURFACE_DC;
    while ctx.state.surface_dcs.contains_key(&dc) {
        dc += 4;
    }
    ctx.mem.write_u32(args[0], dc).map_err(Stop::Fault)?;
    ctx.state.surface_dcs.insert(dc, surface);
    Ok(S_OK)
}

fn release_dc(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // ReleaseDC(dc): the surface's device context let go of. A surface with
    // none out answers DDERR_NODC, as Wine's does; one handed another DC than
    // its own is a wall, since what Windows answers for it is not documented.
    // A screen with nothing behind it has its frame then, as letting go of a
    // lock of the whole of it does (`unlock`): Wine's ReleaseDC brings the
    // front buffer up to date.
    let surface = ctx.state.identity(this);
    let Some(&dc) = ctx
        .state
        .surface_dcs
        .iter()
        .find_map(|(dc, s)| (*s == surface).then_some(dc))
    else {
        return Ok(DDERR_NODC);
    };
    if args[0] != dc {
        return Err(Stop::NotWritten {
            what: format!(
                "{}::ReleaseDC: {:#x}, where the surface's own DC is {dc:#x}",
                ctx.state.interface_of(this).unwrap_or("IDirectDrawSurface"),
                args[0]
            ),
            caller: 0,
        });
    }
    ctx.state.surface_dcs.remove(&dc);
    if let Some(s) = surface_of(ctx, this).filter(|s| s.primary && s.back == 0) {
        ctx.state.present = Some(super::Present {
            pixels: s.pixels,
            width: s.width,
            height: s.height,
            pitch: s.pitch,
            depth: s.format.bits,
        });
    }
    Ok(S_OK)
}

fn surface_desc(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(s) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    write_surface_desc(
        ctx,
        args[0],
        (s.width, s.height, s.pitch),
        s.format,
        s.pixels,
    )?;
    // The caps `dwFlags` says are there, at 0x68 (#210).
    ctx.mem
        .write_u32(args[0] + 0x68, s.caps)
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn surface_caps(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let primary = surface_of(ctx, this).is_some_and(|s| s.primary);
    if args[0] != 0 {
        // DDSCAPS_PRIMARYSURFACE or DDSCAPS_OFFSCREENPLAIN, where
        // `GetSurfaceDesc` answers the surface's own caps: #227's.
        ctx.mem
            .write_u32(args[0], if primary { 0x200 } else { 0x40 })
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn pixel_format(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let format = surface_of(ctx, this).map_or_else(|| screen_format(16), |s| s.format);
    write_pixel_format(ctx, args[0], format)?;
    Ok(S_OK)
}

fn lock(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Lock(rect, descriptor, flags, event). The guest is given the address of
    // its own pixels: the surface is guest memory and always was. One whose DC
    // is out is already locked, which Wine's ddraw conformance test
    // `test_getdc`, which passes on Windows, finds answered DDERR_SURFACEBUSY.
    let Some(s) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    if dc_out(ctx, this) {
        return Ok(DDERR_SURFACEBUSY);
    }
    write_surface_desc(
        ctx,
        args[1],
        (s.width, s.height, s.pitch),
        s.format,
        s.pixels,
    )?;
    Ok(S_OK)
}

fn unlock(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // A title with a flipping chain presents by flipping, and this is the
    // middle of its frame. A title that has only the screen draws straight
    // onto it, and letting go of the whole of it is the moment its frame
    // exists: there is nothing else for it to become. Letting go of a corner
    // is not: a title that draws its frame in pieces has not finished one.
    let given = args.first().copied().unwrap_or(0);
    let Some(s) = surface_of(ctx, this).filter(|s| s.primary && s.back == 0) else {
        return Ok(S_OK);
    };
    // What this argument is depends on which interface the title holds.
    // `IDirectDrawSurface` through `IDirectDrawSurface4` take the pointer
    // `Lock` handed back; only `IDirectDrawSurface7` takes a rectangle. So the
    // surface's own address is the whole surface, as null is, and anything
    // else is a rectangle inside it (#41).
    let whole = given == 0 || given == s.pixels;
    if !whole {
        return Ok(S_OK);
    }
    ctx.state.present = Some(super::Present {
        pixels: s.pixels,
        width: s.width,
        height: s.height,
        pitch: s.pitch,
        depth: s.format.bits,
    });
    Ok(S_OK)
}

fn attached_surface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // What a title asks for right after making a flipping chain: the buffer
    // behind the one on screen.
    let back = surface_of(ctx, this).map_or(0, |s| s.back);
    ctx.mem.write_u32(args[1], back).map_err(Stop::Fault)?;
    if back == 0 {
        return Ok(DDERR_NOTFOUND);
    }
    Ok(S_OK)
}

fn flip(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The back buffer becomes the screen. This is the moment a frame exists,
    // and it is what the machine presents.
    let Some(front) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    busy(ctx, this, "Flip", &[this, front.back])?;
    let source = surface_of(ctx, front.back).map_or(front.pixels, |s| s.pixels);
    if source != front.pixels {
        let len = (front.pitch * front.height) as usize;
        let (s, d) = (u64::from(source), u64::from(front.pixels));
        if s + len as u64 <= d || d + len as u64 <= s {
            let (src, dst) = ctx
                .mem
                .read_and_write((source, len), (front.pixels, len))
                .map_err(Stop::Fault)?;
            dst.copy_from_slice(src);
        } else {
            let bytes = ctx.mem.slice(source, len).map_err(Stop::Fault)?.to_vec();
            ctx.mem.write(front.pixels, &bytes).map_err(Stop::Fault)?;
        }
    }
    ctx.state.present = Some(super::Present {
        pixels: front.pixels,
        width: front.width,
        height: front.height,
        pitch: front.pitch,
        depth: front.format.bits,
    });
    Ok(S_OK)
}

fn blit(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Blt(destination rect, source, source rect, flags, effects).
    busy(ctx, this, "Blt", &[this, args[1]])?;
    const DDBLT_ASYNC: u32 = 0x0000_0200;
    const DDBLT_COLORFILL: u32 = 0x0000_0400;
    const DDBLT_KEYSRC: u32 = 0x0000_8000;
    const DDBLT_ROP: u32 = 0x0002_0000;
    const DDBLT_WAIT: u32 = 0x0100_0000;
    let flags = args[3];
    if flags & !(DDBLT_ASYNC | DDBLT_COLORFILL | DDBLT_KEYSRC | DDBLT_ROP | DDBLT_WAIT) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface::Blt: flags {flags:#x}"),
            caller: 0,
        });
    }
    // DDBLT_ROP: the raster operation is the effects block's dwROP, eight
    // bytes in. SRCCOPY is the source copied as it is, which is what a blit
    // with no operation does: LEGO Island's registration book draws its
    // cursor this way (#124).
    if flags & DDBLT_ROP != 0 {
        const SRCCOPY: u32 = 0x00cc_0020;
        let rop = match args[4] {
            0 => None,
            fx => Some(ctx.mem.read_u32(fx + 8).map_err(Stop::Fault)?),
        };
        if rop != Some(SRCCOPY) {
            return Err(Stop::NotWritten {
                what: format!("IDirectDrawSurface::Blt: raster operation {rop:#x?}"),
                caller: 0,
            });
        }
    }
    let Some(dst) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let to = rect(ctx, args[0], dst.width, dst.height)?;
    // A blit reaching past its destination's edge is clipped when a clipper
    // is attached, and refused when none is, as DirectDraw does; one the
    // wrong way round is refused either way.
    let ordered = to.0 <= to.2 && to.1 <= to.3;
    if !ordered || (!ctx.state.clipped.contains(&dst.pixels) && !within(to, dst.width, dst.height))
    {
        return Ok(DDERR_INVALIDRECT);
    }
    // DDBLT_COLORFILL: the fill value is the effects block's dwFillColor,
    // eighty bytes in (#93).
    if flags & DDBLT_COLORFILL != 0 {
        let colour = if args[4] == 0 {
            0
        } else {
            ctx.mem.read_u32(args[4] + 0x50).map_err(Stop::Fault)?
        };
        let clip = |v: i64, edge: u32| v.clamp(0, i64::from(edge)) as u32;
        let area = (
            clip(to.0, dst.width),
            clip(to.1, dst.height),
            clip(to.2, dst.width),
            clip(to.3, dst.height),
        );
        if area.0 < area.2 && area.1 < area.3 {
            super::fill(ctx, (dst.pixels, dst.pitch, dst.format.bits), area, colour)?;
        }
        return Ok(S_OK);
    }
    let Some(src) = surface_of(ctx, args[1]) else {
        return Ok(S_OK);
    };
    if src.format != dst.format {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirectDrawSurface::Blt: from {} bits {:x?} to {} bits {:x?}",
                src.format.bits, src.format.masks, dst.format.bits, dst.format.masks
            ),
            caller: 0,
        });
    }
    let from = rect(ctx, args[2], src.width, src.height)?;
    // Only the destination is clipped: a source rectangle is the guest's to
    // keep inside its surface.
    if !within(from, src.width, src.height) {
        return Ok(DDERR_INVALIDRECT);
    }
    let key = source_key(ctx, src.pixels, flags & DDBLT_KEYSRC != 0);
    transfer(
        ctx.mem,
        Pixels {
            at: src.pixels,
            pitch: src.pitch,
        },
        from,
        Pixels {
            at: dst.pixels,
            pitch: dst.pitch,
        },
        to,
        (dst.width, dst.height),
        dst.format.bits,
        key,
    )?;
    // A copy that covers the whole of the screen is the moment a frame
    // exists, the same way letting go of the whole of it is (`unlock`): the
    // title has put a finished picture there and there is nothing else for it
    // to become. Alpha Team's movies arrive this way and no other — it blits
    // each decoded frame onto the front buffer of its flipping chain and
    // never flips, so without this its intro is a hundred and nine seconds of
    // black (#63). A fill is not a frame: that is how a title *starts* one.
    if dst.primary && to == (0, 0, i64::from(dst.width), i64::from(dst.height)) {
        ctx.state.present = Some(super::Present {
            pixels: dst.pixels,
            width: dst.width,
            height: dst.height,
            pitch: dst.pitch,
            depth: dst.format.bits,
        });
    }
    Ok(S_OK)
}

/// A surface that holds depth rather than colour.
pub(crate) const DDPF_ZBUFFER: u32 = 0x0000_0400;

/// `DDERR_INVALIDRECT` and `DDERR_BLTFASTCANTCLIP`.
const DDERR_INVALIDRECT: u32 = 0x8876_0096;
const DDERR_BLTFASTCANTCLIP: u32 = 0x8876_023e;
/// What a surface says when it has nothing of the kind that was asked for,
/// which Alpha Team reads as make one, where anything else is give up (#57).
const DDERR_NOTFOUND: u32 = 0x8876_00ff;

/// Whether a rectangle is the right way round and inside a surface.
fn within((left, top, right, bottom): (i64, i64, i64, i64), width: u32, height: u32) -> bool {
    0 <= left
        && left <= right
        && right <= i64::from(width)
        && 0 <= top
        && top <= bottom
        && bottom <= i64::from(height)
}

fn set_clipper(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetClipper(clipper): a clipper attached clips what is blitted to the
    // surface to its clip list; none, and a blit past its edge is refused.
    // What is written is a clip list of the whole surface, which clips to the
    // surface's own edges; anything less stops here.
    let Some(s) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    if args[0] == 0 {
        ctx.state.clipped.remove(&s.pixels);
        return Ok(S_OK);
    }
    let whole = (0, 0, s.width as i32, s.height as i32);
    match ctx.state.clip_lists.get(&ctx.state.identity(args[0])) {
        Some(list) if *list == whole => {
            ctx.state.clipped.insert(s.pixels);
            Ok(S_OK)
        }
        list => Err(Stop::NotWritten {
            what: format!(
                "IDirectDrawSurface::SetClipper: a clipper clipping to {list:?} on a {}x{} surface",
                s.width, s.height
            ),
            caller: 0,
        }),
    }
}

fn set_clip_list(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetClipList(region, flags): an RGNDATA, a header of four words and a
    // bounding rectangle, then its rectangles. One rectangle is a clip list
    // this machine keeps; more stop here.
    let identity = ctx.state.identity(this);
    if args[0] == 0 {
        ctx.state.clip_lists.remove(&identity);
        return Ok(S_OK);
    }
    let count = ctx.mem.read_u32(args[0] + 8).map_err(Stop::Fault)?;
    if count != 1 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawClipper::SetClipList: {count} rectangles"),
            caller: 0,
        });
    }
    let edge = |offset: u32| -> Result<i32, Stop> {
        Ok(ctx
            .mem
            .read_u32(args[0] + 32 + offset)
            .map_err(Stop::Fault)? as i32)
    };
    let list = (edge(0)?, edge(4)?, edge(8)?, edge(12)?);
    ctx.state.clip_lists.insert(identity, list);
    Ok(S_OK)
}

fn set_hwnd(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetHWnd(flags, window): the clipper clips to the part of the screen the
    // window shows, which on this machine is all of it, as `GetClientRect`
    // says (#117). No window, and it clips to nothing it was given.
    let identity = ctx.state.identity(this);
    if args[1] == 0 {
        ctx.state.clip_lists.remove(&identity);
        return Ok(S_OK);
    }
    let (width, height, _) = ctx.state.display_mode;
    ctx.state
        .clip_lists
        .insert(identity, (0, 0, width as i32, height as i32));
    Ok(S_OK)
}

fn blit_fast(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // BltFast(x, y, source, source rect, flags).
    busy(ctx, this, "BltFast", &[this, args[2]])?;
    const DDBLTFAST_SRCCOLORKEY: u32 = 0x1;
    const DDBLTFAST_WAIT: u32 = 0x10;
    let flags = args[4];
    if flags & !(DDBLTFAST_SRCCOLORKEY | DDBLTFAST_WAIT) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface::BltFast: flags {flags:#x}"),
            caller: 0,
        });
    }
    let Some(dst) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let Some(src) = surface_of(ctx, args[2]) else {
        return Ok(S_OK);
    };
    if src.format != dst.format {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirectDrawSurface::BltFast: from {} bits {:x?} to {} bits {:x?}",
                src.format.bits, src.format.masks, dst.format.bits, dst.format.masks
            ),
            caller: 0,
        });
    }
    // BltFast neither clips nor lets a clipper clip for it: a destination
    // with a clipper, or a blit that does not fit, is refused.
    if ctx.state.clipped.contains(&dst.pixels) {
        return Ok(DDERR_BLTFASTCANTCLIP);
    }
    let from = rect(ctx, args[3], src.width, src.height)?;
    let (x, y) = (i64::from(args[0] as i32), i64::from(args[1] as i32));
    let to = (x, y, x + from.2 - from.0, y + from.3 - from.1);
    if !within(from, src.width, src.height) || !within(to, dst.width, dst.height) {
        return Ok(DDERR_INVALIDRECT);
    }
    let key = source_key(ctx, src.pixels, flags & DDBLTFAST_SRCCOLORKEY != 0);
    transfer(
        ctx.mem,
        Pixels {
            at: src.pixels,
            pitch: src.pitch,
        },
        from,
        Pixels {
            at: dst.pixels,
            pitch: dst.pitch,
        },
        to,
        (dst.width, dst.height),
        dst.format.bits,
        key,
    )?;
    Ok(S_OK)
}

fn set_color_key(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetColorKey(flags, key). The source key is what a keyed blit leaves
    // out; it belongs to the pixels, which every face of a surface shares.
    let Some(s) = surface_of(ctx, this) else {
        return Ok(S_OK);
    };
    let pixels = s.pixels;
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

/// `DDCKEY_SRCBLT`: the key a blit or a texture leaves out, which is the only
/// one this machine keeps.
const DDCKEY_SRCBLT: u32 = 0x8;

fn get_color_key(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetColorKey(flags, key): the source key, low and high alike, or that
    // there is none with the key left as it was, as Wine's conformance test
    // `test_texture_load_ckey` finds Windows answering. LEGO Chess's DirectX
    // plugin hands what it reads on to each texture it loads (#210).
    const DDERR_NOCOLORKEY: u32 = 0x8876_00d7;
    let Some(s) = surface_of(ctx, this) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let pixels = s.pixels;
    if args[1] == 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    if args[0] != DDCKEY_SRCBLT {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface::GetColorKey: flags {:#x}", args[0]),
            caller: 0,
        });
    }
    let Some(key) = ctx.state.color_keys.get(&pixels).copied() else {
        return Ok(DDERR_NOCOLORKEY);
    };
    for at in [args[1], args[1] + 4] {
        ctx.mem.write_u32(at, key).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn set_surface_desc(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetSurfaceDesc(description, flags): a surface in system memory takes the
    // memory a title hands it, and with it a width and pitch together; it
    // keeps its key; and the rest is refused, in the order Wine's
    // `ddraw_surface7_SetSurfaceDesc` refuses it and as its conformance test
    // `test_set_surface_desc`, through `IDirectDrawSurface3`, finds Windows
    // answering. A new height or format is a wall. LEGO Chess's DirectX plugin
    // hands each texture's surface the pixels the game decodes into (#210).
    const DDSD_CAPS: u32 = 0x1;
    const DDSD_HEIGHT: u32 = 0x2;
    const DDSD_WIDTH: u32 = 0x4;
    const DDSD_PITCH: u32 = 0x8;
    const DDSD_LPSURFACE: u32 = 0x800;
    const DDSD_PIXELFORMAT: u32 = 0x1000;
    const DDERR_INVALIDCAPS: u32 = 0x8876_0064;
    const DDERR_INVALIDSURFACETYPE: u32 = 0x8876_0250;
    let at = args[0];
    if at == 0 || args[1] != 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    let caps = surface_of(ctx, this).map_or(0, |s| s.caps);
    if caps & ddscaps::SYSTEMMEMORY == 0 || caps & ddscaps::PRIMARYSURFACE != 0 {
        return Ok(DDERR_INVALIDSURFACETYPE);
    }
    let read = |o: u32| ctx.mem.read_u32(at + o).map_err(Stop::Fault);
    let (flags, width, pitch, memory) = (read(4)?, read(12)?, read(16)?, read(0x24)?);
    let allowed =
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_LPSURFACE | DDSD_PIXELFORMAT;
    if flags & !allowed != 0 || flags & DDSD_LPSURFACE == 0 || memory == 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    if flags & DDSD_CAPS != 0 && read(0x68)? != 0 {
        return Ok(DDERR_INVALIDCAPS);
    }
    let sized = flags & (DDSD_WIDTH | DDSD_PITCH);
    let pitch_ok = i32::try_from(pitch).is_ok_and(|p| p > 0) && pitch % 4 == 0;
    if sized != 0 && (sized != DDSD_WIDTH | DDSD_PITCH || width == 0 || !pitch_ok) {
        return Ok(DDERR_INVALIDPARAMS);
    }
    if flags & (DDSD_HEIGHT | DDSD_PIXELFORMAT) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirectDrawSurface3::SetSurfaceDesc: flags {flags:#x}"),
            caller: 0,
        });
    }
    let Some(ComState::Surface {
        pixels,
        width: w,
        pitch: p,
        ..
    }) = ctx.state.com_mut(this).map(|o| &mut o.state)
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let before = std::mem::replace(pixels, memory);
    if sized != 0 {
        (*w, *p) = (width, pitch);
    }
    // Its key and its clipper are its own, kept by where its pixels are, and
    // go with them; a key or a clipper kept where they now are was a
    // surface's there before, released (#210).
    // ponytail: kept by address, two live surfaces over one buffer share a key
    // and a clipper; keep them on `ComState::Surface` if a title points two at
    // one.
    let key = ctx.state.color_keys.remove(&before);
    ctx.state.color_keys.remove(&memory);
    if let Some(key) = key {
        ctx.state.color_keys.insert(memory, key);
    }
    let clipped = ctx.state.clipped.remove(&before);
    ctx.state.clipped.remove(&memory);
    if clipped {
        ctx.state.clipped.insert(memory);
    }
    Ok(S_OK)
}

/// The colour a keyed blit leaves out, when the blit asks for one.
fn source_key(ctx: &Ctx<'_>, source: u32, keyed: bool) -> Option<u32> {
    keyed
        .then(|| ctx.state.color_keys.get(&source).copied())
        .flatten()
}

/// A rectangle the guest passed, its edges signed as a `RECT`'s are, or the
/// whole surface when it passed none.
fn rect(ctx: &Ctx<'_>, at: u32, width: u32, height: u32) -> Result<(i64, i64, i64, i64), Stop> {
    if at == 0 {
        return Ok((0, 0, i64::from(width), i64::from(height)));
    }
    let edge = |offset: u32| -> Result<i64, Stop> {
        Ok(i64::from(
            ctx.mem.read_u32(at + offset).map_err(Stop::Fault)? as i32,
        ))
    };
    Ok((edge(0)?, edge(4)?, edge(8)?, edge(12)?))
}

/// Where a surface's pixels are, and how long its rows are.
#[derive(Clone, Copy)]
struct Pixels {
    at: u32,
    pitch: u32,
}

/// Copy one rectangle into another, stretching when their sizes differ, and
/// leaving out whatever of the destination falls outside its surface.
///
/// A stretch takes the nearest source pixel, which is what DirectDraw's own
/// software blitter does for a surface in system memory; a clipped pixel takes
/// the one it would have had unclipped. A source colour key leaves the pixels
/// that match it untouched.
#[allow(clippy::too_many_arguments)]
fn transfer(
    mem: &mut crate::guest::Arena,
    source: Pixels,
    (sx0, sy0, sx1, sy1): (i64, i64, i64, i64),
    destination: Pixels,
    (dx0, dy0, dx1, dy1): (i64, i64, i64, i64),
    (width, height): (u32, u32),
    depth: u32,
    key: Option<u32>,
) -> Result<(), Stop> {
    let bytes = depth.div_ceil(8) as usize;
    let (sw, sh) = (sx1 - sx0, sy1 - sy0);
    let (dw, dh) = (dx1 - dx0, dy1 - dy0);
    // What of the destination is on its surface.
    let (cx0, cy0) = (dx0.max(0), dy0.max(0));
    let (cx1, cy1) = (dx1.min(i64::from(width)), dy1.min(i64::from(height)));
    if sw <= 0 || sh <= 0 || cx1 <= cx0 || cy1 <= cy0 {
        return Ok(());
    }

    let cols = (cx1 - cx0) as usize;
    let rows = (cy1 - cy0) as usize;
    let row_bytes = cols * bytes;
    let src_pitch = source.pitch as usize;
    let dst_pitch = destination.pitch as usize;
    let unstretched = sw == dw && sh == dh;

    let sy_first = (sy0 + (cy0 - dy0) * sh / dh) as u32;
    let sy_last = (sy0 + (cy1 - 1 - dy0) * sh / dh) as u32;
    let sx_first = (sx0 + (cx0 - dx0) * sw / dw) as usize;
    let sx_last = (sx0 + (cx1 - 1 - dx0) * sw / dw) as usize;
    let src_row_bytes = (sx_last - sx_first + 1) * bytes;
    let src_start =
        u64::from(source.at) + u64::from(sy_first) * src_pitch as u64 + (sx_first * bytes) as u64;
    let src_len = (sy_last - sy_first) as usize * src_pitch + src_row_bytes;
    let dst_start =
        u64::from(destination.at) + cy0 as u64 * dst_pitch as u64 + (cx0 as usize * bytes) as u64;
    let dst_len = (rows - 1) * dst_pitch + row_bytes;

    let to_addr = |addr: u64, len: usize, write: bool| -> Result<u32, Stop> {
        u32::try_from(addr).map_err(|_| {
            Stop::Fault(crate::guest::Fault {
                addr: u32::MAX,
                len,
                write,
            })
        })
    };

    let build_sx_table = || -> Vec<usize> {
        (cx0..cx1)
            .map(|x| (sx0 + (x - dx0) * sw / dw) as usize - sx_first)
            .collect()
    };

    let disjoint =
        src_start + src_len as u64 <= dst_start || dst_start + dst_len as u64 <= src_start;

    if disjoint {
        let src_addr = to_addr(src_start, src_len, false)?;
        let dst_addr = to_addr(dst_start, dst_len, true)?;
        let (src, dst) = mem
            .read_and_write((src_addr, src_len), (dst_addr, dst_len))
            .map_err(Stop::Fault)?;

        if unstretched {
            match key {
                None if src_pitch == row_bytes && dst_pitch == row_bytes => {
                    dst.copy_from_slice(src);
                }
                None => {
                    for row in 0..rows {
                        let s = row * src_pitch;
                        let d = row * dst_pitch;
                        dst[d..d + row_bytes].copy_from_slice(&src[s..s + row_bytes]);
                    }
                }
                Some(k) if src_pitch == row_bytes && dst_pitch == row_bytes => {
                    copy_keyed_row(src, dst, bytes, k);
                }
                Some(k) => {
                    for row in 0..rows {
                        let s = row * src_pitch;
                        let d = row * dst_pitch;
                        copy_keyed_row(
                            &src[s..s + row_bytes],
                            &mut dst[d..d + row_bytes],
                            bytes,
                            k,
                        );
                    }
                }
            }
            return Ok(());
        }

        let sx_table = build_sx_table();
        let mut prev: Option<(u32, usize)> = None;
        for y in cy0..cy1 {
            let sy = (sy0 + (y - dy0) * sh / dh) as u32;
            let d = (y - cy0) as usize * dst_pitch;
            if key.is_none() && dst_pitch >= row_bytes {
                if let Some((_, prev_d)) = prev.filter(|&(ps, _)| ps == sy) {
                    dst.copy_within(prev_d..prev_d + row_bytes, d);
                    continue;
                }
            }
            let s = (sy - sy_first) as usize * src_pitch;
            stretch_row(
                &src[s..s + src_row_bytes],
                &mut dst[d..d + row_bytes],
                &sx_table,
                bytes,
                key,
            );
            prev = Some((sy, d));
        }
        return Ok(());
    }

    // Overlapping source and destination: walk scanlines in ascending `y`
    // order through a reusable row buffer, preserving the reference scanline
    // order across all four self-overlap directions.
    let sx_table = (!unstretched).then(build_sx_table);
    let mut src_buf = vec![0u8; src_row_bytes];
    let mut dst_buf = if unstretched && key.is_none() {
        Vec::new()
    } else {
        vec![0u8; row_bytes]
    };
    for y in cy0..cy1 {
        let sy = (sy0 + (y - dy0) * sh / dh) as u32;
        let src_at = to_addr(
            u64::from(source.at) + u64::from(sy) * src_pitch as u64 + (sx_first * bytes) as u64,
            src_row_bytes,
            false,
        )?;
        src_buf.copy_from_slice(mem.slice(src_at, src_row_bytes).map_err(Stop::Fault)?);
        let dst_at = to_addr(
            u64::from(destination.at) + y as u64 * dst_pitch as u64 + (cx0 as usize * bytes) as u64,
            row_bytes,
            true,
        )?;
        if unstretched && key.is_none() {
            mem.write(dst_at, &src_buf).map_err(Stop::Fault)?;
            continue;
        }
        if key.is_some() {
            dst_buf.copy_from_slice(mem.slice(dst_at, row_bytes).map_err(Stop::Fault)?);
        }
        match (&sx_table, key) {
            (None, Some(k)) => copy_keyed_row(&src_buf, &mut dst_buf, bytes, k),
            (Some(table), k) => stretch_row(&src_buf, &mut dst_buf, table, bytes, k),
            (None, None) => unreachable!(),
        }
        mem.write(dst_at, &dst_buf).map_err(Stop::Fault)?;
    }
    Ok(())
}

/// Copy `src` into `dst` pixel by pixel, leaving destination pixels untouched
/// wherever the source pixel matches `key` (in its low `bytes` little-endian
/// bytes).
fn copy_keyed_row(src: &[u8], dst: &mut [u8], bytes: usize, key: u32) {
    match bytes {
        1 => {
            let k = key as u8;
            for (&s, d) in src.iter().zip(dst.iter_mut()) {
                *d = if s == k { *d } else { s };
            }
        }
        2 => {
            let k = key as u16;
            for (&s, d) in src
                .as_chunks::<2>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<2>().0.iter_mut())
            {
                *d = if u16::from_le_bytes(s) == k { *d } else { s };
            }
        }
        3 => {
            let [k0, k1, k2, _] = key.to_le_bytes();
            let k = [k0, k1, k2];
            for (&s, d) in src
                .as_chunks::<3>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<3>().0.iter_mut())
            {
                if s != k {
                    *d = s;
                }
            }
        }
        4 => {
            for (&s, d) in src
                .as_chunks::<4>()
                .0
                .iter()
                .zip(dst.as_chunks_mut::<4>().0.iter_mut())
            {
                *d = if u32::from_le_bytes(s) == key { *d } else { s };
            }
        }
        0 => {}
        _ => {
            let k = key.to_le_bytes();
            for (s, d) in src.chunks_exact(bytes).zip(dst.chunks_exact_mut(bytes)) {
                if !s.iter().zip(k).all(|(p, kb)| *p == kb) {
                    d.copy_from_slice(s);
                }
            }
        }
    }
}

fn stretch_fixed<const N: usize>(
    src: &[u8],
    dst: &mut [u8],
    sx_table: &[usize],
    key: Option<[u8; N]>,
) {
    let src = src.as_chunks::<N>().0;
    let dst = dst.as_chunks_mut::<N>().0;
    match key {
        None => {
            for (&sx, d) in sx_table.iter().zip(dst.iter_mut()) {
                *d = src[sx];
            }
        }
        Some(k) => {
            for (&sx, d) in sx_table.iter().zip(dst.iter_mut()) {
                let p = src[sx];
                if p != k {
                    *d = p;
                }
            }
        }
    }
}

/// Stretch one scanline from `src` into `dst` using precomputed pixel indices
/// in `sx_table`, leaving destination pixels untouched where `key` matches.
fn stretch_row(src: &[u8], dst: &mut [u8], sx_table: &[usize], bytes: usize, key: Option<u32>) {
    let kb = key.map(u32::to_le_bytes);
    match bytes {
        1 => stretch_fixed::<1>(src, dst, sx_table, kb.map(|k| [k[0]])),
        2 => stretch_fixed::<2>(src, dst, sx_table, kb.map(|k| [k[0], k[1]])),
        3 => stretch_fixed::<3>(src, dst, sx_table, kb.map(|k| [k[0], k[1], k[2]])),
        4 => stretch_fixed::<4>(src, dst, sx_table, kb),
        0 => {}
        _ => {
            for (&sx, d) in sx_table.iter().zip(dst.chunks_exact_mut(bytes)) {
                let p = &src[sx * bytes..(sx + 1) * bytes];
                if kb.is_some_and(|k| p.iter().zip(k).all(|(pb, kb)| *pb == kb)) {
                    continue;
                }
                d.copy_from_slice(p);
            }
        }
    }
}

/// How long a `DDSURFACEDESC` is, and DirectX 6's `DDSURFACEDESC2`.
pub(crate) const SURFACE_DESC_BYTES: u32 = 108;
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
    let pitch = pitch_for(*width, *depth);
    write_surface_desc(
        ctx,
        desc,
        (*width, *height, pitch),
        screen_format(*depth),
        0,
    )?;
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

/// Which of a palette's 256 entries a `SetEntries` or `GetEntries` reaches:
/// from the start it asks for to the end of the palette and no further.
fn entries(args: &[u32]) -> (u32, u32) {
    let start = args[1].min(256);
    (start, args[2].min(256 - start))
}

fn set_entries(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetEntries(flags, start, count, entries).
    let Some(ComState::Palette { store }) = ctx.state.com(this).map(|o| o.state.clone()) else {
        return Ok(S_OK);
    };
    let (start, count) = entries(args);
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
    let (start, count) = entries(args);
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

/// The declared machine's video memory, four megabytes, whole and free. A
/// title sizes its surfaces from this, so it must be the same on every run.
const VIDEO_MEMORY: u32 = 4 * 1024 * 1024;

fn available_video_memory(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    for out in [args[1], args[2]] {
        if out != 0 {
            ctx.mem.write_u32(out, VIDEO_MEMORY).map_err(Stop::Fault)?;
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

#[cfg(test)]
mod tests {
    use super::{transfer, Pixels};
    use crate::cpu::Stop;
    use crate::guest::{Arena, Fault, GuestMem};
    use crate::host::headless::Headless;

    /// Reference implementation of `transfer` (pre-#251 algorithm) used as the
    /// bit-exact oracle for every fast path.
    #[allow(clippy::too_many_arguments)]
    fn reference_transfer(
        mem: &mut Arena,
        source: Pixels,
        (sx0, sy0, sx1, sy1): (i64, i64, i64, i64),
        destination: Pixels,
        (dx0, dy0, dx1, dy1): (i64, i64, i64, i64),
        (width, height): (u32, u32),
        depth: u32,
        key: Option<u32>,
    ) -> Result<(), Stop> {
        let bytes = depth.div_ceil(8) as usize;
        let (sw, sh) = (sx1 - sx0, sy1 - sy0);
        let (dw, dh) = (dx1 - dx0, dy1 - dy0);
        let (cx0, cy0) = (dx0.max(0), dy0.max(0));
        let (cx1, cy1) = (dx1.min(i64::from(width)), dy1.min(i64::from(height)));
        if sw <= 0 || sh <= 0 || cx1 <= cx0 || cy1 <= cy0 {
            return Ok(());
        }
        let key = key.map(u32::to_le_bytes);
        let mut row = vec![0u8; (cx1 - cx0) as usize * bytes];
        for y in cy0..cy1 {
            let sy = (sy0 + (y - dy0) * sh / dh) as u32;
            let line = mem
                .slice(source.at + sy * source.pitch, sx1 as usize * bytes)
                .map_err(Stop::Fault)?
                .to_vec();
            let at = destination.at + y as u32 * destination.pitch + cx0 as u32 * bytes as u32;
            if key.is_some() {
                let len = row.len();
                row.copy_from_slice(mem.slice(at, len).map_err(Stop::Fault)?);
            }
            for x in cx0..cx1 {
                let sx = (sx0 + (x - dx0) * sw / dw) as usize * bytes;
                let pixel = &line[sx..sx + bytes];
                if key.is_some_and(|k| pixel.iter().zip(k).all(|(p, k)| p == &k)) {
                    continue;
                }
                let out = (x - cx0) as usize * bytes;
                row[out..out + bytes].copy_from_slice(pixel);
            }
            mem.write(at, &row).map_err(Stop::Fault)?;
        }
        Ok(())
    }

    /// Deterministic non-repeating pixel data for oracle comparisons.
    fn pattern(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| seed.wrapping_add((i.wrapping_mul(37) ^ (i >> 3)) as u8))
            .collect()
    }

    /// Run `transfer` and `reference_transfer` on identical arenas and assert
    /// every byte in `0..arena_len` matches bit for bit.
    #[allow(clippy::too_many_arguments)]
    fn assert_matches_reference(
        arena_len: usize,
        init: &[(u32, &[u8])],
        source: Pixels,
        from: (i64, i64, i64, i64),
        destination: Pixels,
        to: (i64, i64, i64, i64),
        size: (u32, u32),
        depth: u32,
        key: Option<u32>,
        label: &str,
    ) {
        let mut actual = Arena::new(&mut Headless::new(), arena_len).expect("an arena");
        let mut expected = Arena::new(&mut Headless::new(), arena_len).expect("an arena");
        for &(addr, bytes) in init {
            actual.write(addr, bytes).expect("init fits");
            expected.write(addr, bytes).expect("init fits");
        }
        transfer(&mut actual, source, from, destination, to, size, depth, key)
            .unwrap_or_else(|e| panic!("{label}: transfer failed: {e:?}"));
        reference_transfer(
            &mut expected,
            source,
            from,
            destination,
            to,
            size,
            depth,
            key,
        )
        .unwrap_or_else(|e| panic!("{label}: reference failed: {e:?}"));
        assert_eq!(
            actual.slice(0, arena_len).expect("readable"),
            expected.slice(0, arena_len).expect("readable"),
            "{label}"
        );
    }

    #[test]
    fn a_blit_past_the_top_left_of_its_destination_draws_what_is_still_on_it() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).expect("an arena");
        // A two by two source of one byte a pixel, and a four by four
        // destination of zeros.
        mem.write(0x100, &[1, 2, 3, 4]).expect("fits");
        let (source, destination) = (
            Pixels {
                at: 0x100,
                pitch: 2,
            },
            Pixels {
                at: 0x200,
                pitch: 4,
            },
        );
        // One pixel up and to the left of the corner: only the source's
        // bottom right pixel lands, at the destination's top left.
        transfer(
            &mut mem,
            source,
            (0, 0, 2, 2),
            destination,
            (-1, -1, 1, 1),
            (4, 4),
            8,
            None,
        )
        .expect("in memory");
        assert_eq!(
            mem.slice(0x200, 16).expect("in memory"),
            [4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        // Stretched to four by four from two pixels past the bottom right:
        // what is on the surface is what those pixels would have been.
        transfer(
            &mut mem,
            source,
            (0, 0, 2, 2),
            destination,
            (2, 2, 6, 6),
            (4, 4),
            8,
            None,
        )
        .expect("in memory");
        assert_eq!(
            &mem.slice(0x200, 16).expect("in memory")[10..],
            [1, 1, 0, 0, 1, 1]
        );
    }

    #[test]
    fn unkeyed_one_to_one_blits_match_reference_across_depths_pitches_and_clipping() {
        for depth in [8u32, 16, 24, 32] {
            let bytes = depth.div_ceil(8);
            // Full-width contiguous surface copy (fast single memcpy path).
            let (w, h) = (37u32, 11u32);
            let pitch = w * bytes;
            let src_data = pattern((pitch * h) as usize, 0x11);
            let dst_data = pattern((pitch * h) as usize, 0x99);
            assert_matches_reference(
                1 << 16,
                &[(0x1000, &src_data), (0x8000, &dst_data)],
                Pixels { at: 0x1000, pitch },
                (0, 0, i64::from(w), i64::from(h)),
                Pixels { at: 0x8000, pitch },
                (0, 0, i64::from(w), i64::from(h)),
                (w, h),
                depth,
                None,
                &format!("contiguous 1:1 depth={depth}"),
            );

            // Padded pitches, sub-rectangle source, and clipping on every edge.
            let src_pitch = w * bytes + 12;
            let dst_pitch = w * bytes + 20;
            let src_padded = pattern((src_pitch * h) as usize, 0x23);
            let dst_padded = pattern((dst_pitch * h) as usize, 0x77);
            let cases = [
                ("subrect", (3, 2, 25, 9), (5, 1, 27, 8)),
                ("clip-top-left", (0, 0, 20, 8), (-4, -3, 16, 5)),
                ("clip-bottom-right", (2, 1, 22, 9), (25, 6, 45, 14)),
                ("clip-all-edges", (0, 0, 37, 11), (-2, -2, 35, 9)),
                ("single-row", (4, 5, 30, 6), (1, 10, 27, 11)),
                ("clipped-out", (0, 0, 10, 5), (40, 0, 50, 5)),
            ];
            for (name, from, to) in cases {
                assert_matches_reference(
                    1 << 16,
                    &[(0x1000, &src_padded), (0x8000, &dst_padded)],
                    Pixels {
                        at: 0x1000,
                        pitch: src_pitch,
                    },
                    from,
                    Pixels {
                        at: 0x8000,
                        pitch: dst_pitch,
                    },
                    to,
                    (w, h),
                    depth,
                    None,
                    &format!("padded 1:1 {name} depth={depth}"),
                );
            }
        }
    }

    #[test]
    fn keyed_one_to_one_blits_match_reference_for_all_pixel_widths() {
        // Upper bits set to ensure sub-32-bit depths match only their low bytes.
        let key = 0xDEAD_BEEFu32;
        let key_bytes = key.to_le_bytes();
        for depth in [8u32, 16, 24, 32] {
            let bytes = depth.div_ceil(8) as usize;
            let (w, h) = (39u32, 9u32);
            let tight_pitch = w * bytes as u32;
            let padded_pitch = tight_pitch + 16;

            for (label, src_pitch, dst_pitch, from, to) in [
                (
                    "contiguous",
                    tight_pitch,
                    tight_pitch,
                    (0, 0, i64::from(w), i64::from(h)),
                    (0, 0, i64::from(w), i64::from(h)),
                ),
                (
                    "padded-subrect",
                    padded_pitch,
                    padded_pitch + 8,
                    (2, 1, 37, 8),
                    (1, 1, 36, 8),
                ),
                (
                    "clipped",
                    padded_pitch,
                    padded_pitch,
                    (0, 0, 30, 7),
                    (-5, -2, 25, 5),
                ),
            ] {
                let mut src_data = pattern((src_pitch * h) as usize, 0x31);
                // Sprinkle matching key pixels in every row: start, middle,
                // adjacent pairs, and tail.
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        if x % 3 == 0 || x == 1 || x + 1 == w as usize {
                            let off = y * src_pitch as usize + x * bytes;
                            src_data[off..off + bytes].copy_from_slice(&key_bytes[..bytes]);
                        }
                    }
                }
                let dst_data = pattern((dst_pitch * h) as usize, 0xC5);
                assert_matches_reference(
                    1 << 16,
                    &[(0x1000, &src_data), (0x8000, &dst_data)],
                    Pixels {
                        at: 0x1000,
                        pitch: src_pitch,
                    },
                    from,
                    Pixels {
                        at: 0x8000,
                        pitch: dst_pitch,
                    },
                    to,
                    (w, h),
                    depth,
                    Some(key),
                    &format!("keyed 1:1 {label} depth={depth}"),
                );
            }
        }
    }

    #[test]
    fn self_overlapping_blits_preserve_reference_output_in_all_four_directions() {
        let key = 0x0012_3456u32;
        let key_bytes = key.to_le_bytes();
        for depth in [8u32, 16, 24, 32] {
            let bytes = depth.div_ceil(8) as usize;
            let (w, h) = (16u32, 12u32);
            let pitch = w * bytes as u32 + 4;
            let mut surface = pattern((pitch * h) as usize, 0x42);
            for y in 0..h as usize {
                let off = y * pitch as usize + (y % w as usize) * bytes;
                surface[off..off + bytes].copy_from_slice(&key_bytes[..bytes]);
            }
            let surf = Pixels { at: 0x2000, pitch };
            let directions = [
                ("left", (3, 2, 13, 9), (1, 2, 11, 9)),
                ("right", (1, 2, 11, 9), (3, 2, 13, 9)),
                ("up", (2, 3, 12, 10), (2, 1, 12, 8)),
                ("down", (2, 1, 12, 8), (2, 3, 12, 10)),
                ("up-left", (3, 3, 13, 10), (1, 1, 11, 8)),
                ("down-right", (1, 1, 11, 8), (3, 3, 13, 10)),
                ("same-spot", (2, 2, 10, 8), (2, 2, 10, 8)),
                ("stretched-overlap", (1, 1, 7, 5), (2, 2, 14, 10)),
            ];
            for (name, from, to) in directions {
                for k in [None, Some(key)] {
                    assert_matches_reference(
                        1 << 16,
                        &[(0x2000, &surface)],
                        surf,
                        from,
                        surf,
                        to,
                        (w, h),
                        depth,
                        k,
                        &format!("self-overlap {name} depth={depth} key={k:?}"),
                    );
                }
            }
            // Distinct base pointers whose byte spans still overlap partially.
            assert_matches_reference(
                1 << 16,
                &[(0x2000, &surface)],
                Pixels { at: 0x2000, pitch },
                (0, 0, 12, 8),
                Pixels {
                    at: 0x2000 + pitch * 2,
                    pitch,
                },
                (0, 0, 12, 8),
                (w, h),
                depth,
                None,
                &format!("partial base overlap depth={depth}"),
            );
        }
    }

    #[test]
    fn stretched_blits_match_reference_exact_nearest_neighbour() {
        let key = 0x00A1_B2C3u32;
        let key_bytes = key.to_le_bytes();
        for depth in [8u32, 16, 24, 32] {
            let bytes = depth.div_ceil(8) as usize;
            let (sw, sh) = (17u32, 11u32);
            let (dw, dh) = (35u32, 23u32);
            let src_pitch = sw * bytes as u32 + 8;
            let dst_pitch = dw * bytes as u32 + 12;
            let mut src_data = pattern((src_pitch * sh) as usize, 0x55);
            for y in 0..sh as usize {
                for x in (0..sw as usize).step_by(4) {
                    let off = y * src_pitch as usize + x * bytes;
                    src_data[off..off + bytes].copy_from_slice(&key_bytes[..bytes]);
                }
            }
            let dst_data = pattern((dst_pitch * dh) as usize, 0xAA);
            let cases = [
                ("upscale", (0, 0, 11, 7), (0, 0, 29, 19)),
                ("downscale", (1, 1, 16, 10), (2, 1, 9, 6)),
                ("x-only", (0, 0, 10, 6), (3, 2, 27, 8)),
                ("y-only", (2, 0, 12, 5), (4, 1, 14, 16)),
                ("clipped-upscale", (1, 1, 13, 9), (-5, -3, 38, 25)),
                ("clipped-downscale", (0, 0, 17, 11), (30, 18, 41, 26)),
            ];
            for (name, from, to) in cases {
                for k in [None, Some(key)] {
                    assert_matches_reference(
                        1 << 16,
                        &[(0x1000, &src_data), (0x8000, &dst_data)],
                        Pixels {
                            at: 0x1000,
                            pitch: src_pitch,
                        },
                        from,
                        Pixels {
                            at: 0x8000,
                            pitch: dst_pitch,
                        },
                        to,
                        (dw, dh),
                        depth,
                        k,
                        &format!("stretch {name} depth={depth} key={k:?}"),
                    );
                }
            }
        }
    }

    #[test]
    fn transfer_faults_out_of_bounds_and_notifies_code_pages_and_watch() {
        // Source out of bounds -> read fault.
        let mut mem = Arena::new(&mut Headless::new(), 0x4000)
            .expect("an arena")
            .with_floor(0x1000);
        let err = transfer(
            &mut mem,
            Pixels {
                at: 0x3FF0,
                pitch: 32,
            },
            (0, 0, 16, 2),
            Pixels {
                at: 0x2000,
                pitch: 32,
            },
            (0, 0, 16, 2),
            (16, 2),
            16,
            None,
        )
        .expect_err("source past end faults");
        assert!(matches!(err, Stop::Fault(Fault { write: false, .. })));

        // Destination out of bounds -> write fault.
        let err = transfer(
            &mut mem,
            Pixels {
                at: 0x2000,
                pitch: 32,
            },
            (0, 0, 16, 2),
            Pixels {
                at: 0x3FF0,
                pitch: 32,
            },
            (0, 0, 16, 2),
            (16, 2),
            16,
            None,
        )
        .expect_err("destination past end faults");
        assert!(matches!(err, Stop::Fault(Fault { write: true, .. })));

        // Under floor -> fault.
        let err = transfer(
            &mut mem,
            Pixels {
                at: 0x0100,
                pitch: 16,
            },
            (0, 0, 4, 2),
            Pixels {
                at: 0x2000,
                pitch: 16,
            },
            (0, 0, 4, 2),
            (4, 2),
            16,
            None,
        )
        .expect_err("source under floor faults");
        assert!(matches!(err, Stop::Fault(Fault { write: false, .. })));

        // Code-page invalidation and watch cancellation across every path:
        // disjoint 1:1 unkeyed, disjoint 1:1 keyed, disjoint stretched, and
        // overlapping.
        let paths = [
            ("1:1-unkeyed", 0x1000u32, (0, 0, 8, 4), (0, 0, 8, 4), None),
            ("1:1-keyed", 0x1000, (0, 0, 8, 4), (0, 0, 8, 4), Some(0xFF)),
            ("stretched", 0x1000, (0, 0, 4, 2), (0, 0, 8, 4), None),
            ("overlapping", 0x2000, (0, 0, 6, 3), (1, 1, 7, 4), None),
        ];
        for (name, src_at, from, to, key) in paths {
            let mut mem = Arena::new(&mut Headless::new(), 0x4000).expect("an arena");
            mem.write(src_at, &pattern(256, 0x2A)).expect("fits");
            mem.mark_code(0x2000);
            mem.watch();
            transfer(
                &mut mem,
                Pixels {
                    at: src_at,
                    pitch: 16,
                },
                from,
                Pixels {
                    at: 0x2000,
                    pitch: 16,
                },
                to,
                (8, 4),
                16,
                key,
            )
            .expect("in bounds");
            assert!(mem.code_written, "{name}: must notice code page write");
            assert!(
                mem.written_code.contains(&2),
                "{name}: page 2 (0x2000) recorded"
            );
            assert!(!mem.unchanged(), "{name}: watch must observe write");
        }

        // Completely clipped-out blit touches nothing: neither code nor watch.
        let mut mem = Arena::new(&mut Headless::new(), 0x4000).expect("an arena");
        mem.mark_code(0x2000);
        mem.watch();
        transfer(
            &mut mem,
            Pixels {
                at: 0x1000,
                pitch: 16,
            },
            (0, 0, 4, 4),
            Pixels {
                at: 0x2000,
                pitch: 16,
            },
            (10, 10, 14, 14),
            (8, 8),
            16,
            None,
        )
        .expect("clipped out");
        assert!(!mem.code_written);
        assert!(mem.unchanged());
    }

    /// A palette's entries are reached from the start asked for to its end,
    /// and never past it, however many were asked for.
    #[test]
    fn a_palette_is_not_written_past_its_last_entry() {
        assert_eq!(super::entries(&[0, 250, 10, 0]), (250, 6));
        assert_eq!(super::entries(&[0, 300, 5, 0]), (256, 0));
        assert_eq!(super::entries(&[0, 0, 256, 0]), (0, 256));
    }

    #[test]
    fn flip_disjoint_zero_alloc_overlap_fallback_and_dc_out() {
        use super::{dc_out, flip, screen_format, DIRECT_DRAW_SURFACE};
        use crate::com::S_OK;
        use crate::win32::tests::with_test_ctx;
        use crate::win32::{ComState, State};

        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let (width, height, pitch) = (16u32, 8u32, 32u32);
            let len = (pitch * height) as usize;
            let format = screen_format(16);

            // Disjoint front (0x2000) and back (0x4000) surfaces -> zero-alloc read_and_write path.
            let back = State::make_com(
                ctx,
                &DIRECT_DRAW_SURFACE,
                ComState::Surface {
                    pixels: 0x4000,
                    width,
                    height,
                    pitch,
                    format,
                    primary: false,
                    caps: 0,
                    back: 0,
                    palette: 0,
                },
            )
            .unwrap();
            let front = State::make_com(
                ctx,
                &DIRECT_DRAW_SURFACE,
                ComState::Surface {
                    pixels: 0x2000,
                    width,
                    height,
                    pitch,
                    format,
                    primary: true,
                    caps: 0,
                    back,
                    palette: 0,
                },
            )
            .unwrap();

            let expected = pattern(len, 0x3b);
            ctx.mem.write(0x4000, &expected).unwrap();
            ctx.mem.mark_code(0x2000);
            assert!(!dc_out(ctx, front));
            assert_eq!(flip(ctx, front, &[0, 0]).unwrap(), S_OK);
            assert_eq!(ctx.mem.slice(0x2000, len).unwrap(), expected.as_slice());
            assert!(ctx.mem.code_written);
            assert!(ctx.mem.written_code.contains(&2));
            assert!(ctx.state.present.is_some());

            // Overlapping back (0x2040) and front (0x2000) -> fallback copy preserves exact bytes.
            if let Some(ComState::Surface { pixels, .. }) =
                ctx.state.com_mut(back).map(|o| &mut o.state)
            {
                *pixels = 0x2040;
            }
            let overlap_src = ctx.mem.slice(0x2040, len).unwrap().to_vec();
            assert_eq!(flip(ctx, front, &[0, 0]).unwrap(), S_OK);
            assert_eq!(ctx.mem.slice(0x2000, len).unwrap(), overlap_src.as_slice());

            // Active surface DC makes dc_out true and Flip stop at a wall.
            ctx.state.surface_dcs.insert(0x40000, back);
            assert!(dc_out(ctx, back));
            assert!(!dc_out(ctx, front));
            assert!(matches!(
                flip(ctx, front, &[0, 0]),
                Err(Stop::NotWritten { .. })
            ));
        });
    }
}
