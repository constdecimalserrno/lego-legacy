//! Direct3D's immediate mode.
//!
//! What the roster's titles actually draw through: they take the device
//! underneath retained mode and hand it primitives themselves. Everything it
//! draws lands in the guest's own surface (ADR-0002), so what the guest reads
//! back is what was drawn.
//!
//! Every table is the interface's full width from its header; a slot nobody has
//! called stops the run by name (#11).

use super::{ComState, Ctx, State};
use crate::com::{self, method, wall, Interface, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
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
    devices: Vec<(u32, Device)>,
    /// Materials' own bytes, a `D3DMATERIAL` each.
    materials: BTreeMap<u32, Vec<u8>>,
    /// Which material or texture a handle names: one table, as Wine's ddraw
    /// keeps one, read as documentation.
    handles: BTreeMap<u32, u32>,
    next_handle: u32,
    /// Viewports' own descriptions, as the guest last set them.
    viewports: BTreeMap<u32, Vec<u8>>,
    scratch: DrawScratch,
}

/// Read-only summary of one immediate-mode Direct3D device (#309).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    pub addr: u32,
    pub in_scene: bool,
    pub current_viewport: u32,
    pub render_states_set: usize,
    pub active_textures: Vec<(u32, u32)>,
}

impl Immediate {
    /// Read-only summary of every immediate-mode Direct3D device (#309).
    #[must_use]
    pub fn device_summaries(&self) -> Vec<DeviceSummary> {
        self.devices
            .iter()
            .map(|(addr, dev)| {
                let render_states_set = dev
                    .render_set
                    .iter()
                    .map(|w| w.count_ones() as usize)
                    .sum::<usize>()
                    + dev.render_extra.len();
                let mut active_textures = Vec::new();
                for (stage, &tex) in dev.textures.iter().enumerate() {
                    if tex != 0 {
                        active_textures.push((stage as u32, tex));
                    }
                }
                for (&stage, &tex) in &dev.textures_extra {
                    if tex != 0 {
                        active_textures.push((stage, tex));
                    }
                }
                DeviceSummary {
                    addr: *addr,
                    in_scene: dev.in_scene,
                    current_viewport: dev.current_viewport,
                    render_states_set,
                    active_textures,
                }
            })
            .collect()
    }

    #[must_use]
    pub fn materials_count(&self) -> usize {
        self.materials.len()
    }

    #[must_use]
    pub fn viewports_count(&self) -> usize {
        self.viewports.len()
    }

    pub(crate) fn remove_handle_for(&mut self, identity: u32) {
        self.handles.retain(|_, id| *id != identity);
    }

    /// Drain all texture pointers bound to `identity` when the device drops to
    /// zero references so their COM refcounts are released cleanly (#338).
    pub(crate) fn take_device_textures(&mut self, identity: u32) -> Vec<u32> {
        let mut out = Vec::new();
        if let Some((_, dev)) = self.devices.iter_mut().find(|(id, _)| *id == identity) {
            for slot in &mut dev.textures {
                let tex = std::mem::take(slot);
                if tex != 0 {
                    out.push(tex);
                }
            }
            for (_, tex) in std::mem::take(&mut dev.textures_extra) {
                if tex != 0 {
                    out.push(tex);
                }
            }
        }
        out
    }
}

/// Reusable per-draw buffers kept across immediate-mode and retained-mode
/// draws so hot frames do not allocate or zero-initialise on every draw call.
#[derive(Default)]
struct DrawScratch {
    indices: Vec<u32>,
    lights: Vec<super::pipeline::Light>,
    prepared: Vec<super::pipeline::Clipped>,
    triangles: Vec<[super::pipeline::Clipped; 3]>,
    raster: Vec<[crate::raster::Vertex; 3]>,
    clip_buf: [super::pipeline::Clipped; super::pipeline::CLIP_BUFFER],
    clip_scratch: [super::pipeline::Clipped; super::pipeline::CLIP_BUFFER],
    poly_buf: [crate::raster::Vertex; super::pipeline::CLIP_BUFFER],
}

/// Flat-table bounds for Direct3D 5/6/7 state indices; out-of-range states
/// spill into `*_extra` maps so uncommon indices still round-trip.
const RENDER_STATE_SLOTS: usize = 160;
const STAGE_SLOTS: usize = 8;
const STAGE_STATE_SLOTS: usize = 32;
const TRANSFORM_SLOTS: usize = 4;
const LIGHT_STATE_SLOTS: usize = 8;

const IDENTITY_BITS: [u32; 16] = {
    let one = 1.0f32.to_bits();
    [one, 0, 0, 0, 0, one, 0, 0, 0, 0, one, 0, 0, 0, 0, one]
};

/// One device's state: everything a title sets and may read back.
struct Device {
    render: [u32; RENDER_STATE_SLOTS],
    render_set: [u64; 3],
    render_extra: BTreeMap<u32, u32>,
    light: [u32; LIGHT_STATE_SLOTS],
    light_set: u8,
    light_extra: BTreeMap<u32, u32>,
    transforms: [[u32; 16]; TRANSFORM_SLOTS],
    transforms_extra: BTreeMap<u32, [u32; 16]>,
    /// Cached `VIEW * PROJECTION`, invalidated when either transform changes.
    view_proj: Option<super::scene::Matrix>,
    textures: [u32; STAGE_SLOTS],
    textures_extra: BTreeMap<u32, u32>,
    stages: [[u32; STAGE_STATE_SLOTS]; STAGE_SLOTS],
    stages_set: [u32; STAGE_SLOTS],
    stages_extra: BTreeMap<(u32, u32), u32>,
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

impl Default for Device {
    fn default() -> Self {
        Self {
            render: std::array::from_fn(|s| render_state_at_start(s as u32).unwrap_or(0)),
            render_set: [0; 3],
            render_extra: BTreeMap::new(),
            light: std::array::from_fn(|s| light_state_at_start(s as u32).unwrap_or(0)),
            light_set: 0,
            light_extra: BTreeMap::new(),
            transforms: [IDENTITY_BITS; TRANSFORM_SLOTS],
            transforms_extra: BTreeMap::new(),
            view_proj: None,
            textures: [0; STAGE_SLOTS],
            textures_extra: BTreeMap::new(),
            stages: std::array::from_fn(|stage| {
                std::array::from_fn(|state| {
                    stage_state_at_start(state as u32, stage as u32).unwrap_or(0)
                })
            }),
            stages_set: [0; STAGE_SLOTS],
            stages_extra: BTreeMap::new(),
            current_viewport: 0,
            viewport7: [0; 6],
            in_scene: false,
            direct3d: 0,
            depth: Vec::new(),
        }
    }
}

impl crate::snap::Snap for Device {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            render,
            render_set,
            render_extra,
            light,
            light_set,
            light_extra,
            transforms,
            transforms_extra,
            view_proj,
            textures,
            textures_extra,
            stages,
            stages_set,
            stages_extra,
            current_viewport,
            viewport7,
            in_scene,
            direct3d,
            depth,
        } = self;
        for r in render {
            v.u32(r);
        }
        for w in render_set {
            v.u64(w);
        }
        crate::snap::snap_btree_map(v, render_extra, |k, v| v.u32(k), |val, v| v.u32(val));
        for l in light {
            v.u32(l);
        }
        v.u8(light_set);
        crate::snap::snap_btree_map(v, light_extra, |k, v| v.u32(k), |val, v| v.u32(val));
        for m in transforms {
            for w in m {
                v.u32(w);
            }
        }
        crate::snap::snap_btree_map(
            v,
            transforms_extra,
            |k, v| v.u32(k),
            |m, v| {
                for w in m {
                    v.u32(w);
                }
            },
        );
        if v.is_reading() {
            *view_proj = None;
        }
        for t in textures {
            v.u32(t);
        }
        crate::snap::snap_btree_map(v, textures_extra, |k, v| v.u32(k), |val, v| v.u32(val));
        for st in stages {
            for val in st {
                v.u32(val);
            }
        }
        for s in stages_set {
            v.u32(s);
        }
        crate::snap::snap_btree_map(
            v,
            stages_extra,
            |(a, b), v| {
                v.u32(a);
                v.u32(b);
            },
            |val, v| v.u32(val),
        );
        v.u32(current_viewport);
        for w in viewport7 {
            v.u32(w);
        }
        v.bool(in_scene);
        v.u32(direct3d);
        crate::snap::snap_f32_vec(v, depth);
    }
}

impl crate::snap::Snap for Immediate {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            devices,
            materials,
            handles,
            next_handle,
            viewports,
            scratch: _,
        } = self;
        crate::snap::snap_vec(v, devices, |(addr, dev), v| {
            v.u32(addr);
            dev.snap(v);
        });
        crate::snap::snap_btree_map(v, materials, |k, v| v.u32(k), |b, v| v.byte_vec(b));
        crate::snap::snap_btree_map(v, handles, |k, v| v.u32(k), |val, v| v.u32(val));
        v.u32(next_handle);
        crate::snap::snap_btree_map(v, viewports, |k, v| v.u32(k), |b, v| v.byte_vec(b));
    }
}

impl Device {
    #[inline]
    fn set_render(&mut self, state: u32, value: u32) {
        let idx = state as usize;
        if idx < RENDER_STATE_SLOTS {
            self.render[idx] = value;
            self.render_set[idx >> 6] |= 1u64 << (idx & 63);
        } else {
            self.render_extra.insert(state, value);
        }
    }

    #[inline]
    fn render_is_set(&self, state: u32) -> bool {
        let idx = state as usize;
        if idx < RENDER_STATE_SLOTS {
            (self.render_set[idx >> 6] & (1u64 << (idx & 63))) != 0
        } else {
            self.render_extra.contains_key(&state)
        }
    }

    #[inline]
    fn get_render(&self, state: u32, dx7: bool) -> Option<u32> {
        let idx = state as usize;
        if idx < RENDER_STATE_SLOTS {
            if (self.render_set[idx >> 6] & (1u64 << (idx & 63))) != 0 {
                Some(self.render[idx])
            } else {
                render_state_at_start(state).filter(|_| !dx7 || state != rs::TEXTUREMAPBLEND)
            }
        } else {
            self.render_extra.get(&state).copied()
        }
    }

