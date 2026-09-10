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
        name: "GlobalAlloc",
        args: 2,
        convention: Convention::Stdcall,
        run: global_alloc,
    },
    Shim {
        dll: "KERNEL32",
        name: "GlobalFree",
        args: 1,
        convention: Convention::Stdcall,
        run: global_free,
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
    Shim {
        dll: "KERNEL32",
        name: "CreateMutexA",
        args: 3,
        convention: Convention::Stdcall,
        run: create_mutex,
    },
    Shim {
        dll: "KERNEL32",
        name: "CloseHandle",
        args: 1,
        convention: Convention::Stdcall,
        run: close_handle,
    },
    Shim {
        dll: "KERNEL32",
        name: "ReleaseMutex",
        args: 1,
        convention: Convention::Stdcall,
        run: release_mutex,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCurrentDirectoryA",
        args: 2,
        convention: Convention::Stdcall,
        run: get_current_directory,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetCurrentDirectoryA",
        args: 1,
        convention: Convention::Stdcall,
        run: set_current_directory,
    },
    Shim {
        dll: "KERNEL32",
        name: "CreateFileA",
        args: 7,
        convention: Convention::Stdcall,
        run: create_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "ReadFile",
        args: 5,
        convention: Convention::Stdcall,
        run: read_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetFilePointer",
        args: 4,
        convention: Convention::Stdcall,
        run: set_file_pointer,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetFileSize",
        args: 2,
        convention: Convention::Stdcall,
        run: get_file_size,
    },
    Shim {
        dll: "KERNEL32",
        name: "WriteFile",
        args: 5,
        convention: Convention::Stdcall,
        run: write_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetFileAttributesA",
        args: 1,
        convention: Convention::Stdcall,
        run: get_file_attributes,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetDriveTypeA",
        args: 1,
        convention: Convention::Stdcall,
        run: get_drive_type,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetLogicalDrives",
        args: 0,
        convention: Convention::Stdcall,
        run: get_logical_drives,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetVolumeInformationA",
        args: 8,
        convention: Convention::Stdcall,
        run: get_volume_information,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetTickCount",
        args: 0,
        convention: Convention::Stdcall,
        run: get_tick_count,
    },
    Shim {
        dll: "KERNEL32",
        name: "QueryPerformanceCounter",
        args: 1,
        convention: Convention::Stdcall,
        run: query_performance_counter,
    },
    Shim {
        dll: "KERNEL32",
        name: "QueryPerformanceFrequency",
        args: 1,
        convention: Convention::Stdcall,
        run: query_performance_frequency,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetSystemTime",
        args: 1,
        convention: Convention::Stdcall,
        run: get_system_time,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetLocalTime",
        args: 1,
        convention: Convention::Stdcall,
        run: get_system_time,
    },
    Shim {
        dll: "KERNEL32",
        name: "Sleep",
        args: 1,
        convention: Convention::Stdcall,
        run: sleep,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetSystemDirectoryA",
        args: 2,
        convention: Convention::Stdcall,
        run: get_system_directory,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetWindowsDirectoryA",
        args: 2,
        convention: Convention::Stdcall,
        run: get_windows_directory,
    },
    Shim {
        dll: "KERNEL32",
        name: "LoadLibraryA",
        args: 1,
        convention: Convention::Stdcall,
        run: load_library,
    },
    Shim {
        dll: "KERNEL32",
        name: "FreeLibrary",
        args: 1,
        convention: Convention::Stdcall,
        run: free_library,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetProcAddress",
        args: 2,
        convention: Convention::Stdcall,
        run: get_proc_address,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetPrivateProfileStringA",
        args: 6,
        convention: Convention::Stdcall,
        run: get_private_profile_string,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetPrivateProfileIntA",
        args: 4,
        convention: Convention::Stdcall,
        run: get_private_profile_int,
    },
    Shim {
        dll: "KERNEL32",
        name: "WritePrivateProfileStringA",
        args: 4,
        convention: Convention::Stdcall,
        run: write_private_profile_string,
    },
    Shim {
        dll: "KERNEL32",
        name: "FindFirstFileA",
        args: 2,
        convention: Convention::Stdcall,
        run: find_first_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "FindNextFileA",
        args: 2,
        convention: Convention::Stdcall,
        run: find_next_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "FindClose",
        args: 1,
        convention: Convention::Stdcall,
        run: find_close,
    },
    Shim {
        dll: "KERNEL32",
        name: "DeleteFileA",
        args: 1,
        convention: Convention::Stdcall,
        run: delete_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "CreateDirectoryA",
        args: 2,
        convention: Convention::Stdcall,
        run: create_directory,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetFullPathNameA",
        args: 4,
        convention: Convention::Stdcall,
        run: get_full_path_name,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetFileAttributesA",
        args: 2,
        convention: Convention::Stdcall,
        run: set_file_attributes,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetShortPathNameA",
        args: 3,
        convention: Convention::Stdcall,
        run: get_short_path_name,
    },
    Shim {
        dll: "KERNEL32",
        name: "GlobalMemoryStatus",
        args: 1,
        convention: Convention::Stdcall,
        run: global_memory_status,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetComputerNameA",
        args: 2,
        convention: Convention::Stdcall,
        run: get_computer_name,
    },
    Shim {
        dll: "KERNEL32",
        name: "CreateThread",
        args: 6,
        convention: Convention::Stdcall,
        run: create_thread,
    },
    Shim {
        dll: "KERNEL32",
        name: "OutputDebugStringA",
        args: 1,
        convention: Convention::Stdcall,
        run: output_debug_string,
    },
];

