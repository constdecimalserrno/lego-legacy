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

/// Which classes the title's own disc carries a server for, and which file.
///
/// A declared value: the machine does not go looking through a registry it
/// does not have. Alpha Team's sound system asks for A3D by this identifier
/// and `a3dapi.dll`, in its own cabinet, is what answers it (#60).
/// Empty, and deliberately: see the note on A3D below (#60).
const SERVERS: &[(&str, &str)] = &[];

/// Make a class through the server the title ships for it.
///
/// COM's own two steps, done by the guest's own code: the server hands out a
/// class factory, and the factory makes the object. The machine writes a short
/// program that does both and answers with what the second one answered (#60).
fn from_the_title_s_own_server(
    ctx: &mut Ctx<'_>,
    class: &str,
    args: &[u32],
) -> Result<Option<u32>, Stop> {
    let Some((_, file)) = SERVERS.iter().find(|(c, _)| *c == class) else {
        return Ok(None);
    };
    let under = ctx.title.install_dir;
    // Whether this is the first time: a server attaches once, before anything
    // calls into it.
    let fresh = !ctx.modules.holds(file);
    let loaded = ctx
        .modules
        .load(file, under, ctx.tree, ctx.mem)
        .map_err(|e| Stop::NotWritten {
            what: format!("ole32.dll!CoCreateInstance: {file} will not load: {e}"),
            caller: 0,
        })?;
    if loaded.is_none() {
        ctx.log
            .line(&format!("com: {file} is not on the declared machine"));
        // REGDB_E_CLASSNOTREG, which is what a machine without it says.
        return Ok(Some(0x8004_0154));
    }
    // Its own imports, now that it is mapped.
    let imports = ctx
        .tree
        .read(&format!("{under}\\{file}"))
        .and_then(|bytes| {
            crate::pe::imports_of(&bytes, ctx.modules.base_of_name(file).unwrap_or(0)).ok()
        })
        .unwrap_or_default();
    ctx.modules
        .bind(&imports, ctx.boundary, ctx.mem)
        .map_err(Stop::Fault)?;
    let Some(get_class_object) = ctx.modules.resolve(file, "DllGetClassObject") else {
        return Err(Stop::NotWritten {
            what: format!("ole32.dll!CoCreateInstance: {file} exports no DllGetClassObject"),
            caller: 0,
        });
    };
    let attach = if fresh {
        ctx.modules
            .entry_of(file)
            .filter(|e| *e != 0)
            .map(|entry| (entry, ctx.modules.base_of_name(file).unwrap_or(0)))
    } else {
        None
    };
    ctx.log.line(&format!(
        "com: {class} from {file}, by its own class factory"
    ));
    let program = class_factory_program(ctx, get_class_object, attach, args)?;
    ctx.call_guest = Some((program, Vec::new()));
    ctx.answer_is_the_call_s = true;
    Ok(Some(S_OK))
}

/// `IID_IClassFactory`, which is how COM asks a server for a maker of things.
const CLASS_FACTORY: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

