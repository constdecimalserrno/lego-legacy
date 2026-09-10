//! KERNEL32, as much of it as the roster has asked for.
//!
//! Every value a shim hands back is a declared value: chosen for the machine
//! we say we are, carried with the reason for that value and no other.

use super::{Convention, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "KERNEL32",
        name: "GetVersion",
        args: 0,
        convention: Convention::Stdcall,
        run: get_version,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetVersionExA",
        args: 1,
        convention: Convention::Stdcall,
        run: get_version_ex,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetModuleHandleA",
        args: 1,
        convention: Convention::Stdcall,
        run: get_module_handle,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetModuleFileNameA",
        args: 3,
        convention: Convention::Stdcall,
        run: get_module_file_name,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCommandLineA",
        args: 0,
        convention: Convention::Stdcall,
        run: get_command_line,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetStartupInfoA",
        args: 1,
        convention: Convention::Stdcall,
        run: get_startup_info,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetProcessHeap",
        args: 0,
        convention: Convention::Stdcall,
        run: get_process_heap,
    },
    Shim {
        dll: "KERNEL32",
        name: "HeapCreate",
        args: 3,
        convention: Convention::Stdcall,
        run: heap_create,
    },
    Shim {
        dll: "KERNEL32",
        name: "HeapDestroy",
        args: 1,
        convention: Convention::Stdcall,
        run: heap_destroy,
    },
    Shim {
        dll: "KERNEL32",
        name: "HeapAlloc",
        args: 3,
        convention: Convention::Stdcall,
        run: heap_alloc,
    },
    Shim {
        dll: "KERNEL32",
        name: "HeapFree",
        args: 3,
        convention: Convention::Stdcall,
        run: heap_free,
    },
    Shim {
        dll: "KERNEL32",
        name: "HeapReAlloc",
        args: 4,
        convention: Convention::Stdcall,
        run: heap_realloc,
    },
    Shim {
        dll: "KERNEL32",
        name: "HeapSize",
        args: 3,
        convention: Convention::Stdcall,
        run: heap_size,
    },
    Shim {
        dll: "KERNEL32",
        name: "VirtualAlloc",
        args: 4,
        convention: Convention::Stdcall,
        run: virtual_alloc,
    },
    Shim {
        dll: "KERNEL32",
        name: "VirtualFree",
        args: 3,
        convention: Convention::Stdcall,
        run: virtual_free,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetStdHandle",
        args: 1,
        convention: Convention::Stdcall,
        run: get_std_handle,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetFileType",
        args: 1,
        convention: Convention::Stdcall,
        run: get_file_type,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetHandleCount",
        args: 1,
        convention: Convention::Stdcall,
        run: set_handle_count,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetACP",
        args: 0,
        convention: Convention::Stdcall,
        run: get_acp,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetOEMCP",
        args: 0,
        convention: Convention::Stdcall,
        run: get_oemcp,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCPInfo",
        args: 2,
        convention: Convention::Stdcall,
        run: get_cp_info,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetEnvironmentStrings",
        args: 0,
        convention: Convention::Stdcall,
        run: get_environment_strings,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetEnvironmentStringsW",
        args: 0,
        convention: Convention::Stdcall,
        run: get_environment_strings_w,
    },
    Shim {
        dll: "KERNEL32",
        name: "FreeEnvironmentStringsA",
        args: 1,
        convention: Convention::Stdcall,
        run: free_environment_strings,
    },
    Shim {
        dll: "KERNEL32",
        name: "FreeEnvironmentStringsW",
        args: 1,
        convention: Convention::Stdcall,
        run: free_environment_strings,
    },
    Shim {
        dll: "KERNEL32",
        name: "InitializeCriticalSection",
        args: 1,
        convention: Convention::Stdcall,
        run: critical_section,
    },
    Shim {
        dll: "KERNEL32",
        name: "EnterCriticalSection",
        args: 1,
        convention: Convention::Stdcall,
        run: critical_section,
    },
    Shim {
        dll: "KERNEL32",
        name: "LeaveCriticalSection",
        args: 1,
        convention: Convention::Stdcall,
        run: critical_section,
    },
    Shim {
        dll: "KERNEL32",
        name: "DeleteCriticalSection",
        args: 1,
        convention: Convention::Stdcall,
        run: critical_section,
    },
    Shim {
        dll: "KERNEL32",
        name: "TlsAlloc",
        args: 0,
        convention: Convention::Stdcall,
        run: tls_alloc,
    },
    Shim {
        dll: "KERNEL32",
        name: "TlsSetValue",
        args: 2,
        convention: Convention::Stdcall,
        run: tls_set,
    },
    Shim {
        dll: "KERNEL32",
        name: "TlsGetValue",
        args: 1,
        convention: Convention::Stdcall,
        run: tls_get,
    },
    Shim {
        dll: "KERNEL32",
        name: "TlsFree",
        args: 1,
        convention: Convention::Stdcall,
        run: tls_free,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetUnhandledExceptionFilter",
        args: 1,
        convention: Convention::Stdcall,
        run: set_unhandled_exception_filter,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCurrentProcess",
        args: 0,
        convention: Convention::Stdcall,
        run: current_process,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCurrentProcessId",
        args: 0,
        convention: Convention::Stdcall,
        run: current_process_id,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCurrentThreadId",
        args: 0,
        convention: Convention::Stdcall,
        run: current_thread_id,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetLastError",
        args: 0,
        convention: Convention::Stdcall,
        run: get_last_error,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetLastError",
        args: 1,
        convention: Convention::Stdcall,
        run: set_last_error,
    },
    Shim {
        dll: "KERNEL32",
        name: "ExitProcess",
        args: 1,
        convention: Convention::Stdcall,
        run: exit_process,
    },
    Shim {
        dll: "KERNEL32",
        name: "WideCharToMultiByte",
        args: 8,
        convention: Convention::Stdcall,
        run: wide_to_multi,
    },
    Shim {
        dll: "KERNEL32",
        name: "MultiByteToWideChar",
        args: 6,
        convention: Convention::Stdcall,
        run: multi_to_wide,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetStringTypeA",
        args: 5,
        convention: Convention::Stdcall,
        run: get_string_type_a,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetStringTypeW",
        args: 4,
        convention: Convention::Stdcall,
        run: get_string_type_w,
    },
    Shim {
        dll: "KERNEL32",
        name: "LCMapStringA",
        args: 6,
        convention: Convention::Stdcall,
        run: lc_map_string,
    },
    Shim {
        dll: "KERNEL32",
        name: "LCMapStringW",
        args: 6,
        convention: Convention::Stdcall,
        run: lc_map_string,
    },
];