fn get_version(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.title.declared.windows)
}

fn get_version_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // OSVERSIONINFOA: size, major, minor, build, platform, then a 128-byte
    // string. Platform 1 is Windows 95 and its descendants.
    let at = args[0];
    let windows = ctx.title.declared.windows;
    let (major, minor, build) = (windows & 0xff, (windows >> 8) & 0xff, windows >> 16);
    for (offset, value) in [(4, major), (8, minor), (12, build), (16, 1)] {
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
    let path = format!("{}\\LegoRR.exe", ctx.title.install_dir);
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
    let line = format!("\"{}\\LegoRR.exe\"", ctx.title.install_dir);
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
    if addr == 0 {
        // A guest handed a zero happily uses it as an address, so an exhausted
        // heap is said out loud rather than left to become a null jump later.
        ctx.log.line(&format!(
            "heap exhausted asking for {size} bytes: {} handed out, {} live",
            ctx.state.heap.extent(),
            ctx.state.heap.live_bytes()
        ));
    }
    // HEAP_ZERO_MEMORY. The heap hands out memory that was zero to begin
    // with, but a block handed back and given out again is not, so the zeros
    // are written rather than assumed.
    if addr != 0 && flags & 0x8 != 0 {
        let zeros = vec![0u8; size as usize];
        ctx.mem.write(addr, &zeros).map_err(Stop::Fault)?;
    }
    Ok(addr)
}

fn global_alloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GlobalAlloc(flags, bytes). Fixed memory is an address, which is what a
    // title that never imports GlobalLock asks for; GMEM_ZEROINIT zeroes it.
    const GMEM_MOVEABLE: u32 = 0x2;
    const GMEM_ZEROINIT: u32 = 0x40;
    let (flags, size) = (args[0], args[1]);
    if flags & GMEM_MOVEABLE != 0 {
        return Err(Stop::NotWritten {
            what: format!("KERNEL32.dll!GlobalAlloc: moveable memory, flags {flags:#x}"),
            caller: 0,
        });
    }
    heap_alloc(
        ctx,
        &[0, if flags & GMEM_ZEROINIT != 0 { 0x8 } else { 0 }, size],
    )
}

