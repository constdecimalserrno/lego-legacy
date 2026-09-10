//! Direct3D Retained Mode.
//!
//! The boundary sits above it (ADR-0002): a title hands D3DRM a scene made of
//! frames, meshes and lights, and asks it to draw. What draws is this
//! emulator's own rasteriser, into the guest's own surface.

use super::{ComState, Convention, Ctx, Shim, State};
use crate::com::{Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[Shim {
    dll: "d3drm",
    name: "Direct3DRMCreate",
    args: 1,
    convention: Convention::Stdcall,
    run: create,
}];

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &D3DRM, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("d3drm created at {object:#010x}"));
    Ok(S_OK)
}

/// `IDirect3DRM`, in slot order.
pub static D3DRM: Interface = Interface {
    name: "IDirect3DRM",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "CreateObject",
            args: 4,
            run: unsupported,
        },
        Method {
            name: "CreateFrame",
            args: 2,
            run: create_frame,
        },
        Method {
            name: "CreateMesh",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "CreateMeshBuilder",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "CreateFace",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "CreateAnimation",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "CreateAnimationSet",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "CreateTexture",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "CreateLight",
            args: 3,
            run: create_light,
        },
        Method {
            name: "CreateLightRGB",
            args: 5,
            run: create_light_rgb,
        },
        Method {
            name: "CreateMaterial",
            args: 2,
            run: create_material,
        },
        Method {
            name: "CreateDevice",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "CreateDeviceFromSurface",
            args: 4,
            run: device_from_surface,
        },
        Method {
            name: "CreateDeviceFromD3D",
            args: 4,
            run: unsupported,
        },
        Method {
            name: "CreateDeviceFromClipper",
            args: 5,
            run: unsupported,
        },
        Method {
            name: "CreateTextureFromSurface",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "CreateShadow",
            args: 6,
            run: unsupported,
        },
        Method {
            name: "CreateViewport",
            args: 7,
            run: create_viewport,
        },
        Method {
            name: "CreateWrap",
            args: 12,
            run: unsupported,
        },
        Method {
            name: "CreateUserVisual",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "LoadTexture",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "LoadTextureFromResource",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "SetSearchPath",
            args: 1,
            run: ok,
        },
        Method {
            name: "AddSearchPath",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetSearchPath",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetDefaultTextureColors",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetDefaultTextureShades",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetDevices",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetNamedObject",
            args: 2,
            run: unsupported,
        },
        Method {
            name: "EnumerateObjects",
            args: 2,
            run: ok,
        },
        Method {
            name: "Load",
            args: 10,
            run: unsupported,
        },
        Method {
            name: "Tick",
            args: 1,
            run: ok,
        },
    ],
};

/// `IDirect3DRMDevice`, in slot order after the object's own methods.
pub static DEVICE: Interface = Interface {
    name: "IDirect3DRMDevice",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Clone",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "AddDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetAppData",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetAppData",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetName",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetName",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetClassName",
            args: 2,
            run: ok,
        },
        Method {
            name: "Init",
            args: 2,
            run: ok,
        },
        Method {
            name: "InitFromD3D",
            args: 2,
            run: ok,
        },
        Method {
            name: "InitFromClipper",
            args: 4,
            run: ok,
        },
        Method {
            name: "Update",
            args: 0,
            run: update,
        },
        Method {
            name: "AddUpdateCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteUpdateCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetBufferCount",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetBufferCount",
            args: 0,
            run: one,
        },
        Method {
            name: "SetDither",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetShades",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetQuality",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetTextureQuality",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetViewports",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetDither",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetShades",
            args: 0,
            run: shades,
        },
        Method {
            name: "GetHeight",
            args: 0,
            run: height,
        },
        Method {
            name: "GetWidth",
            args: 0,
            run: width,
        },
        Method {
            name: "GetTrianglesDrawn",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetWireframeOptions",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetQuality",
            args: 0,
            run: quality,
        },
        Method {
            name: "GetColorModel",
            args: 0,
            run: one,
        },
        Method {
            name: "GetTextureQuality",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetDirect3DDevice",
            args: 1,
            run: unsupported,
        },
    ],
};