/// The Windows this machine declares itself to be.
///
/// Windows 98 Second Edition: major 4, minor 10, build 2222, and the top bit
/// clear to say it is not Windows NT. Chosen because it is what the roster
/// shipped against and what their installers check for; a title that asks is
/// told the same thing every run.
const WINDOWS_98_SE: u32 = 0x0a04 | (2222 << 16);

fn get_version(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(WINDOWS_98_SE)
}

fn get_version_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // OSVERSIONINFOA: size, major, minor, build, platform, then a 128-byte
    // string. Platform 1 is Windows 95 and its descendants.
    let at = args[0];
    for (offset, value) in [(4, 4u32), (8, 10), (12, 2222), (16, 1)] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    ctx.write_string(at + 20, 128, "").map_err(Stop::Fault)?;
    Ok(1)
}

fn get_module_handle(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    if args[0] == 0 {
        // The running image itself.
        return Ok(crate::pe::IMAGE_BASE_DEFAULT);
    }
    Ok(ctx.state.module(&name).unwrap_or(0))
}

fn get_module_file_name(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Where the guest believes it was started from. A declared value: the
    // install tree's own path, not the host's.
    let path = format!("C:\\{}\\{}", ctx.title.key, "LegoRR.exe");
    let written = ctx
        .write_string(args[1], args[2], &path)
        .map_err(Stop::Fault)?;
    Ok(written)
}

