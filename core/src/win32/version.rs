//! What a file says its version is (#60).
//!
//! Alpha Team's sound server looks at the version of the libraries it sits on
//! before it will use them. The declared machine answers for the whole of
//! itself: every file it is asked about is part of DirectX 7, which is what
//! this title's own disc installs.

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("VERSION", "GetFileVersionInfoA", 4, info),
    stdcall("VERSION", "VerQueryValueA", 4, query),
];

/// How big the block this machine hands out is: one `VS_FIXEDFILEINFO` and
/// nothing else, because that is the only part `VerQueryValueA` is asked for.
const BLOCK: u32 = 52;

/// DirectX 7, as its own files number themselves: 4.07.00.0700. A declared
/// value — the declared machine has DirectX 7 because that is what this
/// title's disc carries and what its renderer asks for (#57).
const VERSION_MS: u32 = 0x0004_0007;
const VERSION_LS: u32 = 0x0000_02bc;

fn info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetFileVersionInfoA(name, handle, length, data).
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(0);
    }
    if args[2] < BLOCK {
        // ERROR_INSUFFICIENT_BUFFER.
        ctx.state.last_error = 122;
        return Ok(0);
    }
    let at = args[3];
    ctx.mem
        .write(at, &[0u8; BLOCK as usize])
        .map_err(Stop::Fault)?;
    // VS_FIXEDFILEINFO: its own signature, the structure's version, then the
    // file's and the product's.
    ctx.mem.write_u32(at, 0xfeef_04bd).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(at + 4, 0x0001_0000)
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(at + 8, VERSION_MS).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(at + 12, VERSION_LS)
        .map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(at + 16, VERSION_MS)
        .map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(at + 20, VERSION_LS)
        .map_err(Stop::Fault)?;
    Ok(1)
}

fn query(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // VerQueryValueA(block, what, out, out length). The root is the only part
    // this machine keeps, so anything else is not there.
    let what = ctx.string(args[1]).map_err(Stop::Fault)?;
    if what != "\\" {
        ctx.log.line(&format!("version has no {what}"));
        return Ok(0);
    }
    ctx.mem.write_u32(args[2], args[0]).map_err(Stop::Fault)?;
    if args[3] != 0 {
        ctx.mem.write_u32(args[3], BLOCK).map_err(Stop::Fault)?;
    }
    Ok(1)
}
