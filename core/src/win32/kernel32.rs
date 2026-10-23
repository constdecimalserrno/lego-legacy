//! KERNEL32, as much of it as the roster has asked for.
//!
//! Every value a shim hands back is a declared value: chosen for the machine
//! we say we are, carried with the reason for that value and no other.

use super::{stdcall, Contents, Ctx, Object, Shim, State, DRIVE_CDROM};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("KERNEL32", "GetVersion", 0, get_version),
    stdcall("KERNEL32", "GetVersionExA", 1, get_version_ex),
    stdcall("KERNEL32", "GetModuleHandleA", 1, get_module_handle),
    stdcall("KERNEL32", "GetModuleFileNameA", 3, get_module_file_name),
    stdcall(
        "KERNEL32",
        "GetEnvironmentVariableA",
        3,
        get_environment_variable,
    ),
    stdcall(
        "KERNEL32",
        "GetSystemTimeAsFileTime",
        1,
        get_system_time_as_file_time,
    ),
    stdcall("KERNEL32", "InterlockedIncrement", 1, interlocked_increment),
    stdcall("KERNEL32", "InterlockedDecrement", 1, interlocked_decrement),
    stdcall("KERNEL32", "InterlockedExchange", 2, interlocked_exchange),
    stdcall("KERNEL32", "IsBadReadPtr", 2, is_bad_read_ptr),
    stdcall("KERNEL32", "IsBadWritePtr", 2, is_bad_read_ptr),
    stdcall("KERNEL32", "IsBadCodePtr", 1, is_bad_code_ptr),
    stdcall(
        "KERNEL32",
        "DisableThreadLibraryCalls",
        1,
        disable_thread_library_calls,
    ),
    stdcall("KERNEL32", "SetErrorMode", 1, set_error_mode),
    stdcall("KERNEL32", "SetStdHandle", 2, set_std_handle),
    stdcall("KERNEL32", "FlushFileBuffers", 1, flush_file_buffers),
    stdcall("KERNEL32", "GetSystemInfo", 1, get_system_info),
    stdcall("KERNEL32", "TerminateProcess", 2, terminate_process),
    stdcall("KERNEL32", "GetCurrentThread", 0, get_current_thread),
    stdcall("KERNEL32", "CreateFileMappingA", 6, create_file_mapping),
    stdcall("KERNEL32", "MapViewOfFile", 5, map_view_of_file),
    stdcall("KERNEL32", "UnmapViewOfFile", 1, unmap_view_of_file),
    stdcall("KERNEL32", "lstrcpyA", 2, lstrcpy),
    stdcall("KERNEL32", "lstrcatA", 2, lstrcat),
    stdcall("KERNEL32", "lstrlenA", 1, lstrlen),
    stdcall("KERNEL32", "lstrcmpiA", 2, lstrcmpi),
    stdcall("KERNEL32", "CompareFileTime", 2, compare_file_time),
    stdcall("KERNEL32", "GetFileTime", 4, get_file_time),
    stdcall("KERNEL32", "RtlUnwind", 4, rtl_unwind),
    stdcall("KERNEL32", "RaiseException", 4, raise_exception),
    stdcall("KERNEL32", "MulDiv", 3, mul_div),
    stdcall(
        "KERNEL32",
        "GetTimeZoneInformation",
        1,
        get_time_zone_information,
    ),
    stdcall(
        "KERNEL32",
        "GetLogicalDriveStringsA",
        2,
        get_logical_drive_strings,
    ),
    stdcall("KERNEL32", "GetCommandLineW", 0, get_command_line_w),
    stdcall("KERNEL32", "GetUserDefaultLCID", 0, user_default_lcid),
    stdcall("KERNEL32", "GetUserDefaultLangID", 0, user_default_lang_id),
    // The system's language is the user's, as the declared machine has one
    // locale: LEGO Chess asks it at the top of its main (the call that returns to 0x004086E6,
    // #147).
    stdcall(
        "KERNEL32",
        "GetSystemDefaultLangID",
        0,
        user_default_lang_id,
    ),
    stdcall("KERNEL32", "IsValidCodePage", 1, is_valid_code_page),
    stdcall("KERNEL32", "IsValidLocale", 2, is_valid_locale),
    stdcall("KERNEL32", "GetLocaleInfoA", 4, get_locale_info_a),
    stdcall("KERNEL32", "GetLocaleInfoW", 4, get_locale_info_w),
    stdcall("KERNEL32", "GetDateFormatA", 6, get_date_format_a),
    stdcall("KERNEL32", "GetTimeFormatA", 6, get_time_format_a),
    stdcall("KERNEL32", "EnumSystemLocalesA", 2, enum_system_locales),
    stdcall("KERNEL32", "CompareStringA", 6, compare_string_a),
    stdcall("KERNEL32", "CompareStringW", 6, compare_string_w),
    stdcall("KERNEL32", "GetCommandLineA", 0, get_command_line),
    stdcall("KERNEL32", "GetStartupInfoA", 1, get_startup_info),
    stdcall("KERNEL32", "HeapCreate", 3, heap_create),
    stdcall("KERNEL32", "HeapDestroy", 1, heap_destroy),
    stdcall("KERNEL32", "HeapAlloc", 3, heap_alloc),
    stdcall("KERNEL32", "GlobalAlloc", 2, global_alloc),
    stdcall("KERNEL32", "GlobalFree", 1, global_free),
    stdcall("KERNEL32", "HeapFree", 3, heap_free),
    stdcall("KERNEL32", "HeapReAlloc", 4, heap_realloc),
    stdcall("KERNEL32", "HeapSize", 3, heap_size),
    stdcall("KERNEL32", "VirtualAlloc", 4, virtual_alloc),
    stdcall("KERNEL32", "VirtualFree", 3, virtual_free),
    stdcall("KERNEL32", "VirtualQuery", 3, virtual_query),
    stdcall("KERNEL32", "GetStdHandle", 1, get_std_handle),
    stdcall("KERNEL32", "GetFileType", 1, get_file_type),
    stdcall("KERNEL32", "SetHandleCount", 1, set_handle_count),
    stdcall("KERNEL32", "GetACP", 0, get_acp),
    stdcall("KERNEL32", "GetOEMCP", 0, get_oemcp),
    stdcall("KERNEL32", "GetCPInfo", 2, get_cp_info),
    stdcall(
        "KERNEL32",
        "GetEnvironmentStrings",
        0,
        get_environment_strings,
    ),
    stdcall(
        "KERNEL32",
        "GetEnvironmentStringsW",
        0,
        get_environment_strings_w,
    ),
    stdcall(
        "KERNEL32",
        "FreeEnvironmentStringsA",
        1,
        free_environment_strings,
    ),
    stdcall(
        "KERNEL32",
        "FreeEnvironmentStringsW",
        1,
        free_environment_strings,
    ),
    stdcall("KERNEL32", "InitializeCriticalSection", 1, critical_section),
    stdcall(
        "KERNEL32",
        "EnterCriticalSection",
        1,
        enter_critical_section,
    ),
    stdcall(
        "KERNEL32",
        "LeaveCriticalSection",
        1,
        leave_critical_section,
    ),
    stdcall("KERNEL32", "DeleteCriticalSection", 1, critical_section),
    stdcall("KERNEL32", "TlsAlloc", 0, tls_alloc),
    stdcall("KERNEL32", "TlsSetValue", 2, tls_set),
    stdcall("KERNEL32", "TlsGetValue", 1, tls_get),
    stdcall("KERNEL32", "TlsFree", 1, tls_free),
    stdcall(
        "KERNEL32",
        "SetUnhandledExceptionFilter",
        1,
        set_unhandled_exception_filter,
    ),
    stdcall("KERNEL32", "GetCurrentProcess", 0, current_process),
    stdcall("KERNEL32", "GetCurrentProcessId", 0, current_process_id),
    stdcall("KERNEL32", "GetCurrentThreadId", 0, current_thread_id),
    stdcall("KERNEL32", "GetLastError", 0, get_last_error),
    stdcall("KERNEL32", "SetLastError", 1, set_last_error),
    stdcall("KERNEL32", "ExitProcess", 1, exit_process),
    stdcall("KERNEL32", "WideCharToMultiByte", 8, wide_to_multi),
    stdcall("KERNEL32", "MultiByteToWideChar", 6, multi_to_wide),
    stdcall("KERNEL32", "GetStringTypeA", 5, get_string_type_a),
    stdcall("KERNEL32", "GetStringTypeW", 4, get_string_type_w),
    stdcall("KERNEL32", "LCMapStringA", 6, lc_map_string),
    stdcall("KERNEL32", "LCMapStringW", 6, lc_map_string),
    stdcall("KERNEL32", "CreateMutexA", 3, create_mutex),
    stdcall("KERNEL32", "CloseHandle", 1, close_handle),
    stdcall("KERNEL32", "CreateEventA", 4, create_event),
    stdcall("KERNEL32", "SetEvent", 1, set_event),
    stdcall("KERNEL32", "ResetEvent", 1, reset_event),
    stdcall(
        "KERNEL32",
        "WaitForMultipleObjects",
        4,
        wait_for_multiple_objects,
    ),
    stdcall("KERNEL32", "WaitForSingleObject", 2, wait_for_single_object),
    stdcall("KERNEL32", "ReleaseMutex", 1, release_mutex),
    stdcall("KERNEL32", "CreateSemaphoreA", 4, create_semaphore),
    stdcall("KERNEL32", "ReleaseSemaphore", 3, release_semaphore),
    stdcall("KERNEL32", "GetCurrentDirectoryA", 2, get_current_directory),
    stdcall("KERNEL32", "SetCurrentDirectoryA", 1, set_current_directory),
    stdcall("KERNEL32", "CreateFileA", 7, create_file),
    stdcall("KERNEL32", "ReadFile", 5, read_file),
    stdcall("KERNEL32", "SetFilePointer", 4, set_file_pointer),
    stdcall("KERNEL32", "WriteFile", 5, write_file),
    stdcall("KERNEL32", "GetFileSize", 2, get_file_size),
    stdcall("KERNEL32", "SetEndOfFile", 1, set_end_of_file),
    stdcall("KERNEL32", "OpenFile", 3, open_file),
    stdcall("KERNEL32", "_hread", 3, hread),
    stdcall("KERNEL32", "_hwrite", 3, hwrite),
    stdcall("KERNEL32", "_llseek", 3, llseek),
    stdcall("KERNEL32", "_lclose", 1, lclose),
    stdcall("KERNEL32", "GetFileAttributesA", 1, get_file_attributes),
    stdcall("KERNEL32", "GetDriveTypeA", 1, get_drive_type),
    stdcall("KERNEL32", "GetLogicalDrives", 0, get_logical_drives),
    stdcall(
        "KERNEL32",
        "GetVolumeInformationA",
        8,
        get_volume_information,
    ),
    stdcall("KERNEL32", "GetTickCount", 0, get_tick_count),
    stdcall(
        "KERNEL32",
        "QueryPerformanceCounter",
        1,
        query_performance_counter,
    ),
    stdcall(
        "KERNEL32",
        "QueryPerformanceFrequency",
        1,
        query_performance_frequency,
    ),
    stdcall("KERNEL32", "GetSystemTime", 1, get_system_time),
    stdcall("KERNEL32", "GetLocalTime", 1, get_system_time),
    stdcall("KERNEL32", "Sleep", 1, sleep),
    stdcall("KERNEL32", "GetSystemDirectoryA", 2, get_system_directory),
    stdcall("KERNEL32", "LoadLibraryA", 1, load_library),
    stdcall("KERNEL32", "FreeLibrary", 1, free_library),
    stdcall("KERNEL32", "GetProcAddress", 2, get_proc_address),
    stdcall("KERNEL32", "FindFirstFileA", 2, find_first_file),
    stdcall("KERNEL32", "FindNextFileA", 2, find_next_file),
    stdcall("KERNEL32", "FindClose", 1, find_close),
    stdcall("KERNEL32", "DeleteFileA", 1, delete_file),
    stdcall("KERNEL32", "MoveFileA", 2, move_file),
    stdcall("KERNEL32", "CreateDirectoryA", 2, create_directory),
    stdcall("KERNEL32", "GetFullPathNameA", 4, get_full_path_name),
    stdcall("KERNEL32", "SetFileAttributesA", 2, set_file_attributes),
    stdcall("KERNEL32", "GlobalMemoryStatus", 1, global_memory_status),
    stdcall("KERNEL32", "GetComputerNameA", 2, get_computer_name),
    stdcall("KERNEL32", "GetProfileStringA", 5, get_profile_string),
    stdcall("KERNEL32", "CreateThread", 6, create_thread),
    stdcall("KERNEL32", "ResumeThread", 1, resume_thread),
    stdcall("KERNEL32", "SetThreadPriority", 2, set_thread_priority),
    stdcall("KERNEL32", "ExitThread", 1, exit_thread),
    stdcall("KERNEL32", "GetExitCodeThread", 2, get_exit_code_thread),
    stdcall("KERNEL32", "OutputDebugStringA", 1, output_debug_string),
];

fn set_thread_priority(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's threads are cooperative (ADR-0005): one runs
    // until it gives way, so what a title says it would rather have run first
    // is nothing to honour. Saying so is the answer — a title that is refused
    // here decides it is on a machine it cannot use.
    ctx.log.line(&format!(
        "thread {:#x} would rather run at priority {}",
        args[0], args[1] as i32
    ));
    Ok(1)
}

fn get_version(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.title.declared.windows.version)
}

fn get_version_ex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // OSVERSIONINFOA: size, major, minor, build, platform, then a 128-byte
    // string. Platform 1 is Windows 95 and its descendants.
    let at = args[0];
    let windows = &ctx.title.declared.windows;
    let (major, minor) = (windows.version & 0xff, (windows.version >> 8) & 0xff);
    let (build, csd) = (windows.build, windows.csd);
    for (offset, value) in [(4, major), (8, minor), (12, build), (16, 1)] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    ctx.write_string(at + 20, 128, csd).map_err(Stop::Fault)?;
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

/// Where the guest believes it was started from. A declared value: the
/// install tree's own path, and the title's own image, not the host's and
/// not the first title this machine ever ran (#58).
fn image_path(ctx: &Ctx<'_>) -> String {
    let image = ctx
        .title
        .image
        .path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("game.exe");
    format!("{}\\{image}", ctx.title.install_dir)
}

fn get_module_file_name(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = image_path(ctx);
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
    // a player double-clicking the game would produce. The path is the one
    // GetModuleFileNameA gives, the running title's own (#93).
    let line = format!("\"{}\"", image_path(ctx));
    let addr = ctx.intern(&line).map_err(Stop::Fault)?;
    ctx.state.command_line = Some(addr);
    Ok(addr)
}

