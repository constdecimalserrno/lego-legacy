//! What the Windows backend works out for itself, with no Windows to ask.
//!
//! A key message's scan code, where the picture sits in the window and where
//! a point in the window is on the guest's screen, and how far a raw mouse
//! report says the mouse went, are all arithmetic on numbers Windows hands
//! over. They live here, with no `cfg`, so every host's tests hold them, and
//! the backend that uses them (`windows.rs`) is left with only the calls into
//! Windows (#107).

use super::pointer::content_scale;
use super::Input;

/// The key messages: pressed and let go, and the same with Alt held or for
/// F10, which Windows sends as system keys.
pub const WM_KEYDOWN: u32 = 0x0100;
pub const WM_KEYUP: u32 = 0x0101;
pub const WM_SYSKEYDOWN: u32 = 0x0104;
pub const WM_SYSKEYUP: u32 = 0x0105;

/// F4's virtual key, for Alt-F4.
const VK_F4: usize = 0x73;

/// A raw mouse report that says where the mouse is rather than how far it
/// went, and whether that is across the whole desktop or the primary screen.
pub const MOUSE_MOVE_ABSOLUTE: u16 = 0x01;
pub const MOUSE_VIRTUAL_DESKTOP: u16 = 0x02;

/// The DirectInput number for the key a key message's `lParam` is about.
///
/// Bits 16 to 23 are the key's scan code, which says where the key sits on
/// the keyboard, and that is what DirectInput numbers keys by. Bit 24 says
/// the key's code came after the `0xE0` prefix, which DirectInput writes as
/// `0x80` on the scan code: the arrows, the six keys above them, the right
/// Control and Alt, the Windows keys, and the keypad's divide and Enter. Two
/// keys are the other way round. Num Lock arrives marked extended, though it
/// has no prefix, and Pause arrives unmarked, though its own sequence starts
/// `0xE1`. DirectInput calls them `0x45` and `0xC5`.
///
/// Only a key a route can name reaches the guest, as on macOS
/// (`mac::direct_input_key`); a key that is not in that table reaches it as
/// nothing, and so do the shift presses a keyboard fakes around the grey keys,
/// which would be `0xAA` and `0xB6`.
#[must_use]
pub fn direct_input_key(lparam: u32) -> Option<u32> {
    let scan = (lparam >> 16) & 0xff;
    let extended = lparam & (1 << 24) != 0;
    let code = match (scan, extended) {
        (0x45, true) => 0x45,  // Num Lock
        (0x45, false) => 0xc5, // Pause
        (scan, true) => scan | 0x80,
        (scan, false) => scan,
    };
    crate::route::key_name(code).map(|_| code)
}

/// What a key message is to the guest: the key pressed or let go, or nothing.
///
/// A key held down repeating is still the one press: bit 30 of `lParam` is
/// the key's state before this message, and a press of a key that was
/// already down is a repeat.
#[must_use]
pub fn key(message: u32, lparam: u32) -> Option<Input> {
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    if !down && !matches!(message, WM_KEYUP | WM_SYSKEYUP) {
        return None;
    }
    if down && lparam & (1 << 30) != 0 {
        return None;
    }
    direct_input_key(lparam).map(|code| Input::Key { code, down })
}

/// Whether a key message is Alt-F4, which ends the run as the close button
/// does. Bit 29 of `lParam` is Alt held; with it, F4 comes as a system key.
#[must_use]
pub fn closes(message: u32, wparam: usize, lparam: u32) -> bool {
    message == WM_SYSKEYDOWN && wparam == VK_F4 && lparam & (1 << 29) != 0
}

/// Where the picture goes in a window's client area, as `(x, y, width,
/// height)` in the client's own pixels: as large as fits, its aspect kept,
/// and centred, as the macOS layer's `resizeAspect` puts it. What is left
/// either side is the window's black background.
#[must_use]
pub fn picture(client: (i32, i32), frame: (u32, u32)) -> (i32, i32, i32, i32) {
    let (cw, ch) = (client.0.max(0), client.1.max(0));
    let scale = content_scale((f64::from(cw), f64::from(ch)), frame);
    let fit = |n: u32| (f64::from(n.max(1)) * scale).round() as i32;
    let (w, h) = (fit(frame.0).min(cw), fit(frame.1).min(ch));
    ((cw - w) / 2, (ch - h) / 2, w, h)
}

/// Where a point in the window's client area is on the guest's screen.
///
/// Taken back out of the bars either side and down to the frame's own size,
/// and clamped to its edge rather than handed to the guest as a coordinate it
/// would never see on real hardware. Windows' client origin is its top left
/// corner, as the guest's is, so nothing is turned over the way macOS's has
/// to be.
#[must_use]
pub fn guest_point(client: (i32, i32), frame: (u32, u32), x: i32, y: i32) -> (i32, i32) {
    let (cw, ch) = (f64::from(client.0.max(0)), f64::from(client.1.max(0)));
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    let scale = content_scale((cw, ch), frame);
    let (left, top) = ((cw - fw * scale) / 2.0, (ch - fh * scale) / 2.0);
    let gx = ((f64::from(x) - left) / scale).clamp(0.0, fw - 1.0);
    let gy = ((f64::from(y) - top) / scale).clamp(0.0, fh - 1.0);
    (gx as i32, gy as i32)
}

