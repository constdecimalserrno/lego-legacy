//! The host seam (ADR-0001).
//!
//! Everything the emulator touches outside its own process lives in this
//! module. The [`Host`] trait is what a run asks of it: the window, input, the
//! audio device, memory mapping, and holding presentation to a wall-clock pace.
//! The wall clock and dynamic loading are not in it: a backend reads the clock
//! for itself, and [`sys`] declares the loader's entry points. A new operating
//! system is one backend here, implementing [`Host`].
//!
//! What a backend works out without asking its operating system is kept
//! apart from it, with no `cfg`, so that every host's tests hold it: the
//! pointer a title is handed and whose the mouse is ([`pointer`]), the keys
//! it is told of ([`key_change`]), whether a window off the screen is closed
//! ([`closed`]) and whether one is in front ([`in_front`]), the pace
//! ([`pace`]), the feed to an audio device ([`feed`]) and the stretch of the
//! mix ([`stretch`]), Windows' keys and scaling ([`windows_input`]), and X's
//! ([`x11`]).

pub mod feed;
pub mod headless;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "linux")]
pub mod linux_audio;
#[cfg(target_os = "macos")]
pub mod mac;
#[cfg(target_os = "macos")]
pub mod mac_audio;
pub mod pace;
pub mod pointer;
pub mod stretch;
pub mod sys;
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub mod windows_audio;
pub mod windows_input;
pub mod x11;

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

/// How many times the run was asked to end from outside it: Windows' close
/// button, or what the operating system sends a process someone means to
/// stop, which is Ctrl-C, `SIGTERM` and its terminal closing on Unix
/// ([`sys::listen`]), and Ctrl-C, Ctrl-Break and its console closing on
/// Windows (#107, #110). Every host's [`Host::closes`] counts them, so each
/// is answered as the window's close button is: the first is the title's to
/// answer, and a second ends the run there and then, with its `end` line and
/// digest (#140).
pub(crate) static CLOSE_ASKED: AtomicU32 = AtomicU32::new(0);

/// Ask the run to end once more, and say how many times it has been asked.
pub(crate) fn ask_to_close() -> u32 {
    CLOSE_ASKED.fetch_add(1, Ordering::Relaxed) + 1
}

/// Whether a window gone from the screen is the person closing it (#140), for
/// a host that can only tell a close by the window leaving the screen, as
/// macOS's can. A window minimized is off the screen too, and so is every
/// window of an application hidden, and neither is a close: Windows answers a
/// minimize with `SC_MINIMIZE` and an iconic window, never with `SC_CLOSE`
/// (#173).
#[must_use]
pub fn closed(visible: bool, minimized: bool, app_hidden: bool) -> bool {
    !visible && !minimized && !app_hidden
}

/// Whether a window is the one in front (#141), for a host that is told it
/// piece by piece, as macOS's is: on the screen, not minimized, of the active
/// application, and on the Space the person is looking at. A window on
/// another Space is still on the screen, and its application can stay
/// active, but it is out of sight, so it has lost the front (#176).
#[must_use]
pub fn in_front(visible: bool, minimized: bool, app_active: bool, on_active_space: bool) -> bool {
    visible && !minimized && app_active && on_active_space
}

/// Hear the operating system ask a run to end, from now on. A title's `main`
/// does, for a run headless or in a window; the debugger does not, so Ctrl-C
/// there still ends the process. Once.
pub fn listen() {
    #[cfg(unix)]
    sys::listen();
    #[cfg(windows)]
    windows::listen();
}

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
    /// The person asked the window to close: its close button, Command-Q or
    /// -W, Control-Q, Alt-F4, or the run's terminal or console. The title is
    /// told as Windows tells it, and the run ends when the title leaves
    /// (#140).
    Close,
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

/// A key pressed or let go, as the guest is told it, where `held` is the keys
/// it has been told are down. Every backend's keys pass through here.
///
/// A key already held that is pressed again is the key repeating, and is
/// still the one press. A key let go that the guest was never told was
/// pressed goes no further: the key that ends a run with a modifier, the Q of
/// Control-Q and Command-Q, the W of Command-W and the F4 of Alt-F4, is the
/// host's, and so is its release, which reached the guest and the route as
/// the key let go (#108).
pub fn key_change(held: &mut Vec<u32>, code: u32, down: bool) -> Option<Input> {
    if held.contains(&code) == down {
        return None;
    }
    held.retain(|k| *k != code);
    if down {
        held.push(code);
    }
    Some(Input::Key { code, down })
}

/// A pointer shape a title set with `SetCursor`, as the host draws it over
/// the picture (#139).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub width: u32,
    pub height: u32,
    /// The pixel the pointer points with, from the top left.
    pub hot: (u32, u32),
    /// Red, green, blue and alpha a pixel, top row first. Alpha is 0 or 255.
    pub pixels: Vec<u8>,
}

