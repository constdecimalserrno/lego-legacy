//! The Linux backend (ADR-0001, #108).
//!
//! X11 for the window, presenting, input and the pointer, reached through
//! `dlopen("libX11.so.6")`, and ALSA for the sound ([`super::linux_audio`]).
//! No crate is linked, and nothing outside the operating system's own
//! libraries is loaded. X11 serves a Wayland desktop too, through XWayland.
//!
//! What the backend decides without asking the X server is in [`super::x11`],
//! where the tests reach it on every host. This file has been compiled for
//! x86-64 and aarch64 Linux, and not yet run on either: #108 says what is
//! left.
//!
//! Every structure here is Xlib's own, declared by hand from `Xlib.h` for
//! LP64, which is what x86-64 and aarch64 Linux both are: `int` four bytes,
//! and `long`, a pointer and an XID eight. The size of each, and the offset
//! of every field read out of one, is checked when this file is compiled.
//!
//! The rules for the pointer are the Mac backend's, and its comments say what
//! each one cost to learn: the title's pointer is only taken from the person
//! while the title draws its own, the person's pointer is over the picture
//! and this window has the keyboard; while it is taken it is steered by how
//! far the mouse went ([`Steer`]); and it is given back the moment any of the
//! three stops being so.

use super::linux_audio::LinuxAudio;
use super::pace::Pace;
use super::pointer::{moved, Moved, Steer};
use super::stretch::Stretch;
use super::x11::{self, Held};
use super::{sys, Button, Frame, Host, HostError, Input, Mapping};
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_ushort, c_void, CString};
use std::mem::{offset_of, size_of};

/// An X connection, which only Xlib looks inside.
type Display = c_void;
/// A window, atom, cursor, pixmap, keysym or time: an XID, or a number the
/// size of one.
type Xid = c_ulong;
/// A graphics context, which only Xlib looks inside.
type Gc = *mut c_void;

/// The head every event has. Here for its layout, which every event shares.
#[repr(C)]
#[derive(Clone, Copy)]
struct XAnyEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: *mut Display,
    window: Xid,
}

/// `XKeyEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
struct XKeyEvent {
    head: XAnyEvent,
    root: Xid,
    subwindow: Xid,
    time: Xid,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: c_int,
}

/// `XButtonEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
struct XButtonEvent {
    head: XAnyEvent,
    root: Xid,
    subwindow: Xid,
    time: Xid,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    button: c_uint,
    same_screen: c_int,
}

/// `XMotionEvent`.
#[repr(C)]
#[derive(Clone, Copy)]
struct XMotionEvent {
    head: XAnyEvent,
    root: Xid,
    subwindow: Xid,
    time: Xid,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    is_hint: c_char,
    same_screen: c_int,
}

/// `XCrossingEvent`: the pointer came into the window or left it.
#[repr(C)]
#[derive(Clone, Copy)]
struct XCrossingEvent {
    head: XAnyEvent,
    root: Xid,
    subwindow: Xid,
    time: Xid,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    mode: c_int,
    detail: c_int,
    same_screen: c_int,
    focus: c_int,
    state: c_uint,
}

/// `XFocusChangeEvent`: the window took the keyboard or lost it.
#[repr(C)]
#[derive(Clone, Copy)]
struct XFocusChangeEvent {
    head: XAnyEvent,
    mode: c_int,
    detail: c_int,
}

/// `XExposeEvent`: part of the window has to be drawn again.
#[repr(C)]
#[derive(Clone, Copy)]
struct XExposeEvent {
    head: XAnyEvent,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    count: c_int,
}

/// `XConfigureEvent`: the window moved or changed size.
#[repr(C)]
#[derive(Clone, Copy)]
struct XConfigureEvent {
    head: XAnyEvent,
    window: Xid,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: Xid,
    override_redirect: c_int,
}

/// `XClientMessageEvent`, which is how the window manager says the window's
/// close button was pressed.
#[repr(C)]
#[derive(Clone, Copy)]
struct XClientMessageEvent {
    head: XAnyEvent,
    message_type: Xid,
    format: c_int,
    data: [c_long; 5],
}