fn get_command_line_w(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The same line, two bytes a character: what a C runtime asks for when it
    // was built to handle both.
    let addr = get_command_line(ctx, args)?;
    let line = ctx.string(addr).map_err(Stop::Fault)?;
    let wide = wide(format!("{line}\0").as_bytes());
    let at = ctx.state.heap.alloc(u32::try_from(wide.len()).unwrap_or(0));
    ctx.mem.write(at, &wide).map_err(Stop::Fault)?;
    Ok(at)
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

fn heap_create(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // One heap stands behind every handle: the guest cannot tell, because
    // every allocation it gets back is real memory it can write to.
    Ok(ctx.state.give(Object::Heap))
}

fn heap_destroy(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(ctx.state.close(args[0])))
}

pub(super) fn heap_alloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
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

/// Every page the bytes from `at` for `size` touch.
fn pages(at: u32, size: u32) -> impl Iterator<Item = u32> {
    let end = at.saturating_add(size).next_multiple_of(0x1000);
    ((at & !0xfff)..end).step_by(0x1000)
}

/// Write down the pages from `at` for `size` as committed, when the call
/// commits them.
fn commit(state: &mut super::State, kind: u32, at: u32, size: u32, protection: u32) {
    const MEM_COMMIT: u32 = 0x1000;
    if kind & MEM_COMMIT != 0 {
        for page in pages(at, size) {
            state.committed.insert(page, protection);
        }
    }
}

fn virtual_alloc(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // VirtualAlloc(address, size, type, protection).
    //
    // The declared machine's whole address space is already mapped, so
    // committing memory inside a region the guest already reserved is only
    // written down, for `VirtualQuery`: the address it asked about is the
    // address it gets back. Handing out new memory for each commit is how a
    // title that suballocates its own four megabytes eats a heap five hundred
    // megabytes wide.
    let (address, size, kind, protection) = (args[0], args[1], args[2], args[3]);
    if address != 0 {
        // Inside the heap the address is inside a region this machine already
        // handed out, and committing it again is nothing to do. Outside it the
        // address is a claim on free address space, and Windows refuses a
        // claim on space already taken: that refusal is how a guest's own DLL
        // loader learns it must relocate the image it is about to map, instead
        // of laying it over the one already there (#61).
        use crate::guest::heap::{HEAP_BASE, HEAP_BYTES};
        let end = address.saturating_add(size.max(1));
        if address >= HEAP_BASE && end <= HEAP_BASE + HEAP_BYTES {
            commit(ctx.state, kind, address, size, protection);
            return Ok(address);
        }
        if ctx
            .state
            .claimed
            .iter()
            .any(|(s, e)| address < *e && *s < end)
        {
            ctx.log.line(&format!(
                "address {address:#010x} is already taken: {size} bytes refused"
            ));
            return Ok(0);
        }
        ctx.state.claimed.push((address, end));
        ctx.state
            .reserved
            .insert(address, (size.next_multiple_of(0x1000), protection));
        commit(ctx.state, kind, address, size, protection);
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
        return Ok(0);
    }
    ctx.state
        .reserved
        .insert(at, (size.next_multiple_of(0x1000), protection));
    commit(ctx.state, kind, at, size, protection);
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
        ctx.state.claimed.retain(|(start, _)| *start != address);
        if let Some((size, _)) = ctx.state.reserved.remove(&address) {
            for page in pages(address, size) {
                ctx.state.committed.remove(&page);
            }
        }
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
        for page in pages(address, size) {
            ctx.state.committed.remove(&page);
        }
    }
    Ok(1)
}

/// What `VirtualAlloc` made of the page at an address, and of every page
/// after it that is the same: which region they are in, and whether they are
/// committed and how. SmartHeap, linked into LEGO Island's engine, asks how
/// much of a region it reserved is committed (#118).
fn virtual_query(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // VirtualQuery(address, buffer, length), into a MEMORY_BASIC_INFORMATION.
    const MEMORY_BASIC_INFORMATION: u32 = 28;
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_PRIVATE: u32 = 0x2_0000;
    let (address, buffer, length) = (args[0], args[1], args[2]);
    let region = ctx
        .state
        .reserved
        .range(..=address)
        .next_back()
        .map(|(base, (size, protection))| (*base, *size, *protection))
        .filter(|(base, size, _)| address - base < *size);
    let Some((base, size, reserved_with)) = region.filter(|_| length >= MEMORY_BASIC_INFORMATION)
    else {
        // Anywhere else is the machine's own memory, or nothing, and what
        // Windows says of either is not written yet.
        return Err(Stop::NotWritten {
            what: format!(
                "KERNEL32.dll!VirtualQuery({address:#x}) into {length} bytes, outside every \
                 region VirtualAlloc reserved"
            ),
            caller: 0,
        });
    };
    let page = address & !0xfff;
    let here = ctx.state.committed.get(&page).copied();
    let mut end = page + 0x1000;
    while end - base < size && ctx.state.committed.get(&end).copied() == here {
        end += 0x1000;
    }
    // A page that is only reserved has no protection of its own.
    let (state, protection) = here.map_or((MEM_RESERVE, 0), |p| (MEM_COMMIT, p));
    let answer = [
        page,
        base,
        reserved_with,
        end - page,
        state,
        protection,
        MEM_PRIVATE,
    ];
    for (at, value) in (buffer..).step_by(4).zip(answer) {
        ctx.mem.write_u32(at, value).map_err(Stop::Fault)?;
    }
    Ok(MEMORY_BASIC_INFORMATION)
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

/// The locale the declared machine is set to: US English, which is the one
/// every pressing on this roster was authored against. A run that read the
/// host's own would sort and format differently on somebody else's machine.
const LCID: u32 = 0x0409;

fn user_default_lcid(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(LCID)
}

fn user_default_lang_id(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(LCID)
}

/// The code pages the declared machine has, which are the two it answers with
/// from `GetACP` and `GetOEMCP`, plus the two names every Windows has.
fn is_valid_code_page(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(matches!(args[0], 0 | 1 | 2 | 3 | 437 | 1252)))
}

/// Whether the machine has a locale. It has the one it declares, and the
/// system and user defaults name it too.
fn declared_locale(lcid: u32) -> bool {
    matches!(lcid, LCID | 0x0400 | 0x0800 | 0x1000)
}

fn is_valid_locale(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::from(declared_locale(args[0])))
}

/// The pictures the declared locale writes a date and a time by, which
/// `GetLocaleInfoA` hands out and `GetDateFormatA` and `GetTimeFormatA` write
/// by (#87).
///
/// Windows 98's own for US English, which is what a player who never opened
/// Regional Settings had; that version still wrote its short date with two
/// digits of year.
const SHORT_DATE: &str = "M/d/yy";
const LONG_DATE: &str = "dddd, MMMM dd, yyyy";
const TIME: &str = "h:mm:ss tt";

/// What a locale is called and how it writes numbers, dates and money.
///
/// Only what a C runtime reads on its way up. A field this machine has not
/// declared answers as empty rather than as something invented, and the caller
/// then uses the C locale's own.
fn locale_field(what: u32) -> Option<&'static str> {
    Some(match what & 0xffff {
        0x0001 => "0409", // the locale itself, as the hexadecimal it is
        0x0002 => "ENU",  // its abbreviated name
        0x0003 => "0009", // the language
        0x0004 => "English (United States)",
        0x0005 => "US",
        0x0006 => "USA",
        0x0007 => "United States",
        0x000e => ".", // the decimal separator
        0x000f => ",", // and the thousands one
        0x0014 => "$",
        0x001d => "/", // how a date is written
        0x001e => ":", // and a time
        // The short date, the long one and the time (#87).
        0x001f => SHORT_DATE,
        0x0020 => LONG_DATE,
        0x1003 => TIME,
        0x1004 => "1252", // the code page it is written in
        _ => return None,
    })
}

fn get_locale_info_a(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetLocaleInfoA(locale, what, buffer, size).
    let Some(value) = locale_field(args[1]) else {
        // ERROR_INVALID_FLAGS: a field the declared machine does not carry.
        ctx.state.last_error = 1004;
        return Ok(0);
    };
    write_sized(ctx, format!("{value}\0").as_bytes(), 1, args[2], args[3])
}

fn get_locale_info_w(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let Some(value) = locale_field(args[1]) else {
        ctx.state.last_error = 1004;
        return Ok(0);
    };
    let wide = wide(format!("{value}\0").as_bytes());
    write_sized(ctx, &wide, 2, args[2], args[3])
}

/// A string handed back the way the locale's calls hand one back: a size of
/// nothing asks how much room it would take and writes nowhere, too little
/// room is `ERROR_INSUFFICIENT_BUFFER`, and the answer counts characters,
/// each `width` bytes, the terminator among them.
fn write_sized(
    ctx: &mut Ctx<'_>,
    bytes: &[u8],
    width: usize,
    at: u32,
    room: u32,
) -> Result<u32, Stop> {
    let needed = u32::try_from(bytes.len() / width).unwrap_or(0);
    if room == 0 {
        return Ok(needed);
    }
    if room < needed {
        ctx.state.last_error = 122; // ERROR_INSUFFICIENT_BUFFER
        return Ok(0);
    }
    ctx.mem.write(at, bytes).map_err(Stop::Fault)?;
    Ok(needed)
}

/// Text in the declared code page as two bytes a character, which for
/// everything these titles carry is each byte and a zero.
fn wide(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|b| [*b, 0]).collect()
}

/// Every locale the declared machine has, handed to the guest's own callback.
///
/// There is one, so the callback is called once, with the locale written the
/// way this call writes it: eight hexadecimal digits.
fn enum_system_locales(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let name = format!("{LCID:08x}\0");
    let addr = ctx.state.heap.alloc(u32::try_from(name.len()).unwrap_or(0));
    ctx.mem.write(addr, name.as_bytes()).map_err(Stop::Fault)?;
    ctx.call_guest = Some((args[0], vec![addr]));
    Ok(1)
}

/// Which day of the week a SYSTEMTIME's date falls on, Sunday first, or
/// nothing when the calendar has no such day.
///
/// Worked out from the date rather than read from the structure, because
/// that is what Windows does: a caller filling in a date by hand rarely
/// fills in its weekday too.
fn weekday(time: &[u16; 8]) -> Option<usize> {
    // Sakamoto's: the year counted from March, so the leap day falls last.
    const SHIFT: [usize; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let [year, month, _, day, ..] = *time;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let lengths = [
        31,
        28 + u16::from(leap),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let month = usize::from(month).checked_sub(1)?;
    // A FILETIME reaches from 1601 to 30827, and so does a date Windows will
    // write.
    if !(1601..=30827).contains(&year) || !(1..=*lengths.get(month)?).contains(&day) {
        return None;
    }
    let y = usize::from(year) - usize::from(month < 2);
    Some((y + y / 4 - y / 100 + y / 400 + SHIFT[month] + usize::from(day)) % 7)
}

/// `SHORT_DATE`, `M/d/yy`, written out.
fn short_date([year, month, _, day, ..]: [u16; 8]) -> String {
    format!("{month}/{day}/{:02}", year % 100)
}

/// `TIME` without its seconds, `h:mm tt`, as `TIME_NOSECONDS` writes it.
fn time_without_seconds([_, _, _, _, hour, minute, ..]: [u16; 8]) -> String {
    let marker = if hour < 12 { "AM" } else { "PM" };
    format!("{}:{minute:02} {marker}", (hour + 11) % 12 + 1)
}

/// What `GetDateFormatA` and `GetTimeFormatA` share once each knows how it
/// writes: the locale, the moment, and the caller's buffer.
fn write_formatted(
    ctx: &mut Ctx<'_>,
    args: &[u32],
    date: bool,
    write: fn([u16; 8]) -> String,
) -> Result<u32, Stop> {
    // (locale, flags, moment, picture, buffer, size). No moment is now, read
    // off the declared clock like `GetLocalTime`, so a replay writes the same.
    let time: [u16; 8] = if args[2] == 0 {
        declared_now(ctx.declared_millis())
    } else {
        let bytes = ctx.mem.slice(args[2], 16).map_err(Stop::Fault)?;
        std::array::from_fn(|i| u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]))
    };
    let [_, _, _, _, hour, minute, second, _] = time;
    let valid = if date {
        weekday(&time).is_some()
    } else {
        hour < 24 && minute < 60 && second < 60
    };
    if !declared_locale(args[0]) || !valid {
        ctx.state.last_error = 87; // ERROR_INVALID_PARAMETER
        return Ok(0);
    }
    let text = write(time);
    ctx.log.line(&format!(
        "{} written {text:?} for locale {:#x} flags {:#x}",
        if date { "date" } else { "time" },
        args[0],
        args[1]
    ));
    write_sized(ctx, format!("{text}\0").as_bytes(), 1, args[4], args[5])
}

/// The one way a title on the roster asks for a date or a time to be written:
/// these flags and no picture of its own. Anything else is a wall naming it.
fn asked_as(args: &[u32], flags: u32, name: &str) -> Result<(), Stop> {
    // (locale, flags, moment, picture, buffer, size).
    if (args[1], args[3]) == (flags, 0) {
        return Ok(());
    }
    Err(Stop::NotWritten {
        what: format!(
            "KERNEL32.dll!{name}: flags {:#x}, picture {:#010x}",
            args[1], args[3]
        ),
        caller: 0,
    })
}

/// A date, written the way the declared locale writes one (#87).
///
/// Alpha Team's `Localize::SecondsToDateTime` asks with `DATE_SHORTDATE` and
/// no picture of its own, to label a saved adventure on its Select Adventure
/// screen.
fn get_date_format_a(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    asked_as(args, 0x1, "GetDateFormatA")?;
    write_formatted(ctx, args, true, short_date)
}

/// A time of day, written the way the declared locale writes one (#87).
///
/// Alpha Team asks with `TIME_NOSECONDS` and no picture of its own, beside the
/// date `GetDateFormatA` wrote.
fn get_time_format_a(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    asked_as(args, 0x2, "GetTimeFormatA")?;
    write_formatted(ctx, args, false, time_without_seconds)
}

/// How two strings sort against each other. One is less, two is equal, three
/// is greater; zero is a failure.
fn compare(left: &[u8], right: &[u8], flags: u32) -> u32 {
    let fold = |s: &[u8]| -> Vec<u8> {
        if flags & 0x0000_0001 == 0 {
            s.to_vec()
        } else {
            s.to_ascii_lowercase()
        }
    };
    match fold(left).cmp(&fold(right)) {
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Equal => 2,
        std::cmp::Ordering::Greater => 3,
    }
}

fn compare_string_a(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CompareStringA(locale, flags, left, left len, right, right len).
    let left = byte_len(ctx, args[2], args[3])?;
    let left = ctx.mem.slice(args[2], left).map_err(Stop::Fault)?.to_vec();
    let right = byte_len(ctx, args[4], args[5])?;
    let right = ctx.mem.slice(args[4], right).map_err(Stop::Fault)?.to_vec();
    Ok(compare(&left, &right, args[1]))
}

