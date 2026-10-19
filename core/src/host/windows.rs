//! The Windows backend (ADR-0001, #107).
//!
//! user32 for the window, its messages and the pointer, and gdi32 for putting
//! the guest's frame in it, both loaded by name when the window opens and read
//! with `GetProcAddress`; the sound is winmm's (`windows_audio`). Only the
//! operating system's own libraries are loaded, from its own directory, and
//! nothing Microsoft redistributes is installed or run (ADR-0002).
//!
//! What can be worked out without Windows is in `windows_input`, `pointer`,
//! `pace` and `feed`, which every host's tests hold, and what is here is the
//! calls into Windows. It has been compiled for x86-64 and Arm64 Windows, and
//! run on x86-64 Windows in a VM with no sound device (#107).
//!
//! The window presents through `StretchDIBits` on purpose, as macOS presents
//! through a layer: what the guest draws is already a finished frame in its
//! own memory, and gdi32 scales it into the window with nothing asked of us
//! per pixel.

use super::pace::Pace;
use super::pointer::{moved, Moved, Steer};
use super::stretch::Stretch;
use super::windows_input::{self as input, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};
use super::{sys, Button, Frame, Host, HostError, Input, Mapping};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

type Handle = *mut c_void;
type WndProc = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;
type ClipCursor = unsafe extern "system" fn(*const Rect) -> i32;

const WM_PAINT: u32 = 0x000f;
const WM_CLOSE: u32 = 0x0010;
const WM_INPUT: u32 = 0x00ff;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
/// The buttons a mouse message's `wParam` says are still held.
const MK_BUTTONS: usize = 0x0001 | 0x0002;
const PM_REMOVE: u32 = 0x0001;
/// Titled, with a system menu, a frame to resize by, and the buttons to
/// minimise, maximise and close: `WS_OVERLAPPEDWINDOW`, shown.
const WS_OVERLAPPEDWINDOW: u32 = 0x00cf_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const CW_USEDEFAULT: i32 = i32::MIN;
/// Repainted whole when resized either way, so the bars are drawn again.
const CS_REDRAW: u32 = 0x0001 | 0x0002;
const IDC_ARROW: usize = 32512;
const BLACK_BRUSH: i32 = 4;
const RID_INPUT: u32 = 0x1000_0003;
const RIM_TYPEMOUSE: u32 = 0;
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;
/// Nearest pixel, as the macOS layer's `nearest` filter: rows and columns are
/// repeated or dropped, never blended.
const COLORONCOLOR: i32 = 3;
const DIB_RGB_COLORS: u32 = 0;
const SRCCOPY: u32 = 0x00cc_0020;
const BI_RGB: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// A `MSG`.
#[repr(C)]
struct Msg {
    hwnd: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
    private: u32,
}

/// A `WNDCLASSEXW`.
#[repr(C)]
struct WndClassExW {
    size: u32,
    style: u32,
    window_proc: WndProc,
    class_extra: i32,
    window_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    icon_small: Handle,
}

/// A `PAINTSTRUCT`.
#[repr(C)]
struct PaintStruct {
    hdc: Handle,
    erase: i32,
    paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

/// A `BITMAPINFOHEADER`.
#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

/// A `BITMAPINFO`, whose one colour is never read for a 32-bit frame.
#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

/// A `RAWINPUTDEVICE`.
#[repr(C)]
struct RawInputDevice {
    usage_page: u16,
    usage: u16,
    flags: u32,
    target: Handle,
}

/// A `RAWINPUTHEADER`.
#[repr(C)]
struct RawInputHeader {
    kind: u32,
    size: u32,
    device: Handle,
    wparam: usize,
}

/// A `RAWMOUSE`: the union of `ulButtons` and its two halves is `buttons`.
#[repr(C)]
struct RawMouse {
    flags: u16,
    buttons: u32,
    raw_buttons: u32,
    last_x: i32,
    last_y: i32,
    extra: u32,
}

/// A `RAWINPUT` as a mouse sends it, the only device registered.
#[repr(C)]
struct RawInput {
    header: RawInputHeader,
    mouse: RawMouse,
}

/// A plain struct with all of its bytes zero, which is how Windows' own
/// headers have them made.
///
/// # Safety
/// `T` must be one of the plain structs above, for which zero is a value.
unsafe fn zeroed<T>() -> T {
    unsafe { std::mem::zeroed() }
}

/// A Rust string as Windows takes it: UTF-16, NUL-terminated.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
}

