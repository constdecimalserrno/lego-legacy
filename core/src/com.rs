//! COM objects the guest can hold and call.
//!
//! Every DirectX interface is the same shape: the guest has a pointer to an
//! object whose first word points at a table of function pointers, and it
//! calls through that table. The table lives in guest memory and every slot in
//! it is an address on the boundary, so a call arrives here (ADR-0002).

use crate::cpu::Stop;
use crate::win32::Ctx;

/// One method of an interface.
pub struct Method {
    pub name: &'static str,
    /// How many four-byte arguments it takes, not counting the object itself.
    pub args: u32,
    pub run: ComFn,
}

/// What a method does. `this` is the object the guest called through.
pub type ComFn = fn(&mut Ctx<'_>, u32, &[u32]) -> Result<u32, Stop>;

/// One interface, as a table of methods in slot order.
pub struct Interface {
    pub name: &'static str,
    pub methods: &'static [Method],
}

impl Interface {
    /// The method in one slot, if the interface declares one there.
    #[must_use]
    pub fn method(&self, slot: usize) -> Option<&'static Method> {
        self.methods.get(slot)
    }
}

/// A slot the interface's own header declares and nothing here has needed.
///
/// Its name and argument count are taken from the header, so the table is the
/// interface's real width and every slot sits where the guest expects it. What
/// it does is not written: a guest that calls it stops the run, and the wall
/// names the method rather than a slot number (#12).
#[must_use]
pub const fn wall(name: &'static str, args: u32) -> Method {
    Method {
        name,
        args,
        run: unwritten,
    }
}

/// A written method, spelled on one line.
#[must_use]
pub const fn method(name: &'static str, args: u32, run: ComFn) -> Method {
    Method { name, args, run }
}

/// What a declared but unwritten slot does. The machine fills in which
/// interface and method it was, because only it knows which slot was called.
fn unwritten(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Err(Stop::NotWritten {
        what: String::new(),
        caller: 0,
    })
}

/// What every COM object answers in its first three slots.
///
/// Reference counting is real but nothing is ever freed by it: the machine
/// holds every object it made until the run ends, because a guest that
/// released an object and then used it would otherwise read a plausible
/// answer instead of stopping.
pub const QUERY_INTERFACE: Method = Method {
    name: "QueryInterface",
    args: 2,
    run: query_interface,
};
pub const ADD_REF: Method = Method {
    name: "AddRef",
    args: 0,
    run: add_ref,
};
pub const RELEASE: Method = Method {
    name: "Release",
    args: 0,
    run: release,
};

/// The answer a COM call gives when it worked.
pub const S_OK: u32 = 0;
/// The answer when the interface asked for is not one this object has.
pub const E_NOINTERFACE: u32 = 0x8000_4002;

/// Read the sixteen bytes that name an interface.
///
/// # Errors
/// When the address is outside the arena.
pub fn read_iid(ctx: &Ctx<'_>, at: u32) -> Result<[u8; 16], Stop> {
    let mut iid = [0u8; 16];
    iid.copy_from_slice(ctx.mem.slice(at, 16).map_err(Stop::Fault)?);
    Ok(iid)
}

/// An interface identifier, spelled the way everyone writes it down.
#[must_use]
pub fn iid_text(iid: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([iid[0], iid[1], iid[2], iid[3]]);
    let d2 = u16::from_le_bytes([iid[4], iid[5]]);
    let d3 = u16::from_le_bytes([iid[6], iid[7]]);
    let rest: String = iid[8..].iter().map(|b| format!("{b:02x}")).collect();
    format!("{d1:08x}-{d2:04x}-{d3:04x}-{}-{}", &rest[..4], &rest[4..])
}

