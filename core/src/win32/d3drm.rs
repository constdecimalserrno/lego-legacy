//! Direct3D Retained Mode.
//!
//! The boundary sits above it (ADR-0002): a title hands D3DRM a scene made of
//! frames, meshes and lights, and asks it to draw. What draws is this
//! emulator's own rasteriser, into the guest's own surface.
//!
//! Every table is the interface's full width, named and sized from its own
//! header. A slot is written when a title that drives retained mode has
//! called it: DirectX 6's by Rock Raiders, DirectX 5's by LEGO Island (#117).
//! The rest stop the run and name the method (#11).

use super::scene::Matrix;
use super::{stdcall, ComState, Ctx, Shim, State};
use crate::com::{method, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    stdcall("d3drm", "Direct3DRMCreate", 1, create),
    stdcall("d3drm", "D3DRMVectorRotate", 4, vector_rotate),
    stdcall("d3drm", "D3DRMVectorRandom", 1, vector_random),
    stdcall("d3drm", "D3DRMCreateColorRGBA", 4, create_color_rgba),
];

/// D3DRMCreateColorRGBA(red, green, blue, alpha): the `D3DCOLOR` four
/// fractions make, alpha in the top byte, each truncated to a byte as
/// Direct3D does. LEGO Island's engine imports it (#117).
fn create_color_rgba(_ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let byte = |arg: u32| super::component(value(arg));
    Ok(byte(args[3]) << 24 | byte(args[0]) << 16 | byte(args[1]) << 8 | byte(args[2]))
}

/// An object this machine never made, or made as something else.
const D3DRMERR_BADOBJECT: u32 = 0x8876_030d;
/// A value that makes no sense for the call.
const D3DRMERR_BADVALUE: u32 = 0x8876_0316;
/// No texture format the device has matches what was asked for.
const D3DRMERR_TEXTUREFORMATNOTFOUND: u32 = 0x8876_032b;

fn create(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &D3DRM, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!("d3drm created at {object:#010x}"));
    Ok(S_OK)
}

/// `IDirect3DRM`: what `Direct3DRMCreate` hands out. Rock Raiders asks it for
/// its DirectX 6 face straight away, and nothing else of it is written.
pub static D3DRM: Interface = Interface {
    name: "IDirect3DRM",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("CreateObject", 4),
        wall("CreateFrame", 2),
        wall("CreateMesh", 1),
        wall("CreateMeshBuilder", 1),
        wall("CreateFace", 1),
        wall("CreateAnimation", 1),
        wall("CreateAnimationSet", 1),
        wall("CreateTexture", 2),
        wall("CreateLight", 3),
        wall("CreateLightRGB", 5),
        wall("CreateMaterial", 2),
        wall("CreateDevice", 3),
        wall("CreateDeviceFromSurface", 4),
        wall("CreateDeviceFromD3D", 3),
        wall("CreateDeviceFromClipper", 5),
        wall("CreateTextureFromSurface", 2),
        wall("CreateShadow", 9),
        wall("CreateViewport", 7),
        wall("CreateWrap", 16),
        wall("CreateUserVisual", 3),
        wall("LoadTexture", 2),
        wall("LoadTextureFromResource", 2),
        wall("SetSearchPath", 1),
        wall("AddSearchPath", 1),
        wall("GetSearchPath", 2),
        wall("SetDefaultTextureColors", 1),
        wall("SetDefaultTextureShades", 1),
        wall("GetDevices", 1),
        wall("GetNamedObject", 2),
        wall("EnumerateObjects", 2),
        wall("Load", 10),
        wall("Tick", 1),
    ],
};

/// `IDirect3DRM2`: DirectX 5's retained mode, which LEGO Island asks for and
/// drives (#117). Its slots are where DirectX 6's are as far as
/// `CreateProgressiveMesh`, but `CreateDeviceFromSurface` takes no flags, and
/// what its methods make are DirectX 5's objects, not DirectX 6's.
pub static D3DRM2: Interface = Interface {
    name: "IDirect3DRM2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("CreateObject", 4),
        method("CreateFrame", 2, create_frame2),
        method("CreateMesh", 1, create_mesh),
        wall("CreateMeshBuilder", 1),
        wall("CreateFace", 1),
        wall("CreateAnimation", 1),
        wall("CreateAnimationSet", 1),
        wall("CreateTexture", 2),
        wall("CreateLight", 3),
        method("CreateLightRGB", 5, create_light_rgb),
        method("CreateMaterial", 2, create_material1),
        wall("CreateDevice", 3),
        wall("CreateDeviceFromSurface", 4),
        method("CreateDeviceFromD3D", 3, device_from_d3d),
        wall("CreateDeviceFromClipper", 5),
        method("CreateTextureFromSurface", 2, create_texture2),
        wall("CreateShadow", 9),
        method("CreateViewport", 7, create_viewport1),
        wall("CreateWrap", 16),
        wall("CreateUserVisual", 3),
        wall("LoadTexture", 2),
        wall("LoadTextureFromResource", 4),
        wall("SetSearchPath", 1),
        wall("AddSearchPath", 1),
        wall("GetSearchPath", 2),
        method("SetDefaultTextureColors", 1, default_texture_colors),
        method("SetDefaultTextureShades", 1, default_texture_shades),
        wall("GetDevices", 1),
        wall("GetNamedObject", 2),
        wall("EnumerateObjects", 2),
        wall("Load", 10),
        wall("Tick", 1),
        wall("CreateProgressiveMesh", 1),
    ],
};

/// How many colours, and how many shades of each, a texture retained mode
/// makes from here on starts with. Kept, for the texture that reads them; no
/// texture is made yet (#117).
fn default_texture_colors(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "texture colours", args[0])
}

fn default_texture_shades(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "texture shades", args[0])
}

/// `IDirect3DRM3`: DirectX 6's retained mode, which Rock Raiders drives.
pub static D3DRM3: Interface = Interface {
    name: "IDirect3DRM3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("CreateObject", 4),
        method("CreateFrame", 2, create_frame),
        method("CreateMesh", 1, create_mesh),
        method("CreateMeshBuilder", 1, create_mesh_builder),
        wall("CreateFace", 1),
        wall("CreateAnimation", 1),
        wall("CreateAnimationSet", 1),
        method("CreateTexture", 2, create_texture),
        method("CreateLight", 3, create_light),
        method("CreateLightRGB", 5, create_light_rgb),
        method("CreateMaterial", 2, create_material),
        wall("CreateDevice", 3),
        method("CreateDeviceFromSurface", 5, device_from_surface),
        wall("CreateDeviceFromD3D", 3),
        wall("CreateDeviceFromClipper", 5),
        method("CreateTextureFromSurface", 2, create_texture),
        wall("CreateShadow", 9),
        method("CreateViewport", 7, create_viewport),
        wall("CreateWrap", 16),
        method("CreateUserVisual", 3, create_user_visual),
        wall("LoadTexture", 2),
        wall("LoadTextureFromResource", 4),
        wall("SetSearchPath", 1),
        wall("AddSearchPath", 1),
        wall("GetSearchPath", 2),
        wall("SetDefaultTextureColors", 1),
        wall("SetDefaultTextureShades", 1),
        wall("GetDevices", 1),
        wall("GetNamedObject", 2),
        wall("EnumerateObjects", 2),
        wall("Load", 10),
        wall("Tick", 1),
        wall("CreateProgressiveMesh", 1),
        wall("RegisterClient", 2),
        wall("UnregisterClient", 1),
        wall("CreateClippedVisual", 2),
        wall("SetOptions", 1),
        wall("GetOptions", 1),
    ],
};

/// `IDirect3DRMDevice3`: the device retained mode draws with, on a DirectDraw surface.
pub static DEVICE3: Interface = Interface {
    name: "IDirect3DRMDevice3",
    methods: DEVICE3_METHODS,
};

/// `IDirect3DRMDevice2`: DirectX 5's face of the same device, which LEGO
/// Island makes over the immediate-mode device it made itself (#117). It is
/// DirectX 6's as far as `GetDirect3DDevice2`, which is where DirectX 6's
/// own additions start.
pub static DEVICE2: Interface = Interface {
    name: "IDirect3DRMDevice2",
    methods: DEVICE3_METHODS.split_at(39).0,
};

const DEVICE3_METHODS: &[crate::com::Method] = &[
    QUERY_INTERFACE,
    ADD_REF,
    RELEASE,
    wall("Clone", 3),
    wall("AddDestroyCallback", 2),
    wall("DeleteDestroyCallback", 2),
    method("SetAppData", 1, set_app_data),
    method("GetAppData", 0, get_app_data),
    method("SetName", 1, set_name),
    method("GetName", 2, get_name),
    method("GetClassName", 2, get_class_name),
    wall("Init", 2),
    wall("InitFromD3D", 2),
    wall("InitFromClipper", 4),
    method("Update", 0, update),
    wall("AddUpdateCallback", 2),
    wall("DeleteUpdateCallback", 2),
    method("SetBufferCount", 1, set_buffer_count),
    method("GetBufferCount", 0, get_buffer_count),
    method("SetDither", 1, device_set_dither),
    method("SetShades", 1, device_set_shades),
    method("SetQuality", 1, device_set_quality),
    method("SetTextureQuality", 1, device_set_texture_quality),
    wall("GetViewports", 1),
    method("GetDither", 0, device_get_dither),
    method("GetShades", 0, device_get_shades),
    method("GetHeight", 0, device_height),
    method("GetWidth", 0, device_width),
    wall("GetTrianglesDrawn", 0),
    wall("GetWireframeOptions", 0),
    method("GetQuality", 0, device_get_quality),
    wall("GetColorModel", 0),
    method("GetTextureQuality", 0, device_get_texture_quality),
    wall("GetDirect3DDevice", 1),
    wall("InitFromD3D2", 2),
    wall("InitFromSurface", 3),
    method("SetRenderMode", 1, device_set_render_mode),
    method("GetRenderMode", 0, device_get_render_mode),
    method("GetDirect3DDevice2", 1, direct3d_device2),
    method("FindPreferredTextureFormat", 3, preferred_texture_format),
    wall("RenderStateChange", 3),
    wall("LightStateChange", 3),
    wall("GetStateChangeOptions", 3),
    wall("SetStateChangeOptions", 3),
];

/// `IDirect3DRMViewport2`: a camera's view onto part of a device.
pub static VIEWPORT2: Interface = Interface {
    name: "IDirect3DRMViewport2",
    methods: &VIEWPORT2_METHODS,
};

/// `IDirect3DRMViewport`: DirectX 5's face of the same viewport, which LEGO
/// Island's retained mode makes (#117). DirectX 6's as far as
/// `GetDirect3DViewport`, but for `Clear`, which takes no flags: it clears
/// everything.
pub static VIEWPORT: Interface = Interface {
    name: "IDirect3DRMViewport",
    methods: VIEWPORT_METHODS.split_at(39).0,
};

const VIEWPORT_METHODS: &[crate::com::Method] = &{
    let mut methods = VIEWPORT2_METHODS;
    methods[12] = method("Clear", 0, clear1);
    methods
};

const VIEWPORT2_METHODS: [crate::com::Method; 41] = [
    QUERY_INTERFACE,
    ADD_REF,
    RELEASE,
    wall("Clone", 3),
    method("AddDestroyCallback", 2, destroy_callback),
    wall("DeleteDestroyCallback", 2),
    method("SetAppData", 1, set_app_data),
    method("GetAppData", 0, get_app_data),
    method("SetName", 1, set_name),
    method("GetName", 2, get_name),
    method("GetClassName", 2, get_class_name),
    wall("Init", 6),
    method("Clear", 1, clear),
    method("Render", 1, render),
    method("SetFront", 1, viewport_set_front),
    method("SetBack", 1, viewport_set_back),
    method("SetField", 1, viewport_set_field),
    method("SetUniformScaling", 1, viewport_set_uniform_scaling),
    method("SetCamera", 1, viewport_set_camera),
    method("SetProjection", 1, viewport_set_projection),
    method("Transform", 2, viewport_transform),
    method("InverseTransform", 2, viewport_inverse_transform),
    wall("Configure", 4),
    method("ForceUpdate", 4, viewport_force_update),
    method("SetPlane", 4, viewport_set_plane),
    method("GetCamera", 1, viewport_get_camera),
    method("GetDevice", 1, viewport_get_device),
    method("GetPlane", 4, viewport_get_plane),
    method("Pick", 3, pick),
    wall("GetUniformScaling", 0),
    method("GetX", 0, viewport_get_x),
    method("GetY", 0, viewport_get_y),
    method("GetWidth", 0, viewport_get_width),
    method("GetHeight", 0, viewport_get_height),
    method("GetField", 0, viewport_get_field),
    method("GetBack", 0, viewport_get_back),
    method("GetFront", 0, viewport_get_front),
    method("GetProjection", 0, viewport_get_projection),
    method("GetDirect3DViewport", 1, direct3d_viewport),
    wall("TransformVectors", 3),
    wall("InverseTransformVectors", 3),
];

