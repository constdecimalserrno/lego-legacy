//! The macOS backend (ADR-0001).
//!
//! Everything here is reached through `dlopen` and `objc_msgSend`: AppKit for
//! the window and its events, Core Graphics for turning the guest's surface
//! into something a layer will show. No crate is linked, and nothing outside
//! the operating system is loaded.
//!
//! The window presents through a layer rather than a Metal device on purpose.
//! What the guest draws is already a finished frame in its own memory; a layer
//! scales it, keeps its aspect, and asks nothing of us per pixel.

use super::{sys, Button, Frame, Host, HostError, Input, Mapping};
use std::ffi::{c_char, c_void, CStr, CString};
use std::time::Instant;

type Id = *mut c_void;
type Sel = *mut c_void;

/// A rectangle as AppKit spells it.
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// `objc_msgSend` has no one signature, so each call shape gets its own.
///
/// Calling it through the wrong one is how a working program starts returning
/// nonsense on one architecture and not another, which is why there is no
/// generic sender here.
type MsgId = unsafe extern "C" fn(Id, Sel) -> Id;
type MsgVoid = unsafe extern "C" fn(Id, Sel);
type MsgVoidId = unsafe extern "C" fn(Id, Sel, Id);
type MsgVoidBool = unsafe extern "C" fn(Id, Sel, bool);
type MsgVoidI64 = unsafe extern "C" fn(Id, Sel, i64);
type MsgIdRect = unsafe extern "C" fn(Id, Sel, Rect, u64, u64, bool) -> Id;
type MsgIdCharPtr = unsafe extern "C" fn(Id, Sel, *const c_char) -> Id;
type MsgIdEvent = unsafe extern "C" fn(Id, Sel, u64, Id, Id, bool) -> Id;
type MsgU64 = unsafe extern "C" fn(Id, Sel) -> u64;
type MsgCharPtr = unsafe extern "C" fn(Id, Sel) -> *const c_char;
type MsgPoint = unsafe extern "C" fn(Id, Sel) -> Point;
type MsgRect = unsafe extern "C" fn(Id, Sel) -> Rect;
type MsgF64 = unsafe extern "C" fn(Id, Sel) -> f64;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}

/// The Objective-C runtime and the two frameworks, opened by name.
struct Runtime {
    /// Kept open for as long as the runtime is used: closing it would take the
    /// selectors and classes with it.
    _objc: sys::Library,
    _appkit: sys::Library,
    core_graphics: sys::Library,
    get_class: unsafe extern "C" fn(*const c_char) -> Id,
    sel: unsafe extern "C" fn(*const c_char) -> Sel,
    msg: *mut c_void,
}

impl Runtime {
    fn open() -> Result<Self, HostError> {
        let objc = sys::Library::open("/usr/lib/libobjc.A.dylib")?;
        let appkit = sys::Library::open("/System/Library/Frameworks/AppKit.framework/AppKit")?;
        let core_graphics =
            sys::Library::open("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")?;
        // Safety: these are the runtime's own entry points, with the
        // signatures its headers give them.
        let get_class = unsafe {
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> Id>(
                objc.symbol("objc_getClass")?,
            )
        };
        let sel = unsafe {
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*const c_char) -> Sel>(
                objc.symbol("sel_registerName")?,
            )
        };
        let msg = objc.symbol("objc_msgSend")?;
        Ok(Self {
            _objc: objc,
            _appkit: appkit,
            core_graphics,
            get_class,
            sel,
            msg,
        })
    }

    fn class(&self, name: &str) -> Id {
        let c = CString::new(name).unwrap_or_default();
        // Safety: a NUL-terminated name, and the runtime returns null for one
        // it does not know.
        unsafe { (self.get_class)(c.as_ptr()) }
    }

    fn sel(&self, name: &str) -> Sel {
        let c = CString::new(name).unwrap_or_default();
        // Safety: as above; the runtime interns the selector.
        unsafe { (self.sel)(c.as_ptr()) }
    }

    /// Reinterpret `objc_msgSend` as one particular call shape.
    ///
    /// # Safety
    /// `T` must be the signature of the method actually being called.
    unsafe fn send<T: Copy>(&self) -> T {
        debug_assert_eq!(
            std::mem::size_of::<T>(),
            std::mem::size_of::<*mut c_void>(),
            "a message send is a function pointer"
        );
        unsafe { std::mem::transmute_copy(&self.msg) }
    }

    fn nil(&self) -> Id {
        std::ptr::null_mut()
    }

    /// A framework function, by name.
    ///
    /// # Safety
    /// `T` must be that function's signature.
    unsafe fn graphics<T: Copy>(&self, name: &str) -> Result<T, HostError> {
        // Safety: as the caller promises.
        unsafe { self.core_graphics.function(name) }
    }
}

/// How Core Graphics is told to read a presented frame: four bytes a pixel,
/// little-endian, so blue first, and the top byte skipped rather than taken
/// as alpha: `kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst`.
const PRESENT_BITMAP_INFO: u32 = (2 << 12) | 6;

/// How much wall time the pace is worked out over: long enough that one slow
/// flip is not a pace change, short enough that the mix follows the picture
/// (#75). Half a second is about fifteen flips on a host that is struggling.
const PACE_NANOS: u64 = 500_000_000;

/// Where a point in the window's content is on the guest's screen.
///
/// The window shows the guest's frame scaled to fit and centred, its aspect
/// kept, so a point is taken back through that: out of the bars either side,
/// down to the frame's own size, and turned over, because the window's origin
/// is its bottom left corner and the guest's is its top left. A point outside
/// the picture is clamped to its edge rather than handed to the guest as a
/// coordinate it would never see on real hardware.
#[must_use]
pub fn guest_point(content: (f64, f64), frame: (u32, u32), x: f64, y: f64) -> (i32, i32) {
    let (cw, ch) = content;
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    let scale = content_scale(content, frame);
    let (left, bottom) = ((cw - fw * scale) / 2.0, (ch - fh * scale) / 2.0);
    let gx = ((x - left) / scale).clamp(0.0, fw - 1.0);
    let gy = ((ch - y - bottom) / scale).clamp(0.0, fh - 1.0);
    (gx as i32, gy as i32)
}

/// How many points of the window's content one guest pixel covers.
#[must_use]
pub fn content_scale(content: (f64, f64), frame: (u32, u32)) -> f64 {
    let (cw, ch) = content;
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    (cw / fw).min(ch / fh).max(f64::MIN_POSITIVE)
}

/// How far past the edge of the screen the hand may run, in screens, for a
/// title that puts the pointer back every frame ([`Steer::moved`]). Paid back
/// half a screen a frame, so this is also how long a wild swing can go on
/// after the hand has stopped: eight frames.
const OWED_SCREENS: f64 = 4.0;

/// The pointer the guest is handed while the pointer is taken.
///
/// The person's own pointer is frozen, so this is the only one that moves,
/// and it moves by how far the mouse went rather than by where the person's
/// pointer is (#64). Two numbers, and the difference between them is what
/// the title has not been told yet: `hand` is where the hand has got to, in
/// guest pixels with the fraction kept, and `told` is the place on the screen
/// the guest was last handed. The hand may be past the edge of the screen; a
/// place never is.
///
/// `speed` is the declared machine's own pointer speed
/// ([`crate::title::Title::pointer_speed`]): how much pointer there is in a
/// push of the mouse, which on a real machine is a setting of the machine and
/// here is a declared value. A title that only moves its own pointer by a
/// fraction of what it is handed has that fraction put back here, so that the
/// pointer a person watches goes where their hand goes (#86).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steer {
    hand: (f64, f64),
    told: (i32, i32),
    /// The place the guest had when input was last taken from the host.
    /// Everything up to it the title has read; everything past it is still
    /// to come.
    drained: (i32, i32),
    speed: (f64, f64),
    /// Whether the title put the pointer somewhere since input was last
    /// taken, and whether it had the time before. A title that puts the
    /// pointer back every frame is steering by distance, and is owed every
    /// distance the screen had no room to say (#86); one that leaves the
    /// pointer where it is has it stop at the screen's edge, as the cursor
    /// stops at the display's, and a hand pushing past that edge is owed
    /// nothing when it comes back (#40).
    put_since_drain: bool,
    puts_back: bool,
}

