//! Minimal terminal seam: TTY / `NO_COLOR` / `TERM=dumb` detection and POSIX raw-key input.
//!
//! Zero external crates (ADR-0001). All OS-specific terminal FFI lives behind
//! this module (golden rule 5); platforms without POSIX `termios` support fall
//! back cleanly to line mode.

use std::ffi::OsStr;
use std::io::IsTerminal;

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "x86",
            target_arch = "arm",
            target_arch = "riscv64"
        )
    ),
    test
))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    /// Zero-based index for keys `'1'..='9'`.
    Digit(usize),
    Quit,
    Interrupt,
    Other,
}

#[must_use]
pub fn color_enabled() -> bool {
    let no_color = std::env::var_os("NO_COLOR");
    let term = std::env::var_os("TERM");
    color_enabled_for(
        std::io::stdout().is_terminal(),
        no_color.as_deref(),
        term.as_deref(),
    )
}

#[must_use]
pub fn color_enabled_for(is_tty: bool, no_color: Option<&OsStr>, term: Option<&OsStr>) -> bool {
    is_tty && no_color.is_none_or(OsStr::is_empty) && term != Some(OsStr::new("dumb"))
}

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "x86",
            target_arch = "arm",
            target_arch = "riscv64"
        )
    ),
    test
))]
#[must_use]
pub fn decode_key(first: u8, rest: &[u8]) -> Key {
    match first {
        b'\r' | b'\n' => Key::Enter,
        b'k' | b'K' => Key::Up,
        b'j' | b'J' => Key::Down,
        b'q' | b'Q' | 0x04 => Key::Quit,
        0x03 => Key::Interrupt,
        b'1'..=b'9' => Key::Digit((first - b'1') as usize),
        0x1b => match rest {
            [] | [0x1b] => Key::Quit,
            [b'[', b'A'] | [b'O', b'A'] => Key::Up,
            [b'[', b'B'] | [b'O', b'B'] => Key::Down,
            _ => Key::Other,
        },
        _ => Key::Other,
    }
}

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "x86",
            target_arch = "arm",
            target_arch = "riscv64"
        )
    )
))]
pub mod unix {
    use super::{decode_key, Key};
    use std::ffi::{c_ulong, OsStr};
    use std::io::{self, ErrorKind, IsTerminal, Write};

    const STDIN_FD: i32 = 0;
    const STDOUT_FD: i32 = 1;
    const TCSANOW: i32 = 0;
    const POLLIN: i16 = 0x0001;
    const ESC_POLL_TIMEOUT_MS: i32 = 50;

    #[cfg(target_os = "macos")]
    const NCCS: usize = 20;
    #[cfg(target_os = "macos")]
    const ICANON: u64 = 0x0000_0100;
    #[cfg(target_os = "macos")]
    const ECHO: u64 = 0x0000_0008;
    #[cfg(target_os = "macos")]
    const ISIG: u64 = 0x0000_0080;
    #[cfg(target_os = "macos")]
    const VMIN: usize = 16;
    #[cfg(target_os = "macos")]
    const VTIME: usize = 17;
    #[cfg(target_os = "macos")]
    const TIOCGWINSZ: c_ulong = 0x4008_7468;
    #[cfg(target_os = "macos")]
    type Nfds = u32;

