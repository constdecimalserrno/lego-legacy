//! COM objects the guest can hold and call.
//!
//! Every DirectX interface is the same shape: the guest has a pointer to an
//! object whose first word points at a table of function pointers, and it
//! calls through that table. The table lives in guest memory and every slot in
//! it is an address on the boundary, so a call arrives here (ADR-0002).

use crate::cpu::Stop;
use crate::guest::heap::Heap;
use crate::guest::{Fault, GuestMem};
use crate::win32::Ctx;
use std::collections::{BTreeMap, VecDeque};

/// How many slots every interface's table is given.
///
/// Larger than any interface on the roster, so that a call past what is
/// written lands on the boundary rather than in the heap.
pub const SLOTS_PER_INTERFACE: usize = 128;

/// One COM object the machine made.
pub struct ComObject {
    pub interface: &'static str,
    pub refs: i32,
    /// The object this is a face of: its own address for the object a call
    /// made, and that object's for every face the guest asked it for. What an
    /// object keeps on the side is kept under this, so every face sees it.
    pub identity: u32,
    /// What the guest hung on this object with `SetAppData`. Its own pointer,
    /// which it expects to read back unchanged.
    pub app_data: u32,
    /// Whatever the interface keeps about this particular object.
    pub state: ComState,
}

/// What an interface remembers about one of its objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ComState {
    #[default]
    None,
    Device {
        keyboard: bool,
        exclusive: bool,
        acquired: bool,
    },
    /// A sound buffer, whose bytes and playing state DirectSound keeps
    /// (`dsound::Sounds`).
    Sound,
    /// A drawing surface: guest memory, and how to read it.
    Surface {
        pixels: u32,
        /// Whether DirectDraw allocated `pixels` from the guest heap (false
        /// once `SetSurfaceDesc` installs client-owned memory).
        owned_pixels: bool,
        width: u32,
        height: u32,
        pitch: u32,
        /// How a pixel is laid out: what the title asked for, or the screen's.
        format: crate::raster::PixelFormat,
        primary: bool,
        /// What `GetSurfaceDesc` says it is, its `DDSCAPS_*`: what the title
        /// asked for, and where DirectDraw put it (#210).
        caps: u32,
        /// The surface behind this one, when it flips.
        back: u32,
        /// The palette an indexed surface's pixels are looked up in.
        palette: u32,
    },
    /// A palette's own 256 entries, in guest memory.
    Palette { store: u32 },
    /// A multimedia stream: a movie player, whose state is in the players.
    MultiMediaStream,
    /// A DirectShow filter graph (`CLSID_FilterGraph`), whose state is in `State::filter_graphs` (#372).
    FilterGraph,
    /// One stream of a player's movie.
    MediaStream { owner: u32, video: bool },
    /// A sample that a player's video is updated into: the surface it wraps.
    StreamSample { owner: u32, surface: u32 },
    /// One stream of an AVIFile movie: the file object, and which stream.
    AviStream { file: u32, stream: usize },
    /// A three-dimensional device, and the surface it draws into.
    Device3D { surface: u32 },
    /// A buffer of vertices the title fills and then draws from (#57): where
    /// they are, how they are laid out, and how many fit.
    VertexBuffer { store: u32, fvf: u32, count: u32 },
    /// A frame's children or its lights, as they were when a title asked for
    /// them.
    Array { items: Vec<u32> },
    /// What a viewport's `Pick` found, nearest first: each visual, the array
    /// of frames from its scene's root down to it, and its `D3DRMPICKDESC`
    /// as the five words it is written as.
    PickedArray { picks: Vec<(u32, u32, [u32; 5])> },
    /// Something a title draws itself: its callback, and what to pass it.
    UserVisual { callback: u32, argument: u32 },
    /// Retained mode's device: its surface, and how many buffers the title
    /// said it flips between.
    RetainedDevice {
        surface: u32,
        buffers: u32,
        /// The immediate-mode device underneath, once one has been asked for.
        immediate: u32,
    },
    /// A viewport: which device, where on it, and what it clears to.
    Viewport {
        device: u32,
        /// The frame it looks from, for a retained-mode viewport.
        camera: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        background: u32,
        /// The immediate-mode viewport underneath a retained-mode one, once
        /// a title has asked for it.
        immediate: u32,
    },
}