fn compare_string_w(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let read = |ctx: &mut Ctx<'_>, at: u32, count: u32| -> Result<Vec<u8>, Stop> {
        if count == 0xffff_ffff {
            return Ok(ctx.wide_string(at).map_err(Stop::Fault)?.into_bytes());
        }
        let bytes = ctx
            .mem
            .slice(at, count as usize * 2)
            .map_err(Stop::Fault)?
            .to_vec();
        Ok(bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .map(|u| u8::try_from(u).unwrap_or(b'?'))
            .collect())
    };
    let left = read(ctx, args[2], args[3])?;
    let right = read(ctx, args[4], args[5])?;
    Ok(compare(&left, &right, args[1]))
}

/// One variable out of the declared environment.
fn get_environment_variable(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetEnvironmentVariableA(name, buffer, size). The environment is the
    // declared one: nothing the host was started with reaches the guest.
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    let found = ENVIRONMENT
        .split('\0')
        .filter_map(|row| row.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case(&name))
        .map(|(_, value)| value.to_owned());
    let Some(value) = found else {
        // ERROR_ENVVAR_NOT_FOUND: a variable that is not set is not an empty
        // one, and a runtime asking for TZ has to hear the difference.
        ctx.state.last_error = 203;
        return Ok(0);
    };
    let needed = u32::try_from(value.len()).unwrap_or(0);
    if args[1] == 0 || args[2] <= needed {
        // What it would take to hold it, the terminator included.
        return Ok(needed + 1);
    }
    let mut bytes = value.into_bytes();
    bytes.push(0);
    ctx.mem.write(args[1], &bytes).map_err(Stop::Fault)?;
    Ok(needed)
}

/// The declared moment `GetSystemTime` answers with, counted the other way.
fn get_system_time_as_file_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let ticks = declared_file_time(ctx.declared_millis());
    ctx.mem
        .write(args[0], &ticks.to_le_bytes())
        .map_err(Stop::Fault)?;
    Ok(0)
}

/// Guest threads switch only between bursts of instructions and at waits,
/// never inside a call across the boundary (ADR-0005), so the read and the
/// write cannot be interrupted.
fn interlocked_increment(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let value = ctx
        .mem
        .read_u32(args[0])
        .map_err(Stop::Fault)?
        .wrapping_add(1);
    ctx.mem.write_u32(args[0], value).map_err(Stop::Fault)?;
    Ok(value)
}

fn interlocked_decrement(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let value = ctx
        .mem
        .read_u32(args[0])
        .map_err(Stop::Fault)?
        .wrapping_sub(1);
    ctx.mem.write_u32(args[0], value).map_err(Stop::Fault)?;
    Ok(value)
}

fn interlocked_exchange(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let was = ctx.mem.read_u32(args[0]).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], args[1]).map_err(Stop::Fault)?;
    Ok(was)
}

/// Whether a pointer is one the guest can use. Zero means it can.
fn is_bad_read_ptr(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The arena is the whole of what the guest owns, so the question is
    // whether the span is inside it. A length of zero is always fine, which is
    // what Windows says too.
    let ok = args[1] == 0 || (args[0] != 0 && ctx.mem.slice(args[0], args[1] as usize).is_ok());
    Ok(u32::from(!ok))
}

fn is_bad_code_ptr(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let ok = args[0] != 0 && ctx.mem.slice(args[0], 1).is_ok();
    Ok(u32::from(!ok))
}

