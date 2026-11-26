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
    method(name, args, unwritten)
}

/// A written method, spelled on one line.
#[must_use]
pub const fn method(name: &'static str, args: u32, run: ComFn) -> Method {
    Method { name, args, run }
}

/// What a method that has nothing to do answers: that it worked.
pub fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
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
/// Reference counting is real. DirectDraw surfaces reclaim their guest-heap
/// pixel buffers, COM faces, and side-table entries once all faces reach zero
/// references (#338); other COM objects stay held until the run ends.
pub const QUERY_INTERFACE: Method = method("QueryInterface", 2, query_interface);
pub const ADD_REF: Method = method("AddRef", 0, add_ref);
pub const RELEASE: Method = method("Release", 0, release);

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

const fn hex_nibble(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => panic!("invalid hex digit in IID"),
    }
}

const fn hex_byte(s: &[u8], i: usize) -> u8 {
    (hex_nibble(s[i]) << 4) | hex_nibble(s[i + 1])
}

/// Parse a canonical `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` GUID string into
/// its 16-byte Windows little-endian binary layout at compile time.
const fn parse_iid(s: &str) -> [u8; 16] {
    let b = s.as_bytes();
    assert!(b.len() == 36);
    assert!(b[8] == b'-' && b[13] == b'-' && b[18] == b'-' && b[23] == b'-');
    [
        hex_byte(b, 6),
        hex_byte(b, 4),
        hex_byte(b, 2),
        hex_byte(b, 0),
        hex_byte(b, 11),
        hex_byte(b, 9),
        hex_byte(b, 16),
        hex_byte(b, 14),
        hex_byte(b, 19),
        hex_byte(b, 21),
        hex_byte(b, 24),
        hex_byte(b, 26),
        hex_byte(b, 28),
        hex_byte(b, 30),
        hex_byte(b, 32),
        hex_byte(b, 34),
    ]
}