impl Cursor {
    /// No pointer at all: what `SetCursor(NULL)` shows.
    #[must_use]
    pub fn none() -> Self {
        Self {
            width: 1,
            height: 1,
            hot: (0, 0),
            pixels: vec![0; 4],
        }
    }
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
    /// The dynamic loader refused, saying why in its own words.
    Loader { what: &'static str, why: String },
    /// An operating system framework refused, with the status it returned.
    Status { what: &'static str, status: i32 },
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(why) => write!(f, "unsupported: {why}"),
            Self::Os { what, errno } => write!(f, "{what} failed: errno {errno}"),
            Self::Loader { what, why } => write!(f, "{what} failed: {why}"),
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

    /// Map `len` bytes of anonymous memory for the guest's arena.
    ///
    /// # Errors
    /// When the operating system refuses the mapping.
    fn map(&mut self, len: usize) -> Result<Mapping, HostError>;

    /// Draw the pointer a person is really moving, or stop: a title that
    /// draws its own wants one pointer on the screen, not two. So while the
    /// mouse is captured ([`pointer::Capture`], #141) the person's pointer is
    /// hidden and frozen and the title's steered by how far the mouse goes;
    /// for a title that shows the host's, it is kept on the picture
    /// ([`pointer::Mouse`]). A host with no window has nothing to do here.
    fn show_pointer(&mut self, _show: bool) {}

    /// The run is over: the mouse is the person's again, and no click
    /// captures it while the window waits to be closed (#111, #141). A host
    /// with no window has nothing to do here.
    fn free_mouse(&mut self) {}

    /// The title put the pointer at a place on the guest's screen.
    ///
    /// What `SetCursorPos` does. A title that steers by how far the mouse
    /// moved puts the pointer back where it measures from after every frame,
    /// so the pointer the host hands the guest starts again from there and
    /// the mouse's next move is measured from it (#59, #64). The title is told
    /// of that place only while the mouse is captured ([`pointer::Capture`],
    /// #141). The person's own pointer is never moved. A host with no pointer
    /// has nothing to do.
    fn put_pointer(&mut self, _x: i32, _y: i32) {}

    /// The shape the title set its pointer to, or `None` for the host's own
    /// arrow, which is what Windows' own is drawn as.
    ///
    /// Drawn only over the picture, and only while the mouse is captured,
    /// which is only while the window is in front ([`pointer::Capture`],
    /// #141): anywhere else the pointer is the person's, with whatever shape
    /// the desktop gives it (#111). A host with no pointer has nothing to do.
    fn pointer_shape(&mut self, _shape: Option<&Cursor>) {}

    /// How many times the person has asked to end the run from outside it,
    /// by closing the window or by what [`listen`] hears. A host with no
    /// window has only the second. The first close is delivered to the title
    /// at the next flip, as [`Input::Close`]; a second stops the run (#140).
    /// A window minimized, or one of an application hidden, is not closed
    /// ([`closed`], #173).
    fn closes(&self) -> u32 {
        CLOSE_ASKED.load(Ordering::Relaxed)
    }

    /// How many times a device taking this run's audio ran dry mid-sound, or
    /// `None` for a host with no device.
    ///
    /// Zero is a mix that kept up. Anything else is what a person hears as
    /// crackle and as gaps in a voice line, and it is the host's own
    /// shortfall rather than anything the guest did — a test reads what was
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

#[cfg(test)]
mod tests {
    use super::{closed, in_front, key_change, Input};

    #[test]
    fn a_window_off_the_screen_is_closed_only_when_neither_minimized_nor_hidden() {
        assert!(closed(false, false, false), "closed by its button");
        assert!(!closed(true, false, false), "on the screen");
        assert!(!closed(false, true, false), "minimized");
        assert!(!closed(false, false, true), "its application hidden");
    }

    /// A window is in front only on the screen, of the active application,
    /// on the Space the person is looking at (#141, #176).
    #[test]
    fn a_window_on_another_space_is_not_in_front() {
        assert!(in_front(true, false, true, true), "in front");
        assert!(!in_front(false, false, true, true), "off the screen");
        assert!(!in_front(true, true, true, true), "minimized");
        assert!(!in_front(true, false, false, true), "another application's");
        assert!(!in_front(true, false, true, false), "on another Space");
    }

    fn down(code: u32) -> Option<Input> {
        Some(Input::Key { code, down: true })
    }

    fn up(code: u32) -> Option<Input> {
        Some(Input::Key { code, down: false })
    }

    /// A key held down repeating is still the one press, and it is let go
    /// once.
    #[test]
    fn a_held_key_pressed_again_is_the_one_press() {
        let mut held = Vec::new();
        assert_eq!(key_change(&mut held, 0x1e, true), down(0x1e));
        assert_eq!(key_change(&mut held, 0x1e, true), None, "a repeat");
        assert_eq!(key_change(&mut held, 0x2a, true), down(0x2a));
        assert_eq!(key_change(&mut held, 0x1e, false), up(0x1e));
        assert_eq!(held, [0x2a]);
        assert_eq!(
            key_change(&mut held, 0x1e, true),
            down(0x1e),
            "pressed again"
        );
    }

    /// Control-Q: the guest is told of Control pressed and let go, and of
    /// nothing of the Q. The Q pressed ends the run before it gets here, and
    /// the Q let go is the host's too (#108).
    #[test]
    fn the_key_that_ends_a_run_is_not_let_go_in_the_guest() {
        let (control, q) = (0x1d, 0x10);
        let mut held = Vec::new();
        assert_eq!(key_change(&mut held, control, true), down(control));
        assert_eq!(key_change(&mut held, q, false), None, "the Q let go");
        assert_eq!(key_change(&mut held, control, false), up(control));
        assert_eq!(key_change(&mut held, control, false), None, "let go twice");
    }
}
