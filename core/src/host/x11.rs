//! What the X11 backend works out for itself (#108).
//!
//! The backend, [`super::linux`], is compiled only on Linux. What it decides
//! without asking the X server lives here instead, compiled on every host, so
//! that the tests reach it where the backend cannot be run: which DirectInput
//! key an X key is, which keys end a run, where the picture sits in the window
//! and how the guest's frame is scaled into it, which point of the guest's
//! screen a point in the window is, and how far a pointer held in the middle of
//! the window moved.

use super::pointer::content_scale;

/// The DirectInput key number for an X key, by where the key sits on the
/// keyboard rather than what it types, as on the Mac.
///
/// An X keycode is the kernel's evdev key code plus eight on the X servers a
/// stock desktop runs: Xorg with the libinput or evdev driver, and XWayland.
/// The kernel numbers the keys of the PC keyboard's main block, its function
/// keys and its keypad by their set-1 scan codes, which is what DirectInput
/// numbers them by too, so for those the two are the same number; the keys
/// after the grey prefix are listed one by one. A key no route can name
/// ([`crate::route::key_name`]) reaches the guest as nothing, and those are
/// the ones the Mac leaves out too: Caps Lock, Scroll Lock, Print Screen,
/// Pause and the menu key.
///
/// The modifiers are keys like any other here: X sends a press and a release
/// for each, left and right apart.
// ponytail: evdev keycodes only. A server on the old `kbd` driver numbers the
// keys after the grey prefix differently; read the keymap's keycodes name
// (`XkbGetNames`) and keep a second table if one is ever met.
#[must_use]
pub fn direct_input_key(keycode: u32) -> Option<u32> {
    let evdev = keycode.checked_sub(8)?;
    Some(match evdev {
        // Escape to Space, F1 to F10 and Num Lock, the keypad, the key an ISO
        // keyboard has beside Left Shift, F11 and F12: the same numbers.
        1..=57 | 59..=69 | 71..=83 | 86..=88 => evdev,
        96 => 0x9c,  // Keypad Enter
        97 => 0x9d,  // Right Control
        98 => 0xb5,  // Keypad /
        100 => 0xb8, // Right Alt
        102 => 0xc7, // Home
        103 => 0xc8, // Up
        104 => 0xc9, // Page Up
        105 => 0xcb, // Left
        106 => 0xcd, // Right
        107 => 0xcf, // End
        108 => 0xd0, // Down
        109 => 0xd1, // Page Down
        110 => 0xd2, // Insert
        111 => 0xd3, // Delete
        117 => 0x8d, // Keypad =
        125 => 0xdb, // Left Super, as Left Windows
        126 => 0xdc, // Right Super, as Right Windows
        183 => 0x64, // F13
        184 => 0x65, // F14
        185 => 0x66, // F15
        _ => return None,
    })
}

/// `XK_q` and `XK_Q`.
const KEYSYM_Q: [u64; 2] = [0x71, 0x51];

/// `ControlMask`, in a key event's modifier state.
const CONTROL: u32 = 1 << 2;

/// Whether a key pressed ends the run: Control-Q.
///
/// Read by the character the key types, its first keysym, rather than where
/// it sits, so it holds on any keyboard layout, as Command-Q does on the Mac
/// (#100). Control is a key the guest may use, so it still reaches the guest;
/// the Q pressed with it does not.
#[must_use]
pub fn quits(keysym: u64, state: u32) -> bool {
    state & CONTROL != 0 && KEYSYM_Q.contains(&keysym)
}

/// Where the guest's frame is shown in the window, in the window's pixels:
/// scaled to fit and centred, its aspect kept, with black bars either side or
/// above and below, as the Mac's layer shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The picture a frame of this size makes in a window of this size.
#[must_use]
pub fn picture(window: (u32, u32), frame: (u32, u32)) -> Picture {
    let scale = content_scale((f64::from(window.0), f64::from(window.1)), frame);
    let fit = |n: u32, room: u32| {
        let room = room.max(1);
        ((f64::from(n.max(1)) * scale).round() as u32).clamp(1, room)
    };
    let (width, height) = (fit(frame.0, window.0), fit(frame.1, window.1));
    let bar = |room: u32, shown: u32| i32::try_from((room.max(1) - shown) / 2).unwrap_or(0);
    Picture {
        x: bar(window.0, width),
        y: bar(window.1, height),
        width,
        height,
    }
}