impl Steer {
    #[must_use]
    pub fn new(at: (i32, i32), speed: (f64, f64)) -> Self {
        Self {
            hand: (f64::from(at.0), f64::from(at.1)),
            told: at,
            drained: at,
            speed,
            put_since_drain: false,
            puts_back: false,
        }
    }

    /// The place on the screen the hand is at: the edge, when it is past it.
    fn place(&self, frame: (u32, u32)) -> (i32, i32) {
        let on = |had: f64, n: u32| had.floor().clamp(0.0, f64::from(n.max(1)) - 1.0) as i32;
        (on(self.hand.0, frame.0), on(self.hand.1, frame.1))
    }

    /// Hand the guest the place the hand is at, once it has reached another
    /// pixel.
    fn tell(&mut self, frame: (u32, u32)) -> Option<(i32, i32)> {
        let now = self.place(frame);
        (now != self.told).then(|| {
            self.told = now;
            now
        })
    }

    /// The mouse moved: the place to hand the guest, once the pointer has
    /// reached another pixel.
    ///
    /// The move is in points of the window's content, so it is scaled to
    /// guest pixels, and the fraction of a pixel is kept between moves. Where
    /// the hand may go depends on what the title does with the pointer. One
    /// that leaves it where it is has it stop at the screen's edge, so that a
    /// hand pushing past the edge and coming back moves it at once (#40). One
    /// that puts it back in the middle every frame is told a distance, and
    /// half a screen is all a place can say of one in a frame — at twice
    /// speed and thirty frames a second, a hand crossing 160 points in a
    /// frame, which a person playing this way does in a third of every fast
    /// movement (#86). So the hand may run past the edge, and what the
    /// screen had no room for is said at the next put, half a screen a
    /// frame, until it is all said: a flick lands where the hand went, a few
    /// frames late, rather than short of it.
    ///
    /// ponytail: [`OWED_SCREENS`] of it and no more, so that a wild swing
    /// plays out in a fraction of a second rather than going on after the
    /// hand has stopped. Faster than that in one frame is lost, as it is on
    /// a real machine where the cursor is against the edge of the display.
    pub fn moved(
        &mut self,
        moved: (f64, f64),
        content: (f64, f64),
        frame: (u32, u32),
    ) -> Option<(i32, i32)> {
        let scale = content_scale(content, frame);
        let go = |had: f64, moved: f64, speed: f64, n: u32| {
            let edge = f64::from(n.max(1)) - 1.0;
            let (lo, hi) = if self.puts_back {
                (-OWED_SCREENS * edge, (OWED_SCREENS + 1.0) * edge)
            } else {
                (0.0, edge)
            };
            (had + moved * speed / scale).clamp(lo, hi)
        };
        self.hand = (
            go(self.hand.0, moved.0, self.speed.0, frame.0),
            go(self.hand.1, moved.1, self.speed.1, frame.1),
        );
        self.tell(frame)
    }

    /// The pointer is the person's own and it is here: the hand starts again
    /// from where it is.
    pub fn placed(&mut self, at: (i32, i32)) {
        self.hand = (f64::from(at.0), f64::from(at.1));
        self.told = at;
    }

    /// The title put the pointer somewhere: the place to hand the guest, if
    /// what the hand had not been told about reaches past it.
    ///
    /// What the title has read is everything up to the place the guest had
    /// when input was last taken; the rest of the hand's movement is still to
    /// come, and it starts again from where the title put the pointer. The
    /// fraction of a pixel comes with it — a title that puts the pointer back
    /// every flip and steers by how far it went since would otherwise drop up
    /// to a pixel of movement twenty-five times a second (#64). So does what
    /// the screen had no room for (#86). And so does a move taken from the
    /// window since input was last drained, which was measured from a place
    /// the title has now moved on from: the window is pumped when a frame is
    /// presented as well as when input is taken, and a move pumped before the
    /// put and one pumped after it reach the title together, which reads only
    /// the last — a third of a steady hand's movement, at the window (#86).
    pub fn put(&mut self, to: (i32, i32), frame: (u32, u32)) -> Option<(i32, i32)> {
        self.put_since_drain = true;
        self.hand = (
            f64::from(to.0) + self.hand.0 - f64::from(self.drained.0),
            f64::from(to.1) + self.hand.1 - f64::from(self.drained.1),
        );
        self.told = to;
        self.drained = to;
        self.tell(frame)
    }

    /// Input is being taken from the host: the place the guest has now is
    /// the one the title reads next.
    pub fn drained(&mut self) {
        self.drained = self.told;
        self.puts_back = std::mem::take(&mut self.put_since_drain);
    }
}

/// What a mouse-moved event is for, once the window system has said whose
/// window it is and the run has said whether it has taken the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Moved {
    /// The pointer is frozen and the mouse's distance is the whole message.
    Steered,
    /// The pointer is the person's and where it is is where it is.
    Placed,
    /// The person's pointer is somewhere else entirely.
    Elsewhere,
}

/// **A taken pointer is frozen, so it is nowhere and cannot have left.**
///
/// While it is taken the event's own window says nothing — the pointer is not
/// over anything, by construction — and reading it as "the person has gone"
/// gives the pointer back, which puts the very next move down the placing path
/// and teleports the guest's pointer to wherever the person's frozen one
/// happens to sit. A title that puts its pointer back every frame then has it
/// dragged between that place and this one, frame after frame, which is what a
/// person feels as a mouse that will not follow theirs (#40, #64).
#[must_use]
pub fn moved(taken: bool, ours: bool) -> Moved {
    match (taken, ours) {
        (true, _) => Moved::Steered,
        (false, true) => Moved::Placed,
        (false, false) => Moved::Elsewhere,
    }
}

