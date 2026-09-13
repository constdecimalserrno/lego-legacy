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

use super::{sys, Frame, Host, HostError, Input, Mapping};
use std::ffi::{c_char, c_void, CString};
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

/// Where the pointer the guest is handed goes after the mouse moved, while the
/// pointer is taken: the guest's own pointer, steered by how far the mouse
/// moved rather than by where the person's pointer is, which is frozen.
///
/// The move is in points of the window's content, so it is scaled to guest
/// pixels; the pointer keeps its fraction of a pixel between moves, and stays
/// on the guest's screen as Windows keeps the cursor on the display.
#[must_use]
pub fn steered(
    pointer: (f64, f64),
    moved: (f64, f64),
    content: (f64, f64),
    frame: (u32, u32),
) -> (f64, f64) {
    let scale = content_scale(content, frame);
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    (
        (pointer.0 + moved.0 / scale).clamp(0.0, fw - 1.0),
        (pointer.1 + moved.1 / scale).clamp(0.0, fh - 1.0),
    )
}

/// Where the pointer starts again from when the title puts it somewhere.
///
/// The whole pixel is the title's: it has just said that is where the pointer
/// is, and the guest's own pointer is there. The fraction is the person's —
/// how far into the next pixel their hand had got and has not been told about
/// yet — so it carries over (#64).
///
/// It matters here more than it looks. Alpha Team puts the pointer back in
/// the middle of the window **every flip** and steers by how far it went
/// since, so a reset that dropped the fraction dropped up to a whole pixel of
/// movement twenty-five times a second: a hand moving less than a pixel a
/// flip moved the pointer not at all, and a faster one lost a quarter of
/// itself, differently in each direction. A person playing it said the
/// pointer "never really lines up".
#[must_use]
pub fn put_back(steer: (f64, f64), to: (i32, i32)) -> (f64, f64) {
    let keep = |had: f64, whole: i32| f64::from(whole) + (had - had.floor());
    (keep(steer.0, to.0), keep(steer.1, to.1))
}