/// Which faces of an object this machine knows how to hand back.
///
/// A later version of an interface is a different table over the same thing,
/// so the object handed back shares the state of the one asked. A face that is
/// not here is refused rather than guessed at: a guest that believed a guess
/// would call through a table meaning something else.
const FACES: &[(&str, [u8; 16], &str)] = &[
    // DirectX 6's DirectDraw, which is what the roster's titles ask for.
    (
        "IDirectDraw",
        parse_iid("9c59509a-39bd-11d1-8c4a-00c04fd930c5"),
        "IDirectDraw4",
    ),
    // DirectX 7's DirectDraw, which Alpha Team asks its DirectDraw object for
    // before it will draw at all (#57). The same object underneath; its table
    // agrees with DirectX 6's slot for slot and adds two at the end.
    (
        "IDirectDraw",
        parse_iid("15e65ec0-3b9c-11d2-b92f-00609797ea5b"),
        "IDirectDraw7",
    ),
    (
        "IDirectDraw4",
        parse_iid("15e65ec0-3b9c-11d2-b92f-00609797ea5b"),
        "IDirectDraw7",
    ),
    (
        "IDirectDraw7",
        parse_iid("15e65ec0-3b9c-11d2-b92f-00609797ea5b"),
        "IDirectDraw7",
    ),
    // DirectX 7's Direct3D, which comes off the DirectDraw object rather than
    // being made on its own.
    (
        "IDirectDraw7",
        parse_iid("f5049e77-4861-11d2-a407-00a0c90629a8"),
        "IDirect3D7",
    ),
    (
        "IDirectDraw4",
        parse_iid("f5049e77-4861-11d2-a407-00a0c90629a8"),
        "IDirect3D7",
    ),
    // DirectX 6's, which Rock Raiders' mode dialog asks its DirectDraw object
    // for, as Windows 98 with DirectX 6 hands it back (#132). Any DirectDraw
    // face on the same object answers QueryInterface for IDirect3D3 (#370).
    (
        "IDirectDraw",
        parse_iid("bb223240-e72b-11d0-a9b4-00aa00c0993e"),
        "IDirect3D3",
    ),
    (
        "IDirectDraw4",
        parse_iid("bb223240-e72b-11d0-a9b4-00aa00c0993e"),
        "IDirect3D3",
    ),
    // DirectX 5's Direct3D, which comes off the DirectDraw object the same
    // way, and which LEGO Island asks its DirectDraw object for (#117).
    (
        "IDirectDraw",
        parse_iid("6aae1ec1-662a-11d0-889d-00aa00bbb76a"),
        "IDirect3D2",
    ),
    // And off the `IDirectDraw2` face a DirectX 5 program holds, which is
    // this object's `IDirectDraw4` face: any interface must be able to query
    // for any other interface on an object (COM's rules for QueryInterface),
    // and Wine's ddraw, read as documentation, hands Direct3D 2 out from
    // every DirectDraw face. LEGO Chess's DirectX plugin asks this one for
    // it, and calls through what it is given without looking (#148).
    (
        "IDirectDraw4",
        parse_iid("6aae1ec1-662a-11d0-889d-00aa00bbb76a"),
        "IDirect3D2",
    ),
    // DirectX 3's DirectDraw, which a title's movie player asks for: the same
    // slots as the newer one as far as it goes (#9).
    (
        "IDirectDraw4",
        parse_iid("b3a6f3e0-2b43-11cf-a2de-00aa00b93356"),
        "IDirectDraw4",
    ),
    (
        "IDirectDraw",
        parse_iid("b3a6f3e0-2b43-11cf-a2de-00aa00b93356"),
        "IDirectDraw4",
    ),
    // DirectX 5's input device, which a title asks its device for to decide
    // the machine has DirectX 5 or newer. It adds force feedback and Poll
    // after Initialize and agrees slot for slot before that, so one table
    // answers both.
    (
        "IDirectInputDeviceA",
        parse_iid("5944e682-c92e-11cf-bfc7-444553540000"),
        "IDirectInputDeviceA",
    ),
    // DirectX 6's retained mode, which the roster's titles ask for by name.
    (
        "IDirect3DRM",
        parse_iid("4516ec83-8f20-11d0-9b6d-0000c0781bc3"),
        "IDirect3DRM3",
    ),
    // DirectX 5's, which LEGO Island asks for (#117).
    (
        "IDirect3DRM",
        parse_iid("4516ecc8-8f20-11d0-9b6d-0000c0781bc3"),
        "IDirect3DRM2",
    ),
    // Its surfaces. DirectX 2's surface and DirectX 6's agree slot for slot as
    // far as the older one goes, so one table answers both: everything through
    // PageUnlock is in the same place, and the newer one only adds after it.
    (
        "IDirectDrawSurface",
        parse_iid("0b2b8630-ad35-11d0-8ea6-00609797ea5b"),
        "IDirectDrawSurface4",
    ),
    (
        "IDirectDrawSurface4",
        parse_iid("6c14db81-a733-11ce-a521-0020af0be560"),
        "IDirectDrawSurface4",
    ),
    // The device underneath retained mode, asked for as DirectX 6's device,
    // which is what it is (#11).
    (
        "IDirect3DDevice2",
        parse_iid("b0ab3b60-33d7-11d1-a981-00c04fd7b174"),
        "IDirect3DDevice3",
    ),
    // A frame's older face, which a frame array holds, asked for as the frame
    // it is, and the other way (#11).
    (
        "IDirect3DRMFrame",
        parse_iid("ff6b7f70-a40e-11d1-91f9-0000f8758e66"),
        "IDirect3DRMFrame3",
    ),
    (
        "IDirect3DRMFrame3",
        parse_iid("eb16cb03-d271-11ce-ac48-0000c03825a1"),
        "IDirect3DRMFrame",
    ),
    // A retained-mode texture asked for as DirectX 5's face of itself, as
    // LEGO Island asks of a mesh group's texture (#120).
    (
        "IDirect3DRMTexture",
        parse_iid("120f30c0-1629-11d0-941c-0080c80cfa7b"),
        "IDirect3DRMTexture2",
    ),
    // A surface asked for as the texture Direct3D draws it as (#11).
    (
        "IDirectDrawSurface",
        parse_iid("93281502-8cf8-11d0-89ab-00a0c9054129"),
        "IDirect3DTexture2",
    ),
    (
        "IDirectDrawSurface4",
        parse_iid("93281502-8cf8-11d0-89ab-00a0c9054129"),
        "IDirect3DTexture2",
    ),
    // A sound buffer's 3D faces: the primary buffer's listener, and a
    // secondary buffer's own position (#19).
    (
        "IDirectSoundBuffer",
        parse_iid("279afa84-4981-11ce-a521-0020af0be560"),
        "IDirectSound3DListener",
    ),
    (
        "IDirectSoundBuffer",
        parse_iid("279afa86-4981-11ce-a521-0020af0be560"),
        "IDirectSound3DBuffer",
    ),
    // DirectX 5's surface, which is DirectX 6's without its last nine slots.
    (
        "IDirectDrawSurface",
        parse_iid("da044e00-69b2-11d0-a1d5-00aa00b8dfbb"),
        "IDirectDrawSurface4",
    ),
    (
        "IDirectDrawSurface4",
        parse_iid("da044e00-69b2-11d0-a1d5-00aa00b8dfbb"),
        "IDirectDrawSurface4",
    ),
];

