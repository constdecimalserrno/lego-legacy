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
        let p = self.core_graphics.symbol(name)?;
        Ok(unsafe { std::mem::transmute_copy(&p) })
    }
}

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
    let scale = (cw / fw).min(ch / fh).max(f64::MIN_POSITIVE);
    let (left, bottom) = ((cw - fw * scale) / 2.0, (ch - fh * scale) / 2.0);
    let gx = ((x - left) / scale).clamp(0.0, fw - 1.0);
    let gy = ((ch - y - bottom) / scale).clamp(0.0, fh - 1.0);
    (gx as i32, gy as i32)
}

/// The DirectInput key number for a Mac key, by where the key sits on the
/// keyboard rather than what it types.
///
/// Letters, digits, punctuation, the arrows, the function keys and the few keys
/// around them. The modifiers come as flag changes rather than key presses and
/// are not here yet; a key that is not here reaches the guest as nothing.
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
        0x60 => 0x3f, // F5
        0x61 => 0x40, // F6
        0x62 => 0x41, // F7
        0x63 => 0x3d, // F3
        0x64 => 0x42, // F8
        0x65 => 0x43, // F9
        0x67 => 0x57, // F11
        0x6d => 0x44, // F10
        0x6f => 0x58, // F12
        0x76 => 0x3e, // F4
        0x78 => 0x3c, // F2
        0x7a => 0x3b, // F1
        0x7b => 0xcb, // Left
        0x7c => 0xcd, // Right
        0x7d => 0xd0, // Down
        0x7e => 0xc8, // Up
        _ => return None,
    })
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
    /// Whether the person closed the window.
    closed: bool,
    /// How far behind the declared clock the host has fallen and will not
    /// make up: a run the host could not keep pace with carries on in real
    /// time from where it got to, rather than racing to catch up.
    lag_nanos: u64,
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

            Ok(Self {
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
                closed: false,
                lag_nanos: 0,
                frame_size: (width, height),
            })
        }
    }

    /// Whether the person has closed the window.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed
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
                match kind {
                    // Mouse down, mouse up, mouse moved, dragged.
                    1 => self.pending.push(Input::Down),
                    2 => self.pending.push(Input::Up),
                    5 | 6 => {
                        let p: Point =
                            self.rt.send::<MsgPoint>()(event, self.rt.sel("locationInWindow"));
                        let view: Id =
                            self.rt.send::<MsgId>()(self.window, self.rt.sel("contentView"));
                        let bounds: Rect = self.rt.send::<MsgRect>()(view, self.rt.sel("bounds"));
                        let (x, y) =
                            guest_point((bounds.width, bounds.height), self.frame_size, p.x, p.y);
                        self.pending.push(Input::Move { x, y });
                    }
                    // Key down and key up.
                    10 | 11 => {
                        let key: u64 = self.rt.send::<MsgU64>()(event, self.rt.sel("keyCode"));
                        if let Some(code) = direct_input_key(key) {
                            self.pending.push(Input::Key {
                                code,
                                down: kind == 10,
                            });
                        }
                    }
                    _ => {}
                }
                self.rt.send::<MsgVoidId>()(self.app, self.rt.sel("sendEvent:"), event);
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
    }
}

impl Drop for MacWindow {
    fn drop(&mut self) {
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
            let bitmap_info = 1 << 14 | 2; // little-endian, no alpha
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

    fn audio(&mut self, _samples: &[i16]) {
        // No host audio device yet; the machine mixes either way.
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

    fn pace(&mut self, declared_nanos: u64) {
        // A game-second takes a wall-second (ADR-0003): if the machine is
        // ahead of its own clock, it waits.
        let wall = self.wall_nanos();
        self.lag_nanos = self.lag_nanos.max(wall.saturating_sub(declared_nanos));
        let due = declared_nanos + self.lag_nanos;
        if due > wall {
            std::thread::sleep(std::time::Duration::from_nanos(due - wall));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{direct_input_key, guest_point};

    #[test]
    fn a_mac_key_reaches_the_guest_as_the_key_in_the_same_place() {
        assert_eq!(direct_input_key(0x35), Some(0x01)); // Escape
        assert_eq!(direct_input_key(0x00), Some(0x1e)); // A
        assert_eq!(direct_input_key(0x24), Some(0x1c)); // Return
        assert_eq!(direct_input_key(0x7e), Some(0xc8)); // Up
        assert_eq!(direct_input_key(0x3a), None); // Option, a modifier
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