/// Deep table of all guest COM objects, their interface faces, reclaimed
/// 8-byte guest slots, and cached guest vtables.
#[derive(Default)]
pub struct ComTable {
    /// COM objects and faces, by the guest pointer that names them.
    objects: BTreeMap<u32, ComObject>,
    /// Every face of every object, by `(identity, interface_name)`.
    faces: BTreeMap<(u32, &'static str), u32>,
    /// Freed 8-byte COM face slots reused by [`create`] and [`face`].
    free_slots: VecDeque<u32>,
    /// Which vtable an interface's methods live behind, once one is made.
    vtables: BTreeMap<&'static str, u32>,
}

impl ComTable {
    /// Remember a primary COM object at `at` with initial reference count 1.
    pub fn insert(&mut self, at: u32, interface: &'static str, state: ComState) {
        self.insert_entry(at, at, interface, state);
    }

    fn insert_entry(&mut self, identity: u32, at: u32, interface: &'static str, state: ComState) {
        self.faces.insert((identity, interface), at);
        self.objects.insert(
            at,
            ComObject {
                interface,
                refs: 1,
                identity,
                app_data: 0,
                state,
            },
        );
    }

    fn alloc_slot(&mut self, heap: &mut Heap) -> u32 {
        self.free_slots.pop_front().unwrap_or_else(|| heap.alloc(8))
    }

    /// The object a face belongs to.
    #[must_use]
    pub fn identity(&self, at: u32) -> u32 {
        self.objects.get(&at).map_or(at, |o| o.identity)
    }

    /// The primary object a guest pointer is a face of: its state and app data,
    /// whichever face the pointer is.
    #[must_use]
    pub fn get(&self, at: u32) -> Option<&ComObject> {
        let object = self.objects.get(&at)?;
        if object.identity == at {
            Some(object)
        } else {
            self.objects.get(&object.identity)
        }
    }

    pub fn get_mut(&mut self, at: u32) -> Option<&mut ComObject> {
        let identity = {
            let object = self.objects.get_mut(&at)?;
            if object.identity == at {
                // Safety: `object` is a unique mutable borrow from `self.objects`;
                // we return immediately without touching `self.objects` again and
                // tie the returned lifetime to `&mut self`.
                let ptr: *mut ComObject = object;
                return Some(unsafe { &mut *ptr });
            }
            object.identity
        };
        self.objects.get_mut(&identity)
    }

    /// Which interface a guest pointer is: the face itself, not the object.
    #[must_use]
    pub fn interface_of(&self, at: u32) -> Option<&'static str> {
        self.objects.get(&at).map(|o| o.interface)
    }

    /// Read-only view of primary COM objects (where `obj.identity == addr`).
    pub fn primary_objects(&self) -> impl Iterator<Item = (u32, &ComObject)> {
        self.objects
            .iter()
            .filter(|(&addr, obj)| obj.identity == addr)
            .map(|(&addr, obj)| (addr, obj))
    }

    /// Increment one reference on face `at` and return the new reference count.
    pub fn add_ref(&mut self, at: u32) -> u32 {
        let Some(object) = self.objects.get_mut(&at) else {
            return 0;
        };
        object.refs = object.refs.saturating_add(1).max(0);
        u32::try_from(object.refs).unwrap_or(0)
    }

    /// Decrement one reference on face `at` (clamped at 0), returning
    /// `(remaining_refs, identity)`. Private to `com` so zero-ref drops can
    /// never bypass [`release_ref`] reclamation.
    fn dec_ref(&mut self, at: u32) -> Option<(u32, u32)> {
        let object = self.objects.get_mut(&at)?;
        object.refs = object.refs.saturating_sub(1).max(0);
        let left = u32::try_from(object.refs).unwrap_or(0);
        Some((left, object.identity))
    }