/// The DirectInput key number for a Mac key, by where the key sits on the
/// keyboard rather than what it types; the keypad's operators by what they
/// are, as a game binds them.
///
/// Letters, digits, punctuation, the arrows, the function keys, the keypad
/// and the keys above the arrows. The modifiers come as flag changes rather
/// than key presses ([`direct_input_modifier`]); a key that is not here reaches
/// the guest as nothing.
#[must_use]
pub fn direct_input_key(key_code: u64) -> Option<u32> {
    Some(match key_code {
        0x00 => 0x1e, // A
        0x01 => 0x1f, // S
        0x02 => 0x20, // D
        0x03 => 0x21, // F
        0x04 => 0x23, // H
        0x05 => 0x22, // G
        0x06 => 0x2c, // Z
        0x07 => 0x2d, // X
        0x08 => 0x2e, // C
        0x09 => 0x2f, // V
        0x0a => 0x56, // §, where an ISO PC keyboard has its extra key
        0x0b => 0x30, // B
        0x0c => 0x10, // Q
        0x0d => 0x11, // W
        0x0e => 0x12, // E
        0x0f => 0x13, // R
        0x10 => 0x15, // Y
        0x11 => 0x14, // T
        0x12 => 0x02, // 1
        0x13 => 0x03, // 2
        0x14 => 0x04, // 3
        0x15 => 0x05, // 4
        0x16 => 0x07, // 6
        0x17 => 0x06, // 5
        0x18 => 0x0d, // =
        0x19 => 0x0a, // 9
        0x1a => 0x08, // 7
        0x1b => 0x0c, // -
        0x1c => 0x09, // 8
        0x1d => 0x0b, // 0
        0x1e => 0x1b, // ]
        0x1f => 0x18, // O
        0x20 => 0x16, // U
        0x21 => 0x1a, // [
        0x22 => 0x17, // I
        0x23 => 0x19, // P
        0x24 => 0x1c, // Return
        0x25 => 0x26, // L
        0x26 => 0x24, // J
        0x27 => 0x28, // '
        0x28 => 0x25, // K
        0x29 => 0x27, // ;
        0x2a => 0x2b, // \
        0x2b => 0x33, // ,
        0x2c => 0x35, // /
        0x2d => 0x31, // N
        0x2e => 0x32, // M
        0x2f => 0x34, // .
        0x30 => 0x0f, // Tab
        0x31 => 0x39, // Space
        0x32 => 0x29, // `
        0x33 => 0x0e, // Backspace
        0x35 => 0x01, // Escape
        0x41 => 0x53, // Keypad .
        0x43 => 0x37, // Keypad *
        0x45 => 0x4e, // Keypad +
        0x47 => 0x45, // Keypad Clear, where a PC keypad has Num Lock
        0x4b => 0xb5, // Keypad /
        0x4c => 0x9c, // Keypad Enter
        0x4e => 0x4a, // Keypad -
        0x51 => 0x8d, // Keypad =
        0x52 => 0x52, // Keypad 0
        0x53 => 0x4f, // Keypad 1
        0x54 => 0x50, // Keypad 2
        0x55 => 0x51, // Keypad 3
        0x56 => 0x4b, // Keypad 4
        0x57 => 0x4c, // Keypad 5
        0x58 => 0x4d, // Keypad 6
        0x59 => 0x47, // Keypad 7
        0x5b => 0x48, // Keypad 8
        0x5c => 0x49, // Keypad 9
        0x60 => 0x3f, // F5
        0x61 => 0x40, // F6
        0x62 => 0x41, // F7
        0x63 => 0x3d, // F3
        0x64 => 0x42, // F8
        0x65 => 0x43, // F9
        0x67 => 0x57, // F11
        0x69 => 0x64, // F13
        0x6b => 0x65, // F14
        0x6d => 0x44, // F10
        0x6f => 0x58, // F12
        0x71 => 0x66, // F15
        0x72 => 0xd2, // Help, where a PC keyboard has Insert
        0x73 => 0xc7, // Home
        0x74 => 0xc9, // Page Up
        0x75 => 0xd3, // Forward Delete
        0x76 => 0x3e, // F4
        0x77 => 0xcf, // End
        0x78 => 0x3c, // F2
        0x79 => 0xd1, // Page Down
        0x7a => 0x3b, // F1
        0x7b => 0xcb, // Left
        0x7c => 0xcd, // Right
        0x7d => 0xd0, // Down
        0x7e => 0xc8, // Up
        _ => return None,
    })
}

/// The key a modifier's flag change is about, pressed or let go.
///
/// A flag change names the key that changed. The device-dependent bits of the
/// flags say which side of the keyboard is held, so left and right stay apart;
/// an event without them, as one sent from another machine can be, says only
/// whether the modifier is held at all. Caps Lock locks rather than holds and
/// Fn has no DirectInput key: neither is here.
// ponytail: without side bits, a left and a right held together read as one
// modifier; the first let go is not seen until the second is.
#[must_use]
pub fn direct_input_modifier(key_code: u64, flags: u64) -> Option<Input> {
    // The DirectInput key, this side's bit, both sides' bits, and the bit
    // for the modifier itself.
    let (code, side_bit, both_sides, modifier_bit) = match key_code {
        0x36 => (0xdc, 0x10, 0x18, 1 << 20), // Right Command, as Right Windows
        0x37 => (0xdb, 0x08, 0x18, 1 << 20), // Left Command, as Left Windows
        0x38 => (0x2a, 0x02, 0x06, 1 << 17), // Left Shift
        0x3a => (0x38, 0x20, 0x60, 1 << 19), // Left Option, as Left Alt
        0x3b => (0x1d, 0x01, 0x2001, 1 << 18), // Left Control
        0x3c => (0x36, 0x04, 0x06, 1 << 17), // Right Shift
        0x3d => (0xb8, 0x40, 0x60, 1 << 19), // Right Option, as Right Alt
        0x3e => (0x9d, 0x2000, 0x2001, 1 << 18), // Right Control
        _ => return None,
    };
    let down = if flags & both_sides == 0 {
        flags & modifier_bit != 0
    } else {
        flags & side_bit != 0
    };
    Some(Input::Key { code, down })
}

/// Hand a presented frame's copy back once Core Graphics is done with it.
///
/// Core Graphics calls this with the pointer given as the provider's `info`,
/// which is the boxed copy made in `present`.
extern "C" fn release_pixels(info: *mut c_void, _data: *const c_void, _size: usize) {
    if info.is_null() {
        return;
    }
    // Safety: `info` is the pointer `Box::into_raw` produced in `present`, and
    // Core Graphics calls this exactly once.
    unsafe {
        drop(Box::from_raw(info.cast::<Vec<u8>>()));
    }
}

/// A window on the host's screen, showing what the guest drew.
pub struct MacWindow {
    rt: Runtime,
    app: Id,
    window: Id,
    layer: Id,
    started: Instant,
    /// Events a person made since the last flip.
    pending: Vec<Input>,
    /// The keys the guest has been told are down.
    held: Vec<u32>,
    /// Where the mix is heard, when `--audio` asked for it.
    audio: Option<super::mac_audio::MacAudio>,
    /// Whether the person closed the window.
    closed: bool,
    /// Whether we have hidden the pointer. AppKit counts hides, so it is
    /// hidden once and shown once.
    pointer_hidden: bool,
    /// Whether the person's own pointer is over this window.
    ///
    /// A title that draws its own pointer wants the host's out of the way, but
    /// only where its own is drawn: everywhere else the pointer is the
    /// person's and they are using it. Starts true, because a window that has
    /// just opened under the pointer has not been told otherwise.
    pointer_over_us: bool,
    /// Whether the host's pointer is hidden right now, which is not the same
    /// as the title having asked for it: it is hidden only while the title
    /// wants it gone *and* the person is over this window with this
    /// application in front.
    pointer_taken: bool,
    /// The pointer the guest is handed while the pointer is taken ([`Steer`]).
    steer: Steer,
    cg_associate: unsafe extern "C" fn(bool) -> i32,
    cg_show_cursor: unsafe extern "C" fn(u32) -> i32,
    cg_main_display: u32,
    /// How far behind the declared clock the host has fallen and will not
    /// make up: a run the host could not keep pace with carries on in real
    /// time from where it got to, rather than racing to catch up.
    lag_nanos: u64,
    /// Where the wall and the declared clock were when the pace was last
    /// worked out, and the pace it came to: declared seconds a wall second
    /// (#75).
    paced_from: (u64, u64),
    pace: f64,
    /// Carries the mix's own tail from one stretched block to the next.
    stretch: crate::host::stretch::Stretch,
    /// The size of the last frame presented, which the window scales.
    frame_size: (u32, u32),
    colour_space: *mut c_void,
    cg_image_create: unsafe extern "C" fn(
        usize,
        usize,
        usize,
        usize,
        usize,
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        bool,
        i32,
    ) -> *mut c_void,
    cg_provider: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut c_void) -> *mut c_void,
    cg_provider_release: unsafe extern "C" fn(*mut c_void),
    cf_release: unsafe extern "C" fn(*mut c_void),
}

/// Which button an AppKit event is about: 3 and 4 are the right one's down
/// and up, 1 and 2 the left one's.
fn button(kind: u64) -> Button {
    if matches!(kind, 3 | 4) {
        Button::Right
    } else {
        Button::Left
    }
}