    #[inline]
    fn render_value(&self, state: u32) -> u32 {
        debug_assert!((state as usize) < RENDER_STATE_SLOTS);
        debug_assert!(render_state_at_start(state).is_some());
        self.render[state as usize]
    }

    #[inline]
    fn set_light(&mut self, state: u32, value: u32) {
        let idx = state as usize;
        if idx < LIGHT_STATE_SLOTS {
            self.light[idx] = value;
            self.light_set |= 1u8 << idx;
        } else {
            self.light_extra.insert(state, value);
        }
    }

    #[inline]
    fn get_light(&self, state: u32) -> Option<u32> {
        let idx = state as usize;
        if idx < LIGHT_STATE_SLOTS {
            if (self.light_set & (1u8 << idx)) != 0 {
                Some(self.light[idx])
            } else {
                light_state_at_start(state)
            }
        } else {
            self.light_extra.get(&state).copied()
        }
    }

    #[inline]
    fn set_transform(&mut self, which: u32, bits: [u32; 16]) {
        if which == VIEW || which == PROJECTION {
            self.view_proj = None;
        }
        let idx = which as usize;
        if idx < TRANSFORM_SLOTS {
            self.transforms[idx] = bits;
        } else {
            self.transforms_extra.insert(which, bits);
        }
    }

    #[inline]
    fn view_proj(&mut self) -> super::scene::Matrix {
        if let Some(vp) = self.view_proj {
            return vp;
        }
        let vp = super::scene::multiply(&matrix_of(self, VIEW), &matrix_of(self, PROJECTION));
        self.view_proj = Some(vp);
        vp
    }

    #[inline]
    fn get_transform(&self, which: u32) -> [u32; 16] {
        let idx = which as usize;
        if idx < TRANSFORM_SLOTS {
            self.transforms[idx]
        } else {
            self.transforms_extra
                .get(&which)
                .copied()
                .unwrap_or(IDENTITY_BITS)
        }
    }

    #[inline]
    fn set_texture(&mut self, stage: u32, texture: u32) -> u32 {
        let idx = stage as usize;
        if idx < STAGE_SLOTS {
            std::mem::replace(&mut self.textures[idx], texture)
        } else {
            self.textures_extra.insert(stage, texture).unwrap_or(0)
        }
    }

    #[inline]
    fn get_texture(&self, stage: u32) -> u32 {
        let idx = stage as usize;
        if idx < STAGE_SLOTS {
            self.textures[idx]
        } else {
            self.textures_extra.get(&stage).copied().unwrap_or(0)
        }
    }

    #[inline]
    fn set_stage(&mut self, stage: u32, state: u32, value: u32) {
        let (st, s) = (stage as usize, state as usize);
        if st < STAGE_SLOTS && s < STAGE_STATE_SLOTS {
            self.stages[st][s] = value;
            self.stages_set[st] |= 1u32 << s;
        } else {
            self.stages_extra.insert((stage, state), value);
        }
    }

    #[inline]
    fn get_stage(&self, stage: u32, state: u32) -> Option<u32> {
        let (st, s) = (stage as usize, state as usize);
        if st < STAGE_SLOTS && s < STAGE_STATE_SLOTS {
            if (self.stages_set[st] & (1u32 << s)) != 0 {
                Some(self.stages[st][s])
            } else {
                stage_state_at_start(state, stage)
            }
        } else {
            self.stages_extra
                .get(&(stage, state))
                .copied()
                .or_else(|| stage_state_at_start(state, stage))
        }
    }

