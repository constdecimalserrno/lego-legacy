//! USER32: windows, messages and the pointer.
//!
//! The declared machine has one window, because every title on the roster has
//! one. What a person does reaches it through the host, is delivered at the
//! flip boundary, and is read back by the guest as messages.

use super::{cdecl, stdcall, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use std::collections::BTreeMap;

pub const SHIMS: &[Shim] = &[
    stdcall("USER32", "RegisterClassA", 1, register_class),
    stdcall("USER32", "CreateWindowExA", 12, create_window),
    stdcall("USER32", "DestroyWindow", 1, succeed),
    stdcall("USER32", "ShowWindow", 2, succeed),
    stdcall("USER32", "UpdateWindow", 1, succeed),
    stdcall("USER32", "SetFocus", 1, succeed),
    stdcall("USER32", "TranslateAcceleratorA", 3, translate_accelerator),
    stdcall("USER32", "LoadAcceleratorsA", 2, load_accelerators),
    stdcall("USER32", "FindWindowA", 2, find_window),
    stdcall("USER32", "GetActiveWindow", 0, active_window),
    stdcall("USER32", "WaitMessage", 0, wait_message),
    stdcall("USER32", "PeekMessageA", 5, peek_message),
    stdcall("USER32", "TranslateMessage", 1, translate_message),
    stdcall("USER32", "DispatchMessageA", 1, dispatch_message),
    stdcall("USER32", "DefWindowProcA", 4, succeed_zero),
    stdcall("USER32", "PostQuitMessage", 1, post_quit),
    stdcall("USER32", "GetSystemMetrics", 1, system_metrics),
    stdcall("USER32", "ShowCursor", 1, show_cursor),
    stdcall("USER32", "SetWindowsHookExA", 4, set_windows_hook),
    stdcall("USER32", "UnhookWindowsHookEx", 1, succeed),
    stdcall("USER32", "CallNextHookEx", 4, succeed_zero),
    stdcall("USER32", "SetCursor", 1, succeed_zero),
    stdcall("USER32", "LoadCursorA", 2, load_resource),
    stdcall("USER32", "LoadIconA", 2, load_resource),
    stdcall("USER32", "GetClientRect", 2, client_rect),
    stdcall("USER32", "GetWindowRect", 2, client_rect),
    // A window's frame is its border, caption and menu, each as wide as
    // `GetSystemMetrics` says, and the declared machine's are all zero: its
    // window is the screen. So the rectangle a client area needs is the
    // client area, whatever the style asks for (#114).
    stdcall("USER32", "AdjustWindowRect", 3, succeed),
    stdcall("USER32", "AdjustWindowRectEx", 4, succeed),
    stdcall("USER32", "SetWindowPos", 7, succeed),
    stdcall("USER32", "SetWindowLongA", 3, succeed_zero),
    stdcall("USER32", "GetWindowLongA", 2, window_long),
    stdcall("USER32", "MessageBoxA", 4, message_box),
    stdcall("USER32", "GetDC", 1, succeed_zero),
    stdcall("USER32", "ReleaseDC", 2, succeed),
    stdcall("USER32", "GetCursorPos", 1, cursor_pos),
    stdcall("USER32", "SetCursorPos", 2, set_cursor_pos),
    stdcall("USER32", "ClipCursor", 1, succeed),
    stdcall("USER32", "DialogBoxParamA", 5, dialog_box),
    stdcall("USER32", "EndDialog", 2, end_dialog),
    stdcall("USER32", "GetDlgItem", 2, dialog_item),
    stdcall("USER32", "SendMessageA", 4, send_message),
    stdcall("USER32", "GetDesktopWindow", 0, desktop_window),
    stdcall("USER32", "PostMessageA", 4, succeed),
    stdcall("USER32", "SetActiveWindow", 1, succeed_zero),
    stdcall("USER32", "SetTimer", 4, succeed),
    stdcall("USER32", "KillTimer", 2, succeed),
    // The title's window covers the declared screen from its corner, so
    // a point in it is the same point on the screen.
    stdcall("USER32", "ClientToScreen", 2, succeed),
    // Nothing on the declared machine is drawn from where a window is, and
    // the title's own windows cover the screen; the move always succeeds.
    stdcall("USER32", "MoveWindow", 6, succeed),
    stdcall("USER32", "EnableWindow", 2, enable_window),
    stdcall("USER32", "SetWindowTextA", 2, succeed),
    stdcall("USER32", "IsWindow", 1, succeed),
    stdcall("USER32", "SetCapture", 1, succeed_zero),
    stdcall("USER32", "ReleaseCapture", 0, succeed),
    cdecl("USER32", "wsprintfA", 2, wsprintf),
    stdcall("USER32", "SystemParametersInfoA", 4, system_parameters),
];

/// The one window the declared machine has.
///
/// A handle rather than a pointer, and a fixed one: a title that keeps it and
/// compares it later gets the same answer.
pub const WINDOW: u32 = 0x0001_0010;

fn wsprintf(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // wsprintfA(buffer, format, ...): what follows the format on the stack is
    // what it formats, and Windows writes at most 1024 bytes of it.
    let format = ctx.string(args[1]).map_err(Stop::Fault)?;
    let first = ctx.cpu.r[crate::cpu::ESP] + 12;
    let fault = std::cell::Cell::new(None);
    let taken = std::cell::Cell::new(0);
    let text = {
        let ctx = &*ctx;
        fn or_fault<T>(
            fault: &std::cell::Cell<Option<crate::guest::Fault>>,
            read: Result<T, crate::guest::Fault>,
            empty: T,
        ) -> T {
            read.unwrap_or_else(|f| {
                fault.set(Some(f));
                empty
            })
        }
        printf(
            &format,
            || {
                taken.set(taken.get() + 1);
                or_fault(&fault, ctx.mem.read_u32(first + 4 * (taken.get() - 1)), 0)
            },
            |at| or_fault(&fault, ctx.string(at), String::new()),
        )
    };
    if let Some(f) = fault.get() {
        return Err(Stop::Fault(f));
    }
    let text = text.map_err(|what| Stop::NotWritten {
        what: format!("wsprintfA(\"{format}\"): {what}"),
        caller: 0,
    })?;
    ctx.write_string(args[0], 1024, &text).map_err(Stop::Fault)
}

/// Format as `wsprintf` does: `-` and `0` flags, a width, a precision, `l`
/// and `h` sizes, and `d`, `i`, `u`, `x`, `X`, `c`, `s` and `%`. Each argument
/// is a word taken by `word`; a string is the text `text` reads at the word.
///
/// # Errors
/// A conversion with no body here, named.
fn printf(
    format: &str,
    mut word: impl FnMut() -> u32,
    text: impl Fn(u32) -> String,
) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let (mut left, mut zero) = (false, false);
        while let Some(&flag) = chars.peek().filter(|f| matches!(f, '-' | '0' | '#')) {
            if flag == '#' {
                return Err("the # flag".to_owned());
            }
            left |= flag == '-';
            zero |= flag == '0';
            chars.next();
        }
        let number = |chars: &mut std::iter::Peekable<std::str::Chars<'_>>| {
            let mut n = None;
            while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                n = Some(n.unwrap_or(0) * 10 + d as usize);
                chars.next();
            }
            n
        };
        let width = number(&mut chars).unwrap_or(0);
        let precision = if chars.peek() == Some(&'.') {
            chars.next();
            number(&mut chars)
        } else {
            None
        };
        // A long is a word already; a short is the word's low half.
        let mut short = false;
        while let Some(&size) = chars.peek().filter(|c| matches!(c, 'l' | 'h')) {
            short = size == 'h';
            chars.next();
        }
        let mut word = || if short { word() & 0xffff } else { word() };
        let (body, numeric) = match chars.next() {
            Some('d' | 'i') if short => ((word() as i16).to_string(), true),
            Some('d' | 'i') => ((word() as i32).to_string(), true),
            Some('u') => (word().to_string(), true),
            Some('x') => (format!("{:x}", word()), true),
            Some('X') => (format!("{:X}", word()), true),
            Some('c') => (char::from(word() as u8).to_string(), false),
            Some('s') => {
                let s = text(word());
                (
                    precision.map_or(s.clone(), |p| s.chars().take(p).collect()),
                    false,
                )
            }
            Some('%') => ("%".to_owned(), false),
            other => return Err(format!("the conversion {other:?}")),
        };
        let pad = width.saturating_sub(body.chars().count());
        if left {
            out.push_str(&body);
            out.extend(std::iter::repeat_n(' ', pad));
        } else if zero && numeric {
            let (sign, digits) = body.split_at(usize::from(body.starts_with('-')));
            out.push_str(sign);
            out.extend(std::iter::repeat_n('0', pad));
            out.push_str(digits);
        } else {
            out.extend(std::iter::repeat_n(' ', pad));
            out.push_str(&body);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod printf_tests {
    #[test]
    fn a_format_takes_its_arguments_in_order() {
        let mut args = [3u32, (-7i32) as u32, 0xbeef, 0x5678, 0x1_ffff].into_iter();
        let text = super::printf(
            "play cdaudio from %i, %03d|%-4X|%s%%|%hd",
            || args.next().unwrap_or(0),
            |at| format!("<{at:x}>"),
        );
        assert_eq!(
            text.as_deref(),
            Ok("play cdaudio from 3, -07|BEEF|<5678>%|-1")
        );
        assert!(super::printf("%f", || 0, |_| String::new()).is_err());
    }
}

fn succeed(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn succeed_zero(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

fn register_class(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // WNDCLASSA: the window procedure is its second field, and the machine
    // keeps it because a message it delivers has to reach the guest's own code.
    let procedure = ctx.mem.read_u32(args[0] + 4).map_err(Stop::Fault)?;
    // CS_DBLCLKS: a class that does not ask for double clicks is sent two
    // clicks instead, which is what Windows does.
    const CS_DBLCLKS: u32 = 0x0008;
    let style = ctx.mem.read_u32(args[0]).map_err(Stop::Fault)?;
    ctx.state.double_clicks = style & CS_DBLCLKS != 0;
    let name = {
        let at = ctx.mem.read_u32(args[0] + 36).map_err(Stop::Fault)?;
        ctx.string(at).map_err(Stop::Fault)?
    };
    ctx.state.window_procedure = procedure;
    ctx.log
        .line(&format!("window class {name} procedure {procedure:#010x}"));
    // A class atom: distinct, and not an address.
    Ok(0xc000)
}

fn create_window(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let title = ctx.string(args[2]).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "window {title:?} {}x{} at {},{}",
        args[6] as i32, args[7] as i32, args[4] as i32, args[5] as i32
    ));
    // Only the first window: Windows activates an application once, not once
    // a window, and a title that counts the message would hear one nobody
    // sent.
    let first = !ctx.state.window_made;
    ctx.state.window_made = true;
    ctx.state.window_style = args[3];
    ctx.state.window_instance = args[10];
    if first {
        let time = u32::try_from(ctx.clock_millis & 0xffff_ffff).unwrap_or(0);
        activate(ctx.state, time);
    }
    Ok(WINDOW)
}

/// A word of what the window system keeps about a window.
///
/// Its style, which LEGO Island's engine reads to see whether a person could
/// drag its window's edges before it takes the screen (#114), and its
/// instance, which the engine reads before it builds its first picture
/// (#118). Both as `CreateWindowExA` was given them.
// ponytail: the style as `CreateWindowExA` was given it. `SetWindowLongA`
// keeps nothing, and the bits Windows adds itself (`WS_VISIBLE` once shown,
// `WS_CLIPSIBLINGS` on a top-level window) are not added; answer them when a
// title reads one back.
fn window_long(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetWindowLongA(window, index).
    const GWL_STYLE: u32 = -16i32 as u32;
    const GWL_HINSTANCE: u32 = -6i32 as u32;
    match (args[0], args[1]) {
        (WINDOW, GWL_STYLE) if ctx.state.window_made => Ok(ctx.state.window_style),
        (WINDOW, GWL_HINSTANCE) if ctx.state.window_made => Ok(ctx.state.window_instance),
        (window, index) => Err(Stop::NotWritten {
            what: format!(
                "USER32.dll!GetWindowLongA: index {} of window {window:#x}",
                index as i32
            ),
            caller: 0,
        }),
    }
}

/// What Windows tells a title when its own application becomes the active one.
const WM_ACTIVATEAPP: u32 = 0x001c;

/// Tell the title that its application is the one in front.
///
/// The declared machine runs one title and nothing else, so the window it
/// makes is the active window from the moment it exists, and Windows says so.
/// A title that is never told believes it is in the background: Hot Wheels
/// draws a whole race into its own frame buffer and puts none of it on the
/// screen until it has heard this, which is a black race with its sound
/// playing (#38).
fn activate(state: &mut super::State, time: u32) {
    state.messages.push_back(super::Message {
        window: WINDOW,
        message: WM_ACTIVATEAPP,
        wparam: 1,
        lparam: 0,
        time,
        point: state.pointer,
    });
}

/// Whether a keystroke is one of a window's own accelerators.
///
/// The declared machine's titles have no accelerator table — nothing on this
/// roster loads one — so no message is ever translated and every keystroke
/// goes on to the title's own window procedure.
fn translate_accelerator(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

/// A module's table of shortcut keys, by name, looked for where Windows looks:
/// in the resources its own headers point to.
///
/// LEGO Island asks its own image for `AppAccel` (#118), and that image's
/// resources are its cursors, its icons and its version, with no table of
/// keys among them. So Windows answers none, and says the kind of resource was
/// not found. An image that does hold one stops the run: nothing reads one yet.
fn load_accelerators(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // LoadAcceleratorsA(module, name).
    const RT_ACCELERATOR: u32 = 9;
    const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
    let module = args[0];
    let read = |at: u32| ctx.mem.read_u32(at).map_err(Stop::Fault);
    let headers = module.wrapping_add(read(module + 0x3c)?);
    // The resources are the third of the optional header's data directories.
    let resources = read(headers + 24 + 96 + 2 * 8)?;
    if resources != 0 {
        let directory = module.wrapping_add(resources);
        let counts = read(directory + 12)?;
        let (named, numbered) = (counts & 0xffff, counts >> 16);
        for entry in named..named + numbered {
            if read(directory + 16 + 8 * entry)? == RT_ACCELERATOR {
                return Err(Stop::NotWritten {
                    what: "USER32.dll!LoadAcceleratorsA: the module holds a table of keys"
                        .to_owned(),
                    caller: 0,
                });
            }
        }
    }
    ctx.state.last_error = ERROR_RESOURCE_TYPE_NOT_FOUND;
    Ok(0)
}

/// A window belonging to somebody else, by class or by name.
///
/// A title asks this to find out whether a copy of itself is already running.
/// On this machine it is the only thing running, so there is never one.
fn find_window(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(0)
}

fn active_window(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(if ctx.state.window_made { WINDOW } else { 0 })
}

/// Where the mouse's own data format keeps what moved: the two axes, and the
/// first button. `c_dfDIMouse`, which is the format a title asks for by name.
const DIMOUSE_X: u32 = 0;
const DIMOUSE_Y: u32 = 4;
const DIMOUSE_BUTTON0: u32 = 12;
const DIMOUSE_BUTTON1: u32 = 13;

/// What a device says about a button or a key that is down.
const DOWN: u32 = 0x80;

const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_RBUTTONDBLCLK: u32 = 0x0206;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const MK_LBUTTON: u32 = 0x0001;
const MK_RBUTTON: u32 = 0x0002;

/// How long after a click a second one beside it is a double click, and how
/// far beside it is still beside it.
///
/// Declared: Windows' own defaults, `GetDoubleClickTime` and
/// `SM_CXDOUBLECLK`, neither of which this machine lets a person change.
const DOUBLE_CLICK_MILLIS: u32 = 500;
const DOUBLE_CLICK_SLOP: i32 = 4;

/// The virtual key a `DIK_` number stands for, on the keyboard the declared
/// machine has.
///
/// DirectInput numbers a key by where it sits — a PC's set-1 scan codes, with
/// the ones after the grey prefix carrying `0x80`. A title reading its
/// keyboard from its window sees the virtual key instead, and this is the map
/// between them. Every number a host's key table can produce, the Mac's or
/// X's ([`crate::host::x11::direct_input_key`]), is here; anything else
/// reaches the window as nothing (#59).
fn virtual_key(code: u32) -> Option<u32> {
    Some(match code {
        0x01 => 0x1b,                      // Escape
        0x02..=0x0a => 0x30 + code - 0x01, // 1 to 9
        0x0b => 0x30,                      // 0
        0x0c => 0xbd,                      // Minus
        0x0d => 0xbb,                      // Equals
        0x0e => 0x08,                      // Backspace
        0x0f => 0x09,                      // Tab
        0x10 => 0x51,                      // Q
        0x11 => 0x57,                      // W
        0x12 => 0x45,                      // E
        0x13 => 0x52,                      // R
        0x14 => 0x54,                      // T
        0x15 => 0x59,                      // Y
        0x16 => 0x55,                      // U
        0x17 => 0x49,                      // I
        0x18 => 0x4f,                      // O
        0x19 => 0x50,                      // P
        0x1a => 0xdb,                      // Left bracket
        0x1b => 0xdd,                      // Right bracket
        0x1c | 0x9c => 0x0d,               // Return, and the keypad's
        0x1d | 0x9d => 0x11,               // Control, either of them
        0x1e => 0x41,                      // A
        0x1f => 0x53,                      // S
        0x20 => 0x44,                      // D
        0x21 => 0x46,                      // F
        0x22 => 0x47,                      // G
        0x23 => 0x48,                      // H
        0x24 => 0x4a,                      // J
        0x25 => 0x4b,                      // K
        0x26 => 0x4c,                      // L
        0x27 => 0xba,                      // Semicolon
        0x28 => 0xde,                      // Apostrophe
        0x29 => 0xc0,                      // Grave
        0x2a | 0x36 => 0x10,               // Shift, either of them
        0x2b => 0xdc,                      // Backslash
        0x2c => 0x5a,                      // Z
        0x2d => 0x58,                      // X
        0x2e => 0x43,                      // C
        0x2f => 0x56,                      // V
        0x30 => 0x42,                      // B
        0x31 => 0x4e,                      // N
        0x32 => 0x4d,                      // M
        0x33 => 0xbc,                      // Comma
        0x34 => 0xbe,                      // Period
        0x35 | 0xb5 => 0xbf,               // Slash, and the keypad's divide
        0x37 => 0x6a,                      // Keypad multiply
        0x38 | 0xb8 => 0x12,               // Alt, either of them
        0x39 => 0x20,                      // Space
        0x3a => 0x14,                      // Caps lock
        0x3b..=0x44 => 0x70 + code - 0x3b, // F1 to F10
        0x45 => 0x90,                      // Num lock
        0x46 => 0x91,                      // Scroll lock
        0x47 => 0x67,                      // Keypad 7
        0x48 => 0x68,                      // Keypad 8
        0x49 => 0x69,                      // Keypad 9
        0x4a => 0x6d,                      // Keypad minus
        0x4b => 0x64,                      // Keypad 4
        0x4c => 0x65,                      // Keypad 5
        0x4d => 0x66,                      // Keypad 6
        0x4e => 0x6b,                      // Keypad plus
        0x4f => 0x61,                      // Keypad 1
        0x50 => 0x62,                      // Keypad 2
        0x51 => 0x63,                      // Keypad 3
        0x52 => 0x60,                      // Keypad 0
        0x53 => 0x6e,                      // Keypad decimal
        0x56 => 0xe2,                      // The extra key on an ISO keyboard
        0x57 => 0x7a,                      // F11
        0x58 => 0x7b,                      // F12
        0x64..=0x66 => 0x7c + code - 0x64, // F13 to F15
        0x8d => 0x92,                      // Keypad equals
        0xc7 => 0x24,                      // Home
        0xc8 => 0x26,                      // Up
        0xc9 => 0x21,                      // Page up
        0xcb => 0x25,                      // Left
        0xcd => 0x27,                      // Right
        0xcf => 0x23,                      // End
        0xd0 => 0x28,                      // Down
        0xd1 => 0x22,                      // Page down
        0xd2 => 0x2d,                      // Insert
        0xd3 => 0x2e,                      // Delete
        _ => return None,
    })
}

/// Where a button's state lives.
fn button_of(state: &mut super::State, button: crate::host::Button) -> &mut bool {
    match button {
        crate::host::Button::Left => &mut state.mouse_down,
        crate::host::Button::Right => &mut state.right_down,
    }
}

/// When a button last went down and where. Each button counts its own clicks.
fn last_click_of(state: &mut super::State, button: crate::host::Button) -> &mut (u32, (i32, i32)) {
    match button {
        crate::host::Button::Left => &mut state.last_click,
        crate::host::Button::Right => &mut state.last_right_click,
    }
}

/// Where the mouse's own data format keeps a button.
fn lane(button: crate::host::Button) -> u32 {
    match button {
        crate::host::Button::Left => DIMOUSE_BUTTON0,
        crate::host::Button::Right => DIMOUSE_BUTTON1,
    }
}

/// The buttons a mouse message carries as held, in `wParam`.
fn held_buttons(state: &super::State) -> u32 {
    u32::from(state.mouse_down) * MK_LBUTTON + u32::from(state.right_down) * MK_RBUTTON
}

pub fn deliver(state: &mut super::State, event: crate::host::Input, time: u32) {
    use crate::host::{Button, Input};
    match event {
        Input::Move { x, y } => {
            let (dx, dy) = (x - state.last_move.0, y - state.last_move.1);
            state.mouse_delta.0 += dx;
            state.mouse_delta.1 += dy;
            // The buffered lane carries each axis that moved, where the
            // mouse's data format keeps it.
            if dx != 0 {
                state.buffer_input(false, DIMOUSE_X, dx as u32);
            }
            if dy != 0 {
                state.buffer_input(false, DIMOUSE_Y, dy as u32);
            }
            state.last_move = (x, y);
            state.pointer = (x, y);
            post(state, WM_MOUSEMOVE, held_buttons(state), time);
        }
        Input::Down(button) => {
            *button_of(state, button) = true;
            state.buffer_input(false, lane(button), DOWN);
            // The second of two clicks near each other and soon after each
            // other is a double click, which is a message of its own. Each
            // button counts its own: a right click after a left one is not
            // the second of anything.
            let (when, (was_x, was_y)) = *last_click_of(state, button);
            let (x, y) = state.pointer;
            let quick = time.wrapping_sub(when) < DOUBLE_CLICK_MILLIS;
            let near =
                (x - was_x).abs() <= DOUBLE_CLICK_SLOP && (y - was_y).abs() <= DOUBLE_CLICK_SLOP;
            let again = state.double_clicks && when != 0 && quick && near;
            let message = match (button, again) {
                (Button::Left, false) => WM_LBUTTONDOWN,
                (Button::Left, true) => WM_LBUTTONDBLCLK,
                (Button::Right, false) => WM_RBUTTONDOWN,
                (Button::Right, true) => WM_RBUTTONDBLCLK,
            };
            post(state, message, held_buttons(state), time);
            // A double click does not start a third: the next click is a
            // first one again, as Windows counts them.
            *last_click_of(state, button) = if again { (0, (x, y)) } else { (time, (x, y)) };
        }
        Input::Up(button) => {
            *button_of(state, button) = false;
            state.buffer_input(false, lane(button), 0);
            let message = match button {
                Button::Left => WM_LBUTTONUP,
                Button::Right => WM_RBUTTONUP,
            };
            post(state, message, held_buttons(state), time);
        }
        Input::Key { code, down } => {
            // Keys reach a title through DirectInput's keyboard, numbered as
            // that device numbers them, and a key's own number is where it
            // sits in that device's data format.
            state.keys_down.retain(|k| *k != code);
            if down {
                state.keys_down.push(code);
            }
            state.buffer_input(true, code, if down { DOWN } else { 0 });
            // And through the window, for a title that reads its keyboard
            // from its message queue rather than from DirectInput: Alpha
            // Team's `DDKeyboard::processKey` is handed a virtual key and the
            // parameter Windows sent with it (#59).
            if let Some(key) = virtual_key(code) {
                let repeat_and_scan = 1 | ((code & 0x7f) << 16) | ((code & 0x80) << 17);
                let (message, transition) = if down {
                    (WM_KEYDOWN, 0)
                } else {
                    (WM_KEYUP, 0xc000_0000)
                };
                queue(state, message, key, repeat_and_scan | transition, time);
            }
        }
    }
}

/// Queue a message for the title's window with a parameter of its own, rather
/// than the pointer's position.
fn queue(state: &mut super::State, message: u32, wparam: u32, lparam: u32, time: u32) {
    if !state.window_made {
        return;
    }
    state.messages.push_back(super::Message {
        window: WINDOW,
        message,
        wparam,
        lparam,
        time,
        point: state.pointer,
    });
}

/// Queue a mouse message for the title's window, where the pointer is.
///
/// Before the title has a window there is nothing to send one to, as there is
/// on Windows. A move that finds the last message still waiting a move takes
/// its place, as Windows keeps only the latest.
fn post(state: &mut super::State, message: u32, wparam: u32, time: u32) {
    if !state.window_made {
        return;
    }
    let (x, y) = state.pointer;
    let queued = super::Message {
        window: WINDOW,
        message,
        wparam,
        lparam: (u32::from(y as u16) << 16) | u32::from(x as u16),
        time,
        point: state.pointer,
    };
    match state.messages.back_mut() {
        Some(last) if message == WM_MOUSEMOVE && last.message == WM_MOUSEMOVE => *last = queued,
        _ => state.messages.push_back(queued),
    }
}

/// The first waiting message a filter takes: a window, and a range of
/// messages where zero to zero is all of them.
fn waiting(ctx: &Ctx<'_>, window: u32, first: u32, last: u32) -> Option<usize> {
    ctx.state.messages.iter().position(|m| {
        (window == 0 || m.window == window)
            && ((first == 0 && last == 0) || (first..=last).contains(&m.message))
    })
}

/// Write a `MSG`: window, message, the two parameters, the time, the point.
fn write_message(ctx: &mut Ctx<'_>, at: u32, m: &super::Message) -> Result<(), Stop> {
    for (offset, value) in [
        (0, m.window),
        (4, m.message),
        (8, m.wparam),
        (12, m.lparam),
        (16, m.time),
        (20, m.point.0 as u32),
        (24, m.point.1 as u32),
    ] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(())
}

/// Find the first message the filter `(msg, window, first, last)` takes, write
/// it to `msg` unless that is null, and take it off the queue if `remove`.
/// Whether there was one.
fn take_message(ctx: &mut Ctx<'_>, filter: &[u32], remove: bool) -> Result<bool, Stop> {
    let msg = filter[0];
    let Some(index) = waiting(ctx, filter[1], filter[2], filter[3]) else {
        return Ok(false);
    };
    let message = ctx.state.messages[index];
    if remove {
        ctx.state.messages.remove(index);
    }
    if msg != 0 {
        write_message(ctx, msg, &message)?;
    }
    Ok(true)
}

fn peek_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // PeekMessageA(msg, window, first, last, remove). What a person did is
    // posted at the flip boundary; a title that finds nothing here gets on
    // with drawing.
    const PM_REMOVE: u32 = 0x0001;
    Ok(u32::from(take_message(
        ctx,
        args,
        args[4] & PM_REMOVE != 0,
    )?))
}

fn translate_message(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Key messages are posted (#59), and no character message is ever made
    // from one.
    Ok(0)
}

fn dispatch_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DispatchMessageA(msg): the title's own window procedure is called with
    // it. What the procedure answers is not handed back; no title on the
    // roster has been measured reading it.
    let at = args[0];
    let read = |o: u32| ctx.mem.read_u32(at + o).map_err(Stop::Fault);
    let (window, message, wparam, lparam) = (read(0)?, read(4)?, read(8)?, read(12)?);
    let procedure = ctx.state.window_procedure;
    if window != WINDOW || procedure == 0 {
        return Err(Stop::NotWritten {
            what: format!(
                "USER32.dll!DispatchMessageA: message {message:#06x} to window {window:#x}, \
                 which has no procedure here"
            ),
            caller: 0,
        });
    }
    ctx.log
        .line(&format!("window procedure given message {message:#06x}"));
    ctx.call_guest = Some((procedure, vec![window, message, wparam, lparam]));
    Ok(0)
}

fn show_cursor(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A count, not a flag, as Windows keeps it: the pointer is drawn while it
    // is zero or more. A title that draws its own pointer takes it below zero,
    // and the host stops drawing the one a person is really moving, so that
    // there is one pointer on the screen rather than two.
    ctx.state.cursor_shown += if args[0] == 0 { -1 } else { 1 };
    let count = ctx.state.cursor_shown;
    ctx.log
        .at(crate::cli::Trace::Shims, || format!("cursor count {count}"));
    Ok(count as u32)
}

fn set_windows_hook(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // SetWindowsHookExA(which, procedure, module, thread).
    //
    // A hook watches what the window system does on the way to a window. On
    // the declared machine the window system is this machine: one window, made
    // once, in front from the moment it exists, and every message delivered
    // straight to the procedure the title registered. There is nothing on the
    // way for a hook to sit in, so the handle answered here is one that is
    // never called. A3D asks for a `WH_CBT` hook of its own thread to hear
    // about windows being activated, which on this machine happens once,
    // before A3D is loaded (#60).
    let (which, procedure) = (args[0], args[1]);
    ctx.log.line(&format!(
        "hook {which} at {procedure:#010x}, which nothing on this machine calls"
    ));
    Ok(ctx.state.give(Object::Hook(which)))
}

fn set_cursor_pos(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The title puts the pointer somewhere, and that is where it is until a
    // person moves it. The host is told at the flip, so that the person's own
    // pointer is where the title measures their next move from (#59).
    let (x, y) = (args[0] as i32, args[1] as i32);
    ctx.state.pointer = (x, y);
    ctx.state.pointer_put = Some((x, y));
    Ok(1)
}

/// The pointer starts again from where the title put it, for the machine as
/// well as for the host.
///
/// Only the two together are right. The host, once it is told, reports the
/// person's next move as so far past **where the title put the pointer**; the
/// machine reads a move as so far past **where it last was told the pointer
/// was**. A title that puts the pointer back every frame and steers by how far
/// the mouse went since — which is what Alpha Team does, and Hot Wheels — makes
/// those two different numbers, and the difference is the whole of the
/// person's movement: at a steady speed every report is the same number, and
/// the machine reads each one as no movement at all (#40, #64).
pub fn set_pointer_back(state: &mut super::State, (x, y): (i32, i32)) {
    state.pointer = (x, y);
    state.last_move = (x, y);
}

fn wait_message(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Waiting for a message would wait forever: this machine delivers input at
    // a flip, and a title that is inside a call is not flipping. It comes back
    // at once and the title looks at its queue, which is what it would have
    // done anyway.
    Ok(1)
}

fn post_quit(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.log
        .line(&format!("guest asked to quit with {}", args[0]));
    Ok(0)
}

fn system_metrics(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's screen in the mode it is in, as DirectDraw's
    // `GetDisplayMode` and GDI's `GetDeviceCaps` say, and nothing measured
    // from the host's.
    let (width, height, _) = ctx.state.display_mode;
    Ok(match args[0] {
        0 => width,
        1 => height,
        // Everything else is a border or a caption this machine does not have.
        _ => 0,
    })
}

/// A system-wide setting, set or read.
///
/// Only what a title has been seen to ask is answered; any other action is a
/// wall that names it, rather than a success the title would believe.
fn system_parameters(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // SystemParametersInfoA(action, uiParam, pvParam, fWinIni).
    const SPI_SETMOUSETRAILS: u32 = 0x005d;
    match (args[0], args[1]) {
        // Mouse trails off, which Windows spells as a trail of 0 or of 1. This
        // machine draws no trail, so there is nothing to turn off. LEGO Island
        // turns them off before it makes its window (#114).
        (SPI_SETMOUSETRAILS, 0 | 1) => Ok(1),
        (action, param) => Err(Stop::NotWritten {
            what: format!(
                "USER32.dll!SystemParametersInfoA: action {action:#06x}, uiParam {param}"
            ),
            caller: 0,
        }),
    }
}

fn load_resource(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A cursor or an icon: a handle the guest only ever passes back to us.
    Ok(0x0001_0020)
}

fn client_rect(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The one window covers the whole screen, as `GetRegionData` says.
    let (width, height, _) = ctx.state.display_mode;
    let at = args[1];
    for (offset, value) in [(0, 0), (4, 0), (8, width), (12, height)] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn cursor_pos(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (x, y) = ctx.state.pointer;
    ctx.mem.write_u32(args[0], x as u32).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(args[0] + 4, y as u32)
        .map_err(Stop::Fault)?;
    Ok(1)
}

fn message_box(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Whatever a title wanted to tell a person goes in the run log, which is
    // where a person running headless will actually see it.
    let text = ctx.string(args[1]).map_err(Stop::Fault)?;
    let caption = ctx.string(args[2]).map_err(Stop::Fault)?;
    ctx.log.line(&format!("message box {caption:?}: {text:?}"));
    // IDOK.
    Ok(1)
}

/// A modal dialog a title has up: its procedure, and what its controls hold.
///
/// Nothing is drawn. The title's own procedure is run, so it fills its
/// controls and makes its own default choices, and then the default button is
/// pressed, which is what a person pressing Enter does (#9).
pub struct Dialog {
    pub window: u32,
    pub procedure: u32,
    /// How many of the procedure's messages have come back.
    pub turns: u32,
    /// Whether the machine has parked the call that brought the dialog up.
    pub parked: bool,
    /// What the procedure ended the dialog with, once it has.
    pub ended: Option<u32>,
    lists: BTreeMap<u32, List>,
    checks: BTreeMap<u32, u32>,
    /// Controls the procedure has disabled, which nobody can press.
    pub disabled: std::collections::BTreeSet<u32>,
}

/// A list box's strings and which one is selected.
#[derive(Default)]
struct List {
    items: Vec<String>,
    selected: Option<usize>,
}

/// The dialog's own window handle.
pub const DIALOG: u32 = 0x0001_0020;

/// The handle a dialog's control with this identifier has.
#[must_use]
pub const fn control(id: u32) -> u32 {
    0x0002_0000 | (id & 0xffff)
}

/// The identifier of a dialog control a handle names, if it names one.
const fn control_id(window: u32) -> Option<u32> {
    if window >> 16 == 2 {
        Some(window & 0xffff)
    } else {
        None
    }
}

fn dialog_box(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DialogBoxParamA(instance, template, parent, procedure, parameter).
    if ctx.state.dialog.is_some() {
        return Err(Stop::NotWritten {
            what: "USER32.dll!DialogBoxParamA: a dialog on top of a dialog".to_owned(),
            caller: 0,
        });
    }
    ctx.log.line(&format!(
        "dialog {:#x} up, procedure {:#010x}",
        args[1], args[3]
    ));
    ctx.state.dialog = Some(Dialog {
        window: DIALOG,
        procedure: args[3],
        turns: 0,
        parked: false,
        ended: None,
        lists: BTreeMap::new(),
        checks: BTreeMap::new(),
        disabled: std::collections::BTreeSet::new(),
    });
    // WM_INITDIALOG, with the parameter the title passed.
    ctx.call_guest = Some((args[3], vec![DIALOG, 0x0110, 0, args[4]]));
    Ok(0)
}

fn enable_window(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // Answers whether the window was disabled before.
    let (Some(dialog), Some(id)) = (ctx.state.dialog.as_mut(), control_id(args[0])) else {
        return Ok(0);
    };
    let was = if args[1] == 0 {
        !dialog.disabled.insert(id)
    } else {
        dialog.disabled.remove(&id)
    };
    Ok(u32::from(was))
}

fn end_dialog(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    match ctx.state.dialog.as_mut() {
        Some(dialog) if args[0] == dialog.window => {
            dialog.ended = Some(args[1]);
            Ok(1)
        }
        _ => Ok(0),
    }
}

fn dialog_item(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    Ok(match &ctx.state.dialog {
        Some(dialog) if args[0] == dialog.window => control(args[1]),
        _ => 0,
    })
}

/// A message sent to a window and answered before the call returns.
///
/// A dialog's controls are answered here, as the control classes would: list
/// boxes keep their strings and selection, buttons their check. The title's
/// own window is not sent messages this way on the boot path; asking is a wall
/// rather than a zero it would believe.
fn send_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (window, message, wparam, lparam) = (args[0], args[1], args[2], args[3]);
    let text = if matches!(message, 0x000c | 0x0180) && lparam != 0 {
        ctx.string(lparam).map_err(Stop::Fault)?
    } else {
        String::new()
    };
    let (Some(dialog), Some(id)) = (ctx.state.dialog.as_mut(), control_id(window)) else {
        return Err(Stop::NotWritten {
            what: format!("USER32.dll!SendMessageA: message {message:#06x} to window {window:#x}"),
            caller: 0,
        });
    };
    const LB_ERR: u32 = 0xffff_ffff;
    let list = dialog.lists.entry(id).or_default();
    let answer = match message {
        // WM_SETTEXT.
        0x000c => 1,
        // BM_SETCHECK, BM_GETCHECK.
        0x00f1 => {
            dialog.checks.insert(id, wparam);
            0
        }
        0x00f0 => dialog.checks.get(&id).copied().unwrap_or(0),
        // LB_ADDSTRING: appended, because no list on the boot path is sorted.
        0x0180 => {
            list.items.push(text);
            u32::try_from(list.items.len() - 1).unwrap_or(LB_ERR)
        }
        // LB_DELETESTRING: what is left, or an error when there was nothing.
        0x0182 => {
            let index = wparam as usize;
            if index < list.items.len() {
                list.items.remove(index);
                list.selected = list.selected.filter(|s| *s < list.items.len());
                u32::try_from(list.items.len()).unwrap_or(LB_ERR)
            } else {
                LB_ERR
            }
        }
        // LB_SETCURSEL.
        0x0186 => {
            if (wparam as usize) < list.items.len() {
                list.selected = Some(wparam as usize);
                wparam
            } else {
                list.selected = None;
                LB_ERR
            }
        }
        // LB_GETCURSEL.
        0x0188 => list
            .selected
            .and_then(|s| u32::try_from(s).ok())
            .unwrap_or(LB_ERR),
        // LB_GETTEXT.
        0x0189 => match list.items.get(wparam as usize).cloned() {
            Some(item) => {
                let len = u32::try_from(item.len()).unwrap_or(0);
                ctx.write_string(lparam, len + 1, &item)
                    .map_err(Stop::Fault)?;
                len
            }
            None => LB_ERR,
        },
        // LB_GETCOUNT.
        0x018b => u32::try_from(list.items.len()).unwrap_or(LB_ERR),
        _ => {
            return Err(Stop::NotWritten {
                what: format!(
                    "USER32.dll!SendMessageA: message {message:#06x} to dialog control {id}"
                ),
                caller: 0,
            })
        }
    };
    Ok(answer)
}

fn desktop_window(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's screen, as a window handle. Distinct from the
    // title's own so that a guest comparing them can tell them apart.
    Ok(0x0001_0001)
}

#[cfg(test)]
mod tests {
    use super::WM_ACTIVATEAPP;
    use super::{
        activate, deliver, virtual_key, DIMOUSE_BUTTON0, DIMOUSE_X, DIMOUSE_Y, DOWN, WINDOW,
        WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    };
    use crate::host::{Button, Input};
    use crate::win32::State;

    #[test]
    fn a_window_made_here_is_the_active_application() {
        let mut state = State::new();
        state.window_made = true;
        activate(&mut state, 7);
        let first = state.messages[0];
        assert_eq!((first.window, first.message), (WINDOW, WM_ACTIVATEAPP));
        assert_eq!(first.wparam, 1, "active, not deactivated");
        assert_eq!(first.time, 7);
    }

    /// A title that reads its keyboard from its window, as Alpha Team's
    /// `DDKeyboard` does, is sent the virtual key and the parameter Windows
    /// sends with it: the repeat count, the key's own number where the
    /// keyboard reported it, and the bits that say it is being let go (#59).
    #[test]
    fn a_key_reaches_the_window_as_windows_would_send_it() {
        let mut state = State::new();
        state.window_made = true;
        // Space, and then Left, which the keyboard reports after its prefix.
        deliver(
            &mut state,
            Input::Key {
                code: 0x39,
                down: true,
            },
            3,
        );
        deliver(
            &mut state,
            Input::Key {
                code: 0xcb,
                down: false,
            },
            4,
        );

        let sent: Vec<(u32, u32, u32)> = state
            .messages
            .iter()
            .map(|m| (m.message, m.wparam, m.lparam))
            .collect();
        assert_eq!(
            sent,
            [
                (WM_KEYDOWN, 0x20, 0x0039_0001),
                (WM_KEYUP, 0x25, 0xc14b_0001),
            ]
        );
        // And DirectInput's own lane still has both, by the key's number.
        assert_eq!(
            state.keyboard_data.iter().copied().collect::<Vec<_>>(),
            [(0x39, DOWN), (0xcb, 0)]
        );
    }

    /// Every key a host can produce has a virtual key to be sent as, and
    /// nothing else does (#59): every Mac key, every key message Windows can
    /// send (#107), and every X keycode (#108).
    #[test]
    fn every_key_the_host_can_send_has_a_virtual_key() {
        #[cfg(target_os = "macos")]
        for code in 0..=0xffu64 {
            if let Some(key) = crate::host::mac::direct_input_key(code) {
                assert!(
                    virtual_key(key).is_some(),
                    "key {key:#04x}, which mac key {code:#04x} is, has no virtual key"
                );
            }
        }
        // Except the Windows keys, which reach DirectInput and not the
        // window, as they do from macOS, where they are Command and come as
        // a modifier (`direct_input_modifier`), which this does not ask.
        for scan in 0..=0x1ffu32 {
            let lparam = ((scan & 0xff) << 16) | ((scan >> 8) << 24);
            let key = crate::host::windows_input::direct_input_key(lparam);
            if let Some(key) = key.filter(|k| !matches!(k, 0xdb | 0xdc)) {
                assert!(
                    virtual_key(key).is_some(),
                    "key {key:#04x}, which Windows scan code {scan:#05x} is, has no virtual key"
                );
            }
        }
        // X's table is reached on every host (#108). Its modifiers are keys
        // like any other, and the Windows keys among them reach DirectInput
        // alone, as the Mac's Command keys do.
        for code in 0..=0xff {
            let key = crate::host::x11::direct_input_key(code);
            if let Some(key) = key.filter(|key| !matches!(key, 0xdb | 0xdc)) {
                assert!(
                    virtual_key(key).is_some(),
                    "key {key:#04x}, which X keycode {code} is, has no virtual key"
                );
            }
        }
        assert_eq!(virtual_key(0x00), None, "and nothing is not a key");
    }

    /// A second click beside the first and soon after it is one double click,
    /// for a window whose class asked for them (#59).
    #[test]
    fn two_quick_clicks_in_the_same_place_are_a_double_click() {
        let click = |state: &mut State, at: u32| {
            deliver(state, Input::Down(Button::Left), at);
            deliver(state, Input::Up(Button::Left), at);
        };
        let messages =
            |state: &State| -> Vec<u32> { state.messages.iter().map(|m| m.message).collect() };

        let mut wants = State::new();
        wants.window_made = true;
        wants.double_clicks = true;
        click(&mut wants, 1_000);
        click(&mut wants, 1_100);
        // A third click is a first one again, not a triple.
        click(&mut wants, 1_200);
        assert_eq!(
            messages(&wants)[0..5],
            [
                WM_LBUTTONDOWN,
                super::WM_LBUTTONUP,
                WM_LBUTTONDBLCLK,
                super::WM_LBUTTONUP,
                WM_LBUTTONDOWN,
            ]
        );

        // Too slow, and it is two clicks.
        let mut slow = State::new();
        slow.window_made = true;
        slow.double_clicks = true;
        click(&mut slow, 1_000);
        click(&mut slow, 1_600);
        assert_eq!(messages(&slow)[2], WM_LBUTTONDOWN);

        // Too far, and it is two clicks.
        let mut apart = State::new();
        apart.window_made = true;
        apart.double_clicks = true;
        click(&mut apart, 1_000);
        deliver(&mut apart, Input::Move { x: 400, y: 240 }, 1_050);
        click(&mut apart, 1_100);
        assert_eq!(
            *messages(&apart).last().expect("a click was sent"),
            super::WM_LBUTTONUP
        );
        assert!(
            !messages(&apart).contains(&WM_LBUTTONDBLCLK),
            "a click somewhere else is not a double click"
        );

        // A class that did not ask for double clicks is sent two clicks.
        let mut plain = State::new();
        plain.window_made = true;
        click(&mut plain, 1_000);
        click(&mut plain, 1_100);
        assert!(!messages(&plain).contains(&WM_LBUTTONDBLCLK));

        // Each button counts its own clicks: a right click after a left one
        // is a first click, and it is the right button's messages that go
        // out. Alpha Team registers a double-click route for each (#61).
        let mut both = State::new();
        both.window_made = true;
        both.double_clicks = true;
        click(&mut both, 1_000);
        deliver(&mut both, Input::Down(Button::Right), 1_050);
        deliver(&mut both, Input::Up(Button::Right), 1_050);
        deliver(&mut both, Input::Down(Button::Right), 1_100);
        assert_eq!(
            messages(&both),
            [
                WM_LBUTTONDOWN,
                super::WM_LBUTTONUP,
                super::WM_RBUTTONDOWN,
                super::WM_RBUTTONUP,
                super::WM_RBUTTONDBLCLK,
            ]
        );
        // And what a message says is held is what is held: the right button
        // is down, the left one is not.
        assert_eq!(
            both.messages.back().expect("a click was sent").wparam,
            super::MK_RBUTTON
        );
    }

    #[test]
    fn the_mouses_first_move_is_measured_from_the_middle_of_the_screen() {
        // Where Windows leaves the pointer after a mode is set, and so where
        // a title that draws its own pointer starts it (#48).
        let mut state = State::new();
        deliver(&mut state, Input::Move { x: 320, y: 240 }, 0);
        assert!(
            state.mouse_data.is_empty(),
            "to the middle is no move at all"
        );
        deliver(&mut state, Input::Move { x: 325, y: 233 }, 0);
        let moved: Vec<(u32, u32)> = state.mouse_data.iter().copied().collect();
        assert_eq!(moved, [(DIMOUSE_X, 5), (DIMOUSE_Y, (-7i32) as u32)]);

        // A new mode puts the pointer in the middle of that one.
        state.display_mode = (800, 600, 16);
        state.pointer = (400, 300);
        state.last_move = (400, 300);
        state.mouse_data.clear();
        deliver(&mut state, Input::Move { x: 401, y: 300 }, 0);
        let moved: Vec<(u32, u32)> = state.mouse_data.iter().copied().collect();
        assert_eq!(moved, [(DIMOUSE_X, 1)]);
    }

    /// A title that puts the pointer back every frame and steers by how far
    /// the mouse went since (#40, #64).
    ///
    /// The host restarts its own reporting at wherever the title put the
    /// pointer, so the machine has to restart with it, or a hand moving at a
    /// steady speed reads as a hand standing still.
    #[test]
    fn a_hand_moving_the_same_amount_every_frame_keeps_moving_the_pointer() {
        let mut state = State::new();
        state.window_made = true;
        state.pointer = (320, 240);
        state.last_move = (320, 240);
        let moved = |state: &mut State| -> i32 {
            // The title puts the pointer back in the middle, and the machine
            // tells the host, which restarts there.
            super::set_pointer_back(state, (320, 240));
            // Then the hand goes three pixels right, which the host reports
            // as three past where it was put.
            deliver(state, Input::Move { x: 323, y: 240 }, 0);
            let dx = state.mouse_delta.0;
            state.mouse_delta = (0, 0);
            dx
        };
        assert_eq!(moved(&mut state), 3, "the first frame");
        assert_eq!(moved(&mut state), 3, "and every frame after it");
        assert_eq!(moved(&mut state), 3, "for as long as the hand moves");
    }

    #[test]
    fn a_persons_mouse_reaches_the_window_as_windows_would_send_it() {
        let mut state = State::new();
        // No window yet, so nowhere to send anything.
        deliver(&mut state, Input::Move { x: 5, y: 6 }, 1);
        assert!(state.messages.is_empty());

        state.window_made = true;
        deliver(&mut state, Input::Move { x: 10, y: 20 }, 2);
        deliver(&mut state, Input::Move { x: 320, y: 222 }, 3);
        // Moves still waiting are one move, to where the pointer ended up.
        assert_eq!(state.messages.len(), 1);
        let moved = state.messages[0];
        assert_eq!((moved.window, moved.message), (WINDOW, 0x0200));
        assert_eq!(moved.lparam, (222 << 16) | 320);
        assert_eq!((moved.time, moved.point), (3, (320, 222)));

        deliver(&mut state, Input::Down(Button::Left), 4);
        deliver(&mut state, Input::Move { x: 321, y: 222 }, 5);
        deliver(&mut state, Input::Up(Button::Left), 6);
        let kinds: Vec<(u32, u32)> = state
            .messages
            .iter()
            .map(|m| (m.message, m.wparam))
            .collect();
        assert_eq!(
            kinds,
            [(0x0200, 0), (0x0201, 1), (0x0200, 1), (0x0202, 0)],
            "a move with the button held says so"
        );
    }

    #[test]
    fn directinput_reads_how_far_the_mouse_moved_since_it_last_looked() {
        let mut state = State::new();
        // Ten right and twenty down from the middle, where the mouse starts.
        deliver(&mut state, Input::Move { x: 330, y: 260 }, 0);
        deliver(&mut state, Input::Down(Button::Left), 0);
        let first = state.take_mouse_state();
        assert_eq!(first[..8], [10, 0, 0, 0, 20, 0, 0, 0]);
        assert_eq!(first[12], 0x80, "a button held reads as its top bit");

        // The title putting the pointer somewhere is not the mouse moving.
        state.pointer = (300, 300);
        deliver(&mut state, Input::Move { x: 332, y: 259 }, 0);
        let second = state.take_mouse_state();
        assert_eq!(second[..8], [2, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(state.take_mouse_state()[..8], [0; 8], "looking took it");
    }

    #[test]
    fn directinput_buffers_what_a_person_did_for_a_title_that_reads_a_queue() {
        let mut state = State::new();
        deliver(&mut state, Input::Move { x: 330, y: 260 }, 0);
        deliver(&mut state, Input::Down(Button::Left), 0);
        deliver(&mut state, Input::Move { x: 330, y: 258 }, 0);
        deliver(&mut state, Input::Up(Button::Left), 0);
        deliver(
            &mut state,
            Input::Key {
                code: 0x39,
                down: true,
            },
            0,
        );
        deliver(
            &mut state,
            Input::Key {
                code: 0x39,
                down: false,
            },
            0,
        );

        // The mouse's lane: each axis that moved, where its data format keeps
        // it, and the button as it went down and came back up. An axis that
        // did not move is not an event.
        let mouse: Vec<(u32, u32)> = state.mouse_data.iter().copied().collect();
        assert_eq!(
            mouse,
            [
                (DIMOUSE_X, 10),
                (DIMOUSE_Y, 20),
                (DIMOUSE_BUTTON0, DOWN),
                (DIMOUSE_Y, (-2i32) as u32),
                (DIMOUSE_BUTTON0, 0),
            ]
        );
        // The keyboard's: a key is its own offset.
        let keys: Vec<(u32, u32)> = state.keyboard_data.iter().copied().collect();
        assert_eq!(keys, [(0x39, DOWN), (0x39, 0)]);
    }
}
