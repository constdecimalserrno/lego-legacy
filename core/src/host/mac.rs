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

use super::pointer::{back_on_the_picture, content_scale, moved, Capture, Mouse, Moved, Steer};
use super::{sys, Button, Cursor, Frame, Host, HostError, Input, Mapping};
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicU32, Ordering};
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
/// An `NSSize` is laid out as a `Point` is: two doubles.
type MsgIdPtrPoint = unsafe extern "C" fn(Id, Sel, *mut c_void, Point) -> Id;
type MsgIdIdPoint = unsafe extern "C" fn(Id, Sel, Id, Point) -> Id;

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
    objc: sys::Library,
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
            objc,
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

    /// An autorelease pool, open until [`Runtime::drain`] (#180). AppKit
    /// opens one for each turn of its own event loop, and this backend runs
    /// its own, so it opens its own: every object AppKit hands back
    /// autoreleased is let go when the pool drains, and without one never is
    /// (Apple, `NSAutoreleasePool`).
    fn pool(&self) -> Id {
        // Safety: NSAutoreleasePool's own `alloc` and `init`.
        unsafe {
            let pool: Id = self.send::<MsgId>()(self.class("NSAutoreleasePool"), self.sel("alloc"));
            self.send::<MsgId>()(pool, self.sel("init"))
        }
    }

    /// Drain a pool [`Runtime::pool`] opened, in the same function.
    fn drain(&self, pool: Id) {
        // Safety: `drain` on the pool, once.
        unsafe { self.send::<MsgVoid>()(pool, self.sel("drain")) }
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

/// How many times the application was asked to quit (#178): the Dock's Quit,
/// a logout or a restart. Its own count, not [`super::CLOSE_ASKED`], which
/// core's tests of headless runs read, so that the delegate can be tested.
static QUITS: AtomicU32 = AtomicU32::new(0);

/// `applicationShouldTerminate:`, which AppKit asks the application's
/// delegate before it quits (Apple, `terminate(_:)`): one close, counted as
/// the window's button is, and `NSTerminateCancel`, so the title is told and
/// the run ends when it leaves (#140). `NSTerminateLater` would have AppKit
/// run its own loop until answered, inside `pump`, with the title stopped.
extern "C" fn should_terminate(_delegate: Id, _cmd: Sel, _app: Id) -> u64 {
    QUITS.fetch_add(1, Ordering::Relaxed);
    0
}

/// `applicationSupportsSecureRestorableState:`: yes. An application delegate
/// without it has AppKit warn on the terminal that secure coding is not
/// enabled for restorable state (macOS 14 on, as SDL and GLFW found, read as
/// documentation). Nothing here is restored, so the answer changes nothing
/// else (Apple, `NSApplication.h`).
extern "C" fn secure_restorable(_delegate: Id, _cmd: Sel, _app: Id) -> bool {
    true
}

/// The application's delegate, which the caller owns: an object of a class
/// made here at run time, a subclass of `NSObject` with only
/// [`should_terminate`] and [`secure_restorable`] (#178). The class is made
/// once a process.
///
/// # Errors
/// When the runtime's functions cannot be found.
fn quit_delegate(rt: &Runtime) -> Result<Id, HostError> {
    // Safety: each function is called through the signature the runtime's
    // headers give it; each method's types say what its function answers,
    // an `NSUInteger` or a BOOL, and takes: self, the selector and an object.
    unsafe {
        let allocate: unsafe extern "C" fn(Id, *const c_char, usize) -> Id =
            rt.objc.function("objc_allocateClassPair")?;
        let add_method: unsafe extern "C" fn(Id, Sel, *const c_void, *const c_char) -> u8 =
            rt.objc.function("class_addMethod")?;
        let register: unsafe extern "C" fn(Id) = rt.objc.function("objc_registerClassPair")?;
        let mut class = allocate(rt.class("NSObject"), c"LegoLegacyQuit".as_ptr(), 0);
        if class.is_null() {
            // The name is taken: by the class an earlier call made.
            class = rt.class("LegoLegacyQuit");
        } else {
            add_method(
                class,
                rt.sel("applicationShouldTerminate:"),
                should_terminate as *const c_void,
                c"Q@:@".as_ptr(),
            );
            add_method(
                class,
                rt.sel("applicationSupportsSecureRestorableState:"),
                secure_restorable as *const c_void,
                c"B@:@".as_ptr(),
            );
            register(class);
        }
        let delegate: Id = rt.send::<MsgId>()(class, rt.sel("alloc"));
        Ok(rt.send::<MsgId>()(delegate, rt.sel("init")))
    }
}

/// How Core Graphics is told to read a presented frame: four bytes a pixel,
/// little-endian, so blue first, and the top byte skipped rather than taken
/// as alpha: `kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst`.
const PRESENT_BITMAP_INFO: u32 = (2 << 12) | 6;

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

/// A title's pointer shape as an `NSCursor`, which the caller owns: an
/// image of its pixels, by Core Graphics, with its hot spot, which AppKit
/// measures from the top left as Windows does (#139).
///
/// # Errors
/// When Core Graphics' functions cannot be found, or AppKit makes nothing.
fn ns_cursor(rt: &Runtime, shape: &Cursor) -> Result<Id, HostError> {
    let (width, height) = (shape.width as usize, shape.height as usize);
    let Some(pixels) = shape.pixels.get(..width * height * 4) else {
        return Err(HostError::Unsupported(
            "a pointer shape shorter than it says",
        ));
    };
    // Safety: each function is called through the signature Core Graphics'
    // and AppKit's headers give it; the image's copy of the pixels is handed
    // back to `release_pixels` once, and every object made is released once,
    // after the one that keeps it has taken it.
    unsafe {
        let space_named: unsafe extern "C" fn(*const c_char) -> *mut c_void =
            rt.graphics("CGColorSpaceCreateWithName")?;
        let space_release: unsafe extern "C" fn(*mut c_void) =
            rt.graphics("CGColorSpaceRelease")?;
        let provider_with: unsafe extern "C" fn(
            *mut c_void,
            *const u8,
            usize,
            *mut c_void,
        ) -> *mut c_void = rt.graphics("CGDataProviderCreateWithData")?;
        let provider_release: unsafe extern "C" fn(*mut c_void) =
            rt.graphics("CGDataProviderRelease")?;
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
        ) -> *mut c_void = rt.graphics("CGImageCreate")?;
        let image_release: unsafe extern "C" fn(*mut c_void) = rt.graphics("CGImageRelease")?;

        let copy: Box<Vec<u8>> = Box::new(pixels.to_vec());
        let bytes = copy.as_ptr();
        let provider = provider_with(
            Box::into_raw(copy).cast(),
            bytes,
            pixels.len(),
            release_pixels as *mut c_void,
        );
        let name = rt.core_graphics.symbol("kCGColorSpaceSRGB")?;
        let space = space_named(*name.cast::<*const c_char>());
        // Red, green, blue, then alpha, not multiplied in:
        // `kCGImageAlphaLast` in the bytes' own order.
        let image = image_create(
            width,
            height,
            8,
            32,
            width * 4,
            space,
            3,
            provider,
            std::ptr::null_mut(),
            false,
            0,
        );
        space_release(space);
        provider_release(provider);
        if image.is_null() {
            return Err(HostError::Unsupported(
                "Core Graphics made no pointer image",
            ));
        }
        let size = Point {
            x: width as f64,
            y: height as f64,
        };
        let ns_image: Id = rt.send::<MsgId>()(rt.class("NSImage"), rt.sel("alloc"));
        let ns_image: Id =
            rt.send::<MsgIdPtrPoint>()(ns_image, rt.sel("initWithCGImage:size:"), image, size);
        image_release(image);
        let hot = Point {
            x: f64::from(shape.hot.0),
            y: f64::from(shape.hot.1),
        };
        let cursor: Id = rt.send::<MsgId>()(rt.class("NSCursor"), rt.sel("alloc"));
        let cursor: Id =
            rt.send::<MsgIdIdPoint>()(cursor, rt.sel("initWithImage:hotSpot:"), ns_image, hot);
        rt.send::<MsgVoid>()(ns_image, rt.sel("release"));
        if cursor.is_null() {
            return Err(HostError::Unsupported("AppKit made no pointer"));
        }
        Ok(cursor)
    }
}