/// The user32 entry points the window uses, with the signatures winuser.h
/// gives them.
struct User32 {
    _lib: sys::Library,
    register_class: unsafe extern "system" fn(*const WndClassExW) -> u16,
    create_window: unsafe extern "system" fn(
        u32,
        *const u16,
        *const u16,
        u32,
        i32,
        i32,
        i32,
        i32,
        Handle,
        Handle,
        Handle,
        *mut c_void,
    ) -> Handle,
    destroy_window: unsafe extern "system" fn(Handle) -> i32,
    def_window_proc: WndProc,
    peek_message: unsafe extern "system" fn(*mut Msg, Handle, u32, u32, u32) -> i32,
    dispatch_message: unsafe extern "system" fn(*const Msg) -> isize,
    adjust_window_rect: unsafe extern "system" fn(*mut Rect, u32, i32, u32) -> i32,
    client_rect: unsafe extern "system" fn(Handle, *mut Rect) -> i32,
    invalidate_rect: unsafe extern "system" fn(Handle, *const Rect, i32) -> i32,
    begin_paint: unsafe extern "system" fn(Handle, *mut PaintStruct) -> Handle,
    end_paint: unsafe extern "system" fn(Handle, *const PaintStruct) -> i32,
    load_cursor: unsafe extern "system" fn(Handle, *const u16) -> Handle,
    show_cursor: unsafe extern "system" fn(i32) -> i32,
    clip_cursor: ClipCursor,
    cursor_pos: unsafe extern "system" fn(*mut Point) -> i32,
    screen_to_client: unsafe extern "system" fn(Handle, *mut Point) -> i32,
    window_from_point: unsafe extern "system" fn(Point) -> Handle,
    foreground_window: unsafe extern "system" fn() -> Handle,
    set_capture: unsafe extern "system" fn(Handle) -> Handle,
    release_capture: unsafe extern "system" fn() -> i32,
    register_raw_input: unsafe extern "system" fn(*const RawInputDevice, u32, u32) -> i32,
    raw_input_data: unsafe extern "system" fn(isize, u32, *mut c_void, *mut u32, u32) -> u32,
    system_metrics: unsafe extern "system" fn(i32) -> i32,
}

impl User32 {
    fn open() -> Result<Self, HostError> {
        let lib = sys::Library::open("user32.dll")?;
        // Safety: each type is the signature winuser.h gives the function.
        unsafe {
            Ok(Self {
                register_class: lib.function("RegisterClassExW")?,
                create_window: lib.function("CreateWindowExW")?,
                destroy_window: lib.function("DestroyWindow")?,
                def_window_proc: lib.function("DefWindowProcW")?,
                peek_message: lib.function("PeekMessageW")?,
                dispatch_message: lib.function("DispatchMessageW")?,
                adjust_window_rect: lib.function("AdjustWindowRectEx")?,
                client_rect: lib.function("GetClientRect")?,
                invalidate_rect: lib.function("InvalidateRect")?,
                begin_paint: lib.function("BeginPaint")?,
                end_paint: lib.function("EndPaint")?,
                load_cursor: lib.function("LoadCursorW")?,
                show_cursor: lib.function("ShowCursor")?,
                clip_cursor: lib.function("ClipCursor")?,
                cursor_pos: lib.function("GetCursorPos")?,
                screen_to_client: lib.function("ScreenToClient")?,
                window_from_point: lib.function("WindowFromPoint")?,
                foreground_window: lib.function("GetForegroundWindow")?,
                set_capture: lib.function("SetCapture")?,
                release_capture: lib.function("ReleaseCapture")?,
                register_raw_input: lib.function("RegisterRawInputDevices")?,
                raw_input_data: lib.function("GetRawInputData")?,
                system_metrics: lib.function("GetSystemMetrics")?,
                _lib: lib,
            })
        }
    }
}

