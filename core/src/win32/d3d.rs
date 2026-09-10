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
        wall("DrawPrimitive", 5),
        wall("DrawIndexedPrimitive", 7),
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
    let identity = ctx.state.identity(this);
    ctx.state.immediate.devices.entry(identity).or_default()
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
        // D3DRENDERSTATE_TEXTUREPERSPECTIVE: on, in DirectX 6.
        4 => 1,
        // D3DRENDERSTATE_FILLMODE: solid.
        8 => 3,
        // D3DRENDERSTATE_SHADEMODE: Gouraud.
        9 => 2,
        // D3DRENDERSTATE_ZWRITEENABLE: on.
        14 => 1,
        // D3DRENDERSTATE_ALPHATESTENABLE: off.
        15 => 0,
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
        // SPECULARENABLE: all off.
        26..=29 => 0,
        // D3DRENDERSTATE_COLORKEYENABLE: off.
        41 => 0,
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
fn fill_rects(
    ctx: &mut Ctx<'_>,
    this: u32,
    count: u32,
    rects: u32,
    colour: u32,
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
        depth,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let bytes = depth.div_ceil(8);
    let one = super::pixel(colour, depth);
    for i in 0..count {
        // D3DRECT: x1, y1, x2, y2.
        let at = rects + i * 16;
        let read = |o: u32| ctx.mem.read_u32(at + o).map_err(Stop::Fault);
        let (x1, y1) = (read(0)?.min(width), read(4)?.min(height));
        let (x2, y2) = (read(8)?.min(width).max(x1), read(12)?.min(height).max(y1));
        let row: Vec<u8> = (x1..x2).flat_map(|_| one.iter().copied()).collect();
        for y in y1..y2 {
            ctx.mem
                .write(pixels + y * pitch + x1 * bytes, &row)
                .map_err(Stop::Fault)?;
        }
    }
    ctx.state.cleared += 1;
    Ok(S_OK)
}

/// What a clear may ask for that is written: the target.
const D3DCLEAR_TARGET: u32 = 0x1;

fn clear(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear(count, rects, flags): to the background material's colour.
    if args[2] & !D3DCLEAR_TARGET != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DViewport3::Clear: flags {:#x}", args[2]),
            caller: 0,
        });
    }
    let handle = match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Viewport { background, .. }) => *background,
        _ => 0,
    };
    let colour = material_colour(ctx, handle);
    fill_rects(ctx, this, args[0], args[1], colour)
}

fn clear2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear2(count, rects, flags, colour, z, stencil).
    if args[2] & !D3DCLEAR_TARGET != 0 {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DViewport3::Clear2: flags {:#x}", args[2]),
            caller: 0,
        });
    }
    fill_rects(ctx, this, args[0], args[1], args[3])
}