fn global_free(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GlobalFree(memory): null when it is freed, the memory back when not.
    Ok(if ctx.state.heap.free(args[0]) {
        0
    } else {
        args[0]
    })
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
    // VirtualAlloc(address, size, type, protection).
    //
    // The declared machine's whole address space is already mapped, so
    // committing memory inside a region the guest already reserved is nothing
    // to do: the address it asked about is the address it gets back. Handing
    // out new memory for each commit is how a title that suballocates its own
    // four megabytes eats a heap five hundred megabytes wide.
    let (address, size) = (args[0], args[1]);
    if address != 0 {
        return Ok(address);
    }
    // A new region starts on Windows' allocation granularity, sixty-four
    // kilobytes, and is a whole number of pages: the C runtime's own heap
    // works out where its pages are from exactly that.
    let at = ctx
        .state
        .heap
        .alloc_aligned(size.next_multiple_of(0x1000), 0x1_0000);
    if at == 0 {
        ctx.log.line(&format!("no room to reserve {size} bytes"));
    }
    Ok(at)
}

fn virtual_free(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // VirtualFree(address, size, type). Memory given back reads as zero when
    // it is next committed, as Windows' does; the declared machine's pages are
    // always mapped, so that is done now rather than then.
    const MEM_DECOMMIT: u32 = 0x4000;
    const MEM_RELEASE: u32 = 0x8000;
    let (address, size, kind) = (args[0], args[1], args[2]);
    if kind & MEM_RELEASE != 0 {
        let whole = ctx.state.heap.size_of(address).unwrap_or(0);
        ctx.mem
            .write(address, &vec![0; whole as usize])
            .map_err(Stop::Fault)?;
        ctx.state.heap.free(address);
    } else if kind & MEM_DECOMMIT != 0 {
        // A size of zero decommits the whole region the address starts.
        let size = if size == 0 {
            ctx.state.heap.size_of(address).unwrap_or(0)
        } else {
            size
        };
        let start = address & !0xfff;
        let end = (address.wrapping_add(size)).next_multiple_of(0x1000);
        ctx.mem
            .write(start, &vec![0; end.saturating_sub(start) as usize])
            .map_err(Stop::Fault)?;
    }
    Ok(1)
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

fn create_mutex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A single-instance check: the guest makes a named mutex and asks whether
    // it already existed. On this machine it never has, because one run is one
    // machine and nothing else is holding the name.
    let name = ctx.string(args[2]).map_err(Stop::Fault)?;
    ctx.state.last_error = 0;
    Ok(ctx.state.give(Object::Named {
        kind: "mutex",
        name,
    }))
}

fn release_mutex(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn close_handle(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(ctx.state.close(args[0])))
}

fn get_current_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Where the guest believes it is: its own install directory, because that
    // is where the machine started it.
    let here = ctx.state.current_directory.clone();
    let written = ctx
        .write_string(args[1], args[0], &here)
        .map_err(Stop::Fault)?;
    Ok(written)
}

fn set_current_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.state.current_directory = path;
    Ok(1)
}

/// What a call that could not open a file hands back.
const INVALID_HANDLE: u32 = 0xffff_ffff;
const ERROR_FILE_NOT_FOUND: u32 = 2;

fn create_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    match ctx.open_guest_file(&path) {
        Some(bytes) => {
            ctx.log
                .at(crate::cli::Trace::Shims, || format!("file opened {path}"));
            ctx.state.last_error = 0;
            Ok(ctx.state.give(Object::File { path, bytes, at: 0 }))
        }
        None => {
            // Recorded rather than passed over: what a title opens is how the
            // declared install tree learns what it has to hold.
            ctx.log.line(&format!("file missing {path}"));
            ctx.state.last_error = ERROR_FILE_NOT_FOUND;
            Ok(INVALID_HANDLE)
        }
    }
}