/// `XEvent`: every event, in a union Xlib pads to twenty-four longs.
#[repr(C)]
union XEvent {
    kind: c_int,
    key: XKeyEvent,
    button: XButtonEvent,
    motion: XMotionEvent,
    crossing: XCrossingEvent,
    focus: XFocusChangeEvent,
    expose: XExposeEvent,
    configure: XConfigureEvent,
    client: XClientMessageEvent,
    pad: [c_long; 24],
}

/// `XImage`, filled in here and readied by `XInitImage`, so that the pixels
/// it points at stay this backend's own: `XDestroyImage` would free them with
/// C's allocator.
#[repr(C)]
struct XImage {
    width: c_int,
    height: c_int,
    xoffset: c_int,
    format: c_int,
    data: *mut c_char,
    byte_order: c_int,
    bitmap_unit: c_int,
    bitmap_bit_order: c_int,
    bitmap_pad: c_int,
    depth: c_int,
    bytes_per_line: c_int,
    bits_per_pixel: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    obdata: *mut c_char,
    /// The six routines `XInitImage` fills in.
    funcs: [*mut c_void; 6],
}

/// `Visual`, read for its colour masks.
#[repr(C)]
struct Visual {
    ext_data: *mut c_void,
    visualid: Xid,
    class: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    bits_per_rgb: c_int,
    map_entries: c_int,
}

/// `XColor`, which a cursor is made with.
#[repr(C)]
#[derive(Default)]
struct XColor {
    pixel: c_ulong,
    red: c_ushort,
    green: c_ushort,
    blue: c_ushort,
    flags: c_char,
    pad: c_char,
}

// Xlib.h's layout on LP64, which a structure declared wrong here would read
// as some other field: refused at compile time instead.
const _: () = {
    assert!(size_of::<XAnyEvent>() == 40);
    assert!(size_of::<XEvent>() == 192);
    assert!(size_of::<XKeyEvent>() == 96);
    assert!(offset_of!(XKeyEvent, state) == 80);
    assert!(offset_of!(XKeyEvent, keycode) == 84);
    assert!(size_of::<XButtonEvent>() == 96);
    assert!(offset_of!(XButtonEvent, button) == 84);
    assert!(size_of::<XMotionEvent>() == 96);
    assert!(offset_of!(XMotionEvent, x) == 64);
    assert!(offset_of!(XMotionEvent, same_screen) == 88);
    assert!(size_of::<XCrossingEvent>() == 104);
    assert!(offset_of!(XCrossingEvent, mode) == 80);
    assert!(offset_of!(XCrossingEvent, state) == 96);
    assert!(size_of::<XFocusChangeEvent>() == 48);
    assert!(offset_of!(XFocusChangeEvent, detail) == 44);
    assert!(size_of::<XExposeEvent>() == 64);
    assert!(offset_of!(XExposeEvent, count) == 56);
    assert!(size_of::<XConfigureEvent>() == 88);
    assert!(offset_of!(XConfigureEvent, width) == 56);
    assert!(offset_of!(XConfigureEvent, override_redirect) == 80);
    assert!(size_of::<XClientMessageEvent>() == 96);
    assert!(offset_of!(XClientMessageEvent, data) == 56);
    assert!(size_of::<XImage>() == 136);
    assert!(offset_of!(XImage, red_mask) == 56);
    assert!(offset_of!(XImage, funcs) == 88);
    assert!(size_of::<Visual>() == 56);
    assert!(offset_of!(Visual, red_mask) == 24);
    assert!(size_of::<XColor>() == 16);
};

// Event types.
const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const BUTTON_PRESS: c_int = 4;
const BUTTON_RELEASE: c_int = 5;
const MOTION_NOTIFY: c_int = 6;
const ENTER_NOTIFY: c_int = 7;
const LEAVE_NOTIFY: c_int = 8;
const FOCUS_IN: c_int = 9;
const FOCUS_OUT: c_int = 10;
const EXPOSE: c_int = 12;
const DESTROY_NOTIFY: c_int = 17;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