/// `IDirect3DRMViewport`, in slot order.
pub static VIEWPORT: Interface = Interface {
    name: "IDirect3DRMViewport",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Clone",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "AddDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetAppData",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetAppData",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetName",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetName",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetClassName",
            args: 2,
            run: ok,
        },
        Method {
            name: "Init",
            args: 6,
            run: ok,
        },
        Method {
            name: "Clear",
            args: 0,
            run: clear,
        },
        Method {
            name: "Render",
            args: 1,
            run: render,
        },
        Method {
            name: "SetFront",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetBack",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetField",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetUniformScaling",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetCamera",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetProjection",
            args: 1,
            run: ok,
        },
        Method {
            name: "Transform",
            args: 2,
            run: ok,
        },
        Method {
            name: "InverseTransform",
            args: 2,
            run: ok,
        },
        Method {
            name: "Configure",
            args: 4,
            run: ok,
        },
        Method {
            name: "ForceUpdate",
            args: 4,
            run: ok,
        },
        Method {
            name: "SetPlane",
            args: 4,
            run: ok,
        },
        Method {
            name: "GetCamera",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetDevice",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetPlane",
            args: 4,
            run: ok,
        },
        Method {
            name: "Pick",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "GetUniformScaling",
            args: 0,
            run: one,
        },
        Method {
            name: "GetX",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetY",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetWidth",
            args: 0,
            run: width,
        },
        Method {
            name: "GetHeight",
            args: 0,
            run: height,
        },
        Method {
            name: "GetField",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetBack",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetFront",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetProjection",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetDirect3DViewport",
            args: 1,
            run: unsupported,
        },
    ],
};

/// Everything else a scene is made of. These keep no state yet: what a title
/// builds is remembered by the title, and what this machine has to know to
/// draw arrives when the rasteriser does (#12).
pub static OBJECT: Interface = Interface {
    name: "IDirect3DRMObject",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Clone",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "AddDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetAppData",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetAppData",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetName",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetName",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetClassName",
            args: 2,
            run: ok,
        },
    ],
};

/// `IDirect3DRM3`: DirectX 6's own retained mode, and what this title asks for
/// by name. The same machine underneath, with six slots added at the end and a
/// device creator that takes a flags word.
pub static D3DRM3: Interface = Interface {
    name: "IDirect3DRM3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "CreateObject",
            args: 4,
            run: unsupported,
        },
        Method {
            name: "CreateFrame",
            args: 2,
            run: create_frame,
        },
        Method {
            name: "CreateMesh",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "CreateMeshBuilder",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "CreateFace",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "CreateAnimation",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "CreateAnimationSet",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "CreateTexture",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "CreateLight",
            args: 3,
            run: create_light,
        },
        Method {
            name: "CreateLightRGB",
            args: 5,
            run: create_light_rgb,
        },
        Method {
            name: "CreateMaterial",
            args: 2,
            run: create_material,
        },
        Method {
            name: "CreateDevice",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "CreateDeviceFromSurface",
            args: 5,
            run: device_from_surface3,
        },
        Method {
            name: "CreateDeviceFromD3D",
            args: 4,
            run: unsupported,
        },
        Method {
            name: "CreateDeviceFromClipper",
            args: 5,
            run: unsupported,
        },
        Method {
            name: "CreateTextureFromSurface",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "CreateShadow",
            args: 8,
            run: unsupported,
        },
        Method {
            name: "CreateViewport",
            args: 7,
            run: create_viewport,
        },
        Method {
            name: "CreateWrap",
            args: 12,
            run: unsupported,
        },
        Method {
            name: "CreateUserVisual",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "LoadTexture",
            args: 2,
            run: create_texture,
        },
        Method {
            name: "LoadTextureFromResource",
            args: 3,
            run: create_texture,
        },
        Method {
            name: "SetSearchPath",
            args: 1,
            run: ok,
        },
        Method {
            name: "AddSearchPath",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetSearchPath",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetDefaultTextureColors",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetDefaultTextureShades",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetDevices",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetNamedObject",
            args: 2,
            run: unsupported,
        },
        Method {
            name: "EnumerateObjects",
            args: 2,
            run: ok,
        },
        Method {
            name: "Load",
            args: 10,
            run: unsupported,
        },
        Method {
            name: "Tick",
            args: 1,
            run: ok,
        },
        Method {
            name: "CreateProgressiveMesh",
            args: 1,
            run: create_mesh,
        },
        Method {
            name: "RegisterClient",
            args: 1,
            run: ok,
        },
        Method {
            name: "UnregisterClient",
            args: 1,
            run: ok,
        },
        Method {
            name: "CreateClippedVisual",
            args: 2,
            run: unsupported,
        },
        Method {
            name: "SetOptions",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetOptions",
            args: 1,
            run: options,
        },
    ],
};