/// Where a point in the window is on the guest's screen: out of the bars and
/// down to the frame's own size. The window's origin is its top left corner,
/// as the guest's is, so nothing turns over. A point in the bars or outside
/// the window is at the picture's edge, as on the Mac.
#[must_use]
pub fn guest_point(window: (u32, u32), frame: (u32, u32), x: i32, y: i32) -> (i32, i32) {
    let p = picture(window, frame);
    let on = |at: i32, from: i32, shown: u32, n: u32| {
        let n = i64::from(n.max(1));
        let at = (i64::from(at) - i64::from(from)) * n / i64::from(shown.max(1));
        i32::try_from(at.clamp(0, n - 1)).unwrap_or(0)
    };
    (on(x, p.x, p.width, frame.0), on(y, p.y, p.height, frame.1))
}

/// The guest's frame scaled to `shown`, nearest pixel, four bytes a pixel as
/// the guest drew them, row after row into `out`.
///
/// Every pixel shown is the guest pixel [`guest_point`] takes that place in
/// the window back to, so what a person clicks is what they see. A row the
/// frame is too short to hold is left black rather than read past.
pub fn scale(pixels: &[u8], pitch: usize, frame: (u32, u32), shown: (u32, u32), out: &mut Vec<u8>) {
    let (fw, fh) = (frame.0.max(1) as usize, frame.1.max(1) as usize);
    let (w, h) = (shown.0 as usize, shown.1 as usize);
    out.clear();
    out.resize(w * h * 4, 0);
    if w == 0 || h == 0 {
        return;
    }
    let columns: Vec<usize> = (0..w).map(|x| x * fw / w * 4).collect();
    for (y, row) in out.chunks_exact_mut(w * 4).enumerate() {
        let from = y * fh / h * pitch;
        let Some(source) = pixels.get(from..from + fw * 4) else {
            continue;
        };
        for (pixel, &at) in row.as_chunks_mut::<4>().0.iter_mut().zip(&columns) {
            pixel.copy_from_slice(&source[at..at + 4]);
        }
    }
}

/// The pointer while the title has it taken: held in the middle of the
/// window and moved back there whenever it strays, so that how far it went is
/// the whole message, as it is for the Mac's frozen pointer (#40, #64).
///
/// X has no frozen pointer, only one that is moved back, and a move back is
/// itself seen as a move. Moves the person made before the server got to the
/// move back are still on their way when it is asked for, so every move is
/// measured from the last place the pointer was seen, and the first one that
/// lands exactly on the middle after a move back was asked for is taken to be
/// that move back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    at: (i32, i32),
    middle: (i32, i32),
    moving_back: bool,
}

/// The middle of a window this size.
fn middle(window: (u32, u32)) -> (i32, i32) {
    let half = |n: u32| i32::try_from(n / 2).unwrap_or(0);
    (half(window.0), half(window.1))
}

impl Held {
    /// The pointer is taken where it was last seen, `at`, in a window this
    /// size. It is moved to the middle once it strays ([`Held::stray`]),
    /// which may be at once.
    #[must_use]
    pub fn new(at: (i32, i32), window: (u32, u32)) -> Self {
        Self {
            at,
            middle: middle(window),
            moving_back: false,
        }
    }

    /// Where the pointer is held.
    #[must_use]
    pub fn middle(&self) -> (i32, i32) {
        self.middle
    }

    /// The pointer was seen here: how far it moved since it was last seen,
    /// if it moved, and if this was not the move back.
    pub fn seen(&mut self, x: i32, y: i32) -> Option<(i32, i32)> {
        if self.moving_back && (x, y) == self.middle {
            self.moving_back = false;
            self.at = (x, y);
            return None;
        }
        let moved = (x - self.at.0, y - self.at.1);
        self.at = (x, y);
        (moved != (0, 0)).then_some(moved)
    }

