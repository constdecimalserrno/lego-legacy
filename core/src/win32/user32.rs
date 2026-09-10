//! USER32: windows, messages and the pointer.
//!
//! The declared machine has one window, because every title on the roster has
//! one. What a person does reaches it through the host, is delivered at the
//! flip boundary, and is read back by the guest as messages.

use super::{Convention, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;

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
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "DispatchMessageA",
        args: 1,
        convention: Convention::Stdcall,
        run: succeed,
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
        run: succeed,
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
        run: succeed,
    },
    Shim {
        dll: "USER32",
        name: "GetDlgItem",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "SendDlgItemMessageA",
        args: 5,
        convention: Convention::Stdcall,
        run: succeed_zero,
    },
    Shim {
        dll: "USER32",
        name: "SendMessageA",
        args: 4,
        convention: Convention::Stdcall,
        run: succeed_zero,
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
        name: "EnableWindow",
        args: 2,
        convention: Convention::Stdcall,
        run: succeed,
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

fn peek_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // The declared machine's queue is empty. What a person did reaches the
    // guest through its own devices at the flip boundary, not through
    // messages, and a title that finds nothing here gets on with drawing.
    let msg = args[0];
    if msg != 0 {
        for offset in (0..28).step_by(4) {
            ctx.mem.write_u32(msg + offset, 0).map_err(Stop::Fault)?;
        }
    }
    Ok(0)
}

fn get_message(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // A title that waits for a message would wait forever on a machine whose
    // queue is empty, so it is given the one message that ends a loop: quit.
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

fn dialog_box(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // A modal dialog: the title is asking a person something. On this machine
    // nobody is there to answer, so it is told the person accepted whatever it
    // proposed, which is what a player pressing the default button would do.
    ctx.log.line("dialog accepted without being shown");
    // IDOK.
    Ok(1)
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
