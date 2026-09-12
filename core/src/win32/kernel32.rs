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
        name: "GetEnvironmentVariableA",
        args: 3,
        convention: Convention::Stdcall,
        run: get_environment_variable,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetSystemTimeAsFileTime",
        args: 1,
        convention: Convention::Stdcall,
        run: get_system_time_as_file_time,
    },
    Shim {
        dll: "KERNEL32",
        name: "InterlockedIncrement",
        args: 1,
        convention: Convention::Stdcall,
        run: interlocked_increment,
    },
    Shim {
        dll: "KERNEL32",
        name: "InterlockedDecrement",
        args: 1,
        convention: Convention::Stdcall,
        run: interlocked_decrement,
    },
    Shim {
        dll: "KERNEL32",
        name: "InterlockedExchange",
        args: 2,
        convention: Convention::Stdcall,
        run: interlocked_exchange,
    },
    Shim {
        dll: "KERNEL32",
        name: "IsBadReadPtr",
        args: 2,
        convention: Convention::Stdcall,
        run: is_bad_read_ptr,
    },
    Shim {
        dll: "KERNEL32",
        name: "IsBadWritePtr",
        args: 2,
        convention: Convention::Stdcall,
        run: is_bad_read_ptr,
    },
    Shim {
        dll: "KERNEL32",
        name: "IsBadCodePtr",
        args: 1,
        convention: Convention::Stdcall,
        run: is_bad_code_ptr,
    },
    Shim {
        dll: "KERNEL32",
        name: "DisableThreadLibraryCalls",
        args: 1,
        convention: Convention::Stdcall,
        run: disable_thread_library_calls,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetErrorMode",
        args: 1,
        convention: Convention::Stdcall,
        run: set_error_mode,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetStdHandle",
        args: 2,
        convention: Convention::Stdcall,
        run: set_std_handle,
    },
    Shim {
        dll: "KERNEL32",
        name: "FlushFileBuffers",
        args: 1,
        convention: Convention::Stdcall,
        run: flush_file_buffers,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetSystemInfo",
        args: 1,
        convention: Convention::Stdcall,
        run: get_system_info,
    },
    Shim {
        dll: "KERNEL32",
        name: "TerminateProcess",
        args: 2,
        convention: Convention::Stdcall,
        run: terminate_process,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCurrentThread",
        args: 0,
        convention: Convention::Stdcall,
        run: get_current_thread,
    },
    Shim {
        dll: "KERNEL32",
        name: "CreateFileMappingA",
        args: 6,
        convention: Convention::Stdcall,
        run: create_file_mapping,
    },
    Shim {
        dll: "KERNEL32",
        name: "MapViewOfFile",
        args: 5,
        convention: Convention::Stdcall,
        run: map_view_of_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "UnmapViewOfFile",
        args: 1,
        convention: Convention::Stdcall,
        run: unmap_view_of_file,
    },
    Shim {
        dll: "KERNEL32",
        name: "MulDiv",
        args: 3,
        convention: Convention::Stdcall,
        run: mul_div,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetTimeZoneInformation",
        args: 1,
        convention: Convention::Stdcall,
        run: get_time_zone_information,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetLogicalDriveStringsA",
        args: 2,
        convention: Convention::Stdcall,
        run: get_logical_drive_strings,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetCommandLineW",
        args: 0,
        convention: Convention::Stdcall,
        run: get_command_line_w,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetUserDefaultLCID",
        args: 0,
        convention: Convention::Stdcall,
        run: user_default_lcid,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetUserDefaultLangID",
        args: 0,
        convention: Convention::Stdcall,
        run: user_default_lang_id,
    },
    Shim {
        dll: "KERNEL32",
        name: "IsValidCodePage",
        args: 1,
        convention: Convention::Stdcall,
        run: is_valid_code_page,
    },
    Shim {
        dll: "KERNEL32",
        name: "IsValidLocale",
        args: 2,
        convention: Convention::Stdcall,
        run: is_valid_locale,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetLocaleInfoA",
        args: 4,
        convention: Convention::Stdcall,
        run: get_locale_info_a,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetLocaleInfoW",
        args: 4,
        convention: Convention::Stdcall,
        run: get_locale_info_w,
    },
    Shim {
        dll: "KERNEL32",
        name: "EnumSystemLocalesA",
        args: 2,
        convention: Convention::Stdcall,
        run: enum_system_locales,
    },
    Shim {
        dll: "KERNEL32",
        name: "CompareStringA",
        args: 6,
        convention: Convention::Stdcall,
        run: compare_string_a,
    },
    Shim {
        dll: "KERNEL32",
        name: "CompareStringW",
        args: 6,
        convention: Convention::Stdcall,
        run: compare_string_w,
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
        run: enter_critical_section,
    },
    Shim {
        dll: "KERNEL32",
        name: "LeaveCriticalSection",
        args: 1,
        convention: Convention::Stdcall,
        run: leave_critical_section,
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
        name: "CreateEventA",
        args: 4,
        convention: Convention::Stdcall,
        run: create_event,
    },
    Shim {
        dll: "KERNEL32",
        name: "SetEvent",
        args: 1,
        convention: Convention::Stdcall,
        run: set_event,
    },
    Shim {
        dll: "KERNEL32",
        name: "WaitForSingleObject",
        args: 2,
        convention: Convention::Stdcall,
        run: wait_for_single_object,
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
        name: "ResumeThread",
        args: 1,
        convention: Convention::Stdcall,
        run: resume_thread,
    },
    Shim {
        dll: "KERNEL32",
        name: "SuspendThread",
        args: 1,
        convention: Convention::Stdcall,
        run: suspend_thread,
    },
    Shim {
        dll: "KERNEL32",
        name: "ExitThread",
        args: 1,
        convention: Convention::Stdcall,
        run: exit_thread,
    },
    Shim {
        dll: "KERNEL32",
        name: "GetExitCodeThread",
        args: 2,
        convention: Convention::Stdcall,
        run: get_exit_code_thread,
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

fn get_command_line_w(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The same line, two bytes a character: what a C runtime asks for when it
    // was built to handle both.
    let addr = get_command_line(ctx, args)?;
    let line = ctx.string(addr).map_err(Stop::Fault)?;
    let wide: Vec<u8> = line
        .as_bytes()
        .iter()
        .flat_map(|b| [*b, 0])
        .chain([0, 0])
        .collect();
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

/// Whether the machine has a locale. It has the one it declares.
fn is_valid_locale(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The system and user defaults name the declared locale too.
    Ok(u32::from(matches!(
        args[0],
        LCID | 0x0400 | 0x0800 | 0x1000
    )))
}

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
        0x0011 => "M/d/yyyy",
        0x0012 => "h:mm:ss tt",
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
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    let needed = u32::try_from(bytes.len()).unwrap_or(0);
    if args[3] == 0 {
        return Ok(needed);
    }
    if args[3] < needed {
        ctx.state.last_error = 122; // ERROR_INSUFFICIENT_BUFFER
        return Ok(0);
    }
    ctx.mem.write(args[2], &bytes).map_err(Stop::Fault)?;
    Ok(needed)
}

fn get_locale_info_w(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let Some(value) = locale_field(args[1]) else {
        ctx.state.last_error = 1004;
        return Ok(0);
    };
    let wide: Vec<u8> = value
        .as_bytes()
        .iter()
        .flat_map(|b| [*b, 0])
        .chain([0, 0])
        .collect();
    let needed = u32::try_from(wide.len() / 2).unwrap_or(0);
    if args[3] == 0 {
        return Ok(needed);
    }
    if args[3] < needed {
        ctx.state.last_error = 122;
        return Ok(0);
    }
    ctx.mem.write(args[2], &wide).map_err(Stop::Fault)?;
    Ok(needed)
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

/// The declared clock as Windows counts it: hundreds of nanoseconds since the
/// start of 1601.
fn get_system_time_as_file_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The same declared moment `GetSystemTime` answers with — 1999-10-04,
    // plus however long this run has been going — counted the other way.
    const EPOCH: u64 = 11_644_473_600;
    const NINETEEN_NINETY_NINE: u64 = 939_000_000;
    let ticks = (EPOCH + NINETEEN_NINETY_NINE) * 10_000_000 + ctx.declared_millis() * 10_000;
    ctx.mem
        .write(args[0], &ticks.to_le_bytes())
        .map_err(Stop::Fault)?;
    Ok(0)
}

/// Nothing else runs on this machine, so the lock these take is already held
/// (ADR-0005): the read and the write cannot be interrupted.
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

/// A module saying it does not want to hear about threads. There is one thread
/// and its attach has already happened, so there is nothing to stop telling it.
fn disable_thread_library_calls(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

/// Which failures the machine shows a person rather than handing back. Nothing
/// on this machine puts a dialog up, so the mode is kept and never read.
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
    let fields: [(u32, u32); 9] = [
        (0x00, 0),           // x86, and no reserved word
        (0x04, 0x0000_1000), // a four kilobyte page
        (0x08, 0x0001_0000), // the lowest address an application gets
        (0x0c, 0x7fff_ffff), // and the highest
        (0x10, 1),           // one processor, which is the mask of it
        (0x14, 1),           // and one of them
        (0x18, 586),         // a Pentium II, as the declared machine says
        (0x1c, 0x0001_0000), // images are handed out this far apart
        (0x20, 0x0006_0000), // model 6, stepping 0
    ];
    for (offset, value) in fields {
        ctx.mem
            .write(at + offset, &value.to_le_bytes())
            .map_err(Stop::Fault)?;
    }
    Ok(1)
}

/// A process ending itself the hard way, which on this machine is the only
/// process there is.
fn terminate_process(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.log
        .line(&format!("guest terminated itself {}", args[1]));
    Err(Stop::Exited { code: args[1] })
}

/// The handle standing for the thread that is asking, which on this machine is
/// the only one there is.
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
    // Making one and taking it apart again: the structure the guest passes is
    // its own memory and is left exactly as it found it.
    Ok(0)
}

