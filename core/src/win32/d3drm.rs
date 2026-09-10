//! Direct3D Retained Mode.
//!
//! The boundary sits above it (ADR-0002): a title hands D3DRM a scene made of
//! frames, meshes and lights, and asks it to draw. What draws is this
//! emulator's own rasteriser, into the guest's own surface.
//!
//! Every table is the interface's full width, named and sized from its own
//! header. A slot is written when this title has called it; the rest stop the
//! run and name the method (#11).

use super::scene::Matrix;
use super::{ComState, Convention, Ctx, Shim, State};
use crate::com::{method, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

pub const SHIMS: &[Shim] = &[
    Shim {
        dll: "d3drm",
        name: "Direct3DRMCreate",
        args: 1,
        convention: Convention::Stdcall,
        run: create,
    },
    Shim {
        dll: "d3drm",
        name: "D3DRMVectorRotate",
        args: 4,
        convention: Convention::Stdcall,
        run: vector_rotate,
    },
    Shim {
        dll: "d3drm",
        name: "D3DRMVectorRandom",
        args: 1,
        convention: Convention::Stdcall,
        run: vector_random,
    },
];

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

/// `IDirect3DRM`: what `Direct3DRMCreate` hands out. This title asks it for its DirectX 6 face straight away, and nothing else of it is written.
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

/// `IDirect3DRM3`: DirectX 6's retained mode, which this title drives.
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
        method("CreateTextureFromSurface", 2, create_texture_from_surface),
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
    ],
};

/// `IDirect3DRMViewport2`: a camera's view onto part of a device.
pub static VIEWPORT2: Interface = Interface {
    name: "IDirect3DRMViewport2",
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
        wall("ForceUpdate", 4),
        method("SetPlane", 4, viewport_set_plane),
        method("GetCamera", 1, viewport_get_camera),
        method("GetDevice", 1, viewport_get_device),
        method("GetPlane", 4, viewport_get_plane),
        wall("Pick", 3),
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
    ],
};

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
        method("SetSceneFogColor", 1, set_scene_fog_color),
        method("SetSceneFogMode", 1, set_scene_fog_mode),
        method("SetSceneFogParams", 3, set_scene_fog_params),
        method("SetColor", 1, set_color),
        method("SetColorRGB", 3, set_color_rgb),
        method("GetZbufferMode", 0, get_zbuffer_mode),
        method("SetMaterialMode", 1, set_material_mode),
        method("SetOrientation", 7, set_orientation),
        method("SetPosition", 4, set_position),
        wall("SetRotation", 5),
        method("SetSortMode", 1, set_sort_mode),
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
        method("SetSceneFogMethod", 1, set_scene_fog_method),
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

/// `IDirect3DRMMesh`: geometry in groups, ready to draw.
pub static MESH: Interface = Interface {
    name: "IDirect3DRMMesh",
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
        wall("Scale", 3),
        wall("Translate", 3),
        wall("GetBox", 1),
        wall("AddGroup", 5),
        wall("SetVertices", 4),
        wall("SetGroupColor", 2),
        wall("SetGroupColorRGB", 4),
        wall("SetGroupMapping", 2),
        wall("SetGroupQuality", 2),
        wall("SetGroupMaterial", 2),
        wall("SetGroupTexture", 2),
        wall("GetGroupCount", 0),
        wall("GetGroup", 6),
        wall("GetVertices", 4),
        wall("GetGroupColor", 1),
        wall("GetGroupMapping", 1),
        wall("GetGroupQuality", 1),
        wall("GetGroupMaterial", 2),
        wall("GetGroupTexture", 2),
    ],
};

/// `IDirect3DRMTexture3`: an image a mesh is drawn with.
pub static TEXTURE3: Interface = Interface {
    name: "IDirect3DRMTexture3",
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
    ],
};

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
        method("SetSpecular", 3, material_unread),
        method("SetEmissive", 3, material_unread),
        wall("GetPower", 0),
        wall("GetSpecular", 3),
        wall("GetEmissive", 3),
        wall("GetAmbient", 3),
        wall("SetAmbient", 3),
    ],
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