/// The guest program that asks a server for a class factory and has the
/// factory make the object.
///
/// Two calls whose second needs the first's answer, which is more than a shim
/// can park, so it is written as code the guest runs — the same thing the
/// module loader does to run a DllMain (#55).
fn class_factory_program(
    ctx: &mut Ctx<'_>,
    get_class_object: u32,
    attach: Option<(u32, u32)>,
    args: &[u32],
) -> Result<u32, Stop> {
    let iid = ctx.state.heap.alloc(16);
    ctx.mem.write(iid, &CLASS_FACTORY).map_err(Stop::Fault)?;
    let factory = ctx.state.heap.alloc(4);
    ctx.mem.write_u32(factory, 0).map_err(Stop::Fault)?;
    // Nothing there yet, so a caller that believes a failed answer finds a
    // null rather than whatever was on the heap.
    ctx.mem.write_u32(args[4], 0).map_err(Stop::Fault)?;

    let mut code: Vec<u8> = Vec::new();
    let push = |code: &mut Vec<u8>, value: u32| {
        code.push(0x68);
        code.extend_from_slice(&value.to_le_bytes());
    };
    // A server attaches before it is called into, like every other module the
    // loader places (#55). A dynamic load passes no reserved argument.
    if let Some((entry, base)) = attach {
        push(&mut code, 0);
        push(&mut code, 1);
        push(&mut code, base);
        code.push(0xb8);
        code.extend_from_slice(&entry.to_le_bytes());
        code.extend_from_slice(&[0xff, 0xd0]);
    }
    // DllGetClassObject(class, IID_IClassFactory, &factory), stdcall.
    push(&mut code, factory);
    push(&mut code, iid);
    push(&mut code, args[0]);
    code.push(0xb8);
    code.extend_from_slice(&get_class_object.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]); // call eax
    code.extend_from_slice(&[0x85, 0xc0]); // test eax, eax
    code.extend_from_slice(&[0x75, 0x16]); // jnz over the rest, keeping its answer
                                           // factory->CreateInstance(outer, iid, out): slot three of its table.
    push(&mut code, args[4]);
    push(&mut code, args[3]);
    push(&mut code, args[1]);
    code.extend_from_slice(&[0xa1]); // mov eax, [factory]
    code.extend_from_slice(&factory.to_le_bytes());
    code.push(0x50); // push eax: the factory itself
    code.extend_from_slice(&[0x8b, 0x08]); // mov ecx, [eax]: its table
    code.extend_from_slice(&[0x8b, 0x41, 0x0c]); // mov eax, [ecx+0xc]
    code.extend_from_slice(&[0xff, 0xd0]); // call eax
    code.push(0xc3); // ret, back into the machine with its answer in eax

    let at = ctx.state.heap.alloc(u32::try_from(code.len()).unwrap_or(0));
    ctx.mem.write(at, &code).map_err(Stop::Fault)?;
    Ok(at)
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
    // DirectSound, asked for by class rather than through its own creator:
    // Alpha Team's sound server goes through COM for it (#60). What it asks
    // for is the maker rather than the thing.
    if class == "47d4d946-62e8-11cf-93bc-444553540000" && args[1] == 0 {
        let object = match interface.as_str() {
            // Asked for the maker rather than the thing.
            "00000001-0000-0000-c000-000000000046" => super::dsound::make_factory(ctx)?,
            // IID_IDirectSound, and IUnknown, which everything answers to.
            "279afa83-4981-11ce-a521-0020af0be560" | "00000000-0000-0000-c000-000000000046" => {
                super::dsound::make(ctx)?
            }
            _ => {
                return Err(Stop::NotWritten {
                    what: format!(
                        "ole32.dll!CoCreateInstance: DirectSound as {interface} is not a face \
                         this machine has"
                    ),
                    caller: 0,
                })
            }
        };
        ctx.mem.write_u32(args[4], object).map_err(Stop::Fault)?;
        return Ok(S_OK);
    }

    // Classes the declared machine does not have.
    //
    // A3D is one of them, for now. `a3dapi.dll` is in this title's own
    // cabinet, and running it is what ADR-0002 allows and #60 asked to try
    // first. It now loads, attaches, makes its DirectSound, sets a primary
    // format and starts two sound threads of its own — and there the machine
    // stops: its threads wait on events with a timeout this machine answers
    // at once, and the title's own thread ends up parked on a boundary call
    // it never comes back from. What it needs is an event that keeps whether
    // it has been signalled, and a wait that honours its timeout, neither of
    // which the declared machine has yet. Until then it says it has no A3D,
    // which is what a PC without it says and what this title's sound system
    // is written to hear. #60 carries the measurements.
    //
    // The second one is another DirectSound class A3D asks for before
    // settling for the one it made.
    if class == "92fa2c24-253c-11d2-90fb-006008a1f441"
        || class == "d8f1eee0-f634-11cf-8700-00a0245d918b"
    {
        ctx.log.line(&format!(
            "com: {class} is not registered on the declared machine"
        ));
        // REGDB_E_CLASSNOTREG.
        return Ok(0x8004_0154);
    }

    // A class the title's own disc ships a server for is made by that server,
    // which runs as guest code like everything else the disc carries
    // (ADR-0002, #60). Alpha Team's sound is A3D, and `a3dapi.dll` is in its
    // cabinet.
    if let Some(answer) = from_the_title_s_own_server(ctx, &class, args)? {
        return Ok(answer);
    }
    Err(Stop::NotWritten {
        what: format!("ole32.dll!CoCreateInstance: class {class} as {interface} is not a class this machine makes"),
        caller: 0,
    })
}