/// A window on the host's screen, showing what the guest drew.
pub struct MacWindow {
    rt: Runtime,
    app: Id,
    window: Id,
    layer: Id,
    /// Events a person made since the last flip.
    pending: Vec<Input>,
    /// The keys the guest has been told are down.
    held: Vec<u32>,
    /// Where the mix is heard, when `--audio` asked for it.
    audio: Option<super::mac_audio::MacAudio>,
    /// How many times the person asked the window to close (#140).
    closes: u32,
    /// Whether the window is gone from the screen, closed by its button,
    /// which is one close however long it stays gone.
    gone: bool,
    /// Whether the person's own pointer is over this window's picture, where
    /// it takes the title's shape (#139). Starts true, because a window that
    /// has just opened under the pointer has not been told otherwise.
    pointer_over_us: bool,
    /// Whether the host's pointer is hidden and frozen right now, which is
    /// not the same as the title drawing its own: it is only while the mouse
    /// is steered ([`Mouse::Steered`], #141). AppKit counts hides, so it is
    /// hidden once and shown once.
    pointer_taken: bool,
    /// Whose the mouse is ([`Capture`], #141).
    capture: Capture,
    /// The name the window's caption starts with.
    name: String,
    /// The shape the title set its pointer to, as an `NSCursor` this window
    /// owns, or nil for the person's own arrow (#139).
    shape: Id,
    /// Whether the pointer was given that shape at the last pump.
    shaped: bool,
    /// The pointer the guest is handed while the pointer is taken ([`Steer`]).
    steer: Steer,
    cg_associate: unsafe extern "C" fn(bool) -> i32,
    cg_show_cursor: unsafe extern "C" fn(u32) -> i32,
    cg_warp: unsafe extern "C" fn(Point) -> i32,
    cg_main_display: u32,
    /// Presentation held to real time, and the pace it came to (#75).
    pace: crate::host::pace::Pace,
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
            // The Dock's Quit, a logout and a restart ask the application's
            // delegate first (#178). It holds the delegate weakly, so this
            // one is kept, by the retain `alloc` gave, until the process ends.
            rt.send::<MsgVoidId>()(app, rt.sel("setDelegate:"), quit_delegate(&rt)?);

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
            // Its close button only takes it off the screen. A window made in
            // code is released when it closes, by default (Apple,
            // `isReleasedWhenClosed`), and this one is still asked about and
            // drawn into while the title leaves (#140, #175): it is kept, by
            // the retain `alloc` gave, until the process ends.
            rt.send::<MsgVoidBool>()(window, rt.sel("setReleasedWhenClosed:"), false);

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
            let srgb = rt.core_graphics.symbol("kCGColorSpaceSRGB")?;
            let colour_space = create_space(*srgb.cast::<*const c_char>());