/// `IDirect3DRMFrame3`: a node in the scene a title builds.
pub static FRAME3: Interface = Interface {
    name: "IDirect3DRMFrame3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Clone", 3),
        method("AddDestroyCallback", 2, destroy_callback),
        wall("DeleteDestroyCallback", 2),
        method("SetAppData", 1, set_app_data),
        method("GetAppData", 0, get_app_data),
        method("SetName", 1, set_name),
        method("GetName", 2, get_name),
        method("GetClassName", 2, get_class_name),
        method("AddChild", 1, add_child),
        method("AddLight", 1, add_light),
        wall("AddMoveCallback", 3),
        method("AddTransform", 2, add_transform),
        method("AddTranslation", 4, add_translation),
        method("AddScale", 4, add_scale),
        method("AddRotation", 5, add_rotation),
        method("AddVisual", 1, add_visual),
        method("GetChildren", 1, get_children),
        method("GetColor", 0, get_color),
        wall("GetLights", 1),
        wall("GetMaterialMode", 0),
        method("GetParent", 1, get_parent),
        method("GetPosition", 2, get_position),
        wall("GetRotation", 3),
        method("GetScene", 1, get_scene),
        wall("GetSortMode", 0),
        wall("GetTexture", 1),
        method("GetTransform", 2, get_transform),
        wall("GetVelocity", 3),
        method("GetOrientation", 3, get_orientation),
        method("GetVisuals", 2, get_visuals),
        method("InverseTransform", 2, inverse_transform_vector),
        wall("Load", 5),
        wall("LookAt", 3),
        wall("Move", 1),
        method("DeleteChild", 1, delete_child),
        method("DeleteLight", 1, delete_light),
        wall("DeleteMoveCallback", 2),
        method("DeleteVisual", 1, delete_visual),
        method("GetSceneBackground", 0, get_scene_background),
        wall("GetSceneBackgroundDepth", 1),
        wall("GetSceneFogColor", 0),
        wall("GetSceneFogEnable", 0),
        wall("GetSceneFogMode", 0),
        wall("GetSceneFogParams", 3),
        method("SetSceneBackground", 1, set_scene_background),
        method("SetSceneBackgroundRGB", 3, set_scene_background_rgb),
        wall("SetSceneBackgroundDepth", 1),
        wall("SetSceneBackgroundImage", 1),
        method("SetSceneFogEnable", 1, set_scene_fog_enable),
        method("SetSceneFogColor", 1, frame_setting),
        method("SetSceneFogMode", 1, set_scene_fog_mode),
        method("SetSceneFogParams", 3, frame_setting),
        method("SetColor", 1, set_color),
        method("SetColorRGB", 3, set_color_rgb),
        method("GetZbufferMode", 0, get_zbuffer_mode),
        method("SetMaterialMode", 1, frame_setting),
        method("SetOrientation", 7, set_orientation),
        method("SetPosition", 4, set_position),
        wall("SetRotation", 5),
        method("SetSortMode", 1, frame_setting),
        wall("SetTexture", 1),
        wall("SetVelocity", 5),
        method("SetZbufferMode", 1, set_zbuffer_mode),
        method("Transform", 2, transform_vector),
        wall("GetBox", 1),
        wall("GetBoxEnable", 0),
        wall("GetAxes", 2),
        wall("GetMaterial", 1),
        wall("GetInheritAxes", 0),
        wall("GetHierarchyBox", 1),
        wall("SetBox", 1),
        wall("SetBoxEnable", 1),
        wall("SetAxes", 6),
        wall("SetInheritAxes", 1),
        wall("SetMaterial", 1),
        wall("SetQuaternion", 2),
        wall("RayPick", 4),
        wall("Save", 3),
        wall("TransformVectors", 4),
        wall("InverseTransformVectors", 4),
        wall("SetTraversalOptions", 1),
        wall("GetTraversalOptions", 1),
        method("SetSceneFogMethod", 1, frame_setting),
        wall("GetSceneFogMethod", 1),
        wall("SetMaterialOverride", 1),
        wall("GetMaterialOverride", 1),
    ],
};

/// `IDirect3DRMMeshBuilder3`: geometry a title builds up or loads.
pub static MESH_BUILDER3: Interface = Interface {
    name: "IDirect3DRMMeshBuilder3",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Clone", 3),
        wall("AddDestroyCallback", 2),
        wall("DeleteDestroyCallback", 2),
        method("SetAppData", 1, set_app_data),
        method("GetAppData", 0, get_app_data),
        method("SetName", 1, set_name),
        method("GetName", 2, get_name),
        method("GetClassName", 2, get_class_name),
        wall("Load", 5),
        wall("Save", 3),
        wall("Scale", 3),
        wall("Translate", 3),
        wall("SetColorSource", 1),
        wall("GetBox", 1),
        wall("GenerateNormals", 2),
        wall("GetColorSource", 0),
        wall("AddMesh", 1),
        wall("AddMeshBuilder", 2),
        wall("AddFrame", 1),
        wall("AddFace", 1),
        wall("AddFaces", 6),
        wall("ReserveSpace", 3),
        wall("SetColorRGB", 3),
        wall("SetColor", 1),
        wall("SetTexture", 1),
        wall("SetMaterial", 1),
        wall("SetTextureTopology", 2),
        wall("SetQuality", 1),
        wall("SetPerspective", 1),
        wall("SetVertex", 4),
        wall("SetNormal", 4),
        wall("SetTextureCoordinates", 3),
        wall("SetVertexColor", 2),
        wall("SetVertexColorRGB", 4),
        wall("GetFaces", 1),
        wall("GetGeometry", 6),
        wall("GetTextureCoordinates", 3),
        wall("AddVertex", 3),
        wall("AddNormal", 3),
        wall("CreateFace", 1),
        wall("GetQuality", 0),
        wall("GetPerspective", 0),
        wall("GetFaceCount", 0),
        wall("GetVertexCount", 0),
        wall("GetVertexColor", 1),
        wall("CreateMesh", 1),
        wall("GetFace", 2),
        wall("GetVertex", 2),
        wall("GetNormal", 2),
        wall("DeleteVertices", 2),
        wall("DeleteNormals", 2),
        wall("DeleteFace", 1),
        wall("Empty", 1),
        wall("Optimize", 1),
        wall("AddFacesIndexed", 4),
        wall("CreateSubMesh", 1),
        wall("GetParentMesh", 2),
        wall("GetSubMeshes", 2),
        wall("DeleteSubMesh", 1),
        wall("Enable", 1),
        wall("GetEnable", 1),
        wall("AddTriangles", 4),
        wall("SetVertices", 3),
        wall("GetVertices", 3),
        wall("SetNormals", 3),
        wall("GetNormals", 3),
        wall("GetNormalCount", 0),
    ],
};

/// `IDirect3DRMMesh`: geometry in groups, ready to draw. DirectX 5's
/// retained mode makes the same mesh as DirectX 6's, and LEGO Island builds
/// its world out of them (#119). What a group is made of and drawn with is
/// kept in the scene; what reads it back is written when a title asks.
pub static MESH: Interface = Interface {
    name: "IDirect3DRMMesh",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("Clone", 3, mesh_clone),
        wall("AddDestroyCallback", 2),
        wall("DeleteDestroyCallback", 2),
        method("SetAppData", 1, set_app_data),
        method("GetAppData", 0, get_app_data),
        method("SetName", 1, set_name),
        method("GetName", 2, get_name),
        method("GetClassName", 2, get_class_name),
        wall("Scale", 3),
        wall("Translate", 3),
        method("GetBox", 1, mesh_box),
        method("AddGroup", 5, add_group),
        method("SetVertices", 4, set_vertices),
        method("SetGroupColor", 2, set_group_color),
        method("SetGroupColorRGB", 4, set_group_color_rgb),
        method("SetGroupMapping", 2, set_group_mapping),
        method("SetGroupQuality", 2, set_group_quality),
        method("SetGroupMaterial", 2, set_group_material),
        method("SetGroupTexture", 2, set_group_texture),
        wall("GetGroupCount", 0),
        wall("GetGroup", 6),
        wall("GetVertices", 4),
        wall("GetGroupColor", 1),
        wall("GetGroupMapping", 1),
        wall("GetGroupQuality", 1),
        wall("GetGroupMaterial", 2),
        method("GetGroupTexture", 2, get_group_texture),
    ],
};

/// `IDirect3DRMTexture3`: an image a mesh is drawn with.
pub static TEXTURE3: Interface = Interface {
    name: "IDirect3DRMTexture3",
    methods: &TEXTURE3_METHODS,
};

/// `IDirect3DRMTexture2`: DirectX 5's face of the same texture, which LEGO
/// Island's retained mode makes (#118). DirectX 6's as far as
/// `GenerateMIPMap`, but for `Changed`, which takes whether the pixels and
/// whether the palette changed, where DirectX 6's takes flags and rectangles.
pub static TEXTURE2: Interface = Interface {
    name: "IDirect3DRMTexture2",
    methods: TEXTURE2_METHODS.split_at(33).0,
};

/// `IDirect3DRMTexture`: DirectX 3's face of the same texture, which is
/// what a mesh hands back as a group's texture (#120). DirectX 5's as far as
/// `GetDecalTransparentColor`, which is where DirectX 5's own additions
/// start.
pub static TEXTURE: Interface = Interface {
    name: "IDirect3DRMTexture",
    methods: TEXTURE2_METHODS.split_at(30).0,
};

const TEXTURE2_METHODS: &[crate::com::Method] = &{
    let mut methods = TEXTURE3_METHODS;
    methods[14] = method("Changed", 2, texture_changed);
    methods
};

const TEXTURE3_METHODS: [crate::com::Method; 38] = [
    QUERY_INTERFACE,
    ADD_REF,
    RELEASE,
    wall("Clone", 3),
    wall("AddDestroyCallback", 2),
    wall("DeleteDestroyCallback", 2),
    method("SetAppData", 1, set_app_data),
    method("GetAppData", 0, get_app_data),
    method("SetName", 1, set_name),
    method("GetName", 2, get_name),
    method("GetClassName", 2, get_class_name),
    wall("InitFromFile", 1),
    wall("InitFromSurface", 1),
    wall("InitFromResource", 1),
    wall("Changed", 3),
    wall("SetColors", 1),
    wall("SetShades", 1),
    wall("SetDecalSize", 2),
    wall("SetDecalOrigin", 2),
    wall("SetDecalScale", 1),
    wall("SetDecalTransparency", 1),
    wall("SetDecalTransparentColor", 1),
    wall("GetDecalSize", 2),
    wall("GetDecalOrigin", 2),
    wall("GetImage", 0),
    wall("GetShades", 0),
    wall("GetColors", 0),
    wall("GetDecalScale", 0),
    wall("GetDecalTransparency", 0),
    wall("GetDecalTransparentColor", 0),
    wall("InitFromImage", 1),
    wall("InitFromResource2", 3),
    wall("GenerateMIPMap", 1),
    wall("GetSurface", 2),
    wall("SetCacheOptions", 2),
    wall("GetCacheOptions", 2),
    wall("SetDownsampleCallback", 2),
    wall("SetValidationCallback", 2),
];

/// `IDirect3DRMLight`: a light hung on a frame.
pub static LIGHT: Interface = Interface {
    name: "IDirect3DRMLight",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Clone", 3),
        wall("AddDestroyCallback", 2),
        wall("DeleteDestroyCallback", 2),
        method("SetAppData", 1, set_app_data),
        method("GetAppData", 0, get_app_data),
        method("SetName", 1, set_name),
        method("GetName", 2, get_name),
        method("GetClassName", 2, get_class_name),
        method("SetType", 1, light_set_type),
        method("SetColor", 1, light_set_color),
        method("SetColorRGB", 3, light_set_color_rgb),
        method("SetRange", 1, light_set_range),
        method("SetUmbra", 1, light_set_umbra),
        method("SetPenumbra", 1, light_set_penumbra),
        method("SetConstantAttenuation", 1, light_set_constant),
        method("SetLinearAttenuation", 1, light_set_linear),
        method("SetQuadraticAttenuation", 1, light_set_quadratic),
        wall("GetRange", 0),
        wall("GetUmbra", 0),
        wall("GetPenumbra", 0),
        wall("GetConstantAttenuation", 0),
        wall("GetLinearAttenuation", 0),
        wall("GetQuadraticAttenuation", 0),
        method("GetColor", 0, light_get_color),
        method("GetType", 0, light_get_type),
        method("SetEnableFrame", 1, light_set_enable_frame),
        wall("GetEnableFrame", 1),
    ],
};

/// `IDirect3DRMMaterial2`: how a surface reflects light.
pub static MATERIAL2: Interface = Interface {
    name: "IDirect3DRMMaterial2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Clone", 3),
        wall("AddDestroyCallback", 2),
        wall("DeleteDestroyCallback", 2),
        method("SetAppData", 1, set_app_data),
        method("GetAppData", 0, get_app_data),
        method("SetName", 1, set_name),
        method("GetName", 2, get_name),
        method("GetClassName", 2, get_class_name),
        wall("SetPower", 1),
        method("SetSpecular", 3, material_specular),
        method("SetEmissive", 3, material_emissive),
        wall("GetPower", 0),
        wall("GetSpecular", 3),
        wall("GetEmissive", 3),
        wall("GetAmbient", 3),
        wall("SetAmbient", 3),
    ],
};

/// `IDirect3DRMMaterial`: DirectX 5's face of the same material, which LEGO
/// Island's retained mode makes (#118): DirectX 6's without its ambient
/// colour.
pub static MATERIAL: Interface = Interface {
    name: "IDirect3DRMMaterial",
    methods: MATERIAL2.methods.split_at(17).0,
};

/// `IDirect3DRMUserVisual`: something a title draws itself when retained mode renders the frame it hangs on.
pub static USER_VISUAL: Interface = Interface {
    name: "IDirect3DRMUserVisual",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Clone", 3),
        wall("AddDestroyCallback", 2),
        wall("DeleteDestroyCallback", 2),
        method("SetAppData", 1, set_app_data),
        method("GetAppData", 0, get_app_data),
        method("SetName", 1, set_name),
        method("GetName", 2, get_name),
        wall("GetClassName", 2),
        wall("Init", 2),
    ],
};

/// `IDirect3DRMFrame2`: DirectX 5's face of a frame, which LEGO Island's
/// retained mode makes (#117).
///
/// Not DirectX 6's frame with a slot taken out: DirectX 6 dropped
/// `GetTextureTopology`, `SetTextureTopology` and `AddMoveCallback2`, and
/// changed what `AddMoveCallback`, `GetTransform` and `GetVisuals` take. So
/// the slots from 33 up sit one, then two, then three places apart in the
/// two, and this table is written out by name from the header (Wine's
/// `d3drmobj.h`): a method is the same under both faces only where it takes
/// the same arguments.
pub static FRAME2: Interface = Interface {
    name: "IDirect3DRMFrame2",
    methods: FRAME2_METHODS,
};

/// `IDirect3DRMFrame`: DirectX 3's face of a frame, which is what a frame array
/// hands out. It is DirectX 5's as far as `Transform`, which is where
/// DirectX 5's own additions start.
pub static FRAME: Interface = Interface {
    name: "IDirect3DRMFrame",
    methods: FRAME2_METHODS.split_at(69).0,
};