    /// Where to move the pointer back to, once it has strayed a quarter of
    /// the window from the middle: soon enough that a hand never meets the
    /// window's edge between two looks at the window, which would cut its
    /// move short.
    // ponytail: moved back once a pump, so a hand crossing more than a
    // quarter of the window between two pumps meets the edge and loses the
    // rest. XInput2's raw motion has no edge, if a person ever feels it.
    pub fn stray(&mut self, window: (u32, u32)) -> Option<(i32, i32)> {
        self.middle = middle(window);
        let far = |at: i32, middle: i32, n: u32| (at - middle).unsigned_abs() > (n / 4).max(1);
        (far(self.at.0, self.middle.0, window.0) || far(self.at.1, self.middle.1, window.1)).then(
            || {
                self.moving_back = true;
                self.middle
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{direct_input_key, guest_point, picture, quits, scale, Held, Picture};

    #[test]
    fn an_x_key_reaches_the_guest_as_the_key_in_the_same_place() {
        // X keycodes as the evdev keymap names them.
        assert_eq!(direct_input_key(9), Some(0x01)); // Escape
        assert_eq!(direct_input_key(38), Some(0x1e)); // A
        assert_eq!(direct_input_key(36), Some(0x1c)); // Return
        assert_eq!(direct_input_key(50), Some(0x2a)); // Left Shift
        assert_eq!(direct_input_key(62), Some(0x36)); // Right Shift
        assert_eq!(direct_input_key(37), Some(0x1d)); // Left Control
        assert_eq!(direct_input_key(105), Some(0x9d)); // Right Control
        assert_eq!(direct_input_key(64), Some(0x38)); // Left Alt
        assert_eq!(direct_input_key(108), Some(0xb8)); // Right Alt
        assert_eq!(direct_input_key(111), Some(0xc8)); // Up
        assert_eq!(direct_input_key(113), Some(0xcb)); // Left
        assert_eq!(direct_input_key(114), Some(0xcd)); // Right
        assert_eq!(direct_input_key(116), Some(0xd0)); // Down
        assert_eq!(direct_input_key(104), Some(0x9c)); // Keypad Enter
        assert_eq!(direct_input_key(87), Some(0x4f)); // Keypad 1
        assert_eq!(direct_input_key(95), Some(0x57)); // F11
        assert_eq!(direct_input_key(133), Some(0xdb)); // Left Super
        assert_eq!(direct_input_key(66), None); // Caps Lock
        assert_eq!(direct_input_key(0), None);

        // Every key a person can press here, a route can write by name.
        for keycode in 0..=255 {
            if let Some(code) = direct_input_key(keycode) {
                assert!(crate::route::key_name(code).is_some(), "keycode {keycode}");
            }
        }
    }

    /// Control-Q ends a run by the character the key types, wherever it is.
    #[test]
    fn control_q_ends_the_run_and_q_alone_is_a_key() {
        let control = 1 << 2;
        let shift = 1;
        assert!(quits(0x71, control));
        assert!(quits(0x71, control | shift));
        assert!(quits(0x51, control));
        assert!(!quits(0x71, 0), "Q alone is the title's");
        assert!(!quits(0x77, control), "Control-W is the title's");
    }

    #[test]
    fn the_picture_fits_the_window_centred_with_its_aspect_kept() {
        let at = |x, y, width, height| Picture {
            x,
            y,
            width,
            height,
        };
        assert_eq!(picture((640, 480), (640, 480)), at(0, 0, 640, 480));
        assert_eq!(picture((1600, 960), (640, 480)), at(160, 0, 1280, 960));
        assert_eq!(picture((640, 600), (640, 480)), at(0, 60, 640, 480));
        assert_eq!(picture((1000, 750), (640, 480)), at(0, 0, 1000, 750));
        // A window with no room still has a picture, a pixel of it.
        assert_eq!(picture((0, 0), (640, 480)), at(0, 0, 1, 1));
    }

    #[test]
    fn a_point_comes_back_through_the_scale_and_the_bars() {
        let same = (640, 480);
        assert_eq!(guest_point(same, (640, 480), 0, 0), (0, 0));
        assert_eq!(guest_point(same, (640, 480), 320, 479), (320, 479));
        // Twice the size, with a hundred and sixty pixels of bar each side.
        let wide = (1600, 960);
        assert_eq!(guest_point(wide, (640, 480), 160, 0), (0, 0));
        assert_eq!(guest_point(wide, (640, 480), 800, 480), (320, 240));
        assert_eq!(guest_point(wide, (640, 480), 1439, 959), (639, 479));
        // In the bar is at the picture's edge; outside the window is too.
        assert_eq!(guest_point(wide, (640, 480), 10, 480), (0, 240));
        assert_eq!(guest_point(wide, (640, 480), 1590, 480), (639, 240));
        assert_eq!(guest_point(same, (640, 480), -5, 600), (0, 479));
    }

    /// What a person sees at a place in the window is the guest pixel a
    /// click there lands on.
    #[test]
    fn what_is_shown_at_a_place_is_what_a_click_there_lands_on() {
        // A 3x2 frame, each pixel its own colour, with a pitch wider than the
        // frame.
        let frame = (3, 2);
        let pitch = 16;
        let mut pixels = vec![0u8; pitch * 2];
        for y in 0..2 {
            for x in 0..3 {
                let at = y * pitch + x * 4;
                pixels[at..at + 4].copy_from_slice(&[x as u8, y as u8, 0x80, 0]);
            }
        }
        for window in [(3, 2), (6, 4), (7, 5), (10, 3), (4, 9)] {
            let p = picture(window, frame);
            let mut out = Vec::new();
            scale(&pixels, pitch, frame, (p.width, p.height), &mut out);
            assert_eq!(out.len(), (p.width * p.height * 4) as usize);
            for dy in 0..p.height {
                for dx in 0..p.width {
                    let (gx, gy) = guest_point(window, frame, p.x + dx as i32, p.y + dy as i32);
                    let at = ((dy * p.width + dx) * 4) as usize;
                    assert_eq!(
                        out[at..at + 4],
                        [gx as u8, gy as u8, 0x80, 0],
                        "window {window:?} at ({dx}, {dy})"
                    );
                }
            }
        }
    }

    /// A frame shorter than it says it is shows black where it is missing,
    /// rather than being read past (#94).
    #[test]
    fn a_short_frame_leaves_its_missing_rows_black() {
        let pixels = [0xffu8; 8];
        let mut out = Vec::new();
        scale(&pixels, 8, (2, 2), (2, 2), &mut out);
        assert_eq!(out, [[0xff; 8], [0; 8]].concat());
    }

    /// A held pointer measures every move from where it was last seen, and
    /// the move back to the middle is not a move.
    #[test]
    fn a_held_pointer_measures_every_move_from_where_it_was_seen() {
        let window = (640, 480);
        let mut held = Held::new((100, 100), window);
        assert_eq!(held.middle(), (320, 240));
        assert_eq!(
            held.stray(window),
            Some((320, 240)),
            "taken far from the middle"
        );
        assert_eq!(held.seen(320, 240), None, "the move back");
        assert_eq!(held.seen(330, 245), Some((10, 5)));
        assert_eq!(held.stray(window), None, "not far yet");
        assert_eq!(held.seen(500, 240), Some((170, -5)));
        assert_eq!(held.stray(window), Some((320, 240)), "a quarter away");
        // A move the person made before the server moved it back.
        assert_eq!(held.seen(510, 240), Some((10, 0)));
        assert_eq!(held.seen(320, 240), None, "the move back");
        assert_eq!(held.seen(321, 240), Some((1, 0)));
        // A move that lands on the middle with no move back asked for is a
        // move.
        assert_eq!(held.seen(320, 240), Some((-1, 0)));
    }
}