            let cg_associate = rt.graphics("CGAssociateMouseAndMouseCursorPosition")?;
            let cg_show_cursor: unsafe extern "C" fn(u32) -> i32 =
                rt.graphics("CGDisplayShowCursor")?;
            let main_display: unsafe extern "C" fn() -> u32 = rt.graphics("CGMainDisplayID")?;
            let cg_main_display = main_display();
            Ok(Self {
                cg_associate,
                cg_show_cursor,
                cg_warp: rt.graphics("CGWarpMouseCursorPosition")?,
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
                pending: Vec::new(),
                held: Vec::new(),
                audio: None,
                closes: 0,
                gone: false,
                pointer_over_us: true,
                pointer_taken: false,
                capture: Capture::default(),
                name: name.to_owned(),
                shape: std::ptr::null_mut(),
                shaped: false,
                steer: Steer::new(
                    (
                        i32::try_from(width / 2).unwrap_or(320),
                        i32::try_from(height / 2).unwrap_or(240),
                    ),
                    pointer_speed,
                ),
                pace: crate::host::pace::Pace::new(),
                stretch: crate::host::stretch::Stretch::default(),
                frame_size: (width, height),
            })
        }
    }

    /// How many times the person has asked the window to close, the
    /// application to quit ([`QUITS`], #178), or the run to end from its
    /// terminal ([`super::listen`], #110, #140).
    #[must_use]
    pub fn closes(&self) -> u32 {
        self.closes + super::CLOSE_ASKED.load(Ordering::Relaxed) + QUITS.load(Ordering::Relaxed)
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
    /// only while the mouse is its own, which is from a click on the picture
    /// until Escape or this application leaving the front ([`Capture`],
    /// #141). Anywhere else the pointer is the person's and they are using it
    /// — to reach another window, to stop the run, to do something else
    /// entirely — and taking it from them there is taking their machine.
    ///
    /// Hiding is a count on AppKit's side, so this keeps its own flag and only
    /// ever moves it by one.
    fn draw_pointer_where_it_belongs(&mut self) {
        self.caption_says();
        let take = self.capture.mouse() == Mouse::Steered;
        // The title's shape, over the picture while the mouse is kept there,
        // which is only while this application is in front, and the arrow
        // everywhere else (#139, #141). AppKit puts its own arrow back as the
        // pointer crosses its windows, so the shape is set on every pump.
        let shaped =
            self.capture.mouse() == Mouse::Kept && self.pointer_over_us && !self.shape.is_null();
        if shaped || self.shaped {
            // Safety: NSCursor's own class method, and `set` on a cursor.
            unsafe {
                let cursor = if shaped {
                    self.shape
                } else {
                    let class = self.rt.class("NSCursor");
                    self.rt.send::<MsgId>()(class, self.rt.sel("arrowCursor"))
                };
                self.rt.send::<MsgVoid>()(cursor, self.rt.sel("set"));
            }
        }
        self.shaped = shaped;
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
            // is only ever on while the mouse is captured, which is only while
            // this application is in front — settled on every pump, and
            // undone by `arm_the_way_back` if the run is killed.
            (self.cg_associate)(!take);
        }
    }

    /// Say in the window's caption whose the mouse is, once that has changed
    /// (#141).
    fn caption_says(&mut self) {
        let Some(caption) = self.capture.caption(&self.name) else {
            return;
        };
        let caption = CString::new(caption).unwrap_or_default();
        // A caption can change outside a pump, when the title shows its
        // pointer or the run ends, so it has a pool of its own (#180).
        let pool = self.rt.pool();
        // Safety: an NSString made from a NUL-terminated string, and
        // `setTitle:` on our own window, which copies it.
        unsafe {
            let caption: Id = self.rt.send::<MsgIdCharPtr>()(
                self.rt.class("NSString"),
                self.rt.sel("stringWithUTF8String:"),
                caption.as_ptr(),
            );
            self.rt.send::<MsgVoidId>()(self.window, self.rt.sel("setTitle:"), caption);
        }
        self.rt.drain(pool);
    }

    /// The title's pointer is the person's own and it is here on the guest's
    /// screen: the title is told, and the hand starts again from here.
    fn placed(&mut self, at: (i32, i32)) {
        self.steer.placed(at);
        self.pending.push(Input::Move { x: at.0, y: at.1 });
    }

    /// Keep the person's pointer on the picture while the mouse is kept
    /// there ([`Mouse::Kept`], #141), and tell the title where it is when it
    /// has been put back, or when `tell`, the mouse just kept.
    ///
    /// macOS has no `ClipCursor`, so a pointer that has left is put back at
    /// the edge it left by, once a pump, after the window's place in front
    /// has been settled: a pointer that left for a window now in front is
    /// the person's, not put back.
    /// ponytail: put back once a pump, so it can stray past the edge for a
    /// frame, or for as long as the title takes to flip; the upgrade is the
    /// mouse kept apart from the pointer, and the pointer warped to each
    /// place, held on the picture.
    fn keep_on_the_picture(&mut self, tell: bool) {
        // Safety: AppKit's own accessors, which answer a point, rectangles
        // and objects by value; then Core Graphics' warp, which takes a point
        // in its own coordinates, and its switch, which takes a flag.
        unsafe {
            let at: Point =
                self.rt.send::<MsgPoint>()(self.rt.class("NSEvent"), self.rt.sel("mouseLocation"));
            let window: Rect = self.rt.send::<MsgRect>()(self.window, self.rt.sel("frame"));
            let view: Id = self.rt.send::<MsgId>()(self.window, self.rt.sel("contentView"));
            let content: Rect = self.rt.send::<MsgRect>()(view, self.rt.sel("frame"));
            // The content view's frame is the window's own coordinates, which
            // start at the window's bottom left corner on the screen.
            let (left, bottom) = (window.x + content.x, window.y + content.y);
            self.pointer_over_us = true;
            let back = back_on_the_picture(
                (at.x, at.y),
                (left, bottom),
                (content.width, content.height),
            );
            let (x, y) = match back {
                Some(back) => back,
                None if tell => (at.x, at.y),
                None => return,
            };
            if back.is_some() {
                // Core Graphics counts down from the top of the screen with
                // the menu bar, which AppKit lists first.
                let screens: Id =
                    self.rt.send::<MsgId>()(self.rt.class("NSScreen"), self.rt.sel("screens"));
                let first: Id = self.rt.send::<MsgId>()(screens, self.rt.sel("firstObject"));
                let first: Rect = self.rt.send::<MsgRect>()(first, self.rt.sel("frame"));
                (self.cg_warp)(Point {
                    x,
                    y: first.height - y,
                });
                // A warp holds the mouse's own events back for a quarter of a
                // second, unless the mouse is joined to the pointer again
                // straight after (as SDL does, read as documentation); it
                // already is here.
                (self.cg_associate)(true);
            }
            self.placed(guest_point(
                (content.width, content.height),
                self.frame_size,
                x - left,
                y - bottom,
            ));
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
        // What AppKit hands back autoreleased here, each event, the run loop
        // mode, the date, is let go as the pump ends (#180). Nothing kept
        // across pumps is only autoreleased: the window, its view and layer
        // and the application are owned (#175).
        let pool = self.rt.pool();
        let front;
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
                // so they close it here, as its button does, once a press: a
                // key held down repeating is still the one close (#140). They
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
                        let repeat: u64 = self.rt.send::<MsgU64>()(event, self.rt.sel("isARepeat"));
                        if repeat & 0xff == 0 {
                            self.closes += 1;
                        }
                        continue;
                    }
                }
                match kind {
                    // Either button down, either button up. A press on the
                    // window's own frame — its title bar, its edges — is the
                    // person taking hold of the window, not a click on the
                    // title (#49). A press on the picture with the mouse the
                    // person's captures it, and is the host's; a title that
                    // shows the host's pointer is told where it is at the end
                    // of the pump, which no move has told it since (#141). A
                    // press the guest saw is a button it has to see let go
                    // of, and only that one.
                    1 | 3 => {
                        if self.on_the_picture(event).is_some() {
                            self.pointer_over_us = true;
                            match self.capture.press(button(kind), Instant::now()) {
                                Some(down) => self.pending.push(down),
                                None => self.draw_pointer_where_it_belongs(),
                            }
                        }
                    }
                    2 | 4 => self.pending.extend(self.capture.release(button(kind))),
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
                                // movement (#40, #86). Nor is it a place while
                                // the mouse is the person's (#141). All this
                                // event carries then is that the person is over
                                // the window.
                                Some(at) => {
                                    self.pointer_over_us = true;
                                    if self.capture.mouse() == Mouse::Kept {
                                        self.placed(at);
                                    }
                                }
                                // On the frame the pointer is the person's, to
                                // resize or move the window by, and the title's
                                // stays where it last was on the picture; a
                                // kept one is put back at the end of the pump.
                                None => self.pointer_over_us = false,
                            },
                            // A move of nobody's window is the person's pointer
                            // outside ours, and its location is in screen
                            // coordinates rather than the window's, so taking one
                            // would throw the guest's pointer somewhere else. The
                            // pointer stays where it last was inside the window;
                            // what the machine learns is only that the person's
                            // own pointer has left, so that it is drawn as theirs.
                            Moved::Elsewhere => self.pointer_over_us = false,
                        }
                    }
                    // Key down, key up, and a modifier pressed or let go. A
                    // key held down repeating is still the one press, and
                    // the Q or W of Command-Q or Command-W goes no further
                    // let go than pressed (`host::key_change`). Escape with
                    // the mouse captured frees it, and is the host's (#141).
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
                            if let Some(let_go) = self.capture.key(code, down) {
                                self.pending.extend(let_go);
                                self.draw_pointer_where_it_belongs();
                            } else {
                                self.pending
                                    .extend(super::key_change(&mut self.held, code, down));
                            }
                        }
                    }
                    _ => {}
                }
                if !ours_alone {
                    self.rt.send::<MsgVoidId>()(self.app, self.rt.sel("sendEvent:"), event);
                }
            }

            // What AppKit says of the window and the application, asked only
            // of a getter that takes nothing and answers a BOOL, which is the
            // signature `MsgU64` is sent with: a BOOL is one byte, and the
            // rest of the register is not ours to read.
            let ask = |receiver: Id, question: &str| {
                let v: u64 = self.rt.send::<MsgU64>()(receiver, self.rt.sel(question));
                v & 0xff != 0
            };

            // A key let go while another application has the keyboard is
            // never sent here, so what was held is let go when the window
            // loses it.
            if !ask(self.window, "isKeyWindow") {
                for code in self.held.drain(..) {
                    self.pending.push(Input::Key { code, down: false });
                }
            }

            let visible = ask(self.window, "isVisible");
            let miniaturized = ask(self.window, "isMiniaturized");
            if super::closed(visible, miniaturized, ask(self.app, "isHidden")) && !self.gone {
                self.gone = true;
                self.closes += 1;
            }
            // Where the person's pointer is and which application is in front
            // are both things that change without the title being told, so
            // this is settled every time the window system is asked what
            // happened, the front first, so that a pointer kept on the picture
            // is put back only while this application is still in front, with
            // its window on the screen: one closed by its button, minimized,
            // or of an application hidden, which is then not the active one,
            // is not in front, and nor is one on another Space (#176).
            front = super::in_front(
                visible,
                miniaturized,
                ask(self.app, "isActive"),
                ask(self.window, "isOnActiveSpace"),
            );
        }
        self.pending
            .extend(self.capture.front(front, Instant::now()));
        let tell = self.capture.newly_kept();
        if self.capture.mouse() == Mouse::Kept {
            self.keep_on_the_picture(tell);
        }
        self.draw_pointer_where_it_belongs();
        self.rt.drain(pool);
    }
}