const FRAME2_METHODS: &[crate::com::Method] = &[
    QUERY_INTERFACE,
    ADD_REF,
    RELEASE,
    wall("Clone", 3),
    method("AddDestroyCallback", 2, destroy_callback),
    wall("DeleteDestroyCallback", 2),
    method("SetAppData", 1, set_app_data),
    method("GetAppData", 0, get_app_data),
    method("SetName", 1, set_name),
    method("GetName", 2, get_name),
    method("GetClassName", 2, get_class_name),
    method("AddChild", 1, add_child),
    method("AddLight", 1, add_light),
    wall("AddMoveCallback", 2),
    method("AddTransform", 2, add_transform),
    method("AddTranslation", 4, add_translation),
    method("AddScale", 4, add_scale),
    method("AddRotation", 5, add_rotation),
    method("AddVisual", 1, add_visual),
    method("GetChildren", 1, get_children),
    method("GetColor", 0, get_color),
    method("GetLights", 1, get_lights),
    wall("GetMaterialMode", 0),
    method("GetParent", 1, get_parent),
    method("GetPosition", 2, get_position),
    wall("GetRotation", 3),
    method("GetScene", 1, get_scene),
    wall("GetSortMode", 0),
    wall("GetTexture", 1),
    wall("GetTransform", 1),
    wall("GetVelocity", 3),
    method("GetOrientation", 3, get_orientation),
    wall("GetVisuals", 1),
    wall("GetTextureTopology", 2),
    method("InverseTransform", 2, inverse_transform_vector),
    wall("Load", 5),
    wall("LookAt", 3),
    wall("Move", 1),
    method("DeleteChild", 1, delete_child),
    method("DeleteLight", 1, delete_light),
    wall("DeleteMoveCallback", 2),
    method("DeleteVisual", 1, delete_visual),
    method("GetSceneBackground", 0, get_scene_background),
    wall("GetSceneBackgroundDepth", 1),
    wall("GetSceneFogColor", 0),
    wall("GetSceneFogEnable", 0),
    wall("GetSceneFogMode", 0),
    wall("GetSceneFogParams", 3),
    method("SetSceneBackground", 1, set_scene_background),
    method("SetSceneBackgroundRGB", 3, set_scene_background_rgb),
    wall("SetSceneBackgroundDepth", 1),
    wall("SetSceneBackgroundImage", 1),
    method("SetSceneFogEnable", 1, set_scene_fog_enable),
    method("SetSceneFogColor", 1, frame_setting),
    method("SetSceneFogMode", 1, set_scene_fog_mode),
    method("SetSceneFogParams", 3, frame_setting),
    method("SetColor", 1, set_color),
    method("SetColorRGB", 3, set_color_rgb),
    method("GetZbufferMode", 0, get_zbuffer_mode),
    method("SetMaterialMode", 1, frame_setting),
    method("SetOrientation", 7, set_orientation),
    method("SetPosition", 4, set_position),
    wall("SetRotation", 5),
    method("SetSortMode", 1, frame_setting),
    wall("SetTexture", 1),
    wall("SetTextureTopology", 2),
    wall("SetVelocity", 5),
    method("SetZbufferMode", 1, set_zbuffer_mode),
    method("Transform", 2, transform_vector),
    wall("AddMoveCallback2", 3),
    wall("GetBox", 1),
    wall("GetBoxEnable", 0),
    wall("GetAxes", 2),
    wall("GetMaterial", 1),
    wall("GetInheritAxes", 0),
    wall("GetHierarchyBox", 1),
    wall("SetBox", 1),
    wall("SetBoxEnable", 1),
    wall("SetAxes", 6),
    wall("SetInheritAxes", 1),
    wall("SetMaterial", 1),
    wall("SetQuaternion", 2),
    wall("RayPick", 4),
    wall("Save", 3),
];

/// `IDirect3DRMFrameArray`: a frame's children, as they were when it was asked.
pub static FRAME_ARRAY: Interface = Interface {
    name: "IDirect3DRMFrameArray",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetSize", 0, array_size),
        method("GetElement", 2, array_element),
    ],
};

/// `IDirect3DRMLightArray`: a frame's lights, as they were when it was asked.
pub static LIGHT_ARRAY: Interface = Interface {
    name: "IDirect3DRMLightArray",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetSize", 0, array_size),
        method("GetElement", 2, array_element),
    ],
};

/// `IDirect3DRMPickedArray`: what a viewport's `Pick` found under a place,
/// nearest first.
pub static PICKED_ARRAY: Interface = Interface {
    name: "IDirect3DRMPickedArray",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        method("GetSize", 0, picked_size),
        method("GetPick", 4, picked_element),
    ],
};

pub static INTERFACES: &[&Interface] = &[
    &D3DRM,
    &D3DRM2,
    &D3DRM3,
    &DEVICE2,
    &DEVICE3,
    &VIEWPORT,
    &VIEWPORT2,
    &FRAME3,
    &MESH_BUILDER3,
    &MESH,
    &TEXTURE3,
    &TEXTURE,
    &TEXTURE2,
    &LIGHT,
    &MATERIAL2,
    &MATERIAL,
    &USER_VISUAL,
    &FRAME,
    &FRAME2,
    &FRAME_ARRAY,
    &LIGHT_ARRAY,
    &PICKED_ARRAY,
];

/// What a title hangs on one of these objects, and reads back later.
///
/// It is the title's own pointer: giving it back a zero is how a guest ends up
/// dereferencing one.
fn set_app_data(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    match ctx.state.com_mut(this) {
        Some(object) => {
            object.app_data = args[0];
            Ok(S_OK)
        }
        None => Ok(D3DRMERR_BADOBJECT),
    }
}

fn get_app_data(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx.state.com(this).map_or(0, |o| o.app_data))
}

/// Make an object of one of retained mode's own kinds and hand it to the
/// guest through `out`.
fn make(ctx: &mut Ctx<'_>, interface: &'static Interface, out: u32) -> Result<u32, Stop> {
    if out == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let object = State::make_com(ctx, interface, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_frame(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    frame(ctx, &FRAME3, args)
}

/// DirectX 5's frame: the same frame, as the face DirectX 5 hands out.
fn create_frame2(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    frame(ctx, &FRAME2, args)
}

fn frame(ctx: &mut Ctx<'_>, face: &'static Interface, args: &[u32]) -> Result<u32, Stop> {
    // CreateFrame(parent, out).
    let object = State::make_com(ctx, face, ComState::None).map_err(Stop::Fault)?;
    let parent = ctx.state.identity(args[0]);
    ctx.state.scene.add(object, parent);
    if parent != 0 {
        // Its parent holds a reference, as AddChild's does.
        ctx.state.com_ref(object, 1);
    }
    if args[1] != 0 {
        ctx.mem.write_u32(args[1], object).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn create_mesh(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateMesh(out): a mesh with no groups.
    let answer = make(ctx, &MESH, args[0])?;
    if answer == S_OK {
        let object = ctx.mem.read_u32(args[0]).map_err(Stop::Fault)?;
        ctx.state
            .scene
            .meshes
            .insert(object, super::scene::Mesh::default());
    }
    Ok(answer)
}

/// The size of a `D3DRMVERTEX`: nine four-byte values.
const VERTEX_BYTES: usize = 36;

/// One group of a mesh the guest holds, to be changed.
///
/// A group the mesh does not have is a value that makes no sense for the
/// call. `D3DRMGROUP_ALLGROUPS`, every group at once, is a wall: no title
/// has asked for it.
fn with_group(
    state: &mut State,
    this: u32,
    id: u32,
    method: &str,
    change: impl FnOnce(&mut super::scene::Group) -> Result<u32, Stop>,
) -> Result<u32, Stop> {
    let identity = state.identity(this);
    let Some(mesh) = state.scene.meshes.get_mut(&identity) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    if id == u32::MAX {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DRMMesh::{method}: every group at once"),
            caller: 0,
        });
    }
    match mesh.groups.get_mut(id as usize) {
        Some(group) => change(group),
        None => Ok(D3DRMERR_BADVALUE),
    }
}

fn add_group(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // AddGroup(vertices, faces, vertices a face, face data, out): a group of
    // that many vertices, none of them set yet, and the faces' vertex
    // indices, copied. Its number is how many groups the mesh had before it.
    let (vertices, faces, per_face, data, out) = (args[0], args[1], args[2], args[3], args[4]);
    let identity = ctx.state.identity(this);
    let Some(mesh) = ctx.state.scene.meshes.get_mut(&identity) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    if per_face == 0 {
        // Faces of differing sizes, each its own count and then its indices.
        return Err(Stop::NotWritten {
            what: "IDirect3DRMMesh::AddGroup: faces of differing sizes".to_owned(),
            caller: 0,
        });
    }
    if data == 0 || out == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let count = faces as usize * per_face as usize;
    let indices = words(ctx.mem.slice(data, count * 4).map_err(Stop::Fault)?);
    let id = u32::try_from(mesh.groups.len()).unwrap_or(u32::MAX);
    mesh.groups.push(super::scene::Group {
        vertices: vec![None; vertices as usize],
        per_face,
        indices,
        ..super::scene::Group::default()
    });
    ctx.mem.write_u32(out, id).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_vertices(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetVertices(group, first, count, vertices): that many of the group's
    // vertices from the first, copied out of the title's array.
    let (first, count, values) = (args[1] as usize, args[2] as usize, args[3]);
    with_group(ctx.state, this, args[0], "SetVertices", |group| {
        if values == 0 || first + count > group.vertices.len() {
            return Ok(D3DRMERR_BADVALUE);
        }
        let given = words(
            ctx.mem
                .slice(values, count * VERTEX_BYTES)
                .map_err(Stop::Fault)?,
        );
        for (slot, vertex) in group.vertices[first..]
            .iter_mut()
            .zip(given.as_chunks::<9>().0)
        {
            *slot = Some(*vertex);
        }
        Ok(S_OK)
    })
}

/// Guest bytes as the four-byte values they hold.
fn words(bytes: &[u8]) -> Vec<u32> {
    let (words, _) = bytes.as_chunks::<4>();
    words.iter().map(|b| u32::from_le_bytes(*b)).collect()
}

fn set_group_color(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetGroupColor(group, colour), and SetGroupColorRGB(group, red, green,
    // blue): one colour, the later call's.
    with_group(ctx.state, this, args[0], "SetGroupColor", |g| {
        g.colour = Some(args[1]);
        Ok(S_OK)
    })
}

fn set_group_color_rgb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let colour = rgb(args[1], args[2], args[3]);
    with_group(ctx.state, this, args[0], "SetGroupColorRGB", |g| {
        g.colour = Some(colour);
        Ok(S_OK)
    })
}

fn set_group_mapping(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetGroupMapping(group, D3DRMMAPPING): whether its texture wraps across
    // and down, and is corrected for perspective. Kept as given.
    with_group(ctx.state, this, args[0], "SetGroupMapping", |g| {
        g.mapping = Some(args[1]);
        Ok(S_OK)
    })
}

fn set_group_quality(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetGroupQuality(group, D3DRMRENDERQUALITY): how it is shaded, lit and
    // filled. Kept as given.
    with_group(ctx.state, this, args[0], "SetGroupQuality", |g| {
        g.quality = Some(args[1]);
        Ok(S_OK)
    })
}

/// SetGroupMaterial and SetGroupTexture(group, object): the group holds a
/// reference to what it is now drawn with, and lets go of what it was.
fn set_group_object(
    ctx: &mut Ctx<'_>,
    this: u32,
    args: &[u32],
    method: &str,
    which: fn(&mut super::scene::Group) -> &mut u32,
) -> Result<u32, Stop> {
    let mut was = 0;
    let answer = with_group(ctx.state, this, args[0], method, |g| {
        was = std::mem::replace(which(g), args[1]);
        Ok(S_OK)
    })?;
    if answer == S_OK {
        ctx.state.com_ref(args[1], 1);
        ctx.state.com_ref(was, -1);
    }
    Ok(answer)
}

fn set_group_material(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_group_object(ctx, this, args, "SetGroupMaterial", |g| &mut g.material)
}

fn set_group_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    set_group_object(ctx, this, args, "SetGroupTexture", |g| &mut g.texture)
}

fn texture_changed(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Changed(pixels, palette): the title has changed its texture's surface.
    // A texture here keeps no copy of it, and reads its surface every time it
    // is drawn, so the next draw sees the change with nothing done here
    // (#120).
    if ctx
        .state
        .scene
        .textures
        .contains_key(&ctx.state.identity(this))
    {
        Ok(S_OK)
    } else {
        Ok(D3DRMERR_BADOBJECT)
    }
}

fn get_group_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetGroupTexture(group, out): the texture a group is drawn with, as
    // DirectX 3's face of it, which is what the header hands back, with a
    // reference for the caller; or null for a group drawn with none.
    let mut texture = 0;
    let answer = with_group(ctx.state, this, args[0], "GetGroupTexture", |g| {
        texture = g.texture;
        Ok(S_OK)
    })?;
    if answer != S_OK {
        return Ok(answer);
    }
    if args[1] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let face = match texture {
        0 => 0,
        t => State::face(ctx, t, &TEXTURE).map_err(Stop::Fault)?,
    };
    ctx.mem.write_u32(args[1], face).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn mesh_box(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetBox(box): the least and the greatest corner of the box, along the
    // mesh's own axes, that holds every vertex of every group. A vertex never
    // set, or a mesh with none, has no place to be held, and what retained
    // mode answers for either has not been measured.
    let Some(mesh) = ctx.state.scene.meshes.get(&ctx.state.identity(this)) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    if args[0] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let mut corners: Option<([f32; 3], [f32; 3])> = None;
    for vertex in mesh.groups.iter().flat_map(|g| &g.vertices) {
        let Some(vertex) = vertex else {
            return Err(Stop::NotWritten {
                what: "IDirect3DRMMesh::GetBox: a vertex that was never set".to_owned(),
                caller: 0,
            });
        };
        let at = [0, 1, 2].map(|i| value(vertex[i]));
        // Picked by comparing, never worked out, so each corner is the bits
        // of a vertex the guest gave.
        let (mut least, mut most) = corners.unwrap_or((at, at));
        for i in 0..3 {
            if at[i] < least[i] {
                least[i] = at[i];
            }
            if at[i] > most[i] {
                most[i] = at[i];
            }
        }
        corners = Some((least, most));
    }
    let Some((least, most)) = corners else {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMMesh::GetBox: a mesh with no vertices".to_owned(),
            caller: 0,
        });
    };
    write_vector(ctx, args[0], least)?;
    write_vector(ctx, args[0] + 12, most)?;
    Ok(S_OK)
}

fn mesh_clone(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clone(outer, interface, out): a new mesh with this one's groups, each
    // holding its own reference to what it is drawn with.
    // ponytail: the copy starts with no name and no app data of its own;
    // whether retained mode copies those has not been measured, and a title
    // that reads them off a clone is where it would show.
    let interface = crate::com::iid_text(&crate::com::read_iid(ctx, args[1])?);
    // IID_IDirect3DRMMesh, from Wine's d3drmobj.h.
    if args[0] != 0 || interface != "a3a80d01-6e12-11cf-ac4a-0000c03825a1" {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirect3DRMMesh::Clone: outer {:#x}, as {interface}, where a mesh has only \
                 been asked for as itself",
                args[0]
            ),
            caller: 0,
        });
    }
    let Some(mesh) = ctx
        .state
        .scene
        .meshes
        .get(&ctx.state.identity(this))
        .cloned()
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    if args[2] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let object = State::make_com(ctx, &MESH, ComState::None).map_err(Stop::Fault)?;
    for group in &mesh.groups {
        ctx.state.com_ref(group.material, 1);
        ctx.state.com_ref(group.texture, 1);
    }
    ctx.state.scene.meshes.insert(object, mesh);
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_mesh_builder(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make(ctx, &MESH_BUILDER3, args[0])
}

fn create_texture(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateTexture(image, out), and CreateTextureFromSurface(surface, out).
    make(ctx, &TEXTURE3, args[1])
}

/// DirectX 5's texture: the same texture, as the face DirectX 5 hands out.
fn create_texture2(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateTextureFromSurface(surface, out): a texture that draws from the
    // surface, which it holds a reference to (#120).
    let (surface, out) = (args[0], args[1]);
    let answer = make(ctx, &TEXTURE2, out)?;
    if answer == S_OK {
        let object = ctx.mem.read_u32(out).map_err(Stop::Fault)?;
        ctx.state.com_ref(surface, 1);
        ctx.state.scene.textures.insert(object, surface);
    }
    Ok(answer)
}

