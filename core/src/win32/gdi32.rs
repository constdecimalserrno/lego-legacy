//! GDI32, as far as a title reaches into it.
//!
//! These titles draw with DirectDraw and touch GDI only for the odd region or
//! stock object on the way to a window. Nothing here draws anything.

use super::{Convention, Ctx, Shim};
use crate::cpu::Stop;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "GDI32",
        name: "CreateRectRgn",
        args: 4,
        convention: Convention::Stdcall,
        run: region,
    },
    Shim {
        dll: "GDI32",
        name: "CreateRectRgnIndirect",
        args: 1,
        convention: Convention::Stdcall,
        run: region,
    },
    Shim {
        dll: "GDI32",
        name: "CombineRgn",
        args: 4,
        convention: Convention::Stdcall,
        run: combine,
    },
    Shim {
        dll: "GDI32",
        name: "GetRgnBox",
        args: 2,
        convention: Convention::Stdcall,
        run: nothing,
    },
    Shim {
        dll: "GDI32",
        name: "DeleteObject",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "GDI32",
        name: "GetStockObject",
        args: 1,
        convention: Convention::Stdcall,
        run: stock_object,
    },
    Shim {
        dll: "GDI32",
        name: "GetSystemPaletteEntries",
        args: 4,
        convention: Convention::Stdcall,
        run: system_palette_entries,
    },
    Shim {
        dll: "GDI32",
        name: "GetDeviceCaps",
        args: 2,
        convention: Convention::Stdcall,
        run: device_caps,
    },
    Shim {
        dll: "GDI32",
        name: "GetRegionData",
        args: 3,
        convention: Convention::Stdcall,
        run: region_data,
    },
];

/// A region handle. Regions are not drawn with here, so one is as good as
/// another, and a title only ever hands it back.
const REGION: u32 = 0x0002_0010;

fn region(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(REGION)
}

fn combine(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // SIMPLEREGION.
    Ok(2)
}

fn nothing(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

fn succeed(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn stock_object(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Distinct per kind, so a title comparing two of them sees two.
    Ok(0x0002_0100 + args[0])
}

/// The colours the declared screen is showing, which at sixteen bits a pixel
/// is none: there is no palette to read back (#57).
fn system_palette_entries(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // GetSystemPaletteEntries(dc, first, count, entries). Nothing written, and
    // none copied: a caller asking a non-palette display what its palette is
    // gets the same answer from Windows.
    Ok(0)
}

fn device_caps(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (width, height, depth) = ctx.state.display_mode;
    Ok(match args[1] {
        // HORZRES, VERTRES, BITSPIXEL, PLANES.
        8 => width,
        10 => height,
        12 => depth,
        14 => 1,
        _ => 0,
    })
}

fn region_data(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // RGNDATA: a header saying how many rectangles follow, and then one. The
    // clipper this feeds covers the whole screen, because the declared machine
    // has one window and nothing overlaps it.
    // GetRegionData(region, room, data).
    let (room, at) = (args[1], args[2]);
    let (width, height, _) = ctx.state.display_mode;
    let needed = 32 + 16;
    if at == 0 || room < needed {
        return Ok(needed);
    }
    // dwSize, iType RDH_RECTANGLES, nCount, nRgnSize, then the bounding box.
    for (offset, value) in [
        (0, 32),
        (4, 1),
        (8, 1),
        (12, 16),
        (16, 0),
        (20, 0),
        (24, width),
        (28, height),
        (32, 0),
        (36, 0),
        (40, width),
        (44, height),
    ] {
        crate::guest::GuestMem::write_u32(ctx.mem, at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(needed)
}
