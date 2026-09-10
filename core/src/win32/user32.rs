//! USER32: windows, messages and the pointer.
//!
//! The declared machine has one window, because every title on the roster has
//! one. What a person does reaches it through the host, is delivered at the
//! flip boundary, and is read back by the guest as messages.

use super::{Convention, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use std::collections::BTreeMap;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "USER32",
        name: "RegisterClassA",
        args: 1,
        convention: Convention::Stdcall,
        run: register_class,
    },
    Shim {
        dll: "USER32",
        name: "UnregisterClassA",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "CreateWindowExA",
        args: 12,
        convention: Convention::Stdcall,
        run: create_window,
    },
    Shim {
        dll: "USER32",
        name: "DestroyWindow",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "ShowWindow",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "UpdateWindow",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "SetFocus",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "SetForegroundWindow",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetActiveWindow",
        args: 0,
        convention: Convention::Stdcall,
        run: active_window,
    },
    Shim {
        dll: "USER32",
        name: "GetForegroundWindow",
        args: 0,
        convention: Convention::Stdcall,
        run: active_window,
    },
    Shim {
        dll: "USER32",
        name: "PeekMessageA",
        args: 5,
        convention: Convention::Stdcall,
        run: peek_message,
    },
    Shim {
        dll: "USER32",
        name: "GetMessageA",
        args: 4,
        convention: Convention::Stdcall,
        run: get_message,
    },
    Shim {
        dll: "USER32",
        name: "TranslateMessage",
        args: 1,
        convention: Convention::Stdcall,
        run: translate_message,
    },
    Shim {
        dll: "USER32",
        name: "DispatchMessageA",
        args: 1,
        convention: Convention::Stdcall,
        run: dispatch_message,
    },
    Shim {
        dll: "USER32",
        name: "DefWindowProcA",
        args: 4,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "PostQuitMessage",
        args: 1,
        convention: Convention::Stdcall,
        run: post_quit,
    },
    Shim {
        dll: "USER32",
        name: "GetSystemMetrics",
        args: 1,
        convention: Convention::Stdcall,
        run: system_metrics,
    },
    Shim {
        dll: "USER32",
        name: "ShowCursor",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "SetCursor",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "LoadCursorA",
        args: 2,
        convention: Convention::Stdcall,
        run: load_resource,
    },
    Shim {
        dll: "USER32",
        name: "LoadIconA",
        args: 2,
        convention: Convention::Stdcall,
        run: load_resource,
    },
    Shim {
        dll: "USER32",
        name: "GetClientRect",
        args: 2,
        convention: Convention::Stdcall,
        run: client_rect,
    },
    Shim {
        dll: "USER32",
        name: "GetWindowRect",
        args: 2,
        convention: Convention::Stdcall,
        run: client_rect,
    },
    Shim {
        dll: "USER32",
        name: "AdjustWindowRect",
        args: 3,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "SetWindowPos",
        args: 7,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "SetWindowLongA",
        args: 3,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "GetWindowLongA",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "MessageBoxA",
        args: 4,
        convention: Convention::Stdcall,
        run: message_box,
    },
    Shim {
        dll: "USER32",
        name: "GetDC",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "ReleaseDC",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetCursorPos",
        args: 1,
        convention: Convention::Stdcall,
        run: cursor_pos,
    },
    Shim {
        dll: "USER32",
        name: "SetCursorPos",
        args: 2,
        convention: Convention::Stdcall,
        run: set_cursor_pos,
    },
    Shim {
        dll: "USER32",
        name: "ClipCursor",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "ScreenToClient",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "SetRect",
        args: 5,
        convention: Convention::Stdcall,
        run: set_rect,
    },
    Shim {
        dll: "USER32",
        name: "DialogBoxParamA",
        args: 5,
        convention: Convention::Stdcall,
        run: dialog_box,
    },
    Shim {
        dll: "USER32",
        name: "EndDialog",
        args: 2,
        convention: Convention::Stdcall,
        run: end_dialog,
    },
    Shim {
        dll: "USER32",
        name: "GetDlgItem",
        args: 2,
        convention: Convention::Stdcall,
        run: dialog_item,
    },
    Shim {
        dll: "USER32",
        name: "SendDlgItemMessageA",
        args: 5,
        convention: Convention::Stdcall,
        run: send_dialog_item_message,
    },
    Shim {
        dll: "USER32",
        name: "SendMessageA",
        args: 4,
        convention: Convention::Stdcall,
        run: send_message,
    },
    Shim {
        dll: "USER32",
        name: "GetDesktopWindow",
        args: 0,
        convention: Convention::Stdcall,
        run: desktop_window,
    },
    Shim {
        dll: "USER32",
        name: "PostMessageA",
        args: 4,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "SetActiveWindow",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "IsIconic",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "GetWindowThreadProcessId",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "SetTimer",
        args: 4,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "KillTimer",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "ClientToScreen",
        args: 2,
        convention: Convention::Stdcall,
        // The title's window covers the declared screen from its corner, so
        // a point in it is the same point on the screen.
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "MoveWindow",
        args: 6,
        convention: Convention::Stdcall,
        // Nothing on the declared machine is drawn from where a window is, and
        // the title's own windows cover the screen; the move always succeeds.
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "EnableWindow",
        args: 2,
        convention: Convention::Stdcall,
        run: enable_window,
    },
    Shim {
        dll: "USER32",
        name: "InvalidateRect",
        args: 3,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "BeginPaint",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "EndPaint",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetKeyState",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "GetAsyncKeyState",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "MapVirtualKeyA",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "SetWindowTextA",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetWindowTextA",
        args: 3,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "IsWindow",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetParent",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "SetCapture",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "ReleaseCapture",
        args: 0,
        convention: Convention::Stdcall,
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetSystemMenu",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "LoadStringA",
        args: 4,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "CharUpperA",
        args: 1,
        convention: Convention::Stdcall,
        run: identity,
    },
    Shim {
        dll: "USER32",
        name: "CharLowerA",
        args: 1,
        convention: Convention::Stdcall,
        run: identity,
    },
    Shim {
        dll: "USER32",
        name: "wvsprintfA",
        args: 3,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "GetMenu",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
];

