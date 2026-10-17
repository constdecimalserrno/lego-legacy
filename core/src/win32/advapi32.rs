//! The registry.
//!
//! A declared value in the shape of a tree: the keys a title reads are the
//! ones its installer would have written, and what the guest writes back is
//! kept for the rest of the run. Nothing here reaches the host's own registry,
//! because there isn't one, and a run must not depend on the machine it was
//! taken on.

use super::{stdcall, Ctx, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("ADVAPI32", "RegCreateKeyA", 3, reg_create_key),
    stdcall("ADVAPI32", "RegCreateKeyExA", 9, reg_create_key_ex),
    stdcall("ADVAPI32", "RegOpenKeyExA", 5, reg_open_key_ex),
    stdcall("ADVAPI32", "RegQueryValueExA", 6, reg_query_value_ex),
    stdcall("ADVAPI32", "RegSetValueExA", 6, reg_set_value_ex),
    stdcall("ADVAPI32", "RegCloseKey", 1, reg_close_key),
];

/// What the registry answers with.
const ERROR_SUCCESS: u32 = 0;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_MORE_DATA: u32 = 234;

/// The value kinds a title on this roster stores.
pub const REG_SZ: u32 = 1;

/// Where a key sits, spelled the way the guest asked for it.
fn key_path(root: u32, name: &str) -> String {
    let root = match root {
        0x8000_0000 => "HKCR",
        0x8000_0001 => "HKCU",
        0x8000_0002 => "HKLM",
        0x8000_0003 => "HKU",
        _ => "HK?",
    };
    format!("{root}\\{}", name.trim_matches('\\'))
}

fn reg_create_key(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let name = ctx.string(args[1]).map_err(Stop::Fault)?;
    let path = key_path(args[0], &name);
    let handle = ctx.state.open_key(&path);
    ctx.mem.write_u32(args[2], handle).map_err(Stop::Fault)?;
    Ok(ERROR_SUCCESS)
}

fn reg_create_key_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let name = ctx.string(args[1]).map_err(Stop::Fault)?;
    let path = key_path(args[0], &name);
    let existed = ctx.state.registry_has(&path);
    let handle = ctx.state.open_key(&path);
    ctx.mem.write_u32(args[7], handle).map_err(Stop::Fault)?;
    if args[8] != 0 {
        // REG_CREATED_NEW_KEY is 1 and REG_OPENED_EXISTING_KEY is 2.
        ctx.mem
            .write_u32(args[8], if existed { 2 } else { 1 })
            .map_err(Stop::Fault)?;
    }
    Ok(ERROR_SUCCESS)
}

fn reg_open_key_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let name = ctx.string(args[1]).map_err(Stop::Fault)?;
    let path = key_path(args[0], &name);
    if !ctx.state.registry_has(&path) {
        return Ok(ERROR_FILE_NOT_FOUND);
    }
    let handle = ctx.state.open_key(&path);
    ctx.mem.write_u32(args[4], handle).map_err(Stop::Fault)?;
    Ok(ERROR_SUCCESS)
}

fn reg_query_value_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (key, name_at, kind_at, data_at, size_at) = (args[0], args[1], args[3], args[4], args[5]);
    let name = ctx.string(name_at).map_err(Stop::Fault)?;
    let Some(path) = ctx.state.key_of(key) else {
        return Ok(ERROR_FILE_NOT_FOUND);
    };
    let Some((kind, bytes)) = ctx.state.registry_value(&path, &name) else {
        // Recorded the way a missing file is: what a title reads is how the
        // declared machine learns what its installer would have written, and
        // a title reading a value nobody wrote carries on with whatever was
        // in the buffer it asked us to fill.
        ctx.log
            .line(&format!("registry value missing {path}\\{name}"));
        return Ok(ERROR_FILE_NOT_FOUND);
    };
    let needed = u32::try_from(bytes.len()).unwrap_or(0);
    if kind_at != 0 {
        ctx.mem.write_u32(kind_at, kind).map_err(Stop::Fault)?;
    }
    let room = if size_at == 0 {
        0
    } else {
        ctx.mem.read_u32(size_at).map_err(Stop::Fault)?
    };
    let fits = room >= needed;
    if data_at != 0 && fits {
        ctx.mem.write(data_at, &bytes).map_err(Stop::Fault)?;
    }
    if size_at != 0 {
        ctx.mem.write_u32(size_at, needed).map_err(Stop::Fault)?;
    }
    // No buffer at all asks how big the value is; one too small for it is
    // told so, with the size it needs written, as Windows does.
    Ok(if data_at != 0 && !fits {
        ERROR_MORE_DATA
    } else {
        ERROR_SUCCESS
    })
}

fn reg_set_value_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (key, name_at, kind, data_at, len) = (args[0], args[1], args[3], args[4], args[5]);
    let name = ctx.string(name_at).map_err(Stop::Fault)?;
    let Some(path) = ctx.state.key_of(key) else {
        return Ok(ERROR_FILE_NOT_FOUND);
    };
    let bytes = if len == 0 {
        Vec::new()
    } else {
        ctx.mem
            .slice(data_at, len as usize)
            .map_err(Stop::Fault)?
            .to_vec()
    };
    ctx.state.set_registry_value(&path, &name, kind, bytes);
    Ok(ERROR_SUCCESS)
}

fn reg_close_key(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.state.close(args[0]);
    Ok(ERROR_SUCCESS)
}