    #[inline]
    fn stage0_value(&self, state: u32) -> u32 {
        debug_assert!((state as usize) < STAGE_STATE_SLOTS);
        debug_assert!(stage_state_at_start(state, 0).is_some());
        self.stages[0][state as usize]
    }
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
        method("GetCaps", 2, get_caps3),
        wall("GetStats", 1),
        method("AddViewport", 1, add_viewport),
        wall("DeleteViewport", 1),
        wall("NextViewport", 3),
        method("EnumTextureFormats", 2, enum_texture_formats7),
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
        method("CreateDevice", 4, create_device3),
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
    let object = com::create(
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
    match ctx.state.com.get(this).map(|o| &o.state) {
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

/// Where a draw call's vertex indices come from: either sequential `0..n`
/// for non-indexed draws, or preloaded into `Immediate::scratch.indices`.
#[derive(Clone, Copy)]
enum DrawIndices {
    Sequential(u32),
    Scratch,
}

fn fill_u16_indices(ctx: &mut Ctx<'_>, addr: u32, count: u32) -> Result<(), Stop> {
    let bytes = ctx
        .mem
        .slice(addr, count as usize * 2)
        .map_err(Stop::Fault)?;
    let indices = &mut ctx.state.immediate.scratch.indices;
    indices.clear();
    indices.extend(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u32::from(u16::from_le_bytes(*c))),
    );
    Ok(())
}

/// Draw straight out of a buffer the title filled earlier.
fn draw_primitive_vb(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // DrawPrimitiveVB(type, buffer, first vertex, count, flags).
    let Some((store, fvf, _)) = buffer_of(ctx, args[1]) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let at = store + args[2] * fvf_stride(fvf);
    draw(
        ctx,
        "IDirect3DDevice7::DrawPrimitiveVB",
        this,
        args[0],
        fvf,
        at,
        args[3],
        DrawIndices::Sequential(args[3]),
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
    fill_u16_indices(ctx, args[4], args[5])?;
    draw(
        ctx,
        "IDirect3DDevice7::DrawIndexedPrimitiveVB",
        this,
        args[0],
        fvf,
        at,
        args[3],
        DrawIndices::Scratch,
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

/// DirectX 6's `IDirect3DDevice3::GetCaps(lpD3DHWDevDesc, lpD3DHELDevDesc)` (#370).
fn get_caps3(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let size_for = |ctx: &Ctx<'_>, at: u32| -> Result<Option<u32>, Stop> {
        let caller = ctx.mem.read_u32(at).map_err(Stop::Fault)?;
        Ok(match caller {
            0 | DEVICE_DESC6_BYTES => Some(DEVICE_DESC6_BYTES),
            DEVICE_DESC_BYTES => Some(DEVICE_DESC_BYTES),
            _ => None,
        })
    };
    let hw_size = if args[0] != 0 {
        let Some(size) = size_for(ctx, args[0])? else {
            return Ok(DDERR_INVALIDPARAMS);
        };
        Some(size)
    } else {
        None
    };
    let hel_size = if args[1] != 0 {
        let Some(size) = size_for(ctx, args[1])? else {
            return Ok(DDERR_INVALIDPARAMS);
        };
        Some(size)
    } else {
        None
    };
    if let Some(size) = hw_size {
        write_device_desc(ctx, args[0], size)?;
    }
    if let Some(size) = hel_size {
        ctx.mem
            .write(
                args[1],
                &[0u8; DEVICE_DESC6_BYTES as usize][..size as usize],
            )
            .map_err(Stop::Fault)?;
        ctx.mem.write_u32(args[1], size).map_err(Stop::Fault)?;
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
    put(at + 0x18, 0x0000_00ff)?; // dwAlphaCmpCaps: all eight compare functions
    put(at + 0x1c, 0x0000_440a)?; // dwShadeCaps: flat and Gouraud in colour, with alpha
    put(at + 0x20, 0x0000_000d)?; // dwTextureCaps: perspective, alpha, and colour-key transparency
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

/// DirectX 6/7's, each format a `DDPIXELFORMAT`.
fn enum_texture_formats7(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
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
    let label = if ctx.state.com.interface_of(this) == Some(DEVICE3.name) {
        "d3d3"
    } else {
        "d3d7"
    };
    ctx.log
        .line(&format!("{label} texture formats: {} of them", calls.len()));
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
    let object = com::create(ctx, &DEVICE2, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    // A DirectX 5 device starts with specular highlights on, as Wine's
    // conformance test `test_lighting_interface_versions` finds Windows
    // starting one (#210).
    device(ctx, object).set_render(rs::SPECULARENABLE, 1);
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3d2 device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

/// DirectX 6's device, over the back buffer a title made for it: the one
/// device `EnumDevices` offered, or a wall naming the one asked for (#370).
fn create_device3(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDevice(rclsid, lpDDS, lplpD3DDevice, pUnkOuter).
    if let Some(refused) = aggregated(args[3]) {
        return Ok(refused);
    }
    if args[0] == 0 || args[1] == 0 || args[2] == 0 {
        return Ok(DDERR_INVALIDPARAMS);
    }
    let asked = crate::com::read_iid(ctx, args[0])?;
    if asked != HAL_DEVICE {
        return Err(unwritten(format!(
            "IDirect3D3::CreateDevice: device {}, which was never offered",
            crate::com::iid_text(&asked)
        )));
    }
    let surface = args[1];
    let object = com::create(ctx, &DEVICE3, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    if this != 0 {
        device(ctx, object).direct3d = this;
    }
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3d3 device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

/// The device a title draws through, over a surface of its own.
fn create_device7(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateDevice(guid, surface, out). The surface is the guest's own, which
    // is what makes what the device draws guest-visible.
    let surface = args[1];
    let object = com::create(ctx, &DEVICE7, ComState::Device3D { surface }).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[2], object).map_err(Stop::Fault)?;
    ctx.log.line(&format!(
        "d3d7 device at {object:#010x} drawing into surface {surface:#010x}"
    ));
    Ok(S_OK)
}

fn device<'a>(ctx: &'a mut Ctx<'_>, this: u32) -> &'a mut Device {
    device_of(ctx.state, this)
}

fn device_index_by_id(devices: &mut Vec<(u32, Device)>, identity: u32) -> usize {
    devices
        .iter()
        .position(|(id, _)| *id == identity)
        .unwrap_or_else(|| {
            devices.push((identity, Device::default()));
            devices.len() - 1
        })
}

fn device_index(state: &mut State, this: u32) -> usize {
    let identity = state.com.identity(this);
    device_index_by_id(&mut state.immediate.devices, identity)
}

fn device_of(state: &mut State, this: u32) -> &mut Device {
    let idx = device_index(state, this);
    &mut state.immediate.devices[idx].1
}

/// Set one of a device's transforms, as `SetTransform` would: what retained
/// mode does to the device underneath before a visual draws through it.
pub fn set_device_transform(state: &mut State, device: u32, which: u32, m: &super::scene::Matrix) {
    let mut bits = [0u32; 16];
    for r in 0..4 {
        for c in 0..4 {
            bits[r * 4 + c] = m[r][c].to_bits();
        }
    }
    device_of(state, device).set_transform(which, bits);
}

/// Make a viewport the one a device draws through, as `SetCurrentViewport`
/// would.
pub fn set_device_viewport(state: &mut State, device: u32, viewport: u32) {
    device_of(state, device).current_viewport = viewport;
}

fn add_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // A viewport belongs to the device that took it, and draws into that
    // device's surface.
    match ctx.state.com.get_mut(args[0]).map(|o| &mut o.state) {
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
        ctx.state.com.add_ref(current);
    }
    ctx.mem.write_u32(args[0], current).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_render_target(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The surface the device draws into, changed after it was made.
    let old = if let Some(ComState::Device3D { surface }) =
        ctx.state.com.get_mut(this).map(|o| &mut o.state)
    {
        std::mem::replace(surface, args[0])
    } else {
        0
    };
    if old != 0 && old != args[0] {
        let old_id = ctx.state.com.identity(old);
        com::reclaim_surface_if_unreferenced(ctx, old_id);
    }
    ctx.log
        .line(&format!("d3d render target {:#010x}", args[0]));
    Ok(S_OK)
}

fn get_render_target(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let surface = match ctx.state.com.get(this).map(|o| &o.state) {
        Some(ComState::Device3D { surface }) => *surface,
        _ => 0,
    };
    if surface != 0 {
        ctx.state.com.add_ref(surface);
    }
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

fn light_state_at_start(state: u32) -> Option<u32> {
    // D3DLIGHTSTATE_MATERIAL, AMBIENT and FOGMODE start at nothing and
    // COLORMODEL at RGB. What the fog's floating-point states start at has
    // not been measured.
    match state {
        3 => Some(2),
        1 | 2 | 4 => Some(0),
        _ => None,
    }
}

fn get_render_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let state = args[0];
    // DirectX 7 has no `TEXTUREMAPBLEND`: Wine's `d3d_device7_GetRenderState`
    // refuses it as invalid there. So a DirectX 7 device declares no start
    // for it.
    let dx7 = ctx.state.com.interface_of(this) == Some(DEVICE7.name);
    let Some(value) = device(ctx, this).get_render(state, dx7) else {
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
    device(ctx, this).set_render(args[0], args[1]);
    Ok(S_OK)
}

fn get_light_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let state = args[0];
    let Some(value) = device(ctx, this).get_light(state) else {
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
    device(ctx, this).set_light(args[0], args[1]);
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
    device(ctx, this).set_transform(args[0], m);
    Ok(S_OK)
}

fn get_transform(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Every transform starts as the identity.
    let m = device(ctx, this).get_transform(args[0]);
    for (i, cell) in m.iter().enumerate() {
        ctx.mem
            .write_u32(args[1] + 4 * i as u32, *cell)
            .map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn get_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let texture = device(ctx, this).get_texture(args[0]);
    if texture != 0 {
        ctx.state.com.add_ref(texture);
    }
    ctx.mem.write_u32(args[1], texture).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_texture(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // SetTexture(stage, texture): the device holds a reference to what it
    // draws with, and lets go of what it drew with before.
    let previous = device(ctx, this).set_texture(args[0], args[1]);
    if args[1] != 0 {
        ctx.state.com.add_ref(args[1]);
    }
    if previous != 0 {
        com::release_ref(ctx, previous);
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
        // D3DTSS_ADDRESS, ADDRESSU and ADDRESSV: wrap.
        (12..=14, _) => 1,
        // D3DTSS_MAGFILTER and MINFILTER: point. MIPFILTER: none.
        (16..=18, _) => 1,
        _ => return None,
    })
}

fn get_texture_stage_state(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let stage = args[0];
    // D3DTSS_ADDRESS (12) aliases ADDRESSU (13) on readback in DirectX 6.
    let state = if args[1] == tss::ADDRESS {
        tss::ADDRESSU
    } else {
        args[1]
    };
    let Some(value) = device(ctx, this).get_stage(stage, state) else {
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
    let dev = device(ctx, this);
    dev.set_stage(args[0], args[1], args[2]);
    // D3DTSS_ADDRESS (12) sets both U and V addressing on DirectX 6 (#370).
    if args[1] == tss::ADDRESS {
        dev.set_stage(args[0], tss::ADDRESSU, args[2]);
        dev.set_stage(args[0], tss::ADDRESSV, args[2]);
    }
    Ok(S_OK)
}

fn get_direct3d3(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The Direct3D object the device came from: one per device, the same
    // every time it is asked for.
    let existing = device(ctx, this).direct3d;
    let object = if existing == 0 {
        let made = com::create(ctx, &D3D3, ComState::None).map_err(Stop::Fault)?;
        device(ctx, this).direct3d = made;
        made
    } else {
        ctx.state.com.add_ref(existing);
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
    let object = com::create(ctx, &MATERIAL3, ComState::None).map_err(Stop::Fault)?;
    ctx.state.immediate.materials.insert(object, Vec::new());
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_light(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateLight(out, outer).
    if let Some(refused) = aggregated(args[1]) {
        return Ok(refused);
    }
    let object = com::create(ctx, &LIGHT, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn create_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // CreateViewport(out, outer).
    if let Some(refused) = aggregated(args[1]) {
        return Ok(refused);
    }
    let object = com::create(
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
    let bytes = ctx.mem.slice(args[0], size as usize).map_err(Stop::Fault)?;
    let identity = ctx.state.com.identity(this);
    let entry = ctx.state.immediate.materials.entry(identity).or_default();
    entry.clear();
    entry.extend_from_slice(bytes);
    Ok(S_OK)
}

fn get_material(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let identity = ctx.state.com.identity(this);
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
    let identity = ctx.state.com.identity(this);
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
    let surface = |ctx: &Ctx<'_>, at: u32| match ctx.state.com.get(at).map(|o| &o.state) {
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
    if ctx.state.com.identity(args[0]) == ctx.state.com.identity(this) {
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
    if let Some(ComState::Surface { caps, .. }) = ctx.state.com.get_mut(this).map(|o| &mut o.state)
    {
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
    let identity = ctx.state.com.identity(this);
    ctx.state.immediate.viewports.insert(identity, bytes);
    ctx.log.line(&format!("d3d viewport {w}x{h} at {x},{y}"));
    Ok(S_OK)
}

fn fetch_viewport(ctx: &mut Ctx<'_>, this: u32, at: u32) -> Result<u32, Stop> {
    let identity = ctx.state.com.identity(this);
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
    match ctx.state.com.get_mut(this).map(|o| &mut o.state) {
        Some(ComState::Viewport { background, .. }) => {
            *background = args[0];
            Ok(S_OK)
        }
        _ => Ok(DDERR_INVALIDPARAMS),
    }
}

fn get_background(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // GetBackground(handle, valid).
    let handle = match ctx.state.com.get(this).map(|o| &o.state) {
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
    let Some(&ComState::Viewport { device, .. }) = ctx.state.com.get(this).map(|o| &o.state) else {
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
    let surface = match ctx.state.com.get(device).map(|o| &o.state) {
        Some(ComState::Device3D { surface }) => *surface,
        // D3DERR_VIEWPORTHASNODEVICE: never added to a device.
        _ => return Ok(0x8876_0306),
    };
    let Some(&ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format,
        ..
    }) = ctx.state.com.get(surface).map(|o| &o.state)
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    if colour.is_some() && format.masks == [0; 4] {
        return Err(Stop::NotWritten {
            what: format!("{method}: an indexed surface"),
            caller: 0,
        });
    }
    let dev_idx = depth_value.map(|_| device_index(ctx.state, device));
    let mut depth =
        dev_idx.map(|idx| take_depth_at(&mut ctx.state.immediate.devices[idx].1, width, height));
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
    if let (Some(idx), Some(buffer)) = (dev_idx, depth) {
        ctx.state.immediate.devices[idx].1.depth = buffer;
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
    let handle = match ctx.state.com.get(this).map(|o| &o.state) {
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
    draw(
        ctx,
        "IDirect3DDevice3::DrawPrimitive",
        this,
        args[0],
        args[1],
        args[2],
        args[3],
        DrawIndices::Sequential(args[3]),
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

/// `D3DFVF_TLVERTEX`: `XYZRHW | DIFFUSE | SPECULAR | TEX1` (32 bytes).
const D3DFVF_TLVERTEX: u32 = 0x1c4;

/// DirectX 5's, which names its vertices by a `D3DVERTEXTYPE` where DirectX 6
/// has a flexible format. `D3DVT_TLVERTEX` is laid out as
/// `D3DFVF_TLVERTEX` is (d3dtypes.h): on the screen already, with both
/// colours and one pair of texture coordinates. LEGO Chess's DirectX plugin
/// draws all it draws in them (#210); the other two types are walls.
fn draw_indexed_primitive2(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    const D3DVT_TLVERTEX: u32 = 3;
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
    fill_u16_indices(ctx, args[4], args[5])?;
    draw(
        ctx,
        method,
        this,
        args[0],
        format,
        args[2],
        args[3],
        DrawIndices::Scratch,
        args[6],
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
    pub const ADDRESS: u32 = 12;
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

/// A transform as a device keeps it, as a matrix.
fn matrix_of(device: &Device, which: u32) -> super::scene::Matrix {
    let bits = device.get_transform(which);
    std::array::from_fn(|r| std::array::from_fn(|c| f32::from_bits(bits[r * 4 + c])))
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

/// Exact `f32::from(c) / 255.0` lookup table for 8-bit `D3DCOLOR` channels.
const U8_TO_F32: [f32; 256] = {
    let mut table = [0.0; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = i as f32 / 255.0;
        i += 1;
    }
    table
};

#[inline(always)]
fn colour_lut(bytes: [u8; 4]) -> [f32; 4] {
    [
        U8_TO_F32[bytes[2] as usize],
        U8_TO_F32[bytes[1] as usize],
        U8_TO_F32[bytes[0] as usize],
        U8_TO_F32[bytes[3] as usize],
    ]
}

#[inline(always)]
fn decode_tlvertex(raw: &[u8; 32], need_specular: bool, has_fog: bool) -> super::pipeline::Clipped {
    let f = |at: usize| f32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
    let diffuse = colour_lut([raw[16], raw[17], raw[18], raw[19]]);
    let (specular, fog) = if need_specular || has_fog {
        let [sr, sg, sb, sa] = colour_lut([raw[20], raw[21], raw[22], raw[23]]);
        ([sr, sg, sb], if has_fog { sa } else { 1.0 })
    } else {
        ([0.0; 3], 1.0)
    };
    super::pipeline::Clipped {
        clip: [f(0), f(4), f(8), f(12)],
        diffuse,
        specular,
        uv: [f(24), f(28)],
        fog,
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
    indices: DrawIndices,
    flags: u32,
) -> Result<u32, Stop> {
    use super::pipeline::{self, fvf, Clipped};
    use crate::raster::{Blend, Combine, Compare, States};

    let Some(size) = pipeline::vertex_size(format) else {
        return Err(unwritten(format!("{method}: vertex format {format:#x}")));
    };
    let Some(&ComState::Device3D { surface }) = ctx.state.com.get(this).map(|o| &o.state) else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    let Some(&ComState::Surface {
        pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        ..
    }) = ctx.state.com.get(surface).map(|o| &o.state)
    else {
        return Ok(DDERR_INVALIDPARAMS);
    };
    if pixel_format.masks == [0; 4] {
        return Err(unwritten(format!("{method}: into an indexed surface")));
    }

    // Check and borrow the guest vertex bytes before reading device state so
    // an invalid vertex pointer faults before any state wall.
    let bytes = ctx
        .mem
        .slice(vertices_at, count as usize * size)
        .map_err(Stop::Fault)?;

    // A DirectX 5 device has no texture stages: what it draws with is the
    // texture its handle names, and how a texel and the colour make a pixel
    // is one state, `D3DRENDERSTATE_TEXTUREMAPBLEND` (#210). On a later
    // device either is a wall, as it was.
    let dx5 = ctx.state.com.interface_of(this) == Some(DEVICE2.name);
    let dev_idx = device_index(ctx.state, this);
    let by_handle = ctx.state.immediate.devices[dev_idx]
        .1
        .render_value(rs::TEXTUREHANDLE);
    let named = match ctx.state.immediate.handles.get(&by_handle) {
        _ if !dx5 || by_handle == 0 => 0,
        Some(&t)
            if matches!(
                ctx.state.com.get(t).map(|o| &o.state),
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
    let device = &mut ctx.state.immediate.devices[dev_idx].1;
    for (what, on) in [
        ("a texture by handle", !dx5 && by_handle != 0),
        (
            "wrapping across texture edges",
            device.render_value(rs::WRAPU) != 0
                || device.render_value(rs::WRAPV) != 0
                || device.render_value(rs::WRAP0) != 0,
        ),
        (
            "a texture map blend",
            !dx5 && device.render_is_set(rs::TEXTUREMAPBLEND),
        ),
        (
            "a fill that is not solid",
            device.render_value(rs::FILLMODE) != 3,
        ),
    ] {
        if on {
            return Err(unwritten(format!("{method}: {what}")));
        }
    }
    let untransformed = format & fvf::POSITION_MASK == fvf::XYZ;
    let (world, clip_from_model) = if untransformed {
        let world = matrix_of(device, WORLD);
        let clip_from_world = device.view_proj();
        let clip_from_model = super::scene::multiply(&world, &clip_from_world);
        (world, clip_from_model)
    } else {
        (super::scene::IDENTITY, super::scene::IDENTITY)
    };
    let device = &*device;
    let state = |s: u32| device.render_value(s);
    let stage = |s: u32| device.stage0_value(s);
    let viewport = device.current_viewport;
    // DirectX 7 keeps the viewport on the device itself, so there is no object
    // to ask (#57).
    let viewport7 = device.viewport7;
    let texture = if dx5 { named } else { device.textures[0] };
    let material_handle = device.light[LIGHT_MATERIAL as usize];
    let ambient = colour_lut(device.light[LIGHT_AMBIENT as usize].to_le_bytes());
    let compare = |value: u32| {
        Compare::from_d3d(value).ok_or_else(|| unwritten(format!("{method}: comparison {value}")))
    };
    let blend = |value: u32| {
        Blend::from_d3d(value).ok_or_else(|| unwritten(format!("{method}: blend factor {value}")))
    };
    // Fog. With neither mode a curve of its own, Direct3D takes each
    // vertex's factor from the alpha its title wrote in the specular colour,
    // which is how NetImmerse fogs Alpha Team's levels (#61).
    let fog = if state(rs::FOGENABLE) != 0 {
        let (table, vertex) = (state(rs::FOGTABLEMODE), state(rs::FOGVERTEXMODE));
        if table != 0 || vertex != 0 || format & fvf::SPECULAR == 0 {
            return Err(unwritten(format!(
                "{method}: fog, table mode {table}, vertex mode {vertex}, vertex format {format:#x}"
            )));
        }
        let [r, g, b, _] = colour_lut(state(rs::FOGCOLOR).to_le_bytes());
        Some([r, g, b])
    } else {
        None
    };
    let depth_on = state(rs::ZENABLE) != 0;
    // The magnification filter: the legacy render state when a title set it,
    // the stage's own otherwise. One filter serves for minification too.
    // ponytail: no mip maps and one filter both ways; a title that minifies a
    // detailed texture far away shimmers where the card would have blurred.
    let linear = if device.render_is_set(rs::TEXTUREMAG) {
        match state(rs::TEXTUREMAG) {
            1 | 3 | 4 => false,
            2 | 5 | 6 => true,
            other => return Err(unwritten(format!("{method}: texture filter {other}"))),
        }
    } else {
        match stage(tss::MAGFILTER) {
            1 => false,
            2 => true,
            other => return Err(unwritten(format!("{method}: magnification filter {other}"))),
        }
    };
    let address = |legacy: u32, own: u32| -> Result<bool, Stop> {
        let value = if device.render_is_set(legacy) {
            state(legacy)
        } else if device.render_is_set(rs::TEXTUREADDRESS) {
            state(rs::TEXTUREADDRESS)
        } else {
            stage(own)
        };
        clamps(value).ok_or_else(|| unwritten(format!("{method}: texture address {value}")))
    };
    let clamp = [
        address(rs::TEXTUREADDRESSU, tss::ADDRESSU)?,
        address(rs::TEXTUREADDRESSV, tss::ADDRESSV)?,
    ];
    let mut states = States {
        depth_test: if depth_on {
            Some(compare(state(rs::ZFUNC))?)
        } else {
            None
        },
        depth_write: depth_on && state(rs::ZWRITEENABLE) != 0,
        flat: state(rs::SHADEMODE) == 1,
        colour: Combine::Modulate,
        alpha: Combine::Modulate,
        linear,
        colour_key: state(rs::COLORKEYENABLE) != 0,
        alpha_test: if state(rs::ALPHATESTENABLE) != 0 {
            Some((
                compare(state(rs::ALPHAFUNC))?,
                (state(rs::ALPHAREF) & 0xff) as u8,
            ))
        } else {
            None
        },
        blend: if state(rs::ALPHABLENDENABLE) != 0 {
            Some((blend(state(rs::SRCBLEND))?, blend(state(rs::DESTBLEND))?))
        } else {
            None
        },
        specular: state(rs::SPECULARENABLE) != 0,
        fog,
    };
    let cull = state(rs::CULLMODE);
    let (op, a1, a2) = (
        stage(tss::COLOROP),
        stage(tss::COLORARG1),
        stage(tss::COLORARG2),
    );
    let (aop, aa1, aa2) = (
        stage(tss::ALPHAOP),
        stage(tss::ALPHAARG1),
        stage(tss::ALPHAARG2),
    );
    let legacy = dx5.then(|| state(rs::TEXTUREMAPBLEND));
    let texture_alpha = match ctx.state.com.get(texture).map(|o| &o.state) {
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
        match ctx.state.com.get(viewport).map(|o| &o.state) {
            Some(&ComState::Viewport {
                x,
                y,
                width: w,
                height: h,
                ..
            }) if !ctx
                .state
                .immediate
                .viewports
                .contains_key(&ctx.state.com.identity(viewport)) =>
            {
                pipeline::Viewport::whole(x, y, w, h)
            }
            // A viewport the title described itself (`SetViewport` /
            // `SetViewport2`) used with already-transformed screen-space
            // (`XYZRHW`) vertices on DirectX 5 (`DEVICE2`, #210) or DirectX 6
            // (`DEVICE3`, #370): only the screen rectangle (`dwX`, `dwY`,
            // `dwWidth`, `dwHeight`) is needed. Projecting untransformed `XYZ`
            // vertices through a title-described viewport still walls below.
            Some(ComState::Viewport { .. }) if format & fvf::POSITION_MASK == fvf::XYZRHW => {
                let bytes = &ctx.state.immediate.viewports[&ctx.state.com.identity(viewport)];
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
    if !matches!(kind, 4..=6) {
        return Err(unwritten(format!("{method}: primitive type {kind}")));
    }

    let mut lights = std::mem::take(&mut ctx.state.immediate.scratch.lights);
    lights.clear();
    if lighting {
        super::d3drm::fill_current_lights(ctx.state, &mut lights);
    }

    // Every vertex into clip space, coloured; a vertex already on the screen
    // stays where it is.
    let mut prepared = std::mem::take(&mut ctx.state.immediate.scratch.prepared);
    prepared.clear();
    let need_specular = states.specular;
    let has_fog = fog.is_some();
    if format == D3DFVF_TLVERTEX {
        prepared.extend(
            bytes
                .as_chunks::<32>()
                .0
                .iter()
                .map(|raw| decode_tlvertex(raw, need_specular, has_fog)),
        );
    } else {
        prepared.extend(bytes.chunks_exact(size).map(|raw| {
            let v = pipeline::read_vertex(format, raw);
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
                _ => v.diffuse.map_or([1.0; 4], |c| colour_lut(c.to_le_bytes())),
            };
            let (specular, vertex_fog) = if need_specular || has_fog {
                let [sr, sg, sb, sa] = v.specular.map_or([0.0; 4], |c| colour_lut(c.to_le_bytes()));
                ([sr, sg, sb], if has_fog { sa } else { 1.0 })
            } else {
                ([0.0; 3], 1.0)
            };
            Clipped {
                fog: vertex_fog,
                clip: if untransformed {
                    pipeline::project(&clip_from_model, v.position)
                } else {
                    v.position
                },
                diffuse,
                specular,
                uv: v.uv,
            }
        }));
    }

    // The triangles, by the primitive's kind, wound as the title wound them.
    let mut triangles = std::mem::take(&mut ctx.state.immediate.scratch.triangles);
    triangles.clear();
    match indices {
        DrawIndices::Sequential(n) => {
            push_triangles(
                kind,
                n,
                |i| prepared.get(i as usize).copied(),
                &mut triangles,
            );
        }
        DrawIndices::Scratch => {
            let scratch_indices = &ctx.state.immediate.scratch.indices;
            push_triangles(
                kind,
                scratch_indices.len() as u32,
                |i| {
                    scratch_indices
                        .get(i as usize)
                        .and_then(|&n| prepared.get(n as usize))
                        .copied()
                },
                &mut triangles,
            );
        }
    }
    let canvas = Canvas {
        pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        place,
    };
    let result = rasterise(
        ctx,
        method,
        this,
        &canvas,
        &triangles,
        untransformed,
        cull,
        (texture, clamp),
        states,
    );
    ctx.state.immediate.scratch.lights = lights;
    ctx.state.immediate.scratch.prepared = prepared;
    ctx.state.immediate.scratch.triangles = triangles;
    result?;
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
    use crate::raster::{self, Combine};
    if triangles.is_empty() {
        return Ok(());
    }
    let Canvas {
        pixels,
        width,
        height,
        pitch,
        format: pixel_format,
        place,
    } = *canvas;

    // Clip and cull into the reusable triangle buffer before touching texture
    // palettes, depth buffer, or guest arena borrows so fully culled/clipped
    // batches return immediately.
    let scratch = &mut ctx.state.immediate.scratch;
    scratch.raster.clear();
    for &[a, mut b, mut c] in triangles {
        if states.flat {
            // A flat triangle is the first vertex's colour throughout, which
            // clipping must not blend away.
            for v in [&mut b, &mut c] {
                v.diffuse = a.diffuse;
                v.specular = a.specular;
            }
        }
        if !untransformed {
            let poly = [a.on_screen(), b.on_screen(), c.on_screen()];
            emit_polygon(&poly, cull, &mut scratch.raster);
        } else {
            let (oa, ob, oc) = (
                frustum_outcode(&a),
                frustum_outcode(&b),
                frustum_outcode(&c),
            );
            let or = oa | ob | oc;
            if or == 0 {
                let poly = [place.place(&a), place.place(&b), place.place(&c)];
                emit_polygon(&poly, cull, &mut scratch.raster);
            } else {
                let and = oa & ob & oc;
                if and != 0
                    && (or & (and.isolate_lowest_one() - 1)) == 0
                    && finite_positive_w(&a)
                    && finite_positive_w(&b)
                    && finite_positive_w(&c)
                {
                    continue;
                }
                emit_clipped([a, b, c], &place, cull, scratch);
            }
        }
    }
    if scratch.raster.is_empty() {
        return Ok(());
    }

    // The texture the first stage samples, in its own format.
    let texture_info = match ctx.state.com.get(texture).map(|o| &o.state) {
        Some(&ComState::Surface {
            pixels: at,
            width: tw,
            height: th,
            pitch: tp,
            format: tf,
            palette,
            ..
        }) => {
            let palette = match ctx.state.com.get(palette).map(|o| &o.state) {
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
    let dev_idx = device_index(ctx.state, this);
    let mut depth = take_depth_at(&mut ctx.state.immediate.devices[dev_idx].1, width, height);
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
    raster::triangles(
        &mut target,
        &ctx.state.immediate.scratch.raster,
        texture.as_ref(),
        &states,
    );
    ctx.state.immediate.devices[dev_idx].1.depth = depth;
    Ok(())
}

/// Exact 6-bit frustum outcode matching `pipeline::inside_frustum` and the six
/// clipping planes in order: `-w <= x`, `x <= w`, `-w <= y`, `y <= w`,
/// `0 <= z`, `z <= w`. Bit `p` is set whenever the inequality is false
/// (including `NaN`), so `oa | ob | oc == 0` is bit-identical to
/// `inside_frustum(&a) && inside_frustum(&b) && inside_frustum(&c)`.
#[inline(always)]
fn frustum_outcode(v: &super::pipeline::Clipped) -> u8 {
    let [x, y, z, w] = v.clip;
    let inside = u8::from(x >= -w)
        | (u8::from(x <= w) << 1)
        | (u8::from(y >= -w) << 2)
        | (u8::from(y <= w) << 3)
        | (u8::from(z >= 0.0) << 4)
        | (u8::from(z <= w) << 5);
    inside ^ 0x3f
}

/// Finite clip coordinates with strictly positive `w`, under which each
/// outcode bit matches `plane_dist::<P>(clip) < 0.0` exactly without `NaN`,
/// `±inf`, or negative-`w` anomalies.
#[inline(always)]
fn finite_positive_w(v: &super::pipeline::Clipped) -> bool {
    let [x, y, z, w] = v.clip;
    w > 0.0 && w.is_finite() && x.is_finite() && y.is_finite() && z.is_finite()
}

#[inline(always)]
fn plane_dist<const PLANE: u8>(c: &[f32; 4]) -> f32 {
    match PLANE {
        0 => c[3] + c[0],
        1 => c[3] - c[0],
        2 => c[3] + c[1],
        3 => c[3] - c[1],
        4 => c[2],
        5 => c[3] - c[2],
        _ => unreachable!(),
    }
}

#[inline(always)]
fn lerp_clipped(
    a: &super::pipeline::Clipped,
    b: &super::pipeline::Clipped,
    t: f32,
) -> super::pipeline::Clipped {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    super::pipeline::Clipped {
        clip: std::array::from_fn(|i| mix(a.clip[i], b.clip[i])),
        diffuse: std::array::from_fn(|i| mix(a.diffuse[i], b.diffuse[i])),
        specular: std::array::from_fn(|i| mix(a.specular[i], b.specular[i])),
        uv: std::array::from_fn(|i| mix(a.uv[i], b.uv[i])),
        fog: mix(a.fog, b.fog),
    }
}

#[inline(always)]
fn clip_plane<const PLANE: u8>(
    input: &[super::pipeline::Clipped],
    output: &mut [super::pipeline::Clipped; super::pipeline::CLIP_BUFFER],
) -> Option<usize> {
    if input.is_empty() {
        return Some(0);
    }
    let mut out_len = 0;
    let mut previous = &input[input.len() - 1];
    let mut dp = plane_dist::<PLANE>(&previous.clip);
    let mut dp_in = dp >= 0.0;
    for current in input {
        let dc = plane_dist::<PLANE>(&current.clip);
        let dc_in = dc >= 0.0;
        if dc_in != dp_in {
            if out_len == super::pipeline::CLIP_BUFFER {
                return None;
            }
            output[out_len] = lerp_clipped(previous, current, dp / (dp - dc));
            out_len += 1;
        }
        if dc_in {
            if out_len == super::pipeline::CLIP_BUFFER {
                return None;
            }
            output[out_len] = *current;
            out_len += 1;
        }
        previous = current;
        dp = dc;
        dp_in = dc_in;
    }
    Some(out_len)
}

/// Monomorphized six-plane Sutherland-Hodgman clip of a triangle into
/// caller-owned buffers (`out` receives the final polygon after the even
/// 6-plane ping-pong: `tri -> scratch -> out -> scratch -> out -> scratch -> out`).
#[inline]
fn clip_triangle_into(
    tri: &[super::pipeline::Clipped; 3],
    out: &mut [super::pipeline::Clipped; super::pipeline::CLIP_BUFFER],
    scratch: &mut [super::pipeline::Clipped; super::pipeline::CLIP_BUFFER],
) -> Option<usize> {
    let len = clip_plane::<0>(tri, scratch)?;
    let len = clip_plane::<1>(&scratch[..len], out)?;
    let len = clip_plane::<2>(&out[..len], scratch)?;
    let len = clip_plane::<3>(&scratch[..len], out)?;
    let len = clip_plane::<4>(&out[..len], scratch)?;
    clip_plane::<5>(&scratch[..len], out)
}

#[inline]
fn push_triangles(
    kind: u32,
    n: u32,
    corner: impl Fn(u32) -> Option<super::pipeline::Clipped>,
    triangles: &mut Vec<[super::pipeline::Clipped; 3]>,
) {
    let mut push = |i: u32, j: u32, k: u32| {
        if let (Some(a), Some(b), Some(c)) = (corner(i), corner(j), corner(k)) {
            triangles.push([a, b, c]);
        }
    };
    match kind {
        4 => {
            for t in 0..n / 3 {
                push(t * 3, t * 3 + 1, t * 3 + 2);
            }
        }
        5 => {
            for t in 0..n.saturating_sub(2) {
                if t % 2 == 0 {
                    push(t, t + 1, t + 2);
                } else {
                    push(t + 1, t, t + 2);
                }
            }
        }
        6 => {
            for t in 1..n.saturating_sub(1) {
                push(0, t, t + 1);
            }
        }
        _ => unreachable!(),
    }
}

#[inline]
fn emit_polygon(
    polygon: &[crate::raster::Vertex],
    cull: u32,
    list: &mut Vec<[crate::raster::Vertex; 3]>,
) {
    let len = polygon.len();
    if len < 3 {
        return;
    }
    // Twice the polygon's area on the screen: positive when it is wound
    // clockwise, with y running down. Left-fold from 0.0 matches Iterator::sum.
    let mut area = 0.0f32;
    for p in 0..len {
        let u = polygon[p];
        let v = polygon[if p + 1 < len { p + 1 } else { 0 }];
        area += u.x * v.y - v.x * u.y;
    }
    // D3DCULL_CW and D3DCULL_CCW.
    if (cull == 2 && area > 0.0) || (cull == 3 && area < 0.0) {
        return;
    }
    for p in 1..len - 1 {
        list.push([polygon[0], polygon[p], polygon[p + 1]]);
    }
}

#[inline]
fn emit_clipped(
    tri: [super::pipeline::Clipped; 3],
    place: &super::pipeline::Viewport,
    cull: u32,
    scratch: &mut DrawScratch,
) {
    if let Some(len) = clip_triangle_into(&tri, &mut scratch.clip_buf, &mut scratch.clip_scratch) {
        if len < 3 {
            return;
        }
        for (dst, src) in scratch.poly_buf.iter_mut().zip(&scratch.clip_buf[..len]) {
            *dst = place.place(src);
        }
        emit_polygon(&scratch.poly_buf[..len], cull, &mut scratch.raster);
    } else {
        emit_clipped_overflow(&tri, place, cull, &mut scratch.raster);
    }
}

#[cold]
#[inline(never)]
fn emit_clipped_overflow(
    tri: &[super::pipeline::Clipped; 3],
    place: &super::pipeline::Viewport,
    cull: u32,
    list: &mut Vec<[crate::raster::Vertex; 3]>,
) {
    let poly: Vec<crate::raster::Vertex> = super::pipeline::clip(tri)
        .iter()
        .map(|v| place.place(v))
        .collect();
    emit_polygon(&poly, cull, list);
}

/// Part of a device's depth buffer to as far as it goes: left, top, right and
/// bottom, on a surface so wide and high.
pub fn clear_depth(state: &mut State, device: u32, edges: [u32; 4], (width, height): (u32, u32)) {
    let [left, top, right, bottom] = edges;
    let (right, bottom) = (right.min(width), bottom.min(height));
    let dev_idx = device_index(state, device);
    let mut depth = take_depth_at(&mut state.immediate.devices[dev_idx].1, width, height);
    for y in top..bottom {
        let row = (y * width) as usize;
        depth[row + left.min(right) as usize..row + right as usize].fill(1.0);
    }
    state.immediate.devices[dev_idx].1.depth = depth;
}

/// A device's depth buffer, taken out of the device for a draw to use and
/// hand back: the size of its surface, made far the first time it is needed.
fn take_depth_at(device: &mut Device, width: u32, height: u32) -> Vec<f32> {
    let buffer = std::mem::take(&mut device.depth);
    let len = (width * height) as usize;
    if buffer.len() == len {
        buffer
    } else {
        vec![1.0; len]
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clip_triangle_into, decode_tlvertex, device_index_by_id, finite_positive_w,
        frustum_outcode, matrix_of, render_state_at_start, rs, stage_state_at_start, tss, Device,
        D3DFVF_TLVERTEX, IDENTITY_BITS, LIGHT_STATE_SLOTS, PROJECTION, RENDER_STATE_SLOTS,
        STAGE_SLOTS, STAGE_STATE_SLOTS, TRANSFORM_SLOTS, U8_TO_F32, VIEW, WORLD,
    };
    use crate::win32::pipeline::{self, Clipped, CLIP_BUFFER};
    use crate::win32::scene;

    #[test]
    fn every_draw_render_and_stage_state_fits_flat_tables_with_declared_defaults() {
        let render_states = [
            rs::TEXTUREHANDLE,
            rs::TEXTUREADDRESS,
            rs::WRAPU,
            rs::WRAPV,
            rs::ZENABLE,
            rs::FILLMODE,
            rs::SHADEMODE,
            rs::ZWRITEENABLE,
            rs::ALPHATESTENABLE,
            rs::TEXTUREMAG,
            rs::SRCBLEND,
            rs::DESTBLEND,
            rs::TEXTUREMAPBLEND,
            rs::CULLMODE,
            rs::ZFUNC,
            rs::ALPHAREF,
            rs::ALPHAFUNC,
            rs::ALPHABLENDENABLE,
            rs::FOGENABLE,
            rs::SPECULARENABLE,
            rs::FOGCOLOR,
            rs::FOGTABLEMODE,
            rs::COLORKEYENABLE,
            rs::TEXTUREADDRESSU,
            rs::TEXTUREADDRESSV,
            rs::WRAP0,
            rs::FOGVERTEXMODE,
        ];
        let device = Device::default();
        for s in render_states {
            assert!((s as usize) < RENDER_STATE_SLOTS, "rs {s} out of bounds");
            let want = render_state_at_start(s).expect("declared default");
            assert_eq!(device.render_value(s), want, "rs {s} default");
            assert!(!device.render_is_set(s), "rs {s} starts unset");
        }
        let stage_states = [
            tss::COLOROP,
            tss::COLORARG1,
            tss::COLORARG2,
            tss::ALPHAOP,
            tss::ALPHAARG1,
            tss::ALPHAARG2,
            tss::ADDRESSU,
            tss::ADDRESSV,
            tss::MAGFILTER,
        ];
        for s in stage_states {
            assert!((s as usize) < STAGE_STATE_SLOTS, "tss {s} out of bounds");
            let want = stage_state_at_start(s, 0).expect("declared default");
            assert_eq!(device.stage0_value(s), want, "tss {s} default");
        }
    }

    #[test]
    fn flat_device_tables_preserve_set_flags_dx7_rules_and_overflow_spill() {
        let mut d = Device::default();
        // DX7 refuses unset TEXTUREMAPBLEND, DX5/6 returns default 2; once set both read it.
        assert_eq!(d.get_render(rs::TEXTUREMAPBLEND, false), Some(2));
        assert_eq!(d.get_render(rs::TEXTUREMAPBLEND, true), None);
        assert!(!d.render_is_set(rs::TEXTUREMAPBLEND));
        d.set_render(rs::TEXTUREMAPBLEND, 2);
        assert!(d.render_is_set(rs::TEXTUREMAPBLEND));
        assert_eq!(d.get_render(rs::TEXTUREMAPBLEND, true), Some(2));

        // Undeclared in-range and out-of-range render states stay None until set.
        assert_eq!(d.get_render(100, false), None);
        d.set_render(100, 77);
        assert_eq!(d.get_render(100, false), Some(77));
        let hi_rs = RENDER_STATE_SLOTS as u32 + 5;
        assert_eq!(d.get_render(hi_rs, false), None);
        assert!(!d.render_is_set(hi_rs));
        d.set_render(hi_rs, 99);
        assert!(d.render_is_set(hi_rs));
        assert_eq!(d.get_render(hi_rs, false), Some(99));

        // Texture stages: in-range defaults, set override, and out-of-range spill.
        assert_eq!(d.get_stage(0, tss::COLOROP), Some(4));
        assert_eq!(d.get_stage(1, tss::COLOROP), Some(1));
        assert_eq!(d.get_stage(STAGE_SLOTS as u32 + 2, tss::COLOROP), Some(1));
        assert_eq!(d.get_stage(0, 20), None);
        d.set_stage(0, 20, 5);
        assert_eq!(d.get_stage(0, 20), Some(5));
        d.set_stage(STAGE_SLOTS as u32 + 2, STAGE_STATE_SLOTS as u32 + 3, 11);
        assert_eq!(
            d.get_stage(STAGE_SLOTS as u32 + 2, STAGE_STATE_SLOTS as u32 + 3),
            Some(11)
        );

        // Light states, transforms, and textures: defaults and overflow spill.
        assert_eq!(d.get_light(3), Some(2));
        assert_eq!(d.get_light(5), None);
        d.set_light(5, 42);
        assert_eq!(d.get_light(5), Some(42));
        let hi_light = LIGHT_STATE_SLOTS as u32 + 1;
        assert_eq!(d.get_light(hi_light), None);
        d.set_light(hi_light, 88);
        assert_eq!(d.get_light(hi_light), Some(88));

        let hi_xf = TRANSFORM_SLOTS as u32 + 12;
        assert_eq!(d.get_transform(1), IDENTITY_BITS);
        assert_eq!(d.get_transform(hi_xf), IDENTITY_BITS);
        let custom = [7u32; 16];
        d.set_transform(hi_xf, custom);
        assert_eq!(d.get_transform(hi_xf), custom);

        let hi_tex = STAGE_SLOTS as u32 + 3;
        assert_eq!(d.get_texture(0), 0);
        assert_eq!(d.set_texture(0, 0x1000), 0);
        assert_eq!(d.set_texture(0, 0x2000), 0x1000);
        assert_eq!(d.set_texture(hi_tex, 0x3000), 0);
        assert_eq!(d.get_texture(hi_tex), 0x3000);
    }

    #[test]
    fn u8_to_f32_lut_and_tlvertex_decode_match_pipeline_bit_for_bit() {
        for c in 0u8..=255 {
            assert_eq!(
                U8_TO_F32[c as usize].to_bits(),
                (f32::from(c) / 255.0).to_bits(),
                "channel {c}"
            );
        }
        let samples: [([f32; 4], u32, u32, [f32; 2]); 3] = [
            (
                [12.5, -0.0, 0.25, 0.5],
                0x80ff_4020,
                0xc012_3456,
                [0.125, 0.875],
            ),
            (
                [640.0, 480.0, 1.0, 1.0],
                0xffff_ffff,
                0x0000_0000,
                [-2.5, 3.25],
            ),
            ([0.0, 0.0, 0.0, 2.0], 0x0000_0000, 0xffab_cdef, [0.0, 1.0]),
        ];
        for (pos, diff, spec, uv) in samples {
            let mut raw = [0u8; 32];
            for (i, f) in pos.iter().enumerate() {
                raw[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
            }
            raw[16..20].copy_from_slice(&diff.to_le_bytes());
            raw[20..24].copy_from_slice(&spec.to_le_bytes());
            raw[24..28].copy_from_slice(&uv[0].to_le_bytes());
            raw[28..32].copy_from_slice(&uv[1].to_le_bytes());

            let v = pipeline::read_vertex(D3DFVF_TLVERTEX, &raw);
            let want_diffuse = pipeline::colour(v.diffuse.unwrap());
            let [sr, sg, sb, sa] = pipeline::colour(v.specular.unwrap());

            for (need_specular, has_fog) in
                [(true, true), (true, false), (false, true), (false, false)]
            {
                let got = decode_tlvertex(&raw, need_specular, has_fog);
                assert_eq!(got.clip.map(f32::to_bits), v.position.map(f32::to_bits));
                assert_eq!(
                    got.diffuse.map(f32::to_bits),
                    want_diffuse.map(f32::to_bits)
                );
                assert_eq!(got.uv.map(f32::to_bits), v.uv.map(f32::to_bits));
                let want_fog = if has_fog { sa } else { 1.0 };
                assert_eq!(got.fog.to_bits(), want_fog.to_bits());
                if need_specular || has_fog {
                    assert_eq!(
                        got.specular.map(f32::to_bits),
                        [sr, sg, sb].map(f32::to_bits)
                    );
                } else {
                    assert_eq!(got.specular, [0.0; 3]);
                }
            }
        }
    }

    #[test]
    fn frustum_outcode_and_monomorphized_clip_match_pipeline_bit_for_bit() {
        let v = |clip: [f32; 4], u: f32, t: f32| Clipped {
            clip,
            diffuse: [u, t, 0.5, 1.0],
            specular: [0.1, 0.2, 0.3],
            uv: [u, t],
            fog: 0.75,
        };
        let assert_f32_eq = |a: f32, b: f32, label: &str| {
            if a.is_nan() && b.is_nan() {
                return;
            }
            assert_eq!(a.to_bits(), b.to_bits(), "{label}: {a} vs {b}");
        };
        let assert_same_poly = |got: &[Clipped], want: &[Clipped], label: &str| {
            assert_eq!(got.len(), want.len(), "{label}: vertex count");
            for (g, w) in got.iter().zip(want) {
                for i in 0..4 {
                    assert_f32_eq(g.clip[i], w.clip[i], label);
                    assert_f32_eq(g.diffuse[i], w.diffuse[i], label);
                }
                for i in 0..3 {
                    assert_f32_eq(g.specular[i], w.specular[i], label);
                }
                for i in 0..2 {
                    assert_f32_eq(g.uv[i], w.uv[i], label);
                }
                assert_f32_eq(g.fog, w.fog, label);
            }
        };
        // Reuse dirty clip buffers across calls to verify stale tail entries are never read.
        let mut out = [Clipped::default(); CLIP_BUFFER];
        let mut scratch = [Clipped::default(); CLIP_BUFFER];
        let mut run_fast = |tri: [Clipped; 3]| -> (Vec<Clipped>, &'static str) {
            let (oa, ob, oc) = (
                frustum_outcode(&tri[0]),
                frustum_outcode(&tri[1]),
                frustum_outcode(&tri[2]),
            );
            let or = oa | ob | oc;
            if or == 0 {
                return (tri.to_vec(), "accept");
            }
            let and = oa & ob & oc;
            if and != 0
                && (or & (and.isolate_lowest_one() - 1)) == 0
                && finite_positive_w(&tri[0])
                && finite_positive_w(&tri[1])
                && finite_positive_w(&tri[2])
            {
                return (Vec::new(), "reject");
            }
            let poly = match clip_triangle_into(&tri, &mut out, &mut scratch) {
                Some(len) => out[..len].to_vec(),
                None => pipeline::clip(&tri),
            };
            (poly, "clip")
        };

        // 1. Wholly inside -> trivial accept.
        let inside = [
            v([0.0, 0.0, 0.5, 1.0], 0.0, 0.0),
            v([0.5, 0.0, 0.5, 1.0], 1.0, 0.0),
            v([0.0, 0.5, 0.5, 1.0], 0.0, 1.0),
        ];
        let (got, path) = run_fast(inside);
        assert_eq!(path, "accept");
        assert_same_poly(&got, &pipeline::clip(&inside), "inside");

        // 2. Wholly outside each of the 6 planes without straddling prior planes -> trivial reject.
        let outside_cases: [(&str, [Clipped; 3]); 6] = [
            (
                "outside-left",
                [
                    v([-2.0, 0.0, 0.5, 1.0], 0.0, 0.0),
                    v([-3.0, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([-2.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "outside-right",
                [
                    v([2.0, 0.0, 0.5, 1.0], 0.0, 0.0),
                    v([3.0, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([2.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "outside-bottom",
                [
                    v([0.0, -2.0, 0.5, 1.0], 0.0, 0.0),
                    v([0.5, -3.0, 0.5, 1.0], 1.0, 0.0),
                    v([-0.5, -2.0, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "outside-top",
                [
                    v([0.0, 2.0, 0.5, 1.0], 0.0, 0.0),
                    v([0.5, 3.0, 0.5, 1.0], 1.0, 0.0),
                    v([-0.5, 2.0, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "outside-near",
                [
                    v([0.0, 0.0, -0.5, 1.0], 0.0, 0.0),
                    v([0.5, 0.0, -0.2, 1.0], 1.0, 0.0),
                    v([0.0, 0.5, -0.1, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "outside-far",
                [
                    v([0.0, 0.0, 1.5, 1.0], 0.0, 0.0),
                    v([0.5, 0.0, 2.0, 1.0], 1.0, 0.0),
                    v([0.0, 0.5, 1.2, 1.0], 0.0, 1.0),
                ],
            ),
        ];
        for (label, tri) in outside_cases {
            let (got, path) = run_fast(tri);
            assert_eq!(path, "reject", "{label}");
            assert_same_poly(&got, &pipeline::clip(&tri), label);
        }

        // 3. Prior-straddling-plane edge case: all z < 0, but straddling plane 0
        //    must bail out of trivial-reject to the full clipper so FP rounding matches.
        let prior_straddle = [
            v([-2.0, 0.0, -1.0, 1.0], 0.0, 0.0),
            v([0.0, 0.0, -1e-30, 1.0], 1.0, 0.0),
            v([0.0, 0.5, -1e-30, 1.0], 0.0, 1.0),
        ];
        let (got, path) = run_fast(prior_straddle);
        assert_eq!(path, "clip", "prior straddle must bail to full clipper");
        assert_same_poly(&got, &pipeline::clip(&prior_straddle), "prior-straddle");

        // 4. w <= 0 vertices (including all-outside w == 0) must bail to full clipper and match bit-for-bit.
        let w_edge_cases: [(&str, [Clipped; 3]); 4] = [
            (
                "w-zero",
                [
                    v([0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
                    v([1.0, 0.0, 0.5, 0.0], 1.0, 0.0),
                    v([0.0, 1.0, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "w-zero-all-outside-left",
                [
                    v([-1.0, 0.0, 0.5, 0.0], 0.0, 0.0),
                    v([-2.0, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([-3.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "w-negative-all",
                [
                    v([0.0, 0.0, -0.5, -1.0], 0.0, 0.0),
                    v([0.5, 0.0, -0.5, -1.0], 1.0, 0.0),
                    v([0.0, 0.5, -0.5, -1.0], 0.0, 1.0),
                ],
            ),
            (
                "w-mixed-sign",
                [
                    v([-0.5, 0.0, -0.5, -1.0], 0.0, 0.0),
                    v([0.5, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([0.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
        ];
        for (label, tri) in w_edge_cases {
            let (got, path) = run_fast(tri);
            assert_eq!(path, "clip", "{label} must bail to full clipper");
            assert_same_poly(&got, &pipeline::clip(&tri), label);
        }

        // 5. NaN and +/-inf vertices must never trivially reject and must match pipeline::clip.
        let nan_inf_cases: [(&str, [Clipped; 3]); 4] = [
            (
                "nan-x",
                [
                    v([f32::NAN, 0.0, 0.5, 1.0], 0.0, 0.0),
                    v([-2.0, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([-3.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "nan-w",
                [
                    v([0.0, 0.0, 0.5, f32::NAN], 0.0, 0.0),
                    v([0.5, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([0.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "inf-coords",
                [
                    v([f32::INFINITY, 0.0, 0.5, f32::INFINITY], 0.0, 0.0),
                    v([0.5, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([0.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
            (
                "neg-inf-all-outside-left",
                [
                    v([f32::NEG_INFINITY, 0.0, 0.5, 1.0], 0.0, 0.0),
                    v([-2.0, 0.0, 0.5, 1.0], 1.0, 0.0),
                    v([-3.0, 0.5, 0.5, 1.0], 0.0, 1.0),
                ],
            ),
        ];
        for (label, tri) in nan_inf_cases {
            let (got, path) = run_fast(tri);
            assert_ne!(path, "reject", "{label} must not trivially reject");
            assert_same_poly(&got, &pipeline::clip(&tri), label);
        }

        // 6. Large oblique triangle crossing all 6 frustum planes.
        let six_planes = [
            v([-3.0, -2.5, -1.5, 1.0], 0.0, 0.0),
            v([3.5, -1.5, 2.5, 1.0], 1.0, 0.0),
            v([0.2, 3.5, 0.5, 1.0], 0.5, 1.0),
        ];
        let (got, path) = run_fast(six_planes);
        assert_eq!(path, "clip");
        assert!(got.len() >= 3 && got.len() <= 9);
        assert_same_poly(&got, &pipeline::clip(&six_planes), "six-planes");
    }

    #[test]
    fn view_proj_cache_hits_on_world_and_invalidates_on_view_or_projection() {
        let bits = |m: scene::Matrix| std::array::from_fn(|i| m[i / 4][i % 4].to_bits());
        let view1: scene::Matrix = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [2.0, -3.0, 5.0, 1.0],
        ];
        let proj1: scene::Matrix = [
            [1.5, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.0, 0.0, 1.01, 1.0],
            [0.0, 0.0, -0.5, 0.0],
        ];
        let world1: scene::Matrix = [
            [0.5, 0.0, 0.0, 0.0],
            [0.0, 0.5, 0.0, 0.0],
            [0.0, 0.0, 0.5, 0.0],
            [1.0, 2.0, 3.0, 1.0],
        ];
        let mut d = Device::default();
        assert!(d.view_proj.is_none());
        d.set_transform(VIEW, bits(view1));
        d.set_transform(PROJECTION, bits(proj1));
        let vp1 = d.view_proj();
        assert_eq!(
            vp1,
            scene::multiply(&matrix_of(&d, VIEW), &matrix_of(&d, PROJECTION))
        );
        assert!(d.view_proj.is_some());

        // Changing WORLD keeps cached view_proj intact.
        d.set_transform(WORLD, bits(world1));
        assert_eq!(d.view_proj, Some(vp1));
        assert_eq!(d.view_proj(), vp1);

        // Changing VIEW invalidates and recomputes.
        let mut view2 = view1;
        view2[3][0] = -7.0;
        d.set_transform(VIEW, bits(view2));
        assert!(d.view_proj.is_none());
        let vp2 = d.view_proj();
        assert_eq!(vp2, scene::multiply(&view2, &proj1));
        assert_ne!(vp2, vp1);

        // Changing PROJECTION invalidates and recomputes.
        let mut proj2 = proj1;
        proj2[0][0] = 3.0;
        d.set_transform(PROJECTION, bits(proj2));
        assert!(d.view_proj.is_none());
        let vp3 = d.view_proj();
        assert_eq!(vp3, scene::multiply(&view2, &proj2));
        assert_ne!(vp3, vp2);
    }

    #[test]
    fn immediate_devices_vec_reuses_slots_by_identity() {
        let mut devices = Vec::new();
        let a0 = device_index_by_id(&mut devices, 0x1000);
        let b0 = device_index_by_id(&mut devices, 0x2000);
        assert_eq!((a0, b0), (0, 1));
        assert_eq!(devices.len(), 2);
        devices[a0].1.set_render(rs::CULLMODE, 1);
        assert_eq!(device_index_by_id(&mut devices, 0x1000), a0);
        assert_eq!(device_index_by_id(&mut devices, 0x2000), b0);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[a0].1.render_value(rs::CULLMODE), 1);
        assert_eq!(devices[b0].1.render_value(rs::CULLMODE), 3);
    }

    #[test]
    fn d3d3_create_device_get_caps_enum_formats_and_tss_address() {
        use super::{
            create_device3, enum_texture_formats7, get_caps3, set_texture_stage_state, tss,
            DEVICE3, DEVICE_DESC6_BYTES, HAL_DEVICE,
        };
        use crate::com::S_OK;
        use crate::cpu::Stop;
        use crate::guest::GuestMem;
        use crate::win32::tests::with_test_ctx;

        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let guid_at = 0x1000u32;
            let out_at = 0x1040u32;
            ctx.mem.write(guid_at, &HAL_DEVICE).unwrap();

            // Aggregation and null arguments are refused.
            assert_eq!(
                create_device3(ctx, 0, &[guid_at, 0x2000, out_at, 1]).unwrap(),
                0x8004_0110
            );
            assert_eq!(
                create_device3(ctx, 0, &[0, 0x2000, out_at, 0]).unwrap(),
                0x8007_0057
            );
            assert_eq!(
                create_device3(ctx, 0, &[guid_at, 0, out_at, 0]).unwrap(),
                0x8007_0057
            );
            assert_eq!(
                create_device3(ctx, 0, &[guid_at, 0x2000, 0, 0]).unwrap(),
                0x8007_0057
            );
            // Unknown device GUID walls.
            ctx.mem.write(guid_at, &[0u8; 16]).unwrap();
            assert!(matches!(
                create_device3(ctx, 0, &[guid_at, 0x2000, out_at, 0]),
                Err(Stop::NotWritten { .. })
            ));
            // HAL_DEVICE succeeds, produces IDirect3DDevice3, and records parent IDirect3D3.
            let d3d3 = crate::com::create(ctx, &super::D3D3, crate::win32::ComState::None).unwrap();
            ctx.mem.write(guid_at, &HAL_DEVICE).unwrap();
            assert_eq!(
                create_device3(ctx, d3d3, &[guid_at, 0x2000, out_at, 0]).unwrap(),
                S_OK
            );
            let dev = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!(ctx.state.com.interface_of(dev), Some(DEVICE3.name));
            assert_eq!(super::get_direct3d3(ctx, dev, &[out_at]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(out_at).unwrap(), d3d3);

            // GetCaps writes DX6/DX5 hardware desc, zeroed HEL desc, and rejects bogus dwSize.
            let hw = 0x1200u32;
            let hel = 0x1400u32;
            ctx.mem.write_u32(hw, 172).unwrap();
            assert_eq!(get_caps3(ctx, dev, &[hw, 0]).unwrap(), 0x8007_0057);
            ctx.mem.write_u32(hw, super::DEVICE_DESC_BYTES).unwrap();
            assert_eq!(get_caps3(ctx, dev, &[hw, 0]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(hw).unwrap(), super::DEVICE_DESC_BYTES);

            ctx.mem.write_u32(hw, DEVICE_DESC6_BYTES).unwrap();
            ctx.mem.write_u32(hel, DEVICE_DESC6_BYTES).unwrap();
            assert_eq!(get_caps3(ctx, dev, &[hw, hel]).unwrap(), S_OK);
            assert_eq!(ctx.mem.read_u32(hw).unwrap(), DEVICE_DESC6_BYTES);
            assert_eq!(ctx.mem.read_u32(hw + 0x08).unwrap(), 2); // D3DCOLOR_RGB
            assert_eq!(ctx.mem.read_u32(hw + 0x64 + 0x18).unwrap(), 0x0000_00ff);
            assert_eq!(ctx.mem.read_u32(hw + 0x64 + 0x20).unwrap(), 0x0000_000d);
            assert_eq!(ctx.mem.read_u32(hel).unwrap(), DEVICE_DESC6_BYTES);
            assert_eq!(ctx.mem.read_u32(hel + 0x08).unwrap(), 0);

            // EnumTextureFormats rejects null callback and queues formats for non-null callback.
            assert_eq!(
                enum_texture_formats7(ctx, dev, &[0, 0]).unwrap(),
                0x8007_0057
            );
            assert_eq!(
                enum_texture_formats7(ctx, dev, &[0x4000, 0x1234]).unwrap(),
                S_OK
            );
            assert!(ctx.call_guest.is_some());

            // D3DTSS_ADDRESS (12) propagates to ADDRESSU (13) and ADDRESSV (14),
            // and reads alias ADDRESSU so independent U updates are reflected.
            assert_eq!(
                set_texture_stage_state(ctx, dev, &[0, tss::ADDRESS, 3]).unwrap(),
                S_OK
            );
            let d = super::device(ctx, dev);
            assert_eq!(d.get_stage(0, tss::ADDRESSU), Some(3));
            assert_eq!(d.get_stage(0, tss::ADDRESSV), Some(3));
            assert_eq!(
                set_texture_stage_state(ctx, dev, &[0, tss::ADDRESSU, 1]).unwrap(),
                S_OK
            );
            assert_eq!(
                super::get_texture_stage_state(ctx, dev, &[0, tss::ADDRESS, out_at]).unwrap(),
                S_OK
            );
            assert_eq!(ctx.mem.read_u32(out_at).unwrap(), 1);

            // DEVICE3 draws XYZRHW triangles through a title-described SetViewport2 viewport.
            let pixels = ctx.state.heap.alloc(16 * 16 * 2);
            let surface = crate::com::create(
                ctx,
                &crate::win32::ddraw::DIRECT_DRAW_SURFACE4,
                crate::win32::ComState::Surface {
                    pixels,
                    width: 16,
                    height: 16,
                    pitch: 32,
                    format: crate::raster::PixelFormat::RGB565,
                    palette: 0,
                    primary: false,
                    back: 0,
                    caps: 0,
                    owned_pixels: true,
                },
            )
            .unwrap();
            assert_eq!(
                create_device3(ctx, d3d3, &[guid_at, surface, out_at, 0]).unwrap(),
                S_OK
            );
            let draw_dev = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!(
                super::create_viewport(ctx, d3d3, &[out_at, 0]).unwrap(),
                S_OK
            );
            let vp = ctx.mem.read_u32(out_at).unwrap();
            let vp_desc = 0x1600u32;
            ctx.mem.write(vp_desc, &[0u8; 44]).unwrap();
            ctx.mem.write_u32(vp_desc, 44).unwrap();
            ctx.mem.write_u32(vp_desc + 12, 16).unwrap();
            ctx.mem.write_u32(vp_desc + 16, 16).unwrap();
            assert_eq!(super::set_viewport(ctx, vp, &[vp_desc]).unwrap(), S_OK);
            assert_eq!(super::add_viewport(ctx, draw_dev, &[vp]).unwrap(), S_OK);
            assert_eq!(
                super::set_current_viewport(ctx, draw_dev, &[vp]).unwrap(),
                S_OK
            );
            assert_eq!(super::begin_scene(ctx, draw_dev, &[]).unwrap(), S_OK);
            // Disable culling and depth test for a flat red XYZRHW triangle.
            assert_eq!(
                super::set_render_state(ctx, draw_dev, &[22, 1]).unwrap(),
                S_OK
            );
            assert_eq!(
                super::set_render_state(ctx, draw_dev, &[7, 0]).unwrap(),
                S_OK
            );
            let verts = 0x1700u32;
            let tri: [[f32; 4]; 3] = [
                [0.0, 0.0, 0.5, 1.0],
                [16.0, 0.0, 0.5, 1.0],
                [0.0, 16.0, 0.5, 1.0],
            ];
            for (i, pos) in tri.iter().enumerate() {
                let base = verts + (i as u32) * 32;
                for (j, f) in pos.iter().enumerate() {
                    ctx.mem
                        .write_u32(base + (j as u32) * 4, f.to_bits())
                        .unwrap();
                }
                ctx.mem.write_u32(base + 16, 0xffff_0000).unwrap(); // diffuse red
                ctx.mem.write_u32(base + 20, 0).unwrap();
                ctx.mem.write_u32(base + 24, 0).unwrap();
                ctx.mem.write_u32(base + 28, 0).unwrap();
            }
            assert_eq!(
                super::draw_primitive(ctx, draw_dev, &[4, 0x1c4, verts, 3, 0x1c]).unwrap(),
                S_OK
            );
            assert_eq!(super::end_scene(ctx, draw_dev, &[]).unwrap(), S_OK);
            // Top-left pixel inside triangle should now be non-zero red (RGB565 0xf800).
            let px = u16::from_le_bytes(ctx.mem.slice(pixels + 34, 2).unwrap().try_into().unwrap());
            assert_eq!(px, 0xf800);
        });
    }
}