// Event masks.
const KEY_PRESS_MASK: c_long = 1 << 0;
const KEY_RELEASE_MASK: c_long = 1 << 1;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const ENTER_WINDOW_MASK: c_long = 1 << 4;
const LEAVE_WINDOW_MASK: c_long = 1 << 5;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const EXPOSURE_MASK: c_long = 1 << 15;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;
const FOCUS_CHANGE_MASK: c_long = 1 << 21;

/// Everything the window is told about.
const EVENTS: c_long = KEY_PRESS_MASK
    | KEY_RELEASE_MASK
    | BUTTON_PRESS_MASK
    | BUTTON_RELEASE_MASK
    | ENTER_WINDOW_MASK
    | LEAVE_WINDOW_MASK
    | POINTER_MOTION_MASK
    | EXPOSURE_MASK
    | STRUCTURE_NOTIFY_MASK
    | FOCUS_CHANGE_MASK;

/// What a taken pointer still reports, to this window alone.
const POINTER_EVENTS: c_uint =
    (BUTTON_PRESS_MASK | BUTTON_RELEASE_MASK | POINTER_MOTION_MASK) as c_uint;

/// `NotifyNormal`: a crossing the person made, not one a grab made.
const NOTIFY_NORMAL: c_int = 0;
/// `NotifyPointer`: a focus event about where the pointer is, not about this
/// window taking the keyboard.
const NOTIFY_POINTER: c_int = 5;
/// `GrabModeAsync`, `GrabSuccess` and `CurrentTime`.
const GRAB_MODE_ASYNC: c_int = 1;
const GRAB_SUCCESS: c_int = 0;
const CURRENT_TIME: Xid = 0;
/// `ZPixmap` and `LSBFirst`: whole pixels, least significant byte first.
const Z_PIXMAP: c_int = 2;
const LSB_FIRST: c_int = 0;
/// The colour masks the guest's pixels already have: four bytes a pixel,
/// blue first, the top byte unused, which is DirectDraw's RGB32.
const RED: c_ulong = 0x00ff_0000;
const GREEN: c_ulong = 0x0000_ff00;
const BLUE: c_ulong = 0x0000_00ff;

/// Xlib's entry points, by name, with the signatures `Xlib.h` and `XKBlib.h`
/// give them.
macro_rules! xlib {
    ($($field:ident = $name:literal: fn($($arg:ty),*) -> $ret:ty;)*) => {
        struct Xlib {
            /// Kept open for as long as any of the functions may be called.
            _lib: sys::Library,
            $($field: unsafe extern "C" fn($($arg),*) -> $ret,)*
        }

        impl Xlib {
            fn open() -> Result<Self, HostError> {
                let lib = sys::Library::open("libX11.so.6")?;
                // Safety: each type is the signature Xlib's headers give the
                // function of that name.
                unsafe {
                    Ok(Self {
                        $($field: lib.function($name)?,)*
                        _lib: lib,
                    })
                }
            }
        }
    };
}