/// A module saying it does not want to hear about threads starting and ending.
/// The machine tells a module only of its process attaching, never of a
/// thread, so there is nothing to stop telling it.
fn disable_thread_library_calls(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

/// Which failures the machine hands back rather than showing a person. It is
/// read when something looks on the empty floppy drive (#145), where Windows
/// would otherwise put up its box asking for a disk.
fn set_error_mode(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(std::mem::replace(&mut ctx.state.error_mode, args[0]))
}

/// Where the standard handles go. The declared machine has no console, and a
/// runtime redirecting its own output is told the change was made.
fn set_std_handle(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

/// Everything the guest writes is already through the tree, which writes it
/// when it is written: there is nothing held back to flush.
fn flush_file_buffers(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

/// The declared machine, as SYSTEM_INFO describes one.
fn get_system_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Declared values, every one: a run that read the host's own processor
    // count or page size would replay differently on another machine.
    let at = args[0];
    let (kind, level) = system_processor(ctx.title.declared.processor);
    let fields: [(u32, u32); 9] = [
        (0x00, 0),           // x86, and no reserved word
        (0x04, 0x0000_1000), // a four kilobyte page
        (0x08, 0x0001_0000), // the lowest address an application gets
        (0x0c, 0x7fff_ffff), // and the highest
        (0x10, 1),           // one processor, which is the mask of it
        (0x14, 1),           // and one of them
        (0x18, kind),        // what Windows calls the declared processor
        (0x1c, 0x0001_0000), // images are handed out this far apart
        (0x20, level),       // and its level, with the revision above it
    ];
    for (offset, value) in fields {
        ctx.mem
            .write(at + offset, &value.to_le_bytes())
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

/// The declared processor as SYSTEM_INFO names it, from what its own `cpuid`
/// says it is (#93, #104): the type, and the level with the revision above it.
///
/// Windows calls every Pentium, the Pentium II among them,
/// PROCESSOR_INTEL_PENTIUM. The level is the family, and the revision the
/// model and the stepping. An Intel386 cannot be asked, and only the
/// conformance corpus has one.
fn system_processor(processor: crate::cpu::Processor) -> (u32, u32) {
    use crate::cpu::Processor;
    let kind = match processor {
        Processor::I386 => 386,
        Processor::Pentium | Processor::PentiumII => 586,
    };
    let [signature, ..] = crate::cpu::exec::cpuid(processor, 1);
    let (family, model, stepping) = (signature >> 8 & 0xf, signature >> 4 & 0xf, signature & 0xf);
    (kind, family | model << 24 | stepping << 16)
}

/// A process ending itself the hard way, which on this machine is the only
/// process there is.
fn terminate_process(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.log
        .line(&format!("guest terminated itself {}", args[1]));
    Err(Stop::Exited { code: args[1] })
}

/// The pseudo-handle standing for whichever thread is asking, as Windows hands
/// it out.
fn get_current_thread(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0xffff_fffe)
}

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
    let wide = wide(ENVIRONMENT.as_bytes());
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
    // Making one and taking it apart again: the structure the guest passes is
    // its own memory and is left exactly as it found it.
    Ok(0)
}

/// Which thread is asking, as the sections know threads.
pub(super) fn running_thread(ctx: &Ctx<'_>) -> u32 {
    ctx.state
        .threads
        .get(ctx.state.thread)
        .map_or(crate::machine::THREAD_ID, |t| t.id)
}

/// Take a lock, or wait for whoever has it (ADR-0005).
///
/// A title with one thread takes every section at once. With more it is what
/// keeps them out of each other's C++ runtime: Alpha Team's sound thread and
/// its loader share one allocator.
fn enter_critical_section(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let me = running_thread(ctx);
    match ctx.state.sections.get_mut(&args[0]) {
        Some((owner, depth)) if *owner == me => {
            // A thread already holding one may take it again, and has to
            // leave it as many times as it took it.
            *depth += 1;
            park(ctx, &[], 0);
            Ok(0)
        }
        Some(_) => {
            // Somebody else has it. This thread is put down on it until they
            // leave, rather than left runnable to ask again: a thread that
            // asks again retires no instructions, so the declared clock never
            // moves, so whoever holds the section never reaches its hour
            // either — the machine spins with every thread alive and nothing
            // happening (#60).
            park(ctx, &[args[0]], u64::MAX);
            Err(Stop::Blocked)
        }
        None => {
            ctx.state.sections.insert(args[0], (me, 1));
            park(ctx, &[], 0);
            Ok(0)
        }
    }
}

fn leave_critical_section(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let me = running_thread(ctx);
    if let Some((owner, depth)) = ctx.state.sections.get_mut(&args[0]) {
        if *owner == me {
            *depth -= 1;
            if *depth == 0 {
                ctx.state.sections.remove(&args[0]);
                wake_waiters(ctx.state, args[0]);
            }
        }
    }
    Ok(0)
}

fn tls_alloc(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    let index = u32::try_from(ctx.state.tls_slots).unwrap_or(0);
    ctx.state.tls_slots += 1;
    Ok(index)
}

fn tls_set(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Per thread, which is the whole point of them: a C runtime keeps its own
    // per-thread block in one of these, and two threads sharing a slot would
    // be two threads sharing that block (ADR-0005).
    let slot = args[0] as usize;
    if slot >= ctx.state.tls_slots {
        return Ok(0);
    }
    let running = ctx.state.thread;
    let Some(thread) = ctx.state.threads.get_mut(running) else {
        return Ok(0);
    };
    if thread.tls.len() <= slot {
        thread.tls.resize(slot + 1, 0);
    }
    thread.tls[slot] = args[1];
    Ok(1)
}

fn tls_get(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let running = ctx.state.thread;
    Ok(ctx
        .state
        .threads
        .get(running)
        .and_then(|t| t.tls.get(args[0] as usize))
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

fn current_thread_id(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Whichever thread is running, which is the only one until a title makes
    // another (ADR-0005).
    Ok(running_thread(ctx))
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
    ctx.mem
        .write(dst, &wide(&bytes[..take]))
        .map_err(Stop::Fault)?;
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
    // CreateMutexA(security, take it now, name). A single-instance check makes
    // a named one and asks whether it already existed; on this machine it
    // never has, because one run is one machine. What it is *for* here is two
    // of this machine's own threads keeping each other out of one thing at a
    // time — A3D's sound threads and the title's own (#60).
    let name = ctx.string(args[2]).map_err(Stop::Fault)?;
    let me = running_thread(ctx);
    ctx.state.last_error = 0;
    let mine = args[1] != 0;
    Ok(ctx.state.give(Object::Named {
        kind: "mutex",
        name,
        signalled: !mine,
        manual: false,
        held: mine.then_some((me, 1)),
    }))
}

fn release_mutex(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let me = running_thread(ctx);
    let Some(Object::Named {
        signalled, held, ..
    }) = ctx.state.object_mut(args[0])
    else {
        ctx.state.last_error = 6;
        return Ok(0);
    };
    // Only whoever holds it may let it go, and only the outermost release
    // frees it: a thread may take one it already holds.
    match held {
        Some((owner, depth)) if *owner == me => {
            *depth -= 1;
            if *depth == 0 {
                *held = None;
                *signalled = true;
                wake_waiters(ctx.state, args[0]);
            }
            Ok(1)
        }
        _ => {
            // ERROR_NOT_OWNER.
            ctx.state.last_error = 288;
            Ok(0)
        }
    }
}

fn create_event(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateEventA(security, manual reset, initial state, name). A manual one
    // stays up until it is reset; an automatic one wakes one waiter and goes
    // back down as that waiter takes it.
    let name = ctx.string(args[3]).map_err(Stop::Fault)?;
    ctx.state.last_error = 0;
    Ok(ctx.state.give(Object::Named {
        kind: "event",
        name,
        signalled: args[2] != 0,
        manual: args[1] != 0,
        held: None,
    }))
}

fn set_event(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let Some(Object::Named { signalled, .. }) = ctx.state.object_mut(args[0]) else {
        ctx.state.last_error = 6;
        return Ok(0);
    };
    *signalled = true;
    // Whoever is parked on it may run now rather than when its timeout runs
    // out: an event nobody remembers having set is an event nobody can be
    // woken by, which is what kept A3D's threads spinning (#60).
    wake_waiters(ctx.state, args[0]);
    Ok(1)
}

fn reset_event(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let Some(Object::Named { signalled, .. }) = ctx.state.object_mut(args[0]) else {
        ctx.state.last_error = 6;
        return Ok(0);
    };
    *signalled = false;
    Ok(1)
}

fn create_semaphore(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateSemaphoreA(security, initial count, maximum count, name). LEGO
    // Island's engine makes them unnamed, to hand work between its own
    // threads (#115); a named one is a wall until a title asks for one.
    if args[3] != 0 {
        return Err(Stop::NotWritten {
            what: "KERNEL32.dll!CreateSemaphoreA: a named semaphore".to_owned(),
            caller: 0,
        });
    }
    // Both counts are signed: the maximum must be above zero, and the
    // initial count no less than zero and no more than the maximum.
    let (initial, max) = (args[1] as i32, args[2] as i32);
    if max <= 0 || initial < 0 || initial > max {
        ctx.state.last_error = 87; // ERROR_INVALID_PARAMETER
        return Ok(0);
    }
    ctx.state.last_error = 0;
    Ok(ctx.state.give(Object::Semaphore {
        count: args[1],
        max: args[2],
    }))
}

fn release_semaphore(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // ReleaseSemaphore(handle, by how many, out the count before).
    let Some(Object::Semaphore { count, max }) = ctx.state.object_mut(args[0]) else {
        ctx.state.last_error = 6;
        return Ok(0);
    };
    // The count to add is signed, and must be above zero.
    if args[1] as i32 <= 0 {
        ctx.state.last_error = 87; // ERROR_INVALID_PARAMETER
        return Ok(0);
    }
    // A release past the maximum fails and changes nothing, the count before
    // included.
    let was = *count;
    if args[1] > *max - was {
        ctx.state.last_error = 298; // ERROR_TOO_MANY_POSTS
        return Ok(0);
    }
    *count = was + args[1];
    if args[2] != 0 {
        ctx.mem.write_u32(args[2], was).map_err(Stop::Fault)?;
    }
    wake_waiters(ctx.state, args[0]);
    Ok(1)
}

/// Let every thread parked on this object have its turn again.
///
/// It does not hand the object to any of them: each makes its wait again when
/// the machine comes back round to it, and whoever gets there first takes it.
fn wake_waiters(state: &mut super::State, handle: u32) {
    for thread in &mut state.threads {
        if thread.waiting_on.contains(&handle) {
            thread.wake_at = 0;
        }
    }
}

/// Whether the running thread could take the object now, without taking it.
///
/// An event or a thread is there while it is signalled, a semaphore while it
/// has any of its count, and a mutex while nobody else holds it.
fn ready(ctx: &Ctx<'_>, handle: u32) -> bool {
    match ctx.state.object(handle) {
        Some(Object::Semaphore { count, .. }) => *count > 0,
        Some(Object::Named {
            kind: "mutex",
            held,
            ..
        }) => held.is_none_or(|(owner, _)| owner == running_thread(ctx)),
        Some(Object::Named { signalled, .. }) => *signalled,
        _ => false,
    }
}

/// Take the object if it is there to be taken (#60).
///
/// An automatic event goes back down as its waiter takes it; a manual one
/// stays up. A mutex is taken by the thread that waited, and a thread that
/// already holds one takes it again. A semaphore gives one of its count.
fn claim(ctx: &mut Ctx<'_>, handle: u32) -> bool {
    if !ready(ctx, handle) {
        return false;
    }
    let me = running_thread(ctx);
    match ctx.state.object_mut(handle) {
        Some(Object::Semaphore { count, .. }) => *count -= 1,
        Some(Object::Named {
            kind: "mutex",
            signalled,
            held,
            ..
        }) => {
            *held = Some((me, held.map_or(1, |(_, depth)| depth + 1)));
            *signalled = false;
        }
        Some(Object::Named {
            signalled,
            manual: false,
            ..
        }) => *signalled = false,
        _ => {}
    }
    true
}

/// WAIT_TIMEOUT: the wait ran out before what it waited for happened.
const WAIT_TIMEOUT: u32 = 0x0000_0102;
/// WAIT_FAILED: not a handle this machine ever gave out.
const WAIT_FAILED: u32 = 0xffff_ffff;
/// INFINITE, the timeout that is not one.
const INFINITE: u32 = 0xffff_ffff;

fn wait_for_single_object(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    wait_for(ctx, &[args[0]], false, args[1])
}

fn wait_for_multiple_objects(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // WaitForMultipleObjects(count, handles, all of them, timeout).
    let count = args[0] as usize;
    let mut handles = Vec::with_capacity(count);
    for i in 0..count {
        handles.push(
            ctx.mem
                .read_u32(args[1] + (i as u32) * 4)
                .map_err(Stop::Fault)?,
        );
    }
    let all = args[2] != 0;
    // A wait for all of them may not name one twice, as Windows refuses it: a
    // semaphore's last one or an automatic event's signal cannot be taken twice.
    if all && (1..count).any(|i| handles[..i].contains(&handles[i])) {
        ctx.state.last_error = 87; // ERROR_INVALID_PARAMETER
        park(ctx, &[], 0);
        return Ok(WAIT_FAILED);
    }
    wait_for(ctx, &handles, all, args[3])
}

/// Wait for any one of these objects, or for all of them, until the timeout.
///
/// The thread is parked rather than answered at once: a wait that is over the
/// moment it is asked for turns a paced loop into a spin. Each object keeps
/// whether it is signalled, which is what wakes a waiter (#60). The call itself
/// is left where it is — `Stop::Blocked` brings this thread back to it when
/// the machine comes round again, so the wait costs nothing while it waits.
///
/// A wait for any answers the first in the list it can take, by its place. A
/// wait for all takes none of them until it can take every one, as Windows
/// does, and then answers the first place (#130).
fn wait_for(ctx: &mut Ctx<'_>, handles: &[u32], all: bool, timeout: u32) -> Result<u32, Stop> {
    if handles.is_empty() || handles.iter().any(|h| ctx.state.object(*h).is_none()) {
        // ERROR_INVALID_HANDLE.
        ctx.state.last_error = 6;
        park(ctx, &[], 0);
        return Ok(WAIT_FAILED);
    }
    if all {
        if handles.iter().all(|h| ready(ctx, *h)) {
            for handle in handles {
                claim(ctx, *handle);
            }
            park(ctx, &[], 0);
            return Ok(0);
        }
    } else if let Some(i) = handles.iter().position(|h| claim(ctx, *h)) {
        park(ctx, &[], 0);
        return Ok(u32::try_from(i).unwrap_or(0));
    }
    // Not available. Whether this thread can be woken at all decides what a
    // wait means: with nobody else to run, waiting is only spending time.
    let now = ctx.clock_millis;
    let running = ctx.state.thread;
    let parked = ctx
        .state
        .threads
        .get(running)
        .is_some_and(|t| !t.waiting_on.is_empty());
    if ctx.state.threads.len() < 2 {
        // There is nobody to be waiting for. A title with one thread waits on
        // an event a device would have signalled, or a mutex nothing else
        // could be holding; on this machine the device is the machine, so the
        // wait is over as soon as it is asked for. Hot Wheels' race is paced
        // by one of these and stops dead if it is told the wait ran out.
        return Ok(0);
    }
    if !parked {
        if timeout == 0 {
            return Ok(WAIT_TIMEOUT);
        }
        let until = if timeout == INFINITE {
            u64::MAX
        } else {
            now + u64::from(timeout)
        };
        park(ctx, handles, until);
        return Err(Stop::Blocked);
    }
    // Parked already: back for another look.
    if ctx.state.threads[running].wake_at > now {
        return Err(Stop::Blocked);
    }
    // Woken, and not by enough: another waiter took what woke it first (a
    // semaphore released once wakes everyone parked on it, and only one of
    // them gets the count), or one of a wait for all was signalled and not
    // the rest. A wait whose hour has not come goes back down until it does
    // (#115, #130).
    let until = ctx.state.threads[running].gives_up_at;
    if until > now {
        park(ctx, handles, until);
        return Err(Stop::Blocked);
    }
    park(ctx, &[], 0);
    Ok(WAIT_TIMEOUT)
}

/// Put the running thread down on these objects until an hour, or pick it
/// back up again with none.
fn park(ctx: &mut Ctx<'_>, handles: &[u32], until: u64) {
    let running = ctx.state.thread;
    if let Some(thread) = ctx.state.threads.get_mut(running) {
        thread.waiting_on.clear();
        thread.waiting_on.extend_from_slice(handles);
        thread.wake_at = if handles.is_empty() { 0 } else { until };
        thread.gives_up_at = thread.wake_at;
    }
}

fn close_handle(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Letting go of a file the guest wrote is what puts it in the profile,
    // where the next run will read it back: a save outlives its run.
    if let Some(Object::File {
        path,
        contents: Contents::Held(bytes),
        wrote: true,
        ..
    }) = ctx.state.object_mut(args[0])
    {
        let (path, bytes) = (path.clone(), std::mem::take(bytes));
        match ctx.tree.write(&path, &bytes) {
            Ok(at) => ctx.log.line(&format!(
                "file written {path}, {} bytes, to {}",
                bytes.len(),
                at.display()
            )),
            // The guest is still told the handle closed, because changing its
            // answer would change what the title does; the person is told
            // instead, now and when the run ends.
            Err(e) => {
                ctx.log.line(&format!("file write refused {path}: {e}"));
                eprintln!("lego-legacy: {path} was not saved: {e}");
                ctx.state.saves_refused += 1;
            }
        }
    }
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
    // SetCurrentDirectoryA(path), resolved as `GetFullPathNameA` resolves a
    // name. A relative one is the current directory's child, one rooted by
    // either separator is on the current drive, a trailing separator goes and
    // `.` names where it is, as Wine's conformance test `test_setdir` (checks
    // 1, 6 to 9 and 11) finds Windows doing it. Separators of either kind are
    // one, however many, as Microsoft's "File path formats on Windows systems"
    // says `GetFullPathName` makes them. A directory the machine does not have
    // fails with `ERROR_FILE_NOT_FOUND`, or `ERROR_PATH_NOT_FOUND` when the one
    // it would be in is missing too, as Wine's `test_SetCurrentDirectoryA`
    // finds Windows failing it. A name of nothing but white space has no full
    // path (`test_GetFullPathNameA`, and Wine's `get_full_path_helper`), and
    // fails with `ERROR_INVALID_NAME`, as Wine's `RtlSetCurrentDirectory_U`,
    // read as documentation, fails it: Windows NT's answer, Windows 98's being
    // unmeasured. Whenever it fails the directory stays. LEGO Chess sets an
    // empty one at boot, and its own folders by their names (#171).
    // ponytail: only this call collapses separators and drops `.`; moving that
    // into `Ctx::absolute` would answer every file call the same way, and move
    // what `GetFullPathNameA` hands every title back.
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    let wall = |why: &str| -> Result<u32, Stop> {
        Err(Stop::NotWritten {
            what: format!("KERNEL32.dll!SetCurrentDirectoryA({path:?}): {why}"),
            caller: 0,
        })
    };
    let name = path.trim();
    if name.is_empty() {
        ctx.state.last_error = ERROR_INVALID_NAME;
        return Ok(0);
    }
    if name.len() != path.len() {
        return wall("a name with white space around it");
    }
    if ctx.refuse_on_empty_floppy(name)? {
        return Ok(0);
    }
    let separator = |c: char| c == '\\' || c == '/';
    if name.chars().take(2).filter(|&c| separator(c)).count() == 2 {
        return wall("another machine's name");
    }
    let full = ctx.absolute(name);
    let (Some(drive), Some(rest)) = (full.get(..2), full.get(2..)) else {
        return wall("a drive's own current directory");
    };
    if !drive.ends_with(':') || !rest.starts_with(separator) {
        return wall("a drive's own current directory");
    }
    if ctx.drive_type(drive.as_bytes()[0]) == super::DRIVE_NO_ROOT_DIR {
        return wall("a drive the machine does not have");
    }
    let mut parts = Vec::new();
    for part in rest.split(separator).filter(|part| !part.is_empty()) {
        match part {
            "." => {}
            ".." => return wall("a name through `..`"),
            part => parts.push(part),
        }
    }
    let resolved = format!("{drive}\\{}", parts.join("\\"));
    if ctx.tree.readable(&resolved) {
        return wall("the name of a file");
    }
    if !ctx.tree.has_directory(&resolved) {
        let parent = resolved
            .rsplit_once('\\')
            .map_or(drive, |(parent, _)| parent);
        ctx.state.last_error = if ctx.tree.has_directory(parent) {
            ERROR_FILE_NOT_FOUND
        } else {
            ERROR_PATH_NOT_FOUND
        };
        return Ok(0);
    }
    ctx.state.current_directory = resolved;
    Ok(1)
}

/// What a call that could not open a file hands back.
const INVALID_HANDLE: u32 = 0xffff_ffff;
const ERROR_FILE_NOT_FOUND: u32 = 2;
/// A directory on the way to the one named is not there.
const ERROR_PATH_NOT_FOUND: u32 = 3;
/// A name that names nothing, such as an empty one.
const ERROR_INVALID_NAME: u32 = 123;

/// The access bits that make a handle one the guest can read and write
/// through, and the sharing bits that let another handle on the same file do
/// the same.
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;

/// What a title says it wants done with a file that is or is not there.
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const TRUNCATE_EXISTING: u32 = 5;

/// The file is already there, and the disc cannot be written to.
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_WRITE_PROTECT: u32 = 19;
/// A seek to before the start of a file, which is nowhere.
const ERROR_NEGATIVE_SEEK: u32 = 131;
/// A handle that cannot be written through.
const ERROR_ACCESS_DENIED: u32 = 5;
/// A handle that is not one, or not one of the kind asked about.
const ERROR_INVALID_HANDLE: u32 = 6;
/// A file another handle has open and does not share as this one asks.
const ERROR_SHARING_VIOLATION: u32 = 32;
const ERROR_INVALID_PARAMETER: u32 = 87;

fn create_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateFileA(path, access, share, security, disposition, flags, template).
    let named = ctx.string(args[0]).map_err(Stop::Fault)?;
    let path = ctx.absolute(&named);
    open(ctx, path, args[1], args[2], args[4])
}

/// Whether a file can be opened again, for `access` and sharing as `share`
/// says, while other handles have it open: each of them has to share what the
/// new one does, and the new one has to share what each of them does.
/// Windows' rule, both ways round.
fn shared(state: &State, path: &str, access: u32, share: u32) -> bool {
    let allows = |share: u32, access: u32| {
        (access & GENERIC_READ == 0 || share & FILE_SHARE_READ != 0)
            && (access & GENERIC_WRITE == 0 || share & FILE_SHARE_WRITE != 0)
    };
    let same = |open: &str| {
        open.replace('/', "\\")
            .eq_ignore_ascii_case(&path.replace('/', "\\"))
    };
    state.handles.values().all(|object| match object {
        Object::File {
            path: open,
            access: theirs,
            share: shares,
            ..
        } if same(open) => allows(*shares, access) && allows(share, *theirs),
        _ => true,
    })
}

/// Open a file for the guest: what `CreateFileA` does, and `OpenFile` too.
fn open(
    ctx: &mut Ctx<'_>,
    path: String,
    access: u32,
    share: u32,
    disposition: u32,
) -> Result<u32, Stop> {
    let writing = access & GENERIC_WRITE != 0;

    if ctx.refuse_on_empty_floppy(&path)? {
        return Ok(INVALID_HANDLE);
    }

    // The declared machine's D: is a CD-ROM, and a disc is not written to.
    if writing && path.to_ascii_uppercase().starts_with("D:") {
        ctx.log
            .line(&format!("write refused, D: is a disc: {path}"));
        ctx.state.last_error = ERROR_WRITE_PROTECT;
        return Ok(INVALID_HANDLE);
    }

    if !shared(ctx.state, &path, access, share) {
        ctx.state.last_error = ERROR_SHARING_VIOLATION;
        return Ok(INVALID_HANDLE);
    }

    // A title about to make a file asks whether one is there already. That is
    // not the pressing failing to hold something, so it is asked without
    // logging a miss: the run log's `file missing` lines are how the tree
    // learns what it has to ship, and a player's own save is not something it
    // ships (#44). OPEN_ALWAYS keeps what is there, so it reads.
    let making = writing && matches!(disposition, CREATE_NEW | CREATE_ALWAYS | TRUNCATE_EXISTING);
    // A file only read is read where it lies on the pressing, as the guest
    // asks for it, rather than held whole (#116).
    let lies = if writing { None } else { ctx.tree.lies(&path) };
    let there = if making {
        ctx.guest_file_is_there(&path)
            .then(|| Contents::Held(Vec::new()))
    } else if let Some((start, len)) = lies {
        Some(Contents::Lies { start, len })
    } else {
        ctx.open_guest_file(&path).map(Contents::Held)
    };
    let contents = match (there, disposition) {
        // CREATE_NEW is a title asking for a file that is not there yet, and
        // it has to be told when one is.
        (Some(_), CREATE_NEW) if writing => {
            ctx.state.last_error = ERROR_FILE_EXISTS;
            return Ok(INVALID_HANDLE);
        }
        // What was there is thrown away, whatever it was.
        (Some(_), CREATE_ALWAYS | TRUNCATE_EXISTING) if writing => Contents::Held(Vec::new()),
        (Some(contents), _) => contents,
        // Nothing there: only a title that says it is making one gets a
        // handle, which leaves TRUNCATE_EXISTING and OPEN_EXISTING refused.
        (None, CREATE_NEW | CREATE_ALWAYS | OPEN_ALWAYS) if writing => Contents::Held(Vec::new()),
        (None, _) => {
            // Recorded rather than passed over: what a title opens is how the
            // declared install tree learns what it has to hold, and who asked
            // is how a name nobody can explain is chased back to the code that
            // built it.
            let caller = ctx
                .mem
                .read_u32(ctx.cpu.r[crate::cpu::ESP])
                .unwrap_or_default();
            ctx.log
                .line(&format!("file missing {path}, asked from {caller:#010x}"));
            ctx.state.last_error = ERROR_FILE_NOT_FOUND;
            return Ok(INVALID_HANDLE);
        }
    };
    ctx.log
        .at(crate::cli::Trace::Shims, || format!("file opened {path}"));
    ctx.state.last_error = 0;
    Ok(ctx.state.give(Object::File {
        path,
        contents,
        at: 0,
        access,
        share,
        wrote: false,
    }))
}

/// `len` bytes of the pressing's image from `start`, for a file that lies
/// there, or the wall a host that can no longer read the image is.
fn lying(ctx: &mut Ctx<'_>, path: &str, start: u64, len: u32) -> Result<Vec<u8>, Stop> {
    ctx.tree
        .read_at(start, len as usize)
        .ok_or_else(|| Stop::NotWritten {
            what: format!("{path}: the pressing's image could not be read at byte {start}"),
            caller: 0,
        })
}

/// Read from an open file into the guest's buffer, from where the handle is
/// up to and as much of `want` as the file holds from there, and move the
/// handle on past it: what `ReadFile` and `_hread` both do. How much it read,
/// or nothing when the handle is not a file.
fn read(ctx: &mut Ctx<'_>, handle: u32, buffer: u32, want: u32) -> Result<Option<u32>, Stop> {
    let Some(Object::File {
        path, contents, at, ..
    }) = ctx.state.object(handle)
    else {
        ctx.state.last_error = ERROR_INVALID_HANDLE;
        return Ok(None);
    };
    // A handle past the end of its file reads nothing there.
    let (from, n) = (*at, contents.size().saturating_sub(*at).min(want));
    let chunk = match contents {
        _ if n == 0 => Vec::new(),
        Contents::Held(bytes) => bytes[from as usize..(from + n) as usize].to_vec(),
        Contents::Lies { start, .. } => {
            let (path, start) = (path.clone(), start + u64::from(from));
            lying(ctx, &path, start, n)?
        }
    };
    ctx.mem.write(buffer, &chunk).map_err(Stop::Fault)?;
    if let Some(Object::File { at, .. }) = ctx.state.object_mut(handle) {
        *at += n;
    }
    Ok(Some(n))
}

fn read_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // ReadFile(handle, buffer, want, read, overlapped).
    let (handle, buffer, want, read_at) = (args[0], args[1], args[2], args[3]);
    let Some(n) = read(ctx, handle, buffer, want)? else {
        return Ok(0);
    };
    if read_at != 0 {
        ctx.mem.write_u32(read_at, n).map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn write_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // WriteFile(handle, buffer, want, wrote, overlapped). What the guest
    // writes is kept in the open file and reaches its profile when the file
    // is closed.
    let (handle, buffer, want, wrote_at) = (args[0], args[1], args[2], args[3]);
    // The handle before the buffer, as `read_file` does: a write through a
    // handle this machine does not hold is refused, and a fault reading the
    // buffer would have ended the run at a wall naming memory rather than
    // answering the call the title actually made (#43).
    match ctx.state.object(handle) {
        Some(Object::File { access, .. }) if access & GENERIC_WRITE != 0 => {}
        Some(Object::Std(_)) => {}
        _ => {
            ctx.state.last_error = ERROR_ACCESS_DENIED;
            return Ok(0);
        }
    }
    let given = ctx
        .mem
        .slice(buffer, want as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    match ctx.state.object_mut(handle) {
        // A handle that can write holds what it writes to.
        Some(Object::File {
            contents: Contents::Held(bytes),
            at,
            wrote,
            ..
        }) => {
            *wrote = true;
            let start = *at as usize;
            if bytes.len() < start + given.len() {
                bytes.resize(start + given.len(), 0);
            }
            bytes[start..start + given.len()].copy_from_slice(&given);
            *at = u32::try_from(start + given.len()).unwrap_or(u32::MAX);
        }
        // A standard stream: the declared machine has no console to show it
        // in, so what is written there is written and gone.
        Some(Object::Std(_)) => {}
        // Refused above; nothing else reaches here.
        _ => {
            ctx.state.last_error = ERROR_ACCESS_DENIED;
            return Ok(0);
        }
    }
    if wrote_at != 0 {
        ctx.mem.write_u32(wrote_at, want).map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn set_file_pointer(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // SetFilePointer(handle, distance, high, method).
    let (handle, distance, method) = (args[0], args[1] as i32, args[3]);
    let Some(Object::File { contents, at, .. }) = ctx.state.object(handle) else {
        ctx.state.last_error = ERROR_INVALID_HANDLE;
        return Ok(INVALID_HANDLE);
    };
    let (at, size) = (*at, contents.size());
    let base = match method {
        0 => 0,
        1 => at,
        2 => size,
        _ => {
            ctx.state.last_error = ERROR_INVALID_PARAMETER;
            return Ok(INVALID_HANDLE);
        }
    };
    // Past the end of a file is somewhere, whether the handle reads or
    // writes: a read there finds nothing, and a write there makes the file
    // that long. Windows says so of SetFilePointer, and LEGO Island seeks
    // through its scripts by it (#116).
    //
    // Before the start of the file is nowhere, and a seek there moves nothing:
    // Windows refuses it, and letting it wrap would put the next write four
    // gigabytes along a file that the machine would then try to make that long.
    let Some(moved) = seek_to(base, distance) else {
        ctx.state.last_error = ERROR_NEGATIVE_SEEK;
        return Ok(INVALID_HANDLE);
    };
    if let Some(Object::File { at, .. }) = ctx.state.object_mut(handle) {
        *at = moved;
    }
    Ok(moved)
}

fn get_file_size(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetFileSize(file, high): the low half, and the high half where the
    // guest asks for it. Nothing on the declared machine is four gigabytes.
    let Some(Object::File { contents, .. }) = ctx.state.object(args[0]) else {
        ctx.state.last_error = ERROR_INVALID_HANDLE;
        // INVALID_FILE_SIZE.
        return Ok(0xffff_ffff);
    };
    let size = contents.size();
    if args[1] != 0 {
        ctx.mem.write_u32(args[1], 0).map_err(Stop::Fault)?;
    }
    Ok(size)
}

fn set_end_of_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // SetEndOfFile(file): the file ends where the handle is up to, cut short
    // or made longer. Windows leaves what it is made longer by undefined; the
    // declared machine makes it zeros.
    match ctx.state.object_mut(args[0]) {
        Some(Object::File {
            contents: Contents::Held(bytes),
            at,
            access,
            wrote,
            ..
        }) if *access & GENERIC_WRITE != 0 => {
            bytes.resize(*at as usize, 0);
            *wrote = true;
            Ok(1)
        }
        Some(Object::File { .. }) => {
            ctx.state.last_error = ERROR_ACCESS_DENIED;
            Ok(0)
        }
        _ => {
            ctx.state.last_error = ERROR_INVALID_HANDLE;
            Ok(0)
        }
    }
}

/// What `OpenFile` and the 16-bit calls beside it answer when they fail.
const HFILE_ERROR: u32 = 0xffff_ffff;

/// `OpenFile`'s style: how the file is opened and shared, and what else is
/// asked of it.
const OF_WRITE: u32 = 0x0001;
const OF_READWRITE: u32 = 0x0002;
const OF_SHARE_EXCLUSIVE: u32 = 0x0010;
const OF_SHARE_DENY_WRITE: u32 = 0x0020;
const OF_SHARE_DENY_READ: u32 = 0x0030;
const OF_SHARE_DENY_NONE: u32 = 0x0040;
const OF_CANCEL: u32 = 0x0800;
const OF_CREATE: u32 = 0x1000;
const OF_EXIST: u32 = 0x4000;

fn open_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // OpenFile(name, OFSTRUCT, style): Windows 3.1's door onto what
    // CreateFileA opens, kept in Win32, and the one LEGO Island's scripts and
    // world are read through (#116). Its handles are CreateFileA's.
    let (named, record, style) = (ctx.string(args[0]).map_err(Stop::Fault)?, args[1], args[2]);
    // Making a file is MS-DOS's create (function 3Ch), which opens what it
    // makes to read and write it, whatever else the style says.
    let access = match style & 3 {
        _ if style & OF_CREATE != 0 => Some(GENERIC_READ | GENERIC_WRITE),
        0 => Some(GENERIC_READ),
        OF_WRITE => Some(GENERIC_WRITE),
        OF_READWRITE => Some(GENERIC_READ | GENERIC_WRITE),
        _ => None,
    };
    // Each sharing mode as the documentation maps it onto CreateFileA's.
    let share = match style & 0x70 {
        0 | OF_SHARE_DENY_NONE => Some(FILE_SHARE_READ | FILE_SHARE_WRITE),
        OF_SHARE_EXCLUSIVE => Some(0),
        OF_SHARE_DENY_WRITE => Some(FILE_SHARE_READ),
        OF_SHARE_DENY_READ => Some(FILE_SHARE_WRITE),
        _ => None,
    };
    // OF_CANCEL is documented as ignored. Parsing a name, deleting, verifying,
    // prompting and reopening each do something other than open a file, and
    // no title has asked for one: each is a wall, not a guess.
    let answered = 0x73 | OF_CANCEL | OF_CREATE | OF_EXIST;
    let (Some(access), Some(share), 0) = (access, share, style & !answered) else {
        return Err(Stop::NotWritten {
            what: format!("KERNEL32!OpenFile({named:?}) with style {style:#06x}"),
            caller: 0,
        });
    };

    let mut path = ctx.absolute(&named);
    // A bare name is looked for first where the title was started from, then
    // where it is: the system and Windows directories OpenFile looks in next
    // hold nothing on the declared machine. One being made is made here.
    if style & OF_CREATE == 0 && !named.contains(['\\', '/', ':']) {
        let started = format!("{}\\{named}", ctx.title.install_dir);
        if ctx.tree.readable(&started) {
            path = started;
        }
    }
    let disposition = if style & OF_CREATE == 0 {
        OPEN_EXISTING
    } else {
        CREATE_ALWAYS
    };
    let handle = open(ctx, path.clone(), access, share, disposition)?;

    if record != 0 {
        // OFSTRUCT: its own length, whether the file is on a fixed disk, the
        // MS-DOS error when the open failed, which has the same number as the
        // Win32 one, two reserved words, and the path, in 136 bytes.
        let error = if handle == INVALID_HANDLE {
            ctx.state.last_error
        } else {
            0
        };
        let mut fields = [0u8; 8];
        fields[0] = 136;
        fields[1] = u8::from(path.get(..2).is_some_and(|d| d.eq_ignore_ascii_case("C:")));
        fields[2..4].copy_from_slice(&u16::try_from(error).unwrap_or(0).to_le_bytes());
        ctx.mem.write(record, &fields).map_err(Stop::Fault)?;
        ctx.write_string(record + 8, 128, &path)
            .map_err(Stop::Fault)?;
    }
    if handle == INVALID_HANDLE {
        return Ok(HFILE_ERROR);
    }
    if style & OF_EXIST != 0 {
        // Opened and closed again: a question, answered as Windows is
        // observed to answer it, with TRUE rather than a handle that is no
        // longer open.
        close_handle(ctx, &[handle])?;
        return Ok(1);
    }
    Ok(handle)
}

fn hread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // _hread(file, buffer, count): ReadFile, answering with how much it read,
    // which is short of the count at the end of the file.
    Ok(read(ctx, args[0], args[1], args[2])?.unwrap_or(HFILE_ERROR))
}

fn hwrite(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // _hwrite(file, buffer, count): WriteFile, answering with how much it
    // wrote. It is MS-DOS's write (function 40h), where writing nothing sets
    // the end of the file where the handle is up to.
    let done = if args[2] == 0 {
        set_end_of_file(ctx, &args[..1])?
    } else {
        write_file(ctx, &[args[0], args[1], args[2], 0, 0])?
    };
    Ok(if done == 0 { HFILE_ERROR } else { args[2] })
}

fn llseek(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // _llseek(file, offset, origin): SetFilePointer without its high half,
    // whose failure is HFILE_ERROR too.
    set_file_pointer(ctx, &[args[0], args[1], 0, args[2]])
}

fn lclose(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // _lclose(file): CloseHandle, answering zero when it closed something.
    if close_handle(ctx, args)? == 0 {
        ctx.state.last_error = ERROR_INVALID_HANDLE;
        return Ok(HFILE_ERROR);
    }
    Ok(0)
}

/// Where a seek from here lands, and nothing when that is off the file.
///
/// Before the start of a file is nowhere: Windows refuses a seek there rather
/// than wrapping, and a wrap would put the next write four gigabytes along a
/// file the machine would then try to make that long.
fn seek_to(base: u32, distance: i32) -> Option<u32> {
    u32::try_from(i64::from(base) + i64::from(distance)).ok()
}

fn get_file_attributes(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Whether it is there, from the tree: a file packed in a cabinet is not
    // unpacked to say so.
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    let path = ctx.absolute(&path);
    if ctx.refuse_on_empty_floppy(&path)? {
        return Ok(INVALID_HANDLE);
    }
    if ctx.tree.readable(&path) {
        // FILE_ATTRIBUTE_NORMAL.
        return Ok(0x80);
    }
    ctx.state.last_error = ERROR_FILE_NOT_FOUND;
    Ok(INVALID_HANDLE)
}

fn get_drive_type(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's drives: a fixed disk the title is installed on,
    // a CD-ROM its pressing is in, and a floppy drive when it declares one.
    // Anything else does not exist.
    // No root at all is the current directory's, which is taken to be on C:
    // whatever it is: Alpha Team saves only when it hears C: or a floppy
    // drive here (#145's blast review), so it is left as it was.
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    Ok(ctx.drive_type(path.bytes().next().unwrap_or(b'C')))
}

fn get_logical_drives(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A bit for each: bit 0 is A, as Microsoft documents.
    Ok(ctx
        .drives()
        .iter()
        .fold(0, |mask, &(letter, _)| mask | 1 << (letter - b'A')))
}

/// A file the guest would rather read as memory than through a handle.
///
/// Alpha Team's archives are up to a hundred and twenty megabytes each and it
/// maps every one of them, so the bytes go into the arena where the guest can
/// address them, once, and stay there (#57).
fn create_file_mapping(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateFileMappingA(file, security, protection, high, low, name).
    let Some(Object::File { contents, path, .. }) = ctx.state.object(args[0]) else {
        ctx.state.last_error = ERROR_INVALID_HANDLE;
        return Ok(0);
    };
    let (contents, path) = (contents.clone(), path.clone());
    let bytes = match contents {
        Contents::Held(bytes) => bytes,
        Contents::Lies { start, len } => lying(ctx, &path, start, len)?,
    };
    let len = u32::try_from(bytes.len()).unwrap_or(0);
    let at = ctx.state.next_mapping;
    let end = at
        .checked_add(len.next_multiple_of(0x1000))
        .filter(|end| *end as usize <= ctx.mem.len());
    let Some(end) = end else {
        // ERROR_NOT_ENOUGH_MEMORY: the arena has no room left for it.
        ctx.state.last_error = 8;
        return Ok(0);
    };
    ctx.state.next_mapping = end;
    ctx.mem.write(at, &bytes).map_err(Stop::Fault)?;
    ctx.log
        .line(&format!("mapped {path}: {len} bytes at {at:#010x}"));
    Ok(ctx.state.give(Object::Mapping { at, len }))
}

fn map_view_of_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // MapViewOfFile(mapping, access, high, low, bytes). The whole file is
    // already where it is going to be, so a view of it is where it starts.
    let Some(Object::Mapping { at, len }) = ctx.state.object(args[0]) else {
        ctx.state.last_error = 6;
        return Ok(0);
    };
    let (at, len) = (*at, *len);
    let offset = args[3];
    if offset > len {
        ctx.state.last_error = 6;
        return Ok(0);
    }
    Ok(at + offset)
}

fn unmap_view_of_file(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // The bytes stay where they are: nothing else is handed that part of the
    // arena, and a title that unmaps and maps again finds them again.
    Ok(1)
}

/// Which of two file times is the earlier.
fn compare_file_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let read = |ctx: &mut Ctx<'_>, at: u32| -> Result<u64, Stop> {
        let low = u64::from(ctx.mem.read_u32(at).map_err(Stop::Fault)?);
        let high = u64::from(ctx.mem.read_u32(at + 4).map_err(Stop::Fault)?);
        Ok(low | (high << 32))
    };
    let (left, right) = (read(ctx, args[0])?, read(ctx, args[1])?);
    Ok(match left.cmp(&right) {
        std::cmp::Ordering::Less => 0xffff_ffff,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

/// When a file was made, last read and last written.
///
/// One declared moment for all three, and the same one for every file: the
/// pressing's own files were all written when it was pressed, and a run that
/// read the host's clock here would replay differently tomorrow (ADR-0003).
///
/// So `WHEN` stands for the date of the pressing: 1999-10-04 01:20 universal
/// time, 939,000,000 seconds after 1970. That is before `DECLARED_DAY`, the
/// moment every run's clock starts at, so no file is newer than the run. It
/// is declared, not read from the disc's own directory (#91).
fn get_file_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetFileTime(file, created, accessed, written), as Windows counts:
    // hundreds of nanoseconds since the start of 1601.
    const WHEN: u64 = (11_644_473_600 + 939_000_000) * 10_000_000;
    for at in &args[1..4] {
        if *at != 0 {
            ctx.mem
                .write(*at, &WHEN.to_le_bytes())
                .map_err(Stop::Fault)?;
        }
    }
    Ok(1)
}

/// The handler this machine puts on the chain while it is running one of the
/// guest's: `mov eax, 1` — ExceptionContinueSearch — and back.
///
/// Windows' `RtlpExceptionHandler` answers the same while it is being unwound
/// past, which is the case every C++ catch here goes through.
///
// ponytail: Windows answers ExceptionNestedException instead when an
// exception is raised *inside* a handler, which stops the outer dispatcher
// resuming into a frame that is already going away. This keeps searching
// outward instead, which finds a catch rather than a wall; write the real
// answer if a title is ever seen to throw from inside a handler.
const KEEP_LOOKING: [u8; 6] = [0xb8, 0x01, 0x00, 0x00, 0x00, 0xc3];

/// The bytes that call one frame's handler the way Windows' own dispatcher
/// calls it: `handler(record, frame, context, dispatcher)`, cdecl.
///
/// `RtlpExecuteHandler` puts a registration record of its own on the stack and
/// points `fs:[0]` at it before calling a handler, and puts the chain back
/// afterwards. That matters because a handler may save `fs:[0]`, unwind, and
/// then relink what it saved — which is exactly what MSVC's
/// `_UnwindNestedFrames` does around its own `RtlUnwind`. With the frame's own
/// record still at the head, that save and restore writes the record's `next`
/// field with the record's own address, the chain becomes a one-element loop,
/// and the next throw lands on stack it has already given away (#60).
///
/// Where [`KEEP_LOOKING`] will be is not known until the program has been
/// written somewhere, so each place its address goes is noted in `holes`.
fn call_handler(code: &mut Vec<u8>, holes: &mut Vec<usize>, handler: u32, args: [u32; 4]) {
    code.extend_from_slice(&[0x64, 0xa1, 0, 0, 0, 0]); // mov eax, fs:[0]
    code.push(0x68); // push <this machine's own handler>
    holes.push(code.len());
    code.extend_from_slice(&[0; 4]);
    code.push(0x50); // push eax: the chain as it was
    code.extend_from_slice(&[0x64, 0x89, 0x25, 0, 0, 0, 0]); // mov fs:[0], esp

    // Last argument first, as cdecl puts them on the stack.
    for value in args.iter().rev() {
        code.push(0x68);
        code.extend_from_slice(&value.to_le_bytes());
    }
    code.push(0xb8);
    code.extend_from_slice(&handler.to_le_bytes());
    code.extend_from_slice(&[0xff, 0xd0]); // call eax
    code.extend_from_slice(&[0x83, 0xc4, 0x10]); // add esp, 16: it is cdecl
    code.push(0x59); // pop ecx: the chain as it was
    code.extend_from_slice(&[0x64, 0x89, 0x0d, 0, 0, 0, 0]); // mov fs:[0], ecx
    code.extend_from_slice(&[0x83, 0xc4, 0x04]); // and this record is gone
}

/// Raise an exception and offer it to the guest's own handlers (#54).
///
/// The chain is the guest's, at `fs:[0]`, and the handlers on it are guest
/// code: what this does is walk it and call them, in order, until one takes
/// the exception. A C++ throw is exactly this, and the handler that takes it
/// does not come back — it goes to the catch.
fn raise_exception(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // RaiseException(code, flags, argument count, arguments).
    let (code, flags, count, values) = (args[0], args[1], args[2].min(15), args[3]);
    let raised_at = ctx.cpu.eip;
    let params = (0..count)
        .map(|i| ctx.mem.read_u32(values + i * 4).map_err(Stop::Fault))
        .collect::<Result<Vec<_>, _>>()?;
    let what = format!("an exception of {code:#010x} was raised");
    let at = dispatch(ctx, code, flags, &params, raised_at, what, false)?;
    ctx.call_guest = Some((at, Vec::new()));
    Ok(0)
}

/// Offer an exception to the guest's own handlers: the record, the context,
/// and a program that calls each handler on the chain in turn, written into
/// the guest. Where that program starts is the answer.
///
/// `RaiseException` comes here (#54), and so does a fault the processor took
/// (#132). The two differ in what a handler that says to carry on is carrying
/// on from: a call to `RaiseException` returns to its caller, and a fault puts
/// the processor back as the context now says, which is how a handler that
/// has mended the context retries or skips the instruction. If nothing takes
/// it, the run stops at a wall that says `what`.
pub fn dispatch(
    ctx: &mut Ctx<'_>,
    code: u32,
    flags: u32,
    params: &[u32],
    raised_at: u32,
    what: String,
    fault: bool,
) -> Result<u32, Stop> {
    // EXCEPTION_RECORD: what was raised, where, and what was passed with it.
    let record = ctx.state.heap.alloc(0x50);
    ctx.mem.write(record, &[0u8; 0x50]).map_err(Stop::Fault)?;
    ctx.mem.write_u32(record, code).map_err(Stop::Fault)?;
    ctx.mem.write_u32(record + 4, flags).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(record + 12, raised_at)
        .map_err(Stop::Fault)?;
    let count = u32::try_from(params.len()).unwrap_or(0);
    ctx.mem.write_u32(record + 16, count).map_err(Stop::Fault)?;
    for (at, value) in (record + 20..).step_by(4).zip(params) {
        ctx.mem.write_u32(at, *value).map_err(Stop::Fault)?;
    }

    // CONTEXT: where the processor stood. A handler that unwinds reads the
    // stack and instruction pointers out of it.
    let context = ctx.state.heap.alloc(0x2cc);
    ctx.mem.write(context, &[0u8; 0x2cc]).map_err(Stop::Fault)?;
    // CONTEXT_FULL: the integer, control and segment registers are here.
    ctx.mem
        .write_u32(context, 0x0001_0007)
        .map_err(Stop::Fault)?;
    let put = |ctx: &mut Ctx<'_>, at: u32, value: u32| {
        ctx.mem.write_u32(context + at, value).map_err(Stop::Fault)
    };
    use crate::cpu::{EAX, EBP, EBX, ECX, EDI, EDX, ESI, ESP};
    let r = ctx.cpu.r;
    put(ctx, 0x9c, r[EDI])?;
    put(ctx, 0xa0, r[ESI])?;
    put(ctx, 0xa4, r[EBX])?;
    put(ctx, 0xa8, r[EDX])?;
    put(ctx, 0xac, r[ECX])?;
    put(ctx, 0xb0, r[EAX])?;
    put(ctx, 0xb4, r[EBP])?;
    put(ctx, 0xb8, raised_at)?;
    // EFlags and Esp where `winnt.h`'s i386 CONTEXT has them, after SegCs at
    // 0xbc, for a fault and `RaiseException` alike (#133).
    put(ctx, 0xc0, ctx.cpu.flags.to_eflags())?;
    put(ctx, 0xc4, r[ESP])?;

    // The chain, walked here so that the program below has every handler on
    // it: a frame is the next frame and the handler, in that order.
    let mut frames = Vec::new();
    let mut frame = ctx
        .mem
        .read_u32(ctx.cpu.seg_base[crate::cpu::FS])
        .map_err(Stop::Fault)?;
    while frame != 0xffff_ffff && frame != 0 && frames.len() < 64 {
        let handler = ctx.mem.read_u32(frame + 4).map_err(Stop::Fault)?;
        frames.push((frame, handler));
        let next = ctx.mem.read_u32(frame).map_err(Stop::Fault)?;
        if next == frame {
            break;
        }
        frame = next;
    }
    ctx.log.line(&format!(
        "exception {code:#010x} raised at {raised_at:#010x}, {} handlers on the chain",
        frames.len()
    ));

    // The program: each handler in turn, stopping at the first that does not
    // say to keep looking. ExceptionContinueSearch is one; anything else has
    // taken it, and a handler that goes to a catch never comes back at all.
    // The program the machine's own handler sits at the front of, so that its
    // address is the program's own.
    let mut code_bytes: Vec<u8> = KEEP_LOOKING.to_vec();
    let mut holes = Vec::new();
    let mut mine = Vec::new();
    for (frame, handler) in &frames {
        call_handler(
            &mut code_bytes,
            &mut mine,
            *handler,
            [record, *frame, context, 0],
        );
        code_bytes.extend_from_slice(&[0x83, 0xf8, 0x01]); // cmp eax, 1
        code_bytes.extend_from_slice(&[0x0f, 0x85]); // jne <done>
        holes.push(code_bytes.len());
        code_bytes.extend_from_slice(&[0, 0, 0, 0]);
    }
    // Nothing took it.
    let unhandled = ctx.boundary.bind_unhandled(what);
    code_bytes.push(0xb8);
    code_bytes.extend_from_slice(&unhandled.to_le_bytes());
    code_bytes.extend_from_slice(&[0xff, 0xe0]); // jmp eax
    let done = code_bytes.len();
    for hole in holes {
        let rel = u32::try_from(done - (hole + 4)).unwrap_or(0);
        code_bytes[hole..hole + 4].copy_from_slice(&rel.to_le_bytes());
    }
    if fault {
        // ExceptionContinueExecution: every register back from the context,
        // the flags by way of a `popfd` with the stack pointed at them, and on
        // to its Eip, as `NtContinue` does.
        for (op, field) in [
            (&[0x8b, 0x3d][..], 0x9c),   // mov edi, [Edi]
            (&[0x8b, 0x35], 0xa0),       // mov esi, [Esi]
            (&[0x8b, 0x1d], 0xa4),       // mov ebx, [Ebx]
            (&[0x8b, 0x15], 0xa8),       // mov edx, [Edx]
            (&[0x8b, 0x0d], 0xac),       // mov ecx, [Ecx]
            (&[0x8b, 0x2d], 0xb4),       // mov ebp, [Ebp]
            (&[0xbc], 0xc0),             // mov esp, &EFlags
            (&[0x9d, 0x8b, 0x25], 0xc4), // popfd; mov esp, [Esp]
            (&[0xa1], 0xb0),             // mov eax, [Eax]
            (&[0xff, 0x25], 0xb8),       // jmp [Eip]
        ] {
            code_bytes.extend_from_slice(op);
            code_bytes.extend_from_slice(&(context + field).to_le_bytes());
        }
    } else {
        code_bytes.push(0xc3); // ret: a handler said to carry on where it was
    }

    let at = ctx
        .state
        .heap
        .alloc(u32::try_from(code_bytes.len()).unwrap_or(0));
    for hole in mine {
        code_bytes[hole..hole + 4].copy_from_slice(&at.to_le_bytes());
    }
    ctx.mem.write(at, &code_bytes).map_err(Stop::Fault)?;
    Ok(at + KEEP_LOOKING.len() as u32)
}

/// Unwind the guest's handler chain as far as a frame, telling every handler
/// on the way that it is being unwound (#54).
///
/// What a handler that has decided to take an exception calls before it goes
/// to its catch: the frames between are given their chance to clean up.
fn rtl_unwind(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // RtlUnwind(target frame, target instruction, record, return value).
    let (target, target_ip) = (args[0], args[1]);
    // The usual form names the instruction after the call, which is where the
    // machine is going to put the guest back anyway.
    let returns_to = ctx
        .mem
        .read_u32(ctx.cpu.r[crate::cpu::ESP])
        .map_err(Stop::Fault)?;
    if target_ip != 0 && target_ip != returns_to {
        return Err(Stop::NotWritten {
            what: format!(
                "KERNEL32.dll!RtlUnwind: to {target_ip:#010x}, and this machine only unwinds \
                 back to the caller"
            ),
            caller: 0,
        });
    }

    // The record the caller passed, or one of this machine's own saying what
    // is happening, with the unwinding flag set either way.
    const EXCEPTION_UNWINDING: u32 = 2;
    let record = if args[2] == 0 {
        let at = ctx.state.heap.alloc(0x50);
        ctx.mem.write(at, &[0u8; 0x50]).map_err(Stop::Fault)?;
        // STATUS_UNWIND.
        ctx.mem.write_u32(at, 0xc000_0027).map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(at + 4, EXCEPTION_UNWINDING)
            .map_err(Stop::Fault)?;
        at
    } else {
        let was = ctx.mem.read_u32(args[2] + 4).map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(args[2] + 4, was | EXCEPTION_UNWINDING)
            .map_err(Stop::Fault)?;
        args[2]
    };

    let head = ctx.cpu.seg_base[crate::cpu::FS];
    let mut frames = Vec::new();
    let mut frame = ctx.mem.read_u32(head).map_err(Stop::Fault)?;
    while frame != 0xffff_ffff && frame != 0 && frame != target && frames.len() < 64 {
        let handler = ctx.mem.read_u32(frame + 4).map_err(Stop::Fault)?;
        frames.push((frame, handler));
        let next = ctx.mem.read_u32(frame).map_err(Stop::Fault)?;
        if next == frame {
            break;
        }
        frame = next;
    }
    // The chain ends where the unwind was asked to stop, whether or not every
    // handler between has run yet: that is what the frames below are being
    // told.
    ctx.mem.write_u32(head, target).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "unwinding {} handlers back to {target:#010x}",
        frames.len()
    ));
    if frames.is_empty() {
        return Ok(0);
    }

    let mut code: Vec<u8> = KEEP_LOOKING.to_vec();
    let mut mine = Vec::new();
    for (frame, handler) in &frames {
        call_handler(&mut code, &mut mine, *handler, [record, *frame, 0, 0]);
    }
    code.push(0xc3);
    let at = ctx.state.heap.alloc(u32::try_from(code.len()).unwrap_or(0));
    for hole in mine {
        code[hole..hole + 4].copy_from_slice(&at.to_le_bytes());
    }
    ctx.mem.write(at, &code).map_err(Stop::Fault)?;
    ctx.call_guest = Some((at + KEEP_LOOKING.len() as u32, Vec::new()));
    Ok(0)
}