fn create_light(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateLight(type, colour, out): a light of that type and colour, opaque.
    light(ctx, args[0], 0xff00_0000 | args[1], args[2])
}

fn create_light_rgb(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateLightRGB(type, red, green, blue, out).
    light(ctx, args[0], rgb(args[1], args[2], args[3]), args[4])
}

fn light(ctx: &mut Ctx<'_>, kind: u32, colour: u32, out: u32) -> Result<u32, Stop> {
    let answer = make(ctx, &LIGHT, out)?;
    if answer == S_OK {
        let object = ctx.mem.read_u32(out).map_err(Stop::Fault)?;
        ctx.state.scene.lights.insert(
            object,
            super::scene::Light {
                kind,
                colour,
                ..super::scene::Light::default()
            },
        );
    }
    Ok(answer)
}

fn create_material(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    material(ctx, &MATERIAL2, args)
}

/// DirectX 5's material: the same material, as the face DirectX 5 hands out.
fn create_material1(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    material(ctx, &MATERIAL, args)
}

fn material(ctx: &mut Ctx<'_>, face: &'static Interface, args: &[u32]) -> Result<u32, Stop> {
    // CreateMaterial(power, out): the power is how sharp its highlights are.
    let answer = make(ctx, face, args[1])?;
    if answer == S_OK {
        let object = ctx.mem.read_u32(args[1]).map_err(Stop::Fault)?;
        let made = super::scene::Material::new(value(args[0]));
        ctx.state.scene.materials.insert(object, made);
    }
    Ok(answer)
}

/// SetSpecular and SetEmissive(red, green, blue): the colour of the
/// material's highlights, and the colour it gives off.
fn with_material(
    ctx: &mut Ctx<'_>,
    this: u32,
    args: &[u32],
    which: fn(&mut super::scene::Material) -> &mut [f32; 3],
) -> Result<u32, Stop> {
    let colour = [value(args[0]), value(args[1]), value(args[2])];
    let identity = ctx.state.identity(this);
    match ctx.state.scene.materials.get_mut(&identity) {
        Some(material) => {
            *which(material) = colour;
            Ok(S_OK)
        }
        None => Ok(D3DRMERR_BADOBJECT),
    }
}

fn material_specular(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_material(ctx, this, args, |m| &mut m.specular)
}

fn material_emissive(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_material(ctx, this, args, |m| &mut m.emissive)
}

fn destroy_callback(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // AddDestroyCallback(callback, argument): what retained mode calls when
    // the object is destroyed. Nothing on this machine is ever destroyed
    // (`crate::com::RELEASE`), so there is nothing to call it for, and nothing
    // to keep.
    Ok(S_OK)
}

fn vector_rotate(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // D3DRMVectorRotate(result, vector, axis, theta): the vector turned about
    // the axis, as a frame's rotation turns it, into the result, which is
    // what it answers.
    // ponytail: the answer keeps the vector's length. Wine's d3drm answers a
    // vector of length one, and normalises the caller's axis where it lies;
    // what native retained mode does is unmeasured, and a title that rotates
    // a vector that is not of length one is where the difference shows.
    let (vector, axis) = (read_vector(ctx, args[1])?, read_vector(ctx, args[2])?);
    let turned = super::scene::direct(&super::scene::rotation(axis, value(args[3])), vector);
    write_vector(ctx, args[0], turned)?;
    Ok(args[0])
}

fn device_from_surface(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDeviceFromSurface(guid, ddraw, back buffer, flags, out). The
    // surface the device draws into is the guest's own, which is what makes
    // what it draws guest-visible.
    let surface = args[2];
    let object = State::make_com(
        ctx,
        &DEVICE3,
        ComState::RetainedDevice {
            surface,
            buffers: 1,
            immediate: 0,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[4], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3drm device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

fn device_from_d3d(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDeviceFromD3D(direct3d, device, out): retained mode over the
    // immediate-mode device a title made itself, drawing into that device's
    // surface. It holds both for as long as it draws (#117).
    let (direct3d, immediate) = (args[0], args[1]);
    let Some(ComState::Device3D { surface }) = ctx.state.com(immediate).map(|o| o.state.clone())
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    ctx.state.com_ref(direct3d, 1);
    ctx.state.com_ref(immediate, 1);
    let object = State::make_com(
        ctx,
        &DEVICE2,
        ComState::RetainedDevice {
            surface,
            buffers: 1,
            immediate,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3drm device at {object:#010x} over d3d device {immediate:#010x}"
    ));
    Ok(S_OK)
}

fn create_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    viewport(ctx, &VIEWPORT2, args)
}

/// DirectX 5's viewport: the same viewport, as the face DirectX 5 hands out.
fn create_viewport1(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    viewport(ctx, &VIEWPORT, args)
}

fn viewport(ctx: &mut Ctx<'_>, face: &'static Interface, args: &[u32]) -> Result<u32, Stop> {
    // CreateViewport(device, camera, x, y, width, height, out).
    let object = State::make_com(
        ctx,
        face,
        ComState::Viewport {
            device: args[0],
            camera: args[1],
            x: args[2],
            y: args[3],
            width: args[4],
            height: args[5],
            background: 0,
            immediate: 0,
        },
    )
    .map_err(Stop::Fault)?;
    ctx.state
        .scene
        .viewports
        .insert(object, super::scene::View::default());
    ctx.mem.write_u32(args[6], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3drm viewport {}x{} at {},{}",
        args[4], args[5], args[2], args[3]
    ));
    Ok(S_OK)
}

/// The surface a retained-mode device draws into.
fn device_surface(ctx: &Ctx<'_>, device: u32) -> Option<u32> {
    match ctx.state.com(device).map(|o| &o.state) {
        Some(ComState::RetainedDevice { surface, .. }) => Some(*surface),
        _ => None,
    }
}

/// A device's size is its surface's.
fn device_size(ctx: &Ctx<'_>, device: u32) -> (u32, u32) {
    let surface = device_surface(ctx, device).unwrap_or(0);
    match ctx.state.com(surface).map(|o| &o.state) {
        Some(ComState::Surface { width, height, .. }) => (*width, *height),
        _ => (0, 0),
    }
}

fn device_width(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(device_size(ctx, this).0)
}

fn device_height(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(device_size(ctx, this).1)
}

fn set_buffer_count(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // How many buffers the title flips between, which retained mode uses to
    // know how much of the screen to redraw. Kept, and read back.
    match ctx.state.com_mut(this).map(|o| &mut o.state) {
        Some(ComState::RetainedDevice { buffers, .. }) => {
            *buffers = args[0];
            Ok(S_OK)
        }
        _ => Ok(D3DRMERR_BADOBJECT),
    }
}

fn get_buffer_count(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::RetainedDevice { buffers, .. }) => *buffers,
        _ => 0,
    })
}

fn direct3d_device2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The immediate-mode device underneath: what Rock Raiders actually draws
    // through.
    let Some(device) = immediate_device(ctx, this)? else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let face = State::face(ctx, device, &super::d3d::DEVICE2).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], face).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// The immediate-mode device under a retained-mode one, made the first time it
/// is needed. It shares the surface, because there is only one, and it is the
/// same device every time.
fn immediate_device(ctx: &mut Ctx<'_>, retained: u32) -> Result<Option<u32>, Stop> {
    let Some(ComState::RetainedDevice {
        surface, immediate, ..
    }) = ctx.state.com(retained).map(|o| o.state.clone())
    else {
        return Ok(None);
    };
    if immediate != 0 {
        return Ok(Some(immediate));
    }
    let object = State::make_com(ctx, &super::d3d::DEVICE2, ComState::Device3D { surface })
        .map_err(Stop::Fault)?;
    if let Some(ComState::RetainedDevice { immediate, .. }) =
        ctx.state.com_mut(retained).map(|o| &mut o.state)
    {
        *immediate = object;
    }
    ctx.log.line(&format!(
        "d3d immediate device at {object:#010x} on surface {surface:#010x}"
    ));
    Ok(Some(object))
}

fn direct3d_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetDirect3DViewport(out): the immediate-mode viewport underneath, with a
    // reference for the caller.
    let Some(under) = immediate_viewport(ctx, this)? else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    ctx.state.com_ref(under, 1);
    ctx.mem.write_u32(args[0], under).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// The immediate-mode viewport under a retained-mode one, on the device
/// underneath, over the same part of its surface: made the first time it is
/// needed, and the same one every time.
fn immediate_viewport(ctx: &mut Ctx<'_>, this: u32) -> Result<Option<u32>, Stop> {
    let Some(ComState::Viewport {
        device,
        x,
        y,
        width,
        height,
        immediate,
        ..
    }) = viewport_state(ctx, this)
    else {
        return Ok(None);
    };
    if immediate != 0 {
        return Ok(Some(immediate));
    }
    let Some(under) = immediate_device(ctx, device)? else {
        return Ok(None);
    };
    let object = State::make_com(
        ctx,
        &super::d3d::VIEWPORT3,
        ComState::Viewport {
            device: under,
            camera: 0,
            x,
            y,
            width,
            height,
            background: 0,
            immediate: 0,
        },
    )
    .map_err(Stop::Fault)?;
    if let Some(ComState::Viewport { immediate, .. }) =
        ctx.state.com_mut(this).map(|o| &mut o.state)
    {
        *immediate = object;
    }
    Ok(Some(object))
}

fn preferred_texture_format(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // FindPreferredTextureFormat(bit depths, flags, format). The declared card
    // offers what this machine's rasteriser draws, which has no palettized
    // texture. Rock Raiders asks two things: an eight-bit palettized format,
    // which is refused, and then any sixteen-bit one, which is 5-6-5, the
    // screen's own. Nothing else has been asked.
    const DDBD_8: u32 = 0x0800;
    const DDBD_16: u32 = 0x0400;
    const D3DRMFPTF_PALETTIZED: u32 = 0x4;
    let (depths, flags, out) = (args[0], args[1], args[2]);
    match (depths, flags) {
        (DDBD_8, D3DRMFPTF_PALETTIZED) => Ok(D3DRMERR_TEXTUREFORMATNOTFOUND),
        (DDBD_16, 0) => {
            super::ddraw::write_pixel_format(ctx, out, crate::raster::PixelFormat::RGB565)?;
            Ok(S_OK)
        }
        _ => Err(Stop::NotWritten {
            what: format!(
                "IDirect3DRMDevice3::FindPreferredTextureFormat: depths {depths:#x} flags {flags:#x}"
            ),
            caller: 0,
        }),
    }
}

fn viewport_force_update(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // ForceUpdate(left, top, right, bottom): that much of the device is to be
    // copied to the screen at the device's next Update. Here a viewport draws
    // straight into the device's surface and the title presents it itself,
    // so an Update has nothing to copy, and neither has this. LEGO Island
    // asks it for its whole viewport (#119).
    let Some(ComState::Viewport {
        x,
        y,
        width,
        height,
        ..
    }) = viewport_state(ctx, this)
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let [left, top, right, bottom] = [args[0], args[1], args[2], args[3]];
    if left < x
        || top < y
        || right > x + width
        || bottom > y + height
        || left > right
        || top > bottom
    {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirect3DRMViewport::ForceUpdate: {left},{top} to {right},{bottom}, which is \
                 not inside the viewport"
            ),
            caller: 0,
        });
    }
    Ok(S_OK)
}

fn update(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Retained mode brings the device's surface up to date with its
    // viewports. That is drawing, not presenting: the frame reaches the screen
    // when the title flips.
    Ok(S_OK)
}

/// `D3DRMCLEAR_TARGET`, `_ZBUFFER` and `_DIRTYRECTS`.
const D3DRMCLEAR_TARGET: u32 = 0x1;
const D3DRMCLEAR_ZBUFFER: u32 = 0x2;
const D3DRMCLEAR_DIRTYRECTS: u32 = 0x4;

fn clear(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear(flags): Rock Raiders clears its target alone, and asks nothing
    // else of it.
    if args[0] != D3DRMCLEAR_TARGET {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DRMViewport2::Clear: flags {:#x}", args[0]),
            caller: 0,
        });
    }
    clear_viewport(ctx, this, args[0])
}

fn clear1(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // DirectX 5's Clear() takes no flags and clears everything, as Wine's
    // d3drm forwards it: D3DRMCLEAR_ALL. LEGO Island clears so before it
    // draws its island (#120).
    clear_viewport(
        ctx,
        this,
        D3DRMCLEAR_TARGET | D3DRMCLEAR_ZBUFFER | D3DRMCLEAR_DIRTYRECTS,
    )
}