/// DirectShow `CLSID_FilterGraph` faces: every interface in this group is
/// symmetrically queryable from every other via its IID in [`OWN`] (#372).
const FILTER_GRAPH_FACES: &[&str] = &[
    "IGraphBuilder",
    "IMediaControl",
    "IMediaEvent",
    "IMediaPosition",
    "IVideoWindow",
];

/// Every interface this machine hands out, and the identifier it is asked for
/// by, from each interface's own header.
const OWN: &[(&str, [u8; 16])] = &[
    (
        "IAMMultiMediaStream",
        parse_iid("bebe595c-9a6f-11d0-8fde-00c04fd9189d"),
    ),
    (
        "IDirect3D2",
        parse_iid("6aae1ec1-662a-11d0-889d-00aa00bbb76a"),
    ),
    (
        "IDirect3D3",
        parse_iid("bb223240-e72b-11d0-a9b4-00aa00c0993e"),
    ),
    (
        "IDirect3DDevice2",
        parse_iid("93281501-8cf8-11d0-89ab-00a0c9054129"),
    ),
    (
        "IDirect3DDevice3",
        parse_iid("b0ab3b60-33d7-11d1-a981-00c04fd7b174"),
    ),
    (
        "IDirect3DLight",
        parse_iid("4417c142-33ad-11cf-816f-0000c020156e"),
    ),
    (
        "IDirect3DMaterial3",
        parse_iid("ca9c46f4-d3c5-11d1-b75a-00600852b312"),
    ),
    // DirectX 5's material, which the same table answers, `IDirect3DMaterial2`
    // being `IDirect3DMaterial3` slot for slot in d3d.h (#210).
    (
        "IDirect3DMaterial3",
        parse_iid("93281503-8cf8-11d0-89ab-00a0c9054129"),
    ),
    (
        "IDirect3DRM",
        parse_iid("2bc49361-8327-11cf-ac4a-0000c03825a1"),
    ),
    (
        "IDirect3DRM2",
        parse_iid("4516ecc8-8f20-11d0-9b6d-0000c0781bc3"),
    ),
    (
        "IDirect3DRM3",
        parse_iid("4516ec83-8f20-11d0-9b6d-0000c0781bc3"),
    ),
    (
        "IDirect3DRMDevice2",
        parse_iid("4516ec78-8f20-11d0-9b6d-0000c0781bc3"),
    ),
    (
        "IDirect3DRMDevice3",
        parse_iid("549f498b-bfeb-11d1-8ed8-00a0c967a482"),
    ),
    (
        "IDirect3DRMFrame",
        parse_iid("eb16cb03-d271-11ce-ac48-0000c03825a1"),
    ),
    (
        "IDirect3DRMFrame2",
        parse_iid("c3dfbd60-3988-11d0-9ec2-0000c0291ac3"),
    ),
    (
        "IDirect3DRMFrame3",
        parse_iid("ff6b7f70-a40e-11d1-91f9-0000f8758e66"),
    ),
    (
        "IDirect3DRMFrameArray",
        parse_iid("eb16cb12-d271-11ce-ac48-0000c03825a1"),
    ),
    (
        "IDirect3DRMLight",
        parse_iid("eb16cb08-d271-11ce-ac48-0000c03825a1"),
    ),
    (
        "IDirect3DRMMaterial2",
        parse_iid("ff6b7f75-a40e-11d1-91f9-0000f8758e66"),
    ),
    (
        "IDirect3DRMMesh",
        parse_iid("a3a80d01-6e12-11cf-ac4a-0000c03825a1"),
    ),
    (
        "IDirect3DRMMeshBuilder3",
        parse_iid("4516ec82-8f20-11d0-9b6d-0000c0781bc3"),
    ),
    (
        "IDirect3DRMTexture",
        parse_iid("eb16cb09-d271-11ce-ac48-0000c03825a1"),
    ),
    (
        "IDirect3DRMTexture2",
        parse_iid("120f30c0-1629-11d0-941c-0080c80cfa7b"),
    ),
    (
        "IDirect3DRMTexture3",
        parse_iid("ff6b7f73-a40e-11d1-91f9-0000f8758e66"),
    ),
    (
        "IDirect3DRMUserVisual",
        parse_iid("59163de0-6d43-11cf-ac4a-0000c03825a1"),
    ),
    (
        "IDirect3DRMViewport",
        parse_iid("eb16cb02-d271-11ce-ac48-0000c03825a1"),
    ),
    (
        "IDirect3DRMViewport2",
        parse_iid("4a1b1be6-bfed-11d1-8ed8-00a0c967a482"),
    ),
    (
        "IDirect3DTexture2",
        parse_iid("93281502-8cf8-11d0-89ab-00a0c9054129"),
    ),
    (
        "IDirect3DViewport3",
        parse_iid("b0ab3b61-33d7-11d1-a981-00c04fd7b174"),
    ),
    // DirectX 5's and DirectX 3's viewports, which the same table answers, as
    // Wine's `viewport.c`, read as documentation, answers all three (#148).
    (
        "IDirect3DViewport3",
        parse_iid("93281500-8cf8-11d0-89ab-00a0c9054129"),
    ),
    (
        "IDirect3DViewport3",
        parse_iid("4417c146-33ad-11cf-816f-0000c020156e"),
    ),
    (
        "IDirectDraw",
        parse_iid("6c14db80-a733-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectDraw4",
        parse_iid("9c59509a-39bd-11d1-8c4a-00c04fd930c5"),
    ),
    (
        "IDirectDrawClipper",
        parse_iid("6c14db85-a733-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectDrawMediaStream",
        parse_iid("f4104fce-9a70-11d0-8fde-00c04fd9189d"),
    ),
    (
        "IDirectDrawPalette",
        parse_iid("6c14db84-a733-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectDrawStreamSample",
        parse_iid("f4104fcf-9a70-11d0-8fde-00c04fd9189d"),
    ),
    (
        "IDirectDrawSurface",
        parse_iid("6c14db81-a733-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectDrawSurface4",
        parse_iid("0b2b8630-ad35-11d0-8ea6-00609797ea5b"),
    ),
    (
        "IDirectInputA",
        parse_iid("89521360-aa8a-11cf-bfc7-444553540000"),
    ),
    (
        "IDirectInputDeviceA",
        parse_iid("5944e680-c92e-11cf-bfc7-444553540000"),
    ),
    (
        "IDirectSound",
        parse_iid("279afa83-4981-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectSoundBuffer",
        parse_iid("279afa85-4981-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectSound3DBuffer",
        parse_iid("279afa86-4981-11ce-a521-0020af0be560"),
    ),
    (
        "IDirectSound3DListener",
        parse_iid("279afa84-4981-11ce-a521-0020af0be560"),
    ),
    (
        "IMediaStream",
        parse_iid("b502d1bd-9a57-11d0-8fde-00c04fd9189d"),
    ),
    (
        "IGraphBuilder",
        parse_iid("56a868a9-0ad4-11ce-b03a-0020af0ba770"),
    ),
    (
        "IMediaControl",
        parse_iid("56a868b1-0ad4-11ce-b03a-0020af0ba770"),
    ),
    (
        "IMediaPosition",
        parse_iid("56a868b2-0ad4-11ce-b03a-0020af0ba770"),
    ),
    (
        "IVideoWindow",
        parse_iid("56a868b4-0ad4-11ce-b03a-0020af0ba770"),
    ),
    (
        "IMediaEvent",
        parse_iid("56a868b6-0ad4-11ce-b03a-0020af0ba770"),
    ),
];

/// What every object is: `IUnknown`.
const IUNKNOWN: [u8; 16] = parse_iid("00000000-0000-0000-c000-000000000046");

/// What every retained-mode object is too: `IDirect3DRMObject`, and for the
/// kinds that hang on a frame, `IDirect3DRMVisual`.
const RM_OBJECT: [u8; 16] = parse_iid("eb16cb00-d271-11ce-ac48-0000c03825a1");
const RM_VISUAL: [u8; 16] = parse_iid("eb16cb04-d271-11ce-ac48-0000c03825a1");

/// Whether a face answers to an identifier as itself.
fn is_itself(interface: &str, asked: &[u8; 16]) -> bool {
    // Retained mode's objects all derive from IDirect3DRMObject; the retained
    // mode object itself and a frame array derive from IUnknown alone.
    let retained = matches!(
        interface,
        "IDirect3DRMFrame"
            | "IDirect3DRMFrame2"
            | "IDirect3DRMFrame3"
            | "IDirect3DRMDevice2"
            | "IDirect3DRMDevice3"
            | "IDirect3DRMViewport"
            | "IDirect3DRMViewport2"
            | "IDirect3DRMMesh"
            | "IDirect3DRMMeshBuilder3"
            | "IDirect3DRMTexture"
            | "IDirect3DRMTexture2"
            | "IDirect3DRMTexture3"
            | "IDirect3DRMLight"
            | "IDirect3DRMMaterial2"
            | "IDirect3DRMUserVisual"
    );
    let visual = matches!(
        interface,
        "IDirect3DRMFrame"
            | "IDirect3DRMFrame2"
            | "IDirect3DRMFrame3"
            | "IDirect3DRMMesh"
            | "IDirect3DRMMeshBuilder3"
            | "IDirect3DRMTexture"
            | "IDirect3DRMTexture2"
            | "IDirect3DRMTexture3"
            | "IDirect3DRMUserVisual"
    );
    OWN.iter()
        .any(|(name, iid)| *name == interface && iid == asked)
        || (retained && *asked == RM_OBJECT)
        || (visual && *asked == RM_VISUAL)
}

fn query_interface(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let out = args[1];
    if out != 0 {
        ctx.mem.write_u32(out, 0).map_err(Stop::Fault)?;
    }
    let asked = read_iid(ctx, args[0]).ok();
    let Some(have) = ctx.state.interface_of(this) else {
        return Ok(E_NOINTERFACE);
    };

    if let Some(asked) = asked {
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

        let target = FACES
            .iter()
            .find(|(from, iid, _)| *from == have && *iid == asked)
            .map(|(_, _, give)| *give)
            .or_else(|| {
                FILTER_GRAPH_FACES.contains(&have).then(|| {
                    OWN.iter()
                        .find(|(name, iid)| *iid == asked && FILTER_GRAPH_FACES.contains(name))
                        .map(|(name, _)| *name)
                })?
            });
        if let Some(give) = target {
            if let Some(interface) = crate::win32::interface_named(give) {
                let object =
                    crate::win32::State::face(ctx, this, interface).map_err(Stop::Fault)?;
                if out != 0 {
                    ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
                }
                ctx.log.at(crate::cli::Trace::Shims, || {
                    format!("com {have} handed back {give} at {object:#010x}")
                });
                return Ok(S_OK);
            }
        }
    }

    let text = asked.map_or_else(String::new, |iid| iid_text(&iid));

    // DirectX 5's own faces: a face a title asks one of them for that is not
    // here yet stops the run by name, rather than sending the title down a
    // path it would not take on Windows (#117).
    if matches!(
        have,
        "IDirect3D2"
            | "IDirect3DRM2"
            | "IDirect3DRMDevice2"
            | "IDirect3DRMFrame2"
            | "IDirect3DRMViewport"
    ) {
        return Err(Stop::NotWritten {
            what: format!("{have}::QueryInterface: {text}"),
            caller: 0,
        });
    }

    // A guest asking an object for a face nobody wrote. Answering with the
    // same object would be a guess about what that face does.
    ctx.log
        .line(&format!("com {have} was asked for {text} and refused"));
    Ok(E_NOINTERFACE)
}

fn add_ref(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com_ref(this, 1))
}

fn release(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com_ref(this, -1))
}

use crate::guest::GuestMem;

#[cfg(test)]
mod tests {
    use super::{
        iid_text, is_itself, parse_iid, query_interface, E_NOINTERFACE, FACES, IUNKNOWN, OWN,
        RM_OBJECT, RM_VISUAL, S_OK,
    };
    use crate::cpu::Stop;
    use crate::guest::GuestMem;
    use crate::win32::tests::with_test_ctx;
    use crate::win32::{ComState, State};

    #[test]
    fn binary_guids_round_trip_with_iid_text() {
        assert_eq!(iid_text(&IUNKNOWN), "00000000-0000-0000-c000-000000000046");
        assert_eq!(iid_text(&RM_OBJECT), "eb16cb00-d271-11ce-ac48-0000c03825a1");
        assert_eq!(iid_text(&RM_VISUAL), "eb16cb04-d271-11ce-ac48-0000c03825a1");
        // Spot-check little-endian Data1/Data2/Data3 + big-endian Data4 byte layout.
        assert_eq!(
            parse_iid("9c59509a-39bd-11d1-8c4a-00c04fd930c5"),
            [
                0x9a, 0x50, 0x59, 0x9c, 0xbd, 0x39, 0xd1, 0x11, 0x8c, 0x4a, 0x00, 0xc0, 0x4f, 0xd9,
                0x30, 0xc5
            ]
        );
        for &(_, iid, _) in FACES {
            let text = iid_text(&iid);
            assert_eq!(parse_iid(&text), iid, "FACES round-trip for {text}");
        }
        for &(name, iid) in OWN {
            let text = iid_text(&iid);
            assert_eq!(parse_iid(&text), iid, "OWN round-trip for {name} {text}");
            assert!(is_itself(name, &iid), "{name} answers to its own IID");
        }
        assert!(is_itself("IDirect3DRMFrame3", &RM_OBJECT));
        assert!(is_itself("IDirect3DRMFrame3", &RM_VISUAL));
        assert!(is_itself("IDirect3DRMLight", &RM_OBJECT));
        assert!(!is_itself("IDirect3DRMLight", &RM_VISUAL));
        assert!(!is_itself("IDirectDraw", &RM_OBJECT));
    }

    #[test]
    fn query_interface_binary_matching_hits_walls_and_refusals() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let iid_at = 0x1000u32;
            let out_at = 0x1040u32;
            let dd = State::make_com(ctx, &crate::win32::ddraw::DIRECT_DRAW, ComState::None)
                .expect("make_com");

            // 1. Face upgrade: IDirectDraw -> IDirectDraw4 and IDirect3D3.
            let dd4_iid = parse_iid("9c59509a-39bd-11d1-8c4a-00c04fd930c5");
            ctx.mem.write(iid_at, &dd4_iid).unwrap();
            assert_eq!(query_interface(ctx, dd, &[iid_at, out_at]).unwrap(), S_OK);
            let dd4 = ctx.mem.read_u32(out_at).unwrap();
            assert_ne!(dd4, dd);
            assert_eq!(ctx.state.interface_of(dd4), Some("IDirectDraw4"));

            let d3d3_iid = parse_iid("bb223240-e72b-11d0-a9b4-00aa00c0993e");
            ctx.mem.write(iid_at, &d3d3_iid).unwrap();
            assert_eq!(query_interface(ctx, dd, &[iid_at, out_at]).unwrap(), S_OK);
            let d3d3 = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!(ctx.state.interface_of(d3d3), Some("IDirect3D3"));

            // 2. IUnknown on secondary face returns primary identity.
            ctx.mem.write(iid_at, &IUNKNOWN).unwrap();
            assert_eq!(query_interface(ctx, dd4, &[iid_at, out_at]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out_at).unwrap(), dd);

            // 3. Self query returns the same face pointer.
            ctx.mem.write(iid_at, &dd4_iid).unwrap();
            assert_eq!(query_interface(ctx, dd4, &[iid_at, out_at]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out_at).unwrap(), dd4);

            // 4. Unknown IID on IDirectDraw refuses with E_NOINTERFACE and clears out.
            let unknown = parse_iid("11223344-5566-7788-99aa-bbccddeeff00");
            ctx.mem.write(iid_at, &unknown).unwrap();
            assert_eq!(
                query_interface(ctx, dd, &[iid_at, out_at]).unwrap(),
                E_NOINTERFACE
            );
            assert_eq!(ctx.mem.read_u32(out_at).unwrap(), 0);

            // 5. Unknown IID on a DirectX 5 face stops at a named wall containing the text IID.
            let d3d2_iid = parse_iid("6aae1ec1-662a-11d0-889d-00aa00bbb76a");
            ctx.mem.write(iid_at, &d3d2_iid).unwrap();
            assert_eq!(query_interface(ctx, dd, &[iid_at, out_at]).unwrap(), S_OK);
            let d3d2 = ctx.mem.read_u32(out_at).unwrap();
            ctx.mem.write(iid_at, &unknown).unwrap();
            let err = query_interface(ctx, d3d2, &[iid_at, out_at]).unwrap_err();
            assert!(
                matches!(err, Stop::NotWritten { ref what, .. } if what.contains("11223344-5566-7788-99aa-bbccddeeff00")),
                "expected DX5 wall naming formatted IID, got {err:?}"
            );

            // 6. Unreadable IID pointer falls through to refusal without panicking.
            assert_eq!(
                query_interface(ctx, dd, &[0xffff_fff0, out_at]).unwrap(),
                E_NOINTERFACE
            );
        });
    }

    #[test]
    fn directshow_filter_graph_queries_all_five_interfaces_symmetrically() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let iid_at = 0x1000u32;
            let out_at = 0x1040u32;
            let graph = crate::win32::dshow::create(ctx, &crate::win32::dshow::GRAPH_BUILDER)
                .expect("create FilterGraph");
            let all = [
                (
                    "IGraphBuilder",
                    parse_iid("56a868a9-0ad4-11ce-b03a-0020af0ba770"),
                ),
                (
                    "IMediaControl",
                    parse_iid("56a868b1-0ad4-11ce-b03a-0020af0ba770"),
                ),
                (
                    "IMediaPosition",
                    parse_iid("56a868b2-0ad4-11ce-b03a-0020af0ba770"),
                ),
                (
                    "IVideoWindow",
                    parse_iid("56a868b4-0ad4-11ce-b03a-0020af0ba770"),
                ),
                (
                    "IMediaEvent",
                    parse_iid("56a868b6-0ad4-11ce-b03a-0020af0ba770"),
                ),
            ];
            let mut ptrs = Vec::new();
            for &(name, iid) in &all {
                ctx.mem.write(iid_at, &iid).unwrap();
                assert_eq!(
                    query_interface(ctx, graph, &[iid_at, out_at]).unwrap(),
                    S_OK
                );
                let face = ctx.mem.read_u32(out_at).unwrap();
                assert_eq!(ctx.state.interface_of(face), Some(name));
                assert_eq!(ctx.state.identity(face), graph);
                ptrs.push((name, face));
            }
            // Symmetric QI from every face to every other face and IUnknown.
            for &(_, from_face) in &ptrs {
                ctx.mem.write(iid_at, &IUNKNOWN).unwrap();
                assert_eq!(
                    query_interface(ctx, from_face, &[iid_at, out_at]).unwrap(),
                    S_OK
                );
                assert_eq!(ctx.mem.read_u32(out_at).unwrap(), graph);
                for &(to_name, to_iid) in &all {
                    ctx.mem.write(iid_at, &to_iid).unwrap();
                    assert_eq!(
                        query_interface(ctx, from_face, &[iid_at, out_at]).unwrap(),
                        S_OK
                    );
                    let got = ctx.mem.read_u32(out_at).unwrap();
                    assert_eq!(ctx.state.interface_of(got), Some(to_name));
                }
            }
        });
    }
}