/// `IDirect3DRMFrame`: DirectX 3's face of a frame, which is what a frame array hands out; this title only asks it for the newer face.
pub static FRAME: Interface = Interface {
    name: "IDirect3DRMFrame",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        wall("Clone", 3),
        wall("AddDestroyCallback", 2),
        wall("DeleteDestroyCallback", 2),
        wall("SetAppData", 1),
        wall("GetAppData", 0),
        wall("SetName", 1),
        wall("GetName", 2),
        wall("GetClassName", 2),
        wall("AddChild", 1),
        wall("AddLight", 1),
        wall("AddMoveCallback", 2),
        wall("AddTransform", 2),
        wall("AddTranslation", 4),
        wall("AddScale", 4),
        wall("AddRotation", 5),
        wall("AddVisual", 1),
        wall("GetChildren", 1),
        wall("GetColor", 0),
        wall("GetLights", 1),
        wall("GetMaterialMode", 0),
        wall("GetParent", 1),
        wall("GetPosition", 2),
        wall("GetRotation", 3),
        wall("GetScene", 1),
        wall("GetSortMode", 0),
        wall("GetTexture", 1),
        wall("GetTransform", 1),
        wall("GetVelocity", 3),
        wall("GetOrientation", 3),
        wall("GetVisuals", 1),
        wall("GetTextureTopology", 2),
        wall("InverseTransform", 2),
        wall("Load", 5),
        wall("LookAt", 3),
        wall("Move", 1),
        wall("DeleteChild", 1),
        wall("DeleteLight", 1),
        wall("DeleteMoveCallback", 2),
        wall("DeleteVisual", 1),
        wall("GetSceneBackground", 0),
        wall("GetSceneBackgroundDepth", 1),
        wall("GetSceneFogColor", 0),
        wall("GetSceneFogEnable", 0),
        wall("GetSceneFogMode", 0),
        wall("GetSceneFogParams", 3),
        wall("SetSceneBackground", 1),
        wall("SetSceneBackgroundRGB", 3),
        wall("SetSceneBackgroundDepth", 1),
        wall("SetSceneBackgroundImage", 1),
        wall("SetSceneFogEnable", 1),
        wall("SetSceneFogColor", 1),
        wall("SetSceneFogMode", 1),
        wall("SetSceneFogParams", 3),
        wall("SetColor", 1),
        wall("SetColorRGB", 3),
        wall("GetZbufferMode", 0),
        wall("SetMaterialMode", 1),
        wall("SetOrientation", 7),
        wall("SetPosition", 4),
        wall("SetRotation", 5),
        wall("SetSortMode", 1),
        wall("SetTexture", 1),
        wall("SetTextureTopology", 2),
        wall("SetVelocity", 5),
        wall("SetZbufferMode", 1),
        wall("Transform", 2),
    ],
};

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

pub static INTERFACES: &[&Interface] = &[
    &D3DRM,
    &D3DRM3,
    &DEVICE3,
    &VIEWPORT2,
    &FRAME3,
    &MESH_BUILDER3,
    &MESH,
    &TEXTURE3,
    &LIGHT,
    &MATERIAL2,
    &USER_VISUAL,
    &FRAME,
    &FRAME_ARRAY,
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
    // CreateFrame(parent, out).
    let object = State::make_com(ctx, &FRAME3, ComState::None).map_err(Stop::Fault)?;
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
    make(ctx, &MESH, args[0])
}

fn create_mesh_builder(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    make(ctx, &MESH_BUILDER3, args[0])
}

fn create_texture(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateTexture(image, out).
    make(ctx, &TEXTURE3, args[1])
}

fn create_texture_from_surface(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateTextureFromSurface(surface, out).
    make(ctx, &TEXTURE3, args[1])
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
    // CreateMaterial(power, out).
    make(ctx, &MATERIAL2, args[1])
}