fn read_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, buffer, want, read_at) = (args[0], args[1], args[2], args[3]);
    let Some(Object::File { bytes, at, .. }) = ctx.state.object(handle) else {
        ctx.state.last_error = 6;
        return Ok(0);
    };
    let start = *at as usize;
    let end = (start + want as usize).min(bytes.len());
    let chunk = bytes.get(start..end).unwrap_or(&[]).to_vec();
    ctx.mem.write(buffer, &chunk).map_err(Stop::Fault)?;
    if let Some(Object::File { at, .. }) = ctx.state.object_mut(handle) {
        *at += u32::try_from(chunk.len()).unwrap_or(0);
    }
    if read_at != 0 {
        ctx.mem
            .write_u32(read_at, u32::try_from(chunk.len()).unwrap_or(0))
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn write_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Writing is the profile's business, and the profile is #8's own ticket
    // rather than this one's: what the guest writes is counted as written so
    // it carries on, and where it lands is recorded.
    let (handle, want, wrote_at) = (args[0], args[2], args[3]);
    if let Some(Object::File { path, .. }) = ctx.state.object(handle) {
        let path = path.clone();
        ctx.log.line(&format!("file write {want} bytes to {path}"));
    }
    if wrote_at != 0 {
        ctx.mem.write_u32(wrote_at, want).map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn set_file_pointer(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, distance, method) = (args[0], args[1] as i32, args[3]);
    let Some(Object::File { bytes, at, .. }) = ctx.state.object(handle) else {
        return Ok(INVALID_HANDLE);
    };
    let len = u32::try_from(bytes.len()).unwrap_or(0);
    let base = match method {
        0 => 0,
        1 => *at,
        _ => len,
    };
    let moved = base.wrapping_add(distance as u32).min(len);
    if let Some(Object::File { at, .. }) = ctx.state.object_mut(handle) {
        *at = moved;
    }
    Ok(moved)
}

fn get_file_size(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    match ctx.state.object(args[0]) {
        Some(Object::File { bytes, .. }) => Ok(u32::try_from(bytes.len()).unwrap_or(0)),
        _ => Ok(INVALID_HANDLE),
    }
}

fn get_file_attributes(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.open_guest_file(&path).is_some() {
        // FILE_ATTRIBUTE_NORMAL.
        return Ok(0x80);
    }
    ctx.state.last_error = ERROR_FILE_NOT_FOUND;
    Ok(INVALID_HANDLE)
}

fn get_drive_type(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's drives: a fixed disk the title is installed on,
    // and a CD-ROM its pressing is in. Anything else does not exist.
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    let letter = path.chars().next().unwrap_or('C').to_ascii_uppercase();
    Ok(match letter {
        'C' => 3, // DRIVE_FIXED
        'D' => 5, // DRIVE_CDROM
        _ => 1,   // DRIVE_NO_ROOT_DIR
    })
}

fn get_logical_drives(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Bits for C and D, and nothing else.
    Ok((1 << 2) | (1 << 3))
}

fn get_volume_information(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A title looks at the disc's own label to decide whether the right disc
    // is in the drive. The label is the pressing's, read off the image itself.
    let root = ctx.string(args[0]).map_err(Stop::Fault)?;
    let is_cd = root.to_ascii_uppercase().starts_with('D');
    let label = if is_cd {
        ctx.state.volume_label.clone()
    } else {
        String::new()
    };
    ctx.write_string(args[1], args[2], &label)
        .map_err(Stop::Fault)?;
    if args[3] != 0 {
        ctx.mem
            .write_u32(args[3], 0x1234_5678)
            .map_err(Stop::Fault)?;
    }
    if args[4] != 0 {
        ctx.mem.write_u32(args[4], 255).map_err(Stop::Fault)?;
    }
    if args[5] != 0 {
        ctx.mem.write_u32(args[5], 0).map_err(Stop::Fault)?;
    }
    let system = if is_cd { "CDFS" } else { "FAT32" };
    ctx.write_string(args[6], args[7], system)
        .map_err(Stop::Fault)?;
    Ok(1)
}

fn get_tick_count(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // The declared clock, not the host's (ADR-0003).
    Ok(u32::try_from(ctx.declared_millis() & 0xffff_ffff).unwrap_or(0))
}

fn query_performance_counter(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let ticks = ctx.declared_millis() * 1000;
    ctx.mem
        .write_u32(args[0], ticks as u32)
        .map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(args[0] + 4, (ticks >> 32) as u32)
        .map_err(Stop::Fault)?;
    Ok(1)
}

fn query_performance_frequency(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A microsecond counter: declared, so that a title dividing by it gets the
    // same answer on every host.
    ctx.mem.write_u32(args[0], 1_000_000).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0] + 4, 0).map_err(Stop::Fault)?;
    Ok(1)
}