xlib! {
    open_display = "XOpenDisplay": fn(*const c_char) -> *mut Display;
    close_display = "XCloseDisplay": fn(*mut Display) -> c_int;
    default_screen = "XDefaultScreen": fn(*mut Display) -> c_int;
    root_window = "XRootWindow": fn(*mut Display, c_int) -> Xid;
    black_pixel = "XBlackPixel": fn(*mut Display, c_int) -> c_ulong;
    default_depth = "XDefaultDepth": fn(*mut Display, c_int) -> c_int;
    default_visual = "XDefaultVisual": fn(*mut Display, c_int) -> *mut Visual;
    default_gc = "XDefaultGC": fn(*mut Display, c_int) -> Gc;
    create_simple_window = "XCreateSimpleWindow": fn(
        *mut Display, Xid, c_int, c_int, c_uint, c_uint, c_uint, c_ulong, c_ulong
    ) -> Xid;
    destroy_window = "XDestroyWindow": fn(*mut Display, Xid) -> c_int;
    store_name = "XStoreName": fn(*mut Display, Xid, *const c_char) -> c_int;
    select_input = "XSelectInput": fn(*mut Display, Xid, c_long) -> c_int;
    intern_atom = "XInternAtom": fn(*mut Display, *const c_char, c_int) -> Xid;
    set_wm_protocols = "XSetWMProtocols": fn(*mut Display, Xid, *mut Xid, c_int) -> c_int;
    map_window = "XMapWindow": fn(*mut Display, Xid) -> c_int;
    pending = "XPending": fn(*mut Display) -> c_int;
    next_event = "XNextEvent": fn(*mut Display, *mut XEvent) -> c_int;
    flush = "XFlush": fn(*mut Display) -> c_int;
    init_image = "XInitImage": fn(*mut XImage) -> c_int;
    put_image = "XPutImage": fn(
        *mut Display, Xid, Gc, *mut XImage, c_int, c_int, c_int, c_int, c_uint, c_uint
    ) -> c_int;
    lookup_keysym = "XLookupKeysym": fn(*mut XKeyEvent, c_int) -> Xid;
    detectable_auto_repeat = "XkbSetDetectableAutoRepeat": fn(
        *mut Display, c_int, *mut c_int
    ) -> c_int;
    grab_pointer = "XGrabPointer": fn(
        *mut Display, Xid, c_int, c_uint, c_int, c_int, Xid, Xid, Xid
    ) -> c_int;
    ungrab_pointer = "XUngrabPointer": fn(*mut Display, Xid) -> c_int;
    warp_pointer = "XWarpPointer": fn(
        *mut Display, Xid, Xid, c_int, c_int, c_uint, c_uint, c_int, c_int
    ) -> c_int;
    create_bitmap_from_data = "XCreateBitmapFromData": fn(
        *mut Display, Xid, *const c_char, c_uint, c_uint
    ) -> Xid;
    create_pixmap_cursor = "XCreatePixmapCursor": fn(
        *mut Display, Xid, Xid, *mut XColor, *mut XColor, c_uint, c_uint
    ) -> Xid;
    free_pixmap = "XFreePixmap": fn(*mut Display, Xid) -> c_int;
    free_cursor = "XFreeCursor": fn(*mut Display, Xid) -> c_int;
}

/// A window on the host's screen, showing what the guest drew.
pub struct LinuxWindow {
    x: Xlib,
    display: *mut Display,
    window: Xid,
    gc: Gc,
    depth: c_int,
    /// A cursor with nothing in it, shown while the pointer is taken.
    blank: Xid,
    wm_protocols: Xid,
    wm_delete_window: Xid,
    /// The window's size as the window manager last said it was.
    window_size: (u32, u32),
    /// The size of the last frame presented, which the window scales.
    frame_size: (u32, u32),
    /// The last frame presented, as the guest drew it, and its pitch: drawn
    /// again whenever the window is uncovered or changes size, which goes on
    /// after the run has ended and the window is still open.
    frame: Vec<u8>,
    pitch: usize,
    /// The frame scaled to the window, as the X server is handed it.
    shown: Vec<u8>,
    /// Events a person made since the last flip.
    pending: Vec<Input>,
    /// The keys the guest has been told are down.
    keys: Vec<u32>,
    /// Where the mix is heard, when `--audio` asked for it.
    audio: Option<LinuxAudio>,
    /// Whether the person closed the window.
    closed: bool,
    /// Whether the title asked for the host's pointer to be hidden.
    pointer_hidden: bool,
    /// Whether the person's own pointer is over this window. Starts false,
    /// because an X window can open anywhere on the screen, and X says so
    /// when the pointer is over it.
    pointer_over_us: bool,
    /// Whether this window has the keyboard.
    focused: bool,
    /// The pointer while it is taken, held in the middle of the window; none
    /// while it is the person's own.
    taken: Option<Held>,
    /// Where the person's own pointer was last seen in the window, which is
    /// where it is put back when it is given back.
    seen_at: (i32, i32),
    /// The pointer the guest is handed while the pointer is taken.
    steer: Steer,
    /// The host's clock against the declared one (ADR-0003).
    pace: Pace,
    /// Carries the mix's own tail from one stretched block to the next.
    stretch: Stretch,
}