fn material_unread(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // SetSpecular and SetEmissive(red, green, blue). Only a retained-mode mesh
    // is drawn with a retained-mode material, and Render stops at a mesh, so
    // nothing here would read what they set.
    Ok(S_OK)
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

fn create_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateViewport(device, camera, x, y, width, height, out).
    let object = State::make_com(
        ctx,
        &VIEWPORT2,
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
    // The immediate-mode device underneath: what this title actually draws
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
    // is TNT2-class. Rock Raiders asks two things: an eight-bit palettized
    // format, which a TNT2 does not have, and then any sixteen-bit one, which
    // is 5-6-5, the screen's own. What else a TNT2 offers has not been asked.
    const DDBD_8: u32 = 0x0800;
    const DDBD_16: u32 = 0x0400;
    const D3DRMFPTF_PALETTIZED: u32 = 0x4;
    let (depths, flags, out) = (args[0], args[1], args[2]);
    match (depths, flags) {
        (DDBD_8, D3DRMFPTF_PALETTIZED) => Ok(D3DRMERR_TEXTUREFORMATNOTFOUND),
        (DDBD_16, 0) => {
            // DDPIXELFORMAT: size, DDPF_RGB, no FourCC, sixteen bits, masks.
            for (offset, value) in [
                (0, 32),
                (4, 0x40),
                (8, 0),
                (12, 16),
                (16, 0xf800),
                (20, 0x07e0),
                (24, 0x001f),
                (28, 0),
            ] {
                ctx.mem.write_u32(out + offset, value).map_err(Stop::Fault)?;
            }
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

fn update(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Retained mode brings the device's surface up to date with its
    // viewports. That is drawing, not presenting: the frame reaches the screen
    // when the title flips.
    Ok(S_OK)
}

fn clear(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear(flags). The viewport's own part of the device's surface goes to
    // the background colour of the scene its camera is in. A depth buffer or
    // dirty rectangles are not here to clear.
    const D3DRMCLEAR_TARGET: u32 = 0x1;
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
    if args[0] != D3DRMCLEAR_TARGET {
        return Err(Stop::NotWritten {
            what: format!("IDirect3DRMViewport2::Clear: flags {:#x}", args[0]),
            caller: 0,
        });
    }
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
    let depth = format.bits;
    super::fill(
        ctx,
        (pixels, pitch, depth),
        (x, y, (x + width).min(sw), (y + height).min(sh)),
        background,
    )?;
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
    pass.lights
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
    let mut visits = Vec::new();
    let scene = ctx.state.identity(args[0]);
    // A scene's fog is kept when a title sets it up, and nothing here draws it.
    let root = ctx
        .state
        .scene
        .root(scene)
        .and_then(|r| ctx.state.scene.frame(r));
    if root.is_some_and(|f| f.fog.enabled) {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Render: a scene with its fog on".to_owned(),
            caller: 0,
        });
    }
    collect_visits(ctx, scene, &mut visits, 0)?;
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
    let (_, _, width, height) = viewport_rect(ctx, this);
    super::d3d::set_device_transform(ctx.state, immediate, VIEW, &camera_world);
    super::d3d::set_device_transform(
        ctx.state,
        immediate,
        PROJECTION,
        &view.perspective(width, height),
    );
    super::d3d::set_device_viewport(ctx.state, immediate, under);
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
        let half_cos = |angle: f32| crate::cpu::x87::maths::cos(f64::from(angle) / 2.0) as f32;
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
                inner: half_cos(light.umbra.unwrap_or(0.4)),
                outer: half_cos(light.penumbra.unwrap_or(0.8)),
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

/// Every user visual in a frame's tree: the frame's own, then its children's.
fn collect_visits(
    ctx: &Ctx<'_>,
    frame: u32,
    visits: &mut Vec<Visit>,
    depth: usize,
) -> Result<(), Stop> {
    let Some(node) = ctx.state.scene.frame(frame) else {
        return Ok(());
    };
    // A frame that is its own ancestor has no place in any scene.
    if depth > ctx.state.scene.frames.len() || ctx.state.scene.world(frame).is_none() {
        return Err(Stop::NotWritten {
            what: "IDirect3DRMViewport2::Render: a frame that is its own ancestor".to_owned(),
            caller: 0,
        });
    }
    for &visual in &node.visuals {
        match ctx.state.com(visual).map(|o| &o.state) {
            Some(&ComState::UserVisual { callback, argument }) => visits.push(Visit {
                frame,
                callback,
                argument,
                visual,
            }),
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
        collect_visits(ctx, child, visits, depth + 1)?;
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

fn set_sort_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.sort_mode = args[0])
}

/// Where a reference frame the guest passed puts things in the scene.
///
/// A null reference is what this title passes for a frame with no parent,
/// having asked the frame for its parent first; for a frame at the top of its
/// scene, relative to nothing and relative to the scene are the same thing.
/// What a null means for a frame that has a parent has not been measured.
fn reference(ctx: &Ctx<'_>, method: &str, frame: u32, at: u32) -> Result<Matrix, Stop> {
    if at == 0 {
        if ctx.state.scene.frame(frame).is_some_and(|f| f.parent == 0) {
            return Ok(super::scene::IDENTITY);
        }
        return Err(Stop::NotWritten {
            what: format!(
                "IDirect3DRMFrame3::{method}: a null reference for a frame with a parent"
            ),
            caller: 0,
        });
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
        &reference(ctx, method, frame, at)?,
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
        super::scene::invert(&self::reference(ctx, method, this, reference)?).ok_or_else(|| {
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
    with_frame(ctx, this, |f| f.fog.enabled = args[0] != 0)
}

fn set_scene_fog_color(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.fog.color = args[0])
}

fn set_scene_fog_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // D3DRMFOG_LINEAR, _EXPONENTIAL and _EXPONENTIALSQUARED.
    if args[0] > 2 {
        return Ok(D3DRMERR_BADVALUE);
    }
    with_frame(ctx, this, |f| f.fog.mode = args[0])
}

fn set_scene_fog_params(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetSceneFogParams(start, end, density).
    let (start, end, density) = (value(args[0]), value(args[1]), value(args[2]));
    with_frame(ctx, this, |f| {
        f.fog.start = start;
        f.fog.end = end;
        f.fog.density = density;
    })
}

fn set_scene_fog_method(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // D3DRMFOGMETHOD_VERTEX, _TABLE or _ANY: where a card computes fog. Kept
    // for reading back; a scene that turns its fog on stops at Render.
    with_frame(ctx, this, |f| f.fog.method = args[0])
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

fn set_material_mode(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    with_frame(ctx, this, |f| f.material_mode = args[0])
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
        "IDirect3DRMFrame3" => "Frame",
        "IDirect3DRMDevice3" => "Device",
        "IDirect3DRMViewport2" => "Viewport",
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
    let mut faces = Vec::with_capacity(children.len());
    for child in children {
        faces.push(State::face(ctx, child, &FRAME).map_err(Stop::Fault)?);
    }
    let array = State::make_com(ctx, &FRAME_ARRAY, ComState::FrameArray { frames: faces })
        .map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], array).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn array_frames(ctx: &Ctx<'_>, this: u32) -> Vec<u32> {
    match ctx.state.com(this).map(|o| &o.state) {
        Some(ComState::FrameArray { frames }) => frames.clone(),
        _ => Vec::new(),
    }
}

fn array_size(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(u32::try_from(array_frames(ctx, this).len()).unwrap_or(0))
}

fn array_element(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetElement(index, out).
    if args[1] == 0 {
        return Ok(D3DRMERR_BADVALUE);
    }
    let frames = array_frames(ctx, this);
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
    with_view(ctx, this, |v| {
        v.uniform_scaling = Some(args[0] != 0);
        S_OK
    })
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
    answer_float(ctx, this, |v| (v.plane[1] - v.plane[0]) / 2.0)
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
