//! DirectInput.
//!
//! A title asks for a keyboard and a mouse and reads their state each frame.
//! What it reads is the run's own input, so a route replays a session exactly:
//! the device is a view of what was recorded, not of what a person is doing
//! now.

use super::{stdcall, ComState, Ctx, Shim, State};
use crate::com::{method, ok, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[stdcall("DINPUT", "DirectInputCreateA", 4, create)];

const DI_OK: u32 = S_OK;
const DIERR_INVALIDPARAM: u32 = 0x8007_0057;
const DISCL_EXCLUSIVE: u32 = 0x0000_0001;
const DIDC_ATTACHED: u32 = 0x0000_0001;
const DEVTYPE_MOUSE: u32 = 2 | (2 << 8);
const DEVTYPE_KEYBOARD: u32 = 3 | (4 << 8);

const DIDFT_RELAXIS: u32 = 0x0000_0001;
const DIDFT_PSHBUTTON: u32 = 0x0000_0004;
const DIDFT_TYPE_MASK: u32 = 0x0000_00ff;
const DIDFT_ATTR_MASK: u32 = 0x1f00_0000;

const DIDEVCAPS_DX3: u32 = 24;
const DIDEVCAPS_DX5: u32 = 44;
const DIDEVICEINSTANCEA_DX3: u32 = 0x230;
const DIDEVICEINSTANCEA_DX5: u32 = 0x244;
const DIDEVICEOBJECTINSTANCEA_DX5: u32 = 0x13c;

/// Standard DirectInput GUIDs (`dinput.h`, little-endian wire layout).
const GUID_SYS_MOUSE: [u8; 16] = [
    0x60, 0x2b, 0x1d, 0x6f, 0xa0, 0xd5, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const GUID_SYS_KEYBOARD: [u8; 16] = [
    0x61, 0x2b, 0x1d, 0x6f, 0xa0, 0xd5, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const GUID_XAXIS: [u8; 16] = [
    0xe0, 0x02, 0x6d, 0xa3, 0xf3, 0xc9, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const GUID_YAXIS: [u8; 16] = [
    0xe1, 0x02, 0x6d, 0xa3, 0xf3, 0xc9, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const GUID_ZAXIS: [u8; 16] = [
    0xe2, 0x02, 0x6d, 0xa3, 0xf3, 0xc9, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const GUID_BUTTON: [u8; 16] = [
    0xf0, 0x02, 0x6d, 0xa3, 0xf3, 0xc9, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];
const GUID_KEY: [u8; 16] = [
    0x20, 0x82, 0x72, 0x55, 0x3c, 0xd3, 0xcf, 0x11, 0xbf, 0xc7, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00,
];

/// Standard 104-key PC Enhanced keyboard objects (`(DIK_* scancode, name)`).
const KEYBOARD_KEYS: &[(u32, &str)] = &[
    (0x01, "ESC"),
    (0x02, "1"),
    (0x03, "2"),
    (0x04, "3"),
    (0x05, "4"),
    (0x06, "5"),
    (0x07, "6"),
    (0x08, "7"),
    (0x09, "8"),
    (0x0a, "9"),
    (0x0b, "0"),
    (0x0c, "-"),
    (0x0d, "="),
    (0x0e, "BACKSPACE"),
    (0x0f, "TAB"),
    (0x10, "Q"),
    (0x11, "W"),
    (0x12, "E"),
    (0x13, "R"),
    (0x14, "T"),
    (0x15, "Y"),
    (0x16, "U"),
    (0x17, "I"),
    (0x18, "O"),
    (0x19, "P"),
    (0x1a, "["),
    (0x1b, "]"),
    (0x1c, "ENTER"),
    (0x1d, "CTRL"),
    (0x1e, "A"),
    (0x1f, "S"),
    (0x20, "D"),
    (0x21, "F"),
    (0x22, "G"),
    (0x23, "H"),
    (0x24, "J"),
    (0x25, "K"),
    (0x26, "L"),
    (0x27, ";"),
    (0x28, "'"),
    (0x29, "`"),
    (0x2a, "SHIFT"),
    (0x2b, "\\"),
    (0x2c, "Z"),
    (0x2d, "X"),
    (0x2e, "C"),
    (0x2f, "V"),
    (0x30, "B"),
    (0x31, "N"),
    (0x32, "M"),
    (0x33, ","),
    (0x34, "."),
    (0x35, "/"),
    (0x36, "RIGHT SHIFT"),
    (0x37, "NUM *"),
    (0x38, "ALT"),
    (0x39, "SPACE"),
    (0x3a, "CAPS LOCK"),
    (0x3b, "F1"),
    (0x3c, "F2"),
    (0x3d, "F3"),
    (0x3e, "F4"),
    (0x3f, "F5"),
    (0x40, "F6"),
    (0x41, "F7"),
    (0x42, "F8"),
    (0x43, "F9"),
    (0x44, "F10"),
    (0x45, "NUM LOCK"),
    (0x46, "SCROLL LOCK"),
    (0x47, "NUM 7"),
    (0x48, "NUM 8"),
    (0x49, "NUM 9"),
    (0x4a, "NUM -"),
    (0x4b, "NUM 4"),
    (0x4c, "NUM 5"),
    (0x4d, "NUM 6"),
    (0x4e, "NUM +"),
    (0x4f, "NUM 1"),
    (0x50, "NUM 2"),
    (0x51, "NUM 3"),
    (0x52, "NUM 0"),
    (0x53, "NUM DEL"),
    (0x57, "F11"),
    (0x58, "F12"),
    (0x9c, "NUM ENTER"),
    (0x9d, "RIGHT CTRL"),
    (0xb5, "NUM /"),
    (0xb7, "SYSRQ"),
    (0xb8, "RIGHT ALT"),
    (0xc5, "PAUSE"),
    (0xc7, "HOME"),
    (0xc8, "UP"),
    (0xc9, "PAGE UP"),
    (0xcb, "LEFT"),
    (0xcd, "RIGHT"),
    (0xcf, "END"),
    (0xd0, "DOWN"),
    (0xd1, "PAGE DOWN"),
    (0xd2, "INSERT"),
    (0xd3, "DELETE"),
    (0xdb, "LEFT WINDOWS"),
    (0xdc, "RIGHT WINDOWS"),
    (0xdd, "APPS"),
];

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
        method("CreateDevice", 3, create_device),
        method("EnumDevices", 4, enum_devices),
        method("GetDeviceStatus", 1, device_status),
        method("RunControlPanel", 2, ok),
        method("Initialize", 2, ok),
    ],
};

/// `IDirectInputDeviceA` / `IDirectInputDevice2A`, in slot order. Its reads of
/// what a person did, `GetDeviceState` and `GetDeviceData`, are input polls,
/// and are named in `machine::POLLS` as well (#215). Slots 18-26 extend the
/// vtable to DirectX 5/6's `IDirectInputDevice2A` (`QueryInterface` already
/// hands out this table for `IID_IDirectInputDevice2A`), with `Poll` at slot
/// 25 (`+0x64`, #370).
pub static DIRECT_INPUT_DEVICE: Interface = Interface {
    name: "IDirectInputDeviceA",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetCapabilities", 1, get_capabilities),
        method("EnumObjects", 3, enum_objects),
        method("GetProperty", 2, ok),
        method("SetProperty", 2, ok),
        method("Acquire", 0, acquire),
        method("Unacquire", 0, unacquire),
        method("GetDeviceState", 2, device_state),
        method("GetDeviceData", 4, device_data),
        method("SetDataFormat", 1, ok),
        method("SetEventNotification", 1, ok),
        method("SetCooperativeLevel", 2, set_cooperative_level),
        method("GetObjectInfo", 3, ok),
        method("GetDeviceInfo", 1, get_device_info),
        method("RunControlPanel", 2, ok),
        method("Initialize", 3, ok),
        wall("CreateEffect", 4),
        wall("EnumEffects", 3),
        wall("GetEffectInfo", 2),
        wall("GetForceFeedbackState", 1),
        wall("SendForceFeedbackCommand", 1),
        wall("EnumCreatedEffectObjects", 3),
        wall("Escape", 1),
        method("Poll", 0, ok),
        wall("SendDeviceData", 4),
    ],
};

/// Every interface this module can hand out.
pub static INTERFACES: &[&Interface] = &[&DIRECT_INPUT, &DIRECT_INPUT_DEVICE];

/// A method that has nothing to do on this machine, and says so by
/// succeeding: there is no device to configure behind it.
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
    let object = State::make_com(
        ctx,
        &DIRECT_INPUT_DEVICE,
        ComState::Device {
            keyboard,
            exclusive: false,
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

fn is_keyboard(ctx: &Ctx<'_>, this: u32) -> bool {
    matches!(
        ctx.state.com(this).map(|o| &o.state),
        Some(ComState::Device { keyboard: true, .. })
    )
}

/// Acquiring `GUID_SysMouse` with `DISCL_EXCLUSIVE` hides the Windows system
/// cursor until `Unacquire` or final `Release` (#374).
fn set_cooperative_level(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if let Some(ComState::Device { exclusive, .. }) = ctx.state.com_mut(this).map(|o| &mut o.state)
    {
        *exclusive = args[1] & DISCL_EXCLUSIVE != 0;
    }
    Ok(DI_OK)
}

fn acquire(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(ComState::Device { acquired, .. }) = ctx.state.com_mut(this).map(|o| &mut o.state) {
        *acquired = true;
    }
    Ok(DI_OK)
}

fn unacquire(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    if let Some(ComState::Device { acquired, .. }) = ctx.state.com_mut(this).map(|o| &mut o.state) {
        *acquired = false;
    }
    Ok(DI_OK)
}

/// `IDirectInputDeviceA::GetCapabilities(lpDIDevCaps)`.
fn get_capabilities(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[0];
    if out == 0 {
        return Ok(DIERR_INVALIDPARAM);
    }
    let size = ctx.mem.read_u32(out).map_err(Stop::Fault)?;
    if size != DIDEVCAPS_DX3 && size != DIDEVCAPS_DX5 {
        return Ok(DIERR_INVALIDPARAM);
    }
    let keyboard = is_keyboard(ctx, this);
    ctx.mem
        .write(out, &vec![0u8; size as usize])
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, size).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(out + 0x04, DIDC_ATTACHED)
        .map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(
            out + 0x08,
            if keyboard {
                DEVTYPE_KEYBOARD
            } else {
                DEVTYPE_MOUSE
            },
        )
        .map_err(Stop::Fault)?;
    let (axes, buttons) = if keyboard {
        (0u32, u32::try_from(KEYBOARD_KEYS.len()).unwrap_or(0))
    } else {
        (3u32, 3u32)
    };
    ctx.mem.write_u32(out + 0x0c, axes).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(out + 0x10, buttons)
        .map_err(Stop::Fault)?;
    Ok(DI_OK)
}

/// `IDirectInputDeviceA::GetDeviceInfo(pdidi)`.
fn get_device_info(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[0];
    if out == 0 {
        return Ok(DIERR_INVALIDPARAM);
    }
    let size = ctx.mem.read_u32(out).map_err(Stop::Fault)?;
    if size != DIDEVICEINSTANCEA_DX3 && size != DIDEVICEINSTANCEA_DX5 {
        return Ok(DIERR_INVALIDPARAM);
    }
    let keyboard = is_keyboard(ctx, this);
    ctx.mem
        .write(out, &vec![0u8; size as usize])
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, size).map_err(Stop::Fault)?;
    let guid = if keyboard {
        &GUID_SYS_KEYBOARD
    } else {
        &GUID_SYS_MOUSE
    };
    ctx.mem.write(out + 0x04, guid).map_err(Stop::Fault)?;
    ctx.mem.write(out + 0x14, guid).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(
            out + 0x24,
            if keyboard {
                DEVTYPE_KEYBOARD
            } else {
                DEVTYPE_MOUSE
            },
        )
        .map_err(Stop::Fault)?;
    let name = if keyboard { "Keyboard" } else { "Mouse" };
    ctx.write_string(out + 0x28, 260, name)
        .map_err(Stop::Fault)?;
    ctx.write_string(out + 0x12c, 260, name)
        .map_err(Stop::Fault)?;
    if size >= DIDEVICEINSTANCEA_DX5 {
        ctx.mem.write_u16(out + 0x240, 1).map_err(Stop::Fault)?;
        ctx.mem
            .write_u16(out + 0x242, if keyboard { 6 } else { 2 })
            .map_err(Stop::Fault)?;
    }
    Ok(DI_OK)
}

/// `IDirectInputDeviceA::EnumObjects(lpCallback, pvRef, dwFlags)`.
fn enum_objects(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (callback, context, flags) = (args[0], args[1], args[2]);
    if callback == 0 {
        return Ok(DIERR_INVALIDPARAM);
    }
    // None of our keyboard or mouse objects carry force-feedback, output, or
    // vendor/alias attributes, so any attribute filter bit yields zero matches.
    if (flags & DIDFT_ATTR_MASK) != 0 {
        return Ok(DI_OK);
    }
    let type_filter = flags & DIDFT_TYPE_MASK;
    let mut calls = Vec::new();
    let mut push_object = |ctx: &mut Ctx<'_>,
                           guid: &[u8; 16],
                           ofs: u32,
                           obj_type: u32,
                           name: &str|
     -> Result<(), Stop> {
        if type_filter != 0 && (obj_type & type_filter) == 0 {
            return Ok(());
        }
        let at = ctx.state.heap.alloc(DIDEVICEOBJECTINSTANCEA_DX5);
        ctx.mem
            .write(at, &[0u8; DIDEVICEOBJECTINSTANCEA_DX5 as usize])
            .map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(at, DIDEVICEOBJECTINSTANCEA_DX5)
            .map_err(Stop::Fault)?;
        ctx.mem.write(at + 0x04, guid).map_err(Stop::Fault)?;
        ctx.mem.write_u32(at + 0x14, ofs).map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(at + 0x18, obj_type)
            .map_err(Stop::Fault)?;
        ctx.write_string(at + 0x20, 260, name)
            .map_err(Stop::Fault)?;
        calls.push(vec![at, context]);
        Ok(())
    };

    if is_keyboard(ctx, this) {
        for &(code, name) in KEYBOARD_KEYS {
            push_object(ctx, &GUID_KEY, code, DIDFT_PSHBUTTON | (code << 8), name)?;
        }
    } else {
        for (guid, ofs, obj_type, name) in [
            (&GUID_XAXIS, 0, DIDFT_RELAXIS, "X-axis"),
            (&GUID_YAXIS, 4, DIDFT_RELAXIS | (1 << 8), "Y-axis"),
            (&GUID_ZAXIS, 8, DIDFT_RELAXIS | (2 << 8), "Wheel"),
            (&GUID_BUTTON, 12, DIDFT_PSHBUTTON | (3 << 8), "Button 0"),
            (&GUID_BUTTON, 13, DIDFT_PSHBUTTON | (4 << 8), "Button 1"),
            (&GUID_BUTTON, 14, DIDFT_PSHBUTTON | (5 << 8), "Button 2"),
        ] {
            push_object(ctx, guid, ofs, obj_type, name)?;
        }
    }

    if !calls.is_empty() {
        super::call_each(ctx, callback, &calls, true)?;
    }
    Ok(DI_OK)
}

fn device_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDeviceState(size, out). The run's own input decides what is in it.
    let (size, out) = (args[0], args[1]);
    let state = if is_keyboard(ctx, this) {
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
    if ctx.state.keyboard_data.is_empty() && ctx.state.mouse_data.is_empty() {
        if count_at != 0 {
            let _ = ctx.mem.read_u32(count_at).map_err(Stop::Fault)?;
            ctx.mem.write_u32(count_at, 0).map_err(Stop::Fault)?;
        }
        return Ok(DI_OK);
    }
    let keyboard = is_keyboard(ctx, this);
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
    if waiting == 0 {
        if count_at != 0 {
            ctx.mem.write_u32(count_at, 0).map_err(Stop::Fault)?;
        }
        return Ok(DI_OK);
    }
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

#[cfg(test)]
mod tests {
    use super::{device_data, DIRECT_INPUT_DEVICE, DI_OK};
    use crate::cpu::Stop;
    use crate::guest::{Fault, GuestMem};
    use crate::win32::tests::with_test_ctx;
    use crate::win32::{ComState, State};

    #[test]
    fn device_data_empty_fast_path_preserves_fault_checks_and_zero_count() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let kbd = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                },
            )
            .expect("keyboard device");
            let count_at = 0x1000u32;
            let oob = 0x3000_0000u32;

            // Both queues empty, valid count_at -> writes 0.
            ctx.mem.write_u32(count_at, 16).unwrap();
            assert_eq!(
                device_data(ctx, kbd, &[16, 0x1100, count_at, 0]).unwrap(),
                DI_OK
            );
            assert_eq!(ctx.mem.read_u32(count_at).unwrap(), 0);

            // Both queues empty, null count_at -> succeeds without touching memory.
            assert_eq!(device_data(ctx, kbd, &[16, 0x1100, 0, 0]).unwrap(), DI_OK);

            // Both queues empty, out-of-bounds count_at -> preserves read fault.
            let err = device_data(ctx, kbd, &[16, 0x1100, oob, 0]).unwrap_err();
            assert!(matches!(err, Stop::Fault(Fault { write: false, .. })));

            // Other lane non-empty, selected lane empty -> still writes 0 and preserves read fault.
            ctx.state.buffer_input(false, 0, 1);
            ctx.mem.write_u32(count_at, 8).unwrap();
            assert_eq!(
                device_data(ctx, kbd, &[16, 0x1100, count_at, 0]).unwrap(),
                DI_OK
            );
            assert_eq!(ctx.mem.read_u32(count_at).unwrap(), 0);
            let err = device_data(ctx, kbd, &[16, 0x1100, oob, 0]).unwrap_err();
            assert!(matches!(err, Stop::Fault(Fault { write: false, .. })));
        });
    }

    #[test]
    fn device2_vtable_exposes_poll_at_slot_25() {
        let poll = DIRECT_INPUT_DEVICE.method(25).expect("slot 25 (+0x64)");
        assert_eq!(poll.name, "Poll");
        assert_eq!(poll.args, 0);
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let mouse = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: false,
                    exclusive: false,
                    acquired: false,
                },
            )
            .expect("mouse device");
            assert_eq!((poll.run)(ctx, mouse, &[]).unwrap(), DI_OK);
        });
    }

    #[test]
    fn get_capabilities_and_device_info_populate_true_to_windows_structs() {
        use super::{
            get_capabilities, get_device_info, DEVTYPE_KEYBOARD, DEVTYPE_MOUSE, DIDEVCAPS_DX3,
            DIDEVCAPS_DX5, DIDEVICEINSTANCEA_DX3, DIDEVICEINSTANCEA_DX5, DIERR_INVALIDPARAM,
            GUID_SYS_KEYBOARD, GUID_SYS_MOUSE, KEYBOARD_KEYS,
        };
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let mouse = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: false,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();
            let kbd = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();

            assert_eq!(
                get_capabilities(ctx, mouse, &[0]).unwrap(),
                DIERR_INVALIDPARAM
            );
            let caps = 0x2000u32;
            ctx.mem.write_u32(caps, 12).unwrap();
            assert_eq!(
                get_capabilities(ctx, mouse, &[caps]).unwrap(),
                DIERR_INVALIDPARAM
            );

            ctx.mem.write_u32(caps, DIDEVCAPS_DX5).unwrap();
            assert_eq!(get_capabilities(ctx, mouse, &[caps]).unwrap(), DI_OK);
            assert_eq!(ctx.mem.read_u32(caps + 0x04).unwrap(), 1);
            assert_eq!(ctx.mem.read_u32(caps + 0x08).unwrap(), DEVTYPE_MOUSE);
            assert_eq!(ctx.mem.read_u32(caps + 0x0c).unwrap(), 3);
            assert_eq!(ctx.mem.read_u32(caps + 0x10).unwrap(), 3);

            ctx.mem.write_u32(caps, DIDEVCAPS_DX3).unwrap();
            assert_eq!(get_capabilities(ctx, kbd, &[caps]).unwrap(), DI_OK);
            assert_eq!(ctx.mem.read_u32(caps + 0x08).unwrap(), DEVTYPE_KEYBOARD);
            assert_eq!(ctx.mem.read_u32(caps + 0x0c).unwrap(), 0);
            assert_eq!(KEYBOARD_KEYS.len(), 104);
            assert_eq!(ctx.mem.read_u32(caps + 0x10).unwrap(), 104);

            let info = 0x2400u32;
            assert_eq!(
                get_device_info(ctx, mouse, &[0]).unwrap(),
                DIERR_INVALIDPARAM
            );
            ctx.mem.write_u32(info, 100).unwrap();
            assert_eq!(
                get_device_info(ctx, mouse, &[info]).unwrap(),
                DIERR_INVALIDPARAM
            );

            ctx.mem.write_u32(info, DIDEVICEINSTANCEA_DX5).unwrap();
            assert_eq!(get_device_info(ctx, mouse, &[info]).unwrap(), DI_OK);
            assert_eq!(ctx.mem.slice(info + 0x04, 16).unwrap(), &GUID_SYS_MOUSE);
            assert_eq!(ctx.mem.read_u32(info + 0x24).unwrap(), DEVTYPE_MOUSE);
            assert_eq!(ctx.mem.slice(info + 0x28, 6).unwrap(), b"Mouse\0");
            assert_eq!(ctx.mem.slice(info + 0x12c, 6).unwrap(), b"Mouse\0");
            assert_eq!(ctx.mem.read_u16(info + 0x240).unwrap(), 1);
            assert_eq!(ctx.mem.read_u16(info + 0x242).unwrap(), 2);

            ctx.mem.write_u32(info, DIDEVICEINSTANCEA_DX5).unwrap();
            assert_eq!(get_device_info(ctx, kbd, &[info]).unwrap(), DI_OK);
            assert_eq!(ctx.mem.slice(info + 0x04, 16).unwrap(), &GUID_SYS_KEYBOARD);
            assert_eq!(ctx.mem.read_u32(info + 0x24).unwrap(), DEVTYPE_KEYBOARD);
            assert_eq!(ctx.mem.slice(info + 0x28, 9).unwrap(), b"Keyboard\0");
            assert_eq!(ctx.mem.read_u16(info + 0x240).unwrap(), 1);
            assert_eq!(ctx.mem.read_u16(info + 0x242).unwrap(), 6);

            ctx.mem.write_u32(info, DIDEVICEINSTANCEA_DX3).unwrap();
            assert_eq!(get_device_info(ctx, kbd, &[info]).unwrap(), DI_OK);
            assert_eq!(ctx.mem.read_u32(info).unwrap(), DIDEVICEINSTANCEA_DX3);
        });
    }

    #[test]
    fn enum_objects_filters_mouse_axes_buttons_and_keyboard_keys() {
        use super::{
            enum_objects, DIDEVICEOBJECTINSTANCEA_DX5, DIDFT_PSHBUTTON, DIDFT_RELAXIS,
            DIERR_INVALIDPARAM, GUID_KEY, GUID_XAXIS,
        };
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let mouse = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: false,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();
            let kbd = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();

            assert_eq!(
                enum_objects(ctx, mouse, &[0, 0, 0]).unwrap(),
                DIERR_INVALIDPARAM
            );

            // Inspect the first DIDEVICEOBJECTINSTANCEA passed by the
            // trampoline for mouse axes (`DIDFT_AXIS = 0x03`, 3 objects).
            assert_eq!(
                enum_objects(ctx, mouse, &[0x4000, 0x99, 0x03]).unwrap(),
                DI_OK
            );
            let (trampoline, args) = ctx.call_guest.take().expect("mouse axes trampoline");
            assert!(args.is_empty());
            // Trampoline pushes pvRef (`0x99`), then lpddoi, then calls `0x4000`.
            assert_eq!(ctx.mem.read_u8(trampoline).unwrap(), 0x68);
            assert_eq!(ctx.mem.read_u32(trampoline + 1).unwrap(), 0x99);
            assert_eq!(ctx.mem.read_u8(trampoline + 5).unwrap(), 0x68);
            let first_obj = ctx.mem.read_u32(trampoline + 6).unwrap();
            assert_eq!(
                ctx.mem.read_u32(first_obj).unwrap(),
                DIDEVICEOBJECTINSTANCEA_DX5
            );
            assert_eq!(ctx.mem.slice(first_obj + 0x04, 16).unwrap(), &GUID_XAXIS);
            assert_eq!(ctx.mem.read_u32(first_obj + 0x14).unwrap(), 0);
            assert_eq!(ctx.mem.read_u32(first_obj + 0x18).unwrap(), DIDFT_RELAXIS);
            assert_eq!(ctx.mem.slice(first_obj + 0x20, 7).unwrap(), b"X-axis\0");

            // Non-matching type filters (ABSAXIS=0x02, POV=0x10) and upper
            // attribute filters (DIDFT_FFACTUATOR=0x0100_0000) yield zero calls.
            for flags in [0x02u32, 0x10, 0x0100_0000, 0x0100_0003] {
                assert_eq!(
                    enum_objects(ctx, mouse, &[0x4000, 0x99, flags]).unwrap(),
                    DI_OK
                );
                assert!(ctx.call_guest.is_none(), "mouse flags {flags:#x}");
            }
            for flags in [0x03u32, 0x10, 0x0100_0000] {
                assert_eq!(
                    enum_objects(ctx, kbd, &[0x4000, 0x99, flags]).unwrap(),
                    DI_OK
                );
                assert!(ctx.call_guest.is_none(), "kbd flags {flags:#x}");
            }

            assert_eq!(
                enum_objects(ctx, kbd, &[0x4000, 0x77, 0x0c]).unwrap(),
                DI_OK
            );
            let (kbd_trampoline, _) = ctx.call_guest.take().expect("kbd buttons trampoline");
            assert_eq!(ctx.mem.read_u32(kbd_trampoline + 1).unwrap(), 0x77);
            let first_key = ctx.mem.read_u32(kbd_trampoline + 6).unwrap();
            assert!(kbd_trampoline >= first_key + 104 * DIDEVICEOBJECTINSTANCEA_DX5);
            assert_eq!(ctx.mem.slice(first_key + 0x04, 16).unwrap(), &GUID_KEY);
            assert_eq!(ctx.mem.read_u32(first_key + 0x14).unwrap(), 0x01);
            assert_eq!(
                ctx.mem.read_u32(first_key + 0x18).unwrap(),
                DIDFT_PSHBUTTON | (0x01 << 8)
            );
            assert_eq!(ctx.mem.slice(first_key + 0x20, 4).unwrap(), b"ESC\0");
        });
    }

    #[test]
    fn exclusive_mouse_acquire_hides_pointer_until_unacquire_or_release() {
        use super::{acquire, set_cooperative_level, unacquire, DISCL_EXCLUSIVE};
        use crate::snap::{Reader, Snap, Writer};

        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let mouse = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: false,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();
            let kbd = State::make_com(
                ctx,
                &DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();

            assert!(ctx.state.pointer_shown());

            assert_eq!(
                set_cooperative_level(ctx, kbd, &[0x10010, DISCL_EXCLUSIVE | 0x8]).unwrap(),
                DI_OK
            );
            assert_eq!(acquire(ctx, kbd, &[]).unwrap(), DI_OK);
            assert!(ctx.state.pointer_shown());

            assert_eq!(
                set_cooperative_level(ctx, mouse, &[0x10010, 0x6]).unwrap(),
                DI_OK
            );
            assert_eq!(acquire(ctx, mouse, &[]).unwrap(), DI_OK);
            assert!(ctx.state.pointer_shown());
            assert_eq!(unacquire(ctx, mouse, &[]).unwrap(), DI_OK);

            assert_eq!(
                set_cooperative_level(ctx, mouse, &[0x10010, DISCL_EXCLUSIVE | 0x8]).unwrap(),
                DI_OK
            );
            assert_eq!(acquire(ctx, mouse, &[]).unwrap(), DI_OK);
            assert!(!ctx.state.pointer_shown());
            assert_eq!(ctx.state.cursor_shown, 0);

            let mut w = Writer::new();
            let mut dev = ctx.state.com(mouse).unwrap().state.clone();
            dev.snap(&mut w);
            let mut r = Reader::new(&w.out);
            let mut restored = ComState::None;
            restored.snap(&mut r);
            assert_eq!(
                restored,
                ComState::Device {
                    keyboard: false,
                    exclusive: true,
                    acquired: true,
                }
            );

            assert_eq!(unacquire(ctx, mouse, &[]).unwrap(), DI_OK);
            assert!(ctx.state.pointer_shown());

            assert_eq!(acquire(ctx, mouse, &[]).unwrap(), DI_OK);
            assert!(!ctx.state.pointer_shown());
            assert_eq!(ctx.state.com_ref(mouse, -1), 0);
            assert!(ctx.state.pointer_shown());
        });
    }
}