/// `IDirect3DRMDevice3`: the device DirectX 6's retained mode hands out. Ten
/// slots more than the first one, and a title that has the newer interface
/// calls through them.
pub static DEVICE3: Interface = Interface {
    name: "IDirect3DRMDevice3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Clone",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "AddDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetAppData",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetAppData",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetName",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetName",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetClassName",
            args: 2,
            run: ok,
        },
        Method {
            name: "Init",
            args: 2,
            run: ok,
        },
        Method {
            name: "InitFromD3D",
            args: 2,
            run: ok,
        },
        Method {
            name: "InitFromClipper",
            args: 4,
            run: ok,
        },
        Method {
            name: "Update",
            args: 0,
            run: update,
        },
        Method {
            name: "AddUpdateCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteUpdateCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetBufferCount",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetBufferCount",
            args: 0,
            run: one,
        },
        Method {
            name: "SetDither",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetShades",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetQuality",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetTextureQuality",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetViewports",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetDither",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetShades",
            args: 0,
            run: shades,
        },
        Method {
            name: "GetHeight",
            args: 0,
            run: height,
        },
        Method {
            name: "GetWidth",
            args: 0,
            run: width,
        },
        Method {
            name: "GetTrianglesDrawn",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetWireframeOptions",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetQuality",
            args: 0,
            run: quality,
        },
        Method {
            name: "GetColorModel",
            args: 0,
            run: one,
        },
        Method {
            name: "GetTextureQuality",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetDirect3DDevice",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "InitFromD3D2",
            args: 2,
            run: ok,
        },
        Method {
            name: "InitFromSurface",
            args: 3,
            run: ok,
        },
        Method {
            name: "SetRenderMode",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetRenderMode",
            args: 0,
            run: render_mode,
        },
        Method {
            name: "GetDirect3DDevice2",
            args: 1,
            run: missing,
        },
        Method {
            name: "FindPreferredTextureFormat",
            args: 3,
            run: ok,
        },
        Method {
            name: "RenderStateChange",
            args: 3,
            run: ok,
        },
        Method {
            name: "LightStateChange",
            args: 3,
            run: ok,
        },
        Method {
            name: "GetStateChangeOptions",
            args: 3,
            run: ok,
        },
        Method {
            name: "SetStateChangeOptions",
            args: 3,
            run: ok,
        },
    ],
};

/// `IDirect3DRMViewport2`: two slots more than the first viewport.
pub static VIEWPORT2: Interface = Interface {
    name: "IDirect3DRMViewport2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Clone",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "AddDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "DeleteDestroyCallback",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetAppData",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetAppData",
            args: 0,
            run: ok,
        },
        Method {
            name: "SetName",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetName",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetClassName",
            args: 2,
            run: ok,
        },
        Method {
            name: "Init",
            args: 6,
            run: ok,
        },
        Method {
            name: "Clear",
            args: 1,
            run: clear,
        },
        Method {
            name: "Render",
            args: 1,
            run: render,
        },
        Method {
            name: "SetFront",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetBack",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetField",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetUniformScaling",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetCamera",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetProjection",
            args: 1,
            run: ok,
        },
        Method {
            name: "Transform",
            args: 2,
            run: ok,
        },
        Method {
            name: "InverseTransform",
            args: 2,
            run: ok,
        },
        Method {
            name: "Configure",
            args: 4,
            run: ok,
        },
        Method {
            name: "ForceUpdate",
            args: 4,
            run: ok,
        },
        Method {
            name: "SetPlane",
            args: 4,
            run: ok,
        },
        Method {
            name: "GetCamera",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetDevice",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "GetPlane",
            args: 4,
            run: ok,
        },
        Method {
            name: "Pick",
            args: 3,
            run: unsupported,
        },
        Method {
            name: "GetUniformScaling",
            args: 0,
            run: one,
        },
        Method {
            name: "GetX",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetY",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetWidth",
            args: 0,
            run: width,
        },
        Method {
            name: "GetHeight",
            args: 0,
            run: height,
        },
        Method {
            name: "GetField",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetBack",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetFront",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetProjection",
            args: 0,
            run: ok,
        },
        Method {
            name: "GetDirect3DViewport",
            args: 1,
            run: unsupported,
        },
        Method {
            name: "TransformVectors",
            args: 4,
            run: ok,
        },
        Method {
            name: "InverseTransformVectors",
            args: 4,
            run: ok,
        },
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &D3DRM, &D3DRM3, &DEVICE, &DEVICE3, &VIEWPORT, &VIEWPORT2, &OBJECT,
];

fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn one(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(1)
}

fn shades(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // How many shades a ramp has on the declared machine.
    Ok(32)
}

