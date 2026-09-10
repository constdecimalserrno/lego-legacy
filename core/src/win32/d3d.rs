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
    /// Which material a handle names.
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
    in_scene: bool,
    direct3d: u32,
    /// The depth buffer, a value a pixel, row by row across the surface. The
    /// guest never locks it, so it is the machine's own rather than guest
    /// memory.
    depth: Vec<f32>,
}

/// `IDirect3DDevice2`: DirectX 5's face of the device underneath retained mode, which is what retained mode hands a title.
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
        wall("EnumTextureFormats", 2),
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
        wall("DrawIndexedPrimitive", 7),
        wall("SetClipStatus", 1),
        wall("GetClipStatus", 1),
    ],
};

/// `IDirect3DDevice3`: DirectX 6's face of the same device, which this title draws through.
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

/// `IDirect3D2`: DirectX 5's Direct3D object.
pub static D3D2: Interface = Interface {
    name: "IDirect3D2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("EnumDevices", 2),
        wall("CreateLight", 2),
        wall("CreateMaterial", 2),
        wall("CreateViewport", 2),
        wall("FindDevice", 2),
        wall("CreateDevice", 3),
    ],
};

/// `IDirect3D3`: DirectX 6's Direct3D object: what makes materials, lights and viewports for a device.
pub static D3D3: Interface = Interface {
    name: "IDirect3D3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("EnumDevices", 2),
        method("CreateLight", 2, create_light),
        method("CreateMaterial", 2, create_material),
        method("CreateViewport", 2, create_viewport),
        wall("FindDevice", 2),
        wall("CreateDevice", 4),
        wall("CreateVertexBuffer", 4),
        wall("EnumZBufferFormats", 3),
        wall("EvictManagedTextures", 0),
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
        method("GetViewport2", 1, get_viewport2),
        method("SetViewport2", 1, set_viewport2),
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
        method("GetHandle", 2, material_handle),
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
        wall("GetHandle", 2),
        wall("PaletteChanged", 2),
        wall("Load", 1),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &DEVICE2, &DEVICE3, &D3D2, &D3D3, &VIEWPORT3, &MATERIAL3, &LIGHT, &TEXTURE2,
];

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
        // D3DRENDERSTATE_TEXTUREMAPBLEND: modulate.
        21 => 4,
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
        _ => return None,
    })
}

fn get_render_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let state = args[0];
    let value = device(ctx, this)
        .render
        .get(&state)
        .copied()
        .or_else(|| render_state_at_start(state));
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

fn create_material(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateMaterial(out, outer).
    let object = State::make_com(ctx, &MATERIAL3, ComState::None).map_err(Stop::Fault)?;
    ctx.state.immediate.materials.insert(object, Vec::new());
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_light(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &LIGHT, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
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

fn material_handle(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetHandle(device, out): the name a device's states use for a material.
    // One per material, from one, in the order they are asked for.
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

fn set_viewport2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    store_viewport(ctx, this, args[0])
}

fn get_viewport2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    fetch_viewport(ctx, this, args[0])
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

/// Fill rectangles of a viewport's device surface with a colour.
/// Clear rectangles of a viewport's device: the surface to a colour, the
/// depth buffer to a depth, or both.
fn clear_rects(
    ctx: &mut Ctx<'_>,
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
            what: "IDirect3DViewport3::Clear: an indexed surface".to_owned(),
            caller: 0,
        });
    }
    let bytes = format.bits.div_ceil(8);
    let mut depth = depth_value.map(|_| take_depth(ctx.state, device, width, height));
    for i in 0..count {
        // D3DRECT: x1, y1, x2, y2.
        let at = rects + i * 16;
        let read = |o: u32| ctx.mem.read_u32(at + o).map_err(Stop::Fault);
        let (x1, y1) = (read(0)?.min(width), read(4)?.min(height));
        let (x2, y2) = (read(8)?.min(width).max(x1), read(12)?.min(height).max(y1));
        if let Some(colour) = colour {
            let [b, g, r, a] = colour.to_le_bytes();
            let value = format.encode([r, g, b, a]).to_le_bytes();
            let one = &value[..bytes as usize];
            let row: Vec<u8> = (x1..x2).flat_map(|_| one.iter().copied()).collect();
            for y in y1..y2 {
                ctx.mem
                    .write(pixels + y * pitch + x1 * bytes, &row)
                    .map_err(Stop::Fault)?;
            }
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
    if colour.is_some() {
        ctx.state.cleared += 1;
    }
    Ok(S_OK)
}

/// What a clear may ask for that is written: the target.
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
    clear_rects(ctx, this, args[0], args[1], colour, depth)
}