/// Where a point on the guest's screen is in the window's content: the
/// inverse of [`guest_point`], landing on the middle of the guest's pixel.
#[must_use]
pub fn content_point(content: (f64, f64), frame: (u32, u32), gx: i32, gy: i32) -> (f64, f64) {
    let (cw, ch) = content;
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    let scale = content_scale(content, frame);
    let (left, bottom) = ((cw - fw * scale) / 2.0, (ch - fh * scale) / 2.0);
    let x = left + (f64::from(gx) + 0.5) * scale;
    let y = ch - bottom - (f64::from(gy) + 0.5) * scale;
    (x, y)
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
    /// The pointer the guest is handed while the pointer is taken, in guest
    /// pixels with the fraction kept: where the title last put it, plus how
    /// far the mouse has moved since. With the person's own pointer frozen,
    /// this is the only pointer that moves (#64).
    steer: (f64, f64),
    /// The last place the guest was told the pointer was, so that a move too
    /// small to reach the next pixel is not a move.
    told: (i32, i32),
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

impl MacWindow {
    /// Open a window of this size, titled after the title being run.
    ///
    /// # Errors
    /// When the frameworks cannot be opened, or the window cannot be made.
    pub fn open(name: &str, width: u32, height: u32) -> Result<Self, HostError> {
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
                steer: (f64::from(width) / 2.0, f64::from(height) / 2.0),
                told: (i32::MAX, i32::MAX),
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
            // went (`steered`). That is what a title steering by distance
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

    /// Hand the guest the steered pointer, once it has reached another pixel.
    fn tell_where_the_pointer_is(&mut self) {
        // Truncation is the intent: a pixel is where the pointer is until it
        // has crossed into the next.
        let now = (self.steer.0 as i32, self.steer.1 as i32);
        if now != self.told {
            self.told = now;
            self.pending.push(Input::Move { x: now.0, y: now.1 });
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
                // Command is not the title's — it is how a person quits or
                // closes the window — so that one is still offered.
                let ours_alone = matches!(kind, 10 | 11) && {
                    const COMMAND: u64 = 1 << 20;
                    let flags: u64 = self.rt.send::<MsgU64>()(event, self.rt.sel("modifierFlags"));
                    flags & COMMAND == 0
                };
                match kind {
                    // A move of nobody's window carries screen coordinates
                    // rather than the window's, so taking one would throw the
                    // pointer somewhere else. Ignored, the pointer stays where
                    // it last was inside the window, which is what a title
                    // scrolling its camera from a pointer held at an edge
                    // needs. A button is not ignored: a press that is dropped
                    // is a button the guest never sees let go of.
                    // A move of nobody's window is the pointer outside ours,
                    // and its location is in screen coordinates rather than
                    // the window's, so taking one would throw the guest's
                    // pointer somewhere else. The pointer stays where it last
                    // was inside the window; what the machine learns from the
                    // move is only that the person's own pointer has left, so
                    // that it can give it back to them.
                    5 | 6 if !self.event_is_ours(event) => {
                        self.pointer_over_us = false;
                    }
                    // Mouse down, mouse up, mouse moved, dragged. A press on
                    // the window's own frame — its title bar, its edges — is
                    // the person taking hold of the window, not a click on the
                    // title (#49). A release is never dropped: a press the
                    // guest saw is a button it has to see let go of.
                    1 => {
                        if self.on_the_picture(event).is_some() {
                            self.pending.push(Input::Down);
                        }
                    }
                    2 => self.pending.push(Input::Up),
                    5 | 6 if self.pointer_taken => {
                        // The person's pointer is frozen over the picture, so
                        // where the event is says nothing; how far the mouse
                        // moved is the whole message.
                        let dx: f64 = self.rt.send::<MsgF64>()(event, self.rt.sel("deltaX"));
                        let dy: f64 = self.rt.send::<MsgF64>()(event, self.rt.sel("deltaY"));
                        let view: Id =
                            self.rt.send::<MsgId>()(self.window, self.rt.sel("contentView"));
                        let frame: Rect = self.rt.send::<MsgRect>()(view, self.rt.sel("frame"));
                        self.steer = steered(
                            self.steer,
                            (dx, dy),
                            (frame.width, frame.height),
                            self.frame_size,
                        );
                        self.tell_where_the_pointer_is();
                    }
                    5 | 6 => match self.on_the_picture(event) {
                        Some((x, y)) => {
                            self.pointer_over_us = true;
                            self.steer = (f64::from(x), f64::from(y));
                            self.told = (x, y);
                            self.pending.push(Input::Move { x, y });
                        }
                        // On the frame the pointer is the person's, to resize
                        // or move the window by, and the title's stays where it
                        // last was on the picture.
                        None => self.pointer_over_us = false,
                    },
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
        self.frame_size = (frame.width, frame.height);
        let len = (frame.pitch * frame.height) as usize;
        let copy: Box<Vec<u8>> = Box::new(frame.pixels[..len.min(frame.pixels.len())].to_vec());
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
        // not move at all.
        self.steer = put_back(self.steer, (x, y));
        self.told = (x, y);
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn wall_nanos(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        let (ptr, len) = sys::map_anon(len)?;
        // Safety: the mapping is live and this is its only owner.
        Ok(unsafe { Mapping::from_raw(ptr, len) })
    }

    fn load_library(&mut self, name: &str) -> Result<sys::Library, HostError> {
        sys::Library::open(name)
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
        content_point, direct_input_key, direct_input_modifier, guest_point, put_back, steered,
        Rect, PRESENT_BITMAP_INFO,
    };
    use crate::host::{sys, Input};
    use std::ffi::{c_char, c_void};

    /// Putting the pointer where the guest says and reading it back finds the
    /// same pixel, whatever the window has been stretched to (#59).
    #[test]
    fn a_pointer_put_on_a_guest_pixel_reads_back_as_that_pixel() {
        let frame = (640, 480);
        for content in [(640.0, 480.0), (1281.0, 963.0), (900.0, 480.0)] {
            for point in [(0, 0), (319, 239), (639, 479), (12, 400)] {
                let (x, y) = content_point(content, frame, point.0, point.1);
                assert_eq!(
                    guest_point(content, frame, x, y),
                    point,
                    "content {content:?}, point {point:?}"
                );
            }
        }
    }

    /// A taken pointer moves by how far the mouse moved, in guest pixels
    /// whatever the window is stretched to, keeps the fraction of a pixel it
    /// has, and stops at the guest's screen edge as Windows stops the cursor
    /// at the display's (#64).
    #[test]
    fn a_taken_pointer_is_steered_by_the_mouse_and_stays_on_the_screen() {
        let frame = (640, 480);
        let same = (640.0, 480.0);
        assert_eq!(
            steered((320.0, 240.0), (6.0, -4.0), same, frame),
            (326.0, 236.0)
        );
        // Twice the window is twice the points for the same guest pixel.
        let twice = (1280.0, 960.0);
        assert_eq!(
            steered((320.0, 240.0), (6.0, 0.0), twice, frame),
            (323.0, 240.0)
        );
        // Half a pixel is kept, not lost.
        let (x, _) = steered((320.0, 240.0), (0.5, 0.0), same, frame);
        let (x, _) = steered((x, 240.0), (0.5, 0.0), same, frame);
        assert_eq!(x, 321.0);
        assert_eq!(
            steered((630.0, 470.0), (500.0, 500.0), same, frame),
            (639.0, 479.0)
        );
        assert_eq!(
            steered((5.0, 5.0), (-500.0, -500.0), same, frame),
            (0.0, 0.0)
        );
    }

    /// The loop this title actually runs: put the pointer back in the middle,
    /// move the mouse, read where it got to, put it back again (#64).
    #[test]
    fn the_fraction_of_a_pixel_survives_the_title_putting_the_pointer_back() {
        let (frame, same) = ((640, 480), (640.0, 480.0));
        // A hand moving half a pixel a flip has moved five pixels in ten, and
        // the guest has to be told all five.
        let mut steer = (320.0, 240.0);
        let mut delivered = 0;
        for _ in 0..10 {
            steer = steered(steer, (0.5, 0.0), same, frame);
            // The guest is told the whole pixel the pointer reached, and the
            // title puts it back in the middle straight after, so the next
            // flip measures from the middle again.
            delivered += steer.0 as i32 - 320;
            steer = put_back(steer, (320, 240));
        }
        assert_eq!(delivered, 5, "half a pixel a flip is five pixels in ten");

        // Dropping the fraction instead — which is what this did — delivers
        // nothing at all, because the pointer never reaches the next pixel.
        let mut steer = (320.0, 240.0);
        let mut dropped = 0;
        for _ in 0..10 {
            steer = steered(steer, (0.5, 0.0), same, frame);
            dropped += steer.0 as i32 - 320;
            steer = (320.0, 240.0);
        }
        assert_eq!(dropped, 0, "and this is the pointer that never lined up");
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