/// The viewport's own part of the device's surface goes to the background
/// colour of the scene its camera is in, and of its depth buffer to as far as
/// it goes. A viewport here draws straight into the surface, so there are no
/// dirty rectangles to clear.
fn clear_viewport(ctx: &mut Ctx<'_>, this: u32, flags: u32) -> Result<u32, Stop> {
    let Some(ComState::Viewport {
        device,
        camera,
        x,
        y,
        width,
        height,
        ..
    }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let camera = ctx.state.identity(camera);
    let background = ctx
        .state
        .scene
        .root(camera)
        .and_then(|root| ctx.state.scene.frame(root))
        .map_or(0, |f| f.background);
    let Some(ComState::Surface {
        pixels,
        pitch,
        format,
        width: sw,
        height: sh,
        ..
    }) = device_surface(ctx, device).and_then(|s| ctx.state.com(s).map(|o| o.state.clone()))
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let edges = (x, y, (x + width).min(sw), (y + height).min(sh));
    if flags & D3DRMCLEAR_TARGET != 0 {
        let depth = format.bits;
        super::fill(
            ctx,
            (pixels, pitch, depth),
            edges,
            super::pixel(background, depth),
        )?;
    }
    if flags & D3DRMCLEAR_ZBUFFER != 0 {
        let Some(immediate) = immediate_device(ctx, device)? else {
            return Ok(D3DRMERR_BADOBJECT);
        };
        let (left, top, right, bottom) = edges;
        super::d3d::clear_depth(ctx.state, immediate, [left, top, right, bottom], (sw, sh));
    }
    Ok(S_OK)
}

/// A viewport rendering a frame's tree: the title's own visuals, each asked
/// whether it can be seen and then told to draw, one call into the guest at a
/// time.
#[derive(Debug)]
pub struct RenderPass {
    visits: Vec<Visit>,
    /// The visit being called.
    next: usize,
    /// Whether that visit has said it can be seen and been told to draw.
    drawing: bool,
    device: u32,
    viewport: u32,
    immediate: u32,
    /// The scene's lights, where they are in the world, each with the one
    /// frame it lights if it lights only one.
    lights: Vec<(u32, super::pipeline::Light)>,
    /// Whether the machine has taken the pass's first call.
    pub parked: bool,
}

/// The lights on the visual a render pass is drawing: every light in its
/// scene, less those enabled only for a frame this visual's is not under.
/// Outside a render pass there are none.
#[must_use]
pub fn current_lights(state: &State) -> Vec<super::pipeline::Light> {
    let Some(pass) = state.render.as_ref() else {
        return Vec::new();
    };
    let frame = pass.visits.get(pass.next).map_or(0, |v| v.frame);
    lights_on(state, &pass.lights, frame)
}

/// The lights on what hangs on a frame: every light in its scene, less those
/// enabled only for a frame this one is not under.
fn lights_on(
    state: &State,
    lights: &[(u32, super::pipeline::Light)],
    frame: u32,
) -> Vec<super::pipeline::Light> {
    let under = |only: u32| {
        let mut at = frame;
        for _ in 0..=state.scene.frames.len() {
            if at == only {
                return true;
            }
            match state.scene.frame(at) {
                Some(f) if f.parent != 0 => at = f.parent,
                _ => return false,
            }
        }
        false
    };
    lights
        .iter()
        .filter(|(only, _)| *only == 0 || under(*only))
        .map(|(_, light)| *light)
        .collect()
}

/// One user visual, on the frame it hangs from.
#[derive(Clone, Copy, Debug)]
struct Visit {
    frame: u32,
    callback: u32,
    argument: u32,
    visual: u32,
}

/// `D3DRMUSERVISUAL_CANSEE` and `D3DRMUSERVISUAL_RENDER`.
const CAN_SEE: u32 = 0;
const DRAW: u32 = 1;

/// `D3DTRANSFORMSTATE_WORLD`, `_VIEW` and `_PROJECTION`.
const WORLD: u32 = 1;
const VIEW: u32 = 2;
const PROJECTION: u32 = 3;

fn render(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Render(frame): every visual in the frame's tree, through the device
    // under this viewport, seen from its camera. Retained mode sets the
    // device's view and projection, makes this viewport's own the current
    // one, and puts each visual's frame in the world before calling it.
    let Some(ComState::Viewport { device, camera, .. }) = viewport_state(ctx, this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let (mut visits, mut meshes) = (Vec::new(), Vec::new());
    let scene = ctx.state.identity(args[0]);
    // Whether a scene's fog is on is kept when a title sets it, and nothing
    // here draws fog, so a scene with it on is a wall.
    let root = ctx
        .state
        .scene
        .root(scene)
        .and_then(|r| ctx.state.scene.frame(r));
    if root.is_some_and(|f| f.fog) {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Render: a scene with its fog on".to_owned(),
            caller: 0,
        });
    }
    collect_visits(ctx, scene, &mut visits, &mut meshes, 0, None)?;
    // What retained mode draws itself and what it calls a title to draw are
    // drawn in the order of the tree; no title has hung both in one, so the
    // order between the two has not been needed.
    if !visits.is_empty() && !meshes.is_empty() {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Render: a scene of both meshes and user visuals"
                .to_owned(),
            caller: 0,
        });
    }
    let mut lights = Vec::new();
    collect_lights(ctx, scene, &mut lights, 0)?;
    let (Some(immediate), Some(under)) = (
        immediate_device(ctx, device)?,
        immediate_viewport(ctx, this)?,
    ) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let Some(camera_world) = ctx
        .state
        .scene
        .world(ctx.state.identity(camera))
        .and_then(|m| super::scene::invert(&m))
    else {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Render: a camera with no place in a scene".to_owned(),
            caller: 0,
        });
    };
    let identity = ctx.state.identity(this);
    let view = ctx
        .state
        .scene
        .viewports
        .get(&identity)
        .cloned()
        .unwrap_or_default();
    if view.projection != 0 {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirect3DRMViewport2::Render: projection {}, which is not perspective",
                view.projection
            ),
            caller: 0,
        });
    }
    super::d3d::set_device_transform(ctx.state, immediate, VIEW, &camera_world);
    let rect = viewport_rect(ctx, this);
    let projection = view.perspective(rect.2, rect.3);
    super::d3d::set_device_transform(ctx.state, immediate, PROJECTION, &projection);
    super::d3d::set_device_viewport(ctx.state, immediate, under);
    if !meshes.is_empty() {
        let count = meshes.len();
        ctx.log.at(crate::cli::Trace::Shims, || {
            format!("render {count} meshes")
        });
        let clip_from_world = super::scene::multiply(&camera_world, &projection);
        // The camera looks down its own z axis, so the viewer is back along
        // it.
        let back = super::scene::direct(
            &ctx.state
                .scene
                .world(ctx.state.identity(camera))
                .unwrap_or(super::scene::IDENTITY),
            [0.0, 0.0, -1.0],
        );
        let viewer = super::scene::normalise(back).unwrap_or([0.0, 0.0, -1.0]);
        let pass = MeshPass {
            device,
            immediate,
            rect,
            clip_from_world,
            viewer,
            lights: &lights,
        };
        for (world, frame, mesh) in meshes {
            draw_mesh(ctx, &pass, &world, frame, mesh)?;
        }
        return Ok(S_OK);
    }
    let visuals = visits.len();
    ctx.log.at(crate::cli::Trace::Shims, || {
        format!("render {visuals} user visuals")
    });
    if visits.is_empty() {
        return Ok(S_OK);
    }
    ctx.state.render = Some(RenderPass {
        visits,
        next: 0,
        drawing: false,
        device,
        viewport: this,
        immediate,
        lights,
        parked: false,
    });
    ctx.call_guest = render_call(ctx.state);
    Ok(S_OK)
}

/// Every light in a frame's tree, put in the world by the frame it hangs on:
/// at the frame's origin, shining down its z axis.
///
/// What a title has not told a light is what DirectX 6's documentation says
/// retained mode starts one with: constant attenuation of one and none
/// otherwise, a range as far as a float goes, an umbra of 0.4 radians and a
/// penumbra of 0.8.
fn collect_lights(
    ctx: &Ctx<'_>,
    frame: u32,
    lights: &mut Vec<(u32, super::pipeline::Light)>,
    depth: usize,
) -> Result<(), Stop> {
    use super::pipeline::Light;
    let Some(node) = ctx.state.scene.frame(frame) else {
        return Ok(());
    };
    // The visits have been walked already, so every frame here has a place.
    let (Some(world), false) = (
        ctx.state.scene.world(frame),
        depth > ctx.state.scene.frames.len(),
    ) else {
        return Ok(());
    };
    let position = [world[3][0], world[3][1], world[3][2]];
    let direction = [world[2][0], world[2][1], world[2][2]];
    for id in &node.lights {
        let Some(light) = ctx.state.scene.lights.get(id) else {
            return Err(Stop::NotWritten {
                what: format!("IDirect3DRMViewport2::Render: a light at {id:#x} nothing here made"),
                caller: 0,
            });
        };
        let [r, g, b, _] = super::pipeline::colour(light.colour);
        let colour = [r, g, b];
        let range = light.range.unwrap_or(f32::MAX.sqrt());
        let attenuation = [
            light.attenuation[0].unwrap_or(1.0),
            light.attenuation[1].unwrap_or(0.0),
            light.attenuation[2].unwrap_or(0.0),
        ];
        // An umbra and a penumbra are each the angle off the light's axis to
        // the cone's edge: as half the cone's width, Rock Raiders' lamps
        // light a narrow circle and leave its level's walls dark.
        // ponytail: measured against an earlier, unpublished emulator's frames
        // of that mission, not native retained mode; immediate mode documents
        // its own spot angles as the whole cone, so a title passing those
        // through would be lit too wide.
        let cone_cos = |angle: f32| crate::cpu::x87::maths::cos(f64::from(angle)) as f32;
        let converted = match light.kind {
            0 => Light::Ambient { colour },
            // ponytail: a parallel point light shines as a point light does,
            // attenuated by distance, where retained mode's shines in parallel
            // from where it is; nothing on the roster has been seen to use one.
            1 | 4 => Light::Point {
                colour,
                position,
                range,
                attenuation,
            },
            2 => Light::Spot {
                colour,
                position,
                direction,
                range,
                attenuation,
                inner: cone_cos(light.umbra.unwrap_or(0.4)),
                outer: cone_cos(light.penumbra.unwrap_or(0.8)),
            },
            3 => Light::Directional { colour, direction },
            other => {
                return Err(Stop::NotWritten {
                    what: format!("IDirect3DRMViewport2::Render: a light of kind {other}"),
                    caller: 0,
                })
            }
        };
        lights.push((ctx.state.identity(light.enable_frame), converted));
    }
    for &child in &node.children {
        collect_lights(ctx, child, lights, depth + 1)?;
    }
    Ok(())
}

/// Every visual in a frame's tree, the frame's own then its children's: the
/// title's own user visuals, and the meshes retained mode draws itself, each
/// where its frame puts it in the world, with the frame in the scene it is
/// lit as hanging from, and the mesh.
///
/// A frame can be a visual itself, which draws its tree again wherever the
/// frame it was added to is (#120): that frame's place in the world is
/// `instance`'s, and the frame in the scene it is drawn on.
fn collect_visits(
    ctx: &Ctx<'_>,
    frame: u32,
    visits: &mut Vec<Visit>,
    meshes: &mut Vec<(Matrix, u32, u32)>,
    depth: usize,
    instance: Option<(Matrix, u32)>,
) -> Result<(), Stop> {
    let Some(node) = ctx.state.scene.frame(frame) else {
        return Ok(());
    };
    // A frame that is its own ancestor, or its own visual, has no place in
    // any scene.
    let world = match instance {
        Some((world, _)) => Some(world),
        None => ctx.state.scene.world(frame),
    };
    let Some(world) = world.filter(|_| depth <= ctx.state.scene.frames.len()) else {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Render: a frame that is its own ancestor".to_owned(),
            caller: 0,
        });
    };
    let lit_as = instance.map_or(frame, |(_, on)| on);
    for &visual in &node.visuals {
        let identity = ctx.state.identity(visual);
        match ctx.state.com(visual).map(|o| &o.state) {
            Some(&ComState::UserVisual { callback, argument }) if instance.is_none() => {
                visits.push(Visit {
                    frame,
                    callback,
                    argument,
                    visual,
                });
            }
            _ if ctx.state.scene.meshes.contains_key(&identity) => {
                meshes.push((world, lit_as, identity));
            }
            _ if ctx.state.scene.frames.contains_key(&identity) => {
                let local = ctx.state.scene.frames[&identity].local;
                let placed = super::scene::multiply(&local, &world);
                collect_visits(
                    ctx,
                    identity,
                    visits,
                    meshes,
                    depth + 1,
                    Some((placed, lit_as)),
                )?;
            }
            _ => {
                return Err(Stop::NotWritten {
                    what: format!(
                        "IDirect3DRMViewport2::Render: a {} visual, which nothing here draws",
                        ctx.state.interface_of(visual).unwrap_or("foreign")
                    ),
                    caller: 0,
                })
            }
        }
    }
    for &child in &node.children {
        let child_instance = instance.map(|(_, on)| {
            let local = ctx
                .state
                .scene
                .frames
                .get(&child)
                .map_or(super::scene::IDENTITY, |f| f.local);
            (super::scene::multiply(&local, &world), on)
        });
        collect_visits(ctx, child, visits, meshes, depth + 1, child_instance)?;
    }
    Ok(())
}

/// What a viewport draws every mesh of a render with (#120).
struct MeshPass<'a> {
    /// The retained-mode device, whose quality is the most any group is
    /// drawn with, and the immediate-mode device under it, which draws.
    device: u32,
    immediate: u32,
    rect: super::scene::Rect,
    clip_from_world: Matrix,
    /// The direction back towards the camera.
    viewer: [f32; 3],
    lights: &'a [(u32, super::pipeline::Light)],
}

/// `D3DRMRENDERQUALITY`'s three parts: how a face is shaded, whether the
/// lights light it, and how it is filled.
const SHADE_MASK: u32 = 0x7;
const SHADE_FLAT: u32 = 0;
const SHADE_GOURAUD: u32 = 1;
const LIGHT_MASK: u32 = 0x38;
const FILL_MASK: u32 = 0x1c0;
const FILL_SOLID: u32 = 0x80;
/// `D3DRMMAP_PERSPCORRECT`: a texture corrected for perspective, which is
/// how this machine's rasteriser always draws one, and wrapping neither way.
const MAP_PERSPCORRECT: u32 = 0x4;

