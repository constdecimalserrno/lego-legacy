//! Direct3D's immediate mode.
//!
//! What the roster's titles actually draw through: they take the device
//! underneath retained mode and hand it primitives themselves. Everything it
//! draws lands in the guest's own surface (ADR-0002), so what the guest reads
//! back is what was drawn.
//!
//! Every table is the interface's full width from its header; a slot nobody has
//! called stops the run by name (#11).

use super::scene::IDENTITY;
use super::{ComState, Ctx, State};
use crate::com::{method, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use std::collections::BTreeMap;

/// A handle or state this device was never given.
const DDERR_INVALIDPARAMS: u32 = 0x8007_0057;
/// BeginScene inside a scene, or EndScene outside one.
const D3DERR_SCENE_IN_SCENE: u32 = 0x8876_02f8;
const D3DERR_SCENE_NOT_IN_SCENE: u32 = 0x8876_02f9;

/// What immediate mode keeps about its objects, by the object each face
/// belongs to.
#[derive(Default)]
pub struct Immediate {
    devices: BTreeMap<u32, Device>,
    /// Materials' own bytes, a `D3DMATERIAL` each.
    materials: BTreeMap<u32, Vec<u8>>,
    /// Which material or texture a handle names: one table, as Wine's ddraw
    /// keeps one, read as documentation.
    handles: BTreeMap<u32, u32>,
    next_handle: u32,
    /// Viewports' own descriptions, as the guest last set them.
    viewports: BTreeMap<u32, Vec<u8>>,
}

/// One device's state: everything a title sets and may read back.
#[derive(Default)]
struct Device {
    render: BTreeMap<u32, u32>,
    light: BTreeMap<u32, u32>,
    transforms: BTreeMap<u32, [u32; 16]>,
    textures: BTreeMap<u32, u32>,
    stages: BTreeMap<(u32, u32), u32>,
    current_viewport: u32,
    /// DirectX 7 keeps the viewport on the device rather than in an object of
    /// its own: where on the surface it draws, and the depth range it draws
    /// into (#57).
    viewport7: [u32; 6],
    in_scene: bool,
    direct3d: u32,
    /// The depth buffer, a value a pixel, row by row across the surface. The
    /// guest never locks it, so it is the machine's own rather than guest
    /// memory.
    depth: Vec<f32>,
}

/// `IDirect3DDevice2`: DirectX 5's face of the device underneath retained mode, which is what retained mode hands a title,
/// and what LEGO Island makes itself to hand to retained mode (#117).
pub static DEVICE2: Interface = Interface {
    name: "IDirect3DDevice2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetCaps", 2),
        wall("SwapTextureHandles", 2),
        wall("GetStats", 1),
        method("AddViewport", 1, add_viewport),
        wall("DeleteViewport", 1),
        wall("NextViewport", 3),
        method("EnumTextureFormats", 2, enum_texture_formats2),
        method("BeginScene", 0, begin_scene),
        method("EndScene", 0, end_scene),
        wall("GetDirect3D", 1),
        method("SetCurrentViewport", 1, set_current_viewport),
        method("GetCurrentViewport", 1, get_current_viewport),
        method("SetRenderTarget", 2, set_render_target),
        method("GetRenderTarget", 1, get_render_target),
        wall("Begin", 3),
        wall("BeginIndexed", 5),
        wall("Vertex", 1),
        wall("Index", 1),
        wall("End", 1),
        method("GetRenderState", 2, get_render_state),
        method("SetRenderState", 2, set_render_state),
        method("GetLightState", 2, get_light_state),
        method("SetLightState", 2, set_light_state),
        method("SetTransform", 2, set_transform),
        method("GetTransform", 2, get_transform),
        wall("MultiplyTransform", 2),
        wall("DrawPrimitive", 5),
        method("DrawIndexedPrimitive", 7, draw_indexed_primitive2),
        wall("SetClipStatus", 1),
        wall("GetClipStatus", 1),
    ],
};

/// `IDirect3DDevice3`: DirectX 6's face of the same device, which Rock Raiders
/// draws through.
pub static DEVICE3: Interface = Interface {
    name: "IDirect3DDevice3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("GetCaps", 2),
        wall("GetStats", 1),
        method("AddViewport", 1, add_viewport),
        wall("DeleteViewport", 1),
        wall("NextViewport", 3),
        wall("EnumTextureFormats", 2),
        method("BeginScene", 0, begin_scene),
        method("EndScene", 0, end_scene),
        method("GetDirect3D", 1, get_direct3d3),
        method("SetCurrentViewport", 1, set_current_viewport),
        method("GetCurrentViewport", 1, get_current_viewport),
        method("SetRenderTarget", 2, set_render_target),
        method("GetRenderTarget", 1, get_render_target),
        wall("Begin", 3),
        wall("BeginIndexed", 5),
        wall("Vertex", 1),
        wall("Index", 1),
        wall("End", 1),
        method("GetRenderState", 2, get_render_state),
        method("SetRenderState", 2, set_render_state),
        method("GetLightState", 2, get_light_state),
        method("SetLightState", 2, set_light_state),
        method("SetTransform", 2, set_transform),
        method("GetTransform", 2, get_transform),
        wall("MultiplyTransform", 2),
        method("DrawPrimitive", 5, draw_primitive),
        method("DrawIndexedPrimitive", 7, draw_indexed_primitive),
        wall("SetClipStatus", 1),
        wall("GetClipStatus", 1),
        wall("DrawPrimitiveStrided", 5),
        wall("DrawIndexedPrimitiveStrided", 7),
        wall("DrawPrimitiveVB", 5),
        wall("DrawIndexedPrimitiveVB", 5),
        wall("ComputeSphereVisibility", 5),
        method("GetTexture", 2, get_texture),
        method("SetTexture", 2, set_texture),
        method("GetTextureStageState", 3, get_texture_stage_state),
        method("SetTextureStageState", 3, set_texture_stage_state),
        wall("ValidateDevice", 1),
    ],
};

/// `IDirect3D3`: DirectX 6's Direct3D object: what makes materials, lights and viewports for a device.
pub static D3D3: Interface = Interface {
    name: "IDirect3D3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("EnumDevices", 2, enum_devices3),
        method("CreateLight", 2, create_light),
        method("CreateMaterial", 2, create_material),
        method("CreateViewport", 2, create_viewport),
        wall("FindDevice", 2),
        wall("CreateDevice", 4),
        method("CreateVertexBuffer", 3, create_vertex_buffer),
        method("EnumZBufferFormats", 3, enum_z_buffer_formats),
        wall("EvictManagedTextures", 0),
    ],
};

/// `IDirect3D2`: DirectX 5's Direct3D object, which comes off the DirectDraw
/// object as LEGO Island asks for it (#117).
pub static D3D2: Interface = Interface {
    name: "IDirect3D2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("EnumDevices", 2, enum_devices2),
        wall("CreateLight", 2),
        // DirectX 5's material, `IDirect3DMaterial2`, is DirectX 6's slot for
        // slot, as d3d.h declares the two, so one table answers both. LEGO
        // Chess's DirectX plugin makes one for each texture (#210).
        method("CreateMaterial", 2, create_material),
        // DirectX 5's viewport, `IDirect3DViewport2`, is DirectX 6's slot for
        // slot as far as it goes, as d3d.h declares the two, so the one table
        // answers both. LEGO Chess's DirectX plugin makes one for its device
        // (#148).
        method("CreateViewport", 2, create_viewport),
        wall("FindDevice", 2),
        method("CreateDevice", 3, create_device2),
    ],
};

/// `IDirect3DViewport3`: the part of a device a title clears and draws into.
pub static VIEWPORT3: Interface = Interface {
    name: "IDirect3DViewport3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Initialize", 1),
        method("GetViewport", 1, get_viewport),
        method("SetViewport", 1, set_viewport),
        wall("TransformVertices", 4),
        wall("LightElements", 2),
        method("SetBackground", 1, set_background),
        method("GetBackground", 2, get_background),
        wall("SetBackgroundDepth", 1),
        wall("GetBackgroundDepth", 2),
        method("Clear", 3, clear),
        wall("AddLight", 1),
        wall("DeleteLight", 1),
        wall("NextLight", 3),
        method("GetViewport2", 1, get_viewport),
        method("SetViewport2", 1, set_viewport),
        wall("SetBackgroundDepth2", 1),
        wall("GetBackgroundDepth2", 2),
        method("Clear2", 6, clear2),
    ],
};

/// `IDirect3DMaterial3`: a surface's colours, known to a device by its handle.
pub static MATERIAL3: Interface = Interface {
    name: "IDirect3DMaterial3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("SetMaterial", 1, set_material),
        method("GetMaterial", 1, get_material),
        method("GetHandle", 2, get_handle),
    ],
};

/// `IDirect3DLight`: a light a viewport lights its primitives with.
pub static LIGHT: Interface = Interface {
    name: "IDirect3DLight",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Initialize", 1),
        wall("SetLight", 1),
        wall("GetLight", 1),
    ],
};

/// `IDirect3DTexture2`: a surface a device draws with.
pub static TEXTURE2: Interface = Interface {
    name: "IDirect3DTexture2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetHandle", 2, get_handle),
        wall("PaletteChanged", 2),
        method("Load", 1, load_texture),
    ],
};

/// `IDirect3D7`: DirectX 7's Direct3D, which comes off the DirectDraw object
/// rather than being created on its own (#57).
///
/// Where DirectX 6 made lights, materials and viewports, DirectX 7 keeps all
/// three on the device: this interface only enumerates devices and makes them.
pub static D3D7: Interface = Interface {
    name: "IDirect3D7",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("EnumDevices", 2, enum_devices7),
        method("CreateDevice", 3, create_device7),
        method("CreateVertexBuffer", 3, create_vertex_buffer),
        method("EnumZBufferFormats", 3, enum_z_buffer_formats),
        wall("EvictManagedTextures", 0),
    ],
};

/// `IDirect3DVertexBuffer7`: vertices a title fills once and draws from many
/// times (#57).
pub static VERTEX_BUFFER7: Interface = Interface {
    name: "IDirect3DVertexBuffer7",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("Lock", 3, lock_vertex_buffer),
        method("Unlock", 0, unlock_vertex_buffer),
        wall("ProcessVertices", 7),
        method("GetVertexBufferDesc", 1, vertex_buffer_desc),
        wall("Optimize", 2),
        wall("ProcessVerticesStrided", 6),
    ],
};

/// How many bytes one vertex of a format takes, as the pipeline reads them.
fn fvf_stride(fvf: u32) -> u32 {
    u32::try_from(super::pipeline::vertex_size(fvf).unwrap_or(0)).unwrap_or(0)
}