fn get_system_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A declared date: a run that read the host's calendar would replay
    // differently tomorrow. SYSTEMTIME is eight sixteen-bit fields.
    let at = args[0];
    let millis = ctx.declared_millis();
    let fields: [u16; 8] = [
        1999,
        10,
        4,
        29,
        u16::try_from(millis / 3_600_000 % 24).unwrap_or(0),
        u16::try_from(millis / 60_000 % 60).unwrap_or(0),
        u16::try_from(millis / 1000 % 60).unwrap_or(0),
        u16::try_from(millis % 1000).unwrap_or(0),
    ];
    for (i, value) in fields.iter().enumerate() {
        ctx.mem
            .write(at + (i as u32) * 2, &value.to_le_bytes())
            .map_err(Stop::Fault)?;
    }
    Ok(0)
}

fn sleep(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared clock moves; the host is not asked to wait, because
    // presentation is what paces a run (ADR-0003).
    ctx.state.waited_millis += u64::from(args[0]);
    Ok(0)
}

fn get_system_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let written = ctx
        .write_string(args[0], args[1], "C:\\WINDOWS\\SYSTEM")
        .map_err(Stop::Fault)?;
    Ok(written)
}

fn get_windows_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let written = ctx
        .write_string(args[0], args[1], "C:\\WINDOWS")
        .map_err(Stop::Fault)?;
    Ok(written)
}

fn load_library(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A library the title ships is a guest binary and belongs on the tree; one
    // Microsoft wrote is written here and has no image to load. Either way the
    // guest gets a handle, and what it does with it decides whether the run
    // goes on.
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("load library {name}"));
    if let Some(handle) = ctx.state.module(&name) {
        return Ok(handle);
    }
    Ok(ctx.state.give(Object::Module { name, base: 0 }))
}

fn free_library(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn get_proc_address(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // What a guest asks a library for by name. Answering with an address that
    // is not there would be worse than answering with nothing: a title that
    // probes for a symbol and finds it will then call it.
    let name = ctx.string(args[1]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("proc address {name}"));
    ctx.state.last_error = 127;
    Ok(0)
}

/// Where a title's settings file lives on the declared machine.
fn profile_path(file: &str) -> String {
    if file.contains('\\') {
        file.to_owned()
    } else {
        format!("C:\\WINDOWS\\{file}")
    }
}

fn get_private_profile_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Settings files: what is not there is what the guest passed as its own
    // default, which is how a first run behaves.
    let file = ctx.string(args[4]).map_err(Stop::Fault)?;
    let section = ctx.string(args[0]).map_err(Stop::Fault)?;
    let key = ctx.string(args[1]).map_err(Stop::Fault)?;
    let fallback = ctx.string(args[2]).map_err(Stop::Fault)?;
    let path = profile_path(&file);
    let value = ctx
        .state
        .profile_value(&path, &section, &key)
        .unwrap_or(fallback);
    let written = ctx
        .write_string(args[3], args[5], &value)
        .map_err(Stop::Fault)?;
    Ok(written)
}

