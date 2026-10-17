//! COM's own library: what a title calls to make an object by its class.
//!
//! Nothing here is a registry of installed components. A class this machine
//! makes is one a roster title asked for, written in Rust like everything
//! else at the boundary (ADR-0002); any other class is a wall that names it.

use super::{stdcall, Ctx, Shim};
use crate::com::{iid_text, read_iid, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("ole32", "CoInitialize", 1, initialize),
    stdcall("ole32", "CoUninitialize", 0, uninitialize),
    stdcall("ole32", "CoTaskMemFree", 1, task_mem_free),
    stdcall("ole32", "CoCreateInstance", 5, create_instance),
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
///
/// The disc's own `a3dapi.dll` runs as guest code, rather than `IA3d4` and its
/// source and listener interfaces being written here (#60). A3D mixes in
/// software when it finds no Aureal hardware — and it finds none here — so
/// what it produces is ordinary DirectSound buffers, which this machine
/// already puts in its mix (#19, #23); writing it here would mean a 3D mixer
/// against no specification to replace a software one the disc already
/// carries. Running it needs a machine whose waits are real: events that
/// remember being signalled, mutexes that remember who holds them, timeouts
/// that are honoured, and threads that park rather than ask again (ADR-0005).
const SERVERS: &[(&str, &str)] = &[("92fa2c24-253c-11d2-90fb-006008a1f441", "a3dapi.dll")];

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
    // Whether this is the first time: a server attaches once, before anything
    // calls into it.
    let fresh = !ctx.modules.holds(file);
    if super::kernel32::load_and_bind(ctx, file)?.is_none() {
        ctx.log
            .line(&format!("com: {file} is not on the declared machine"));
        // REGDB_E_CLASSNOTREG, which is what a machine without it says.
        return Ok(Some(0x8004_0154));
    }
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

    let code = class_factory_code(get_class_object, attach, args, iid, factory);
    let at = ctx.state.heap.alloc(u32::try_from(code.len()).unwrap_or(0));
    ctx.mem.write(at, &code).map_err(Stop::Fault)?;
    Ok(at)
}

/// The class factory program's bytes, given where the interface it asks for
/// and the factory it is handed live.
fn class_factory_code(
    get_class_object: u32,
    attach: Option<(u32, u32)>,
    args: &[u32],
    iid: u32,
    factory: u32,
) -> Vec<u8> {
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
    code.extend_from_slice(&[0x75, 0x1c]); // jnz past the second call's 0x1c bytes to the ret

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
    code
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

    // A class the declared machine does not have: another DirectSound class
    // A3D asks for before settling for the one it made.
    if class == "d8f1eee0-f634-11cf-8700-00a0245d918b" {
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

#[cfg(test)]
mod tests {
    use super::class_factory_code;
    use crate::cpu::{interp::Interp, Cpu, CpuEngine, Mode, EAX, ESP};
    use crate::guest::{Arena, GuestMem};
    use crate::host::headless::Headless;

    #[test]
    fn a_server_that_will_not_hand_out_a_factory_is_answered_with_its_own_refusal() {
        const SERVER: u32 = 0x0050_0000;
        const PROGRAM: u32 = 0x0051_0000;
        const BACK: u32 = 0x0052_0000;
        const STACK: u32 = 0x0010_0000;
        let mut host = Headless::new();
        let mut mem = Arena::new(&mut host, crate::machine::ARENA_BYTES).unwrap();
        // A DllGetClassObject that answers CLASS_E_CLASSNOTAVAILABLE:
        // `mov eax, 0x80040111; ret 12`.
        mem.write(SERVER, &[0xb8, 0x11, 0x01, 0x04, 0x80, 0xc2, 0x0c, 0x00])
            .unwrap();
        let args = [0x0053_0000, 0, 0, 0x0053_0010, 0x0053_0020];
        let code = class_factory_code(SERVER, None, &args, 0x0053_0030, 0x0053_0040);
        mem.write(PROGRAM, &code).unwrap();
        // Called by the machine, which takes its answer back at BACK.
        mem.write_u32(STACK, BACK).unwrap();
        let mut cpu = Cpu {
            eip: PROGRAM,
            ..Cpu::default()
        };
        cpu.r[ESP] = STACK;

        let mut engine = Interp::new(Mode::FLAT);
        for _ in 0..32 {
            if cpu.eip == BACK {
                break;
            }
            engine
                .step(&mut cpu, &mut mem)
                .expect("the program runs whole instructions");
        }
        assert_eq!(cpu.eip, BACK, "the program comes back");
        assert_eq!(cpu.r[EAX], 0x8004_0111, "the server's own answer");
        assert_eq!(cpu.r[ESP], STACK + 4, "and the stack as it was");
    }
}
