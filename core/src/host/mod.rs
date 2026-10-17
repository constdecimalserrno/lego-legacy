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
pub mod stretch;
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
    Down(Button),
    Up(Button),
    /// A key, numbered as DirectInput numbers the keyboard's keys; a route
    /// names it ([`crate::route::key_name`]).
    Key {
        code: u32,
        down: bool,
    },
}

/// Which of the pointer's buttons a person pressed.
///
/// Two, because this title asks about two: Alpha Team registers a route for
/// the left button and another for the right, and a double-click route for
/// each (#61). A machine with one button answers every question about the
/// second with "not held", which is an answer, and the wrong one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
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

    /// Draw the pointer a person is really moving, or stop: a title that
    /// draws its own wants one pointer on the screen, not two. A host with no
    /// window has nothing to do here.
    fn show_pointer(&mut self, _show: bool) {}

    /// The title put the pointer at a place on the guest's screen.
    ///
    /// What `SetCursorPos` does. A title that steers by how far the mouse
    /// moved puts the pointer back where it measures from after every frame,
    /// so the pointer the host hands the guest starts again from there and
    /// the mouse's next move is measured from it (#59, #64). The person's own
    /// pointer is never moved. A host with no pointer has nothing to do.
    fn put_pointer(&mut self, _x: i32, _y: i32) {}

    /// Whether the person has ended the run from outside it, by closing the
    /// window. A host with no window never has.
    fn closed(&self) -> bool {
        false
    }

    /// How many times a device taking this run's audio ran dry mid-sound, or
    /// `None` for a host with no device.
    ///
    /// Zero is a mix that kept up. Anything else is what a person hears as
    /// crackle and as gaps in a voice line, and it is the host's own
    /// shortfall rather than anything the guest did — a golden hashes what was
    /// mixed, not what was heard (#75).
    fn audio_underruns(&self) -> Option<u64> {
        None
    }

    /// Hold presentation back so that a game-second takes a wall-second
    /// (ADR-0003), and answer with the pace the host settled on: declared
    /// seconds a wall second, 1.0 being real time.
    ///
    /// A host that cannot keep up with a title answers below 1.0, which is
    /// what lets the mix be stretched to match rather than torn (#75). A
    /// headless host does nothing here, runs uncapped, and has no pace.
    fn pace(&mut self, _declared_nanos: u64) -> f64 {
        1.0
    }
}