/// The gdi32 entry points presenting uses, with wingdi.h's signatures.
struct Gdi32 {
    _lib: sys::Library,
    stretch_dibits: unsafe extern "system" fn(
        Handle,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        *const c_void,
        *const BitmapInfo,
        u32,
        u32,
    ) -> i32,
    stretch_mode: unsafe extern "system" fn(Handle, i32) -> i32,
    stock_object: unsafe extern "system" fn(i32) -> Handle,
}

impl Gdi32 {
    fn open() -> Result<Self, HostError> {
        let lib = sys::Library::open("gdi32.dll")?;
        // Safety: each type is the signature wingdi.h gives the function.
        unsafe {
            Ok(Self {
                stretch_dibits: lib.function("StretchDIBits")?,
                stretch_mode: lib.function("SetStretchBltMode")?,
                stock_object: lib.function("GetStockObject")?,
                _lib: lib,
            })
        }
    }
}

/// `DefWindowProcW`, for the window procedure, which Windows calls with no
/// way to reach the window's own state; written once, before the window is
/// made.
static DEF_WINDOW_PROC: AtomicUsize = AtomicUsize::new(0);

/// Whether the person asked the window to close: the close button, its
/// system menu, or the console's own close, Ctrl-C or Ctrl-Break
/// ([`way_back`]). The window is not destroyed; the run is ended (#107).
static CLOSE_ASKED: AtomicBool = AtomicBool::new(false);

/// What Windows calls with a message sent to the window rather than posted
/// to it. Everything a person does is posted, and taken from the queue in
/// [`WinWindow::pump`]; all that is answered here is a close.
unsafe extern "system" fn window_proc(
    hwnd: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if message == WM_CLOSE {
        CLOSE_ASKED.store(true, Ordering::Relaxed);
        return 0;
    }
    // Safety: written with `DefWindowProcW`'s own address before the window
    // that calls this was made.
    unsafe {
        let def: WndProc = std::mem::transmute(DEF_WINDOW_PROC.load(Ordering::Relaxed));
        def(hwnd, message, wparam, lparam)
    }
}

/// Which button a mouse message is about.
fn button(message: u32) -> Button {
    if matches!(message, WM_RBUTTONDOWN | WM_RBUTTONUP) {
        Button::Right
    } else {
        Button::Left
    }
}

/// A mouse message's place in the client area, signed: a captured pointer
/// can be left of or above it.
fn client_xy(lparam: isize) -> (i32, i32) {
    (
        i32::from((lparam & 0xffff) as u16 as i16),
        i32::from(((lparam >> 16) & 0xffff) as u16 as i16),
    )
}

/// A window on the host's screen, showing what the guest drew.
pub struct WinWindow {
    user: User32,
    gdi: Gdi32,
    window: Handle,
    /// Events a person made since the last flip.
    pending: Vec<Input>,
    /// The keys the guest has been told are down.
    held: Vec<u32>,
    /// Where the mix is heard, when `--audio` asked for it.
    audio: Option<super::windows_audio::WinAudio>,
    /// Whether the person pressed Alt-F4. The close button and the console
    /// say so in [`CLOSE_ASKED`].
    closed: bool,
    /// Whether the title asked for the host's pointer to be hidden.
    pointer_hidden: bool,
    /// Whether the person's own pointer is over this window's picture. Asked
    /// of Windows every pump while the pointer is the person's, and never
    /// while it is taken: a taken pointer is frozen, so it is nowhere and
    /// cannot have left ([`moved`]).
    pointer_over_us: bool,
    /// Whether the host's pointer is hidden and frozen right now: only while
    /// the title wants it gone *and* the person is over this window with it
    /// in front, as on macOS.
    pointer_taken: bool,
    /// The one pixel the person's pointer is clipped to while it is taken.
    frozen: Rect,
    /// The pointer the guest is handed while the pointer is taken ([`Steer`]).
    steer: Steer,
    /// Where the last raw report that said where rather than how far put the
    /// pointer ([`input::raw_moved`]).
    raw_at: Option<(i32, i32)>,
    /// Presentation held to real time, and the pace it came to (#75).
    pace: Pace,
    /// Carries the mix's own tail from one stretched block to the next.
    stretch: Stretch,
    /// The size and row length of the last frame presented, and its bytes:
    /// kept, because Windows asks for the window to be painted again whenever
    /// it was covered, and after the run has ended too.
    frame_size: (u32, u32),
    pitch: u32,
    last: Vec<u8>,
}