/// Which thread is asking, as the sections know threads.
fn running_thread(ctx: &Ctx<'_>) -> u32 {
    ctx.state
        .threads
        .get(ctx.state.thread)
        .map_or(crate::machine::THREAD_ID, |t| t.id)
}

/// Take a lock, or wait for whoever has it (ADR-0005).
///
/// With one thread this costs nothing, which is what it did when there was
/// only ever one. With two it is the only thing keeping them out of each
/// other's C++ runtime: a section that did nothing let Alpha Team's sound
/// thread and its loader tear the same allocator apart.
fn enter_critical_section(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let me = running_thread(ctx);
    match ctx.state.sections.get_mut(&args[0]) {
        Some((owner, depth)) if *owner == me => {
            // A thread already holding one may take it again, and has to
            // leave it as many times as it took it.
            *depth += 1;
            Ok(0)
        }
        Some(_) => Err(Stop::Blocked),
        None => {
            ctx.state.sections.insert(args[0], (me, 1));
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
    let running = ctx.state.thread;
    Ok(ctx
        .state
        .threads
        .get(running)
        .map_or(crate::machine::THREAD_ID, |t| t.id))
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

fn create_event(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateEventA(security, manual reset, initial state, name). A title
    // makes one for a device to signal it on; on a machine where nothing else
    // runs, nothing ever waits on one for long.
    let name = ctx.string(args[3]).map_err(Stop::Fault)?;
    ctx.state.last_error = 0;
    Ok(ctx.state.give(Object::Named {
        kind: "event",
        name,
    }))
}

fn set_event(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Nothing is waiting to be woken: the only thread there is set it.
    Ok(1)
}

fn wait_for_single_object(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Nothing else runs on this machine: no other thread holds a mutex, and
    // no other process sets an event. A wait is over as soon as it is asked
    // for. WAIT_OBJECT_0, and WAIT_FAILED for a handle nobody was given.
    if ctx.state.object(args[0]).is_none() {
        ctx.state.last_error = 6;
        return Ok(0xffff_ffff);
    }
    Ok(0)
}

fn close_handle(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Letting go of a file the guest wrote is what puts it in the profile,
    // where the next run will read it back: a save outlives its run.
    if let Some(Object::File {
        path,
        bytes,
        writing: true,
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
            Err(e) => ctx.log.line(&format!("file write refused {path}: {e}")),
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
    let path = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.state.current_directory = path;
    Ok(1)
}

/// What a call that could not open a file hands back.
const INVALID_HANDLE: u32 = 0xffff_ffff;
const ERROR_FILE_NOT_FOUND: u32 = 2;

/// The access bit that makes a handle one the guest can write through.
const GENERIC_WRITE: u32 = 0x4000_0000;

/// What a title says it wants done with a file that is or is not there.
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_ALWAYS: u32 = 4;
const TRUNCATE_EXISTING: u32 = 5;

/// The file is already there, and the disc cannot be written to.
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_WRITE_PROTECT: u32 = 19;
/// A seek to before the start of a file, which is nowhere.
const ERROR_NEGATIVE_SEEK: u32 = 131;
/// A handle that cannot be written through.
const ERROR_ACCESS_DENIED: u32 = 5;

fn create_file(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateFileA(path, access, share, security, disposition, flags, template).
    let named = ctx.string(args[0]).map_err(Stop::Fault)?;
    let path = ctx.absolute(&named);
    let writing = args[1] & GENERIC_WRITE != 0;

    // The declared machine's D: is a CD-ROM, and a disc is not written to.
    if writing && path.to_ascii_uppercase().starts_with("D:") {
        ctx.log
            .line(&format!("write refused, D: is a disc: {path}"));
        ctx.state.last_error = ERROR_WRITE_PROTECT;
        return Ok(INVALID_HANDLE);
    }

    // A title about to make a file asks whether one is there already. That is
    // not the pressing failing to hold something, so it is asked without
    // recording a miss: the list of what the declared tree is missing is how
    // the tree learns what it has to ship, and a player's own save is not
    // something it ships (#44). OPEN_ALWAYS keeps what is there, so it reads.
    let making = writing && matches!(args[4], CREATE_NEW | CREATE_ALWAYS | TRUNCATE_EXISTING);
    let there = if making {
        ctx.guest_file_is_there(&path).then(Vec::new)
    } else {
        ctx.open_guest_file(&path)
    };
    let bytes = match (there, args[4]) {
        // CREATE_NEW is a title asking for a file that is not there yet, and
        // it has to be told when one is.
        (Some(_), CREATE_NEW) if writing => {
            ctx.state.last_error = ERROR_FILE_EXISTS;
            return Ok(INVALID_HANDLE);
        }
        // What was there is thrown away, whatever it was.
        (Some(_), CREATE_ALWAYS | TRUNCATE_EXISTING) if writing => Vec::new(),
        (Some(bytes), _) => bytes,
        // Nothing there: only a title that says it is making one gets a
        // handle, which leaves TRUNCATE_EXISTING and OPEN_EXISTING refused.
        (None, CREATE_NEW | CREATE_ALWAYS | OPEN_ALWAYS) if writing => Vec::new(),
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
        bytes,
        at: 0,
        writing,
        wrote: false,
    }))
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
    // WriteFile(handle, buffer, want, wrote, overlapped). What the guest
    // writes is kept in the open file and reaches its profile when the file
    // is closed.
    let (handle, buffer, want, wrote_at) = (args[0], args[1], args[2], args[3]);
    // The handle before the buffer, as `read_file` does: a write through a
    // handle this machine does not hold is refused, and a fault reading the
    // buffer would have ended the run at a wall naming memory rather than
    // answering the call the title actually made (#43).
    match ctx.state.object(handle) {
        Some(Object::File { writing: true, .. } | Object::Std(_)) => {}
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
        Some(Object::File {
            bytes,
            at,
            writing: true,
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
    let (handle, distance, method) = (args[0], args[1] as i32, args[3]);
    let Some(Object::File {
        bytes, at, writing, ..
    }) = ctx.state.object(handle)
    else {
        return Ok(INVALID_HANDLE);
    };
    let len = u32::try_from(bytes.len()).unwrap_or(0);
    let base = match method {
        0 => 0,
        1 => *at,
        _ => len,
    };
    // A title laying a file out seeks past the end of what it has written so
    // far, and what it writes there is what makes the file that long. Reading
    // cannot go past what is there.
    //
    // Before the start of the file is nowhere, and a seek there moves nothing:
    // Windows refuses it, and letting it wrap would put the next write four
    // gigabytes along a file that the machine would then try to make that long.
    let Some(moved) = seek_to(base, distance) else {
        ctx.state.last_error = ERROR_NEGATIVE_SEEK;
        return Ok(INVALID_HANDLE);
    };
    let moved = if *writing { moved } else { moved.min(len) };
    if let Some(Object::File { at, .. }) = ctx.state.object_mut(handle) {
        *at = moved;
    }
    Ok(moved)
}

/// Where a seek from here lands, and nothing when that is off the file.
///
/// Before the start of a file is nowhere: Windows refuses a seek there rather
/// than wrapping, and a wrap would put the next write four gigabytes along a
/// file the machine would then try to make that long.
fn seek_to(base: u32, distance: i32) -> Option<u32> {
    u32::try_from(i64::from(base) + i64::from(distance)).ok()
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

/// A file the guest would rather read as memory than through a handle.
///
/// This title's archives are up to a hundred and twenty megabytes each and it
/// maps every one of them, so the bytes go into the arena where the guest can
/// address them, once, and stay there (#57).
fn create_file_mapping(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // CreateFileMappingA(file, security, protection, high, low, name).
    let Some(Object::File { bytes, path, .. }) = ctx.state.object(args[0]) else {
        // ERROR_INVALID_HANDLE.
        ctx.state.last_error = 6;
        return Ok(0);
    };
    let (bytes, path) = (bytes.clone(), path.clone());
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
    // GetLogicalDriveStringsA(size, buffer). The same two drives
    // `GetLogicalDrives` names in bits: the disk the title is installed on and
    // the drive its pressing is in.
    const DRIVES: &[u8] = b"C:\\\0D:\\\0\0";
    let needed = u32::try_from(DRIVES.len()).unwrap_or(0);
    if args[0] < needed {
        // What it would take, which is what a caller asking with nothing gets.
        return Ok(needed);
    }
    ctx.mem.write(args[1], DRIVES).map_err(Stop::Fault)?;
    // Everything but the last terminator, which is how this one counts.
    Ok(needed - 1)
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
    if thread.wake_at > ctx.clock_millis {
        // Still asleep: the machine comes back to this call until its hour.
        return Err(Stop::Blocked);
    }
    if until > ctx.clock_millis {
        thread.wake_at = until;
        return Err(Stop::Blocked);
    }
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
    // A library the title ships is a guest binary and is loaded and run as
    // one (ADR-0002, #55); one Microsoft wrote that the disc does not carry is
    // written here and has no image to load. Either way the guest gets a
    // handle, and what it does with it decides whether the run goes on.
    let name = ctx.string(args[0]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("load library {name}"));
    // A path is one the installer would have written; only the file at the end
    // of it is the title's own.
    let file = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned();
    if let Some(module) = ctx.modules.base_of_name(&file) {
        return Ok(module);
    }
    let under = ctx.title.install_dir;
    match ctx.modules.load(&file, under, ctx.tree, ctx.mem) {
        Ok(Some(base)) => {
            let imports = ctx
                .tree
                .read(&format!("{under}\\{file}"))
                .and_then(|bytes| crate::pe::imports_of(&bytes, base).ok())
                .unwrap_or_default();
            ctx.modules
                .bind(&imports, ctx.boundary, ctx.mem)
                .map_err(Stop::Fault)?;
            // Its own code attaches it, as it does for the ones loaded before
            // the title started. A dynamic load passes no reserved argument.
            if let Some(entry) = ctx.modules.entry_of(&file).filter(|e| *e != 0) {
                ctx.call_guest = Some((entry, vec![base, 1, 0]));
            }
            Ok(base)
        }
        Ok(None) => Ok(ctx.state.give(Object::Module { name, base: 0 })),
        Err(e) => Err(Stop::NotWritten {
            what: format!("{file} will not load: {e}"),
            caller: 0,
        }),
    }
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
    // What a title walking its own install directory asks: everything under
    // one directory whose name matches a pattern. The declared tree is what it
    // is walking, so what it finds is what it can then open (#57).
    let pattern = ctx.string(args[0]).map_err(Stop::Fault)?;
    let full = ctx.absolute(&pattern);
    let (dir, glob) = match full.rsplit_once('\\') {
        Some((dir, glob)) => (dir.to_owned(), glob.to_owned()),
        None => (ctx.state.current_directory.clone(), full.clone()),
    };
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
    let mut esp = top - 16;
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
    esp = 0;
    let _ = esp;

    let id = ctx.state.next_thread_id + 1;
    ctx.state.next_thread_id = id;
    let handle = ctx.state.give(Object::Named {
        kind: "thread",
        name: format!("thread {id}"),
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
/// enough for the one this title makes.
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

fn suspend_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let handle = args[0];
    let Some(thread) = ctx.state.threads.iter_mut().find(|t| t.handle == handle) else {
        ctx.state.last_error = 6;
        return Ok(0xffff_ffff);
    };
    let was = u32::from(thread.suspended);
    thread.suspended = true;
    Ok(was)
}

/// The thread that is running says it is done. The machine takes it out of the
/// rotation at the next switch.
fn exit_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let running = ctx.state.thread;
    if let Some(thread) = ctx.state.threads.get_mut(running) {
        thread.ended = true;
        thread.code = args[0];
    }
    Err(Stop::ThreadEnded)
}

fn get_exit_code_thread(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let handle = args[0];
    let found = ctx.state.threads.iter().find(|t| t.handle == handle);
    let (ended, code) = found.map_or((true, 0), |t| (t.ended, t.code));
    // STILL_ACTIVE, which is how a caller knows to keep waiting.
    let answer = if ended { code } else { 259 };
    ctx.mem.write_u32(args[1], answer).map_err(Stop::Fault)?;
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
    use super::seek_to;

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