impl Drop for MacWindow {
    fn drop(&mut self) {
        // A run that ended with the mouse captured would leave a person
        // without a pointer, or with the title's shape.
        self.free_mouse();
        self.pointer_shape(None);
        // Safety: closing a window we opened, which is alive even when its
        // button already closed it (#175).
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
        let stretched = self.stretch.block(samples, self.pace.current());
        if let Some(device) = &self.audio {
            device.play(&stretched);
        }
    }

    fn show_pointer(&mut self, show: bool) {
        self.capture.pointer_shown(show);
        self.draw_pointer_where_it_belongs();
    }

    fn free_mouse(&mut self) {
        self.capture.end();
        self.draw_pointer_where_it_belongs();
    }

    fn pointer_shape(&mut self, shape: Option<&Cursor>) {
        // Safety: the cursor is one this window made, released once.
        unsafe {
            if !self.shape.is_null() {
                self.rt.send::<MsgVoid>()(self.shape, self.rt.sel("release"));
            }
        }
        // A shape AppKit will not make is the person's arrow.
        self.shape = shape
            .and_then(|shape| ns_cursor(&self.rt, shape).ok())
            .unwrap_or(std::ptr::null_mut());
        self.draw_pointer_where_it_belongs();
    }

    fn put_pointer(&mut self, x: i32, y: i32) {
        // The pointer the guest is handed starts again from where the title
        // put it, and the mouse's next move is measured from there. That is
        // all `SetCursorPos` needs on this machine: the person's own pointer
        // is never moved under their hand (#46), and while it is taken it does
        // not move at all. A move already waiting was measured from a place
        // the title has now moved on from, so it goes, and what it carried is
        // said again from here ([`Steer::put`], #86), while the mouse is the
        // title's (#141).
        self.pending
            .retain(|input| !matches!(input, Input::Move { .. }));
        let told = self.steer.put((x, y), self.frame_size);
        if let Some((x, y)) = told.filter(|_| self.capture.captured()) {
            self.pending.push(Input::Move { x, y });
        }
    }