/// The string calls Windows has always had, which are the C ones under other
/// names.
fn lstrcpy(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let mut bytes = ctx.string(args[1]).map_err(Stop::Fault)?.into_bytes();
    bytes.push(0);
    ctx.mem.write(args[0], &bytes).map_err(Stop::Fault)?;
    Ok(args[0])
}

fn lstrcat(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let already = ctx.string(args[0]).map_err(Stop::Fault)?;
    let added = ctx.string(args[1]).map_err(Stop::Fault)?;
    let mut bytes = format!("{already}{added}").into_bytes();
    bytes.push(0);
    ctx.mem.write(args[0], &bytes).map_err(Stop::Fault)?;
    Ok(args[0])
}

fn lstrlen(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(0);
    }
    let text = ctx.string(args[0]).map_err(Stop::Fault)?;
    Ok(u32::try_from(text.len()).unwrap_or(0))
}

fn lstrcmpi(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Less, the same, or greater, as a negative, zero or positive answer.
    let left = ctx.string(args[0]).map_err(Stop::Fault)?.to_lowercase();
    let right = ctx.string(args[1]).map_err(Stop::Fault)?.to_lowercase();
    Ok(match left.cmp(&right) {
        std::cmp::Ordering::Less => 0xffff_ffff,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    })
}