/// The one window the declared machine has.
///
/// A handle rather than a pointer, and a fixed one: a title that keeps it and
/// compares it later gets the same answer.
pub const WINDOW: u32 = 0x0001_0010;

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
    ctx.state.window_made = true;
    Ok(WINDOW)
}

fn active_window(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    Ok(if ctx.state.window_made { WINDOW } else { 0 })
}

const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const MK_LBUTTON: u32 = 0x0001;

/// What a person did, as the declared machine's devices and its message queue
/// see it: the pointer, the button and the keys change, and the title's window
/// is sent what Windows would send it.
pub fn deliver(state: &mut super::State, event: crate::host::Input, time: u32) {
    use crate::host::Input;
    match event {
        Input::Move { x, y } => {
            state.mouse_delta.0 += x - state.last_move.0;
            state.mouse_delta.1 += y - state.last_move.1;
            state.last_move = (x, y);
            state.pointer = (x, y);
            let buttons = if state.mouse_down { MK_LBUTTON } else { 0 };
            post(state, WM_MOUSEMOVE, buttons, time);
        }
        Input::Down => {
            state.mouse_down = true;
            post(state, WM_LBUTTONDOWN, MK_LBUTTON, time);
        }
        Input::Up => {
            state.mouse_down = false;
            post(state, WM_LBUTTONUP, 0, time);
        }
        Input::Key { code, down } => {
            // Keys reach a title through DirectInput's keyboard, numbered as
            // that device numbers them.
            state.keys_down.retain(|k| *k != code);
            if down {
                state.keys_down.push(code);
            }
        }
    }
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
    // Only a key message has a character to make, and this machine posts
    // none: a title's keys arrive through DirectInput.
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

fn set_cursor_pos(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The title puts the pointer somewhere, and that is where it is until a
    // person moves it.
    ctx.state.pointer = (args[0] as i32, args[1] as i32);
    Ok(1)
}

fn get_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // GetMessageA(msg, window, first, last). A message waiting is taken. A
    // title that waits for one on an empty queue would wait forever on a
    // machine that only takes input at a flip, so it is given the one message
    // that ends a loop: quit.
    if take_message(ctx, args, true)? {
        return Ok(1);
    }
    let msg = args[0];
    if msg != 0 {
        ctx.mem.write_u32(msg, 0).map_err(Stop::Fault)?;
        // WM_QUIT.
        ctx.mem.write_u32(msg + 4, 0x0012).map_err(Stop::Fault)?;
    }
    Ok(0)
}

fn post_quit(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    ctx.log
        .line(&format!("guest asked to quit with {}", args[0]));
    ctx.state.quit_asked = true;
    Ok(0)
}

fn system_metrics(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's screen, and nothing measured from the host's.
    let (width, height, _) = ctx
        .title
        .declared
        .modes
        .first()
        .copied()
        .unwrap_or((640, 480, 16));
    Ok(match args[0] {
        0 => width,
        1 => height,
        // Everything else is a border or a caption this machine does not have.
        _ => 0,
    })
}

fn load_resource(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A cursor or an icon: a handle the guest only ever passes back to us.
    Ok(0x0001_0020)
}

fn client_rect(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (width, height, _) = ctx
        .title
        .declared
        .modes
        .first()
        .copied()
        .unwrap_or((640, 480, 16));
    let at = args[1];
    for (offset, value) in [(0, 0), (4, 0), (8, width), (12, height)] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(1)
}

fn set_rect(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    for (i, offset) in (0..4).zip([0, 4, 8, 12]) {
        ctx.mem
            .write_u32(args[0] + offset, args[i + 1])
            .map_err(Stop::Fault)?;
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

/// Keep the module's own use of `Object` honest: a window is a handle this
/// machine hands out rather than an object it keeps.
const _: Option<Object> = None;

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
    texts: BTreeMap<u32, String>,
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
        texts: BTreeMap::new(),
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

fn send_dialog_item_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let window = dialog_item(ctx, &args[..2])?;
    send_message(ctx, &[window, args[2], args[3], args[4]])
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
        0x000c => {
            dialog.texts.insert(id, text);
            1
        }
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

fn identity(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The character functions hand back what they were given; case folding is
    // the C runtime's, and this machine's strings are already in one case.
    Ok(args[0])
}

#[cfg(test)]
mod tests {
    use super::{deliver, WINDOW};
    use crate::host::Input;
    use crate::win32::State;

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

        deliver(&mut state, Input::Down, 4);
        deliver(&mut state, Input::Move { x: 321, y: 222 }, 5);
        deliver(&mut state, Input::Up, 6);
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
        deliver(&mut state, Input::Move { x: 10, y: 20 }, 0);
        deliver(&mut state, Input::Down, 0);
        let first = state.take_mouse_state();
        assert_eq!(first[..8], [10, 0, 0, 0, 20, 0, 0, 0]);
        assert_eq!(first[12], 0x80, "a button held reads as its top bit");

        // The title putting the pointer somewhere is not the mouse moving.
        state.pointer = (300, 300);
        deliver(&mut state, Input::Move { x: 12, y: 19 }, 0);
        let second = state.take_mouse_state();
        assert_eq!(second[..8], [2, 0, 0, 0, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(state.take_mouse_state()[..8], [0; 8], "looking took it");
    }
}