/// One mesh hung on a frame, group by group: its faces put in the world by
/// the frame and on the viewport by the camera; lit, as its quality says, by
/// the scene's lights falling on its colour and material, or left its colour;
/// and textured from its texture's surface as that surface is now.
///
/// Every face is in front when its corners go round clockwise as the camera
/// sees it, as they are in Direct3D, and a face turned away is not drawn.
// ponytail: a translucent group's colour is blended over what is behind it,
// in the order the groups are drawn. DirectX 5's retained mode stipples
// translucency unless a title asks its device for blending, which LEGO
// Island does not, and stippling is a pattern each card made its own way.
fn draw_mesh(
    ctx: &mut Ctx<'_>,
    pass: &MeshPass<'_>,
    world: &Matrix,
    frame: u32,
    mesh: u32,
) -> Result<(), Stop> {
    use super::pipeline::{self, Clipped};
    use crate::raster::{Blend, Combine, Compare, States};
    const METHOD: &str = "IDirect3DRMViewport::Render";
    let unwritten = |what: String| Stop::NotWritten {
        what: format!("{METHOD}: {what}"),
        caller: 0,
    };
    let setting = |name: &'static str| {
        let device = ctx.state.identity(pass.device);
        ctx.state.set_values.get(&(device, name)).copied()
    };
    let device_quality = setting("quality")
        .ok_or_else(|| unwritten("a device whose quality was never set".to_owned()))?;
    // A device's textures start nearest, as its documentation says.
    let linear = match setting("texture quality") {
        None | Some(0) => false,
        Some(1) => true,
        Some(other) => return Err(unwritten(format!("texture quality {other}"))),
    };
    let Some(ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format,
        ..
    }) = device_surface(ctx, pass.device).and_then(|s| ctx.state.com(s).map(|o| o.state.clone()))
    else {
        return Ok(());
    };
    if format.masks == [0; 4] {
        return Err(unwritten("into an indexed surface".to_owned()));
    }
    let (x, y, w, h) = pass.rect;
    let canvas = super::d3d::Canvas {
        pixels,
        width,
        height,
        pitch,
        format,
        place: pipeline::Viewport::whole(x, y, w, h),
    };
    let Some(groups) = ctx.state.scene.meshes.get(&mesh).map(|m| m.groups.len()) else {
        return Ok(());
    };
    let clip_from_model = super::scene::multiply(world, &pass.clip_from_world);
    let lights = lights_on(ctx.state, pass.lights, frame);
    for group in 0..groups {
        let scene = &ctx.state.scene;
        let g = &scene.meshes[&mesh].groups[group];
        let quality = g
            .quality
            .ok_or_else(|| unwritten("a group whose quality was never set".to_owned()))?;
        // The device's quality is the most any group is drawn with.
        let part = |mask: u32| (quality & mask).min(device_quality & mask);
        if part(FILL_MASK) != FILL_SOLID {
            return Err(unwritten(format!(
                "a group filled as {:#x}",
                part(FILL_MASK)
            )));
        }
        if part(SHADE_MASK) > SHADE_GOURAUD {
            return Err(unwritten(format!("a group shaded as {}", part(SHADE_MASK))));
        }
        let lit = part(LIGHT_MASK) != 0;
        let colour = pipeline::colour(
            g.colour
                .ok_or_else(|| unwritten("a group whose colour was never set".to_owned()))?,
        );
        let material = match g.material {
            0 => None,
            m => Some(
                *scene
                    .materials
                    .get(&ctx.state.identity(m))
                    .ok_or_else(|| unwritten(format!("a material at {m:#x} nothing here made")))?,
            ),
        };
        let texture = match g.texture {
            0 => 0,
            t => *scene
                .textures
                .get(&ctx.state.identity(t))
                .ok_or_else(|| unwritten(format!("a texture at {t:#x} nothing here made")))?,
        };
        if texture != 0 && g.mapping != Some(MAP_PERSPCORRECT) {
            return Err(unwritten(format!("a texture mapped as {:?}", g.mapping)));
        }
        let surface = pipeline::Material {
            diffuse: colour,
            ambient: colour,
            specular: material.map_or([0.0; 4], |m| {
                [m.specular[0], m.specular[1], m.specular[2], 0.0]
            }),
            emissive: material.map_or([0.0; 4], |m| {
                [m.emissive[0], m.emissive[1], m.emissive[2], 0.0]
            }),
            power: material.map_or(0.0, |m| m.power),
        };
        let highlights = surface.specular != [0.0; 4];
        // Every vertex once, lit where it is in the world.
        let vertices: Vec<Option<Clipped>> = g
            .vertices
            .iter()
            .map(|vertex| {
                let vertex = (*vertex)?;
                let v = |i: usize| value(vertex[i]);
                let position = [v(0), v(1), v(2), 1.0];
                let (diffuse, specular) = if lit {
                    let at = pipeline::project(world, position);
                    pipeline::lit(
                        [at[0], at[1], at[2]],
                        super::scene::direct(world, [v(3), v(4), v(5)]),
                        &surface,
                        [0.0; 3],
                        &lights,
                        highlights.then_some(pass.viewer),
                    )
                } else {
                    (colour, [0.0; 3])
                };
                Some(Clipped {
                    clip: pipeline::project(&clip_from_model, position),
                    diffuse,
                    specular,
                    uv: [v(6), v(7)],
                    fog: 1.0,
                })
            })
            .collect();
        let corner = |index: u32| {
            vertices
                .get(index as usize)
                .copied()
                .flatten()
                .ok_or_else(|| unwritten(format!("a face's vertex {index}, never set")))
        };
        // Each face a fan of triangles from its first corner.
        let mut triangles = Vec::new();
        for face in g.indices.chunks_exact(g.per_face as usize) {
            let first = corner(face[0])?;
            for pair in face[1..].windows(2) {
                triangles.push([first, corner(pair[0])?, corner(pair[1])?]);
            }
        }
        let states = States {
            depth_test: Some(Compare::LessEqual),
            depth_write: true,
            flat: part(SHADE_MASK) == SHADE_FLAT,
            colour: Combine::Modulate,
            alpha: Combine::Modulate,
            linear,
            colour_key: false,
            alpha_test: None,
            blend: (colour[3] < 1.0).then_some((Blend::SourceAlpha, Blend::InverseSourceAlpha)),
            specular: lit && highlights,
            fog: None,
        };
        // D3DCULL_CCW.
        super::d3d::rasterise(
            ctx,
            METHOD,
            pass.immediate,
            &canvas,
            &triangles,
            true,
            3,
            (texture, [false; 2]),
            states,
        )?;
    }
    Ok(())
}

/// The call the render pass makes next: the visit's frame is put in the
/// world, and its callback is asked or told.
fn render_call(state: &mut State) -> Option<(u32, Vec<u32>)> {
    let pass = state.render.as_ref()?;
    let visit = *pass.visits.get(pass.next)?;
    let (reason, device, viewport, immediate) = (
        if pass.drawing { DRAW } else { CAN_SEE },
        pass.device,
        pass.viewport,
        pass.immediate,
    );
    let world = state
        .scene
        .world(visit.frame)
        .unwrap_or(super::scene::IDENTITY);
    super::d3d::set_device_transform(state, immediate, WORLD, &world);
    Some((
        visit.callback,
        vec![visit.visual, visit.argument, reason, device, viewport],
    ))
}

/// The guest has returned from a render pass's call with `answered`: the
/// next call, or `None` once every visual has had its turn.
pub fn render_turn(state: &mut State, answered: u32) -> Option<(u32, Vec<u32>)> {
    let pass = state.render.as_mut()?;
    if !pass.drawing && answered != 0 {
        pass.drawing = true;
    } else {
        pass.drawing = false;
        pass.next += 1;
    }
    if pass.next >= pass.visits.len() {
        state.render = None;
        return None;
    }
    render_call(state)
}

/// A frame the guest holds, to be changed.
///
/// A pointer this machine never made a frame for is a guest bug or a face
/// nobody wrote; either way the answer is the interface's own refusal rather
/// than a frame invented on the spot.
fn with_frame(
    ctx: &mut Ctx<'_>,
    this: u32,
    change: impl FnOnce(&mut super::scene::Frame),
) -> Result<u32, Stop> {
    match ctx.state.scene.frame_mut(ctx.state.identity(this)) {
        Some(frame) => {
            change(frame);
            Ok(S_OK)
        }
        None => Ok(D3DRMERR_BADOBJECT),
    }
}

/// A frame's setting that nothing here draws by: answered as a frame answers
/// it, and kept nowhere.
fn frame_setting(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |_| {})
}

/// Where a reference frame the guest passed puts things in the scene.
///
/// A null reference is the world itself, the space the scene's root is in,
/// whatever the frame hangs from (#120). Rock Raiders passes one for a frame
/// with no parent, where relative to nothing and relative to the scene are
/// the same thing; LEGO Island, for its camera.
fn reference(ctx: &Ctx<'_>, method: &str, at: u32) -> Result<Matrix, Stop> {
    if at == 0 {
        return Ok(super::scene::IDENTITY);
    }
    let at = ctx.state.identity(at);
    ctx.state.scene.world(at).ok_or_else(|| Stop::NotWritten {
        what: format!("IDirect3DRMFrame3::{method}: {at:#x} is not a frame"),
        caller: 0,
    })
}

/// A float the guest passed in a four-byte argument.
fn value(arg: u32) -> f32 {
    f32::from_bits(arg)
}

/// Where a frame's parent puts things: the transform from the scene into the
/// parent's own space.
fn into_parent(ctx: &Ctx<'_>, method: &str, frame: u32) -> Result<Matrix, Stop> {
    let parent = ctx.state.scene.frame(frame).map_or(0, |f| f.parent);
    if parent == 0 {
        return Ok(super::scene::IDENTITY);
    }
    let world = ctx
        .state
        .scene
        .world(parent)
        .unwrap_or(super::scene::IDENTITY);
    super::scene::invert(&world).ok_or_else(|| Stop::NotWritten {
        what: format!("IDirect3DRMFrame3::{method}: a parent whose transform flattens space"),
        caller: 0,
    })
}

fn set_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetPosition(reference, x, y, z): the frame's origin lands at this point
    // in the reference frame's space, and its orientation is left alone.
    if ctx.state.scene.frame(ctx.state.identity(this)).is_none() {
        return Ok(D3DRMERR_BADOBJECT);
    }
    let into = reference_into_parent(ctx, "SetPosition", this, args[0])?;
    let [x, y, z] =
        super::scene::transform(&into, [value(args[1]), value(args[2]), value(args[3])]);
    with_frame(ctx, this, |f| f.local[3] = [x, y, z, 1.0])
}

/// What takes a point in a reference frame's space into the space of the
/// frame's parent, where the frame's own transform is. A reference that is
/// the parent needs nothing at all, which a parent that flattens space (one
/// scaled to nothing, as a title hides a thing) still allows.
fn reference_into_parent(ctx: &Ctx<'_>, method: &str, frame: u32, at: u32) -> Result<Matrix, Stop> {
    let parent = ctx
        .state
        .scene
        .frame(ctx.state.identity(frame))
        .map_or(0, |f| f.parent);
    if ctx.state.identity(at) == parent {
        return Ok(super::scene::IDENTITY);
    }
    Ok(super::scene::multiply(
        &reference(ctx, method, at)?,
        &into_parent(ctx, method, frame)?,
    ))
}

fn frame_or_bad(ctx: &Ctx<'_>, at: u32) -> bool {
    ctx.state.scene.frame(ctx.state.identity(at)).is_some()
}

fn add_child(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // A frame hangs from one parent at a time: adding it here takes it off
    // wherever it was.
    let (parent, child) = (ctx.state.identity(this), ctx.state.identity(args[0]));
    if !frame_or_bad(ctx, parent) || !frame_or_bad(ctx, child) {
        return Ok(D3DRMERR_BADOBJECT);
    }
    // The parent holds a reference to a child it did not already have.
    if ctx.state.scene.frame(child).map(|f| f.parent) != Some(parent) {
        ctx.state.com_ref(child, 1);
    }
    ctx.state.scene.attach(parent, child);
    Ok(S_OK)
}

fn delete_child(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let (parent, child) = (ctx.state.identity(this), ctx.state.identity(args[0]));
    if !frame_or_bad(ctx, child) {
        return Ok(D3DRMERR_BADOBJECT);
    }
    if ctx.state.scene.frame(child).map(|f| f.parent) != Some(parent) {
        return Ok(D3DRMERR_BADVALUE);
    }
    ctx.state.scene.attach(0, child);
    ctx.state.com_ref(child, -1);
    Ok(S_OK)
}

/// Hand the guest a frame through `out`, counting the reference it now holds.
fn give_frame(ctx: &mut Ctx<'_>, out: u32, frame: u32) -> Result<u32, Stop> {
    if out == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    if frame != 0 {
        ctx.state.com_ref(frame, 1);
    }
    ctx.mem.write_u32(out, frame).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn get_parent(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(parent) = ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map(|f| f.parent)
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    give_frame(ctx, args[0], parent)
}

fn get_scene(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(root) = ctx.state.scene.root(this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    give_frame(ctx, args[0], root)
}

/// How a new transform is put together with the frame's own.
fn combine(
    ctx: &mut Ctx<'_>,
    this: u32,
    how: u32,
    with: Matrix,
    replace: Matrix,
) -> Result<u32, Stop> {
    use super::scene::multiply;
    // D3DRMCOMBINE_REPLACE, _BEFORE and _AFTER: before means the new transform
    // is applied to a point first, in the frame's own space.
    if how > 2 {
        return Ok(D3DRMERR_BADVALUE);
    }
    with_frame(ctx, this, |f| {
        f.local = match how {
            0 => replace,
            1 => multiply(&with, &f.local),
            _ => multiply(&f.local, &with),
        };
    })
}

fn read_matrix(ctx: &Ctx<'_>, at: u32) -> Result<Matrix, Stop> {
    let mut m = [[0.0; 4]; 4];
    for (i, cell) in m.iter_mut().flatten().enumerate() {
        *cell = value(ctx.mem.read_u32(at + 4 * i as u32).map_err(Stop::Fault)?);
    }
    Ok(m)
}

fn write_matrix(ctx: &mut Ctx<'_>, at: u32, m: &Matrix) -> Result<(), Stop> {
    for (i, cell) in m.iter().flatten().enumerate() {
        ctx.mem
            .write_u32(at + 4 * i as u32, cell.to_bits())
            .map_err(Stop::Fault)?;
    }
    Ok(())
}

fn add_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let m = read_matrix(ctx, args[1])?;
    // Only an affine transform is a place a frame can be.
    if m[0][3] != 0.0 || m[1][3] != 0.0 || m[2][3] != 0.0 || m[3][3] != 1.0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    combine(ctx, this, args[0], m, m)
}

fn add_translation(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let mut m = super::scene::IDENTITY;
    m[3] = [value(args[1]), value(args[2]), value(args[3]), 1.0];
    combine(ctx, this, args[0], m, m)
}

fn add_scale(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let mut m = super::scene::IDENTITY;
    m[0][0] = value(args[1]);
    m[1][1] = value(args[2]);
    m[2][2] = value(args[3]);
    combine(ctx, this, args[0], m, m)
}

fn add_rotation(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let m = super::scene::rotation(
        [value(args[1]), value(args[2]), value(args[3])],
        value(args[4]),
    );
    combine(ctx, this, args[0], m, m)
}

/// A frame's transform into a reference frame's space, or into the scene's
/// when the reference is null.
fn relative(ctx: &Ctx<'_>, method: &str, this: u32, reference: u32) -> Result<Matrix, Stop> {
    // A reference the frame hangs under needs no inverse: it is the frame's
    // own transforms from there down, which a reference that flattens space
    // still has.
    let under = ctx.state.identity(reference);
    if under != 0 {
        let (mut at, mut down) = (ctx.state.identity(this), super::scene::IDENTITY);
        for _ in 0..=ctx.state.scene.frames.len() {
            if at == under {
                return Ok(down);
            }
            match ctx.state.scene.frame(at) {
                Some(f) if f.parent != 0 => {
                    down = super::scene::multiply(&down, &f.local);
                    at = f.parent;
                }
                _ => break,
            }
        }
    }
    let world = ctx
        .state
        .scene
        .world(this)
        .ok_or_else(|| Stop::NotWritten {
            what: format!("IDirect3DRMFrame3::{method}: {this:#x} is not a frame"),
            caller: 0,
        })?;
    let into =
        super::scene::invert(&self::reference(ctx, method, reference)?).ok_or_else(|| {
            Stop::NotWritten {
                what: format!("IDirect3DRMFrame3::{method}: a reference that flattens space"),
                caller: 0,
            }
        })?;
    Ok(super::scene::multiply(&world, &into))
}

fn get_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let m = relative(ctx, "GetTransform", this, args[0])?;
    write_matrix(ctx, args[1], &m)?;
    Ok(S_OK)
}

fn read_vector(ctx: &Ctx<'_>, at: u32) -> Result<[f32; 3], Stop> {
    let mut v = [0.0; 3];
    for (i, c) in v.iter_mut().enumerate() {
        *c = value(ctx.mem.read_u32(at + 4 * i as u32).map_err(Stop::Fault)?);
    }
    Ok(v)
}

fn write_vector(ctx: &mut Ctx<'_>, at: u32, v: [f32; 3]) -> Result<(), Stop> {
    for (i, c) in v.iter().enumerate() {
        ctx.mem
            .write_u32(at + 4 * i as u32, c.to_bits())
            .map_err(Stop::Fault)?;
    }
    Ok(())
}