    fn face_entries(&self, identity: u32) -> impl Iterator<Item = ((u32, &'static str), u32)> + '_ {
        use std::ops::Bound;
        let end = match identity.checked_add(1) {
            Some(next) => Bound::Excluded((next, "")),
            None => Bound::Unbounded,
        };
        self.faces
            .range((Bound::Included((identity, "")), end))
            .map(|(&k, &v)| (k, v))
    }

    /// Whether any face sharing `identity` still holds a live reference.
    #[must_use]
    pub fn has_live_refs(&self, identity: u32) -> bool {
        self.face_entries(identity)
            .any(|(_, addr)| self.objects.get(&addr).is_some_and(|o| o.refs > 0))
    }

    /// Remove every COM face belonging to `identity` and return its 8-byte
    /// guest slot to `free_slots` for reuse by [`create`] / [`face`].
    fn reclaim_faces(&mut self, identity: u32) {
        loop {
            let Some((key, face_addr)) = self.face_entries(identity).next() else {
                break;
            };
            self.faces.remove(&key);
            if self.objects.remove(&face_addr).is_some() {
                self.free_slots.push_back(face_addr);
            }
        }
    }

    /// The vtable an interface's objects share, once it has one.
    #[must_use]
    pub fn vtable(&self, interface: &'static str) -> Option<u32> {
        self.vtables.get(interface).copied()
    }

    /// True if any live `GUID_SysMouse` holds a `DISCL_EXCLUSIVE` DirectInput acquisition.
    #[must_use]
    pub(crate) fn has_exclusive_acquired_mouse(&self) -> bool {
        self.objects.values().any(|o| {
            matches!(
                o.state,
                ComState::Device {
                    keyboard: false,
                    exclusive: true,
                    acquired: true,
                }
            ) && self.has_live_refs(o.identity)
        })
    }

    /// True if any live flipping-chain parent, `Device3D`, `RetainedDevice`, or
    /// `StreamSample` still guards surface `identity`.
    fn has_live_surface_holder(&self, identity: u32) -> bool {
        self.objects.iter().any(|(&addr, o)| match o.state {
            ComState::Surface { back: b, .. } => b != 0 && self.identity(b) == identity,
            ComState::Device3D { surface }
            | ComState::RetainedDevice { surface, .. }
            | ComState::StreamSample { surface, .. } => {
                surface != 0 && self.identity(surface) == identity && self.has_live_refs(addr)
            }
            _ => false,
        })
    }

    fn has_surface_with_pixels(&self, pixels: u32) -> bool {
        self.objects
            .values()
            .any(|o| matches!(o.state, ComState::Surface { pixels: p, .. } if p == pixels))
    }

    fn has_live_palette_holder(&self, identity: u32, primary_only: bool) -> bool {
        self.objects.values().any(|o| {
            matches!(
                o.state,
                ComState::Surface {
                    primary, palette, ..
                } if (!primary_only || primary)
                    && palette != 0
                    && self.identity(palette) == identity
            )
        })
    }

    fn has_live_device_holder(&self, identity: u32) -> bool {
        self.objects.iter().any(|(&addr, o)| {
            matches!(
                o.state,
                ComState::Viewport { device, .. }
                    if device != 0
                        && self.identity(device) == identity
                        && self.has_live_refs(addr)
            )
        })
    }
}

fn resolve_interface_name(s: &str) -> Option<&'static str> {
    crate::win32::interface_named(s).map(|i| i.name)
}

fn ensure_vtable(ctx: &mut Ctx<'_>, interface: &'static Interface) -> Result<u32, Fault> {
    if let Some(at) = ctx.state.com.vtable(interface.name) {
        return Ok(at);
    }
    // The table is bound wider than the methods that are written, so a call
    // into a slot nobody declared arrives at the boundary and stops with a
    // name. A vtable that ended where the written methods end would have the
    // guest read whatever is next in the heap and call it, which is a jump
    // into nothing half a million instructions later.
    let slots = interface.methods.len().max(SLOTS_PER_INTERFACE);
    let addresses = ctx.boundary.bind_interface(interface.name, slots);
    let at = ctx
        .state
        .heap
        .alloc(u32::try_from(addresses.len() * 4).unwrap_or(4));
    for (i, address) in addresses.iter().enumerate() {
        ctx.mem.write_u32(at + (i as u32) * 4, *address)?;
    }
    ctx.state.com.vtables.insert(interface.name, at);
    Ok(at)
}

/// Make a COM object the guest can call through.
///
/// The object is one word of guest memory pointing at the interface's table,
/// which is itself guest memory full of boundary addresses.
///
/// # Errors
/// When the heap is exhausted or the write faults.
pub fn create(
    ctx: &mut Ctx<'_>,
    interface: &'static Interface,
    state: ComState,
) -> Result<u32, Fault> {
    let vtable = ensure_vtable(ctx, interface)?;
    let object = ctx.state.com.alloc_slot(&mut ctx.state.heap);
    ctx.mem.write_u32(object, vtable)?;
    ctx.state.com.insert(object, interface.name, state);
    Ok(object)
}

/// Hand back an object's face as another interface: the same face again if
/// the guest has had it before, as COM promises, and a new one sharing the
/// object's state if not.
///
/// # Errors
/// When the heap is exhausted or the write faults.
pub fn face(ctx: &mut Ctx<'_>, of: u32, interface: &'static Interface) -> Result<u32, Fault> {
    let identity = ctx.state.com.identity(of);
    if let Some(at) = ctx
        .state
        .com
        .faces
        .get(&(identity, interface.name))
        .copied()
    {
        ctx.state.com.add_ref(at);
        return Ok(at);
    }
    // The face keeps nothing of its own: what the object keeps is found
    // through its identity, so a change made through one face is seen through
    // every other.
    let vtable = ensure_vtable(ctx, interface)?;
    let at = ctx.state.com.alloc_slot(&mut ctx.state.heap);
    ctx.mem.write_u32(at, vtable)?;
    ctx.state
        .com
        .insert_entry(identity, at, interface.name, ComState::None);
    Ok(at)
}

enum ReclaimWork {
    ReleaseFace(u32),
    ReclaimSurface(u32),
}

/// Decrement one reference on COM face `at`, returning the face's remaining
/// count and draining iterative-worklist reclamation whenever all faces of an
/// object reach zero references.
pub fn release_ref(ctx: &mut Ctx<'_>, at: u32) -> u32 {
    let Some((left, identity)) = ctx.state.com.dec_ref(at) else {
        return 0;
    };
    if left != 0 {
        return left;
    }
    let mut work = Vec::new();
    enqueue_reclaim_for_identity(ctx, identity, &mut work);
    drain_reclaim_worklist(ctx, &mut work);
    0
}

/// Reclaim a DirectDraw surface's pixel buffer, COM faces, and side-table
/// entries once every face of `identity` has refcount 0 and no live flipping-
/// chain parent, 3D device, or stream sample still holds it.
pub(crate) fn reclaim_surface_if_unreferenced(ctx: &mut Ctx<'_>, identity: u32) {
    let mut work = vec![ReclaimWork::ReclaimSurface(identity)];
    drain_reclaim_worklist(ctx, &mut work);
}

/// Reclaim a DirectDraw palette's 1 KiB store and COM faces once every face of
/// `identity` has refcount 0 and no live surface or screen palette still holds it.
pub(crate) fn reclaim_palette_if_unreferenced(
    ctx: &mut Ctx<'_>,
    identity: u32,
    from_primary: bool,
) {
    let Some(&ComState::Palette { store }) = ctx.state.com.get(identity).map(|o| &o.state) else {
        return;
    };
    if from_primary
        && store != 0
        && ctx.state.palette == store
        && !ctx.state.com.has_live_palette_holder(identity, true)
    {
        ctx.state.palette = 0;
    }
    if ctx.state.com.has_live_refs(identity)
        || ctx.state.com.has_live_palette_holder(identity, false)
        || (store != 0 && ctx.state.palette == store)
    {
        return;
    }
    ctx.state.com.reclaim_faces(identity);
    if store != 0 {
        if ctx.state.heap.size_of(store) == Some(256 * 4) {
            ctx.state.palette_pool.free(&ctx.state.heap, store);
        } else {
            ctx.state.heap.free(store);
        }
    }
}

fn enqueue_reclaim_for_identity(ctx: &mut Ctx<'_>, identity: u32, work: &mut Vec<ReclaimWork>) {
    if ctx.state.com.has_live_refs(identity) {
        return;
    }
    match ctx.state.com.get(identity).map(|o| &o.state) {
        Some(ComState::Surface { .. }) => work.push(ReclaimWork::ReclaimSurface(identity)),
        Some(ComState::Palette { .. }) => reclaim_palette_if_unreferenced(ctx, identity, false),
        Some(&ComState::VertexBuffer { store, .. }) => {
            if store != 0 {
                ctx.state.vb_pool.free(&ctx.state.heap, store);
            }
            ctx.state.com.reclaim_faces(identity);
        }
        Some(&ComState::Device3D { surface }) => {
            if ctx.state.com.has_live_device_holder(identity) {
                return;
            }
            if surface != 0 {
                let target = ctx.state.com.identity(surface);
                work.push(ReclaimWork::ReclaimSurface(target));
            }
            for tex in ctx
                .state
                .immediate
                .reclaim_device(identity)
                .into_iter()
                .rev()
            {
                work.push(ReclaimWork::ReleaseFace(tex));
            }
            ctx.state.com.reclaim_faces(identity);
        }
        Some(&ComState::RetainedDevice {
            surface, immediate, ..
        }) => {
            if ctx.state.com.has_live_device_holder(identity) {
                return;
            }
            if surface != 0 {
                let target = ctx.state.com.identity(surface);
                work.push(ReclaimWork::ReclaimSurface(target));
            }
            if immediate != 0 {
                work.push(ReclaimWork::ReleaseFace(immediate));
            }
            ctx.state.names.remove(&identity);
            ctx.state.set_values.retain(|&(id, _), _| id != identity);
            ctx.state.com.reclaim_faces(identity);
        }
        Some(&ComState::Viewport { device, .. }) if device != 0 => {
            let dev_id = ctx.state.com.identity(device);
            enqueue_reclaim_for_identity(ctx, dev_id, work);
        }
        Some(&ComState::StreamSample { surface, .. }) => {
            if surface != 0 {
                let target = ctx.state.com.identity(surface);
                work.push(ReclaimWork::ReclaimSurface(target));
            }
            ctx.state.com.reclaim_faces(identity);
        }
        Some(ComState::Device { .. }) => {
            ctx.state.com.reclaim_faces(identity);
        }
        Some(ComState::Array { items }) => {
            work.extend(
                items
                    .iter()
                    .rev()
                    .copied()
                    .filter(|&item| item != 0)
                    .map(ReclaimWork::ReleaseFace),
            );
            ctx.state.com.reclaim_faces(identity);
        }
        Some(ComState::PickedArray { picks }) => {
            work.extend(
                picks
                    .iter()
                    .rev()
                    .map(|&(_, array, _)| array)
                    .filter(|&array| array != 0)
                    .map(ReclaimWork::ReleaseFace),
            );
            ctx.state.com.reclaim_faces(identity);
        }
        Some(ComState::Sound) => {
            crate::win32::dsound::reclaim_buffer(ctx, identity);
            ctx.state.com.reclaim_faces(identity);
        }
        Some(ComState::MultiMediaStream) => {
            crate::win32::amstream::reclaim_stream(ctx, identity);
        }
        Some(ComState::FilterGraph) => {
            crate::win32::dshow::reclaim_graph(ctx, identity);
            ctx.state.com.reclaim_faces(identity);
        }
        _ => {}
    }
}

fn drain_reclaim_worklist(ctx: &mut Ctx<'_>, work: &mut Vec<ReclaimWork>) {
    while let Some(item) = work.pop() {
        match item {
            ReclaimWork::ReleaseFace(face) => {
                if let Some((0, identity)) = ctx.state.com.dec_ref(face) {
                    enqueue_reclaim_for_identity(ctx, identity, work);
                }
            }
            ReclaimWork::ReclaimSurface(identity) => {
                try_reclaim_surface(ctx, identity, work);
            }
        }
    }
}

fn try_reclaim_surface(ctx: &mut Ctx<'_>, identity: u32, work: &mut Vec<ReclaimWork>) {
    let Some(&ComState::Surface {
        pixels,
        owned_pixels,
        primary,
        back,
        palette,
        ..
    }) = ctx.state.com.get(identity).map(|o| &o.state)
    else {
        return;
    };
    if ctx.state.com.has_live_refs(identity) || ctx.state.com.has_live_surface_holder(identity) {
        return;
    }

    ctx.state.com.reclaim_faces(identity);

    let gdi = &mut ctx.state.gdi;
    ctx.state.surface_dcs.retain(|&dc, &mut s| {
        if s == identity {
            gdi.dcs.remove(&dc);
            false
        } else {
            true
        }
    });
    ctx.state.immediate.remove_handle_for(identity);

    if pixels != 0
        && (ctx.state.color_keys.contains_key(&pixels) || ctx.state.clipped.contains(&pixels))
        && !ctx.state.com.has_surface_with_pixels(pixels)
    {
        ctx.state.color_keys.remove(&pixels);
        ctx.state.clipped.remove(&pixels);
    }

    if owned_pixels && pixels != 0 {
        ctx.state.surface_pool.free(&ctx.state.heap, pixels);
    }

    if palette != 0 {
        let pal_id = ctx.state.com.identity(palette);
        reclaim_palette_if_unreferenced(ctx, pal_id, primary);
    }

    if back != 0 {
        let back_id = ctx.state.com.identity(back);
        work.push(ReclaimWork::ReclaimSurface(back_id));
    }
}

impl crate::snap::Snap for ComState {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let mut tag = match self {
            Self::None => 0u8,
            Self::Device { .. } => 1u8,
            Self::Sound => 2u8,
            Self::Surface { .. } => 3u8,
            Self::Palette { .. } => 4u8,
            Self::MultiMediaStream => 5u8,
            Self::MediaStream { .. } => 6u8,
            Self::StreamSample { .. } => 7u8,
            Self::AviStream { .. } => 8u8,
            Self::Device3D { .. } => 9u8,
            Self::VertexBuffer { .. } => 10u8,
            Self::Array { .. } => 11u8,
            Self::PickedArray { .. } => 12u8,
            Self::UserVisual { .. } => 13u8,
            Self::RetainedDevice { .. } => 14u8,
            Self::Viewport { .. } => 15u8,
            Self::FilterGraph => 16u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => *self = Self::None,
                1 => {
                    let (mut keyboard, mut exclusive, mut acquired) = (false, false, false);
                    v.bool(&mut keyboard);
                    v.bool(&mut exclusive);
                    v.bool(&mut acquired);
                    *self = Self::Device {
                        keyboard,
                        exclusive,
                        acquired,
                    };
                }
                2 => *self = Self::Sound,
                3 => {
                    let (mut pixels, mut width, mut height, mut pitch) = (0u32, 0u32, 0u32, 0u32);
                    let mut owned_pixels = false;
                    let mut format = crate::raster::PixelFormat::RGB565;
                    let (mut primary, mut caps, mut back, mut palette) = (false, 0u32, 0u32, 0u32);
                    v.u32(&mut pixels);
                    v.bool(&mut owned_pixels);
                    v.u32(&mut width);
                    v.u32(&mut height);
                    v.u32(&mut pitch);
                    format.snap(v);
                    v.bool(&mut primary);
                    v.u32(&mut caps);
                    v.u32(&mut back);
                    v.u32(&mut palette);
                    *self = Self::Surface {
                        pixels,
                        owned_pixels,
                        width,
                        height,
                        pitch,
                        format,
                        primary,
                        caps,
                        back,
                        palette,
                    };
                }
                4 => {
                    let mut store = 0u32;
                    v.u32(&mut store);
                    *self = Self::Palette { store };
                }
                5 => *self = Self::MultiMediaStream,
                6 => {
                    let (mut owner, mut video) = (0u32, false);
                    v.u32(&mut owner);
                    v.bool(&mut video);
                    *self = Self::MediaStream { owner, video };
                }
                7 => {
                    let (mut owner, mut surface) = (0u32, 0u32);
                    v.u32(&mut owner);
                    v.u32(&mut surface);
                    *self = Self::StreamSample { owner, surface };
                }
                8 => {
                    let (mut file, mut stream) = (0u32, 0usize);
                    v.u32(&mut file);
                    v.usize(&mut stream);
                    *self = Self::AviStream { file, stream };
                }
                9 => {
                    let mut surface = 0u32;
                    v.u32(&mut surface);
                    *self = Self::Device3D { surface };
                }
                10 => {
                    let (mut store, mut fvf, mut count) = (0u32, 0u32, 0u32);
                    v.u32(&mut store);
                    v.u32(&mut fvf);
                    v.u32(&mut count);
                    *self = Self::VertexBuffer { store, fvf, count };
                }
                11 => {
                    let mut items = Vec::new();
                    crate::snap::snap_u32_vec(v, &mut items);
                    *self = Self::Array { items };
                }
                12 => {
                    let mut picks = Vec::new();
                    crate::snap::snap_vec(v, &mut picks, |(a, b, desc), v| {
                        v.u32(a);
                        v.u32(b);
                        for w in desc {
                            v.u32(w);
                        }
                    });
                    *self = Self::PickedArray { picks };
                }
                13 => {
                    let (mut callback, mut argument) = (0u32, 0u32);
                    v.u32(&mut callback);
                    v.u32(&mut argument);
                    *self = Self::UserVisual { callback, argument };
                }
                14 => {
                    let (mut surface, mut buffers, mut immediate) = (0u32, 0u32, 0u32);
                    v.u32(&mut surface);
                    v.u32(&mut buffers);
                    v.u32(&mut immediate);
                    *self = Self::RetainedDevice {
                        surface,
                        buffers,
                        immediate,
                    };
                }
                15 => {
                    let (
                        mut device,
                        mut camera,
                        mut x,
                        mut y,
                        mut width,
                        mut height,
                        mut background,
                        mut immediate,
                    ) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
                    v.u32(&mut device);
                    v.u32(&mut camera);
                    v.u32(&mut x);
                    v.u32(&mut y);
                    v.u32(&mut width);
                    v.u32(&mut height);
                    v.u32(&mut background);
                    v.u32(&mut immediate);
                    *self = Self::Viewport {
                        device,
                        camera,
                        x,
                        y,
                        width,
                        height,
                        background,
                        immediate,
                    };
                }
                16 => *self = Self::FilterGraph,
                _ => v.mark_corrupt("com::ComState tag"),
            }
        } else {
            match self {
                Self::None | Self::Sound | Self::MultiMediaStream | Self::FilterGraph => {}
                Self::Device {
                    keyboard,
                    exclusive,
                    acquired,
                } => {
                    v.bool(keyboard);
                    v.bool(exclusive);
                    v.bool(acquired);
                }
                Self::Surface {
                    pixels,
                    owned_pixels,
                    width,
                    height,
                    pitch,
                    format,
                    primary,
                    caps,
                    back,
                    palette,
                } => {
                    v.u32(pixels);
                    v.bool(owned_pixels);
                    v.u32(width);
                    v.u32(height);
                    v.u32(pitch);
                    format.snap(v);
                    v.bool(primary);
                    v.u32(caps);
                    v.u32(back);
                    v.u32(palette);
                }
                Self::Palette { store } => v.u32(store),
                Self::MediaStream { owner, video } => {
                    v.u32(owner);
                    v.bool(video);
                }
                Self::StreamSample { owner, surface } => {
                    v.u32(owner);
                    v.u32(surface);
                }
                Self::AviStream { file, stream } => {
                    v.u32(file);
                    v.usize(stream);
                }
                Self::Device3D { surface } => v.u32(surface),
                Self::VertexBuffer { store, fvf, count } => {
                    v.u32(store);
                    v.u32(fvf);
                    v.u32(count);
                }
                Self::Array { items } => crate::snap::snap_u32_vec(v, items),
                Self::PickedArray { picks } => {
                    crate::snap::snap_vec(v, picks, |(a, b, desc), v| {
                        v.u32(a);
                        v.u32(b);
                        for w in desc {
                            v.u32(w);
                        }
                    });
                }
                Self::UserVisual { callback, argument } => {
                    v.u32(callback);
                    v.u32(argument);
                }
                Self::RetainedDevice {
                    surface,
                    buffers,
                    immediate,
                } => {
                    v.u32(surface);
                    v.u32(buffers);
                    v.u32(immediate);
                }
                Self::Viewport {
                    device,
                    camera,
                    x,
                    y,
                    width,
                    height,
                    background,
                    immediate,
                } => {
                    v.u32(device);
                    v.u32(camera);
                    v.u32(x);
                    v.u32(y);
                    v.u32(width);
                    v.u32(height);
                    v.u32(background);
                    v.u32(immediate);
                }
            }
        }
    }
}