/// Multiply two signed numbers, then divide, rounding to nearest and keeping
/// the whole product along the way.
fn mul_div(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (a, b, c) = (args[0] as i32, args[1] as i32, args[2] as i32);
    if c == 0 {
        // What Windows answers when it cannot: the caller checks for it.
        return Ok(0xffff_ffff);
    }
    let product = i64::from(a) * i64::from(b);
    let half = i64::from(c) / 2;
    let rounded = if (product < 0) == (c < 0) {
        product + half
    } else {
        product - half
    } / i64::from(c);
    Ok(i32::try_from(rounded).map_or(0xffff_ffff, |n| n as u32))
}

/// What the declared machine's clock is offset from universal time by.
///
/// None: the declared clock is the machine's own and has no zone behind it
/// (ADR-0003), so the two are the same and nothing a title formats moves with
/// where the host happens to be.
fn get_time_zone_information(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // TIME_ZONE_INFORMATION is 172 bytes: a bias, two names, two dates and two
    // more biases. All zero says universal time with no daylight saving.
    ctx.mem.write(args[0], &[0u8; 172]).map_err(Stop::Fault)?;
    // TIME_ZONE_ID_UNKNOWN: there is no daylight saving to be in or out of.
    Ok(0)
}

/// The declared machine's drives, written out as the roots they are.
fn get_logical_drive_strings(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetLogicalDriveStringsA(size, buffer). The same drives
    // `GetLogicalDrives` names in bits.
    let drives: Vec<u8> = ctx
        .drives()
        .iter()
        .flat_map(|&(letter, _)| [letter, b':', b'\\', 0])
        .chain([0])
        .collect();
    let needed = u32::try_from(drives.len()).unwrap_or(0);
    if args[0] < needed {
        // What it would take, which is what a caller asking with nothing gets.
        return Ok(needed);
    }
    ctx.mem.write(args[1], &drives).map_err(Stop::Fault)?;
    // Everything but the last terminator, which is how this one counts.
    Ok(needed - 1)
}