    fn closes(&self) -> u32 {
        MacWindow::closes(self)
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
        self.pace.hold(declared_nanos)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        direct_input_key, direct_input_modifier, guest_point, ns_cursor, quit_delegate, Id, MsgId,
        MsgPoint, MsgU64, MsgVoid, Point, Rect, Runtime, Sel, PRESENT_BITMAP_INFO, QUITS,
    };
    use crate::host::{sys, Input};
    use std::sync::atomic::Ordering;

    use std::ffi::{c_char, c_void};

    /// `applicationShouldTerminate:`'s call shape: an `NSUInteger` back.
    type MsgU64Id = unsafe extern "C" fn(Id, Sel, Id) -> u64;

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
        // Caps Lock locks rather than holds, so it is not here.
        assert_eq!(direct_input_modifier(0x39, 0x0001_0000), None);
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

    /// A title's pointer shape becomes an AppKit cursor of its size, with
    /// its hot spot (#139). Nothing is shown and no window is opened: the
    /// cursor is made, read back and let go.
    #[test]
    fn a_pointer_shape_is_an_appkit_cursor_with_its_hot_spot() {
        let rt = Runtime::open().expect("AppKit is on every Mac");
        let shape = crate::host::Cursor {
            width: 3,
            height: 2,
            hot: (2, 1),
            pixels: vec![0xff; 3 * 2 * 4],
        };
        let cursor = ns_cursor(&rt, &shape).expect("AppKit makes it");
        // Safety: AppKit's own accessors on the cursor just made, which is
        // released once.
        unsafe {
            let hot: Point = rt.send::<MsgPoint>()(cursor, rt.sel("hotSpot"));
            assert_eq!((hot.x, hot.y), (2.0, 1.0));
            let image: Id = rt.send::<MsgId>()(cursor, rt.sel("image"));
            let size: Point = rt.send::<MsgPoint>()(image, rt.sel("size"));
            assert_eq!((size.x, size.y), (3.0, 2.0));
            rt.send::<MsgVoid>()(cursor, rt.sel("release"));
        }
        let short = crate::host::Cursor {
            pixels: vec![0; 4],
            ..shape
        };
        assert!(
            ns_cursor(&rt, &short).is_err(),
            "a shape short of its pixels"
        );
    }

