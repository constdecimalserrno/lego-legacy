//! COM's own library: what a title calls to make an object by its class.
//!
//! Nothing here is a registry of installed components. A class this machine
//! makes is one a roster title asked for, written in Rust like everything
//! else at the boundary (ADR-0002); any other class is a wall that names it.

use super::{Convention, Ctx, Shim};
use crate::com::{iid_text, read_iid, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "ole32",
        name: "CoInitialize",
        args: 1,
        convention: Convention::Stdcall,
        run: initialize,
    },
    Shim {
        dll: "ole32",
        name: "CoUninitialize",
        args: 0,
        convention: Convention::Stdcall,
        run: uninitialize,
    },
    Shim {
        dll: "ole32",
        name: "CoTaskMemFree",
        args: 1,
        convention: Convention::Stdcall,
        run: task_mem_free,
    },
    Shim {
        dll: "ole32",
        name: "CoCreateInstance",
        args: 5,
        convention: Convention::Stdcall,
        run: create_instance,
    },
];

fn initialize(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // S_OK: the library is up for this thread, and this is the first time it
    // was asked. Nothing is kept, because the classes this machine makes need
    // no apartment.
    Ok(S_OK)
}

fn uninitialize(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Nothing was kept when the library came up, so there is nothing to put
    // down. The objects a title still holds are its own to release.
    Ok(0)
}

fn task_mem_free(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // What COM allocated for a title came off the guest heap, so it goes back
    // there. A null pointer is not an error, which is what COM says too.
    if args[0] != 0 {
        ctx.state.heap.free(args[0]);
    }
    Ok(0)
}

fn create_instance(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CoCreateInstance(class, outer, context, interface, out).
    let class = iid_text(&read_iid(ctx, args[0])?);
    let interface = iid_text(&read_iid(ctx, args[3])?);
    // CLSID_AMMultiMediaStream, as IAMMultiMediaStream: the movie player (#9).
    if class == "49c47ce5-9ba4-11d0-8212-00c04fc32c45"
        && interface == "bebe595c-9a6f-11d0-8fde-00c04fd9189d"
        && args[1] == 0
    {
        let object = super::amstream::create(ctx)?;
        ctx.mem.write_u32(args[4], object).map_err(Stop::Fault)?;
        return Ok(S_OK);
    }
    Err(Stop::NotWritten {
        what: format!("ole32.dll!CoCreateInstance: class {class} as {interface} is not a class this machine makes"),
        caller: 0,
    })
}