fn get_command_line(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(addr) = ctx.state.command_line {
        return Ok(addr);
    }
    // The launch quotes the program's own path and passes nothing else: what
    // a player double-clicking the game would produce.
    let line = format!("\"C:\\{}\\LegoRR.exe\"", ctx.title.key);
    let addr = ctx.intern(&line).map_err(Stop::Fault)?;
    ctx.state.command_line = Some(addr);
    Ok(addr)
}

fn get_startup_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // STARTUPINFOA is 68 bytes. Everything zero but its own size, and no
    // flags: the guest is being started by a person, not by a shell handing
    // it a window state.
    let at = args[0];
    for offset in (0..68).step_by(4) {
        ctx.mem.write_u32(at + offset, 0).map_err(Stop::Fault)?;
    }
    ctx.mem.write_u32(at, 68).map_err(Stop::Fault)?;
    Ok(0)
}

fn get_process_heap(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.give(Object::Heap))
}

fn heap_create(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // One heap stands behind every handle: the guest cannot tell, because
    // every allocation it gets back is real memory it can write to.
    Ok(ctx.state.give(Object::Heap))
}

fn heap_destroy(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(ctx.state.close(args[0])))
}

fn heap_alloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (flags, size) = (args[1], args[2]);
    let addr = ctx.state.heap.alloc(size);
    // HEAP_ZERO_MEMORY. The heap hands out memory that was zero to begin
    // with, but a block handed back and given out again is not, so the zeros
    // are written rather than assumed.
    if addr != 0 && flags & 0x8 != 0 {
        let zeros = vec![0u8; size as usize];
        ctx.mem.write(addr, &zeros).map_err(Stop::Fault)?;
    }
    Ok(addr)
}

fn heap_free(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(ctx.state.heap.free(args[2])))
}

fn heap_realloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (old, size) = (args[2], args[3]);
    let previous = ctx.state.heap.size_of(old).unwrap_or(0);
    let new = ctx.state.heap.alloc(size);
    if new == 0 {
        return Ok(0);
    }
    if old != 0 && previous != 0 {
        let take = previous.min(size) as usize;
        let bytes = ctx.mem.slice(old, take).map_err(Stop::Fault)?.to_vec();
        ctx.mem.write(new, &bytes).map_err(Stop::Fault)?;
        ctx.state.heap.free(old);
    }
    Ok(new)
}

fn heap_size(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.heap.size_of(args[2]).unwrap_or(u32::MAX))
}

fn virtual_alloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The guest asks for pages; it gets heap, because the declared machine's
    // whole address space is already mapped and the difference it could
    // measure is protection, which this machine does not have.
    let size = args[1];
    Ok(ctx.state.heap.alloc(size.next_multiple_of(0x1000)))
}

fn virtual_free(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(ctx.state.heap.free(args[0])))
}

fn get_std_handle(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.give(Object::Std(args[0])))
}

fn get_file_type(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // FILE_TYPE_CHAR: the standard streams are a console, which is what the C
    // runtime expects of a machine with no redirection.
    Ok(2)
}

fn set_handle_count(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The C runtime asks for room for this many files and is told it has it.
    Ok(args[0])
}

fn get_acp(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Windows-1252: the code page every one of these titles was authored in.
    Ok(1252)
}

fn get_oemcp(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Code page 437, the one a Western PC's console used.
    Ok(437)
}

fn get_cp_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CPINFO: one byte per character, a default character of '?', and no
    // lead-byte ranges, because Windows-1252 has none.
    let at = args[1];
    ctx.mem.write_u32(at, 1).map_err(Stop::Fault)?;
    ctx.mem.write(at + 4, b"?\0").map_err(Stop::Fault)?;
    ctx.mem.write(at + 6, &[0u8; 12]).map_err(Stop::Fault)?;
    Ok(1)
}

