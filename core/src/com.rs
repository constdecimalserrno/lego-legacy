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
    // A movie's video stream, asked for as the DirectDraw stream it is (#9).
    (
        "IDirectDrawMediaStream",
        "f4104fce-9a70-11d0-8fde-00c04fd9189d",
        "IDirectDrawMediaStream",
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

fn query_interface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[1];
    if out != 0 {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
    }
    let asked = read_iid(ctx, args[0])
        .map(|iid| iid_text(&iid))
        .unwrap_or_default();
    let Some(object) = ctx.state.com(this) else {
        return Ok(E_NOINTERFACE);
    };
    let have = object.interface;
    let state = object.state.clone();

    if let Some((_, _, give)) = FACES
        .iter()
        .find(|(from, iid, _)| *from == have && *iid == asked)
    {
        if let Some(interface) = crate::win32::interface_named(give) {
            let object =
                crate::win32::State::make_com(ctx, interface, state).map_err(Stop::Fault)?;
            if out != 0 {
                ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
            }
            ctx.log
                .line(&format!("com {have} handed back {give} at {object:#010x}"));
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