fn clear2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear2(count, rects, flags, colour, z, stencil). There is no stencil
    // buffer on the declared card's modes this title asks for.
    if args[2] & !(D3DCLEAR_TARGET | D3DCLEAR_ZBUFFER) != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DViewport3::Clear2: flags {:#x}", args[2]),
            caller: 0,
        });
    }
    let colour = (args[2] & D3DCLEAR_TARGET != 0).then_some(args[3]);
    let depth = (args[2] & D3DCLEAR_ZBUFFER != 0).then(|| f32::from_bits(args[4]));
    clear_rects(ctx, this, args[0], args[1], colour, depth)
}

fn draw_primitive(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DrawPrimitive(type, format, vertices, count, flags).
    let indices: Vec<u32> = (0..args[3]).collect();
    draw(
        ctx, this, args[0], args[1], args[2], args[3], &indices, args[4],
    )
}

fn draw_indexed_primitive(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DrawIndexedPrimitive(type, format, vertices, count, indices, index
    // count, flags): sixteen-bit indices into the vertices.
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
        ctx, this, args[0], args[1], args[2], args[3], &indices, args[6],
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
    pub const COLORKEYENABLE: u32 = 41;
    pub const TEXTUREADDRESSU: u32 = 44;
    pub const TEXTUREADDRESSV: u32 = 45;
    pub const WRAP0: u32 = 128;
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
fn render_state(device: &Device, state: u32) -> Result<u32, Stop> {
    device
        .render
        .get(&state)
        .copied()
        .or_else(|| render_state_at_start(state))
        .ok_or_else(|| {
            unwritten(format!(
                "IDirect3DDevice3::DrawPrimitive: render state {state}, never set and not declared"
            ))
        })
}

/// The first texture stage's state, set or as it starts.
fn stage_state(device: &Device, state: u32) -> Result<u32, Stop> {
    device
        .stages
        .get(&(0, state))
        .copied()
        .or_else(|| stage_state_at_start(state, 0))
        .ok_or_else(|| unwritten(format!("IDirect3DDevice3::DrawPrimitive: texture stage state {state}, never set and not declared")))
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
    this: u32,
    kind: u32,
    format: u32,
    vertices_at: u32,
    count: u32,
    indices: &[u32],
    flags: u32,
) -> Result<u32, Stop> {
    use super::pipeline::{self, fvf, Clipped};
    use crate::raster::{self, Blend, Combine, Compare, States};

    let Some(size) = pipeline::vertex_size(format) else {
        return Err(unwritten(format!(
            "IDirect3DDevice3::DrawPrimitive: vertex format {format:#x}"
        )));
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
        return Err(unwritten(
            "IDirect3DDevice3::DrawPrimitive: into an indexed surface".to_owned(),
        ));
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

    // What the device draws them with. A state this pipeline does not honour
    // is a wall once a title turns it on, not a frame drawn as if it had not.
    let device = device_of(ctx.state, this);
    let state = |s: u32| render_state(device, s);
    let stage = |s: u32| stage_state(device, s);
    for (what, on) in [
        ("fog", state(rs::FOGENABLE)? != 0),
        ("a texture by handle", state(rs::TEXTUREHANDLE)? != 0),
        (
            "wrapping across texture edges",
            state(rs::WRAPU)? != 0 || state(rs::WRAPV)? != 0 || state(rs::WRAP0)? != 0,
        ),
        (
            "a texture map blend",
            device.render.contains_key(&rs::TEXTUREMAPBLEND),
        ),
        ("a fill that is not solid", state(rs::FILLMODE)? != 3),
    ] {
        if on {
            return Err(unwritten(format!(
                "IDirect3DDevice3::DrawPrimitive: {what}"
            )));
        }
    }
    let world = matrix_of(device, WORLD);
    let clip_from_world =
        super::scene::multiply(&matrix_of(device, VIEW), &matrix_of(device, PROJECTION));
    let clip_from_model = super::scene::multiply(&world, &clip_from_world);
    let viewport = device.current_viewport;
    let texture = device.textures.get(&0).copied().unwrap_or(0);
    let material_handle = device.light.get(&LIGHT_MATERIAL).copied().unwrap_or(0);
    let ambient = pipeline::colour(device.light.get(&LIGHT_AMBIENT).copied().unwrap_or(0));
    let compare = |value: u32| {
        Compare::from_d3d(value).ok_or_else(|| {
            unwritten(format!(
                "IDirect3DDevice3::DrawPrimitive: comparison {value}"
            ))
        })
    };
    let blend = |value: u32| {
        Blend::from_d3d(value).ok_or_else(|| {
            unwritten(format!(
                "IDirect3DDevice3::DrawPrimitive: blend factor {value}"
            ))
        })
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
            other => {
                return Err(unwritten(format!(
                    "IDirect3DDevice3::DrawPrimitive: texture filter {other}"
                )))
            }
        }
    } else {
        match stage(tss::MAGFILTER)? {
            1 => false,
            2 => true,
            other => {
                return Err(unwritten(format!(
                    "IDirect3DDevice3::DrawPrimitive: magnification filter {other}"
                )))
            }
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
        clamps(value).ok_or_else(|| {
            unwritten(format!(
                "IDirect3DDevice3::DrawPrimitive: texture address {value}"
            ))
        })
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
    let (Some(colour_combine), Some(alpha_combine)) = (combine(op, a1, a2), combine(aop, aa1, aa2))
    else {
        return Err(unwritten(format!(
            "IDirect3DDevice3::DrawPrimitive: texture stage colour {op} ({a1:#x}, {a2:#x}), alpha {aop} ({aa1:#x}, {aa2:#x})"
        )));
    };
    states.colour = colour_combine;
    states.alpha = alpha_combine;

    // Where the viewport puts the view volume.
    let place = match ctx.state.com(viewport).map(|o| o.state.clone()) {
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
        _ => {
            return Err(unwritten(
                "IDirect3DDevice3::DrawPrimitive: a viewport the title described itself".to_owned(),
            ))
        }
    };

    // The material and the lights, when the vertices are to be lit.
    let untransformed = format & fvf::POSITION_MASK == fvf::XYZ;
    let lighting = untransformed && format & fvf::NORMAL != 0 && flags & D3DDP_DONOTLIGHT == 0;
    if lighting && states.specular {
        return Err(unwritten(
            "IDirect3DDevice3::DrawPrimitive: specular highlights on lit vertices".to_owned(),
        ));
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
                    "IDirect3DDevice3::DrawPrimitive: lit vertices with material handle {material_handle}"
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
            let [sr, sg, sb, _] = v.specular.map_or([0.0; 4], pipeline::colour);
            Clipped {
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
        other => {
            return Err(unwritten(format!(
                "IDirect3DDevice3::DrawPrimitive: primitive type {other}"
            )))
        }
    };

    // The texture the first stage samples, in its own format.
    let texture_state = ctx.state.com(texture).map(|o| o.state.clone());
    let (texels, texture_info) = match texture_state {
        Some(ComState::Surface {
            pixels: at,
            width: tw,
            height: th,
            pitch: tp,
            format: tf,
            palette,
            ..
        }) => {
            let texels = ctx
                .mem
                .slice(at, (tp * th) as usize)
                .map_err(Stop::Fault)?
                .to_vec();
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
                    return Err(unwritten(
                        "IDirect3DDevice3::DrawPrimitive: an indexed texture with no palette"
                            .to_owned(),
                    ))
                }
                _ => None,
            };
            let key = ctx.state.color_keys.get(&at).copied();
            (texels, Some((tw, th, tp, tf, palette, key)))
        }
        _ => (Vec::new(), None),
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
    let texture = texture_info
        .as_ref()
        .map(|(tw, th, tp, tf, palette, key)| raster::Texture {
            bytes: &texels,
            width: *tw,
            height: *th,
            pitch: *tp,
            format: *tf,
            palette: palette.as_ref(),
            colour_key: *key,
            clamp,
        });

    // Into the surface, through the depth buffer.
    let mut depth = take_depth(ctx.state, this, width, height);
    let target_pixels = ctx
        .mem
        .slice_mut(pixels, (pitch * height) as usize)
        .map_err(Stop::Fault)?;
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
    };
    for [i, j, k] in triangles {
        let (Some(a), Some(mut b), Some(mut c)) = (corner(i), corner(j), corner(k)) else {
            continue;
        };
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
            raster::triangle(
                &mut target,
                [polygon[0], polygon[p], polygon[p + 1]],
                texture.as_ref(),
                &states,
            );
        }
    }
    device_of(ctx.state, this).depth = depth;
    Ok(S_OK)
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