fn get_volume_information(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A title looks at the disc's own label to decide whether the right disc
    // is in the drive. The label is the pressing's, read off the image itself.
    let root = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&root)? {
        return Ok(0);
    }
    let is_cd = ctx.drive_type(root.bytes().next().unwrap_or(b'C')) == DRIVE_CDROM;
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
    // The declared clock, not the host's (ADR-0003). It moves a millisecond at
    // a time, which a clock spin's pass relies on (`spin.rs`, #217): an answer
    // finer than that needs its own horizon there.
    Ok(u32::try_from(ctx.declared_millis() & 0xffff_ffff).unwrap_or(0))
}

fn query_performance_counter(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // In whole declared milliseconds, which a clock spin's pass relies on
    // (`spin.rs`, #217): a counter finer than that, as Windows 98's 1,193,182
    // Hz one is, needs its own horizon there.
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

/// The declared moment a run starts at: midnight, universal time, at the start
/// of Friday 1999-10-29, in whole days since the start of 1601, which is where
/// Windows counts from.
///
/// A declared date: a run that read the host's calendar would replay
/// differently tomorrow. This one is the date `GetSystemTime` answers with
/// (#8), and the one Alpha Team's Select Adventure labels a saved adventure
/// with (#87). Every clock that tells the date is this day plus the declared
/// clock's milliseconds, so none of them can disagree with another (#91).
const DECLARED_DAY: u64 = 145_667;

/// The declared moment `millis` into a run, as a SYSTEMTIME's eight
/// sixteen-bit fields: year, month, day of the week, day, hour, minute,
/// second, millisecond. `GetSystemTime`, `GetLocalTime` and a date or time
/// formatted for "now" (#87) all read it here.
fn declared_now(millis: u64) -> [u16; 8] {
    // Hinnant's `civil_from_days`, counted from 0000-03-01 so that every four
    // hundred years start alike and the leap day falls last.
    let days = DECLARED_DAY + millis / 86_400_000 + 584_694;
    let (era, day) = (days / 146_097, days % 146_097);
    let year = (day - day / 1460 + day / 36_524 - day / 146_096) / 365;
    let day = day - (365 * year + year / 4 - year / 100);
    let march = (5 * day + 2) / 153;
    let date = day - (153 * march + 2) / 5 + 1;
    let (year, month) = if march < 10 {
        (era * 400 + year, march + 3)
    } else {
        (era * 400 + year + 1, march - 9)
    };
    let field = |value: u64| u16::try_from(value).unwrap_or(0);
    let mut now = [
        field(year),
        field(month),
        0,
        field(date),
        field(millis / 3_600_000 % 24),
        field(millis / 60_000 % 60),
        field(millis / 1000 % 60),
        field(millis % 1000),
    ];
    now[2] = weekday(&now).map_or(0, |day| day as u16);
    now
}

/// The same moment as a FILETIME: hundreds of nanoseconds since the start of
/// 1601.
fn declared_file_time(millis: u64) -> u64 {
    (DECLARED_DAY * 86_400_000 + millis) * 10_000
}

fn get_system_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let at = args[0];
    let fields = declared_now(ctx.declared_millis());
    for (i, value) in fields.iter().enumerate() {
        ctx.mem
            .write(at + (i as u32) * 2, &value.to_le_bytes())
            .map_err(Stop::Fault)?;
    }
    Ok(0)
}

fn sleep(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A thread that sleeps is not running, and something else may be: the
    // machine puts it down until the declared clock reaches its hour and runs
    // whoever else can (ADR-0005). With one thread there is nobody else, and
    // the clock moves by the whole wait — the host is not asked to wait
    // either way, because presentation is what paces a run (ADR-0003).
    let running = ctx.state.thread;
    if ctx.state.threads.len() < 2 {
        ctx.state.waited_millis += u64::from(args[0]);
        return Ok(0);
    }
    let until = ctx.clock_millis + u64::from(args[0]);
    let Some(thread) = ctx.state.threads.get_mut(running) else {
        return Ok(0);
    };
    if thread.wake_at != 0 {
        if thread.wake_at > ctx.clock_millis {
            // Still asleep: the machine comes back to this call until its hour.
            return Err(Stop::Blocked);
        }
        // Its hour has come, and the call it was parked on returns.
        thread.wake_at = 0;
        return Ok(0);
    }
    if until > ctx.clock_millis {
        thread.wake_at = until;
        return Err(Stop::Blocked);
    }
    // Sleep(0) gives the rest of this thread's turn to the next thread that
    // can run, in the machine's own order, never the host's (ADR-0005, #115):
    // the call returns now and the machine switches before this thread runs
    // on. With nobody else ready it runs on at once, as Windows' does.
    ctx.state.yielded = true;
    Ok(0)
}

fn get_system_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let written = ctx
        .write_string(args[0], args[1], "C:\\WINDOWS\\SYSTEM")
        .map_err(Stop::Fault)?;
    Ok(written)
}

/// What Windows 9x answers for a library the machine does not have: Wine's
/// conformance test from when it still ran on 9x accepts this or
/// `ERROR_MOD_NOT_FOUND`, and names this one 9x's (wine-1.0,
/// `dlls/kernel32/tests/module.c`, `testLoadLibraryA_Wrong`).
const ERROR_DLL_NOT_FOUND: u32 = 1157;
/// What Windows NT answers for it, as Microsoft documents.
const ERROR_MOD_NOT_FOUND: u32 = 126;

fn load_library(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A library the title ships is a guest binary and is loaded and run as
    // one (ADR-0002, #55), found whatever case it is asked for in; one
    // Microsoft wrote that the disc does not carry is written here and has no
    // image to load. One the title declares its machine lacks fails as
    // Windows fails for a module it cannot find: NULL, and the declared
    // Windows' error (#146). Anything else is a wall.
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("load library {name}"));
    // Except on the empty floppy drive, which has nothing to load.
    if ctx.refuse_on_empty_floppy(&name)? {
        return Ok(0);
    }
    // A path is one the installer would have written; only the file at the end
    // of it is the title's own. A name with no extension is looked for with
    // ".DLL" added, as Microsoft documents.
    let mut file = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned();
    if !file.contains('.') {
        file.push_str(".DLL");
    }
    if let Some(module) = ctx.modules.base_of_name(&file) {
        return Ok(module);
    }
    match load_and_bind(ctx, &file)? {
        Some(base) => {
            // Its own code attaches it, as it does for the ones loaded before
            // the title started. A dynamic load passes no reserved argument.
            if let Some(entry) = ctx.modules.entry_of(&file).filter(|e| *e != 0) {
                ctx.call_guest = Some((entry, vec![base, 1, 0]));
            }
            Ok(base)
        }
        // Known by the file it names, as GetProcAddress looks it up.
        None if super::supplies(&file) => Ok(ctx.state.give(Object::Module {
            name: file,
            base: 0,
        })),
        None => {
            let declared = &ctx.title.declared;
            let Some((_, why)) = declared
                .absent_libraries
                .iter()
                .find(|(absent, _)| absent.eq_ignore_ascii_case(&file))
            else {
                return Err(Stop::NotWritten {
                    what: format!(
                        "KERNEL32.dll!LoadLibraryA: {file} is neither the title's own, \
                         nor written, nor declared absent"
                    ),
                    caller: 0,
                });
            };
            ctx.log.line(&format!("library missing {file}: {why}"));
            // GetVersion's top bit is set on Windows 9x.
            ctx.state.last_error = if declared.windows.version & 0x8000_0000 != 0 {
                ERROR_DLL_NOT_FOUND
            } else {
                ERROR_MOD_NOT_FOUND
            };
            Ok(0)
        }
    }
}

/// Load a binary the title ships while it runs, and bind its own imports:
/// where it was put, or nothing when it is not one of the title's own.
///
/// # Errors
/// When the image will not load, or its address table is outside the arena.
pub(super) fn load_and_bind(ctx: &mut Ctx<'_>, file: &str) -> Result<Option<u32>, Stop> {
    let under = ctx.title.install_dir;
    let Some(base) = ctx
        .modules
        .load(file, under, ctx.tree, ctx.mem)
        .map_err(|e| Stop::NotWritten {
            what: format!("{file} will not load: {e}"),
            caller: 0,
        })?
    else {
        return Ok(None);
    };
    let imports = ctx
        .tree
        .read(&format!("{under}\\{file}"))
        .and_then(|bytes| crate::pe::imports_of(&bytes, base).ok())
        .unwrap_or_default();
    ctx.modules
        .bind(&imports, ctx.boundary, ctx.mem)
        .map_err(Stop::Fault)?;
    Ok(Some(base))
}

fn free_library(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn get_proc_address(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // What a guest asks a library for by name. A library the title ships has
    // an export table to answer from (#55); for anything else, answering with
    // an address that is not there would be worse than answering with nothing,
    // because a title that probes for a symbol and finds it will then call it.
    // The top half of the argument being zero is how Windows says an ordinal
    // rather than a pointer.
    let name = if args[1] < 0x1_0000 {
        format!("#{}", args[1])
    } else {
        ctx.string(args[1]).map_err(Stop::Fault)?
    };
    ctx.log.line(&format!("proc address {name}"));
    if let Some(addr) = ctx.modules.resolve_in(args[0], &name) {
        return Ok(addr);
    }
    // A library this emulator writes rather than runs has no export table to
    // look in: what it has is a place on the boundary, which is what a guest
    // that calls the address will arrive at. A name nothing stands behind is
    // still nothing, because a title that finds it will then call it.
    if let Some(Object::Module { name: dll, .. }) = ctx.state.object(args[0]) {
        let dll = dll.clone();
        if super::find(&dll, &name).is_some() {
            return Ok(ctx.boundary.bind_named(&dll, &name));
        }
    }
    ctx.state.last_error = 127;
    Ok(0)
}

fn find_first_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // What a title walking its own install directory asks: everything under
    // one directory whose name matches a pattern. The declared tree is what it
    // is walking, so what it finds is what it can then open (#57).
    let pattern = ctx.string(args[0]).map_err(Stop::Fault)?;
    let full = ctx.absolute(&pattern);
    if ctx.refuse_on_empty_floppy(&full)? {
        return Ok(INVALID_HANDLE);
    }
    let (dir, glob) = match full.rsplit_once('\\') {
        Some((dir, glob)) => (dir.to_owned(), glob.to_owned()),
        None => (ctx.state.current_directory.clone(), full.clone()),
    };
    // No title searches its CD yet, and what Windows answers for a disc read
    // through its Joliet tree, the 8.3 names included, is not known (#144):
    // a search of it is a wall until a title asks, not an empty disc.
    if full.get(..2).is_some_and(|d| d.eq_ignore_ascii_case("D:")) {
        return Err(Stop::NotWritten {
            what: format!("KERNEL32!FindFirstFileA({pattern:?}): a search of the CD"),
            caller: 0,
        });
    }
    let matches = ctx.tree.matching(&dir, &glob);
    ctx.log
        .line(&format!("find first {pattern}: {} match", matches.len()));
    let Some(first) = matches.first().cloned() else {
        ctx.state.last_error = ERROR_FILE_NOT_FOUND;
        return Ok(INVALID_HANDLE);
    };
    write_find_data(ctx, args[1], &first)?;
    Ok(ctx.state.give(Object::Find { matches, at: 1 }))
}