fn create_vertex_buffer(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateVertexBuffer(description, out, flags).
    let at = args[0];
    let fvf = ctx.mem.read_u32(at + 8).map_err(Stop::Fault)?;
    let count = ctx.mem.read_u32(at + 12).map_err(Stop::Fault)?;
    let store = ctx.state.heap.alloc(fvf_stride(fvf) * count.max(1));
    let object = State::make_com(
        ctx,
        &VERTEX_BUFFER7,
        ComState::VertexBuffer { store, fvf, count },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "vertex buffer at {object:#010x}: {count} vertices of {} bytes at {store:#010x}",
        fvf_stride(fvf)
    ));
    Ok(S_OK)
}

/// Where a vertex buffer keeps its vertices, and how they are laid out.
fn buffer_of(ctx: &mut Ctx<'_>, this: u32) -> Option<(u32, u32, u32)> {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::VertexBuffer { store, fvf, count }) => Some((*store, *fvf, *count)),
        _ => None,
    }
}

fn lock_vertex_buffer(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Lock(flags, out pointer, out size). The vertices are guest memory, so
    // locking one hands back where they already are.
    let Some((store, fvf, count)) = buffer_of(ctx, this) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    ctx.mem.write_u32(args[1], store).map_err(Stop::Fault)?;
    if args[2] != 0 {
        ctx.mem
            .write_u32(args[2], fvf_stride(fvf) * count)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn unlock_vertex_buffer(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Nothing was moved to lock it, so there is nothing to put back.
    Ok(S_OK)
}

/// Draw straight out of a buffer the title filled earlier.
fn draw_primitive_vb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DrawPrimitiveVB(type, buffer, first vertex, count, flags).
    let Some((store, fvf, _)) = buffer_of(ctx, args[1]) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let at = store + args[2] * fvf_stride(fvf);
    let indices: Vec<u32> = (0..args[3]).collect();
    draw(
        ctx,
        "IDirect3DDevice7::DrawPrimitiveVB",
        this,
        args[0],
        fvf,
        at,
        args[3],
        &indices,
        args[4],
    )
}

fn draw_indexed_primitive_vb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DrawIndexedPrimitiveVB(type, buffer, first vertex, count, indices,
    // index count, flags): sixteen-bit indices, counted from the first vertex.
    let Some((store, fvf, _)) = buffer_of(ctx, args[1]) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let at = store + args[2] * fvf_stride(fvf);
    let bytes = ctx
        .mem
        .slice(args[4], args[5] as usize * 2)
        .map_err(Stop::Fault)?
        .to_vec();
    let indices: Vec<u32> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u32::from(u16::from_le_bytes(*c)))
        .collect();
    draw(
        ctx,
        "IDirect3DDevice7::DrawIndexedPrimitiveVB",
        this,
        args[0],
        fvf,
        at,
        args[3],
        &indices,
        args[6],
    )
}

fn vertex_buffer_desc(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some((_, fvf, count)) = buffer_of(ctx, this) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let at = args[0];
    ctx.mem.write_u32(at, 16).map_err(Stop::Fault)?;
    ctx.mem.write_u32(at + 4, 0).map_err(Stop::Fault)?;
    ctx.mem.write_u32(at + 8, fvf).map_err(Stop::Fault)?;
    ctx.mem.write_u32(at + 12, count).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// `IDirect3DDevice7`: DirectX 7's device, which Alpha Team draws through.
///
/// Not DirectX 6's device with slots added: the execute-buffer calls are gone,
/// viewports and lights belong to the device rather than to objects of their
/// own, and what is left sits in different places. The whole table is written
/// out because that is what it is (#57).
pub static DEVICE7: Interface = Interface {
    name: "IDirect3DDevice7",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetCaps", 1, get_caps7),
        method("EnumTextureFormats", 2, enum_texture_formats7),
        method("BeginScene", 0, begin_scene),
        method("EndScene", 0, end_scene),
        wall("GetDirect3D", 1),
        wall("SetRenderTarget", 2),
        wall("GetRenderTarget", 1),
        method("Clear", 6, clear7),
        method("SetTransform", 2, set_transform),
        method("GetTransform", 2, get_transform),
        method("SetViewport", 1, set_viewport7),
        wall("MultiplyTransform", 2),
        method("GetViewport", 1, get_viewport7),
        wall("SetMaterial", 1),
        wall("GetMaterial", 1),
        wall("SetLight", 2),
        wall("GetLight", 2),
        method("SetRenderState", 2, set_render_state),
        method("GetRenderState", 2, get_render_state),
        wall("BeginStateBlock", 0),
        wall("EndStateBlock", 1),
        wall("PreLoad", 1),
        wall("DrawPrimitive", 5),
        wall("DrawIndexedPrimitive", 7),
        wall("SetClipStatus", 1),
        wall("GetClipStatus", 1),
        wall("DrawPrimitiveStrided", 5),
        wall("DrawIndexedPrimitiveStrided", 7),
        method("DrawPrimitiveVB", 5, draw_primitive_vb),
        method("DrawIndexedPrimitiveVB", 7, draw_indexed_primitive_vb),
        wall("ComputeSphereVisibility", 5),
        method("GetTexture", 2, get_texture),
        method("SetTexture", 2, set_texture),
        method("GetTextureStageState", 3, get_texture_stage_state),
        method("SetTextureStageState", 3, set_texture_stage_state),
        method("ValidateDevice", 1, validate_device7),
        wall("ApplyStateBlock", 1),
        wall("CaptureStateBlock", 1),
        wall("DeleteStateBlock", 1),
        wall("CreateStateBlock", 2),
        wall("Load", 5),
        wall("LightEnable", 2),
        wall("GetLightEnable", 2),
        wall("SetClipPlane", 2),
        wall("GetClipPlane", 2),
        wall("GetInfo", 3),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &DEVICE2,
    &DEVICE3,
    &DEVICE7,
    &D3D2,
    &D3D3,
    &D3D7,
    &VERTEX_BUFFER7,
    &VIEWPORT3,
    &MATERIAL3,
    &LIGHT,
    &TEXTURE2,
];

/// How big a `D3DDEVICEDESC7` is, and what is where in it.
///
/// Measured against `NiDirect3d.dll`, which copies 0x3b words of one and then
/// reads the device capabilities at zero and the render depth at 0x74 (#57).
const DEVICE_DESC7_BYTES: u32 = 0x100;

/// The declared machine's one Direct3D device, handed to a title's own
/// callback.
///
/// Every number here is a declared value: what this machine's rasteriser can
/// actually do, not what a card of the period could. A capability claimed and
/// then not implemented is a wall in the middle of a frame rather than a
/// device the title passes over.
fn enum_devices7(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // EnumDevices(callback, context). One device, so one call: the machine
    // finishes this call when the guest's callback returns.
    let (callback, context) = (args[0], args[1]);
    let desc = ctx.state.heap.alloc(DEVICE_DESC7_BYTES);
    write_device_desc7(ctx, desc)?;
    let description = ctx.intern("lego-legacy rasteriser").map_err(Stop::Fault)?;
    let name = ctx.intern("Direct3D HAL").map_err(Stop::Fault)?;
    ctx.log
        .line("d3d7 enumerate: one device, and it rasterises");
    if callback != 0 {
        ctx.call_guest = Some((callback, vec![description, name, desc, context]));
    }
    Ok(S_OK)
}

/// The same description, asked of the device the title already made.
fn get_caps7(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    write_device_desc7(ctx, args[0])?;
    Ok(S_OK)
}

/// Write the declared device's description where the guest asked for it.
fn write_device_desc7(ctx: &mut Ctx<'_>, desc: u32) -> Result<(), Stop> {
    ctx.mem
        .write(desc, &vec![0; DEVICE_DESC7_BYTES as usize])
        .map_err(Stop::Fault)?;
    let mut put = |at: u32, value: u32| ctx.mem.write_u32(desc + at, value).map_err(Stop::Fault);

    // A device that rasterises in hardware, which is what a title looks for
    // before it will use one at all. This machine's rasteriser is the
    // hardware: there is nothing else underneath it (ADR-0002).
    const HW_RASTERISATION: u32 = 0x0008_0000;
    put(0x00, HW_RASTERISATION)?;
    // Both capability blocks say how big they are, which is how a caller knows
    // which version of them it is looking at.
    put(0x04, 0x38)?;
    write_triangle_caps(&mut put, 0x3c)?;

    // The screen this machine declares, and a depth buffer to match.
    put(0x74, DDBD_16)?;
    put(0x78, DDBD_16)?;
    // Textures: one pixel up to a thousand a side, which is every texture
    // Alpha Team ships.
    put(0x7c, 1)?;
    put(0x80, 1)?;
    put(0x84, 1024)?;
    put(0x88, 1024)?;
    put(0x8c, 1024)?;
    put(0x94, 1)?; // no anisotropy, which is one sample
    put(0xb8, 1 | (1 << 16))?; // one blend stage, one texture at a time
    put(0xbc, 8)?; // lights lit at once

    // The device this is: a hardware one, named by the identifier every title
    // compares against.
    ctx.mem
        .write(desc + 0xc4, &HAL_DEVICE)
        .map_err(Stop::Fault)?;
    Ok(())
}

/// The declared screen's depth, and the depth buffer's.
const DDBD_16: u32 = 0x0000_0400;

/// The triangle block of a device's description, a `D3DPRIMCAPS` at `at`:
/// what the rasteriser does. Z comparison against a depth buffer, flat and
/// Gouraud shading, textures with modulation and alpha, and the blends the
/// pipeline already mixes with.
fn write_triangle_caps(
    put: &mut impl FnMut(u32, u32) -> Result<(), Stop>,
    at: u32,
) -> Result<(), Stop> {
    put(at, 0x38)?; // dwSize
    put(at + 0x04, 0x0000_0072)?; // dwMiscCaps: a depth mask, and culling either way round
    put(at + 0x08, 0x0000_0030)?; // dwRasterCaps: depth testing, at subpixel accuracy
    put(at + 0x0c, 0x0000_008a)?; // dwZCmpCaps: less, less-or-equal, always
    put(at + 0x10, 0x0000_0033)?; // dwSrcBlendCaps: zero, one, source alpha and its inverse
    put(at + 0x14, 0x0000_0033)?; // dwDestBlendCaps: the same
    put(at + 0x18, 0x0000_0080)?; // dwAlphaCmpCaps: always, which is no alpha test
    put(at + 0x1c, 0x0000_440a)?; // dwShadeCaps: flat and Gouraud in colour, with alpha
    put(at + 0x20, 0x0000_0001)?; // dwTextureCaps: perspective correction
    put(at + 0x24, 0x0000_0001)?; // dwTextureFilterCaps: nearest, which is what it samples
    put(at + 0x28, 0x0000_0002)?; // dwTextureBlendCaps: modulate, less than it draws (#231)
    put(at + 0x2c, 0x0000_0001) // dwTextureAddressCaps: wrap
}

/// How big DirectX 5's `D3DDEVICEDESC` is: through the stipple sizes, before
/// what DirectX 6 added. LEGO Island copies exactly this much of each one
/// it is handed (`LEGO1.DLL` RVA 0x9be90).
const DEVICE_DESC_BYTES: u32 = 0xcc;

/// And DirectX 6's, which goes on to the texture stages: `D3DDEVICEDESC` in
/// Wine's `d3dcaps.h`, through `wMaxSimultaneousTextures`.
const DEVICE_DESC6_BYTES: u32 = 0xfc;

/// DirectX 5's enumeration of the declared machine's one device (#117).
fn enum_devices2(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    enum_devices(ctx, args, DEVICE_DESC_BYTES, "d3d2")
}

/// DirectX 6's, which Rock Raiders' mode dialog lists its devices from. Asked
/// for `IDirect3D3` and refused, it listed "Error: DirectX6 not installed."
/// and went on with no device at all, reading its flags through a null
/// pointer (#132).
fn enum_devices3(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    enum_devices(ctx, args, DEVICE_DESC6_BYTES, "d3d3")
}

/// The declared machine's one device, as DirectX 5 and 6 enumerate it.
///
/// The same device DirectX 7's enumeration offers, described the way those
/// versions describe one, `bytes` of it: its identifier, a description and a
/// name, then what the hardware does and what is emulated. A device whose
/// hardware description has a colour model is a hardware device, and nothing
/// of this one is emulated, so its emulated description is empty.
fn enum_devices(ctx: &mut Ctx<'_>, args: &[u32], bytes: u32, which: &str) -> Result<u32, Stop> {
    // EnumDevices(callback, context). One device, so one call.
    let (callback, context) = (args[0], args[1]);
    let guid = ctx.state.heap.alloc(16);
    ctx.mem.write(guid, &HAL_DEVICE).map_err(Stop::Fault)?;
    let hardware = ctx.state.heap.alloc(bytes);
    write_device_desc(ctx, hardware, bytes)?;
    let emulated = ctx.state.heap.alloc(bytes);
    ctx.mem
        .write(emulated, &vec![0; bytes as usize])
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(emulated, bytes).map_err(Stop::Fault)?;
    let description = ctx.intern("lego-legacy rasteriser").map_err(Stop::Fault)?;
    let name = ctx.intern("Direct3D HAL").map_err(Stop::Fault)?;
    ctx.log
        .line(&format!("{which} enumerate: one device, and it rasterises"));
    if callback != 0 {
        ctx.call_guest = Some((
            callback,
            vec![guid, description, name, hardware, emulated, context],
        ));
    }
    Ok(S_OK)
}

/// Write the declared device's DirectX 5 or 6 description, `bytes` long,
/// where the guest asked for it: every field it fills says so in `dwFlags`.
fn write_device_desc(ctx: &mut Ctx<'_>, desc: u32, bytes: u32) -> Result<(), Stop> {
    ctx.mem
        .write(desc, &vec![0; bytes as usize])
        .map_err(Stop::Fault)?;
    let mut put = |at: u32, value: u32| ctx.mem.write_u32(desc + at, value).map_err(Stop::Fault);
    put(0x00, bytes)?;
    // D3DDD_COLORMODEL through D3DDD_DEVICEZBUFFERBITDEPTH: no execute
    // buffer, so neither of its limits.
    put(0x04, 0x0000_01ff)?;
    put(0x08, 2)?; // D3DCOLOR_RGB, which is what makes it a hardware device
                   // dtcTransformCaps and bClipping: the pipeline clips to the view volume
                   // as it transforms.
    put(0x10, 8)?;
    put(0x14, 1)?;
    put(0x18, 1)?;
    put(0x1c, 0x10)?; // dlcLightingCaps' size
    put(0x1c + 0x04, 0x7)?; // point, spot and directional lights
    put(0x1c + 0x08, 1)?; // D3DLIGHTINGMODEL_RGB
    put(0x1c + 0x0c, 8)?; // lights lit at once, as DirectX 7's says
    put(0x2c, 0x38)?; // dpcLineCaps: its size, and no lines
    write_triangle_caps(&mut put, 0x64)?;
    put(0x9c, DDBD_16)?;
    put(0xa0, DDBD_16)?;
    // Textures: one pixel up to a thousand a side, as DirectX 7's says.
    put(0xac, 1)?;
    put(0xb0, 1)?;
    put(0xb4, 1024)?;
    put(0xb8, 1024)?;
    if bytes < DEVICE_DESC6_BYTES {
        return Ok(());
    }
    // What DirectX 6 added, as DirectX 7's description says it: a texture
    // repeats as far as it is wide, one sample with no anisotropy, and one
    // blend stage with one texture in it.
    put(0xcc, 1024)?;
    put(0xd4, 1)?;
    put(0xf8, 1 | (1 << 16))
}

/// The depth buffers the declared machine offers, which is one: sixteen bits
/// deep and no stencil, matching the screen it declares (#57).
fn enum_z_buffer_formats(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // EnumZBufferFormats(device, callback, context). A DDPIXELFORMAT, which is
    // the same thirty-two bytes a surface's format is described by.
    let (callback, context) = (args[1], args[2]);
    let format = ctx.state.heap.alloc(32);
    ctx.mem.write(format, &[0u8; 32]).map_err(Stop::Fault)?;
    let mut put = |at: u32, value: u32| ctx.mem.write_u32(format + at, value).map_err(Stop::Fault);
    put(0x00, 32)?; // how big it says it is
    put(0x04, super::ddraw::DDPF_ZBUFFER)?; // what kind of format this is
    put(0x0c, 16)?; // bits of depth
    put(0x14, 0x0000_ffff)?; // and which of them the depth is in
    ctx.log.line("d3d7 z buffers: one, sixteen bits deep");
    if callback != 0 {
        ctx.call_guest = Some((callback, vec![format, context]));
    }
    Ok(S_OK)
}

/// Whether the device can draw what it has been set up to draw, in one pass.
///
/// It can: this machine's rasteriser has one texture stage and no fixed
/// pipeline underneath it to run out of, so there is never a second pass to
/// report (#57).
fn validate_device7(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], 1).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// The texture formats the declared machine's rasteriser can read.
///
/// Each is one this machine actually samples: a title picks the closest to
/// what its own data holds and converts into it, so claiming one that is not
/// read would come back as a wrong-coloured texture rather than as a wall
/// (#57).
fn texture_formats() -> [crate::raster::PixelFormat; 6] {
    let format = |bits, masks| crate::raster::PixelFormat { bits, masks };
    [
        // Sixteen bits: five-six-five, and the two with alpha in them.
        format(16, [0xf800, 0x07e0, 0x001f, 0]),
        format(16, [0x7c00, 0x03e0, 0x001f, 0x8000]),
        format(16, [0x0f00, 0x00f0, 0x000f, 0xf000]),
        // Thirty-two, with and without.
        format(32, [0x00ff_0000, 0xff00, 0xff, 0]),
        format(32, [0x00ff_0000, 0xff00, 0xff, 0xff00_0000]),
        // And a byte an entry, looked up in the surface's own palette.
        format(8, [0, 0, 0, 0]),
    ]
}