fn quality(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // D3DRMRENDER_GOURAUD.
    Ok(0x0000_0004)
}

fn width(ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.display_mode.0)
}

fn height(ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.display_mode.1)
}

/// Something this machine has not been taught, which the title can do without.
/// It is told so in the way the interface's own errors are spelled.
fn unsupported(ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    ctx.log
        .line("d3drm was asked for something it has not been taught");
    // D3DRMERR_NOTFOUND.
    Ok(0x8876_0244)
}

/// Something the title cannot do without.
///
/// A refusal here is worse than a wall: this title does not check the answer,
/// so it would call through the pointer it did not get and end up executing
/// its own stack. Stopping names what is missing instead (ADR-0003's stub
/// policy).
fn missing(ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Err(Stop::NotWritten {
        what: "IDirect3DRMDevice3::GetDirect3DDevice2: this title draws through Direct3D's \
               immediate mode, which is not written yet"
            .to_owned(),
        caller: ctx.cpu.eip,
    })
}

fn make_object(ctx: &mut Ctx<'_>, out: u32) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &OBJECT, ComState::None).map_err(Stop::Fault)?;
    if out != 0 {
        ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn create_frame(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make_object(ctx, args[1])
}

fn create_mesh(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make_object(ctx, args[0])
}

fn create_texture(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make_object(ctx, args[1])
}

fn create_material(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make_object(ctx, args[1])
}

fn create_light(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make_object(ctx, args[2])
}

fn create_light_rgb(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make_object(ctx, args[4])
}

fn render_mode(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // D3DRMRENDERMODE_BLENDEDTRANSPARENCY | D3DRMRENDERMODE_SORTEDTRANSPARENCY.
    Ok(0x0000_0003)
}

fn options(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] != 0 {
        // D3DRMOPTIONS_ROTATIONS.
        ctx.mem.write_u32(args[0], 1).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

/// The DirectX 6 spelling: a flags word before its answer, and the device it
/// hands back is the one that interface's own tables describe.
fn device_from_surface3(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let surface = args[2];
    let object =
        State::make_com(ctx, &DEVICE3, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[4], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3drm device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

fn device_from_surface(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDeviceFromSurface(guid, ddraw, surface, out). The surface the
    // device draws into is the guest's own, which is what makes what it draws
    // guest-visible.
    let surface = args[2];
    let object =
        State::make_com(ctx, &DEVICE, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[3], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3drm device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

fn create_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateViewport(device, camera, x, y, width, height, out). A viewport
    // made by DirectX 6's interface is one of its own.
    let newer = ctx
        .state
        .com(this)
        .is_some_and(|o| o.interface == "IDirect3DRM3");
    let interface = if newer { &VIEWPORT2 } else { &VIEWPORT };
    let object = State::make_com(
        ctx,
        interface,
        ComState::Viewport {
            device: args[0],
            x: args[2],
            y: args[3],
            width: args[4],
            height: args[5],
            background: 0,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[6], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3drm viewport {}x{} at {},{}",
        args[4], args[5], args[2], args[3]
    ));
    Ok(S_OK)
}

fn clear(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The viewport's own part of the device's surface goes to its background
    // colour. This is the first thing a title draws, and the first thing this
    // machine can show.
    let Some(ComState::Viewport {
        device,
        x,
        y,
        width,
        height,
        background,
    }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };
    let Some(ComState::Device3D { surface }) = ctx.state.com(device).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };
    let Some(ComState::Surface {
        pixels,
        pitch,
        depth,
        width: sw,
        height: sh,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };

    let bytes = depth.div_ceil(8);
    let x1 = (x + width).min(sw);
    let y1 = (y + height).min(sh);
    let row: Vec<u8> = (x..x1)
        .flat_map(|_| background.to_le_bytes().into_iter().take(bytes as usize))
        .collect();
    for row_y in y..y1 {
        ctx.mem
            .write(pixels + row_y * pitch + x * bytes, &row)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn render(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Drawing the scene is the rasteriser's, and it arrives with #12. What is
    // on the surface until then is what Clear left there.
    Ok(S_OK)
}

fn update(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The device's surface is finished with: what is on it is a frame.
    let Some(ComState::Device3D { surface }) = ctx.state.com(this).map(|o| o.state.clone()) else {
        return Ok(S_OK);
    };
    if let Some(ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        depth,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    {
        ctx.state.present = Some(super::Present {
            pixels,
            width,
            height,
            pitch,
            depth,
        });
    }
    Ok(S_OK)
}