impl WinWindow {
    /// Open a window whose picture is this size, titled after the title being
    /// run, with the declared machine's own pointer speed ([`Steer`]).
    ///
    /// # Errors
    /// When user32 or gdi32 cannot be loaded, or the window cannot be made.
    pub fn open(
        name: &str,
        width: u32,
        height: u32,
        pointer_speed: (f64, f64),
    ) -> Result<Self, HostError> {
        let user = User32::open()?;
        let gdi = Gdi32::open()?;
        DEF_WINDOW_PROC.store(user.def_window_proc as usize, Ordering::Relaxed);
        let class_name = wide("lego-legacy");
        let title = wide(name);
        let (w, h) = (
            i32::try_from(width).unwrap_or(640),
            i32::try_from(height).unwrap_or(480),
        );
        // Safety: each call is made with the types winuser.h declares, and
        // every pointer passed is live for the call.
        let window = unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let class = WndClassExW {
                size: std::mem::size_of::<WndClassExW>() as u32,
                style: CS_REDRAW,
                window_proc,
                class_extra: 0,
                window_extra: 0,
                instance,
                icon: std::ptr::null_mut(),
                cursor: (user.load_cursor)(std::ptr::null_mut(), IDC_ARROW as *const u16),
                background: (gdi.stock_object)(BLACK_BRUSH),
                menu_name: std::ptr::null(),
                class_name: class_name.as_ptr(),
                icon_small: std::ptr::null_mut(),
            };
            // A class already registered, by a window this process opened
            // before, is the same class; making the window says if not.
            (user.register_class)(&class);
            // The size asked for is the picture's; the window around it is
            // larger by its frame and title bar.
            let mut outer = Rect {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            (user.adjust_window_rect)(&mut outer, WS_OVERLAPPEDWINDOW, 0, 0);
            (user.create_window)(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                outer.right - outer.left,
                outer.bottom - outer.top,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null_mut(),
            )
        };
        if window.is_null() {
            return Err(os("CreateWindowExW"));
        }
        let ours = Self {
            user,
            gdi,
            window,
            pending: Vec::new(),
            held: Vec::new(),
            audio: None,
            closed: false,
            pointer_hidden: false,
            pointer_over_us: false,
            pointer_taken: false,
            frozen: Rect::default(),
            steer: Steer::new(
                (
                    i32::try_from(width / 2).unwrap_or(320),
                    i32::try_from(height / 2).unwrap_or(240),
                ),
                pointer_speed,
            ),
            raw_at: None,
            pace: Pace::new(),
            stretch: Stretch::default(),
            frame_size: (width, height),
            pitch: width * 4,
            last: Vec::new(),
        };
        // The mouse, as raw input: how far it went, which is all a frozen
        // pointer can say, as macOS's event deltas say it (#64).
        let mouse = RawInputDevice {
            usage_page: 0x01,
            usage: 0x02,
            flags: 0,
            target: window,
        };
        // Safety: one device, of the size Windows is told.
        let registered = unsafe {
            (ours.user.register_raw_input)(&mouse, 1, std::mem::size_of::<RawInputDevice>() as u32)
        };
        if registered == 0 {
            return Err(os("RegisterRawInputDevices"));
        }
        way_back::listen();
        Ok(ours)
    }

    /// Whether the person has closed the window, or the console. Read from
    /// the console's own thread as it is asked, not at the next pump: a run
    /// that is loading does not pump, and a closing console waits only five
    /// seconds for the run to end.
    #[must_use]
    pub fn closed(&self) -> bool {
        self.closed || CLOSE_ASKED.load(Ordering::Relaxed)
    }