fn find_next_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let found = match ctx.state.object_mut(args[0]) {
        Some(Object::Find { matches, at }) => {
            let found = matches.get(*at).cloned();
            *at += 1;
            found
        }
        _ => None,
    };
    let Some(next) = found else {
        // ERROR_NO_MORE_FILES.
        ctx.state.last_error = 18;
        return Ok(0);
    };
    write_find_data(ctx, args[1], &next)?;
    Ok(1)
}

/// `WIN32_FIND_DATAA`: what one match looks like to the guest.
fn write_find_data(ctx: &mut Ctx<'_>, at: u32, found: &(String, u32)) -> Result<(), Stop> {
    ctx.mem.write(at, &[0u8; 0x140]).map_err(Stop::Fault)?;
    // FILE_ATTRIBUTE_READONLY: everything the pressing holds is.
    ctx.mem.write_u32(at, 1).map_err(Stop::Fault)?;
    ctx.mem.write_u32(at + 0x20, found.1).map_err(Stop::Fault)?;
    let mut name = found.0.clone().into_bytes();
    name.truncate(259);
    name.push(0);
    ctx.mem.write(at + 0x2c, &name).map_err(Stop::Fault)?;
    Ok(())
}

fn find_close(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn delete_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Only what the guest itself wrote is the guest's to delete; the
    // pressing's own files are not, and a title is told so rather than
    // believing a file it can still open has gone.
    let named = ctx.string(args[0]).map_err(Stop::Fault)?;
    let path = ctx.absolute(&named);
    if ctx.refuse_on_empty_floppy(&path)? {
        return Ok(0);
    }
    let gone = ctx.tree.remove(&path);
    ctx.log.line(&format!(
        "delete {path}{}",
        if gone { "" } else { ": nothing written there" }
    ));
    if !gone {
        ctx.state.last_error = ERROR_FILE_NOT_FOUND;
    }
    Ok(u32::from(gone))
}

/// The path a file is moved to is already taken.
const ERROR_ALREADY_EXISTS: u32 = 183;

fn move_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // MoveFileA(from, to): LEGO Island moves each player's save down a slot
    // when a new name signs its book (#125). Nothing is there to move, or
    // something is already where it would go, and the move fails as
    // Windows' does.
    let from = ctx.string(args[0]).map_err(Stop::Fault)?;
    let to = ctx.string(args[1]).map_err(Stop::Fault)?;
    let (from, to) = (ctx.absolute(&from), ctx.absolute(&to));
    if ctx.refuse_on_empty_floppy(&from)? || ctx.refuse_on_empty_floppy(&to)? {
        return Ok(0);
    }
    if !ctx.tree.readable(&from) {
        ctx.state.last_error = ERROR_FILE_NOT_FOUND;
        return Ok(0);
    }
    if ctx.tree.holds(&to) {
        ctx.state.last_error = ERROR_ALREADY_EXISTS;
        return Ok(0);
    }
    // Only a closed file the guest wrote moves, and only on C:. A file a
    // handle holds open is written where it was when the handle closes, and
    // the pressing's own files and the disc in D: are not the guest's to
    // change, so moving any of those is a wall rather than a guess.
    let key = |path: &str| path.replace('/', "\\").to_ascii_uppercase();
    let open = ctx
        .state
        .handles
        .values()
        .any(|object| matches!(object, Object::File { path, .. } if key(path) == key(&from)));
    let moved = if open || key(&to).starts_with("D:") {
        None
    } else {
        ctx.tree.rename(&from, &to)
    };
    match moved {
        None => {
            return Err(Stop::NotWritten {
                what: format!("KERNEL32.dll!MoveFileA: {from} to {to}"),
                caller: 0,
            })
        }
        Some(Ok(at)) => ctx
            .log
            .line(&format!("file moved {from} to {to}, to {}", at.display())),
        // As a save the host refuses when its handle closes: the guest is
        // told it moved, and the person is told it did not.
        Some(Err(e)) => {
            ctx.log
                .line(&format!("file move refused {from} to {to}: {e}"));
            eprintln!("lego-legacy: {from} was not moved to {to}: {e}");
            ctx.state.saves_refused += 1;
        }
    }
    Ok(1)
}

fn create_directory(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&path)? {
        return Ok(0);
    }
    ctx.log.line(&format!("create directory {path}"));
    Ok(1)
}

fn set_file_attributes(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Told it was done, for any file anywhere but the empty floppy drive
    // (#171).
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&path)? {
        return Ok(0);
    }
    Ok(1)
}

fn get_full_path_name(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    let full = ctx.absolute(&path);
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

/// A setting from `WIN.INI`, which the declared machine does not have.
///
/// So no section holds the key, and Windows' answer for a missing key is the
/// caller's own default, cut to fit, with any trailing blanks left off. LEGO
/// Island asks its own section for a `MediaPath` before it starts its engine
/// (#114).
fn get_profile_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetProfileStringA(section, key, default, out, room).
    let (section, key, default, out, room) = (args[0], args[1], args[2], args[3], args[4]);
    if section == 0 || key == 0 {
        // A null section or key asks for the names in the file instead.
        return Err(Stop::NotWritten {
            what: "KERNEL32.dll!GetProfileStringA: listing WIN.INI's names".to_owned(),
            caller: 0,
        });
    }
    // A null default is an empty one.
    let default = ctx.string(default).map_err(Stop::Fault)?;
    ctx.write_string(out, room, default.trim_end_matches(' '))
        .map_err(Stop::Fault)
}

/// A second thread, which the machine runs cooperatively (ADR-0005).
///
/// One host thread runs them all, switching at a declared quantum of retired
/// instructions: a schedule that depended on the host's own threads would make
/// every run of a threaded title a different run.
fn create_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateThread(security, stack bytes, start, parameter, flags, out id).
    let (start, parameter, flags) = (args[2], args[3], args[4]);

    // A stack of its own, and a thread block of its own for `fs` to be based
    // at. Both come off the guest heap, which is where everything the machine
    // hands a guest comes from.
    let bytes = if args[1] == 0 { THREAD_STACK } else { args[1] };
    let stack = ctx.state.heap.alloc(bytes);
    let block = ctx.state.heap.alloc(0x1000);
    if stack == 0 || block == 0 {
        // ERROR_NOT_ENOUGH_MEMORY, which is what a caller checks for.
        ctx.state.last_error = 8;
        return Ok(0);
    }
    let top = stack + bytes;
    crate::machine::write_tib(ctx.mem, block, top, stack, ctx.state.next_thread_id)
        .map_err(Stop::Fault)?;

    // Its first frame: the argument it was made with, and a return address
    // that ends the thread rather than returning into nothing.
    let esp = top - 16;
    ctx.mem.write_u32(esp + 4, parameter).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(esp, ctx.boundary.bind_thread_end())
        .map_err(Stop::Fault)?;
    let mut cpu = crate::cpu::Cpu {
        eip: start,
        ..crate::cpu::Cpu::default()
    };
    cpu.r[crate::cpu::ESP] = esp;
    cpu.seg_base[crate::cpu::FS] = block;

    let id = ctx.state.next_thread_id + 1;
    ctx.state.next_thread_id = id;
    let handle = ctx.state.give(Object::Named {
        kind: "thread",
        name: format!("thread {id}"),
        signalled: false,
        manual: true,
        held: None,
    });
    // CREATE_SUSPENDED: made, and not run until somebody says so.
    let suspended = flags & 4 != 0;
    ctx.state.threads.push(crate::win32::GuestThread {
        cpu,
        handle,
        id,
        suspended,
        ended: false,
        code: 0,
        tls: Vec::new(),
        wake_at: 0,
        gives_up_at: 0,
        waiting_on: Vec::new(),
    });
    if args[5] != 0 {
        ctx.mem.write_u32(args[5], id).map_err(Stop::Fault)?;
    }
    ctx.log.line(&format!(
        "thread {id} at {start:#010x}, stack {stack:#010x}+{bytes}{}",
        if suspended { ", suspended" } else { "" }
    ));
    Ok(handle)
}

/// How much stack a thread gets when it does not ask for a size.
///
/// A declared value: what an image's own header would have asked for, and
/// enough for each of the threads Alpha Team makes.
const THREAD_STACK: u32 = 0x0010_0000;

fn resume_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let handle = args[0];
    let Some(thread) = ctx.state.threads.iter_mut().find(|t| t.handle == handle) else {
        ctx.state.last_error = 6; // ERROR_INVALID_HANDLE
        return Ok(0xffff_ffff);
    };
    // What the count was before this call, which is one when it was suspended.
    let was = u32::from(thread.suspended);
    thread.suspended = false;
    Ok(was)
}

/// The thread that is running says it is done. The machine takes it out of the
/// rotation at the next switch.
fn exit_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    end_thread(ctx.state, args[0]);
    Err(Stop::ThreadEnded)
}

/// The running thread is done, by `ExitThread` or by returning: it is not run
/// again, and its handle is signalled, as Windows signals it, so a wait on it
/// is over and `GetExitCodeThread` gives what it ended with (#130).
pub(crate) fn end_thread(state: &mut super::State, code: u32) {
    let running = state.thread;
    let Some(thread) = state.threads.get_mut(running) else {
        return;
    };
    thread.ended = true;
    thread.code = code;
    let handle = thread.handle;
    // The thread a title starts on has no handle of its own to signal.
    if let Some(Object::Named { signalled, .. }) = state.object_mut(handle) {
        *signalled = true;
        wake_waiters(state, handle);
    }
}

fn get_exit_code_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetExitCodeThread(handle, out code): STILL_ACTIVE until it has ended.
    // A handle closed, or never given out, is no thread's.
    let thread = ctx.state.threads.iter().find(|t| t.handle == args[0]);
    let Some(thread) = thread.filter(|_| ctx.state.object(args[0]).is_some()) else {
        ctx.state.last_error = 6; // ERROR_INVALID_HANDLE
        return Ok(0);
    };
    let code = if thread.ended { thread.code } else { 259 };
    ctx.mem.write_u32(args[1], code).map_err(Stop::Fault)?;
    Ok(1)
}

fn output_debug_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // What the title's own developers watched. It goes in the run log, where
    // it is the guest's own account of what it is doing.
    let text = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("guest says {}", text.trim_end()));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{
        declared_file_time, declared_now, seek_to, short_date, system_processor,
        time_without_seconds, weekday, DECLARED_DAY,
    };
    use crate::cpu::Processor;

    #[test]
    fn system_info_names_the_processor_cpuid_does() {
        // A Pentium 166: family 5, model 2, stepping C.
        assert_eq!(system_processor(Processor::Pentium), (586, 0x020c_0005));
        // A Pentium II 450, as it always said: family 6, model 5, stepping 2.
        assert_eq!(system_processor(Processor::PentiumII), (586, 0x0502_0006));
    }

    /// 1999-10-29, three minutes and fourteen seconds past midnight, where
    /// Select Adventure saves (#87), with a weekday that is not its own (#91).
    const SAVED: [u16; 8] = [1999, 10, 4, 29, 0, 3, 14, 0];

    #[test]
    fn the_declared_clock_names_one_moment_however_it_is_asked() {
        // 1999-10-29 00:00 universal time is 941,155,200 seconds after 1970,
        // and 1970 is 11,644,473,600 seconds after the start of 1601.
        assert_eq!(
            declared_file_time(0),
            (11_644_473_600 + 941_155_200) * 10_000_000
        );
        assert_eq!(declared_now(0), [1999, 10, 5, 29, 0, 0, 0, 0], "a Friday");

        // Where Select Adventure saves, both ways round (#87).
        assert_eq!(declared_now(194_000), [1999, 10, 5, 29, 0, 3, 14, 0]);
        assert_eq!(
            declared_file_time(194_000),
            declared_file_time(0) + 194_000 * 10_000
        );

        // Past midnight the date turns over with the FILETIME's day, and the
        // weekday with it.
        let later = (25 * 3600 + 3) * 1000 + 7;
        assert_eq!(declared_now(later), [1999, 10, 6, 30, 1, 0, 3, 7]);
        assert_eq!(
            declared_file_time(later) / 864_000_000_000,
            DECLARED_DAY + 1
        );
        // And over the year's end and the next leap day.
        assert_eq!(declared_now(64 * 86_400_000)[..4], [2000, 1, 6, 1]);
        assert_eq!(declared_now(123 * 86_400_000)[..4], [2000, 2, 2, 29]);
    }

    #[test]
    fn a_date_and_a_time_are_written_as_us_english_writes_them() {
        // What Alpha Team's Select Adventure asks for: the short date, and the
        // time without its seconds (#87).
        assert_eq!(short_date(SAVED), "10/29/99");
        assert_eq!(time_without_seconds(SAVED), "12:03 AM");
        assert_eq!(
            time_without_seconds([2005, 3, 0, 7, 13, 5, 9, 0]),
            "1:05 PM"
        );
    }

    #[test]
    fn only_a_day_the_calendar_has_is_written() {
        assert_eq!(weekday(&SAVED), Some(5), "a Friday");
        assert_eq!(weekday(&[2000, 2, 0, 29, 0, 0, 0, 0]), Some(2));
        assert_eq!(weekday(&[1999, 2, 0, 29, 0, 0, 0, 0]), None);
        assert_eq!(weekday(&[1999, 13, 0, 1, 0, 0, 0, 0]), None);
        assert_eq!(weekday(&[1999, 0, 0, 1, 0, 0, 0, 0]), None);
        assert_eq!(weekday(&[1999, 1, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(weekday(&[1600, 1, 0, 1, 0, 0, 0, 0]), None);
    }

    #[test]
    fn a_seek_lands_where_it_is_asked_for_unless_that_is_off_the_file() {
        assert_eq!(seek_to(0, 16), Some(16));
        assert_eq!(seek_to(100, -4), Some(96));
        assert_eq!(seek_to(4, -4), Some(0), "the start of the file is a place");
        // Before the start is nowhere, and a title laying a file out is told
        // so rather than being sent four gigabytes along it.
        assert_eq!(seek_to(0, -4), None);
        assert_eq!(seek_to(3, -4), None);
    }
}
