//! The operating system's own entry points, declared by hand.
//!
//! ADR-0001: no crate is linked, so the handful of libc and dynamic-loader
//! symbols the emulator needs are declared here and nowhere else.

use super::HostError;

pub const PROT_READ: i32 = 0x1;
pub const PROT_WRITE: i32 = 0x2;
pub const MAP_PRIVATE: i32 = 0x0002;
#[cfg(target_os = "macos")]
pub const MAP_ANON: i32 = 0x1000;
#[cfg(not(target_os = "macos"))]
pub const MAP_ANON: i32 = 0x20;

pub const RTLD_NOW: i32 = 0x2;

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
}

fn errno() -> i32 {
    // Safety: the platform's own thread-local errno accessor.
    unsafe { *errno_location() }
}

/// Why the dynamic loader last refused. It says so through `dlerror`, and
/// leaves `errno` holding whatever it held.
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

/// Map anonymous, private, readable and writable memory.
///
/// # Errors
/// When the operating system refuses.
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

/// Release a mapping made by [`map_anon`].
///
/// # Safety
/// `ptr` and `len` must name a live mapping this call is entitled to release.
pub unsafe fn unmap(ptr: *mut u8, len: usize) {
    munmap(ptr.cast::<core::ffi::c_void>(), len);
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
        let p = unsafe { dlsym(self.handle, c.as_ptr()) };
        if p.is_null() {
            return Err(loader_error("dlsym"));
        }
        Ok(p)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // Safety: the handle came from `dlopen` and is closed once.
        unsafe { dlclose(self.handle) };
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
        assert!(said.starts_with("dlopen failed: "), "{said}");
        assert!(said.contains("lego-legacy.dylib"), "{said}");
    }
}