impl MacWindow {
    /// Open a window of this size, titled after the title being run, with the
    /// declared machine's own pointer speed ([`Steer`]).
    ///
    /// # Errors
    /// When the frameworks cannot be opened, or the window cannot be made.
    pub fn open(
        name: &str,
        width: u32,
        height: u32,
        pointer_speed: (f64, f64),
    ) -> Result<Self, HostError> {
        let rt = Runtime::open()?;

        // Safety: each call below is made through the signature its own
        // method has, with arguments of the types AppKit declares.
        unsafe {
            let app_class = rt.class("NSApplication");
            let app: Id = rt.send::<MsgId>()(app_class, rt.sel("sharedApplication"));
            // A regular application: it gets a menu bar and can take focus.
            rt.send::<MsgVoidI64>()(app, rt.sel("setActivationPolicy:"), 0);

            let frame = Rect {
                x: 0.0,
                y: 0.0,
                width: f64::from(width),
                height: f64::from(height),
            };
            let window_class = rt.class("NSWindow");
            let window: Id = rt.send::<MsgId>()(window_class, rt.sel("alloc"));
            // Titled, closable, miniaturisable, resizable.
            let style = 1 | 2 | 4 | 8;
            let window: Id = rt.send::<MsgIdRect>()(
                window,
                rt.sel("initWithContentRect:styleMask:backing:defer:"),
                frame,
                style,
                2,
                false,
            );

            let title = CString::new(name).unwrap_or_default();
            let string_class = rt.class("NSString");
            let title: Id = rt.send::<MsgIdCharPtr>()(
                string_class,
                rt.sel("stringWithUTF8String:"),
                title.as_ptr(),
            );
            rt.send::<MsgVoidId>()(window, rt.sel("setTitle:"), title);

            let view: Id = rt.send::<MsgId>()(window, rt.sel("contentView"));
            rt.send::<MsgVoidBool>()(view, rt.sel("setWantsLayer:"), true);
            let layer: Id = rt.send::<MsgId>()(view, rt.sel("layer"));

            // The layer scales the guest's frame to the window and keeps its
            // aspect, so resizing is the host's business and the guest never
            // learns the window changed.
            let gravity = CString::new("resizeAspect").unwrap_or_default();
            let gravity: Id = rt.send::<MsgIdCharPtr>()(
                string_class,
                rt.sel("stringWithUTF8String:"),
                gravity.as_ptr(),
            );
            rt.send::<MsgVoidId>()(layer, rt.sel("setContentsGravity:"), gravity);
            let nearest = CString::new("nearest").unwrap_or_default();
            let nearest: Id = rt.send::<MsgIdCharPtr>()(
                string_class,
                rt.sel("stringWithUTF8String:"),
                nearest.as_ptr(),
            );
            rt.send::<MsgVoidId>()(layer, rt.sel("setMagnificationFilter:"), nearest);

            rt.send::<MsgVoidId>()(window, rt.sel("makeKeyAndOrderFront:"), rt.nil());
            rt.send::<MsgVoidBool>()(app, rt.sel("activateIgnoringOtherApps:"), true);
            rt.send::<MsgVoid>()(app, rt.sel("finishLaunching"));

            let create_space: unsafe extern "C" fn(*const c_char) -> *mut c_void =
                rt.graphics("CGColorSpaceCreateWithName")?;
            let name = rt.core_graphics.symbol("kCGColorSpaceSRGB")?;
            let colour_space = create_space(*name.cast::<*const c_char>());

            let cg_associate = rt.graphics("CGAssociateMouseAndMouseCursorPosition")?;
            let cg_show_cursor: unsafe extern "C" fn(u32) -> i32 =
                rt.graphics("CGDisplayShowCursor")?;
            let main_display: unsafe extern "C" fn() -> u32 = rt.graphics("CGMainDisplayID")?;
            let cg_main_display = main_display();
            Ok(Self {
                cg_associate,
                cg_show_cursor,
                cg_main_display,
                cg_image_create: rt.graphics("CGImageCreate")?,
                cg_provider: rt.graphics("CGDataProviderCreateWithData")?,
                cg_provider_release: rt.graphics("CGDataProviderRelease")?,
                cf_release: rt.graphics("CGImageRelease")?,
                colour_space,
                rt,
                app,
                window,
                layer,
                started: Instant::now(),
                pending: Vec::new(),
                held: Vec::new(),
                audio: None,
                closed: false,
                pointer_hidden: false,
                pointer_over_us: true,
                pointer_taken: false,
                steer: Steer::new(
                    (
                        i32::try_from(width / 2).unwrap_or(320),
                        i32::try_from(height / 2).unwrap_or(240),
                    ),
                    pointer_speed,
                ),
                lag_nanos: 0,
                paced_from: (0, 0),
                pace: 1.0,
                stretch: crate::host::stretch::Stretch::default(),
                frame_size: (width, height),
            })
        }
    }

    /// Whether the person has closed the window.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Let the mix be heard through the Mac's output from now on, or say why
    /// it cannot be.
    ///
    /// # Errors
    /// When there is no output to open.
    pub fn open_audio(&mut self) -> Result<(), HostError> {
        self.audio = Some(super::mac_audio::MacAudio::open()?);
        Ok(())
    }

    /// Stop the output, once there is no more mix to hear.
    pub fn close_audio(&mut self) {
        self.audio = None;
    }

    /// Put the host's pointer where it belongs, which is wherever the person
    /// is not being shown the title's own.
    ///
    /// A title that draws its own pointer wants the host's out of the way, but
    /// only over its own window and only while it is the application in front.
    /// Anywhere else the pointer is the person's and they are using it — to
    /// reach another window, to stop the run, to do something else entirely —
    /// and taking it from them there is taking their machine.
    ///
    /// Hiding is a count on AppKit's side, so this keeps its own flag and only
    /// ever moves it by one.
    fn draw_pointer_where_it_belongs(&mut self) {
        // Safety: `isActive` on NSApplication. A BOOL is one byte, and the
        // rest of the register is not ours to read.
        let active = unsafe {
            let v: u64 = self.rt.send::<MsgU64>()(self.app, self.rt.sel("isActive"));
            v & 0xff != 0
        };
        let take = self.pointer_hidden && self.pointer_over_us && active;
        if take == self.pointer_taken {
            return;
        }
        self.pointer_taken = take;
        // A run that takes the pointer arms the way back, because the pointer
        // belongs to the person and not to the run: a signal that ends the
        // process skips every destructor, and what is left behind is a machine
        // with no pointer and nothing on screen saying why (#45).
        if take {
            arm_the_way_back(self.cg_associate, self.cg_show_cursor, self.cg_main_display);
        }
        // Safety: NSCursor's own class methods, which take nothing; then Core
        // Graphics' switch between the mouse and the pointer.
        unsafe {
            let cursor = self.rt.class("NSCursor");
            let what = if take { "hide" } else { "unhide" };
            self.rt.send::<MsgVoid>()(cursor, self.rt.sel(what));
            // A taken pointer is also a frozen one: the mouse stops moving the
            // person's pointer and moves the guest's instead, by how far it
            // went (`Steer`). That is what a title steering by distance
            // needs, and it is why a hand can never outrun the window (#64,
            // #40). It is switched off for the whole desktop, which is why it
            // is only ever on while the title has asked, the person is over
            // this window and this application is in front — settled on every
            // pump, and undone by `arm_the_way_back` if the run is killed.
            (self.cg_associate)(!take);
        }
    }

    /// Where an event in our window landed on the guest's screen, or nothing
    /// when it landed on the window's frame rather than on the picture: the
    /// title bar, the edges a person resizes by. The frame is the window
    /// system's and the pointer over it is the person's (#49).
    fn on_the_picture(&self, event: Id) -> Option<(i32, i32)> {
        // Safety: AppKit's own accessors, on an event in this window and on
        // the window's content view; a point and two rectangles come back by
        // value.
        unsafe {
            let p: Point = self.rt.send::<MsgPoint>()(event, self.rt.sel("locationInWindow"));
            let view: Id = self.rt.send::<MsgId>()(self.window, self.rt.sel("contentView"));
            let frame: Rect = self.rt.send::<MsgRect>()(view, self.rt.sel("frame"));
            let inside = p.x >= frame.x
                && p.x < frame.x + frame.width
                && p.y >= frame.y
                && p.y < frame.y + frame.height;
            inside.then(|| {
                guest_point(
                    (frame.width, frame.height),
                    self.frame_size,
                    p.x - frame.x,
                    p.y - frame.y,
                )
            })
        }
    }

    /// Whether an event happened in our own window. One that did not is one
    /// the pointer was outside the window for, and its location is in screen
    /// coordinates.
    fn event_is_ours(&self, event: Id) -> bool {
        // Safety: `window` on an NSEvent, which answers a window or nil.
        let window: Id = unsafe { self.rt.send::<MsgId>()(event, self.rt.sel("window")) };
        std::ptr::eq(window, self.window)
    }