    /// Let the mix be heard through Windows' default output from now on, or
    /// say why it cannot be.
    ///
    /// # Errors
    /// When there is no output to open.
    pub fn open_audio(&mut self) -> Result<(), HostError> {
        self.audio = Some(super::windows_audio::WinAudio::open()?);
        Ok(())
    }

    /// Stop the output, once there is no more mix to hear.
    pub fn close_audio(&mut self) {
        self.audio = None;
    }

    /// The client area's size, in its own pixels.
    fn client_size(&self) -> (i32, i32) {
        let mut r = Rect::default();
        // Safety: our own window, and a rectangle to write into.
        unsafe { (self.user.client_rect)(self.window, &mut r) };
        (r.right - r.left, r.bottom - r.top)
    }

    /// The client area's size, as [`Steer`] measures it.
    fn content(&self) -> (f64, f64) {
        let (w, h) = self.client_size();
        (f64::from(w), f64::from(h))
    }

    /// Whether the person's own pointer is over this window's client area,
    /// with nothing else in front of it there. The frame, the title bar and
    /// the edges a person resizes by are the window system's, and the pointer
    /// over them is the person's (#49).
    fn over_the_picture(&self) -> bool {
        let mut at = Point::default();
        // Safety: user32's own, with a point to write into.
        unsafe {
            if (self.user.cursor_pos)(&mut at) == 0
                || (self.user.window_from_point)(at) != self.window
            {
                return false;
            }
            (self.user.screen_to_client)(self.window, &mut at);
        }
        let (w, h) = self.client_size();
        at.x >= 0 && at.y >= 0 && at.x < w && at.y < h
    }

    /// Put the host's pointer where it belongs, which is wherever the person
    /// is not being shown the title's own, by macOS's rules.
    ///
    /// A title that draws its own pointer wants the host's out of the way, but
    /// only over its own window and only while it is the window in front.
    /// Anywhere else the pointer is the person's and they are using it, and
    /// taking it from them there is taking their machine. Taken, it is hidden
    /// once (`ShowCursor` counts, as AppKit does) and frozen where it is, by
    /// clipping it to the one pixel under it: the mouse then moves the
    /// guest's pointer by how far it went (#64, #40), read as raw input.
    fn draw_pointer_where_it_belongs(&mut self) {
        // Safety: user32's own, which takes nothing.
        let active = unsafe { (self.user.foreground_window)() } == self.window;
        if !self.pointer_taken {
            self.pointer_over_us = self.over_the_picture();
        }
        let take = self.pointer_hidden && self.pointer_over_us && active;
        if take != self.pointer_taken {
            self.pointer_taken = take;
            // Safety: user32's own, with a point to write into and a
            // rectangle, or none, to clip to.
            unsafe {
                if take {
                    // A run that takes the pointer arms the way back, because
                    // the pointer belongs to the person and not to the run
                    // (#45).
                    way_back::arm(self.user.clip_cursor);
                    let mut at = Point::default();
                    (self.user.cursor_pos)(&mut at);
                    self.frozen = Rect {
                        left: at.x,
                        top: at.y,
                        right: at.x + 1,
                        bottom: at.y + 1,
                    };
                    (self.user.show_cursor)(0);
                } else {
                    (self.user.show_cursor)(1);
                    (self.user.clip_cursor)(std::ptr::null());
                }
            }
        }
        // Windows lets go of a clip by itself when something else takes the
        // screen, so a taken pointer is frozen again on every pump rather
        // than once.
        if self.pointer_taken {
            // Safety: a rectangle of ours, for the call's duration.
            unsafe { (self.user.clip_cursor)(&self.frozen) };
        }
    }

    /// The person's own pointer moved over the window, to here in the client
    /// area: the place to hand the guest, unless the title draws its own,
    /// in which case where the person's pointer is says nothing (#40, #86).
    fn placed(&mut self, x: i32, y: i32) {
        let (w, h) = self.client_size();
        if self.pointer_hidden || x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let at = input::guest_point((w, h), self.frame_size, x, y);
        self.steer.placed(at);
        self.pending.push(Input::Move { x: at.0, y: at.1 });
    }