    #[cfg(target_os = "macos")]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Termios {
        c_iflag: u64,
        c_oflag: u64,
        c_cflag: u64,
        c_lflag: u64,
        c_cc: [u8; NCCS],
        c_ispeed: u64,
        c_ospeed: u64,
    }

    #[cfg(target_os = "linux")]
    const NCCS: usize = 32;
    #[cfg(target_os = "linux")]
    const ICANON: u32 = 0x0000_0002;
    #[cfg(target_os = "linux")]
    const ECHO: u32 = 0x0000_0008;
    #[cfg(target_os = "linux")]
    const ISIG: u32 = 0x0000_0001;
    #[cfg(target_os = "linux")]
    const VMIN: usize = 6;
    #[cfg(target_os = "linux")]
    const VTIME: usize = 5;
    #[cfg(target_os = "linux")]
    const TIOCGWINSZ: c_ulong = 0x5413;
    #[cfg(target_os = "linux")]
    type Nfds = usize;

    #[cfg(target_os = "linux")]
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Termios {
        c_iflag: u32,
        c_oflag: u32,
        c_cflag: u32,
        c_lflag: u32,
        c_line: u8,
        c_cc: [u8; NCCS],
        c_ispeed: u32,
        c_ospeed: u32,
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    #[repr(C)]
    struct WinSize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    #[cfg(target_os = "macos")]
    const _: () = assert!(std::mem::size_of::<Termios>() == 72);
    #[cfg(target_os = "linux")]
    const _: () = assert!(std::mem::size_of::<Termios>() == 60);
    const _: () = assert!(std::mem::size_of::<PollFd>() == 8);
    const _: () = assert!(std::mem::size_of::<WinSize>() == 8);

    extern "C" {
        fn tcgetattr(fd: i32, termios_p: *mut Termios) -> i32;
        fn tcsetattr(fd: i32, optional_actions: i32, termios_p: *const Termios) -> i32;
        fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
        fn poll(fds: *mut PollFd, nfds: Nfds, timeout: i32) -> i32;
        fn ioctl(fd: i32, request: c_ulong, ...) -> i32;
    }

    /// RAII guard that puts stdin into non-canonical no-echo mode and restores
    /// original terminal state and cursor visibility unconditionally on drop.
    pub struct RawModeGuard {
        orig: Termios,
    }

    impl RawModeGuard {
        pub fn enter() -> Option<Self> {
            if !io::stdin().is_terminal()
                || !io::stdout().is_terminal()
                || std::env::var_os("TERM").as_deref() == Some(OsStr::new("dumb"))
            {
                return None;
            }
            let mut orig = std::mem::MaybeUninit::<Termios>::zeroed();
            // Safety: `orig` is valid writable storage for `Termios` on this target.
            if unsafe { tcgetattr(STDIN_FD, orig.as_mut_ptr()) } != 0 {
                return None;
            }
            // Safety: `tcgetattr` succeeded and initialized `orig`.
            let orig = unsafe { orig.assume_init() };
            let mut raw = orig;
            raw.c_lflag &= !(ICANON | ECHO | ISIG);
            raw.c_cc[VMIN] = 1;
            raw.c_cc[VTIME] = 0;
            // Safety: `raw` is a valid `Termios` derived from `orig`.
            if unsafe { tcsetattr(STDIN_FD, TCSANOW, &raw) } != 0 {
                return None;
            }
            let mut out = io::stdout();
            let _ = out.write_all(b"\x1b[?25l");
            let _ = out.flush();
            Some(Self { orig })
        }
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            // Safety: `self.orig` was returned by `tcgetattr` for `STDIN_FD`.
            unsafe {
                tcsetattr(STDIN_FD, TCSANOW, &self.orig);
            }
            let mut out = io::stdout();
            let _ = out.write_all(b"\x1b[?25h");
            let _ = out.flush();
        }
    }

    #[must_use]
    pub fn terminal_cols() -> usize {
        let mut ws = WinSize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // Safety: `TIOCGWINSZ` takes a single `*mut WinSize` argument on macOS and Linux.
        let rc = unsafe { ioctl(STDOUT_FD, TIOCGWINSZ, &mut ws as *mut WinSize) };
        if rc == 0 && ws.ws_col > 0 {
            usize::from(ws.ws_col)
        } else {
            80
        }
    }

    fn read_byte() -> Option<u8> {
        let mut byte = 0u8;
        loop {
            // Safety: `byte` is a 1-byte valid writable buffer.
            let n = unsafe { read(STDIN_FD, &mut byte, 1) };
            if n > 0 {
                return Some(byte);
            }
            if n == 0 || io::Error::last_os_error().kind() != ErrorKind::Interrupted {
                return None;
            }
        }
    }

    fn poll_readable(timeout_ms: i32) -> bool {
        let mut pfd = PollFd {
            fd: STDIN_FD,
            events: POLLIN,
            revents: 0,
        };
        loop {
            // Safety: `pfd` points to a single valid `PollFd`.
            let rc = unsafe { poll(&mut pfd, 1, timeout_ms) };
            if rc > 0 {
                return (pfd.revents & POLLIN) != 0;
            }
            if rc == 0 || io::Error::last_os_error().kind() != ErrorKind::Interrupted {
                return false;
            }
        }
    }

    /// Read one keypress directly from fd 0 without `BufReader` pre-buffering
    /// so `poll` accurately separates bare `Esc` from CSI/SS3 arrow sequences.
    pub fn read_key() -> Option<Key> {
        let first = read_byte()?;
        if first != 0x1b {
            return Some(decode_key(first, &[]));
        }
        if !poll_readable(ESC_POLL_TIMEOUT_MS) {
            return Some(Key::Quit);
        }
        let Some(b1) = read_byte() else {
            return Some(Key::Quit);
        };
        if (b1 == b'[' || b1 == b'O') && poll_readable(ESC_POLL_TIMEOUT_MS) {
            if let Some(b2) = read_byte() {
                if (0x20..=0x3f).contains(&b2) {
                    // Drain extended CSI/SS3 parameters (e.g. PageUp `ESC[5~`,
                    // modified arrows, SGR mouse) so trailing digits never trigger picks.
                    for _ in 0..64 {
                        if !poll_readable(ESC_POLL_TIMEOUT_MS) {
                            break;
                        }
                        match read_byte() {
                            Some(b) if (0x40..=0x7e).contains(&b) => break,
                            Some(_) => {}
                            None => break,
                        }
                    }
                    return Some(Key::Other);
                }
                return Some(decode_key(first, &[b1, b2]));
            }
        }
        Some(decode_key(first, &[b1]))
    }
}