/// DirectX 7's, each format a `DDPIXELFORMAT`.
fn enum_texture_formats7(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // EnumTextureFormats(callback, context).
    if args[0] == 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    let mut calls = Vec::new();
    for format in texture_formats() {
        let at = ctx.state.heap.alloc(32);
        super::ddraw::write_pixel_format(ctx, at, format)?;
        calls.push(vec![at, args[1]]);
    }
    ctx.log
        .line(&format!("d3d7 texture formats: {} of them", calls.len()));
    super::call_each(ctx, args[0], &calls, true)?;
    Ok(S_OK)
}

/// DirectX 5's, the same formats each in a whole `DDSURFACEDESC`, as
/// d3dtypes.h has `LPD3DENUMTEXTUREFORMATSCALLBACK` take one, filled as Wine's
/// `d3d_device2_EnumTextureFormats`, read as documentation, fills it: its
/// size, flags saying it holds a pixel format and caps, the format at 72 and
/// a texture's caps at 104. LEGO Chess's DirectX plugin lists them as it makes
/// its device, and draws with five-six-five and one-five-five-five (#148).
fn enum_texture_formats2(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // EnumTextureFormats(callback, context). No callback is refused, as Wine's
    // `device.c` refuses it.
    use super::ddraw::SURFACE_DESC_BYTES;
    const DDSD_CAPS_PIXELFORMAT: u32 = 0x0000_1001;
    const DDSCAPS_TEXTURE: u32 = 0x0000_1000;
    if args[0] == 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    let mut calls = Vec::new();
    for format in texture_formats() {
        let at = ctx.state.heap.alloc(SURFACE_DESC_BYTES);
        ctx.mem
            .write(at, &[0; SURFACE_DESC_BYTES as usize])
            .map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(at, SURFACE_DESC_BYTES)
            .map_err(Stop::Fault)?;
        ctx.mem
            .write_u32(at + 4, DDSD_CAPS_PIXELFORMAT)
            .map_err(Stop::Fault)?;
        super::ddraw::write_pixel_format(ctx, at + 72, format)?;
        ctx.mem
            .write_u32(at + 104, DDSCAPS_TEXTURE)
            .map_err(Stop::Fault)?;
        calls.push(vec![at, args[1]]);
    }
    ctx.log
        .line(&format!("d3d2 texture formats: {} of them", calls.len()));
    super::call_each(ctx, args[0], &calls, true)?;
    Ok(S_OK)
}

/// `IID_IDirect3DHALDevice`, which is what the declared device is.
const HAL_DEVICE: [u8; 16] = [
    0xe0, 0x3d, 0xe6, 0x84, 0xaa, 0x46, 0xcf, 0x11, 0x81, 0x6f, 0x00, 0x00, 0xc0, 0x20, 0x15, 0x6e,
];