/// How far a raw mouse report says the mouse went, in screen pixels.
///
/// A mouse says how far it moved, and that is the whole answer. A pen, a
/// remote desktop or a virtual machine's own pointer says where it is
/// instead, as `0..=65535` across the primary screen, or across the whole
/// desktop when `MOUSE_VIRTUAL_DESKTOP` is set; the distance is then how far
/// that is from where its last report put it. The first such report has
/// nothing to be measured from, and moves nothing.
#[must_use]
pub fn raw_moved(
    flags: u16,
    x: i32,
    y: i32,
    last: &mut Option<(i32, i32)>,
    primary: (i32, i32),
    desktop: (i32, i32),
) -> (f64, f64) {
    if flags & MOUSE_MOVE_ABSOLUTE == 0 {
        *last = None;
        return (f64::from(x), f64::from(y));
    }
    let screen = if flags & MOUSE_VIRTUAL_DESKTOP == 0 {
        primary
    } else {
        desktop
    };
    let was = last.replace((x, y));
    let Some((was_x, was_y)) = was else {
        return (0.0, 0.0);
    };
    let pixels =
        |now: i32, was: i32, across: i32| f64::from(now - was) * f64::from(across.max(1)) / 65535.0;
    (pixels(x, was_x, screen.0), pixels(y, was_y, screen.1))
}

#[cfg(test)]
mod tests {
    use super::{
        closes, direct_input_key, guest_point, key, picture, raw_moved, WM_KEYDOWN, WM_KEYUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP,
    };
    use crate::host::Input;

    /// A key message's `lParam`: its scan code, and whether it is extended.
    fn lparam(scan: u32, extended: bool) -> u32 {
        (scan << 16) | (u32::from(extended) << 24) | 1
    }

    #[test]
    fn a_windows_key_reaches_the_guest_as_the_key_in_the_same_place() {
        let dik = |scan, extended| direct_input_key(lparam(scan, extended));
        assert_eq!(dik(0x01, false), Some(0x01), "Escape");
        assert_eq!(dik(0x1e, false), Some(0x1e), "A");
        assert_eq!(dik(0x1c, false), Some(0x1c), "Return");
        assert_eq!(dik(0x39, false), Some(0x39), "Space");
        assert_eq!(dik(0x36, false), Some(0x36), "Right Shift is not extended");
        assert_eq!(dik(0x3e, false), Some(0x3e), "F4");
    }

    /// The extended bit is DirectInput's `0x80`: the arrows and the keys
    /// above them are not the keypad keys that share their scan codes.
    #[test]
    fn an_extended_key_is_not_the_keypad_key_it_shares_a_scan_code_with() {
        let dik = |scan, extended| direct_input_key(lparam(scan, extended));
        assert_eq!(dik(0x48, true), Some(0xc8), "Up");
        assert_eq!(dik(0x48, false), Some(0x48), "Keypad 8");
        assert_eq!(dik(0x4b, true), Some(0xcb), "Left");
        assert_eq!(dik(0x4d, true), Some(0xcd), "Right");
        assert_eq!(dik(0x50, true), Some(0xd0), "Down");
        assert_eq!(dik(0x47, true), Some(0xc7), "Home");
        assert_eq!(dik(0x47, false), Some(0x47), "Keypad 7");
        assert_eq!(dik(0x52, true), Some(0xd2), "Insert");
        assert_eq!(dik(0x53, true), Some(0xd3), "Delete");
        assert_eq!(dik(0x1c, true), Some(0x9c), "Keypad Enter");
        assert_eq!(dik(0x1d, true), Some(0x9d), "Right Control");
        assert_eq!(dik(0x1d, false), Some(0x1d), "Left Control");
        assert_eq!(dik(0x38, true), Some(0xb8), "Right Alt");
        assert_eq!(dik(0x35, true), Some(0xb5), "Keypad divide");
        assert_eq!(dik(0x35, false), Some(0x35), "Slash");
        assert_eq!(dik(0x5b, true), Some(0xdb), "Left Windows");
        assert_eq!(dik(0x5c, true), Some(0xdc), "Right Windows");
        // Num Lock comes marked extended, and is DirectInput's 0x45; Pause
        // comes unmarked, and is 0xC5, which no route names.
        assert_eq!(dik(0x45, true), Some(0x45), "Num Lock");
        assert_eq!(dik(0x45, false), None, "Pause");
        // The shift a keyboard fakes around a grey key is not a key.
        assert_eq!(dik(0x2a, true), None);
        assert_eq!(dik(0x36, true), None);
        // Caps Lock locks rather than holds, and no route names it.
        assert_eq!(dik(0x3a, false), None);
        assert_eq!(dik(0x00, false), None, "and nothing is not a key");
    }