impl crate::snap::Snap for ComObject {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            interface,
            refs,
            identity,
            app_data,
            state,
        } = self;
        crate::snap::snap_static_str(v, interface, "ComObject interface", resolve_interface_name);
        v.i32(refs);
        v.u32(identity);
        v.u32(app_data);
        state.snap(v);
    }
}

impl crate::snap::Snap for ComTable {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            objects,
            faces,
            free_slots,
            vtables,
        } = self;
        crate::snap::snap_btree_map_with(
            v,
            objects,
            u32::default,
            || ComObject {
                interface: "",
                refs: 0,
                identity: 0,
                app_data: 0,
                state: ComState::None,
            },
            |k, v| v.u32(k),
            |obj, v| obj.snap(v),
        );
        crate::snap::snap_btree_map_with(
            v,
            faces,
            || (0u32, ""),
            u32::default,
            |(id, iface), v| {
                v.u32(id);
                crate::snap::snap_static_str(v, iface, "faces interface", resolve_interface_name);
            },
            |val, v| v.u32(val),
        );
        crate::snap::snap_vec_deque(v, free_slots, |addr, v| v.u32(addr));
        crate::snap::snap_btree_map_with(
            v,
            vtables,
            || "",
            u32::default,
            |iface, v| {
                crate::snap::snap_static_str(v, iface, "vtables interface", resolve_interface_name);
            },
            |val, v| v.u32(val),
        );
        if v.is_reading() && !v.failed() {
            let mut seen_free = std::collections::BTreeSet::new();
            let objects_valid = faces.len() == objects.len()
                && objects.iter().all(|(&addr, obj)| {
                    addr != 0
                        && faces.get(&(obj.identity, obj.interface)) == Some(&addr)
                        && objects
                            .get(&obj.identity)
                            .is_some_and(|p| p.identity == obj.identity)
                });
            let free_valid = free_slots
                .iter()
                .all(|&slot| slot != 0 && !objects.contains_key(&slot) && seen_free.insert(slot));
            if !objects_valid || !free_valid {
                objects.clear();
                faces.clear();
                free_slots.clear();
                v.mark_corrupt("ComTable invariants");
            }
        }
    }
}

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
/// Reference counting is real. Once every face sharing an object's identity
/// reaches zero references, [`release_ref`] reclaims DirectDraw surfaces
/// (#338), palettes (#406), Direct3D vertex buffers (#406), DirectSound buffers
/// (#385), DirectShow filter graphs (#372, #405), `AMMultiMediaStream` players,
/// `Device3D`, `RetainedDevice`, `StreamSample`, DirectInput `Device`, and
/// D3DRM `Array`/`PickedArray` COM objects; other COM objects stay held until
/// the run ends.
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
        "IDirectMusic",
        parse_iid("6536115a-7b2d-11d2-ba18-0000f875ac12"),
    ),
    (
        "IDirectMusicPort",
        parse_iid("08f2d8c9-37c2-11d2-b9f9-0000f875ac12"),
    ),
    (
        "IDirectMusicPerformance",
        parse_iid("07d43d03-6523-11d2-871d-00600893b1bd"),
    ),
    (
        "IDirectMusicLoader",
        parse_iid("2ffaaca2-5dca-11d2-afa6-00aa0024d8b6"),
    ),
    (
        "IDirectMusicSegment",
        parse_iid("f96029a2-4282-11d2-8717-00600893b1bd"),
    ),
    (
        "IDirectMusicStyle",
        parse_iid("d2ac28bd-b39b-11d1-8704-00600893b1bd"),
    ),
    (
        "IDirectMusicComposer",
        parse_iid("d2ac28bf-b39b-11d1-8704-00600893b1bd"),
    ),
    (
        "IDirectMusicBand",
        parse_iid("d2ac28c0-b39b-11d1-8704-00600893b1bd"),
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
    let Some(have) = ctx.state.com.interface_of(this) else {
        return Ok(E_NOINTERFACE);
    };

    if let Some(asked) = asked {
        // IUnknown is the object's identity, the same pointer whichever face is
        // asked: how COM says two pointers are one object.
        if asked == IUNKNOWN {
            let identity = ctx.state.com.identity(this);
            ctx.state.com.add_ref(identity);
            if out != 0 {
                ctx.mem.write_u32(out, identity).map_err(Stop::Fault)?;
            }
            return Ok(S_OK);
        }

        // Asked for what it already is: the same face, one more reference.
        if is_itself(have, &asked) {
            ctx.state.com.add_ref(this);
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
                let object = face(ctx, this, interface).map_err(Stop::Fault)?;
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
    Ok(ctx.state.com.add_ref(this))
}

fn release(ctx: &mut Ctx<'_>, this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(release_ref(ctx, this))
}

#[cfg(test)]
mod tests {
    use super::{
        create, face, iid_text, is_itself, parse_iid, query_interface, ComState, E_NOINTERFACE,
        FACES, IUNKNOWN, OWN, RM_OBJECT, RM_VISUAL, S_OK,
    };
    use crate::cpu::Stop;
    use crate::guest::GuestMem;
    use crate::win32::tests::with_test_ctx;

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
            let dd =
                create(ctx, &crate::win32::ddraw::DIRECT_DRAW, ComState::None).expect("create");

            // 1. Face upgrade: IDirectDraw -> IDirectDraw4 and IDirect3D3.
            let dd4_iid = parse_iid("9c59509a-39bd-11d1-8c4a-00c04fd930c5");
            ctx.mem.write(iid_at, &dd4_iid).unwrap();
            assert_eq!(query_interface(ctx, dd, &[iid_at, out_at]).unwrap(), S_OK);
            let dd4 = ctx.mem.read_u32(out_at).unwrap();
            assert_ne!(dd4, dd);
            assert_eq!(ctx.state.com.interface_of(dd4), Some("IDirectDraw4"));

            let d3d3_iid = parse_iid("bb223240-e72b-11d0-a9b4-00aa00c0993e");
            ctx.mem.write(iid_at, &d3d3_iid).unwrap();
            assert_eq!(query_interface(ctx, dd, &[iid_at, out_at]).unwrap(), S_OK);
            let d3d3 = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!(ctx.state.com.interface_of(d3d3), Some("IDirect3D3"));

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
                let got_face = ctx.mem.read_u32(out_at).unwrap();
                assert_eq!(ctx.state.com.interface_of(got_face), Some(name));
                assert_eq!(ctx.state.com.identity(got_face), graph);
                ptrs.push((name, got_face));
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
                    assert_eq!(ctx.state.com.interface_of(got), Some(to_name));
                }
            }
        });
    }

    #[test]
    fn com_table_fast_path_and_secondary_face_lookup() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let dd = create(
                ctx,
                &crate::win32::ddraw::DIRECT_DRAW,
                ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                },
            )
            .expect("create");
            let dd4 = face(ctx, dd, &crate::win32::ddraw::DIRECT_DRAW4).expect("face");
            assert_ne!(dd, dd4);
            assert_eq!(ctx.state.com.get(dd).map(|o| o.identity), Some(dd));
            assert_eq!(ctx.state.com.get(dd4).map(|o| o.identity), Some(dd));
            assert_eq!(
                ctx.state.com.get(dd4).map(|o| &o.state),
                Some(&ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                })
            );
            ctx.state.com.get_mut(dd4).expect("face").app_data = 0x1234;
            assert_eq!(ctx.state.com.get(dd).expect("primary").app_data, 0x1234);
            ctx.state.com.get_mut(dd).expect("primary").app_data = 0x5678;
            assert_eq!(ctx.state.com.get(dd4).expect("face").app_data, 0x5678);
            assert!(ctx.state.com.get(0xdead_beef).is_none());
            assert!(ctx.state.com.get_mut(0xdead_beef).is_none());
        });
    }
}