#[cfg(test)]
mod tests {
    use super::{color_enabled_for, decode_key, Key};
    use std::ffi::OsStr;

    #[test]
    fn no_color_and_dumb_term_disable_ansi_even_on_tty() {
        assert!(color_enabled_for(
            true,
            None,
            Some(OsStr::new("xterm-256color"))
        ));
        assert!(color_enabled_for(true, Some(OsStr::new("")), None));
        assert!(!color_enabled_for(true, Some(OsStr::new("1")), None));
        assert!(!color_enabled_for(true, Some(OsStr::new("0")), None));
        assert!(!color_enabled_for(true, None, Some(OsStr::new("dumb"))));
        assert!(!color_enabled_for(
            false,
            None,
            Some(OsStr::new("xterm-256color"))
        ));
    }

    #[test]
    fn decodes_arrows_vim_digits_enter_and_quit() {
        assert_eq!(decode_key(0x1b, b"[A"), Key::Up);
        assert_eq!(decode_key(0x1b, b"OA"), Key::Up);
        assert_eq!(decode_key(0x1b, b"[B"), Key::Down);
        assert_eq!(decode_key(0x1b, b"OB"), Key::Down);
        assert_eq!(decode_key(0x1b, b""), Key::Quit);
        assert_eq!(decode_key(0x1b, &[0x1b]), Key::Quit);
        assert_eq!(decode_key(0x1b, b"[C"), Key::Other);
        assert_eq!(decode_key(b'k', b""), Key::Up);
        assert_eq!(decode_key(b'K', b""), Key::Up);
        assert_eq!(decode_key(b'j', b""), Key::Down);
        assert_eq!(decode_key(b'J', b""), Key::Down);
        assert_eq!(decode_key(b'\r', b""), Key::Enter);
        assert_eq!(decode_key(b'\n', b""), Key::Enter);
        assert_eq!(decode_key(b'1', b""), Key::Digit(0));
        assert_eq!(decode_key(b'6', b""), Key::Digit(5));
        assert_eq!(decode_key(b'9', b""), Key::Digit(8));
        assert_eq!(decode_key(b'q', b""), Key::Quit);
        assert_eq!(decode_key(b'Q', b""), Key::Quit);
        assert_eq!(decode_key(0x04, b""), Key::Quit);
        assert_eq!(decode_key(0x03, b""), Key::Interrupt);
        assert_eq!(decode_key(b'x', b""), Key::Other);
    }
}