/// DirectX 5's device, over the back buffer a title made for it: the one
/// device `EnumDevices` offered, or a wall naming the one asked for (#117).
fn create_device2(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDevice(guid, surface, out).
    let asked = crate::com::read_iid(ctx, args[0])?;
    if asked != HAL_DEVICE {
        return Err(unwritten(format!(
            "IDirect3D2::CreateDevice: device {}, which was never offered",
            crate::com::iid_text(&asked)
        )));
    }
    let surface = args[1];
    let object =
        State::make_com(ctx, &DEVICE2, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    // A DirectX 5 device starts with specular highlights on, as Wine's
    // conformance test `test_lighting_interface_versions` finds Windows
    // starting one (#210).
    device(ctx, object).render.insert(rs::SPECULARENABLE, 1);
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3d2 device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

/// The device a title draws through, over a surface of its own.
fn create_device7(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDevice(guid, surface, out). The surface is the guest's own, which
    // is what makes what the device draws guest-visible.
    let surface = args[1];
    let object =
        State::make_com(ctx, &DEVICE7, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3d7 device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

fn device<'a>(ctx: &'a mut Ctx<'_>, this: u32) -> &'a mut Device {
    device_of(ctx.state, this)
}

fn device_of(state: &mut State, this: u32) -> &mut Device {
    let identity = state.identity(this);
    state.immediate.devices.entry(identity).or_default()
}

/// Set one of a device's transforms, as `SetTransform` would: what retained
/// mode does to the device underneath before a visual draws through it.
pub fn set_device_transform(state: &mut State, device: u32, which: u32, m: &super::scene::Matrix) {
    let cells: Vec<u32> = m.iter().flatten().map(|c| c.to_bits()).collect();
    let mut bits = [0u32; 16];
    bits.copy_from_slice(&cells);
    device_of(state, device).transforms.insert(which, bits);
}

/// Make a viewport the one a device draws through, as `SetCurrentViewport`
/// would.
pub fn set_device_viewport(state: &mut State, device: u32, viewport: u32) {
    device_of(state, device).current_viewport = viewport;
}

fn add_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // A viewport belongs to the device that took it, and draws into that
    // device's surface.
    match ctx.state.com_mut(args[0]).map(|o| &mut o.state) {
        Some(ComState::Viewport { device, .. }) => {
            *device = this;
            Ok(S_OK)
        }
        _ => Ok(DDERR_INVALIDPARAMS),
    }
}

fn set_current_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    device(ctx, this).current_viewport = args[0];
    Ok(S_OK)
}

fn get_current_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let current = device(ctx, this).current_viewport;
    if current != 0 {
        ctx.state.com_ref(current, 1);
    }
    ctx.mem.write_u32(args[0], current).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_render_target(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The surface the device draws into, changed after it was made.
    if let Some(ComState::Device3D { surface }) = ctx.state.com_mut(this).map(|o| &mut o.state) {
        *surface = args[0];
    }
    ctx.log
        .line(&format!("d3d render target {:#010x}", args[0]));
    Ok(S_OK)
}

fn get_render_target(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let surface = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Device3D { surface }) => *surface,
        _ => 0,
    };
    ctx.mem.write_u32(args[0], surface).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn begin_scene(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let device = device(ctx, this);
    if device.in_scene {
        return Ok(D3DERR_SCENE_IN_SCENE);
    }
    device.in_scene = true;
    Ok(S_OK)
}

fn end_scene(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The scene is finished on the device's surface; the title presents it by
    // flipping, which is where the machine counts a frame.
    let device = device(ctx, this);
    if !device.in_scene {
        return Ok(D3DERR_SCENE_NOT_IN_SCENE);
    }
    device.in_scene = false;
    Ok(S_OK)
}

/// What a render state is before a title sets it, from DirectX 6's own
/// documentation of each state.
fn render_state_at_start(state: u32) -> Option<u32> {
    Some(match state {
        // D3DRENDERSTATE_TEXTUREHANDLE: no texture. ANTIALIAS: none.
        1 | 2 => 0,
        // D3DRENDERSTATE_TEXTUREADDRESS: wrap.
        3 => 1,
        // D3DRENDERSTATE_TEXTUREPERSPECTIVE: on, in DirectX 6.
        4 => 1,
        // D3DRENDERSTATE_WRAPU and WRAPV: off.
        5 | 6 => 0,
        // D3DRENDERSTATE_ZENABLE: on, because retained mode's device has a
        // depth buffer.
        7 => 1,
        // D3DRENDERSTATE_FILLMODE: solid.
        8 => 3,
        // D3DRENDERSTATE_SHADEMODE: Gouraud.
        9 => 2,
        // D3DRENDERSTATE_LINEPATTERN and MONOENABLE: nothing.
        10 | 11 => 0,
        // D3DRENDERSTATE_ROP2: copy the pen. PLANEMASK: every bit.
        12 => 13,
        13 => u32::MAX,
        // D3DRENDERSTATE_ZWRITEENABLE: on.
        14 => 1,
        // D3DRENDERSTATE_ALPHATESTENABLE: off.
        15 => 0,
        // D3DRENDERSTATE_LASTPIXEL: on.
        16 => 1,
        // D3DRENDERSTATE_TEXTUREMAG and TEXTUREMIN: nearest.
        17 | 18 => 1,
        // D3DRENDERSTATE_SRCBLEND and DESTBLEND: one and zero.
        19 => 2,
        20 => 1,
        // D3DRENDERSTATE_TEXTUREMAPBLEND: D3DTBLEND_MODULATE, as Wine's
        // `d3d_device_init`, read as documentation, starts every device before
        // DirectX 7's (#210). DirectX 7's face has no such state, and
        // `get_render_state` declares none for it.
        21 => 2,
        // D3DRENDERSTATE_CULLMODE: counter-clockwise.
        22 => 3,
        // D3DRENDERSTATE_ZFUNC: less or equal.
        23 => 4,
        // D3DRENDERSTATE_ALPHAREF and ALPHAFUNC: zero, always.
        24 => 0,
        25 => 8,
        // D3DRENDERSTATE_DITHERENABLE, ALPHABLENDENABLE, FOGENABLE,
        // SPECULARENABLE, ZVISIBLE, SUBPIXEL, SUBPIXELX, STIPPLEDALPHA: all
        // off. FOGCOLOR: black. FOGTABLEMODE: none. FOGTABLESTART: 0.0.
        26..=36 => 0,
        // D3DRENDERSTATE_FOGTABLEEND and FOGTABLEDENSITY: 1.0.
        37 | 38 => 1.0f32.to_bits(),
        // D3DRENDERSTATE_STIPPLEENABLE, EDGEANTIALIAS, COLORKEYENABLE: off.
        39..=41 => 0,
        // D3DRENDERSTATE_BORDERCOLOR: black.
        43 => 0,
        // D3DRENDERSTATE_TEXTUREADDRESSU and V: wrap.
        44 | 45 => 1,
        // D3DRENDERSTATE_MIPMAPLODBIAS, ZBIAS, RANGEFOGENABLE: nothing.
        46..=48 => 0,
        // D3DRENDERSTATE_ANISOTROPY: one.
        49 => 1,
        // D3DRENDERSTATE_TRANSLUCENTSORTINDEPENDENT, STENCILENABLE: off.
        51 | 52 => 0,
        // D3DRENDERSTATE_STENCILFAIL, ZFAIL, PASS: keep. STENCILFUNC: always.
        53..=55 => 1,
        56 => 8,
        // D3DRENDERSTATE_STENCILREF: zero. STENCILMASK, WRITEMASK: every bit.
        57 => 0,
        58 | 59 => u32::MAX,
        // D3DRENDERSTATE_WRAP0 to WRAP7: off.
        128..=135 => 0,
        // D3DRENDERSTATE_FOGVERTEXMODE: none, which leaves the factor to the
        // vertices themselves (#61).
        140 => 0,
        _ => return None,
    })
}

fn get_render_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let state = args[0];
    // DirectX 7 has no `TEXTUREMAPBLEND`: Wine's `d3d_device7_GetRenderState`
    // refuses it as invalid there. So a DirectX 7 device declares no start
    // for it.
    let dx7 = ctx.state.interface_of(this) == Some(DEVICE7.name);
    let value =
        device(ctx, this).render.get(&state).copied().or_else(|| {
            render_state_at_start(state).filter(|_| !dx7 || state != rs::TEXTUREMAPBLEND)
        });
    let Some(value) = value else {
        ctx.log.line(&format!(
            "d3d render state {state} read before it was set, and what it starts as is not declared"
        ));
        return Err(Stop::NotWritten {
            what: String::new(),
            caller: 0,
        });
    };
    ctx.mem.write_u32(args[1], value).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_render_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Kept, and read back. What each state does to a primitive is the
    // rasteriser's to honour when it draws one.
    device(ctx, this).render.insert(args[0], args[1]);
    Ok(S_OK)
}

fn get_light_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let state = args[0];
    // D3DLIGHTSTATE_MATERIAL, AMBIENT and FOGMODE start at nothing and
    // COLORMODEL at RGB. What the fog's floating-point states start at has
    // not been measured.
    let at_start = match state {
        3 => Some(2),
        1 | 2 | 4 => Some(0),
        _ => None,
    };
    let Some(value) = device(ctx, this).light.get(&state).copied().or(at_start) else {
        ctx.log.line(&format!(
            "d3d light state {state} read before it was set, and what it starts as is not declared"
        ));
        return Err(Stop::NotWritten {
            what: String::new(),
            caller: 0,
        });
    };
    ctx.mem.write_u32(args[1], value).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_light_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    device(ctx, this).light.insert(args[0], args[1]);
    Ok(S_OK)
}

fn set_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetTransform(which, matrix): world, view or projection.
    let mut m = [0u32; 16];
    for (i, cell) in m.iter_mut().enumerate() {
        *cell = ctx
            .mem
            .read_u32(args[1] + 4 * i as u32)
            .map_err(Stop::Fault)?;
    }
    device(ctx, this).transforms.insert(args[0], m);
    Ok(S_OK)
}

fn get_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Every transform starts as the identity.
    let identity: Vec<u32> = IDENTITY.iter().flatten().map(|c| c.to_bits()).collect();
    let m = device(ctx, this)
        .transforms
        .get(&args[0])
        .map_or(identity, |m| m.to_vec());
    for (i, cell) in m.iter().enumerate() {
        ctx.mem
            .write_u32(args[1] + 4 * i as u32, *cell)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn get_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let texture = device(ctx, this)
        .textures
        .get(&args[0])
        .copied()
        .unwrap_or(0);
    if texture != 0 {
        ctx.state.com_ref(texture, 1);
    }
    ctx.mem.write_u32(args[1], texture).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetTexture(stage, texture): the device holds a reference to what it
    // draws with, and lets go of what it drew with before.
    let previous = device(ctx, this).textures.insert(args[0], args[1]);
    if args[1] != 0 {
        ctx.state.com_ref(args[1], 1);
    }
    if let Some(previous) = previous.filter(|p| *p != 0) {
        ctx.state.com_ref(previous, -1);
    }
    Ok(S_OK)
}

/// What a texture stage's state is before a title sets it, from DirectX 6's
/// documentation.
fn stage_state_at_start(state: u32, stage: u32) -> Option<u32> {
    Some(match (state, stage) {
        // D3DTSS_COLOROP: modulate on the first stage, off after it.
        (1, 0) => 4,
        (1, _) => 1,
        // D3DTSS_COLORARG1 and ARG2: the texture, and the current colour.
        (2, _) => 2,
        (3, _) => 1,
        // D3DTSS_ALPHAOP: the first argument on the first stage, off after.
        (4, 0) => 2,
        (4, _) => 1,
        (5, _) => 2,
        (6, _) => 1,
        // D3DTSS_ADDRESSU and ADDRESSV: wrap.
        (13 | 14, _) => 1,
        // D3DTSS_MAGFILTER and MINFILTER: point. MIPFILTER: none.
        (16..=18, _) => 1,
        _ => return None,
    })
}

