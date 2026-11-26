//! GDI32, as far as a title reaches into it.
//!
//! These titles draw with DirectDraw and touch GDI only for the odd region or
//! stock object on the way to a window. Nothing here draws anything.

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;

pub const SHIMS: &[Shim] = &[
    stdcall("GDI32", "CreateRectRgn", 4, region),
    stdcall("GDI32", "CreateSolidBrush", 1, create_solid_brush),
    stdcall("GDI32", "DeleteObject", 1, delete_object),
    stdcall("GDI32", "GetStockObject", 1, stock_object),
    stdcall(
        "GDI32",
        "GetSystemPaletteEntries",
        4,
        system_palette_entries,
    ),
    stdcall("GDI32", "GetDeviceCaps", 2, device_caps),
    stdcall("GDI32", "GetRegionData", 3, region_data),
];

/// A region handle. Regions are not drawn with here, so one is as good as
/// another, and a title only ever hands it back.
const REGION: u32 = 0x0002_0010;

fn region(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(REGION)
}

/// A solid brush handle. GDI does not paint the DirectDraw screen here, so a
/// distinct non-zero pseudo-handle per colour is all a caller passes to
/// `FillRect` and `DeleteObject`.
fn create_solid_brush(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(0x0100_0000 | (args[0] & 0x00ff_ffff))
}

fn delete_object(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(args[0] != 0))
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

#[cfg(test)]
mod tests {
    use super::{create_solid_brush, delete_object};
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
}
