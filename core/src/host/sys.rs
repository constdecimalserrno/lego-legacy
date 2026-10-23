//! The operating system's own entry points, declared by hand.
//!
//! ADR-0001: no crate is linked, so the handful of libc and dynamic-loader
//! symbols the emulator needs are declared here and nowhere else: `mmap` and
//! `dlopen` on a POSIX host, and on Windows their kernel32 equivalents,
//! `VirtualAlloc` and `LoadLibraryExW`, which std links already (#107). The
//! signals that end a run are heard here too, on a POSIX host (#110).

use super::HostError;

#[cfg(unix)]
pub const PROT_READ: i32 = 0x1;
#[cfg(unix)]
pub const PROT_WRITE: i32 = 0x2;
#[cfg(unix)]
pub const MAP_PRIVATE: i32 = 0x0002;
#[cfg(target_os = "macos")]
pub const MAP_ANON: i32 = 0x1000;
#[cfg(all(unix, not(target_os = "macos")))]
pub const MAP_ANON: i32 = 0x20;

#[cfg(unix)]
pub const RTLD_NOW: i32 = 0x2;

/// `VirtualAlloc`'s `MEM_COMMIT | MEM_RESERVE`, and `PAGE_READWRITE`.
#[cfg(windows)]
const MEM_COMMIT_RESERVE: u32 = 0x1000 | 0x2000;
#[cfg(windows)]
const PAGE_READWRITE: u32 = 0x04;
/// `VirtualFree`'s `MEM_RELEASE`: the whole reservation, given a size of zero.
#[cfg(windows)]
const MEM_RELEASE: u32 = 0x8000;
/// `LoadLibraryExW`'s search of `System32` alone.
#[cfg(windows)]
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0800;

#[cfg(unix)]
extern "C" {
    fn mmap(
        addr: *mut core::ffi::c_void,
        len: usize,
        prot: i32,
        flags: i32,
        fd: i32,
        offset: i64,
    ) -> *mut core::ffi::c_void;
    fn munmap(addr: *mut core::ffi::c_void, len: usize) -> i32;
    fn dlopen(path: *const core::ffi::c_char, mode: i32) -> *mut core::ffi::c_void;
    fn dlsym(
        handle: *mut core::ffi::c_void,
        symbol: *const core::ffi::c_char,
    ) -> *mut core::ffi::c_void;
    fn dlclose(handle: *mut core::ffi::c_void) -> i32;
    fn dlerror() -> *const core::ffi::c_char;
    #[cfg_attr(target_os = "macos", link_name = "__error")]
    #[cfg_attr(target_os = "linux", link_name = "__errno_location")]
    fn errno_location() -> *mut i32;
    fn signal(sig: i32, handler: usize) -> usize;
    fn raise(sig: i32) -> i32;
}

/// `SIG_DFL` and `SIG_IGN`, and the signals that end a run someone is
/// watching: its terminal closing, Ctrl-C and `SIGTERM`, numbered alike on
/// every Unix.
#[cfg(unix)]
const SIG_DFL: usize = 0;
#[cfg(unix)]
const SIG_IGN: usize = 1;
#[cfg(unix)]
const SIGNALS: [i32; 3] = [1, 2, 15];

/// Hear the signals that end a run from outside it ([`super::listen`]).
///
/// A signal the run was started ignoring stays ignored: `nohup` is someone
/// asking a run to outlive its terminal, and a shell starts what it runs in
/// the background deaf to Ctrl-C.
#[cfg(unix)]
pub fn listen() {
    for sig in SIGNALS {
        // Safety: libc's own, with a handler of the shape it asks for.
        // `signal` keeps the handler armed and restarts what it interrupts,
        // on macOS and in glibc and musl alike.
        unsafe {
            if signal(sig, on_signal as *const () as usize) == SIG_IGN {
                signal(sig, SIG_IGN);
            }
        }
    }
}