fn get_private_profile_int(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let file = ctx.string(args[3]).map_err(Stop::Fault)?;
    let section = ctx.string(args[0]).map_err(Stop::Fault)?;
    let key = ctx.string(args[1]).map_err(Stop::Fault)?;
    let path = profile_path(&file);
    Ok(ctx
        .state
        .profile_value(&path, &section, &key)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(args[2]))
}

fn write_private_profile_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let file = ctx.string(args[3]).map_err(Stop::Fault)?;
    let section = ctx.string(args[0]).map_err(Stop::Fault)?;
    let key = ctx.string(args[1]).map_err(Stop::Fault)?;
    let value = ctx.string(args[2]).map_err(Stop::Fault)?;
    let path = profile_path(&file);
    ctx.state.set_profile_value(&path, &section, &key, value);
    Ok(1)
}

fn find_first_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Nothing matches. A title enumerating the disc is looking for files the
    // declared tree does not hold yet, and telling it there are none is
    // honest; telling it there is one it cannot then open would not be.
    let pattern = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("find first {pattern}"));
    ctx.state.last_error = ERROR_FILE_NOT_FOUND;
    Ok(INVALID_HANDLE)
}

fn find_next_file(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // ERROR_NO_MORE_FILES.
    ctx.state.last_error = 18;
    Ok(0)
}

fn find_close(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn delete_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("delete {path}"));
    Ok(1)
}

fn create_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("create directory {path}"));
    Ok(1)
}

fn set_file_attributes(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn get_full_path_name(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A relative path is resolved against where the guest believes it is.
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    let full = if path.len() > 1 && path.as_bytes().get(1) == Some(&b':') {
        path
    } else {
        format!(
            "{}\\{}",
            ctx.state.current_directory,
            path.trim_start_matches('\\')
        )
    };
    let written = ctx
        .write_string(args[2], args[1], &full)
        .map_err(Stop::Fault)?;
    if args[3] != 0 {
        // The file part starts after the last separator.
        let at = full.rfind('\\').map_or(0, |i| i as u32 + 1);
        ctx.mem
            .write_u32(args[3], args[2] + at)
            .map_err(Stop::Fault)?;
    }
    Ok(written)
}

fn get_short_path_name(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's paths are already short.
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    let written = ctx
        .write_string(args[1], args[2], &path)
        .map_err(Stop::Fault)?;
    Ok(written)
}

fn global_memory_status(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // MEMORYSTATUS: its own size, the load, then five sizes. All declared: a
    // title that sized its caches from the host's memory would size them
    // differently on every host.
    let total = ctx.title.declared.megabytes * 1024 * 1024;
    let at = args[0];
    for (offset, value) in [
        (0, 32),
        (4, 20),
        (8, total),
        (12, total / 2),
        (16, total),
        (20, total / 2),
        (24, total),
        (28, total / 2),
    ] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(0)
}

fn get_computer_name(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let name = ctx.title.declared.computer.to_owned();
    let room = ctx.mem.read_u32(args[1]).map_err(Stop::Fault)?;
    let written = ctx
        .write_string(args[0], room, &name)
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], written).map_err(Stop::Fault)?;
    Ok(1)
}

fn create_thread(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A second flow of control is scheduled by the machine, not by the host
    // (ADR-0005), and nothing on this roster has needed one yet. The first
    // title that does gets the cooperative scheduler; until then this is a
    // wall rather than a thread that silently never runs.
    Err(Stop::NotWritten {
        what: "KERNEL32.dll!CreateThread".to_owned(),
        caller: ctx.cpu.r[crate::cpu::ESP],
    })
}

fn output_debug_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // What the title's own developers watched. It goes in the run log, where
    // it is the guest's own account of what it is doing.
    let text = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("guest says {}", text.trim_end()));
    Ok(0)
}