    /// A raw mouse report, while the pointer is taken: how far the mouse
    /// went is the whole message.
    fn steered(&mut self, lparam: isize) {
        // Safety: a buffer the size Windows is told, which a mouse's report
        // fits; anything larger is refused and comes back as no report.
        let report = unsafe {
            let mut raw: RawInput = zeroed();
            let mut size = std::mem::size_of::<RawInput>() as u32;
            let got = (self.user.raw_input_data)(
                lparam,
                RID_INPUT,
                (&raw mut raw).cast(),
                &mut size,
                std::mem::size_of::<RawInputHeader>() as u32,
            );
            (got != u32::MAX && raw.header.kind == RIM_TYPEMOUSE).then_some(raw.mouse)
        };
        let Some(mouse) = report else {
            return;
        };
        // Safety: user32's own, which takes a number.
        let (primary, desktop) = unsafe {
            let metric = |n| (self.user.system_metrics)(n);
            (
                (metric(SM_CXSCREEN), metric(SM_CYSCREEN)),
                (metric(SM_CXVIRTUALSCREEN), metric(SM_CYVIRTUALSCREEN)),
            )
        };
        let went = input::raw_moved(
            mouse.flags,
            mouse.last_x,
            mouse.last_y,
            &mut self.raw_at,
            primary,
            desktop,
        );
        if let Some((x, y)) = self.steer.moved(went, self.content(), self.frame_size) {
            self.pending.push(Input::Move { x, y });
        }
    }

    /// Draw the last frame presented, aspect kept, into a device context
    /// Windows handed over to be painted.
    fn draw(&self, hdc: Handle) {
        if self.last.is_empty() || hdc.is_null() {
            return;
        }
        let (x, y, w, h) = input::picture(self.client_size(), self.frame_size);
        let (fw, fh) = (
            i32::try_from(self.frame_size.0).unwrap_or(0),
            i32::try_from(self.frame_size.1).unwrap_or(0),
        );
        // The guest's surfaces are 32 bits a pixel, blue first, with the top
        // byte ignored, which is what a 32-bit `BI_RGB` bitmap is. Its rows
        // are the surface's pitch apart, so it is that wide and only the
        // visible part is taken; a negative height is rows top first, as the
        // guest's are.
        let info = BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width: i32::try_from(self.pitch / 4).unwrap_or(0),
                height: -fh,
                planes: 1,
                bit_count: 32,
                compression: BI_RGB,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0],
        };
        // Safety: the kept frame is `pitch * height` bytes, which is what
        // the bitmap described says it is.
        unsafe {
            (self.gdi.stretch_mode)(hdc, COLORONCOLOR);
            (self.gdi.stretch_dibits)(
                hdc,
                x,
                y,
                w,
                h,
                0,
                0,
                fw,
                fh,
                self.last.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }

    /// Paint what Windows asked to be painted. The black background is
    /// erased in first, where Windows wants it, which is the bars.
    fn paint(&self) {
        // Safety: our own window, and a paint record for it to fill and take
        // back.
        unsafe {
            let mut ps: PaintStruct = zeroed();
            let hdc = (self.user.begin_paint)(self.window, &mut ps);
            self.draw(hdc);
            (self.user.end_paint)(self.window, &ps);
        }
    }

    /// Take one message from the queue: turn what a person did into guest
    /// input, and say whether it is done with, rather than handed on to
    /// `DefWindowProcW`.
    fn take(&mut self, msg: &Msg) -> bool {
        // A key or mouse message says everything in the low 32 bits.
        let lparam = msg.lparam as u32;
        match msg.message {
            // A key this window took is a key the title read, and none is
            // handed on: `DefWindowProcW` answers Alt and F10 by entering the
            // window menu, which holds the thread in a loop of its own and
            // the run with it. Alt-F4 is answered here instead, as macOS
            // answers Cmd-Q, and its F4 goes no further let go than pressed
            // (`host::key_change`).
            WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP => {
                if input::closes(msg.message, msg.wparam, lparam) {
                    self.closed = true;
                } else if let Some(Input::Key { code, down }) = input::key(msg.message, lparam) {
                    self.pending
                        .extend(super::key_change(&mut self.held, code, down));
                }
                true
            }
            // A press on the window's own frame comes as a non-client
            // message, never as one of these: it is the person taking hold
            // of the window, not a click on the title (#49). The button is
            // captured while it is held, so that the release is seen
            // wherever it happens: a press the guest saw is a button it has
            // to see let go of.
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                self.pending.push(Input::Down(button(msg.message)));
                // Safety: our own window.
                unsafe { (self.user.set_capture)(self.window) };
                false
            }
            WM_LBUTTONUP | WM_RBUTTONUP => {
                self.pending.push(Input::Up(button(msg.message)));
                if msg.wparam & MK_BUTTONS == 0 {
                    // Safety: user32's own, which takes nothing.
                    unsafe { (self.user.release_capture)() };
                }
                false
            }
            // Where the person's pointer is, which is the whole message while
            // it is theirs; while it is taken it is frozen, and how far the
            // mouse went comes as raw input instead ([`moved`]).
            WM_MOUSEMOVE => {
                if moved(self.pointer_taken, true) == Moved::Placed {
                    let (x, y) = client_xy(msg.lparam);
                    self.placed(x, y);
                }
                false
            }
            // Handed on as well, because `DefWindowProcW` is what cleans up
            // after a raw report.
            WM_INPUT => {
                if moved(self.pointer_taken, true) == Moved::Steered {
                    self.steered(msg.lparam);
                }
                false
            }
            WM_PAINT => {
                self.paint();
                true
            }
            _ => false,
        }
    }

    /// Take everything Windows has for us, turning what a person did into
    /// guest input and noticing a close.
    pub fn pump(&mut self) {
        // Safety: the standard message loop, with Windows' own types; a
        // message is written into `msg` before it is read.
        unsafe {
            let mut msg: Msg = zeroed();
            while (self.user.peek_message)(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                if !self.take(&msg) {
                    (self.user.dispatch_message)(&msg);
                }
            }
        }
        // A key let go while another window has the keyboard is never sent
        // here, so what was held is let go when this one loses it.
        // Safety: user32's own, which takes nothing.
        if unsafe { (self.user.foreground_window)() } != self.window {
            for code in self.held.drain(..) {
                self.pending.push(Input::Key { code, down: false });
            }
        }
        // Where the person's pointer is and which window is in front both
        // change without the title being told, so this is settled every time
        // Windows is asked what happened.
        self.draw_pointer_where_it_belongs();
    }
}