/// The environment the declared machine hands a title.
///
/// Two variables and nothing else: enough for a C runtime to parse a block
/// without being given a host's own environment, which would make a run depend
/// on the machine it was taken on.
const ENVIRONMENT: &str = "PATH=C:\\WINDOWS;C:\\WINDOWS\\COMMAND\0TEMP=C:\\WINDOWS\\TEMP\0\0";

fn get_environment_strings(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    let bytes = ENVIRONMENT.as_bytes();
    let addr = ctx
        .state
        .heap
        .alloc(u32::try_from(bytes.len()).unwrap_or(0));
    if addr != 0 {
        ctx.mem.write(addr, bytes).map_err(Stop::Fault)?;
    }
    Ok(addr)
}

fn get_environment_strings_w(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    let wide: Vec<u8> = ENVIRONMENT
        .as_bytes()
        .iter()
        .flat_map(|b| [*b, 0])
        .collect();
    let addr = ctx.state.heap.alloc(u32::try_from(wide.len()).unwrap_or(0));
    if addr != 0 {
        ctx.mem.write(addr, &wide).map_err(Stop::Fault)?;
    }
    Ok(addr)
}

fn free_environment_strings(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.state.heap.free(args[0]);
    Ok(1)
}

fn critical_section(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A machine with one flow of control cannot contend for a lock, so
    // entering one costs nothing and always succeeds. The structure the guest
    // passes is its own memory and is left exactly as it found it.
    Ok(0)
}

fn tls_alloc(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    let index = u32::try_from(ctx.state.scratch.len()).unwrap_or(0);
    ctx.state.scratch.push(0);
    Ok(index)
}

fn tls_set(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if let Some(slot) = ctx.state.scratch.get_mut(args[0] as usize) {
        *slot = args[1];
        return Ok(1);
    }
    Ok(0)
}

fn tls_get(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx
        .state
        .scratch
        .get(args[0] as usize)
        .copied()
        .unwrap_or(0))
}

fn tls_free(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn set_unhandled_exception_filter(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // There was no filter before this one.
    Ok(0)
}

/// The handle a process uses to mean itself. A declared value, and deliberately
/// not a real handle: nothing may close it.
const CURRENT_PROCESS: u32 = 0xffff_ffff;

fn current_process(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(CURRENT_PROCESS)
}

fn current_process_id(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Declared, not the host's: a recorded run must not depend on which
    // process happened to run it.
    Ok(0x0000_1234)
}

fn current_thread_id(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0x0000_1000)
}

fn get_last_error(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.last_error)
}

fn set_last_error(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.state.last_error = args[0];
    Ok(0)
}

fn exit_process(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.log.line(&format!("guest exit {}", args[0]));
    Err(Stop::Exited { code: args[0] })
}

/// How many wide characters a string holds, given the count the guest passed.
///
/// A negative count means the string ends at its own NUL, which is counted in.
fn wide_len(ctx: &Ctx<'_>, at: u32, count: u32) -> Result<usize, Stop> {
    if count as i32 >= 0 {
        return Ok(count as usize);
    }
    let mut n = 0;
    loop {
        let unit = ctx.mem.read_u32(at + (n as u32) * 2).map_err(Stop::Fault)? & 0xffff;
        n += 1;
        if unit == 0 || n > 0x1_0000 {
            break;
        }
    }
    Ok(n)
}

fn byte_len(ctx: &Ctx<'_>, at: u32, count: u32) -> Result<usize, Stop> {
    if count as i32 >= 0 {
        return Ok(count as usize);
    }
    Ok(ctx.string(at).map_err(Stop::Fault)?.len() + 1)
}