/// Which faces of an object this machine knows how to hand back.
///
/// A later version of an interface is a different table over the same thing,
/// so the object handed back shares the state of the one asked. A face that is
/// not here is refused rather than guessed at: a guest that believed a guess
/// would call through a table meaning something else.
const FACES: &[(&str, &str, &str)] = &[
    // DirectX 6's DirectDraw, which is what the roster's titles ask for.
    (
        "IDirectDraw",
        "9c59509a-39bd-11d1-8c4a-00c04fd930c5",
        "IDirectDraw4",
    ),
    (
        "IDirectDraw4",
        "9c59509a-39bd-11d1-8c4a-00c04fd930c5",
        "IDirectDraw4",
    ),
    // DirectX 3's DirectDraw, which a title's movie player asks for: the same
    // slots as the newer one as far as it goes (#9).
    (
        "IDirectDraw4",
        "b3a6f3e0-2b43-11cf-a2de-00aa00b93356",
        "IDirectDraw4",
    ),
    (
        "IDirectDraw",
        "b3a6f3e0-2b43-11cf-a2de-00aa00b93356",
        "IDirectDraw4",
    ),
    // DirectX 5's input device, which a title asks its device for to decide
    // the machine has DirectX 5 or newer. It adds force feedback and Poll
    // after Initialize and agrees slot for slot before that, so one table
    // answers both.
    (
        "IDirectInputDeviceA",
        "5944e682-c92e-11cf-bfc7-444553540000",
        "IDirectInputDeviceA",
    ),
    // DirectX 6's retained mode, which the roster's titles ask for by name.
    (
        "IDirect3DRM",
        "4516ec83-8f20-11d0-9b6d-0000c0781bc3",
        "IDirect3DRM3",
    ),
    (
        "IDirect3DRM3",
        "4516ec83-8f20-11d0-9b6d-0000c0781bc3",
        "IDirect3DRM3",
    ),
    // Its surfaces. DirectX 2's surface and DirectX 6's agree slot for slot as
    // far as the older one goes, so one table answers both: everything through
    // PageUnlock is in the same place, and the newer one only adds after it.
    (
        "IDirectDrawSurface",
        "0b2b8630-ad35-11d0-8ea6-00609797ea5b",
        "IDirectDrawSurface4",
    ),
    (
        "IDirectDrawSurface",
        "6c14db81-a733-11ce-a521-0020af0be560",
        "IDirectDrawSurface4",
    ),
    (
        "IDirectDrawSurface4",
        "6c14db81-a733-11ce-a521-0020af0be560",
        "IDirectDrawSurface4",
    ),
    (
        "IDirectDrawSurface4",
        "0b2b8630-ad35-11d0-8ea6-00609797ea5b",
        "IDirectDrawSurface4",
    ),
    // The device underneath retained mode, asked for as DirectX 6's device,
    // which is what it is (#11).
    (
        "IDirect3DDevice2",
        "b0ab3b60-33d7-11d1-a981-00c04fd7b174",
        "IDirect3DDevice3",
    ),
    (
        "IDirect3DDevice3",
        "b0ab3b60-33d7-11d1-a981-00c04fd7b174",
        "IDirect3DDevice3",
    ),
    // A movie's video stream, asked for as the DirectDraw stream it is (#9).
    (
        "IDirectDrawMediaStream",
        "f4104fce-9a70-11d0-8fde-00c04fd9189d",
        "IDirectDrawMediaStream",
    ),
    // A frame's older face, which a frame array holds, asked for as the frame
    // it is, and the other way (#11).
    (
        "IDirect3DRMFrame",
        "ff6b7f70-a40e-11d1-91f9-0000f8758e66",
        "IDirect3DRMFrame3",
    ),
    (
        "IDirect3DRMFrame3",
        "eb16cb03-d271-11ce-ac48-0000c03825a1",
        "IDirect3DRMFrame",
    ),
    // A surface asked for as the texture Direct3D draws it as (#11).
    (
        "IDirectDrawSurface",
        "93281502-8cf8-11d0-89ab-00a0c9054129",
        "IDirect3DTexture2",
    ),
    (
        "IDirectDrawSurface4",
        "93281502-8cf8-11d0-89ab-00a0c9054129",
        "IDirect3DTexture2",
    ),
    // A sound buffer's 3D faces: the primary buffer's listener, and a
    // secondary buffer's own position (#19).
    (
        "IDirectSoundBuffer",
        "279afa84-4981-11ce-a521-0020af0be560",
        "IDirectSound3DListener",
    ),
    (
        "IDirectSoundBuffer",
        "279afa86-4981-11ce-a521-0020af0be560",
        "IDirectSound3DBuffer",
    ),
    // DirectX 5's surface, which is DirectX 6's without its last nine slots.
    (
        "IDirectDrawSurface",
        "da044e00-69b2-11d0-a1d5-00aa00b8dfbb",
        "IDirectDrawSurface4",
    ),
    (
        "IDirectDrawSurface4",
        "da044e00-69b2-11d0-a1d5-00aa00b8dfbb",
        "IDirectDrawSurface4",
    ),
];