    /// Every key Windows can send is one a route can write by name.
    #[test]
    fn every_key_windows_can_send_a_route_can_name() {
        for scan in 0..=0xff {
            for extended in [false, true] {
                if let Some(code) = direct_input_key(lparam(scan, extended)) {
                    assert!(crate::route::key_name(code).is_some(), "{scan:#x}");
                }
            }
        }
    }

    #[test]
    fn a_key_held_down_repeating_is_still_the_one_press() {
        let space = lparam(0x39, false);
        let again = space | (1 << 30);
        let down = Some(Input::Key {
            code: 0x39,
            down: true,
        });
        let up = Some(Input::Key {
            code: 0x39,
            down: false,
        });
        assert_eq!(key(WM_KEYDOWN, space), down);
        assert_eq!(key(WM_KEYDOWN, again), None, "a repeat");
        // A key let go was down before, and is still the let-go.
        assert_eq!(key(WM_KEYUP, again | (1 << 31)), up);
        // Alt held makes it a system key, which is still the key.
        let alt = lparam(0x38, false) | (1 << 29);
        assert_eq!(
            key(WM_SYSKEYDOWN, alt),
            Some(Input::Key {
                code: 0x38,
                down: true
            })
        );
        assert_eq!(
            key(WM_SYSKEYUP, alt),
            Some(Input::Key {
                code: 0x38,
                down: false
            })
        );
        assert_eq!(key(0x0200, space), None, "a message that is not a key");
    }

    #[test]
    fn alt_f4_ends_the_run_and_f4_alone_does_not() {
        let f4 = lparam(0x3e, false);
        assert!(closes(WM_SYSKEYDOWN, 0x73, f4 | (1 << 29)));
        assert!(!closes(WM_KEYDOWN, 0x73, f4), "F4 alone is a key");
        assert!(!closes(WM_SYSKEYUP, 0x73, f4 | (1 << 29)));
        assert!(!closes(
            WM_SYSKEYDOWN,
            0x72,
            lparam(0x3d, false) | (1 << 29)
        ));
    }

    #[test]
    fn the_picture_is_as_large_as_fits_with_its_aspect_kept_and_centred() {
        assert_eq!(picture((640, 480), (640, 480)), (0, 0, 640, 480));
        // Wider than four by three: bars left and right.
        assert_eq!(picture((1600, 960), (640, 480)), (160, 0, 1280, 960));
        // Taller: bars above and below.
        assert_eq!(picture((640, 800), (640, 480)), (0, 160, 640, 480));
        // Smaller than the frame: scaled down.
        assert_eq!(picture((320, 240), (640, 480)), (0, 0, 320, 240));
        // A minimised window has no room, and nothing goes wrong.
        assert_eq!(picture((0, 0), (640, 480)), (0, 0, 0, 0));
    }

    #[test]
    fn a_point_comes_back_through_the_scale_and_the_bars_without_turning_over() {
        let same = (640, 480);
        assert_eq!(guest_point(same, (640, 480), 0, 0), (0, 0));
        assert_eq!(guest_point(same, (640, 480), 100, 300), (100, 300));
        assert_eq!(guest_point(same, (640, 480), 639, 479), (639, 479));
        // Twice the size, with a hundred and sixty pixels of bar each side.
        let wide = (1600, 960);
        assert_eq!(guest_point(wide, (640, 480), 160, 0), (0, 0));
        assert_eq!(guest_point(wide, (640, 480), 800, 480), (320, 240));
        // In the bar is at the picture's edge; past the window is clamped.
        assert_eq!(guest_point(wide, (640, 480), 10, 480), (0, 240));
        assert_eq!(guest_point(wide, (640, 480), 1700, -5), (639, 0));
    }

    #[test]
    fn a_raw_report_says_how_far_or_where_and_where_becomes_how_far() {
        let mut last = None;
        let screen = (1920, 1080);
        let desktop = (3840, 1080);
        // A mouse: how far, as it is.
        assert_eq!(raw_moved(0, 5, -3, &mut last, screen, desktop), (5.0, -3.0));
        // A virtual machine's pointer: where, first measured from nothing.
        assert_eq!(raw_moved(1, 0, 0, &mut last, screen, desktop), (0.0, 0.0));
        // Then a whole screen's width across, and half its height down.
        let (dx, dy) = raw_moved(1, 65535, 32767, &mut last, screen, desktop);
        assert!((dx - 1920.0).abs() < 1e-9 && (dy - 540.0).abs() < 0.1);
        // Across the whole desktop, the desktop's width.
        let (dx, _) = raw_moved(3, 0, 32767, &mut last, screen, desktop);
        assert!((dx - -3840.0).abs() < 1e-9, "{dx}");
        // A mouse again forgets where the pointer was.
        let _ = raw_moved(0, 1, 1, &mut last, screen, desktop);
        assert_eq!(last, None);
    }
}