/// What Windows last said went wrong, as a host error.
fn os(what: &'static str) -> HostError {
    HostError::Os {
        what,
        errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
    }
}

impl Drop for WinWindow {
    fn drop(&mut self) {
        // A run that ended with the pointer hidden and frozen would leave a
        // person without one.
        self.show_pointer(true);
        // Safety: destroying a window we made.
        unsafe { (self.user.destroy_window)(self.window) };
    }
}

impl Host for WinWindow {
    fn present(&mut self, frame: &Frame<'_>) {
        // Kept, because Windows may ask for it to be painted again at any
        // time, after the guest's memory is gone too.
        //
        // ponytail: one copy per presented frame, as on macOS. Measure before
        // painting from the guest's memory directly.
        // A frame shorter than it says it is would have gdi32 read past the
        // copy, so it is not shown at all (#94).
        let len = (frame.pitch * frame.height) as usize;
        let Some(bytes) = frame.pixels.get(..len) else {
            return;
        };
        // A frame of another size leaves other bars, so those are erased.
        let resized = (frame.width, frame.height) != self.frame_size;
        self.frame_size = (frame.width, frame.height);
        self.pitch = frame.pitch;
        self.last.clear();
        self.last.extend_from_slice(bytes);
        // Safety: our own window; the whole of it is to be painted again,
        // which the pump does.
        unsafe { (self.user.invalidate_rect)(self.window, std::ptr::null(), i32::from(resized)) };
        self.pump();
    }

    fn poll_input(&mut self, _flip: u64) -> Vec<Input> {
        self.pump();
        self.steer.drained();
        std::mem::take(&mut self.pending)
    }

