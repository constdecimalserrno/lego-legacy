//! DirectInput.
//!
//! A title asks for a keyboard and a mouse and reads their state each frame.
//! What it reads is the run's own input, so a route replays a session exactly:
//! the device is a view of what was recorded, not of what a person is doing
//! now.

use super::{ComState, Convention, Ctx, Shim, State};
use crate::com::{Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[Shim {
    dll: "DINPUT",
    name: "DirectInputCreateA",
    args: 4,
    convention: Convention::Stdcall,
    run: create,
}];

/// What a DirectInput call answers with when it worked.
const DI_OK: u32 = S_OK;

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // DirectInputCreateA(hinst, version, out, outer).
    let out = args[2];
    let object = State::make_com(ctx, &DIRECT_INPUT, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("dinput created at {object:#010x}"));
    Ok(DI_OK)
}

/// `IDirectInputA`, in slot order.
pub static DIRECT_INPUT: Interface = Interface {
    name: "IDirectInputA",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "CreateDevice",
            args: 3,
            run: create_device,
        },
        Method {
            name: "EnumDevices",
            args: 4,
            run: enum_devices,
        },
        Method {
            name: "GetDeviceStatus",
            args: 1,
            run: device_status,
        },
        Method {
            name: "RunControlPanel",
            args: 2,
            run: ok,
        },
        Method {
            name: "Initialize",
            args: 2,
            run: ok,
        },
    ],
};

/// `IDirectInputDeviceA`, in slot order.
pub static DIRECT_INPUT_DEVICE: Interface = Interface {
    name: "IDirectInputDeviceA",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "GetCapabilities",
            args: 1,
            run: ok,
        },
        Method {
            name: "EnumObjects",
            args: 3,
            run: ok,
        },
        Method {
            name: "GetProperty",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetProperty",
            args: 2,
            run: ok,
        },
        Method {
            name: "Acquire",
            args: 0,
            run: acquire,
        },
        Method {
            name: "Unacquire",
            args: 0,
            run: unacquire,
        },
        Method {
            name: "GetDeviceState",
            args: 2,
            run: device_state,
        },
        Method {
            name: "GetDeviceData",
            args: 4,
            run: device_data,
        },
        Method {
            name: "SetDataFormat",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetEventNotification",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetCooperativeLevel",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetObjectInfo",
            args: 3,
            run: ok,
        },
        Method {
            name: "GetDeviceInfo",
            args: 1,
            run: ok,
        },
        Method {
            name: "RunControlPanel",
            args: 2,
            run: ok,
        },
        Method {
            name: "Initialize",
            args: 3,
            run: ok,
        },
    ],
};

/// Every interface this module can hand out.
pub static INTERFACES: &[&Interface] = &[&DIRECT_INPUT, &DIRECT_INPUT_DEVICE];

/// A method that has nothing to do on this machine, and says so by
/// succeeding: there is no device to configure behind it.
fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(DI_OK)
}

/// The two device identifiers a title asks for by name.
///
/// GUID_SysKeyboard and GUID_SysMouse, recognised by their first word, which
/// is enough to tell them apart and all this machine needs.
const GUID_KEYBOARD: u32 = 0x6f1d_2b61;
const GUID_MOUSE: u32 = 0x6f1d_2b60;

fn create_device(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDevice(guid, out, outer).
    let guid = ctx.mem.read_u32(args[0]).map_err(Stop::Fault)?;
    let keyboard = guid == GUID_KEYBOARD;
    if guid != GUID_KEYBOARD && guid != GUID_MOUSE {
        // A device this machine does not have. DIERR_DEVICENOTREG.
        ctx.log.line(&format!(
            "dinput device {guid:#010x} is not one this machine has"
        ));
        return Ok(0x8007_0002);
    }
    let object = State::make_com(
        ctx,
        &DIRECT_INPUT_DEVICE,
        ComState::Device {
            keyboard,
            acquired: false,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "dinput {} at {object:#010x}",
        if keyboard { "keyboard" } else { "mouse" }
    ));
    Ok(DI_OK)
}

fn enum_devices(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The callback is not called: this machine has exactly the two devices a
    // title asks for by name, and enumerating tells it nothing new.
    Ok(DI_OK)
}

fn device_status(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(DI_OK)
}

fn acquire(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Device { acquired, .. } = &mut object.state {
            *acquired = true;
        }
    }
    Ok(DI_OK)
}

fn unacquire(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Device { acquired, .. } = &mut object.state {
            *acquired = false;
        }
    }
    Ok(DI_OK)
}

fn device_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDeviceState(size, out). The run's own input decides what is in it.
    let (size, out) = (args[0], args[1]);
    let keyboard = matches!(
        ctx.state.com(this).map(|o| &o.state),
        Some(ComState::Device { keyboard: true, .. })
    );
    let state = if keyboard {
        ctx.state.keyboard_state()
    } else {
        ctx.state.take_mouse_state()
    };
    let take = (size as usize).min(state.len());
    ctx.mem.write(out, &state[..take]).map_err(Stop::Fault)?;
    Ok(DI_OK)
}

fn device_data(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDeviceData(size, out, count, flags). Nothing is buffered: this
    // machine delivers input at the flip boundary and a title reads the state
    // rather than a queue.
    let count_at = args[2];
    if count_at != 0 {
        ctx.mem.write_u32(count_at, 0).map_err(Stop::Fault)?;
    }
    Ok(DI_OK)
}
