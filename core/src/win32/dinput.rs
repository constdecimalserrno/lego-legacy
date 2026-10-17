//! DirectInput.
//!
//! A title asks for a keyboard and a mouse and reads their state each frame.
//! What it reads is the run's own input, so a route replays a session exactly:
//! the device is a view of what was recorded, not of what a person is doing
//! now.

use super::{stdcall, ComState, Ctx, Shim, State};
use crate::com::{Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[stdcall("DINPUT", "DirectInputCreateA", 4, create)];

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
            run: ok,
        },
        Method {
            name: "Unacquire",
            args: 0,
            run: ok,
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
        // A device this machine does not have. DIERR_DEVICENOTREG, which is
        // REGDB_E_CLASSNOTREG (#93).
        ctx.log.line(&format!(
            "dinput device {guid:#010x} is not one this machine has"
        ));
        return Ok(0x8004_0154);
    }
    let object = State::make_com(ctx, &DIRECT_INPUT_DEVICE, ComState::Device { keyboard })
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

fn device_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDeviceState(size, out). The run's own input decides what is in it.
    let (size, out) = (args[0], args[1]);
    let keyboard = matches!(
        ctx.state.com(this).map(|o| &o.state),
        Some(ComState::Device { keyboard: true })
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

/// `DIGDD_PEEK`: look at what is waiting without taking it.
const PEEK: u32 = 1;

fn device_data(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDeviceData(size, out, count, flags). A title that reads its device
    // this way gets what a person did, oldest first; one that reads the state
    // instead gets the same input through `device_state`.
    let (size, out, count_at, flags) = (args[0], args[1], args[2], args[3]);
    let keyboard = matches!(
        ctx.state.com(this).map(|o| &o.state),
        Some(ComState::Device { keyboard: true })
    );
    let asked = if count_at == 0 {
        0
    } else {
        ctx.mem.read_u32(count_at).map_err(Stop::Fault)?
    };
    let waiting = if keyboard {
        ctx.state.keyboard_data.len()
    } else {
        ctx.state.mouse_data.len()
    };
    // A title asks how much is waiting by passing no buffer, and empties the
    // lane by passing none and not peeking.
    let take = (asked as usize).min(waiting);
    let stamp = u32::try_from(ctx.clock_millis & 0xffff_ffff).unwrap_or(0);
    for i in 0..take {
        let lane = if keyboard {
            &mut ctx.state.keyboard_data
        } else {
            &mut ctx.state.mouse_data
        };
        let Some(&(offset, value)) = lane.get(i) else {
            break;
        };
        if out != 0 {
            let at = out + u32::try_from(i).unwrap_or(0) * size;
            ctx.state.input_sequence += 1;
            let sequence = ctx.state.input_sequence;
            for (field, word) in [(0, offset), (4, value), (8, stamp), (12, sequence)] {
                ctx.mem.write_u32(at + field, word).map_err(Stop::Fault)?;
            }
        }
    }
    if flags & PEEK == 0 {
        let lane = if keyboard {
            &mut ctx.state.keyboard_data
        } else {
            &mut ctx.state.mouse_data
        };
        lane.drain(..take);
    }
    if count_at != 0 {
        ctx.mem
            .write_u32(count_at, u32::try_from(take).unwrap_or(0))
            .map_err(Stop::Fault)?;
    }
    Ok(DI_OK)
}