    fn audio(&mut self, samples: &[i16]) {
        // Stretched as on macOS, so a host running slower than real time
        // does not leave the device dry between blocks (#75).
        let stretched = self.stretch.block(samples, self.pace.current());
        if let Some(device) = &self.audio {
            device.play(&stretched);
        }
    }

    fn show_pointer(&mut self, show: bool) {
        self.pointer_hidden = !show;
        self.draw_pointer_where_it_belongs();
    }

    fn put_pointer(&mut self, x: i32, y: i32) {
        // As on macOS: the pointer the guest is handed starts again from
        // where the title put it, and the person's own is never moved under
        // their hand (#46). A move already waiting was measured from a place
        // the title has moved on from, so it goes, and what it carried is said
        // again from here ([`Steer::put`], #86).
        self.pending
            .retain(|input| !matches!(input, Input::Move { .. }));
        if let Some((x, y)) = self.steer.put((x, y), self.frame_size) {
            self.pending.push(Input::Move { x, y });
        }
    }

    fn closed(&self) -> bool {
        WinWindow::closed(self)
    }

    fn audio_underruns(&self) -> Option<u64> {
        self.audio
            .as_ref()
            .map(super::windows_audio::WinAudio::underruns)
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

/// The way back to a pointer, and to the run's wrap-up, from the console a
/// title is a program of (#45, #107).
///
/// A title is a console program, so that what it says reaches the terminal it
/// was started from, and the console it runs in can be closed, or sent Ctrl-C
/// or Ctrl-Break, under the window. Each ends the run as the close button
/// does, with its `end` line and digest. A hidden pointer is the thread's and
/// goes with the process, but a clip is the whole desktop's, so that is let go
/// of first, in case the run does not get to its own. A process killed
/// outright runs nothing, as a `SIGKILL` runs nothing on macOS.
mod way_back {
    use super::{ClipCursor, Rect, CLOSE_ASKED};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    static CLIP: AtomicUsize = AtomicUsize::new(0);
    static ARMED: AtomicBool = AtomicBool::new(false);

    /// The events a console sends, by wincon.h's numbers: Ctrl-C is 0,
    /// Ctrl-Break 1, and its window closing 2. Logging off and shutting down
    /// are not sent to a process with a window.
    const CTRL_CLOSE_EVENT: u32 = 2;

    type Handler = unsafe extern "system" fn(u32) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
    }

    /// Let go of the pointer, from wherever this is called. Safe when nothing
    /// was ever taken: the slot is empty and it does nothing.
    fn restore() {
        let clip = CLIP.load(Ordering::Relaxed);
        if clip == 0 {
            return;
        }
        // Safety: the address is user32's `ClipCursor`, stored by `arm`.
        unsafe {
            let clip: ClipCursor = std::mem::transmute(clip);
            clip(std::ptr::null::<Rect>());
        }
    }

    /// Windows calls this on a thread of its own. Answering true is the
    /// event handled, and false hands it on to the next handler, which ends
    /// the process.
    ///
    /// Ctrl-C and Ctrl-Break are handled: the run sees the close at its next
    /// burst and ends, and the process with it. A second press, with the run
    /// already ending, is handed on, so a run that never gets there can still
    /// be stopped. A console closing ends the process as soon as this returns,
    /// or five seconds after it was sent if it does not, so this never
    /// returns: the run writes its wrap-up meanwhile, and `main` returning
    /// ends the process, this thread with it.
    unsafe extern "system" fn on_console(event: u32) -> i32 {
        restore();
        if event > CTRL_CLOSE_EVENT {
            return 0;
        }
        let again = CLOSE_ASKED.swap(true, Ordering::Relaxed);
        if event == CTRL_CLOSE_EVENT {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        i32::from(!again)
    }

    /// Remember how to let go of the pointer.
    pub fn arm(clip: ClipCursor) {
        CLIP.store(clip as usize, Ordering::Relaxed);
    }

    /// Hear the console, from when a window opens. Once.
    pub fn listen() {
        if ARMED.swap(true, Ordering::Relaxed) {
            return;
        }
        // Safety: kernel32's own, with a handler of the shape it asks for.
        unsafe { SetConsoleCtrlHandler(Some(on_console), 1) };
    }
}