fn get_texture_stage_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (stage, state) = (args[0], args[1]);
    let value = device(ctx, this)
        .stages
        .get(&(stage, state))
        .copied()
        .or_else(|| stage_state_at_start(state, stage));
    let Some(value) = value else {
        ctx.log.line(&format!(
            "d3d texture stage {stage} state {state} read before it was set, \
             and what it starts as is not declared"
        ));
        return Err(Stop::NotWritten {
            what: String::new(),
            caller: 0,
        });
    };
    ctx.mem.write_u32(args[2], value).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_texture_stage_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    device(ctx, this).stages.insert((args[0], args[1]), args[2]);
    Ok(S_OK)
}

fn get_direct3d3(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The Direct3D object the device came from: one per device, the same
    // every time it is asked for.
    let existing = device(ctx, this).direct3d;
    let object = if existing == 0 {
        let made = State::make_com(ctx, &D3D3, ComState::None).map_err(Stop::Fault)?;
        device(ctx, this).direct3d = made;
        made
    } else {
        ctx.state.com_ref(existing, 1);
        existing
    };
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// `CLASS_E_NOAGGREGATION`: what a Direct3D object is answered when it is
/// asked to make a material, a light or a viewport inside another object,
/// whose outer `IUnknown` DirectX's headers say must be NULL; Wine's
/// `ddraw.c`, read as documentation, refuses it so.
fn aggregated(outer: u32) -> Option<u32> {
    const CLASS_E_NOAGGREGATION: u32 = 0x8004_0110;
    (outer != 0).then_some(CLASS_E_NOAGGREGATION)
}

fn create_material(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateMaterial(out, outer).
    if let Some(refused) = aggregated(args[1]) {
        return Ok(refused);
    }
    let object = State::make_com(ctx, &MATERIAL3, ComState::None).map_err(Stop::Fault)?;
    ctx.state.immediate.materials.insert(object, Vec::new());
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_light(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateLight(out, outer).
    if let Some(refused) = aggregated(args[1]) {
        return Ok(refused);
    }
    let object = State::make_com(ctx, &LIGHT, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateViewport(out, outer).
    if let Some(refused) = aggregated(args[1]) {
        return Ok(refused);
    }
    let object = State::make_com(
        ctx,
        &VIEWPORT3,
        ComState::Viewport {
            device: 0,
            camera: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            background: 0,
            immediate: 0,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_material(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetMaterial(D3DMATERIAL): kept whole, as long as it says it is.
    let size = ctx
        .mem
        .read_u32(args[0])
        .map_err(Stop::Fault)?
        .clamp(4, 0x100);
    let bytes = ctx
        .mem
        .slice(args[0], size as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    let identity = ctx.state.identity(this);
    ctx.state.immediate.materials.insert(identity, bytes);
    Ok(S_OK)
}

fn get_material(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let bytes = ctx
        .state
        .immediate
        .materials
        .get(&identity)
        .cloned()
        .unwrap_or_default();
    ctx.mem.write(args[0], &bytes).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// GetHandle(device, out), a material's or a texture's: the name a device's
/// states use for it. One each whichever device asks, from one, in the order
/// they are asked for, from the one table, as Wine's
/// `d3d_texture2_GetHandle`, read as documentation, gives a texture its own
/// (#210).
fn get_handle(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let immediate = &mut ctx.state.immediate;
    let handle = match immediate.handles.iter().find(|(_, m)| **m == identity) {
        Some((handle, _)) => *handle,
        None => {
            immediate.next_handle += 1;
            immediate.handles.insert(immediate.next_handle, identity);
            immediate.next_handle
        }
    };
    ctx.mem.write_u32(args[1], handle).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// `D3DERR_TEXTURE_LOAD_FAILED`.
const D3DERR_TEXTURE_LOAD_FAILED: u32 = 0x8876_02d5;

/// Load(source): fill a texture from another the same size, which is what a
/// `DDSCAPS_ALLOCONLOAD` texture in video memory is made for. As Wine's
/// `d3d_texture2_Load` and its conformance test `test_texture_load_ckey` find
/// Windows doing it: the pixels are copied, the source's colour key replaces
/// the destination's and none leaves it be, `ALLOCONLOAD` is no longer
/// claimed, and another size is `D3DERR_TEXTURE_LOAD_FAILED`. The pixels go
/// through a copy, as `memmove` copies, so two textures over the same memory
/// load as Wine's `memcpy` over one buffer leaves it. Another pixel format, a
/// palette to copy, and mipmaps, whose levels Wine loads one by one, are
/// walls. LEGO Chess's DirectX plugin loads every texture it draws this way
/// (#210).
fn load_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    use super::ddraw::ddscaps::{ALLOCONLOAD, MIPMAP};
    let surface = |ctx: &Ctx<'_>, at: u32| match ctx.state.com(at).map(|o| &o.state) {
        Some(ComState::Surface {
            pixels,
            width,
            height,
            pitch,
            format,
            palette,
            caps,
            ..
        }) => Some((*pixels, *width, *height, *pitch, *format, *palette, *caps)),
        _ => None,
    };
    if ctx.state.identity(args[0]) == ctx.state.identity(this) {
        return Ok(S_OK);
    }
    let (Some(to), Some(from)) = (surface(ctx, this), surface(ctx, args[0])) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let (
        (into, width, height, into_pitch, format, _, into_caps),
        (out_of, w, h, pitch, f, palette, caps),
    ) = (to, from);
    if (w, h) != (width, height) {
        return Ok(D3DERR_TEXTURE_LOAD_FAILED);
    }
    if f != format || palette != 0 || (caps | into_caps) & MIPMAP != 0 {
        return Err(unwritten(format!(
            "IDirect3DTexture2::Load: a {f:?} texture, caps {caps:#x}, palette {palette:#x}, \
             into a {format:?} one, caps {into_caps:#x}"
        )));
    }
    let row = (width * format.bits.div_ceil(8)) as usize;
    let span = |pitch: u32| pitch as usize * (height.max(1) - 1) as usize + row;
    let texels = ctx
        .mem
        .slice(out_of, span(pitch))
        .map_err(Stop::Fault)?
        .to_vec();
    let target = ctx
        .mem
        .slice_mut(into, span(into_pitch))
        .map_err(Stop::Fault)?;
    for y in 0..height as usize {
        let (from, to) = (y * pitch as usize, y * into_pitch as usize);
        target[to..to + row].copy_from_slice(&texels[from..from + row]);
    }
    if let Some(key) = ctx.state.color_keys.get(&out_of).copied() {
        ctx.state.color_keys.insert(into, key);
    }
    if let Some(ComState::Surface { caps, .. }) = ctx.state.com_mut(this).map(|o| &mut o.state) {
        *caps &= !ALLOCONLOAD;
    }
    Ok(S_OK)
}

/// A viewport description, `D3DVIEWPORT` or `D3DVIEWPORT2`: size, then x, y,
/// width and height in the same place in both.
fn store_viewport(ctx: &mut Ctx<'_>, this: u32, at: u32) -> Result<u32, Stop> {
    let size = ctx.mem.read_u32(at).map_err(Stop::Fault)?.clamp(20, 0x40);
    let bytes = ctx
        .mem
        .slice(at, size as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    let field = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    let (x, y, w, h) = (field(4), field(8), field(12), field(16));
    let identity = ctx.state.identity(this);
    ctx.state.immediate.viewports.insert(identity, bytes);
    ctx.log.line(&format!("d3d viewport {w}x{h} at {x},{y}"));
    Ok(S_OK)
}

fn fetch_viewport(ctx: &mut Ctx<'_>, this: u32, at: u32) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let Some(bytes) = ctx.state.immediate.viewports.get(&identity).cloned() else {
        // D3DERR_VIEWPORTDATANOTSET.
        return Ok(0x8876_0305);
    };
    ctx.mem.write(at, &bytes).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    store_viewport(ctx, this, args[0])
}

fn get_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    fetch_viewport(ctx, this, args[0])
}

/// `D3DVIEWPORT7`: where on the surface the device draws, and the depth range
/// it draws into. Six words, and no size field to say which version it is.
fn set_viewport7(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let mut fields = [0u32; 6];
    for (i, field) in fields.iter_mut().enumerate() {
        *field = ctx
            .mem
            .read_u32(args[0] + u32::try_from(i).unwrap_or(0) * 4)
            .map_err(Stop::Fault)?;
    }
    device(ctx, this).viewport7 = fields;
    ctx.log.line(&format!(
        "d3d7 viewport {}x{} at {},{}",
        fields[2], fields[3], fields[0], fields[1]
    ));
    Ok(S_OK)
}

fn get_viewport7(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let fields = device(ctx, this).viewport7;
    for (i, field) in fields.iter().enumerate() {
        ctx.mem
            .write_u32(args[0] + u32::try_from(i).unwrap_or(0) * 4, *field)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn set_background(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetBackground(material handle): the viewport clears to that material's
    // colour.
    match ctx.state.com_mut(this).map(|o| &mut o.state) {
        Some(ComState::Viewport { background, .. }) => {
            *background = args[0];
            Ok(S_OK)
        }
        _ => Ok(DDERR_INVALIDPARAMS),
    }
}

fn get_background(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetBackground(handle, valid).
    let handle = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Viewport { background, .. }) => *background,
        _ => 0,
    };
    ctx.mem.write_u32(args[0], handle).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(args[1], u32::from(handle != 0))
        .map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// A material's diffuse colour as an opaque `D3DCOLOR`.
fn material_colour(ctx: &Ctx<'_>, handle: u32) -> u32 {
    let Some(bytes) = ctx
        .state
        .immediate
        .handles
        .get(&handle)
        .and_then(|m| ctx.state.immediate.materials.get(m))
    else {
        return 0;
    };
    let channel = |o: usize| {
        bytes
            .get(o..o + 4)
            .map_or(0.0, |b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    0xff00_0000
        | (super::component(channel(4)) << 16)
        | (super::component(channel(8)) << 8)
        | super::component(channel(12))
}

/// Clear rectangles of a viewport's device: the surface to a colour, the
/// depth buffer to a depth, or both.
fn clear_rects(
    ctx: &mut Ctx<'_>,
    method: &str,
    this: u32,
    count: u32,
    rects: u32,
    colour: Option<u32>,
    depth_value: Option<f32>,
) -> Result<u32, Stop> {
    let Some(ComState::Viewport { device, .. }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    clear_on(ctx, method, device, count, rects, colour, depth_value)
}

/// Clear a device's own surface, which is what both versions of the call come
/// down to: DirectX 6 asks a viewport and DirectX 7 asks the device (#57).
fn clear_on(
    ctx: &mut Ctx<'_>,
    method: &str,
    device: u32,
    count: u32,
    rects: u32,
    colour: Option<u32>,
    depth_value: Option<f32>,
) -> Result<u32, Stop> {
    let surface = match ctx.state.com(device).map(|o| &o.state) {
        Some(ComState::Device3D { surface }) => *surface,
        // D3DERR_VIEWPORTHASNODEVICE: never added to a device.
        _ => return Ok(0x8876_0306),
    };
    let Some(ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    if colour.is_some() && format.masks == [0; 4] {
        return Err(Stop::NotWritten {
            what: format!("{method}: an indexed surface"),
            caller: 0,
        });
    }
    let mut depth = depth_value.map(|_| take_depth(ctx.state, device, width, height));
    // No rectangles at all is the whole of it, which is how DirectX 7's own
    // call is usually made.
    for i in 0..count.max(1) {
        // D3DRECT: x1, y1, x2, y2.
        let at = rects + i * 16;
        let read = |o: u32| ctx.mem.read_u32(at + o).map_err(Stop::Fault);
        let (x1, y1) = if count == 0 {
            (0, 0)
        } else {
            (read(0)?.min(width), read(4)?.min(height))
        };
        let (x2, y2) = if count == 0 {
            (width, height)
        } else {
            (read(8)?.min(width).max(x1), read(12)?.min(height).max(y1))
        };
        if let Some(colour) = colour {
            let [b, g, r, a] = colour.to_le_bytes();
            let value = format.encode([r, g, b, a]);
            super::fill(ctx, (pixels, pitch, format.bits), (x1, y1, x2, y2), value)?;
        }
        if let (Some(z), Some(buffer)) = (depth_value, depth.as_mut()) {
            for y in y1..y2 {
                let row = (y * width) as usize;
                buffer[row + x1 as usize..row + x2 as usize].fill(z);
            }
        }
    }
    if let Some(buffer) = depth {
        device_of(ctx.state, device).depth = buffer;
    }
    Ok(S_OK)
}

/// What a clear may ask for that is written: the target and the depth buffer.
const D3DCLEAR_TARGET: u32 = 0x1;
const D3DCLEAR_ZBUFFER: u32 = 0x2;

fn clear(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear(count, rects, flags): to the background material's colour, and
    // the depth buffer to as far as it goes.
    if args[2] & !(D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DViewport3::Clear: flags {:#x}", args[2]),
            caller: 0,
        });
    }
    let handle = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Viewport { background, .. }) => *background,
        _ => 0,
    };
    let colour = (args[2] & D3DCLEAR_TARGET != 0).then(|| material_colour(ctx, handle));
    let depth = (args[2] & D3DCLEAR_ZBUFFER != 0).then_some(1.0);
    clear_rects(
        ctx,
        "IDirect3DViewport3::Clear",
        this,
        args[0],
        args[1],
        colour,
        depth,
    )
}

/// DirectX 7's clear, which the device answers itself.
fn clear7(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear(count, rects, flags, colour, z, stencil). There is no stencil
    // buffer on the mode Alpha Team asks for, and it does not ask to clear
    // one.
    if args[2] & !(D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DDevice7::Clear: flags {:#x}", args[2]),
            caller: 0,
        });
    }
    let colour = (args[2] & D3DCLEAR_TARGET != 0).then_some(args[3]);
    let depth = (args[2] & D3DCLEAR_ZBUFFER != 0).then(|| f32::from_bits(args[4]));
    clear_on(
        ctx,
        "IDirect3DDevice7::Clear",
        this,
        args[0],
        args[1],
        colour,
        depth,
    )
}

fn clear2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear2(count, rects, flags, colour, z, stencil). There is no stencil
    // buffer on the declared card's mode Rock Raiders asks for.
    if args[2] & !(D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DViewport3::Clear2: flags {:#x}", args[2]),
            caller: 0,
        });
    }
    let colour = (args[2] & D3DCLEAR_TARGET != 0).then_some(args[3]);
    let depth = (args[2] & D3DCLEAR_ZBUFFER != 0).then(|| f32::from_bits(args[4]));
    clear_rects(
        ctx,
        "IDirect3DViewport3::Clear2",
        this,
        args[0],
        args[1],
        colour,
        depth,
    )
}

fn draw_primitive(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DrawPrimitive(type, format, vertices, count, flags).
    let indices: Vec<u32> = (0..args[3]).collect();
    draw(
        ctx,
        "IDirect3DDevice3::DrawPrimitive",
        this,
        args[0],
        args[1],
        args[2],
        args[3],
        &indices,
        args[4],
    )
}

fn draw_indexed_primitive(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    draw_indexed(
        ctx,
        "IDirect3DDevice3::DrawIndexedPrimitive",
        this,
        args[1],
        args,
    )
}

/// DirectX 5's, which names its vertices by a `D3DVERTEXTYPE` where DirectX 6
/// has a flexible format. `D3DVT_TLVERTEX` is laid out as
/// `D3DFVF_TLVERTEX` is (d3dtypes.h): on the screen already, with both
/// colours and one pair of texture coordinates. LEGO Chess's DirectX plugin
/// draws all it draws in them (#210); the other two types are walls.
fn draw_indexed_primitive2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    const D3DVT_TLVERTEX: u32 = 3;
    const D3DFVF_TLVERTEX: u32 = 0x1c4;
    let method = "IDirect3DDevice2::DrawIndexedPrimitive";
    if args[1] != D3DVT_TLVERTEX {
        return Err(unwritten(format!("{method}: vertex type {}", args[1])));
    }
    draw_indexed(ctx, method, this, D3DFVF_TLVERTEX, args)
}

/// DrawIndexedPrimitive(type, vertices' type or format, vertices, count,
/// indices, index count, flags), the vertices in `format`: sixteen-bit
/// indices into the vertices.
fn draw_indexed(
    ctx: &mut Ctx<'_>,
    method: &str,
    this: u32,
    format: u32,
    args: &[u32],
) -> Result<u32, Stop> {
    let bytes = ctx
        .mem
        .slice(args[4], args[5] as usize * 2)
        .map_err(Stop::Fault)?;
    let indices: Vec<u32> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|w| u32::from(u16::from_le_bytes(*w)))
        .collect();
    draw(
        ctx, method, this, args[0], format, args[2], args[3], &indices, args[6],
    )
}

/// `D3DDP_DONOTLIGHT`: the vertices are lit already.
const D3DDP_DONOTLIGHT: u32 = 0x10;

/// The render states a draw reads, by Direct3D's numbers for them.
mod rs {
    pub const TEXTUREHANDLE: u32 = 1;
    pub const TEXTUREADDRESS: u32 = 3;
    pub const WRAPU: u32 = 5;
    pub const WRAPV: u32 = 6;
    pub const ZENABLE: u32 = 7;
    pub const FILLMODE: u32 = 8;
    pub const SHADEMODE: u32 = 9;
    pub const ZWRITEENABLE: u32 = 14;
    pub const ALPHATESTENABLE: u32 = 15;
    pub const TEXTUREMAG: u32 = 17;
    pub const SRCBLEND: u32 = 19;
    pub const DESTBLEND: u32 = 20;
    pub const TEXTUREMAPBLEND: u32 = 21;
    pub const CULLMODE: u32 = 22;
    pub const ZFUNC: u32 = 23;
    pub const ALPHAREF: u32 = 24;
    pub const ALPHAFUNC: u32 = 25;
    pub const ALPHABLENDENABLE: u32 = 27;
    pub const FOGENABLE: u32 = 28;
    pub const SPECULARENABLE: u32 = 29;
    pub const FOGCOLOR: u32 = 34;
    pub const FOGTABLEMODE: u32 = 35;
    pub const COLORKEYENABLE: u32 = 41;
    pub const TEXTUREADDRESSU: u32 = 44;
    pub const TEXTUREADDRESSV: u32 = 45;
    pub const WRAP0: u32 = 128;
    pub const FOGVERTEXMODE: u32 = 140;
}

/// The first texture stage's states a draw reads.
mod tss {
    pub const COLOROP: u32 = 1;
    pub const COLORARG1: u32 = 2;
    pub const COLORARG2: u32 = 3;
    pub const ALPHAOP: u32 = 4;
    pub const ALPHAARG1: u32 = 5;
    pub const ALPHAARG2: u32 = 6;
    pub const ADDRESSU: u32 = 13;
    pub const ADDRESSV: u32 = 14;
    pub const MAGFILTER: u32 = 16;
}

/// `D3DTRANSFORMSTATE_WORLD`, `_VIEW` and `_PROJECTION`.
const WORLD: u32 = 1;
const VIEW: u32 = 2;
const PROJECTION: u32 = 3;

/// `D3DLIGHTSTATE_MATERIAL` and `_AMBIENT`.
const LIGHT_MATERIAL: u32 = 1;
const LIGHT_AMBIENT: u32 = 2;

fn unwritten(what: String) -> Stop {
    Stop::NotWritten { what, caller: 0 }
}

/// A render state's value, set or as it starts; one with no declared start
/// value is a wall rather than a guess.
fn render_state(device: &Device, method: &str, state: u32) -> Result<u32, Stop> {
    device
        .render
        .get(&state)
        .copied()
        .or_else(|| render_state_at_start(state))
        .ok_or_else(|| {
            unwritten(format!(
                "{method}: render state {state}, never set and not declared"
            ))
        })
}

/// The first texture stage's state, set or as it starts.
fn stage_state(device: &Device, method: &str, state: u32) -> Result<u32, Stop> {
    device
        .stages
        .get(&(0, state))
        .copied()
        .or_else(|| stage_state_at_start(state, 0))
        .ok_or_else(|| {
            unwritten(format!(
                "{method}: texture stage state {state}, never set and not declared"
            ))
        })
}

/// A transform as a device keeps it, as a matrix.
fn matrix_of(device: &Device, which: u32) -> super::scene::Matrix {
    device.transforms.get(&which).map_or(IDENTITY, |bits| {
        std::array::from_fn(|r| std::array::from_fn(|c| f32::from_bits(bits[r * 4 + c])))
    })
}

/// How a texture stage combines a texel and the vertex colour, for the
/// arguments a draw here understands: the texture, and the diffuse or current
/// colour, which on the first stage are the same.
fn combine(op: u32, arg1: u32, arg2: u32) -> Option<crate::raster::Combine> {
    use crate::raster::Combine;
    const DIFFUSE: u32 = 0;
    const CURRENT: u32 = 1;
    const TEXTURE: u32 = 2;
    let texture = |a: u32| a == TEXTURE;
    let colour = |a: u32| a == DIFFUSE || a == CURRENT;
    match op {
        // D3DTOP_DISABLE.
        1 => Some(Combine::Diffuse),
        // D3DTOP_SELECTARG1 and SELECTARG2.
        2 | 3 => {
            let a = if op == 2 { arg1 } else { arg2 };
            if texture(a) {
                Some(Combine::Texture)
            } else if colour(a) {
                Some(Combine::Diffuse)
            } else {
                None
            }
        }
        // D3DTOP_MODULATE and ADD, of a texel and a colour.
        4 | 7 if (texture(arg1) && colour(arg2)) || (colour(arg1) && texture(arg2)) => {
            Some(if op == 4 {
                Combine::Modulate
            } else {
                Combine::Add
            })
        }
        _ => None,
    }
}

/// How DirectX 5's `D3DTEXTUREBLEND` makes a pixel's colour and alpha, as
/// Wine's `d3d_device3_SetRenderState` maps each onto the first texture stage:
/// `D3DTBLEND_DECAL` and `D3DTBLEND_COPY` take the texel's,
/// `D3DTBLEND_MODULATE` multiplies the colours and takes the texel's alpha if
/// the texture has alpha and the vertex's if not, and
/// `D3DTBLEND_MODULATEALPHA` (4 in d3dtypes.h) multiplies both, which LEGO
/// Chess's throne room draws with (#210). The others are walls.
fn texture_map_blend(
    blend: u32,
    texture_alpha: bool,
) -> (
    Option<crate::raster::Combine>,
    Option<crate::raster::Combine>,
) {
    use crate::raster::Combine;
    match blend {
        1 | 7 => (Some(Combine::Texture), Some(Combine::Texture)),
        2 if texture_alpha => (Some(Combine::Modulate), Some(Combine::Texture)),
        2 => (Some(Combine::Modulate), Some(Combine::Diffuse)),
        4 => (Some(Combine::Modulate), Some(Combine::Modulate)),
        _ => (None, None),
    }
}

/// How a texture is addressed past its edge: `D3DTADDRESS_WRAP` or `_CLAMP`.
fn clamps(address: u32) -> Option<bool> {
    match address {
        1 => Some(false),
        3 => Some(true),
        _ => None,
    }
}

/// Draw primitives through a device: every vertex through the pipeline, and
/// every triangle into the device's surface.
#[allow(clippy::too_many_arguments)]
fn draw(
    ctx: &mut Ctx<'_>,
    method: &str,
    this: u32,
    kind: u32,
    format: u32,
    vertices_at: u32,
    count: u32,
    indices: &[u32],
    flags: u32,
) -> Result<u32, Stop> {
    use super::pipeline::{self, fvf, Clipped};
    use crate::raster::{Blend, Combine, Compare, States};

    let Some(size) = pipeline::vertex_size(format) else {
        return Err(unwritten(format!("{method}: vertex format {format:#x}")));
    };
    let Some(ComState::Device3D { surface }) = ctx.state.com(this).map(|o| o.state.clone()) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let Some(ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    if pixel_format.masks == [0; 4] {
        return Err(unwritten(format!("{method}: into an indexed surface")));
    }

    // The vertices as the title wrote them.
    let bytes = ctx
        .mem
        .slice(vertices_at, count as usize * size)
        .map_err(Stop::Fault)?;
    let inputs: Vec<pipeline::Input> = bytes
        .chunks_exact(size)
        .map(|v| pipeline::read_vertex(format, v))
        .collect();

    // A DirectX 5 device has no texture stages: what it draws with is the
    // texture its handle names, and how a texel and the colour make a pixel
    // is one state, `D3DRENDERSTATE_TEXTUREMAPBLEND` (#210). On a later
    // device either is a wall, as it was.
    let dx5 = ctx.state.interface_of(this) == Some(DEVICE2.name);
    let by_handle = render_state(device_of(ctx.state, this), method, rs::TEXTUREHANDLE)?;
    let named = match ctx.state.immediate.handles.get(&by_handle) {
        _ if !dx5 || by_handle == 0 => 0,
        Some(&t)
            if matches!(
                ctx.state.com(t).map(|o| &o.state),
                Some(ComState::Surface { .. })
            ) =>
        {
            t
        }
        _ => {
            return Err(unwritten(format!(
                "{method}: texture handle {by_handle}, which names no texture"
            )))
        }
    };

    // What the device draws them with. A state this pipeline does not honour
    // is a wall once a title turns it on, not a frame drawn as if it had not.
    let device = device_of(ctx.state, this);
    let state = |s: u32| render_state(device, method, s);
    let stage = |s: u32| stage_state(device, method, s);
    for (what, on) in [
        ("a texture by handle", !dx5 && by_handle != 0),
        (
            "wrapping across texture edges",
            state(rs::WRAPU)? != 0 || state(rs::WRAPV)? != 0 || state(rs::WRAP0)? != 0,
        ),
        (
            "a texture map blend",
            !dx5 && device.render.contains_key(&rs::TEXTUREMAPBLEND),
        ),
        ("a fill that is not solid", state(rs::FILLMODE)? != 3),
    ] {
        if on {
            return Err(unwritten(format!("{method}: {what}")));
        }
    }
    let world = matrix_of(device, WORLD);
    let clip_from_world =
        super::scene::multiply(&matrix_of(device, VIEW), &matrix_of(device, PROJECTION));
    let clip_from_model = super::scene::multiply(&world, &clip_from_world);
    let viewport = device.current_viewport;
    // DirectX 7 keeps the viewport on the device itself, so there is no object
    // to ask (#57).
    let viewport7 = device.viewport7;
    let texture = if dx5 {
        named
    } else {
        device.textures.get(&0).copied().unwrap_or(0)
    };
    let material_handle = device.light.get(&LIGHT_MATERIAL).copied().unwrap_or(0);
    let ambient = pipeline::colour(device.light.get(&LIGHT_AMBIENT).copied().unwrap_or(0));
    let compare = |value: u32| {
        Compare::from_d3d(value).ok_or_else(|| unwritten(format!("{method}: comparison {value}")))
    };
    let blend = |value: u32| {
        Blend::from_d3d(value).ok_or_else(|| unwritten(format!("{method}: blend factor {value}")))
    };
    // Fog. With neither mode a curve of its own, Direct3D takes each
    // vertex's factor from the alpha its title wrote in the specular colour,
    // which is how NetImmerse fogs Alpha Team's levels (#61).
    let fog = if state(rs::FOGENABLE)? != 0 {
        let (table, vertex) = (state(rs::FOGTABLEMODE)?, state(rs::FOGVERTEXMODE)?);
        if table != 0 || vertex != 0 || format & fvf::SPECULAR == 0 {
            return Err(unwritten(format!(
                "{method}: fog, table mode {table}, vertex mode {vertex}, vertex format {format:#x}"
            )));
        }
        let [r, g, b, _] = pipeline::colour(state(rs::FOGCOLOR)?);
        Some([r, g, b])
    } else {
        None
    };
    let depth_on = state(rs::ZENABLE)? != 0;
    // The magnification filter: the legacy render state when a title set it,
    // the stage's own otherwise. One filter serves for minification too.
    // ponytail: no mip maps and one filter both ways; a title that minifies a
    // detailed texture far away shimmers where the card would have blurred.
    let linear = if device.render.contains_key(&rs::TEXTUREMAG) {
        match state(rs::TEXTUREMAG)? {
            1 | 3 | 4 => false,
            2 | 5 | 6 => true,
            other => return Err(unwritten(format!("{method}: texture filter {other}"))),
        }
    } else {
        match stage(tss::MAGFILTER)? {
            1 => false,
            2 => true,
            other => return Err(unwritten(format!("{method}: magnification filter {other}"))),
        }
    };
    let address = |legacy: u32, own: u32| -> Result<bool, Stop> {
        let value = if device.render.contains_key(&legacy) {
            state(legacy)?
        } else if device.render.contains_key(&rs::TEXTUREADDRESS) {
            state(rs::TEXTUREADDRESS)?
        } else {
            stage(own)?
        };
        clamps(value).ok_or_else(|| unwritten(format!("{method}: texture address {value}")))
    };
    let clamp = [
        address(rs::TEXTUREADDRESSU, tss::ADDRESSU)?,
        address(rs::TEXTUREADDRESSV, tss::ADDRESSV)?,
    ];
    let mut states = States {
        depth_test: if depth_on {
            Some(compare(state(rs::ZFUNC)?)?)
        } else {
            None
        },
        depth_write: depth_on && state(rs::ZWRITEENABLE)? != 0,
        flat: state(rs::SHADEMODE)? == 1,
        colour: Combine::Modulate,
        alpha: Combine::Modulate,
        linear,
        colour_key: state(rs::COLORKEYENABLE)? != 0,
        alpha_test: if state(rs::ALPHATESTENABLE)? != 0 {
            Some((
                compare(state(rs::ALPHAFUNC)?)?,
                (state(rs::ALPHAREF)? & 0xff) as u8,
            ))
        } else {
            None
        },
        blend: if state(rs::ALPHABLENDENABLE)? != 0 {
            Some((blend(state(rs::SRCBLEND)?)?, blend(state(rs::DESTBLEND)?)?))
        } else {
            None
        },
        specular: state(rs::SPECULARENABLE)? != 0,
        fog,
    };
    let cull = state(rs::CULLMODE)?;
    let (op, a1, a2) = (
        stage(tss::COLOROP)?,
        stage(tss::COLORARG1)?,
        stage(tss::COLORARG2)?,
    );
    let (aop, aa1, aa2) = (
        stage(tss::ALPHAOP)?,
        stage(tss::ALPHAARG1)?,
        stage(tss::ALPHAARG2)?,
    );
    let legacy = dx5.then(|| state(rs::TEXTUREMAPBLEND)).transpose()?;
    let texture_alpha = match ctx.state.com(texture).map(|o| &o.state) {
        Some(ComState::Surface { format, .. }) => format.masks[3] != 0,
        _ => true,
    };
    let combines = match legacy {
        Some(blend) => texture_map_blend(blend, texture_alpha),
        None => (combine(op, a1, a2), combine(aop, aa1, aa2)),
    };
    let (Some(colour_combine), Some(alpha_combine)) = combines else {
        return Err(unwritten(match legacy {
            Some(blend) => format!("{method}: texture map blend {blend}"),
            None => format!(
                "{method}: texture stage colour {op} ({a1:#x}, {a2:#x}), alpha {aop} ({aa1:#x}, {aa2:#x})"
            ),
        }));
    };
    states.colour = colour_combine;
    states.alpha = alpha_combine;

    // Where the viewport puts the view volume.
    let place = if viewport7[2] != 0 {
        pipeline::Viewport::whole(viewport7[0], viewport7[1], viewport7[2], viewport7[3])
    } else {
        match ctx.state.com(viewport).map(|o| o.state.clone()) {
            Some(ComState::Viewport {
                x,
                y,
                width: w,
                height: h,
                ..
            }) if !ctx
                .state
                .immediate
                .viewports
                .contains_key(&ctx.state.identity(viewport)) =>
            {
                pipeline::Viewport::whole(x, y, w, h)
            }
            // What LEGO Chess's DirectX plugin needs of a viewport it describes
            // with `SetViewport2` and draws vertices already on the screen
            // through, on its DirectX 5 device (#210): the rectangle they are
            // cut to, which `D3DVIEWPORT` and `D3DVIEWPORT2` both start with.
            // Projecting through one a title described is not written, nor
            // is this on a later device.
            Some(ComState::Viewport { .. })
                if dx5 && format & fvf::POSITION_MASK == fvf::XYZRHW =>
            {
                let bytes = &ctx.state.immediate.viewports[&ctx.state.identity(viewport)];
                let field = |o: usize| {
                    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
                };
                pipeline::Viewport::whole(field(4), field(8), field(12), field(16))
            }
            _ => {
                return Err(unwritten(format!(
                    "{method}: a viewport the title described itself"
                )))
            }
        }
    };

    // The material and the lights, when the vertices are to be lit.
    let untransformed = format & fvf::POSITION_MASK == fvf::XYZ;
    let lighting = untransformed && format & fvf::NORMAL != 0 && flags & D3DDP_DONOTLIGHT == 0;
    if lighting && states.specular {
        return Err(unwritten(format!(
            "{method}: specular highlights on lit vertices"
        )));
    }
    let material = if lighting {
        let bytes = ctx
            .state
            .immediate
            .handles
            .get(&material_handle)
            .and_then(|m| ctx.state.immediate.materials.get(m));
        match bytes.and_then(|b| pipeline::Material::read(b)) {
            Some(m) => Some(m),
            None => {
                return Err(unwritten(format!(
                    "{method}: lit vertices with material handle {material_handle}"
                )))
            }
        }
    } else {
        None
    };
    let lights = super::d3drm::current_lights(ctx.state);

    // Every vertex into clip space, coloured; a vertex already on the screen
    // stays where it is.
    let prepared: Vec<Clipped> = inputs
        .iter()
        .map(|v| {
            let diffuse = match (&material, v.normal) {
                (Some(material), Some(normal)) => {
                    let at = pipeline::project(&world, v.position);
                    let [r, g, b, _] = ambient;
                    pipeline::light(
                        [at[0], at[1], at[2]],
                        super::scene::direct(&world, normal),
                        material,
                        [r, g, b],
                        &lights,
                    )
                }
                _ => v.diffuse.map_or([1.0; 4], pipeline::colour),
            };
            let [sr, sg, sb, sa] = v.specular.map_or([0.0; 4], pipeline::colour);
            Clipped {
                fog: if fog.is_some() { sa } else { 1.0 },
                clip: if untransformed {
                    pipeline::project(&clip_from_model, v.position)
                } else {
                    v.position
                },
                diffuse,
                specular: [sr, sg, sb],
                uv: v.uv,
            }
        })
        .collect();

    // The triangles, by the primitive's kind, wound as the title wound them.
    let corner = |i: u32| {
        indices
            .get(i as usize)
            .and_then(|&n| prepared.get(n as usize))
            .copied()
    };
    let n = indices.len() as u32;
    let triangles: Vec<[u32; 3]> = match kind {
        4 => (0..n / 3).map(|t| [t * 3, t * 3 + 1, t * 3 + 2]).collect(),
        5 => (0..n.saturating_sub(2))
            .map(|t| {
                if t % 2 == 0 {
                    [t, t + 1, t + 2]
                } else {
                    [t + 1, t, t + 2]
                }
            })
            .collect(),
        6 => (1..n.saturating_sub(1)).map(|t| [0, t, t + 1]).collect(),
        other => return Err(unwritten(format!("{method}: primitive type {other}"))),
    };
    let triangles: Vec<[Clipped; 3]> = triangles
        .into_iter()
        .filter_map(|[i, j, k]| Some([corner(i)?, corner(j)?, corner(k)?]))
        .collect();
    let canvas = Canvas {
        pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        place,
    };
    rasterise(
        ctx,
        method,
        this,
        &canvas,
        &triangles,
        untransformed,
        cull,
        (texture, clamp),
        states,
    )?;
    Ok(S_OK)
}

/// Where a draw lands: a device's surface, and the viewport over it.
pub struct Canvas {
    pub pixels: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub format: crate::raster::PixelFormat,
    pub place: super::pipeline::Viewport,
}

/// Triangles in clip space, or already on the screen, into a device's
/// surface through its depth buffer: cut to the view volume, culled as `cull`
/// says, and drawn with the texture surface `texture` addressed as `clamp`
/// says. What immediate mode's draws and retained mode's meshes (#120) both
/// come down to.
#[allow(clippy::too_many_arguments)]
pub fn rasterise(
    ctx: &mut Ctx<'_>,
    method: &str,
    this: u32,
    canvas: &Canvas,
    triangles: &[[super::pipeline::Clipped; 3]],
    untransformed: bool,
    cull: u32,
    (texture, clamp): (u32, [bool; 2]),
    mut states: crate::raster::States,
) -> Result<(), Stop> {
    use super::pipeline;
    use crate::raster::{self, Combine};
    let Canvas {
        pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        place,
    } = *canvas;

    // The texture the first stage samples, in its own format.
    let texture_state = ctx.state.com(texture).map(|o| o.state.clone());
    let texture_info = match texture_state {
        Some(ComState::Surface {
            pixels: at,
            width: tw,
            height: th,
            pitch: tp,
            format: tf,
            palette,
            ..
        }) => {
            let palette = match ctx.state.com(palette).map(|o| &o.state) {
                Some(ComState::Palette { store }) => {
                    let entries = ctx.mem.slice(*store, 256 * 4).map_err(Stop::Fault)?;
                    let mut out = [[0u8; 4]; 256];
                    for (o, e) in out.iter_mut().zip(entries.as_chunks::<4>().0) {
                        *o = [e[0], e[1], e[2], 0xff];
                    }
                    Some(out)
                }
                _ if tf.masks == [0; 4] => {
                    return Err(unwritten(format!(
                        "{method}: an indexed texture with no palette"
                    )))
                }
                _ => None,
            };
            let key = ctx.state.color_keys.get(&at).copied();
            Some((at, tw, th, tp, tf, palette, key))
        }
        _ => None,
    };
    if texture_info.is_none() {
        // With no texture, what a stage takes from it is the colour alone.
        let untextured = |c: Combine| {
            if matches!(c, Combine::Texture | Combine::Modulate | Combine::Add) {
                Combine::Diffuse
            } else {
                c
            }
        };
        states.colour = untextured(states.colour);
        states.alpha = untextured(states.alpha);
    }
    // Into the surface, through the depth buffer, reading the texture where
    // it lies unless it is the surface being drawn into.
    let mut depth = take_depth(ctx.state, this, width, height);
    let into = (pixels, (pitch * height) as usize);
    let from = texture_info.map_or((0, 0), |(at, _, th, tp, ..)| (at, (tp * th) as usize));
    let apart =
        from.0 as usize >= into.0 as usize + into.1 || into.0 as usize >= from.0 as usize + from.1;
    let (texels, target_pixels): (std::borrow::Cow<'_, [u8]>, &mut [u8]) = if apart {
        let (texels, target) = ctx.mem.read_and_write(from, into).map_err(Stop::Fault)?;
        (std::borrow::Cow::Borrowed(texels), target)
    } else {
        let texels = ctx.mem.slice(from.0, from.1).map_err(Stop::Fault)?.to_vec();
        let target = ctx.mem.slice_mut(into.0, into.1).map_err(Stop::Fault)?;
        (std::borrow::Cow::Owned(texels), target)
    };
    let texture = texture_info
        .as_ref()
        .map(|(_, tw, th, tp, tf, palette, key)| raster::Texture {
            bytes: &texels,
            width: *tw,
            height: *th,
            pitch: *tp,
            format: *tf,
            palette: palette.as_ref(),
            colour_key: *key,
            clamp,
        });
    let mut target = raster::Target {
        pixels: target_pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        depth: &mut depth,
        clip: [
            place.x as i32,
            place.y as i32,
            (place.x + place.width) as i32,
            (place.y + place.height) as i32,
        ],
        first_row: 0,
    };
    let mut list = Vec::new();
    for &[a, mut b, mut c] in triangles {
        if states.flat {
            // A flat triangle is the first vertex's colour throughout, which
            // clipping must not blend away.
            for v in [&mut b, &mut c] {
                v.diffuse = a.diffuse;
                v.specular = a.specular;
            }
        }
        let polygon: Vec<raster::Vertex> = if untransformed {
            pipeline::clip(&[a, b, c])
                .iter()
                .map(|v| place.place(v))
                .collect()
        } else {
            [a, b, c].iter().map(pipeline::Clipped::on_screen).collect()
        };
        if polygon.len() < 3 {
            continue;
        }
        // Twice the polygon's area on the screen: positive when it is wound
        // clockwise, with y running down.
        let area: f32 = (0..polygon.len())
            .map(|p| {
                let (u, v) = (polygon[p], polygon[(p + 1) % polygon.len()]);
                u.x * v.y - v.x * u.y
            })
            .sum();
        // D3DCULL_CW and D3DCULL_CCW.
        if (cull == 2 && area > 0.0) || (cull == 3 && area < 0.0) {
            continue;
        }
        for p in 1..polygon.len() - 1 {
            list.push([polygon[0], polygon[p], polygon[p + 1]]);
        }
    }
    raster::triangles(&mut target, &list, texture.as_ref(), &states);
    device_of(ctx.state, this).depth = depth;
    Ok(())
}

/// Part of a device's depth buffer to as far as it goes: left, top, right and
/// bottom, on a surface so wide and high.
pub fn clear_depth(state: &mut State, device: u32, edges: [u32; 4], (width, height): (u32, u32)) {
    let [left, top, right, bottom] = edges;
    let (right, bottom) = (right.min(width), bottom.min(height));
    let mut depth = take_depth(state, device, width, height);
    for y in top..bottom {
        let row = (y * width) as usize;
        depth[row + left.min(right) as usize..row + right as usize].fill(1.0);
    }
    device_of(state, device).depth = depth;
}

/// A device's depth buffer, taken out of the device for a draw to use and
/// hand back: the size of its surface, made far the first time it is needed.
fn take_depth(state: &mut State, device: u32, width: u32, height: u32) -> Vec<f32> {
    let buffer = std::mem::take(&mut device_of(state, device).depth);
    let len = (width * height) as usize;
    if buffer.len() == len {
        buffer
    } else {
        vec![1.0; len]
    }
}