    /// The Dock's Quit, a logout or a restart asks the application's
    /// delegate whether to terminate: it counts one close and answers
    /// `NSTerminateCancel`, so the title is told and the run ends when it
    /// leaves (#178). No window and no application: only the delegate.
    #[test]
    fn a_quit_asked_of_the_application_is_a_close_and_cancelled() {
        let rt = Runtime::open().expect("AppKit is on every Mac");
        let delegate = quit_delegate(&rt).expect("the runtime builds it");
        let before = QUITS.load(Ordering::Relaxed);
        // Safety: the method the delegate's class was given, with the
        // signature AppKit's header declares; the delegate is released once.
        let reply = unsafe {
            let reply =
                rt.send::<MsgU64Id>()(delegate, rt.sel("applicationShouldTerminate:"), rt.nil());
            let secure: u64 = rt.send::<MsgU64Id>()(
                delegate,
                rt.sel("applicationSupportsSecureRestorableState:"),
                rt.nil(),
            );
            assert_eq!(secure & 0xff, 1, "secure restorable state, a BOOL");
            rt.send::<MsgVoid>()(delegate, rt.sel("release"));
            reply
        };
        assert_eq!(reply, 0, "NSTerminateCancel");
        assert_eq!(QUITS.load(Ordering::Relaxed), before + 1, "one close");
        assert!(!quit_delegate(&rt).expect("and again").is_null());
    }