fn transform_vector(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Transform(destination, source): a point in the frame's space, in the
    // scene's.
    let Some(world) = ctx.state.scene.world(this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let v = read_vector(ctx, args[1])?;
    write_vector(ctx, args[0], super::scene::transform(&world, v))?;
    Ok(S_OK)
}

fn inverse_transform_vector(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // InverseTransform(destination, source): a point in the scene's space, in
    // the frame's.
    let Some(world) = ctx.state.scene.world(this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let Some(into) = super::scene::invert(&world) else {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMFrame3::InverseTransform: a frame that flattens space".to_owned(),
            caller: 0,
        });
    };
    let v = read_vector(ctx, args[1])?;
    write_vector(ctx, args[0], super::scene::transform(&into, v))?;
    Ok(S_OK)
}

fn add_visual(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 {
        return Ok(D3DRMERR_BADOBJECT);
    }
    let visual = args[0];
    let added = ctx
        .state
        .scene
        .frame_mut(ctx.state.identity(this))
        .map(|f| {
            let new = !f.visuals.contains(&visual);
            if new {
                f.visuals.push(visual);
            }
            new
        });
    match added {
        None => Ok(D3DRMERR_BADOBJECT),
        Some(new) => {
            if new {
                ctx.state.com_ref(visual, 1);
            }
            Ok(S_OK)
        }
    }
}

fn delete_visual(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let visual = args[0];
    let removed = ctx
        .state
        .scene
        .frame_mut(ctx.state.identity(this))
        .map(|f| {
            let before = f.visuals.len();
            f.visuals.retain(|v| *v != visual);
            f.visuals.len() != before
        });
    match removed {
        None => Ok(D3DRMERR_BADOBJECT),
        Some(false) => Ok(D3DRMERR_BADVALUE),
        Some(true) => {
            ctx.state.com_ref(visual, -1);
            Ok(S_OK)
        }
    }
}

fn add_light(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let light = args[0];
    if light == 0 {
        return Ok(D3DRMERR_BADOBJECT);
    }
    let mut added = false;
    let answer = with_frame(ctx, this, |f| {
        if !f.lights.contains(&light) {
            f.lights.push(light);
            added = true;
        }
    })?;
    if added {
        ctx.state.com_ref(light, 1);
    }
    Ok(answer)
}

fn delete_light(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let light = args[0];
    with_frame(ctx, this, |f| f.lights.retain(|l| *l != light))
}

fn vector_random(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // D3DRMVectorRandom(result): a vector of length one, into the result,
    // which is what it answers. Each of its three parts is the C library's
    // rand, whose sequence this is from its documented seed of one, so the
    // same vectors come in the same order on every run; and as rand is never
    // negative, neither is a part.
    // ponytail: that is Wine's d3drm, whose generator native retained mode's
    // has not been measured against.
    let mut next = || {
        let state = ctx.state.scene.random.get_or_insert(1);
        *state = state.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((*state >> 16) & 0x7fff) as f32
    };
    let v = [next(), next(), next()];
    let unit = super::scene::normalise(v).unwrap_or([1.0, 0.0, 0.0]);
    write_vector(ctx, args[0], unit)?;
    Ok(args[0])
}

fn set_scene_fog_enable(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.fog = args[0] != 0)
}

fn set_scene_fog_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // D3DRMFOG_LINEAR, _EXPONENTIAL and _EXPONENTIALSQUARED.
    if args[0] > 2 {
        return Ok(D3DRMERR_BADVALUE);
    }
    with_frame(ctx, this, |_| {})
}

fn set_scene_background(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.background = args[0])
}

/// An opaque colour from three fractions.
fn rgb(red: u32, green: u32, blue: u32) -> u32 {
    0xff00_0000
        | (super::component(value(red)) << 16)
        | (super::component(value(green)) << 8)
        | super::component(value(blue))
}

fn set_scene_background_rgb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let colour = rgb(args[0], args[1], args[2]);
    with_frame(ctx, this, |f| f.background = colour)
}

fn get_scene_background(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map_or(0, |f| f.background))
}

fn get_position(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetPosition(reference, out): the frame's origin in the reference's
    // space.
    let m = relative(ctx, "GetPosition", this, args[0])?;
    write_vector(ctx, args[1], [m[3][0], m[3][1], m[3][2]])?;
    Ok(S_OK)
}

fn set_orientation(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetOrientation(reference, direction, up): the frame's z axis points
    // along the direction and its y axis as near the up vector as a right
    // angle allows, both in the reference frame's space. Where the frame is
    // does not move.
    use super::scene::{cross, direct, normalise};
    if !frame_or_bad(ctx, this) {
        return Ok(D3DRMERR_BADOBJECT);
    }
    let into = reference_into_parent(ctx, "SetOrientation", this, args[0])?;
    let dir = [value(args[1]), value(args[2]), value(args[3])];
    let up = [value(args[4]), value(args[5]), value(args[6])];
    let (Some(dir), Some(right)) = (normalise(dir), normalise(cross(up, dir))) else {
        // No direction, or an up vector along it: there is no orientation.
        return Ok(D3DRMERR_BADVALUE);
    };
    let up = cross(dir, right);
    let axes = [right, up, dir].map(|axis| direct(&into, axis));
    with_frame(ctx, this, |f| {
        for (row, axis) in f.local.iter_mut().zip(axes) {
            *row = [axis[0], axis[1], axis[2], 0.0];
        }
    })
}

fn get_orientation(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetOrientation(reference, direction, up): the frame's z and y axes in the
    // reference's space.
    let m = relative(ctx, "GetOrientation", this, args[0])?;
    let normal =
        |row: [f32; 4]| super::scene::normalise([row[0], row[1], row[2]]).unwrap_or([0.0; 3]);
    write_vector(ctx, args[1], normal(m[2]))?;
    write_vector(ctx, args[2], normal(m[1]))?;
    Ok(S_OK)
}

fn set_zbuffer_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.zbuffer_mode = args[0])
}

fn get_zbuffer_mode(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map_or(0, |f| f.zbuffer_mode))
}

fn set_color(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.color = args[0])
}

fn set_color_rgb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let colour = rgb(args[0], args[1], args[2]);
    with_frame(ctx, this, |f| f.color = colour)
}

fn get_color(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map_or(0, |f| f.color))
}

/// A light the guest holds, to be changed.
fn with_light(
    ctx: &mut Ctx<'_>,
    this: u32,
    change: impl FnOnce(&mut super::scene::Light),
) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    match ctx.state.scene.lights.get_mut(&identity) {
        Some(light) => {
            change(light);
            Ok(S_OK)
        }
        None => Ok(D3DRMERR_BADOBJECT),
    }
}

fn light_set_type(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.kind = args[0])
}

fn light_set_color(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.colour = 0xff00_0000 | args[0])
}

fn light_set_color_rgb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let colour = rgb(args[0], args[1], args[2]);
    with_light(ctx, this, |l| l.colour = colour)
}

fn light_set_range(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.range = Some(value(args[0])))
}

fn light_set_umbra(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.umbra = Some(value(args[0])))
}

fn light_set_penumbra(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.penumbra = Some(value(args[0])))
}

fn light_set_constant(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.attenuation[0] = Some(value(args[0])))
}

fn light_set_linear(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.attenuation[1] = Some(value(args[0])))
}

fn light_set_quadratic(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.attenuation[2] = Some(value(args[0])))
}

fn light_set_enable_frame(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_light(ctx, this, |l| l.enable_frame = args[0])
}

fn light_get_color(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    Ok(ctx
        .state
        .scene
        .lights
        .get(&identity)
        .map_or(0, |l| l.colour))
}

fn light_get_type(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    Ok(ctx.state.scene.lights.get(&identity).map_or(0, |l| l.kind))
}

/// A device value a title sets and may read back: dither, shades, quality,
/// texture quality, render mode.
///
/// What each does to drawing is the rasteriser's to honour. Retained mode's
/// own defaults have not been measured, so reading one that was never set is
/// a wall rather than a number made up.
fn keep(ctx: &mut Ctx<'_>, this: u32, setting: &'static str, value: u32) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    ctx.state.set_values.insert((identity, setting), value);
    Ok(S_OK)
}

fn kept(ctx: &mut Ctx<'_>, this: u32, method: &str, setting: &'static str) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    ctx.state
        .set_values
        .get(&(identity, setting))
        .copied()
        .ok_or_else(|| Stop::NotWritten {
            what: format!("IDirect3DRMDevice3::{method}: read before it was set"),
            caller: 0,
        })
}

fn device_set_dither(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "dither", args[0])
}

fn device_get_dither(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    kept(ctx, this, "GetDither", "dither")
}

fn device_set_shades(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "shades", args[0])
}

fn device_get_shades(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    kept(ctx, this, "GetShades", "shades")
}

fn device_set_quality(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "quality", args[0])
}

fn device_get_quality(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    kept(ctx, this, "GetQuality", "quality")
}

fn device_set_texture_quality(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "texture quality", args[0])
}

fn device_get_texture_quality(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    kept(ctx, this, "GetTextureQuality", "texture quality")
}

fn device_set_render_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    keep(ctx, this, "render mode", args[0])
}

fn device_get_render_mode(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    kept(ctx, this, "GetRenderMode", "render mode")
}