/// Ask for the close, which is all a handler can safely do: the title is told
/// the next time the host is asked ([`super::Host::poll_input`]), and a
/// second signal ends the run at its next burst,
/// writing its `end` line and digest and giving the pointer back as it ends
/// (#110, #140).
///
/// A third signal, with the run already ending, is the signal let be what it
/// was, so a run that never gets there can still be stopped. That one puts
/// the Mac's pointer back first, since the process is going without its
/// destructors (#45).
#[cfg(unix)]
extern "C" fn on_signal(sig: i32) {
    if super::ask_to_close() < 3 {
        return;
    }
    #[cfg(target_os = "macos")]
    super::mac::way_back::restore();
    // Safety: libc's own, and this process is on its way out.
    unsafe {
        signal(sig, SIG_DFL);
        raise(sig);
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn VirtualAlloc(
        addr: *mut core::ffi::c_void,
        len: usize,
        kind: u32,
        protect: u32,
    ) -> *mut core::ffi::c_void;
    fn VirtualFree(addr: *mut core::ffi::c_void, len: usize, kind: u32) -> i32;
    fn LoadLibraryExW(
        name: *const u16,
        file: *mut core::ffi::c_void,
        flags: u32,
    ) -> *mut core::ffi::c_void;
    fn GetProcAddress(
        module: *mut core::ffi::c_void,
        name: *const core::ffi::c_char,
    ) -> *mut core::ffi::c_void;
    fn FreeLibrary(module: *mut core::ffi::c_void) -> i32;
}

#[cfg(unix)]
fn errno() -> i32 {
    // Safety: the platform's own thread-local errno accessor.
    unsafe { *errno_location() }
}

/// Why the dynamic loader last refused. It says so through `dlerror`, and
/// leaves `errno` holding whatever it held.
#[cfg(unix)]
fn loader_error(what: &'static str) -> HostError {
    // Safety: the loader's own message, read before any other call to it on
    // this thread can replace it.
    let message = unsafe { dlerror() };
    let why = if message.is_null() {
        "no reason given".to_owned()
    } else {
        // Safety: a non-null answer is a NUL-terminated string.
        unsafe { std::ffi::CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    };
    HostError::Loader { what, why }
}

/// Why Windows' loader last refused, in Windows' own words: std reads
/// `GetLastError` and has `FormatMessageW` say it. That sentence never names
/// what was asked for, so the name is said first.
#[cfg(windows)]
fn loader_error(what: &'static str, name: &str) -> HostError {
    let why = std::io::Error::last_os_error();
    HostError::Loader {
        what,
        why: format!("{name}: {why}"),
    }
}

/// Map anonymous, private, readable and writable memory.
///
/// # Errors
/// When the operating system refuses.
#[cfg(unix)]
pub fn map_anon(len: usize) -> Result<(*mut u8, usize), HostError> {
    // Safety: a null hint asks the kernel to choose the address; an anonymous
    // mapping needs no file descriptor.
    let p = unsafe {
        mmap(
            core::ptr::null_mut(),
            len,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANON,
            -1,
            0,
        )
    };
    if p as isize == -1 {
        return Err(HostError::Os {
            what: "mmap",
            errno: errno(),
        });
    }
    Ok((p.cast::<u8>(), len))
}

/// Map anonymous, private, readable and writable memory: reserved and
/// committed in one call, so that every page reads as zero, as `mmap`'s do.
///
/// ponytail: committed whole, so a machine's two-gigabyte arena is charged
/// against the commit limit (memory plus the page file) when it is made,
/// where `mmap` charges nothing until a page is touched. A suite running many
/// machines at once may be refused with error 1455; reserving, and committing
/// a page when it is first touched, is the way past that.
///
/// # Errors
/// When the operating system refuses.
#[cfg(windows)]
pub fn map_anon(len: usize) -> Result<(*mut u8, usize), HostError> {
    // Safety: a null address asks Windows to choose; nothing else is passed.
    let p = unsafe {
        VirtualAlloc(
            core::ptr::null_mut(),
            len,
            MEM_COMMIT_RESERVE,
            PAGE_READWRITE,
        )
    };
    if p.is_null() {
        return Err(HostError::Os {
            what: "VirtualAlloc",
            errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        });
    }
    Ok((p.cast::<u8>(), len))
}

/// Release a mapping made by [`map_anon`].
///
/// # Safety
/// `ptr` and `len` must name a live mapping this call is entitled to release.
#[cfg(unix)]
pub unsafe fn unmap(ptr: *mut u8, len: usize) {
    munmap(ptr.cast::<core::ffi::c_void>(), len);
}

/// Release a mapping made by [`map_anon`]. Windows releases a reservation
/// whole, from its base, so the length is not passed on.
///
/// # Safety
/// `ptr` must be the base of a live mapping this call is entitled to release.
#[cfg(windows)]
pub unsafe fn unmap(ptr: *mut u8, _len: usize) {
    VirtualFree(ptr.cast::<core::ffi::c_void>(), 0, MEM_RELEASE);
}

/// A host library opened by name, and the symbols read out of it.
pub struct Library {
    handle: *mut core::ffi::c_void,
}

impl Library {
    /// Open a host library.
    ///
    /// # Errors
    /// When the dynamic loader cannot open it.
    #[cfg(unix)]
    pub fn open(name: &str) -> Result<Self, HostError> {
        let c = std::ffi::CString::new(name)
            .map_err(|_| HostError::Unsupported("library name contains a NUL"))?;
        // Safety: `c` is a live NUL-terminated string for the call's duration.
        let handle = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            return Err(loader_error("dlopen"));
        }
        Ok(Self { handle })
    }

    /// Open a host library: on Windows, one of the operating system's own,
    /// by its file name, looked for in Windows' own `System32` and nowhere
    /// else.
    ///
    /// `LoadLibraryW` would look beside the executable first, which is where
    /// a player who has run these games before may well have left a
    /// `winmm.dll` or a `ddraw.dll` of someone else's; only the operating
    /// system's own libraries are loaded (ADR-0002), so it is `LoadLibraryExW`
    /// with `LOAD_LIBRARY_SEARCH_SYSTEM32`, which Windows 8 and later have.
    ///
    /// # Errors
    /// When Windows cannot load it.
    #[cfg(windows)]
    pub fn open(name: &str) -> Result<Self, HostError> {
        if name.contains('\0') {
            return Err(HostError::Unsupported("library name contains a NUL"));
        }
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // Safety: `wide` is a live NUL-terminated UTF-16 string for the
        // call's duration, and no file handle is passed.
        let handle = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                core::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if handle.is_null() {
            return Err(loader_error("LoadLibraryExW", name));
        }
        Ok(Self { handle })
    }

    /// A function out of the library, by name.
    ///
    /// # Errors
    /// When the library does not export it.
    ///
    /// # Safety
    /// `T` must be that function's signature.
    pub unsafe fn function<T: Copy>(&self, name: &str) -> Result<T, HostError> {
        let p = self.symbol(name)?;
        // Safety: as the caller promises.
        Ok(unsafe { std::mem::transmute_copy(&p) })
    }

    /// Read one symbol out of the library.
    ///
    /// # Errors
    /// When the library does not export it.
    pub fn symbol(&self, symbol: &str) -> Result<*mut core::ffi::c_void, HostError> {
        let c = std::ffi::CString::new(symbol)
            .map_err(|_| HostError::Unsupported("symbol name contains a NUL"))?;
        // Safety: `c` is a live NUL-terminated string for the call's duration.
        #[cfg(unix)]
        let p = unsafe { dlsym(self.handle, c.as_ptr()) };
        #[cfg(windows)]
        let p = unsafe { GetProcAddress(self.handle, c.as_ptr()) };
        if p.is_null() {
            #[cfg(unix)]
            return Err(loader_error("dlsym"));
            #[cfg(windows)]
            return Err(loader_error("GetProcAddress", symbol));
        }
        Ok(p)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // Safety: the handle came from `dlopen` and is closed once.
        #[cfg(unix)]
        unsafe {
            dlclose(self.handle)
        };
        // Safety: the handle came from `LoadLibraryExW` and is freed once.
        #[cfg(windows)]
        unsafe {
            FreeLibrary(self.handle)
        };
    }
}

#[cfg(test)]
mod tests {
    /// A library that is not there is refused in the loader's own words,
    /// which name it, rather than with an errno the loader never set.
    #[test]
    fn a_library_that_is_not_there_is_refused_in_the_loaders_words() {
        let said = match super::Library::open("/nonexistent/lego-legacy.dylib") {
            Ok(_) => panic!("a library that is not there opened"),
            Err(e) => e.to_string(),
        };
        #[cfg(unix)]
        assert!(said.starts_with("dlopen failed: "), "{said}");
        #[cfg(windows)]
        assert!(said.starts_with("LoadLibraryExW failed: "), "{said}");
        assert!(said.contains("lego-legacy.dylib"), "{said}");
    }
}