    /// What is autoreleased while a pool is open is let go when it is
    /// drained (#180): an object held twice and autoreleased once is held
    /// once after. No window: an `NSObject` of our own.
    #[test]
    fn an_autoreleased_object_is_let_go_when_the_pool_drains() {
        let rt = Runtime::open().expect("AppKit is on every Mac");
        // Safety: NSObject's own methods; the object is let go of as many
        // times as it is held.
        unsafe {
            let object: Id = rt.send::<MsgId>()(rt.class("NSObject"), rt.sel("alloc"));
            let object: Id = rt.send::<MsgId>()(object, rt.sel("init"));
            rt.send::<MsgId>()(object, rt.sel("retain"));
            let pool = rt.pool();
            rt.send::<MsgId>()(object, rt.sel("autorelease"));
            rt.drain(pool);
            let held: u64 = rt.send::<MsgU64>()(object, rt.sel("retainCount"));
            assert_eq!(held, 1, "the autorelease was let go");
            rt.send::<MsgVoid>()(object, rt.sel("release"));
        }
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
/// A first signal ends the run, and `Drop` with it (#110). So the two calls
/// that put it back are kept where the handler of a second can reach them
/// ([`super::sys`]), and it puts the pointer back before letting the signal
/// do what it was going to do anyway.
pub(super) mod way_back {
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

    static ASSOCIATE: AtomicUsize = AtomicUsize::new(0);
    static SHOW: AtomicUsize = AtomicUsize::new(0);
    static DISPLAY: AtomicU32 = AtomicU32::new(0);

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

    /// Remember how to put the pointer back.
    pub fn arm(associate: usize, show: usize, display: u32) {
        ASSOCIATE.store(associate, Ordering::Relaxed);
        SHOW.store(show, Ordering::Relaxed);
        DISPLAY.store(display, Ordering::Relaxed);
    }
}

/// Remember how to give the pointer back, for the signal that would
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
