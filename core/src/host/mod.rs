//! The host seam (ADR-0001).
//!
//! Everything the emulator touches outside its own process lives behind this
//! trait: the window, input, the audio device, the wall clock, memory mapping
//! and dynamic loading. A new operating system is one implementation of
//! [`Host`] and nothing else.

pub mod headless;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "macos")]
pub mod mac_audio;
pub mod sys;

use std::fmt;

/// One presented frame, as the guest's own surface bytes.
///
/// The pixels are guest memory, not a copy the emulator owns: what the host
/// presents is exactly what the guest would read back.
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// Bytes per row, which may exceed `width * 4` when the guest's surface is
    /// wider than the visible area.
    pub pitch: u32,
    pub pixels: &'a [u8],
}

/// One thing a person did, delivered to the guest at a flip boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Move {
        x: i32,
        y: i32,
    },
    Down,
    Up,
    /// A key, numbered as DirectInput numbers the keyboard's keys; a route
    /// names it ([`crate::route::key_name`]).
    Key {
        code: u32,
        down: bool,
    },
}

/// A region of host memory the guest's arena lives in.
///
/// Owned: dropping it releases the mapping. The host hands one out rather than
/// a bare pointer so that no part of the emulator can outlive the memory it is
/// reading.
pub struct Mapping {
    ptr: *mut u8,
    len: usize,
}

impl Mapping {
    /// # Safety
    /// `ptr` must be the start of a live mapping of `len` bytes, and this
    /// `Mapping` must become its sole owner.
    #[must_use]
    pub unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        Self { ptr, len }
    }

    #[must_use]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // Safety: we are the sole owner by construction, and the pointer and
        // length are the ones the mapping was made with.
        unsafe { sys::unmap(self.ptr, self.len) }
    }
}

/// What the host could not do.
#[derive(Debug)]
pub enum HostError {
    /// The backend does not provide this at all, and says why.
    Unsupported(&'static str),
    /// The operating system refused, with its own errno.
    Os { what: &'static str, errno: i32 },
    /// An operating system framework refused, with the status it returned.
    Status { what: &'static str, status: i32 },
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(why) => write!(f, "unsupported: {why}"),
            Self::Os { what, errno } => write!(f, "{what} failed: errno {errno}"),
            Self::Status { what, status } => write!(f, "{what} failed: status {status}"),
        }
    }
}

impl std::error::Error for HostError {}

/// The real machine the emulator runs on. Everything the guest cannot see.
pub trait Host {
    /// Show the guest's surface. The host scales it to whatever window it has;
    /// the guest never learns the window's size.
    fn present(&mut self, frame: &Frame<'_>);

    /// What a person did since the last flip.
    fn poll_input(&mut self, flip: u64) -> Vec<Input>;

    /// Take one block of mixed audio. Mixing always runs; whether these
    /// samples reach a device is the backend's business.
    fn audio(&mut self, samples: &[i16]);

    /// The host's own clock, in nanoseconds since the run began. The guest
    /// cannot reach this: it exists to pace presentation and to time the
    /// emulator itself.
    fn wall_nanos(&self) -> u64;

    /// Map `len` bytes of anonymous memory for the guest's arena.
    ///
    /// # Errors
    /// When the operating system refuses the mapping.
    fn map(&mut self, len: usize) -> Result<Mapping, HostError>;

    /// Load a host library by name, for a backend that reaches its operating
    /// system through `dlopen` rather than through a link.
    ///
    /// # Errors
    /// When the backend has no dynamic loader, or the library is absent.
    fn load_library(&mut self, name: &str) -> Result<sys::Library, HostError>;

    /// Whether the person has ended the run from outside it, by closing the
    /// window. A host with no window never has.
    fn closed(&self) -> bool {
        false
    }

    /// Hold presentation back so that a game-second takes a wall-second
    /// (ADR-0003). A headless host does nothing here and runs uncapped.
    fn pace(&mut self, _declared_nanos: u64) {}
}