/// Every interface this machine hands out, and the identifier it is asked for
/// by, from each interface's own header.
const OWN: &[(&str, &str)] = &[
    (
        "IAMMultiMediaStream",
        "bebe595c-9a6f-11d0-8fde-00c04fd9189d",
    ),
    ("IDirect3D2", "6aae1ec1-662a-11d0-889d-00aa00bbb76a"),
    ("IDirect3D3", "bb223240-e72b-11d0-a9b4-00aa00c0993e"),
    ("IDirect3DDevice2", "93281501-8cf8-11d0-89ab-00a0c9054129"),
    ("IDirect3DDevice3", "b0ab3b60-33d7-11d1-a981-00c04fd7b174"),
    ("IDirect3DLight", "4417c142-33ad-11cf-816f-0000c020156e"),
    ("IDirect3DMaterial3", "ca9c46f4-d3c5-11d1-b75a-00600852b312"),
    ("IDirect3DRM", "2bc49361-8327-11cf-ac4a-0000c03825a1"),
    ("IDirect3DRM3", "4516ec83-8f20-11d0-9b6d-0000c0781bc3"),
    ("IDirect3DRMDevice3", "549f498b-bfeb-11d1-8ed8-00a0c967a482"),
    ("IDirect3DRMFrame", "eb16cb03-d271-11ce-ac48-0000c03825a1"),
    ("IDirect3DRMFrame3", "ff6b7f70-a40e-11d1-91f9-0000f8758e66"),
    (
        "IDirect3DRMFrameArray",
        "eb16cb12-d271-11ce-ac48-0000c03825a1",
    ),
    ("IDirect3DRMLight", "eb16cb08-d271-11ce-ac48-0000c03825a1"),
    (
        "IDirect3DRMMaterial2",
        "ff6b7f75-a40e-11d1-91f9-0000f8758e66",
    ),
    ("IDirect3DRMMesh", "a3a80d01-6e12-11cf-ac4a-0000c03825a1"),
    (
        "IDirect3DRMMeshBuilder3",
        "4516ec82-8f20-11d0-9b6d-0000c0781bc3",
    ),
    (
        "IDirect3DRMTexture3",
        "ff6b7f73-a40e-11d1-91f9-0000f8758e66",
    ),
    (
        "IDirect3DRMUserVisual",
        "59163de0-6d43-11cf-ac4a-0000c03825a1",
    ),
    (
        "IDirect3DRMViewport2",
        "4a1b1be6-bfed-11d1-8ed8-00a0c967a482",
    ),
    ("IDirect3DTexture2", "93281502-8cf8-11d0-89ab-00a0c9054129"),
    ("IDirect3DViewport3", "b0ab3b61-33d7-11d1-a981-00c04fd7b174"),
    ("IDirectDraw", "6c14db80-a733-11ce-a521-0020af0be560"),
    ("IDirectDraw4", "9c59509a-39bd-11d1-8c4a-00c04fd930c5"),
    ("IDirectDrawClipper", "6c14db85-a733-11ce-a521-0020af0be560"),
    (
        "IDirectDrawMediaStream",
        "f4104fce-9a70-11d0-8fde-00c04fd9189d",
    ),
    ("IDirectDrawPalette", "6c14db84-a733-11ce-a521-0020af0be560"),
    (
        "IDirectDrawStreamSample",
        "f4104fcf-9a70-11d0-8fde-00c04fd9189d",
    ),
    ("IDirectDrawSurface", "6c14db81-a733-11ce-a521-0020af0be560"),
    (
        "IDirectDrawSurface4",
        "0b2b8630-ad35-11d0-8ea6-00609797ea5b",
    ),
    ("IDirectInputA", "89521360-aa8a-11cf-bfc7-444553540000"),
    (
        "IDirectInputDeviceA",
        "5944e680-c92e-11cf-bfc7-444553540000",
    ),
    ("IDirectSound", "279afa83-4981-11ce-a521-0020af0be560"),
    ("IDirectSoundBuffer", "279afa85-4981-11ce-a521-0020af0be560"),
    (
        "IDirectSound3DBuffer",
        "279afa86-4981-11ce-a521-0020af0be560",
    ),
    (
        "IDirectSound3DListener",
        "279afa84-4981-11ce-a521-0020af0be560",
    ),
    ("IMediaStream", "b502d1bd-9a57-11d0-8fde-00c04fd9189d"),
];