fn wide_to_multi(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Windows-1252 is a byte per character for everything these titles carry,
    // so the conversion is the low half of each unit and a question mark for
    // anything above it.
    let (src, count, dst, room) = (args[2], args[3], args[4], args[5]);
    let n = wide_len(ctx, src, count)?;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let unit = ctx
            .mem
            .read_u32(src + (i as u32) * 2)
            .map_err(Stop::Fault)?
            & 0xffff;
        out.push(if unit < 0x100 { unit as u8 } else { b'?' });
    }
    if room == 0 {
        return Ok(u32::try_from(out.len()).unwrap_or(0));
    }
    let take = out.len().min(room as usize);
    ctx.mem.write(dst, &out[..take]).map_err(Stop::Fault)?;
    Ok(u32::try_from(take).unwrap_or(0))
}

fn multi_to_wide(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (src, count, dst, room) = (args[2], args[3], args[4], args[5]);
    let n = byte_len(ctx, src, count)?;
    let bytes = ctx.mem.slice(src, n).map_err(Stop::Fault)?.to_vec();
    if room == 0 {
        return Ok(u32::try_from(bytes.len()).unwrap_or(0));
    }
    let take = bytes.len().min(room as usize);
    let wide: Vec<u8> = bytes[..take].iter().flat_map(|b| [*b, 0]).collect();
    ctx.mem.write(dst, &wide).map_err(Stop::Fault)?;
    Ok(u32::try_from(take).unwrap_or(0))
}

/// What Windows says a character is, for the one code page this machine has.
fn char_type(b: u8) -> u16 {
    let mut t = 0;
    if b.is_ascii_uppercase() {
        t |= 0x0001 | 0x0100; // upper, alpha
    }
    if b.is_ascii_lowercase() {
        t |= 0x0002 | 0x0100;
    }
    if b.is_ascii_digit() {
        t |= 0x0004;
    }
    if b == b' ' || b == b'\t' {
        t |= 0x0008;
    }
    if b.is_ascii_punctuation() {
        t |= 0x0010;
    }
    if b.is_ascii_control() {
        t |= 0x0020;
    }
    if b == b' ' {
        t |= 0x0040;
    }
    if b.is_ascii_hexdigit() {
        t |= 0x0080;
    }
    t
}

fn get_string_type_a(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The locale comes first in the A form; everything after it matches the W
    // form's arguments.
    let (src, count, dst) = (args[2], args[3], args[4]);
    let n = byte_len(ctx, src, count)?;
    for i in 0..n {
        let b = ctx.mem.slice(src + i as u32, 1).map_err(Stop::Fault)?[0];
        let t = u32::from(char_type(b));
        ctx.mem
            .write(dst + (i as u32) * 2, &(t as u16).to_le_bytes())
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn get_string_type_w(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (src, count, dst) = (args[1], args[2], args[3]);
    let n = wide_len(ctx, src, count)?;
    for i in 0..n {
        let unit = ctx
            .mem
            .read_u32(src + (i as u32) * 2)
            .map_err(Stop::Fault)?
            & 0xffff;
        let t = char_type(u8::try_from(unit).unwrap_or(b'?'));
        ctx.mem
            .write(dst + (i as u32) * 2, &t.to_le_bytes())
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn lc_map_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Only the two mappings a C runtime asks for: upper and lower case.
    let (flags, src, count, dst, room) = (args[1], args[2], args[3], args[4], args[5]);
    let n = byte_len(ctx, src, count)?;
    let bytes = ctx.mem.slice(src, n).map_err(Stop::Fault)?.to_vec();
    let mapped: Vec<u8> = bytes
        .iter()
        .map(|b| {
            if flags & 0x0000_0200 != 0 {
                b.to_ascii_uppercase()
            } else if flags & 0x0000_0100 != 0 {
                b.to_ascii_lowercase()
            } else {
                *b
            }
        })
        .collect();
    if room == 0 {
        return Ok(u32::try_from(mapped.len()).unwrap_or(0));
    }
    let take = mapped.len().min(room as usize);
    ctx.mem.write(dst, &mapped[..take]).map_err(Stop::Fault)?;
    Ok(u32::try_from(take).unwrap_or(0))
}