/// Whether the screen's own visual takes the guest's pixels as they are.
///
/// # Safety
/// `visual` must be null or the screen's own `Visual`.
unsafe fn takes_guest_pixels(visual: *const Visual, depth: c_int) -> bool {
    // Safety: as the caller promises.
    let Some(visual) = (unsafe { visual.as_ref() }) else {
        return false;
    };
    matches!(depth, 24 | 32)
        && (visual.red_mask, visual.green_mask, visual.blue_mask) == (RED, GREEN, BLUE)
}

impl LinuxWindow {
    /// Open a window of this size, titled after the title being run, with
    /// the declared machine's own pointer speed ([`Steer`]).
    ///
    /// # Errors
    /// When libX11 cannot be opened, there is no X display, or the display
    /// does not show 24-bit colour as the guest draws it.
    pub fn open(
        name: &str,
        width: u32,
        height: u32,
        pointer_speed: (f64, f64),
    ) -> Result<Self, HostError> {
        let x = Xlib::open()?;
        // Safety: each call is made through the signature Xlib gives it, on
        // the display it opened and the window it made.
        unsafe {
            let display = (x.open_display)(std::ptr::null());
            if display.is_null() {
                return Err(HostError::Unsupported(
                    "no X display to open a window on; is DISPLAY set?",
                ));
            }
            let screen = (x.default_screen)(display);
            let depth = (x.default_depth)(display, screen);
            if !takes_guest_pixels((x.default_visual)(display, screen), depth) {
                (x.close_display)(display);
                return Err(HostError::Unsupported(
                    "the X display does not show 24-bit true colour",
                ));
            }
            let black = (x.black_pixel)(display, screen);
            let window = (x.create_simple_window)(
                display,
                (x.root_window)(display, screen),
                0,
                0,
                width,
                height,
                0,
                black,
                black,
            );
            let title = CString::new(name).unwrap_or_default();
            (x.store_name)(display, window, title.as_ptr());
            (x.select_input)(display, window, EVENTS);

            // The close button is the window manager's, and it says it was
            // pressed with a message rather than closing the window itself.
            let wm_protocols = (x.intern_atom)(display, c"WM_PROTOCOLS".as_ptr(), 0);
            let mut wm_delete_window = (x.intern_atom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            (x.set_wm_protocols)(display, window, &raw mut wm_delete_window, 1);

            // A key held down repeats as presses alone, with no release
            // between them, so that a repeat can be told from a second press
            // (`x11::key`). A server without it repeats as releases and
            // presses, which the guest would see as the key tapped.
            // ponytail: no fallback for a server without XKB; every server a
            // stock desktop runs has it.
            let mut supported = 0;
            (x.detectable_auto_repeat)(display, 1, &raw mut supported);

            let empty = [0 as c_char; 8];
            let bitmap = (x.create_bitmap_from_data)(display, window, empty.as_ptr(), 8, 8);
            let mut colour = XColor::default();
            let blank = (x.create_pixmap_cursor)(
                display,
                bitmap,
                bitmap,
                &raw mut colour,
                &raw mut colour,
                0,
                0,
            );
            (x.free_pixmap)(display, bitmap);

            let gc = (x.default_gc)(display, screen);
            (x.map_window)(display, window);
            (x.flush)(display);
            Ok(Self {
                x,
                display,
                window,
                gc,
                depth,
                blank,
                wm_protocols,
                wm_delete_window,
                window_size: (width, height),
                frame_size: (width, height),
                frame: Vec::new(),
                pitch: 0,
                shown: Vec::new(),
                pending: Vec::new(),
                keys: Vec::new(),
                audio: None,
                closed: false,
                pointer_hidden: false,
                pointer_over_us: false,
                focused: false,
                taken: None,
                seen_at: (0, 0),
                steer: Steer::new(
                    (
                        i32::try_from(width / 2).unwrap_or(320),
                        i32::try_from(height / 2).unwrap_or(240),
                    ),
                    pointer_speed,
                ),
                pace: Pace::new(),
                stretch: Stretch::default(),
            })
        }
    }

    /// Whether the person has closed the window.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Let the mix be heard through ALSA's default output from now on, or
    /// say why it cannot be.
    ///
    /// # Errors
    /// When libasound cannot be opened, or there is no output to open.
    pub fn open_audio(&mut self) -> Result<(), HostError> {
        self.audio = Some(LinuxAudio::open()?);
        Ok(())
    }

    /// Stop the output, once there is no more mix to hear.
    pub fn close_audio(&mut self) {
        self.audio = None;
    }

    /// Show the last frame presented, scaled to the window, its aspect kept.
    /// The bars either side are the window's own black background, which the
    /// X server paints.
    // ponytail: the scaled frame goes to the server through the socket,
    // every frame, which is about 1.2 MB at 640x480 and 5 MB at twice that.
    // MIT-SHM, or a server-side scale through XRender, if a large window
    // cannot keep pace.
    fn draw(&mut self) {
        if self.frame.is_empty() {
            return;
        }
        let p = x11::picture(self.window_size, self.frame_size);
        x11::scale(
            &self.frame,
            self.pitch,
            self.frame_size,
            (p.width, p.height),
            &mut self.shown,
        );
        let (Ok(width), Ok(height), Ok(bytes_per_line)) = (
            c_int::try_from(p.width),
            c_int::try_from(p.height),
            c_int::try_from(p.width * 4),
        ) else {
            return;
        };
        let mut image = XImage {
            width,
            height,
            xoffset: 0,
            format: Z_PIXMAP,
            data: self.shown.as_mut_ptr().cast(),
            byte_order: LSB_FIRST,
            bitmap_unit: 32,
            bitmap_bit_order: LSB_FIRST,
            bitmap_pad: 32,
            depth: self.depth,
            bytes_per_line,
            bits_per_pixel: 32,
            red_mask: RED,
            green_mask: GREEN,
            blue_mask: BLUE,
            obdata: std::ptr::null_mut(),
            funcs: [std::ptr::null_mut(); 6],
        };
        // Safety: an image over `shown`, which outlives the call; Xlib copies
        // the pixels out before XPutImage returns, converting their byte
        // order if the server's is not this machine's.
        unsafe {
            if (self.x.init_image)(&raw mut image) == 0 {
                return;
            }
            (self.x.put_image)(
                self.display,
                self.window,
                self.gc,
                &raw mut image,
                0,
                0,
                p.x,
                p.y,
                p.width,
                p.height,
            );
            (self.x.flush)(self.display);
        }
    }

    /// Take everything the X server has for us, turning what a person did
    /// into guest input and noticing a close.
    pub fn pump(&mut self) {
        // Safety: the display is open, and each event is read as the type
        // its own `kind` says it is.
        unsafe {
            while (self.x.pending)(self.display) > 0 {
                let mut event = XEvent { pad: [0; 24] };
                (self.x.next_event)(self.display, &raw mut event);
                self.event(&mut event);
            }
        }
        // Whether the person's pointer is over the window and whether the
        // window has the keyboard both change without the title being told,
        // so this is settled every time the server is asked what happened.
        self.take_pointer_where_it_belongs();
        // A taken pointer that has strayed from the middle is moved back
        // there, once every event already sent has been measured.
        if let Some((x, y)) = self.taken.as_mut().and_then(|h| h.stray(self.window_size)) {
            // Safety: the window is ours and the pointer is grabbed to it.
            unsafe {
                (self.x.warp_pointer)(self.display, 0, self.window, 0, 0, 0, 0, x, y);
                (self.x.flush)(self.display);
            }
        }
    }

    /// One event from the server.
    ///
    /// # Safety
    /// `event` must have come from `XNextEvent`.
    unsafe fn event(&mut self, event: &mut XEvent) {
        // Safety: every event starts with its type, and is then read as the
        // event of that type.
        unsafe {
            match event.kind {
                KEY_PRESS | KEY_RELEASE => {
                    let down = event.kind == KEY_PRESS;
                    let (state, keycode) = (event.key.state, event.key.keycode);
                    // Control-Q is how a person quits, as Command-Q is on the
                    // Mac, and the Q goes no further.
                    if down && x11::quits((self.x.lookup_keysym)(&raw mut event.key, 0), state) {
                        self.closed = true;
                        return;
                    }
                    if let Some(input) = x11::direct_input_key(keycode)
                        .and_then(|code| x11::key(&mut self.keys, code, down))
                    {
                        self.pending.push(input);
                    }
                }
                // The left button is X's first, the right its third. Every
                // press reaches the guest: the window's frame, where a person
                // takes hold of it, is the window manager's own window, not
                // this one (#49). A release is never dropped.
                BUTTON_PRESS | BUTTON_RELEASE => {
                    let button = match event.button.button {
                        1 => Button::Left,
                        3 => Button::Right,
                        _ => return,
                    };
                    self.pending.push(if event.kind == BUTTON_PRESS {
                        Input::Down(button)
                    } else {
                        Input::Up(button)
                    });
                }
                MOTION_NOTIFY => self.moved(event.motion.x, event.motion.y),
                // A crossing that a grab made says nothing about where the
                // person is.
                ENTER_NOTIFY | LEAVE_NOTIFY if event.crossing.mode == NOTIFY_NORMAL => {
                    if event.kind == ENTER_NOTIFY {
                        self.pointer_over_us = true;
                        self.seen_at = (event.crossing.x, event.crossing.y);
                    } else if moved(self.taken.is_some(), false) == Moved::Elsewhere {
                        self.pointer_over_us = false;
                    }
                }
                FOCUS_IN | FOCUS_OUT if event.focus.detail != NOTIFY_POINTER => {
                    self.focused = event.kind == FOCUS_IN;
                    // A key let go while another window has the keyboard is
                    // never sent here, so what was held is let go when the
                    // window loses it.
                    if !self.focused {
                        for code in self.keys.drain(..) {
                            self.pending.push(Input::Key { code, down: false });
                        }
                    }
                }
                EXPOSE if event.expose.count == 0 => self.draw(),
                CONFIGURE_NOTIFY => {
                    let size = |n: c_int| u32::try_from(n).unwrap_or(1).max(1);
                    self.window_size = (size(event.configure.width), size(event.configure.height));
                }
                CLIENT_MESSAGE => {
                    let client = event.client;
                    if client.message_type == self.wm_protocols
                        && client.data[0] as Xid == self.wm_delete_window
                    {
                        self.closed = true;
                    }
                }
                DESTROY_NOTIFY => self.closed = true,
                _ => {}
            }
        }
    }

    /// The pointer moved within the window, which is one of three things
    /// ([`moved`]).
    fn moved(&mut self, x: i32, y: i32) {
        match moved(self.taken.is_some(), true) {
            // The person's pointer is held in the middle of the window, so
            // where it is says nothing; how far it went is the whole message.
            Moved::Steered => {
                let Some((dx, dy)) = self.taken.as_mut().and_then(|h| h.seen(x, y)) else {
                    return;
                };
                let window = (f64::from(self.window_size.0), f64::from(self.window_size.1));
                if let Some((x, y)) =
                    self.steer
                        .moved((f64::from(dx), f64::from(dy)), window, self.frame_size)
                {
                    self.pending.push(Input::Move { x, y });
                }
            }
            // Where the title draws its own pointer, the person's is not a
            // place the guest's pointer should be moved to: all this says
            // then is that the person is over the window (#40, #86).
            Moved::Placed => {
                self.pointer_over_us = true;
                self.seen_at = (x, y);
                if !self.pointer_hidden {
                    let at = x11::guest_point(self.window_size, self.frame_size, x, y);
                    self.steer.placed(at);
                    self.pending.push(Input::Move { x: at.0, y: at.1 });
                }
            }
            Moved::Elsewhere => self.pointer_over_us = false,
        }
    }

    /// Take the pointer from the person, or give it back, as the title, the
    /// person's pointer and the keyboard now say.
    ///
    /// Taken only while the title draws its own pointer, the person's is
    /// over this window and this window has the keyboard. Anywhere else the
    /// pointer is the person's and they are using it, and taking it from
    /// them there is taking their machine.
    ///
    /// Taking it grabs it to this window with a cursor that shows nothing,
    /// so the mouse moves only the guest's pointer. Giving it back puts it
    /// where it was seen last before it was taken, so it comes back where it
    /// went, as the Mac's frozen one does. The grab and the cursor are the
    /// connection's, so a run killed outright gives the pointer back with
    /// its connection: there is nothing to restore from a signal, as there
    /// is on the Mac (#45).
    fn take_pointer_where_it_belongs(&mut self) {
        let take = self.pointer_hidden && self.pointer_over_us && self.focused;
        if take == self.taken.is_some() {
            return;
        }
        // Safety: the window is ours and the display open.
        unsafe {
            if take {
                let grabbed = (self.x.grab_pointer)(
                    self.display,
                    self.window,
                    0,
                    POINTER_EVENTS,
                    GRAB_MODE_ASYNC,
                    GRAB_MODE_ASYNC,
                    self.window,
                    self.blank,
                    CURRENT_TIME,
                );
                // Refused, as it is while another client holds a grab: asked
                // again at the next pump.
                if grabbed != GRAB_SUCCESS {
                    return;
                }
                self.taken = Some(Held::new(self.seen_at, self.window_size));
            } else {
                let (x, y) = self.seen_at;
                (self.x.warp_pointer)(self.display, 0, self.window, 0, 0, 0, 0, x, y);
                (self.x.ungrab_pointer)(self.display, CURRENT_TIME);
                self.taken = None;
            }
            (self.x.flush)(self.display);
        }
    }
}

impl Drop for LinuxWindow {
    fn drop(&mut self) {
        self.audio = None;
        self.show_pointer(true);
        // Safety: the cursor and window are ours, and the display is closed
        // last and once.
        unsafe {
            (self.x.free_cursor)(self.display, self.blank);
            (self.x.destroy_window)(self.display, self.window);
            (self.x.close_display)(self.display);
        }
    }
}

impl Host for LinuxWindow {
    fn present(&mut self, frame: &Frame<'_>) {
        // A frame shorter than it says it is is not shown at all (#94).
        let len = (frame.pitch * frame.height) as usize;
        let Some(bytes) = frame.pixels.get(..len) else {
            return;
        };
        self.frame_size = (frame.width, frame.height);
        self.frame.clear();
        self.frame.extend_from_slice(bytes);
        self.pitch = frame.pitch as usize;
        self.draw();
        self.pump();
    }

    fn poll_input(&mut self, _flip: u64) -> Vec<Input> {
        self.pump();
        self.steer.drained();
        std::mem::take(&mut self.pending)
    }

    fn audio(&mut self, samples: &[i16]) {
        // Stretched to the pace the host keeps, as on the Mac (#75).
        let stretched = self.stretch.block(samples, self.pace.current());
        if let Some(device) = &self.audio {
            device.play(&stretched);
        }
    }

    fn show_pointer(&mut self, show: bool) {
        self.pointer_hidden = !show;
        self.take_pointer_where_it_belongs();
    }

    fn put_pointer(&mut self, x: i32, y: i32) {
        // As on the Mac: the pointer the guest is handed starts again from
        // where the title put it, the person's own is never moved under
        // their hand (#46), and a move already waiting was measured from a
        // place the title has moved on from ([`Steer::put`], #86).
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
        self.audio.as_ref().map(LinuxAudio::underruns)
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