/// What every object is: `IUnknown`.
const IUNKNOWN: &str = "00000000-0000-0000-c000-000000000046";

/// What every retained-mode object is too: `IDirect3DRMObject`, and for the
/// kinds that hang on a frame, `IDirect3DRMVisual`.
const RM_OBJECT: &str = "eb16cb00-d271-11ce-ac48-0000c03825a1";
const RM_VISUAL: &str = "eb16cb04-d271-11ce-ac48-0000c03825a1";

/// Whether a face answers to an identifier as itself.
fn is_itself(interface: &str, asked: &str) -> bool {
    // Retained mode's objects all derive from IDirect3DRMObject; the retained
    // mode object itself and a frame array derive from IUnknown alone.
    let retained = matches!(
        interface,
        "IDirect3DRMFrame"
            | "IDirect3DRMFrame3"
            | "IDirect3DRMDevice3"
            | "IDirect3DRMViewport2"
            | "IDirect3DRMMesh"
            | "IDirect3DRMMeshBuilder3"
            | "IDirect3DRMTexture3"
            | "IDirect3DRMLight"
            | "IDirect3DRMMaterial2"
            | "IDirect3DRMUserVisual"
    );
    let visual = matches!(
        interface,
        "IDirect3DRMFrame"
            | "IDirect3DRMFrame3"
            | "IDirect3DRMMesh"
            | "IDirect3DRMMeshBuilder3"
            | "IDirect3DRMTexture3"
            | "IDirect3DRMUserVisual"
    );
    OWN.iter()
        .any(|(name, iid)| *name == interface && *iid == asked)
        || (retained && asked == RM_OBJECT)
        || (visual && asked == RM_VISUAL)
}

fn query_interface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[1];
    if out != 0 {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
    }
    let asked = read_iid(ctx, args[0])
        .map(|iid| iid_text(&iid))
        .unwrap_or_default();
    let Some(have) = ctx.state.interface_of(this) else {
        return Ok(E_NOINTERFACE);
    };

    // IUnknown is the object's identity, the same pointer whichever face is
    // asked: how COM says two pointers are one object.
    if asked == IUNKNOWN {
        let identity = ctx.state.identity(this);
        ctx.state.com_ref(identity, 1);
        if out != 0 {
            ctx.mem.write_u32(out, identity).map_err(Stop::Fault)?;
        }
        return Ok(S_OK);
    }

    // Asked for what it already is: the same face, one more reference.
    if is_itself(have, &asked) {
        ctx.state.com_ref(this, 1);
        if out != 0 {
            ctx.mem.write_u32(out, this).map_err(Stop::Fault)?;
        }
        return Ok(S_OK);
    }

    if let Some((_, _, give)) = FACES
        .iter()
        .find(|(from, iid, _)| *from == have && *iid == asked)
    {
        if let Some(interface) = crate::win32::interface_named(give) {
            let object = crate::win32::State::face(ctx, this, interface).map_err(Stop::Fault)?;
            if out != 0 {
                ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
            }
            ctx.log.at(crate::cli::Trace::Shims, || {
                format!("com {have} handed back {give} at {object:#010x}")
            });
            return Ok(S_OK);
        }
    }

    // A guest asking an object for a face nobody wrote. Answering with the
    // same object would be a guess about what that face does.
    ctx.log
        .line(&format!("com {have} was asked for {asked} and refused"));
    Ok(E_NOINTERFACE)
}

fn add_ref(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com_ref(this, 1))
}

fn release(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com_ref(this, -1))
}

use crate::guest::GuestMem;