    /// Take everything the window system has for us, turning what a person did
    /// into guest input and noticing a close.
    pub fn pump(&mut self) {
        // Safety: the standard event loop, called with AppKit's own types.
        unsafe {
            loop {
                let distant_past: Id =
                    self.rt.send::<MsgId>()(self.rt.class("NSDate"), self.rt.sel("distantPast"));
                let mode: Id = self.rt.send::<MsgIdCharPtr>()(
                    self.rt.class("NSString"),
                    self.rt.sel("stringWithUTF8String:"),
                    c"kCFRunLoopDefaultMode".as_ptr(),
                );
                let event: Id = self.rt.send::<MsgIdEvent>()(
                    self.app,
                    self.rt
                        .sel("nextEventMatchingMask:untilDate:inMode:dequeue:"),
                    u64::MAX,
                    distant_past,
                    mode,
                    true,
                );
                if event.is_null() {
                    break;
                }
                let kind: u64 = self.rt.send::<MsgU64>()(event, self.rt.sel("type"));
                // A key this window took is a key the title read, and handing
                // it on to AppKit as well means offering it to a responder
                // chain in which nothing answers keys: what comes back is the
                // system beep, once a keystroke, which is what a person
                // driving with the arrow keys hears (#47). A key held with
                // Command is not the title's, so that one is still offered.
                let command = matches!(kind, 10 | 11) && {
                    const COMMAND: u64 = 1 << 20;
                    let flags: u64 = self.rt.send::<MsgU64>()(event, self.rt.sel("modifierFlags"));
                    flags & COMMAND != 0
                };
                let ours_alone = matches!(kind, 10 | 11) && !command;
                // Command-Q and Command-W are how a person quits or closes the
                // window, and with no main menu nothing in AppKit answers them,
                // so they end the run here, as closing the window does. They
                // are read by the character the key types, not where it sits,
                // so they hold on any keyboard layout (#100).
                if kind == 10 && command {
                    let typed: Id =
                        self.rt.send::<MsgId>()(event, self.rt.sel("charactersIgnoringModifiers"));
                    let utf8: *const c_char =
                        self.rt.send::<MsgCharPtr>()(typed, self.rt.sel("UTF8String"));
                    if !utf8.is_null()
                        && matches!(CStr::from_ptr(utf8).to_bytes(), b"q" | b"Q" | b"w" | b"W")
                    {
                        self.closed = true;
                        continue;
                    }
                }
                match kind {
                    // A move of nobody's window carries screen coordinates
                    // rather than the window's, so taking one would throw the
                    // pointer somewhere else. Ignored, the pointer stays where
                    // it last was inside the window, which is what a title
                    // scrolling its camera from a pointer held at an edge
                    // needs. A button is not ignored: a press that is dropped
                    // is a button the guest never sees let go of.
                    // Either button down, either button up. A press on the
                    // window's own frame — its title bar, its edges — is the
                    // person taking hold of the window, not a click on the
                    // title (#49). A release is never dropped: a press the
                    // guest saw is a button it has to see let go of.
                    1 | 3 => {
                        if self.on_the_picture(event).is_some() {
                            self.pending.push(Input::Down(button(kind)));
                        }
                    }
                    2 | 4 => self.pending.push(Input::Up(button(kind))),
                    // Moved and dragged, which is one of three things
                    // ([`moved`]).
                    5..=7 => {
                        match moved(self.pointer_taken, self.event_is_ours(event)) {
                            Moved::Steered => {
                                // The person's pointer is frozen over the picture,
                                // so where the event is says nothing; how far the
                                // mouse moved is the whole message.
                                let dx: f64 =
                                    self.rt.send::<MsgF64>()(event, self.rt.sel("deltaX"));
                                let dy: f64 =
                                    self.rt.send::<MsgF64>()(event, self.rt.sel("deltaY"));
                                let view: Id = self.rt.send::<MsgId>()(
                                    self.window,
                                    self.rt.sel("contentView"),
                                );
                                let frame: Rect =
                                    self.rt.send::<MsgRect>()(view, self.rt.sel("frame"));
                                if let Some((x, y)) = self.steer.moved(
                                    (dx, dy),
                                    (frame.width, frame.height),
                                    self.frame_size,
                                ) {
                                    self.pending.push(Input::Move { x, y });
                                }
                            }
                            Moved::Placed => match self.on_the_picture(event) {
                                // Where the title draws its own pointer, the
                                // person's is not a place the guest's pointer
                                // should be moved to. The two are only ever in
                                // the same regime for as long as it takes the
                                // application to come back to the front, and
                                // taking the person's place while it is away
                                // teleports the title's pointer to wherever their
                                // frozen one happens to sit — which a title that
                                // steers by distance reads as one enormous
                                // movement (#40, #86). All this event carries
                                // then is that the person is over the window.
                                Some((x, y)) => {
                                    self.pointer_over_us = true;
                                    if !self.pointer_hidden {
                                        self.steer.placed((x, y));
                                        self.pending.push(Input::Move { x, y });
                                    }
                                }
                                // On the frame the pointer is the person's, to
                                // resize or move the window by, and the title's
                                // stays where it last was on the picture.
                                None => self.pointer_over_us = false,
                            },
                            // A move of nobody's window is the person's pointer
                            // outside ours, and its location is in screen
                            // coordinates rather than the window's, so taking one
                            // would throw the guest's pointer somewhere else. The
                            // pointer stays where it last was inside the window;
                            // what the machine learns is only that the person's
                            // own pointer has left, so that it can be given back.
                            Moved::Elsewhere => self.pointer_over_us = false,
                        }
                    }
                    // Key down, key up, and a modifier pressed or let go. A
                    // key held down repeating is still the one press.
                    10..=12 => {
                        let key: u64 = self.rt.send::<MsgU64>()(event, self.rt.sel("keyCode"));
                        let input = if kind == 12 {
                            let flags: u64 =
                                self.rt.send::<MsgU64>()(event, self.rt.sel("modifierFlags"));
                            direct_input_modifier(key, flags)
                        } else {
                            let repeat: u64 =
                                self.rt.send::<MsgU64>()(event, self.rt.sel("isARepeat"));
                            direct_input_key(key)
                                .filter(|_| repeat & 0xff == 0)
                                .map(|code| Input::Key {
                                    code,
                                    down: kind == 10,
                                })
                        };
                        if let Some(Input::Key { code, down }) = input {
                            self.held.retain(|k| *k != code);
                            if down {
                                self.held.push(code);
                            }
                            self.pending.push(Input::Key { code, down });
                        }
                    }
                    _ => {}
                }
                if !ours_alone {
                    self.rt.send::<MsgVoidId>()(self.app, self.rt.sel("sendEvent:"), event);
                }
            }

            // A key let go while another application has the keyboard is
            // never sent here, so what was held is let go when the window
            // loses it.
            let key_window = {
                let v: u64 = self.rt.send::<MsgU64>()(self.window, self.rt.sel("isKeyWindow"));
                v & 0xff != 0
            };
            if !key_window {
                for code in self.held.drain(..) {
                    self.pending.push(Input::Key { code, down: false });
                }
            }

            // A BOOL is one byte, and the rest of the register is not ours
            // to read.
            let visible = {
                let v: u64 = self.rt.send::<MsgU64>()(self.window, self.rt.sel("isVisible"));
                v & 0xff != 0
            };
            if !visible {
                self.closed = true;
            }
        }
        // Where the person's pointer is and which application is in front are
        // both things that change without the title being told, so this is
        // settled every time the window system is asked what happened.
        self.draw_pointer_where_it_belongs();
    }