fn create_user_visual(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateUserVisual(callback, argument, out): the title's own function,
    // which retained mode calls when it renders a frame this hangs on.
    if args[0] == 0 || args[2] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let object = State::make_com(
        ctx,
        &USER_VISUAL,
        ComState::UserVisual {
            callback: args[0],
            argument: args[1],
        },
    )
    .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// E_INVALIDARG: a buffer too small, or no size to say how big.
const E_INVALIDARG: u32 = 0x8007_0057;

fn set_name(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    if args[0] == 0 {
        ctx.state.names.remove(&identity);
    } else {
        let name = ctx.string(args[0]).map_err(Stop::Fault)?;
        ctx.state.names.insert(identity, name);
    }
    Ok(S_OK)
}

/// GetName and GetClassName: `size` in is how big the buffer is, and out is
/// how big the answer is with its terminator. No buffer asks only the size.
fn write_name(
    ctx: &mut Ctx<'_>,
    size_at: u32,
    buffer: u32,
    name: Option<&str>,
) -> Result<u32, Stop> {
    if size_at == 0 {
        return Ok(E_INVALIDARG);
    }
    let size = ctx.mem.read_u32(size_at).map_err(Stop::Fault)?;
    let needed = name.map_or(0, |n| u32::try_from(n.len() + 1).unwrap_or(u32::MAX));
    if buffer != 0 {
        if size < needed {
            return Ok(E_INVALIDARG);
        }
        match name {
            Some(name) => {
                ctx.write_string(buffer, needed, name)
                    .map_err(Stop::Fault)?;
            }
            None if size > 0 => ctx.mem.write(buffer, &[0]).map_err(Stop::Fault)?,
            None => {}
        }
    }
    ctx.mem.write_u32(size_at, needed).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn get_name(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    let name = ctx.state.names.get(&identity).cloned();
    write_name(ctx, args[0], args[1], name.as_deref())
}

fn get_class_name(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // What retained mode calls each of its kinds of object.
    let class = match ctx.state.interface_of(this).unwrap_or("") {
        "IDirect3DRMFrame" | "IDirect3DRMFrame2" | "IDirect3DRMFrame3" => "Frame",
        "IDirect3DRMDevice2" | "IDirect3DRMDevice3" => "Device",
        "IDirect3DRMViewport" | "IDirect3DRMViewport2" => "Viewport",
        "IDirect3DRMMeshBuilder3" => "Builder",
        "IDirect3DRMMesh" => "Mesh",
        "IDirect3DRMTexture3" => "Texture",
        "IDirect3DRMLight" => "Light",
        "IDirect3DRMMaterial2" => "Material",
        _ => return Ok(D3DRMERR_BADOBJECT),
    };
    write_name(ctx, args[0], args[1], Some(class))
}

fn get_visuals(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetVisuals(count, visuals): how many visuals hang on the frame, and,
    // given room for them all, the visuals themselves, each with a reference.
    let (count_at, out) = (args[0], args[1]);
    let Some(visuals) = ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map(|f| f.visuals.clone())
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    if count_at == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let room = ctx.mem.read_u32(count_at).map_err(Stop::Fault)?;
    let have = u32::try_from(visuals.len()).unwrap_or(u32::MAX);
    ctx.mem.write_u32(count_at, have).map_err(Stop::Fault)?;
    if out == 0 {
        return Ok(S_OK);
    }
    if room < have {
        return Ok(D3DRMERR_BADVALUE);
    }
    for (i, visual) in visuals.iter().enumerate() {
        ctx.state.com_ref(*visual, 1);
        ctx.mem
            .write_u32(out + 4 * i as u32, *visual)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn get_children(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetChildren(out): an array of the frame's children as they are now,
    // each as the older face of a frame, which is what the array holds.
    if args[0] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let Some(children) = ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map(|f| f.children.clone())
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    give_array(ctx, args[0], (&FRAME_ARRAY, &FRAME), children)
}

fn get_lights(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetLights(out): an array of the lights hung on the frame as they are
    // now. LEGO Island's lights are each a frame with one light on it, and it
    // reaches the light through this to change its colour (#124).
    if args[0] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let Some(lights) = ctx
        .state
        .scene
        .frame(ctx.state.identity(this))
        .map(|f| f.lights.clone())
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    give_array(ctx, args[0], (&LIGHT_ARRAY, &LIGHT), lights)
}

/// An array of objects as they are now, each as the face the array holds,
/// handed to the guest through `out`.
fn give_array(
    ctx: &mut Ctx<'_>,
    out: u32,
    (array, face): (&'static Interface, &'static Interface),
    objects: Vec<u32>,
) -> Result<u32, Stop> {
    let mut items = Vec::with_capacity(objects.len());
    for object in objects {
        items.push(State::face(ctx, object, face).map_err(Stop::Fault)?);
    }
    let array = State::make_com(ctx, array, ComState::Array { items }).map_err(Stop::Fault)?;
    ctx.mem.write_u32(out, array).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn array_items(ctx: &Ctx<'_>, this: u32) -> Vec<u32> {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::Array { items }) => items.clone(),
        _ => Vec::new(),
    }
}

fn array_size(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::try_from(array_items(ctx, this).len()).unwrap_or(0))
}

fn array_element(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetElement(index, out).
    if args[1] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let frames = array_items(ctx, this);
    let Some(frame) = frames.get(args[0] as usize).copied() else {
        ctx.mem.write_u32(args[1], 0).map_err(Stop::Fault)?;
        return Ok(D3DRMERR_BADVALUE);
    };
    ctx.state.com_ref(frame, 1);
    ctx.mem.write_u32(args[1], frame).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// A viewport's view of its scene, to be changed.
fn with_view(
    ctx: &mut Ctx<'_>,
    this: u32,
    change: impl FnOnce(&mut super::scene::View) -> u32,
) -> Result<u32, Stop> {
    let identity = ctx.state.identity(this);
    match ctx.state.scene.viewports.get_mut(&identity) {
        Some(view) => Ok(change(view)),
        None => Ok(D3DRMERR_BADOBJECT),
    }
}

fn view(ctx: &Ctx<'_>, this: u32) -> Option<super::scene::View> {
    ctx.state
        .scene
        .viewports
        .get(&ctx.state.identity(this))
        .cloned()
}

fn viewport_set_front(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let front = value(args[0]);
    with_view(ctx, this, |v| {
        if front <= 0.0 {
            return D3DRMERR_BADVALUE;
        }
        v.front = front;
        S_OK
    })
}

fn viewport_set_back(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let back = value(args[0]);
    with_view(ctx, this, |v| {
        if back <= v.front {
            return D3DRMERR_BADVALUE;
        }
        v.back = back;
        S_OK
    })
}

fn viewport_set_field(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let field = value(args[0]);
    with_view(ctx, this, |v| {
        if field <= 0.0 {
            return D3DRMERR_BADVALUE;
        }
        v.plane = [-field, field, -field, field];
        S_OK
    })
}

fn viewport_set_plane(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetPlane(left, right, bottom, top).
    let plane = [
        value(args[0]),
        value(args[1]),
        value(args[2]),
        value(args[3]),
    ];
    with_view(ctx, this, |v| {
        v.plane = plane;
        S_OK
    })
}

fn viewport_set_projection(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_view(ctx, this, |v| {
        v.projection = args[0];
        S_OK
    })
}

fn viewport_set_uniform_scaling(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetUniformScaling(TRUE) is how a viewport starts (`View::perspective`).
    // FALSE scales the view "exactly into the window", across and up apart,
    // which no title has asked for.
    if args[0] == 0 {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::SetUniformScaling(FALSE)".to_owned(),
            caller: 0,
        });
    }
    with_view(ctx, this, |_| S_OK)
}

fn viewport_set_camera(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[0] == 0 || !frame_or_bad(ctx, args[0]) {
        return Ok(D3DRMERR_BADOBJECT);
    }
    match ctx.state.com_mut(this).map(|o| &mut o.state) {
        Some(ComState::Viewport { camera, .. }) => {
            *camera = args[0];
            Ok(S_OK)
        }
        _ => Ok(D3DRMERR_BADOBJECT),
    }
}

/// A viewport's answer that is a float, handed back on the floating-point
/// stack where the caller reads it.
fn answer_float(
    ctx: &mut Ctx<'_>,
    this: u32,
    pick: fn(&super::scene::View) -> f32,
) -> Result<u32, Stop> {
    // A caller reading a float pops the floating-point stack whatever the
    // call did, so there is no refusal to give: a viewport this machine did
    // not make is a wall.
    let Some(view) = view(ctx, this) else {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DRMViewport2: {this:#x} has no view to read a float from"),
            caller: 0,
        });
    };
    ctx.float_answer = Some(pick(&view));
    Ok(0)
}

fn viewport_get_front(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    answer_float(ctx, this, |v| v.front)
}

fn viewport_get_back(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    answer_float(ctx, this, |v| v.back)
}

fn viewport_get_field(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    answer_float(ctx, this, |v| {
        let [left, right, ..] = v.plane;
        crate::cpu::x87::answer_single((right - left) / 2.0, [left, right])
    })
}

fn viewport_get_plane(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(view) = view(ctx, this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    for (at, v) in args.iter().zip(view.plane) {
        ctx.mem.write_u32(*at, v.to_bits()).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn viewport_get_projection(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(view(ctx, this).map_or(0, |v| v.projection))
}

fn viewport_state(ctx: &Ctx<'_>, this: u32) -> Option<ComState> {
    ctx.state.com(this).map(|o| o.state.clone())
}

/// What a viewport sees through: its view, its rectangle, and where its
/// camera is in its scene.
fn looking(
    ctx: &Ctx<'_>,
    method: &str,
    this: u32,
) -> Result<Option<(super::scene::View, super::scene::Rect, Matrix)>, Stop> {
    let (Some(ComState::Viewport { camera, .. }), Some(view)) =
        (viewport_state(ctx, this), view(ctx, this))
    else {
        return Ok(None);
    };
    if view.projection != 0 {
        return Err(Stop::NotWritten {
            what: format!(
                "IDirect3DRMViewport2::{method}: projection {}, which is not perspective",
                view.projection
            ),
            caller: 0,
        });
    }
    let camera = ctx.state.scene.world(ctx.state.identity(camera));
    Ok(camera.map(|camera| (view, viewport_rect(ctx, this), camera)))
}

fn viewport_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Transform(destination, source): a point in the scene, on the device, as
    // four values to divide by the fourth.
    let Some((view, rect, camera)) = looking(ctx, "Transform", this)? else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let point = read_vector(ctx, args[1])?;
    let Some(screen) = view.to_screen(rect, &camera, point) else {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Transform: a camera that flattens space".to_owned(),
            caller: 0,
        });
    };
    for (i, v) in screen.iter().enumerate() {
        ctx.mem
            .write_u32(args[0] + 4 * i as u32, v.to_bits())
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn viewport_inverse_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // InverseTransform(destination, source): a place on the device, as four
    // values, back into the scene.
    let Some((view, rect, camera)) = looking(ctx, "InverseTransform", this)? else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let mut screen = [0.0; 4];
    for (i, v) in screen.iter_mut().enumerate() {
        *v = value(
            ctx.mem
                .read_u32(args[1] + 4 * i as u32)
                .map_err(Stop::Fault)?,
        );
    }
    let Some(point) = view.from_screen(rect, &camera, screen) else {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DRMViewport2::InverseTransform: {screen:?}, which is no point"),
            caller: 0,
        });
    };
    write_vector(ctx, args[0], point)?;
    Ok(S_OK)
}

/// Pick(x, y, out): every mesh the viewport draws over a place on the device,
/// nearest first, each with the frames from its scene's root down to the one
/// it hangs on (#124). LEGO Island asks at every press of the button, and
/// takes the first frame below the root on the nearest one's path that it
/// hung data of its own on as what was pressed.
///
/// A face is under the place where `Render` would draw it there: cut to the
/// view volume and culled as `Render` cuts and culls it, a pixel's centre
/// being its whole coordinates. Each mesh is picked once, at its nearest face,
/// and its `D3DRMPICKDESC` says which face of which group, and where.
// ponytail: "where" is the place picked in the scene's own space. `Pick`'s
// documentation says only "the position of the picked object", and nothing on
// the roster reads it; measure native retained mode when a title does.
fn pick(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let at = (args[0] as i32 as f32, args[1] as i32 as f32);
    if args[2] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let (Some((view, rect, camera)), Some(ComState::Viewport { camera: eye, .. })) =
        (looking(ctx, "Pick", this)?, viewport_state(ctx, this))
    else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    let Some(seen_from) = super::scene::invert(&camera) else {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport::Pick: a camera that flattens space".to_owned(),
            caller: 0,
        });
    };
    let clip_from_world = super::scene::multiply(&seen_from, &view.perspective(rect.2, rect.3));
    let root = ctx.state.scene.root(ctx.state.identity(eye)).unwrap_or(0);
    let mut meshes = Vec::new();
    if let Some(world) = ctx.state.scene.world(root) {
        collect_paths(ctx, root, world, &mut Vec::new(), &mut meshes)?;
    }
    let place = super::pipeline::Viewport::whole(rect.0, rect.1, rect.2, rect.3);
    let mut found = Vec::new();
    for (world, path, visual, mesh) in meshes {
        let clip_from_model = super::scene::multiply(&world, &clip_from_world);
        if let Some((depth, group, face)) = nearest_face(ctx, mesh, &clip_from_model, &place, at) {
            found.push((depth, path, visual, group, face));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut picks = Vec::with_capacity(found.len());
    for (depth, path, visual, group, face) in found {
        let mut frames = Vec::with_capacity(path.len());
        for frame in path {
            frames.push(State::face(ctx, frame, &FRAME).map_err(Stop::Fault)?);
        }
        let array = State::make_com(ctx, &FRAME_ARRAY, ComState::Array { items: frames })
            .map_err(Stop::Fault)?;
        let [x, y, z] = view
            .from_screen(rect, &camera, [at.0, at.1, depth, 1.0])
            .unwrap_or_default()
            .map(f32::to_bits);
        picks.push((visual, array, [face, group, x, y, z]));
    }
    let picked = picks.len();
    ctx.log.at(crate::cli::Trace::Shims, || {
        format!("picked {picked} meshes")
    });
    let array = State::make_com(ctx, &PICKED_ARRAY, ComState::PickedArray { picks })
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], array).map_err(Stop::Fault)?;
    Ok(S_OK)
}

/// Every mesh in a frame's tree as `Render` finds them, each where it is in
/// the world, with the frames from the scene's root down to the one it hangs
/// on, a frame hung as a visual among them, the visual as it was hung, and the
/// mesh.
fn collect_paths(
    ctx: &Ctx<'_>,
    frame: u32,
    world: Matrix,
    path: &mut Vec<u32>,
    meshes: &mut Vec<(Matrix, Vec<u32>, u32, u32)>,
) -> Result<(), Stop> {
    let Some(node) = ctx.state.scene.frame(frame) else {
        return Ok(());
    };
    if path.len() > ctx.state.scene.frames.len() {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport::Pick: a frame that is its own ancestor".to_owned(),
            caller: 0,
        });
    }
    path.push(frame);
    let local = |f: u32| {
        ctx.state
            .scene
            .frames
            .get(&f)
            .map_or(super::scene::IDENTITY, |f| f.local)
    };
    for &visual in &node.visuals {
        let identity = ctx.state.identity(visual);
        if ctx.state.scene.meshes.contains_key(&identity) {
            meshes.push((world, path.clone(), visual, identity));
        } else if ctx.state.scene.frames.contains_key(&identity) {
            let placed = super::scene::multiply(&local(identity), &world);
            collect_paths(ctx, identity, placed, path, meshes)?;
        } else {
            return Err(Stop::NotWritten {
                what: format!(
                    "IDirect3DRMViewport::Pick: a {} visual, which nothing here picks",
                    ctx.state.interface_of(visual).unwrap_or("foreign")
                ),
                caller: 0,
            });
        }
    }
    for &child in &node.children {
        let placed = super::scene::multiply(&local(child), &world);
        collect_paths(ctx, child, placed, path, meshes)?;
    }
    path.pop();
    Ok(())
}

/// The nearest face of a mesh over a place on the device, if any: its depth,
/// its group and its number in the group.
fn nearest_face(
    ctx: &Ctx<'_>,
    mesh: u32,
    clip_from_model: &Matrix,
    place: &super::pipeline::Viewport,
    (x, y): (f32, f32),
) -> Option<(f32, u32, u32)> {
    use super::pipeline::{self, Clipped};
    let mut nearest: Option<(f32, u32, u32)> = None;
    for (g, group) in (0u32..).zip(&ctx.state.scene.meshes.get(&mesh)?.groups) {
        let corner = |index: &u32| {
            let vertex = group.vertices.get(*index as usize).copied().flatten()?;
            let position = [value(vertex[0]), value(vertex[1]), value(vertex[2]), 1.0];
            Some(Clipped {
                clip: pipeline::project(clip_from_model, position),
                diffuse: [0.0; 4],
                specular: [0.0; 3],
                uv: [0.0; 2],
                fog: 1.0,
            })
        };
        let faces = group.indices.chunks_exact(group.per_face.max(1) as usize);
        for (f, face) in (0u32..).zip(faces) {
            let Some(corners) = face.iter().map(corner).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let polygon: Vec<_> = pipeline::clip(&corners)
                .iter()
                .map(|v| place.place(v))
                .collect();
            // Wound anticlockwise on the screen, it is turned away and not
            // drawn (`d3d::rasterise`'s D3DCULL_CCW).
            let area: f32 = (0..polygon.len())
                .map(|p| {
                    let (u, v) = (polygon[p], polygon[(p + 1) % polygon.len()]);
                    u.x * v.y - v.x * u.y
                })
                .sum();
            if polygon.len() < 3 || area < 0.0 {
                continue;
            }
            let edge = |a: crate::raster::Vertex, b: crate::raster::Vertex| {
                (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
            };
            for p in 1..polygon.len() - 1 {
                let (a, b, c) = (polygon[0], polygon[p], polygon[p + 1]);
                let (wa, wb, wc) = (edge(b, c), edge(c, a), edge(a, b));
                let total = wa + wb + wc;
                if total <= 0.0 || wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let depth = (wa * a.z + wb * b.z + wc * c.z) / total;
                if nearest.is_none_or(|(d, ..)| depth < d) {
                    nearest = Some((depth, g, f));
                }
            }
        }
    }
    nearest
}

fn picks(ctx: &Ctx<'_>, this: u32) -> Vec<(u32, u32, [u32; 5])> {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::PickedArray { picks }) => picks.clone(),
        _ => Vec::new(),
    }
}

fn picked_size(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::try_from(picks(ctx, this).len()).unwrap_or(0))
}

fn picked_element(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetPick(index, visual, frames, description).
    let (visual_at, frames_at, description) = (args[1], args[2], args[3]);
    let Some((visual, frames, words)) = picks(ctx, this).get(args[0] as usize).copied() else {
        return Ok(D3DRMERR_BADVALUE);
    };
    if visual_at == 0 || frames_at == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    for (at, object) in [(visual_at, visual), (frames_at, frames)] {
        ctx.state.com_ref(object, 1);
        ctx.mem.write_u32(at, object).map_err(Stop::Fault)?;
    }
    if description != 0 {
        for (i, word) in (0u32..).zip(words) {
            ctx.mem
                .write_u32(description + 4 * i, word)
                .map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn viewport_get_camera(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(ComState::Viewport { camera, .. }) = viewport_state(ctx, this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    give_frame(ctx, args[0], camera)
}

fn viewport_get_device(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(ComState::Viewport { device, .. }) = viewport_state(ctx, this) else {
        return Ok(D3DRMERR_BADOBJECT);
    };
    if args[0] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    ctx.state.com_ref(device, 1);
    ctx.mem.write_u32(args[0], device).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn viewport_rect(ctx: &Ctx<'_>, this: u32) -> super::scene::Rect {
    match viewport_state(ctx, this) {
        Some(ComState::Viewport {
            x,
            y,
            width,
            height,
            ..
        }) => (x, y, width, height),
        _ => (0, 0, 0, 0),
    }
}

fn viewport_get_x(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(viewport_rect(ctx, this).0)
}

fn viewport_get_y(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(viewport_rect(ctx, this).1)
}

fn viewport_get_width(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(viewport_rect(ctx, this).2)
}

fn viewport_get_height(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(viewport_rect(ctx, this).3)
}