    /// The host's own clock, in nanoseconds since the window opened. The guest
    /// cannot reach this: it paces presentation.
    fn wall_nanos(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

impl Drop for MacWindow {
    fn drop(&mut self) {
        // A run that ended with the pointer hidden would leave a person
        // without one.
        self.show_pointer(true);
        // Safety: closing a window we opened.
        unsafe {
            self.rt.send::<MsgVoid>()(self.window, self.rt.sel("close"));
        }
    }
}

impl Host for MacWindow {
    fn present(&mut self, frame: &Frame<'_>) {
        // The layer keeps the image it is given for as long as it likes, so
        // the pixels handed over are a copy this call gives away: the guest's
        // own memory is unmapped when the run ends, and a layer still holding
        // it would be reading freed pages.
        //
        // ponytail: one copy per presented frame. Measure before replacing it
        // with a pool of surfaces the layer can cycle through.
        // A frame shorter than it says it is would have Core Graphics read
        // past the copy, so it is not shown at all (#94).
        let len = (frame.pitch * frame.height) as usize;
        let Some(bytes) = frame.pixels.get(..len) else {
            return;
        };
        self.frame_size = (frame.width, frame.height);
        let copy: Box<Vec<u8>> = Box::new(bytes.to_vec());
        let pixels = copy.as_ptr();
        let owned = Box::into_raw(copy).cast::<c_void>();

        // Safety: the provider takes ownership of the copy and hands it back
        // to `release_pixels` when the last image over it is released.
        unsafe {
            let provider = (self.cg_provider)(owned, pixels, len, release_pixels as *mut c_void);
            // The guest's surfaces are 32 bits a pixel, blue first, with the
            // top byte ignored: what DirectDraw calls RGB32.
            let bitmap_info = PRESENT_BITMAP_INFO;
            let image = (self.cg_image_create)(
                frame.width as usize,
                frame.height as usize,
                8,
                32,
                frame.pitch as usize,
                self.colour_space,
                bitmap_info,
                provider,
                std::ptr::null_mut(),
                false,
                0,
            );
            if !image.is_null() {
                self.rt.send::<MsgVoidId>()(self.layer, self.rt.sel("setContents:"), image);
                (self.cf_release)(image);
            }
            (self.cg_provider_release)(provider);
        }
        self.pump();
    }

    fn poll_input(&mut self, _flip: u64) -> Vec<Input> {
        self.pump();
        self.steer.drained();
        std::mem::take(&mut self.pending)
    }

    fn audio(&mut self, samples: &[i16]) {
        // The mix is made on the declared clock and the device eats it in
        // real time, so a host running slower than real time has to stretch
        // it or the device runs dry between blocks (#75). At a pace of 1.0
        // this hands the device the block byte for byte.
        let stretched = self.stretch.block(samples, self.pace);
        if let Some(device) = &self.audio {
            device.play(&stretched);
        }
    }

    fn show_pointer(&mut self, show: bool) {
        self.pointer_hidden = !show;
        self.draw_pointer_where_it_belongs();
    }

    fn put_pointer(&mut self, x: i32, y: i32) {
        // The pointer the guest is handed starts again from where the title
        // put it, and the mouse's next move is measured from there. That is
        // all `SetCursorPos` needs on this machine: the person's own pointer
        // is never moved under their hand (#46), and while it is taken it does
        // not move at all. A move already waiting was measured from a place
        // the title has now moved on from, so it goes, and what it carried is
        // said again from here ([`Steer::put`], #86).
        self.pending
            .retain(|input| !matches!(input, Input::Move { .. }));
        if let Some((x, y)) = self.steer.put((x, y), self.frame_size) {
            self.pending.push(Input::Move { x, y });
        }
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn audio_underruns(&self) -> Option<u64> {
        self.audio
            .as_ref()
            .map(super::mac_audio::MacAudio::underruns)
    }

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        let (ptr, len) = sys::map_anon(len)?;
        // Safety: the mapping is live and this is its only owner.
        Ok(unsafe { Mapping::from_raw(ptr, len) })
    }

    fn pace(&mut self, declared_nanos: u64) -> f64 {
        // A game-second takes a wall-second (ADR-0003): if the machine is
        // ahead of its own clock, it waits.
        let wall = self.wall_nanos();
        self.lag_nanos = self.lag_nanos.max(wall.saturating_sub(declared_nanos));
        let due = declared_nanos + self.lag_nanos;
        if due > wall {
            std::thread::sleep(std::time::Duration::from_nanos(due - wall));
        }
        // How much declared time the last window of wall time was worth. Over
        // a window rather than a flip, because one flip's worth is noise, and
        // the mix is stretched by this (#75).
        let now = self.wall_nanos();
        let (was_wall, was_declared) = self.paced_from;
        let (gone, made) = (
            now.saturating_sub(was_wall),
            declared_nanos.saturating_sub(was_declared),
        );
        if gone >= PACE_NANOS {
            self.pace = if gone == 0 {
                1.0
            } else {
                (made as f64 / gone as f64).clamp(0.0, 1.0)
            };
            self.paced_from = (now, declared_nanos);
        }
        self.pace
    }
}

#[cfg(test)]
mod tests {
    use super::{
        direct_input_key, direct_input_modifier, guest_point, moved, Moved, Rect, Steer,
        PRESENT_BITMAP_INFO,
    };
    use crate::host::{sys, Input};

    /// A taken pointer is frozen, so it is nowhere and cannot have left (#40,
    /// #64).
    ///
    /// Reading a frozen pointer's event as "the person has gone" gives the
    /// pointer back, and the very next move is then taken as a place rather
    /// than a distance — which drags the guest's pointer to wherever the
    /// person's frozen one happens to sit, every frame, for a title that puts
    /// its own back every frame.
    #[test]
    fn a_frozen_pointer_is_steered_whatever_the_event_says_it_is_over() {
        assert_eq!(moved(true, true), Moved::Steered);
        assert_eq!(
            moved(true, false),
            Moved::Steered,
            "a frozen pointer is over nothing, and that is not it leaving"
        );
        // And with the pointer the person's own, where it is is what counts.
        assert_eq!(moved(false, true), Moved::Placed);
        assert_eq!(moved(false, false), Moved::Elsewhere);
    }
    use std::ffi::{c_char, c_void};

    /// A machine whose pointer goes exactly as far as the mouse did.
    const ONE: (f64, f64) = (1.0, 1.0);
    /// Alpha Team's declared pointer speed: 1280/640 across, 1280/480 down.
    const ALPHA_TEAM: (f64, f64) = (1280.0 / 640.0, 1280.0 / 480.0);
    const FRAME: (u32, u32) = (640, 480);
    const SAME: (f64, f64) = (640.0, 480.0);
    const MIDDLE: (i32, i32) = (320, 240);

    /// What Alpha Team does with a place it is handed: a 1280th of the screen
    /// for each pixel past the middle, drawn on a 640x480 screen.
    fn alpha_team_draws(place: (i32, i32)) -> (f64, f64) {
        (
            f64::from(place.0 - 320) / 1280.0 * 640.0,
            f64::from(place.1 - 240) / 1280.0 * 480.0,
        )
    }

    /// One flip of a title that puts the pointer back in the middle every
    /// frame, as the host runs it: the window is pumped when the frame is
    /// presented, then the title's put is passed on, then the window is
    /// pumped again and input is drained. What comes out is the place the
    /// title reads next, if any: the last place delivered.
    fn flip(
        steer: &mut Steer,
        before_put: &[(f64, f64)],
        after_put: &[(f64, f64)],
    ) -> Option<(i32, i32)> {
        let mut delivered = Vec::new();
        for m in before_put {
            delivered.extend(steer.moved(*m, SAME, FRAME));
        }
        // The put throws away a move already waiting (`put_pointer`).
        delivered.clear();
        delivered.extend(steer.put(MIDDLE, FRAME));
        for m in after_put {
            delivered.extend(steer.moved(*m, SAME, FRAME));
        }
        steer.drained();
        delivered.last().copied()
    }

    /// The declared pointer speed is what a title takes back out again.
    ///
    /// Alpha Team moves its own pointer a 1280th of the screen for every
    /// pixel it is handed, on both axes, which is half a pixel across and
    /// three eighths of one down; its machine declares 1280/640 and 1280/480
    /// so that the pointer a person watches goes exactly where their hand
    /// went (#86). The arithmetic the title does is written out here, because
    /// the whole point of the declared value is the number that comes out of
    /// the far end of it.
    #[test]
    fn the_declared_speed_puts_back_what_the_title_takes_off() {
        for hand in [(10.0, 10.0), (-7.0, 3.0), (100.0, -60.0), (1.0, 1.0)] {
            let place = Steer::new(MIDDLE, ALPHA_TEAM)
                .moved(hand, SAME, FRAME)
                .expect("a whole pixel of hand reaches another pixel");
            let moved = alpha_team_draws(place);
            assert!(
                (moved.0 - hand.0).abs() < 1.0 && (moved.1 - hand.1).abs() < 1.0,
                "hand {hand:?} drew {moved:?}"
            );
            // And at one for one — which is what it was — the title's pointer
            // covers half the ground across and three eighths of it down.
            let flat = Steer::new(MIDDLE, ONE)
                .moved(hand, SAME, FRAME)
                .map(alpha_team_draws)
                .unwrap_or_default();
            assert!((flat.0 - hand.0 / 2.0).abs() < 1.0);
            assert!((flat.1 - hand.1 * 0.375).abs() < 1.0);
        }
    }

    /// A move pumped before the title put the pointer back is not lost.
    ///
    /// Measured at the window with a synthetic hand (#86): ten points up,
    /// three times, forty milliseconds apart, at fifteen flips a second. The
    /// second reached the window while the frame was being presented, before
    /// the title's put, and the third after it; the title read only the last
    /// place delivered, and a third of the movement was gone. A person felt
    /// it as a pointer that fell short of their hand by a different amount
    /// every time.
    #[test]
    fn a_move_pumped_before_the_title_put_the_pointer_back_is_not_lost() {
        let mut steer = Steer::new(MIDDLE, ALPHA_TEAM);
        let up = (0.0, -10.0);
        let mut drawn = 0.0;
        if let Some(place) = flip(&mut steer, &[], &[up]) {
            drawn += alpha_team_draws(place).1;
        }
        if let Some(place) = flip(&mut steer, &[up], &[up]) {
            drawn += alpha_team_draws(place).1;
        }
        assert!(
            (drawn - -30.0).abs() < 1.0,
            "thirty points of hand drew {drawn} pixels"
        );
    }

    /// A hand that swings further than the screen is paid back over the
    /// frames after, not cut off.
    ///
    /// The place handed over stops at the screen's edge and the title puts
    /// the pointer back in the middle every frame, so only half a screen of
    /// movement can be said in one frame. A person playing this title spent a
    /// third of every fast movement past that edge and felt it as a mouse
    /// they had to sweep again and again (#86). What does not fit is said at
    /// the puts after, half a screen at a time, and only a swing wider than
    /// the hand is allowed to run past the edge loses anything.
    #[test]
    fn a_swing_wider_than_the_screen_is_paid_back_over_the_frames_after() {
        let mut steer = Steer::new(MIDDLE, (2.0, 1.0));
        // Once the title has put the pointer back, the host knows what kind
        // of title it is steering for.
        flip(&mut steer, &[], &[]);
        // 300 points of hand at twice speed wants 600 pixels, and there are
        // 319 to the right edge.
        assert_eq!(steer.moved((300.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        assert_eq!(
            steer.put(MIDDLE, FRAME),
            Some((601, 240)),
            "the rest is said from the middle at the next put"
        );
        steer.drained();
        assert_eq!(steer.put(MIDDLE, FRAME), None, "and it is paid in full");
        steer.drained();

        // Three screens of swing in one frame arrive whole, half a screen a
        // frame: 319 now, then 319 at each of five puts, then the last 6.
        let paid = |steer: &mut Steer, frames: usize| {
            (0..frames)
                .map(|_| {
                    let got = steer.put(MIDDLE, FRAME).map_or(0, |(x, _)| x - 320);
                    steer.drained();
                    got
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(steer.moved((960.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        assert_eq!(
            paid(&mut steer, 7),
            [319, 319, 319, 319, 319, 1920 - 6 * 319, 0]
        );

        // Ten screens in one frame is a wild swing: the hand may run four
        // screens past the edge and the rest is lost, as it is on a real
        // machine where the cursor is against the edge of the display.
        assert_eq!(steer.moved((3200.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        let got: i32 = paid(&mut steer, 10).iter().sum();
        assert_eq!(got + 319, 319 + 4 * 639, "the debt is capped");
    }

    /// A pointer the title leaves where it is stops at the screen's edge and
    /// comes straight back, as the cursor does at the display's edge (#40).
    #[test]
    fn a_pointer_the_title_leaves_stops_at_the_edge_and_comes_straight_back() {
        let mut steer = Steer::new(MIDDLE, ONE);
        assert_eq!(steer.moved((300.0, 0.0), SAME, FRAME), Some((620, 240)));
        assert_eq!(steer.moved((100.0, 0.0), SAME, FRAME), Some((639, 240)));
        steer.drained();
        assert_eq!(
            steer.moved((-50.0, 0.0), SAME, FRAME),
            Some((589, 240)),
            "a hand pushing past the edge is owed nothing when it comes back"
        );
    }

    /// A taken pointer moves by how far the mouse moved, in guest pixels
    /// whatever the window is stretched to, keeps the fraction of a pixel it
    /// has, and stops at the guest's screen edge as Windows stops the cursor
    /// at the display's.
    #[test]
    fn a_taken_pointer_is_steered_by_the_mouse_and_stays_on_the_screen() {
        let mut steer = Steer::new(MIDDLE, ONE);
        assert_eq!(steer.moved((6.0, -4.0), SAME, FRAME), Some((326, 236)));
        // Twice the window is twice the points for the same guest pixel.
        let twice = (1280.0, 960.0);
        assert_eq!(steer.moved((6.0, 0.0), twice, FRAME), Some((329, 236)));
        // Half a pixel is kept, not lost.
        assert_eq!(steer.moved((0.5, 0.0), SAME, FRAME), None);
        assert_eq!(steer.moved((0.5, 0.0), SAME, FRAME), Some((330, 236)));
        assert_eq!(steer.moved((500.0, 500.0), SAME, FRAME), Some((639, 479)));
        assert_eq!(steer.moved((-900.0, -900.0), SAME, FRAME), Some((0, 0)));
    }

    /// The fraction of a pixel survives the title putting the pointer back.
    ///
    /// Alpha Team puts the pointer back in the middle of the window every
    /// flip and steers by how far it went since. A put that dropped the
    /// fraction dropped up to a whole pixel of movement twenty-five times a
    /// second: a hand moving less than a pixel a flip moved the pointer not at
    /// all, and a person playing it said the pointer "never really lines up"
    /// (#64).
    #[test]
    fn the_fraction_of_a_pixel_survives_the_title_putting_the_pointer_back() {
        // A hand moving half a pixel a flip has moved five pixels in ten, and
        // the guest has to be told all five.
        let mut steer = Steer::new(MIDDLE, ONE);
        let mut delivered = 0;
        for _ in 0..10 {
            if let Some((x, _)) = flip(&mut steer, &[], &[(0.5, 0.0)]) {
                delivered += x - 320;
            }
        }
        assert_eq!(delivered, 5, "half a pixel a flip is five pixels in ten");
    }

    #[test]
    fn a_presented_pixel_reaches_core_graphics_as_the_colour_it_is() {
        // An orange as the machine presents it: blue, green, red, and a byte
        // Core Graphics is to skip.
        let pixel: [u8; 4] = [0x10, 0x80, 0xf0, 0x00];
        let graphics =
            sys::Library::open("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics")
                .expect("Core Graphics is on every Mac");
        // Safety: each type is the signature CoreGraphics' headers give it,
        // and each object is released once, after its last use.
        unsafe {
            let space_named: unsafe extern "C" fn(*const c_char) -> *mut c_void =
                graphics.function("CGColorSpaceCreateWithName").unwrap();
            let provider_with: unsafe extern "C" fn(
                *mut c_void,
                *const u8,
                usize,
                *mut c_void,
            ) -> *mut c_void = graphics.function("CGDataProviderCreateWithData").unwrap();
            let image_create: unsafe extern "C" fn(
                usize,
                usize,
                usize,
                usize,
                usize,
                *mut c_void,
                u32,
                *mut c_void,
                *mut c_void,
                bool,
                i32,
            ) -> *mut c_void = graphics.function("CGImageCreate").unwrap();
            let context_create: unsafe extern "C" fn(
                *mut u8,
                usize,
                usize,
                usize,
                usize,
                *mut c_void,
                u32,
            ) -> *mut c_void = graphics.function("CGBitmapContextCreate").unwrap();
            let draw: unsafe extern "C" fn(*mut c_void, Rect, *mut c_void) =
                graphics.function("CGContextDrawImage").unwrap();
            let release: unsafe extern "C" fn(*mut c_void) =
                graphics.function("CFRelease").unwrap();

            let srgb = graphics.symbol("kCGColorSpaceSRGB").unwrap();
            let space = space_named(*srgb.cast::<*const c_char>());
            let provider = provider_with(
                std::ptr::null_mut(),
                pixel.as_ptr(),
                4,
                std::ptr::null_mut(),
            );
            let image = image_create(
                1,
                1,
                8,
                32,
                4,
                space,
                PRESENT_BITMAP_INFO,
                provider,
                std::ptr::null_mut(),
                false,
                0,
            );
            // Drawn into red, green, blue and alpha, in that order.
            let mut out = [0u8; 4];
            const PREMULTIPLIED_LAST: u32 = 1;
            let context = context_create(out.as_mut_ptr(), 1, 1, 8, 4, space, PREMULTIPLIED_LAST);
            let whole = Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            };
            draw(context, whole, image);
            for object in [context, image, provider, space] {
                release(object);
            }
            assert_eq!(out, [0xf0, 0x80, 0x10, 0xff], "orange, and opaque");
        }
    }

    #[test]
    fn a_mac_key_reaches_the_guest_as_the_key_in_the_same_place() {
        assert_eq!(direct_input_key(0x35), Some(0x01)); // Escape
        assert_eq!(direct_input_key(0x00), Some(0x1e)); // A
        assert_eq!(direct_input_key(0x24), Some(0x1c)); // Return
        assert_eq!(direct_input_key(0x7e), Some(0xc8)); // Up
        assert_eq!(direct_input_key(0x3a), None); // Option, a modifier
        assert_eq!(direct_input_key(0x4c), Some(0x9c)); // Keypad Enter
        assert_eq!(direct_input_key(0x73), Some(0xc7)); // Home

        // Every key a person can press here, a route can write by name.
        for key in 0..=0x7f {
            let modifier = match direct_input_modifier(key, 0) {
                Some(Input::Key { code, .. }) => Some(code),
                _ => None,
            };
            for code in [direct_input_key(key), modifier].into_iter().flatten() {
                assert!(crate::route::key_name(code).is_some(), "{key:#x}");
            }
        }
    }

    #[test]
    fn a_modifier_is_down_while_its_own_side_is_held() {
        let key = |code, down| Some(Input::Key { code, down });
        // Left Shift pressed: the flags carry Shift and the left device bit.
        assert_eq!(direct_input_modifier(0x38, 0x0002_0002), key(0x2a, true));
        // Right Shift pressed with the left still held: the right one is down.
        assert_eq!(direct_input_modifier(0x3c, 0x0002_0006), key(0x36, true));
        // Left Shift let go while the right is still held: Shift is still in
        // the flags, but the left one is up.
        assert_eq!(direct_input_modifier(0x38, 0x0002_0004), key(0x2a, false));
        assert_eq!(direct_input_modifier(0x3e, 0x0004_2000), key(0x9d, true));
        assert_eq!(direct_input_modifier(0x39, 0x0001_0000), None); // Caps Lock
                                                                    // An event that does not say which side: the modifier's own bit.
        assert_eq!(direct_input_modifier(0x3a, 0x0008_0000), key(0x38, true));
        assert_eq!(direct_input_modifier(0x3a, 0), key(0x38, false));
    }

    #[test]
    fn the_vertical_axis_turns_over_between_the_window_and_the_guest() {
        // The window's own origin is its bottom left corner.
        let same = (640.0, 480.0);
        assert_eq!(guest_point(same, (640, 480), 0.0, 480.0), (0, 0));
        assert_eq!(guest_point(same, (640, 480), 320.0, 1.0), (320, 479));
        assert_eq!(guest_point(same, (640, 480), 100.0, 180.0), (100, 300));
        // A point outside the window is clamped rather than passed through
        // negative.
        assert_eq!(guest_point(same, (640, 480), -5.0, 600.0), (0, 0));
    }

    #[test]
    fn a_point_comes_back_through_the_scale_and_the_bars() {
        // Twice the size, with a hundred and sixty points of bar each side.
        let wide = (1600.0, 960.0);
        assert_eq!(guest_point(wide, (640, 480), 160.0, 960.0), (0, 0));
        assert_eq!(guest_point(wide, (640, 480), 800.0, 480.0), (320, 240));
        // In the bar is at the picture's edge.
        assert_eq!(guest_point(wide, (640, 480), 10.0, 480.0), (0, 240));
    }
}

/// The way back to a pointer, for a run that does not get to finish.
///
/// The pointer belongs to the person sitting there, not to the run. A title
/// that draws its own takes the host's away and lets go of the one the mouse
/// moves, and `Drop` puts both back — but a signal that ends the process runs
/// no destructor, and what is left behind is a machine with no pointer and
/// nothing on screen saying why (#45).
///
/// So the two calls that put it back are kept where a signal handler can reach
/// them, and the handler puts the pointer back before letting the signal do
/// what it was going to do anyway.
mod way_back {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    static ASSOCIATE: AtomicUsize = AtomicUsize::new(0);
    static SHOW: AtomicUsize = AtomicUsize::new(0);
    static DISPLAY: AtomicU32 = AtomicU32::new(0);
    static ARMED: AtomicUsize = AtomicUsize::new(0);

    /// `SIG_DFL`, and the signals that end a run someone is watching.
    const SIG_DFL: usize = 0;
    const SIGNALS: [i32; 3] = [1, 2, 15];

    extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
        fn raise(sig: i32) -> i32;
    }

    /// Put the pointer back, from wherever this is called.
    ///
    /// Safe to call when nothing was ever taken: both slots are empty and it
    /// does nothing.
    pub fn restore() {
        let (associate, show) = (
            ASSOCIATE.load(Ordering::Relaxed),
            SHOW.load(Ordering::Relaxed),
        );
        if associate == 0 || show == 0 {
            return;
        }
        // Safety: the two addresses were taken from Core Graphics' own symbols
        // with these signatures, and are only ever written once.
        unsafe {
            let associate: unsafe extern "C" fn(bool) -> i32 = std::mem::transmute(associate);
            let show: unsafe extern "C" fn(u32) -> i32 = std::mem::transmute(show);
            associate(true);
            show(DISPLAY.load(Ordering::Relaxed));
        }
    }

    extern "C" fn on_signal(sig: i32) {
        restore();
        // Then let the signal be what it was: the default for it, raised
        // again, so the run ends the way whoever sent it meant it to.
        // Safety: libc's own, and this process is on its way out.
        unsafe {
            signal(sig, SIG_DFL);
            raise(sig);
        }
    }

    /// Remember how to put the pointer back, and arm the signals. Once.
    pub fn arm(associate: usize, show: usize, display: u32) {
        ASSOCIATE.store(associate, Ordering::Relaxed);
        SHOW.store(show, Ordering::Relaxed);
        DISPLAY.store(display, Ordering::Relaxed);
        if ARMED.swap(1, Ordering::Relaxed) == 1 {
            return;
        }
        for sig in SIGNALS {
            // Safety: libc's own, with a handler of the shape it asks for.
            unsafe {
                signal(sig, on_signal as *const () as usize);
            }
        }
    }
}

/// Remember how to give the pointer back, and arm the signals that would
/// otherwise take it away for good.
fn arm_the_way_back(
    associate: unsafe extern "C" fn(bool) -> i32,
    show: unsafe extern "C" fn(u32) -> i32,
    display: u32,
) {
    way_back::arm(
        associate as *const () as usize,
        show as *const () as usize,
        display,
    );
}
