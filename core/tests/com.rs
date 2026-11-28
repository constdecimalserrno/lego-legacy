//! COM objects, and what happens when a guest calls a slot nobody wrote.

use lego_legacy_core::win32;

#[test]
fn every_interface_starts_with_the_three_slots_every_com_object_has() {
    for interface in win32::interfaces() {
        let names: Vec<&str> = interface.methods.iter().map(|m| m.name).collect();
        assert_eq!(
            &names[..3],
            ["QueryInterface", "AddRef", "Release"],
            "{} does not start where every COM object starts",
            interface.name
        );
    }
}

#[test]
fn a_slot_nobody_declared_is_not_answered() {
    // The machine has eight methods on this interface; a guest calling past
    // them stops the run rather than being given whatever is next in memory.
    assert!(win32::com_method("IDirectInputA", 7).is_some());
    assert!(win32::com_method("IDirectInputA", 8).is_none());
    assert!(win32::com_method("IDirectInputDeviceA", 99).is_none());
    assert!(win32::com_method("INoSuchInterface", 0).is_none());
}

#[test]
fn every_method_says_how_many_arguments_it_takes() {
    // The count is what cleans the stack when the call returns, so a wrong one
    // is a guest that carries on with a stack pointer nobody can explain. The
    // longest method in the interfaces' own headers is retained mode's
    // CreateWrap, at sixteen.
    for interface in win32::interfaces() {
        for (slot, method) in interface.methods.iter().enumerate() {
            assert!(
                method.args <= 16,
                "{}::{} at slot {slot} claims {} arguments",
                interface.name,
                method.name,
                method.args
            );
        }
    }
}

fn with_ctx<R>(f: impl FnOnce(&mut win32::Ctx<'_>) -> R) -> R {
    use lego_legacy_core::cli::Trace;
    use lego_legacy_core::cpu::Cpu;
    use lego_legacy_core::guest::heap::HEAP_BASE;
    use lego_legacy_core::guest::Arena;
    use lego_legacy_core::host::headless::Headless;
    use lego_legacy_core::install::Tree;
    use lego_legacy_core::loader::Modules;
    use lego_legacy_core::runlog::RunLog;
    use lego_legacy_core::shim::Boundary;
    use lego_legacy_core::Title;
    use std::sync::Once;

    static INIT_RUNS: Once = Once::new();
    INIT_RUNS.call_once(|| {
        std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    });
    let mut cpu = Cpu::default();
    let mut mem = Arena::new(&mut Headless::new(), HEAP_BASE as usize + 0x20_0000).expect("arena");
    let mut state = win32::State::new();
    let mut log = RunLog::create("com-test", Trace::Off).expect("runlog");
    let dir = log.dir().to_path_buf();
    let mut tree = Tree::default();
    let mut modules = Modules::default();
    let mut boundary = Boundary::new();
    let out = {
        let mut ctx = win32::Ctx {
            cpu: &mut cpu,
            mem: &mut mem,
            state: &mut state,
            title: &Title::STUB,
            log: &mut log,
            tree: &mut tree,
            modules: &mut modules,
            clock_millis: 0,
            boundary: &mut boundary,
            call_guest: None,
            float_answer: None,
            answer_is_the_call_s: false,
        };
        f(&mut ctx)
    };
    drop(log);
    let _ = std::fs::remove_dir_all(dir);
    out
}

#[test]
fn slab_pool_exact_and_best_fit_fifo_respect_cap_and_overflow() {
    use lego_legacy_core::guest::heap::{Heap, SlabPool};

    let mut heap = Heap::new();

    // Exact pool: strict FIFO per bucket, refuses larger buckets, ignores unknown addrs.
    let mut exact = SlabPool::exact();
    let (a, reused) = exact.alloc(&mut heap, 64);
    assert!(!reused && a != 0);
    let (b, reused) = exact.alloc(&mut heap, 64);
    assert!(!reused && b != 0 && b != a);
    let (big, reused) = exact.alloc(&mut heap, 128);
    assert!(!reused && big != 0);

    exact.free(&heap, b);
    exact.free(&heap, a);
    exact.free(&heap, big);
    exact.free(&heap, 0);
    exact.free(&heap, 0xdead_beef);

    // Exact mode must not hand out 128-byte slab for a 72-byte request.
    let (c, reused) = exact.alloc(&mut heap, 72);
    assert!(!reused && c != big);
    // Strict FIFO order inside the 64-byte bucket: b then a.
    assert_eq!(exact.alloc(&mut heap, 64), (b, true));
    assert_eq!(exact.alloc(&mut heap, 64), (a, true));
    assert_eq!(exact.alloc(&mut heap, 128), (big, true));
    // Pool is now empty: next 64-byte allocation bumps heap fresh.
    assert!(!exact.alloc(&mut heap, 64).1);

    // `size == 0` rounds to 8 and `best_fit(0)` clamps to exact (1x).
    let mut clamped = SlabPool::best_fit(0);
    let (z, reused) = clamped.alloc(&mut heap, 0);
    assert!(!reused && z != 0);
    clamped.free(&heap, z);
    assert_eq!(clamped.alloc(&mut heap, 0), (z, true));

    // Best-fit 2x pool: picks smallest bucket in [rounded, 2*rounded] in FIFO order,
    // refuses > 2x slabs (preventing small-block hijacking), and handles size overflow.
    let mut best = SlabPool::best_fit(2);
    let (s16a, _) = best.alloc(&mut heap, 16_384);
    let (s16b, _) = best.alloc(&mut heap, 16_384);
    let (s180, _) = best.alloc(&mut heap, 180_224);
    best.free(&heap, s16a);
    best.free(&heap, s16b);
    best.free(&heap, s180);

    // 11,432 rounded to 11,432 -> 2x cap = 22,864 -> picks 16,384 in FIFO order (s16a then s16b).
    assert_eq!(best.alloc(&mut heap, 11_432), (s16a, true));
    assert_eq!(best.alloc(&mut heap, 11_432), (s16b, true));
    // 1,024 rounded to 1,024 -> 2x cap = 2,048 -> must NOT hijack 180,224 slab.
    let (tiny, reused) = best.alloc(&mut heap, 1_024);
    assert!(!reused && tiny != s180);
    // 100,000 -> 2x cap = 200,000 -> reuses 180,224 slab.
    assert_eq!(best.alloc(&mut heap, 100_000), (s180, true));
    assert!(!best.alloc(&mut heap, 100_000).1);
    // Overflow & near-u32::MAX/2 saturation return (0, false) cleanly without panicking.
    assert_eq!(best.alloc(&mut heap, u32::MAX), (0, false));
    assert_eq!(best.alloc(&mut heap, 0x8000_0000), (0, false));
}

#[test]
fn symmetric_qi_and_amstream_multi_face_refcounting_preserve_lifetime_until_all_faces_release() {
    use lego_legacy_core::com::{self, ComState, E_NOINTERFACE, QUERY_INTERFACE, RELEASE, S_OK};
    use lego_legacy_core::guest::GuestMem;

    const IID_IUNKNOWN: [u8; 16] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x46,
    ];
    const IID_IAM_MULTI_MEDIA_STREAM: [u8; 16] = [
        0x5c, 0x59, 0xbe, 0xbe, 0x6f, 0x9a, 0xd0, 0x11, 0x8f, 0xde, 0x00, 0xc0, 0x4f, 0xd9, 0x18,
        0x9d,
    ];
    const IID_UNKNOWN_BOGUS: [u8; 16] = [0x11; 16];

    with_ctx(|ctx| {
        let iid_at = ctx.state.heap.alloc(16);
        let out_at = ctx.state.heap.alloc(8);

        // 1. AMMultiMediaStream + QI(IUnknown) keeps player alive until BOTH references reach 0.
        let player1 = win32::amstream::create(ctx).expect("create player1");
        assert!(ctx.state.players.contains_key(&player1));

        ctx.mem.write(iid_at, &IID_IUNKNOWN).unwrap();
        assert_eq!(
            (QUERY_INTERFACE.run)(ctx, player1, &[iid_at, out_at]).unwrap(),
            S_OK
        );
        let unk = ctx.mem.read_u32(out_at).unwrap();
        assert_eq!(unk, player1, "IUnknown returns object identity");

        // Self-QI returns the same pointer and bumps refcount to 3.
        ctx.mem.write(iid_at, &IID_IAM_MULTI_MEDIA_STREAM).unwrap();
        assert_eq!(
            (QUERY_INTERFACE.run)(ctx, player1, &[iid_at, out_at]).unwrap(),
            S_OK
        );
        assert_eq!(ctx.mem.read_u32(out_at).unwrap(), player1);

        // Unknown IID refuses with E_NOINTERFACE and zeros out pointer.
        ctx.mem.write(iid_at, &IID_UNKNOWN_BOGUS).unwrap();
        assert_eq!(
            (QUERY_INTERFACE.run)(ctx, player1, &[iid_at, out_at]).unwrap(),
            E_NOINTERFACE
        );
        assert_eq!(ctx.mem.read_u32(out_at).unwrap(), 0);

        assert_eq!((RELEASE.run)(ctx, player1, &[]).unwrap(), 2);
        assert!(ctx.state.players.contains_key(&player1));
        assert_eq!((RELEASE.run)(ctx, unk, &[]).unwrap(), 1);
        assert!(
            ctx.state.players.contains_key(&player1),
            "player must stay alive while second ref is live"
        );
        assert_eq!((RELEASE.run)(ctx, player1, &[]).unwrap(), 0);
        assert!(
            !ctx.state.players.contains_key(&player1),
            "player reclaimed once final ref drops to 0"
        );

        // 2. AMMultiMediaStream with a distinct secondary face sharing `identity`:
        // releasing primary face to 0 MUST NOT reclaim `players` until the secondary face
        // also reaches 0 (catches the missing `!has_live_refs(identity)` bug).
        let player2 = win32::amstream::create(ctx).expect("create player2");
        let alt_face =
            com::face(ctx, player2, &win32::amstream::MEDIA_STREAM).expect("secondary face");
        assert_ne!(alt_face, player2);
        assert_eq!(ctx.state.com.identity(alt_face), player2);

        // Drop primary face refcount to 0 while secondary face still holds refcount 1.
        assert_eq!((RELEASE.run)(ctx, player2, &[]).unwrap(), 0);
        assert!(
            ctx.state.players.contains_key(&player2),
            "player must stay alive while another face sharing its identity holds a ref"
        );
        // Symmetric IUnknown QI on secondary face returns primary identity and bumps primary refs.
        ctx.mem.write(iid_at, &IID_IUNKNOWN).unwrap();
        assert_eq!(
            (QUERY_INTERFACE.run)(ctx, alt_face, &[iid_at, out_at]).unwrap(),
            S_OK
        );
        assert_eq!(ctx.mem.read_u32(out_at).unwrap(), player2);
        assert_eq!((RELEASE.run)(ctx, alt_face, &[]).unwrap(), 0);
        assert!(
            ctx.state.players.contains_key(&player2),
            "player must stay alive while IUnknown ref on primary face is still live"
        );
        assert_eq!((RELEASE.run)(ctx, player2, &[]).unwrap(), 0);
        assert!(
            !ctx.state.players.contains_key(&player2),
            "player reclaimed once every face sharing its identity hits 0"
        );

        // 3. Generic ComTable objects created via com::create support AddRef/Release cleanly.
        let generic = com::create(ctx, &win32::ddraw::DIRECT_DRAW, ComState::None).unwrap();
        assert_eq!(ctx.state.com.add_ref(generic), 2);
        assert_eq!(com::release_ref(ctx, generic), 1);
        assert_eq!(com::release_ref(ctx, generic), 0);
    });
}

#[test]
fn filter_graph_reclaims_all_five_faces_and_reuses_slots_without_heap_growth() {
    use lego_legacy_core::com::{QUERY_INTERFACE, RELEASE, S_OK};
    use lego_legacy_core::guest::GuestMem;

    let iids: [(&str, [u8; 16]); 5] = [
        (
            "IGraphBuilder",
            [
                0xa9, 0x68, 0xa8, 0x56, 0xd4, 0x0a, 0xce, 0x11, 0xb0, 0x3a, 0x00, 0x20, 0xaf, 0x0b,
                0xa7, 0x70,
            ],
        ),
        (
            "IMediaControl",
            [
                0xb1, 0x68, 0xa8, 0x56, 0xd4, 0x0a, 0xce, 0x11, 0xb0, 0x3a, 0x00, 0x20, 0xaf, 0x0b,
                0xa7, 0x70,
            ],
        ),
        (
            "IMediaPosition",
            [
                0xb2, 0x68, 0xa8, 0x56, 0xd4, 0x0a, 0xce, 0x11, 0xb0, 0x3a, 0x00, 0x20, 0xaf, 0x0b,
                0xa7, 0x70,
            ],
        ),
        (
            "IVideoWindow",
            [
                0xb4, 0x68, 0xa8, 0x56, 0xd4, 0x0a, 0xce, 0x11, 0xb0, 0x3a, 0x00, 0x20, 0xaf, 0x0b,
                0xa7, 0x70,
            ],
        ),
        (
            "IMediaEvent",
            [
                0xb6, 0x68, 0xa8, 0x56, 0xd4, 0x0a, 0xce, 0x11, 0xb0, 0x3a, 0x00, 0x20, 0xaf, 0x0b,
                0xa7, 0x70,
            ],
        ),
    ];

    with_ctx(|ctx| {
        let iid_at = ctx.state.heap.alloc(16);
        let out_at = ctx.state.heap.alloc(8);
        let qi_all =
            |ctx: &mut win32::Ctx<'_>, from: u32, subset: &[(&str, [u8; 16])]| -> Vec<u32> {
                let mut out = Vec::with_capacity(subset.len());
                for &(name, iid) in subset {
                    ctx.mem.write(iid_at, &iid).unwrap();
                    assert_eq!(
                        (QUERY_INTERFACE.run)(ctx, from, &[iid_at, out_at]).unwrap(),
                        S_OK
                    );
                    let f = ctx.mem.read_u32(out_at).unwrap();
                    assert_eq!(ctx.state.com.interface_of(f), Some(name));
                    out.push(f);
                }
                out
            };

        // Warm up once so all 5 vtables are allocated, then measure steady-state heap extent.
        let warm = win32::dshow::create(ctx, &win32::dshow::GRAPH_BUILDER).expect("warm graph");
        let mut warm_faces = vec![warm];
        warm_faces.extend(qi_all(ctx, warm, &iids[1..]));
        for &face in &warm_faces {
            assert_eq!((RELEASE.run)(ctx, face, &[]).unwrap(), 0);
        }
        assert!(ctx.state.filter_graphs.is_empty());

        let base_extent = ctx.state.heap.extent();
        let base_primary = ctx.state.com.primary_objects().count();

        // Create graph via non-primary interface (IMediaControl) and QI all 5 faces symmetrically.
        let mc = win32::dshow::create(ctx, &win32::dshow::MEDIA_CONTROL).expect("create via MC");
        let identity = ctx.state.com.identity(mc);
        assert_ne!(mc, identity);
        assert_eq!(ctx.state.com.interface_of(mc), Some("IMediaControl"));
        assert_eq!(ctx.state.com.interface_of(identity), Some("IGraphBuilder"));

        let faces = qi_all(ctx, mc, &iids);
        for &f in &faces {
            assert_eq!(ctx.state.com.identity(f), identity);
        }
        // Release the extra IMediaControl reference from QI (create gave 1, QI gave +1).
        assert_eq!((RELEASE.run)(ctx, mc, &[]).unwrap(), 1);

        // Release first 4 faces: graph and all 5 faces must remain live until the 5th reaches 0.
        for &face in &faces[..4] {
            assert_eq!((RELEASE.run)(ctx, face, &[]).unwrap(), 0);
            assert!(ctx.state.filter_graphs.contains_key(&identity));
            assert!(ctx.state.com.has_live_refs(identity));
        }

        // Release the 5th and final face: graph AND all 5 COM faces must be reclaimed.
        assert_eq!((RELEASE.run)(ctx, faces[4], &[]).unwrap(), 0);
        assert!(ctx.state.filter_graphs.is_empty());
        assert_eq!(ctx.state.com.primary_objects().count(), base_primary);
        for &face in &faces {
            assert_eq!(
                ctx.state.com.interface_of(face),
                None,
                "reclaimed FilterGraph face {face:#x} must be removed from ComTable"
            );
            assert!(ctx.state.com.get(face).is_none());
        }

        // Creating another 5-face graph reuses the 5 reclaimed slots with zero heap growth.
        let g2 = win32::dshow::create(ctx, &win32::dshow::GRAPH_BUILDER).expect("second graph");
        let mut faces2 = vec![g2];
        faces2.extend(qi_all(ctx, g2, &iids[1..]));
        assert_eq!(
            ctx.state.heap.extent(),
            base_extent,
            "reclaimed 5 FilterGraph COM slots must be reused without growing guest heap"
        );
        for &face in &faces2 {
            assert_eq!((RELEASE.run)(ctx, face, &[]).unwrap(), 0);
        }
        assert_eq!(ctx.state.com.primary_objects().count(), base_primary);
    });
}

#[test]
fn com_release_ref_cascades_device3d_textures_and_backbuffer_in_dfs_order() {
    use lego_legacy_core::com::{self, ComState, RELEASE, S_OK};
    use lego_legacy_core::raster::PixelFormat;

    with_ctx(|ctx| {
        let make_surf = |ctx: &mut win32::Ctx<'_>, back: u32| -> (u32, u32) {
            let (pixels, _) = ctx.state.surface_pool.alloc(&mut ctx.state.heap, 1024);
            let obj = com::create(
                ctx,
                &win32::ddraw::DIRECT_DRAW_SURFACE,
                ComState::Surface {
                    pixels,
                    owned_pixels: true,
                    width: 16,
                    height: 32,
                    pitch: 32,
                    format: PixelFormat::RGB565,
                    primary: false,
                    caps: 0,
                    back,
                    palette: 0,
                },
            )
            .unwrap();
            (obj, pixels)
        };

        // Two bound textures (stages 0 & 1) plus a render target with an attached backbuffer.
        let (tex0, px_t0) = make_surf(ctx, 0);
        let (tex1, px_t1) = make_surf(ctx, 0);
        let (back, px_back) = make_surf(ctx, 0);
        ctx.state.com.get_mut(back).unwrap().refs = 0;
        let (rt, px_rt) = make_surf(ctx, back);

        let dev = com::create(
            ctx,
            &win32::d3d::DEVICE3,
            ComState::Device3D { surface: rt },
        )
        .unwrap();
        let set_texture = win32::d3d::DEVICE3
            .methods
            .iter()
            .find(|m| m.name == "SetTexture")
            .unwrap()
            .run;
        assert_eq!(set_texture(ctx, dev, &[0, tex0]).unwrap(), S_OK);
        assert_eq!(set_texture(ctx, dev, &[1, tex1]).unwrap(), S_OK);

        // Drop external refs: device keeps tex0, tex1, rt, and rt's backbuffer alive.
        assert_eq!((RELEASE.run)(ctx, tex0, &[]).unwrap(), 1);
        assert_eq!((RELEASE.run)(ctx, tex1, &[]).unwrap(), 1);
        assert_eq!((RELEASE.run)(ctx, rt, &[]).unwrap(), 0);
        assert!(ctx.state.com.get(tex0).is_some());
        assert!(ctx.state.com.get(tex1).is_some());
        assert!(ctx.state.com.get(rt).is_some());
        assert!(ctx.state.com.get(back).is_some());

        // Final Device3D Release cascades iteratively in exact DFS order:
        // tex0 -> tex1 -> rt -> back, pooling pixel slabs in that exact FIFO order.
        assert_eq!((RELEASE.run)(ctx, dev, &[]).unwrap(), 0);
        for surf in [tex0, tex1, rt, back] {
            assert!(
                ctx.state.com.get(surf).is_none(),
                "surface {surf:#x} reclaimed"
            );
        }
        assert_eq!(
            ctx.state.surface_pool.alloc(&mut ctx.state.heap, 1024),
            (px_t0, true)
        );
        assert_eq!(
            ctx.state.surface_pool.alloc(&mut ctx.state.heap, 1024),
            (px_t1, true)
        );
        assert_eq!(
            ctx.state.surface_pool.alloc(&mut ctx.state.heap, 1024),
            (px_rt, true)
        );
        assert_eq!(
            ctx.state.surface_pool.alloc(&mut ctx.state.heap, 1024),
            (px_back, true)
        );
    });
}

fn method_run(
    iface: &'static lego_legacy_core::com::Interface,
    name: &str,
) -> lego_legacy_core::com::ComFn {
    iface.methods.iter().find(|m| m.name == name).unwrap().run
}

#[test]
fn palette_churn_bounds_heap_and_zeros_reused_slab() {
    use lego_legacy_core::com::{self, ComState, RELEASE, S_OK};
    use lego_legacy_core::guest::GuestMem;

    with_ctx(|ctx| {
        let dd = com::create(ctx, &win32::ddraw::DIRECT_DRAW, ComState::None).unwrap();
        let create_palette = method_run(&win32::ddraw::DIRECT_DRAW, "CreatePalette");
        let get_entries = method_run(&win32::ddraw::DIRECT_DRAW_PALETTE, "GetEntries");

        let src_entries = ctx.state.heap.alloc(256 * 4);
        let dst_entries = ctx.state.heap.alloc(256 * 4);
        let out_at = ctx.state.heap.alloc(8);
        ctx.mem.write(src_entries, &[0x5au8; 256 * 4]).unwrap();

        // Warm up once so vtables and initial pool/COM slot exist.
        assert_eq!(
            create_palette(ctx, dd, &[0x0000_0004, src_entries, out_at, 0]).unwrap(),
            S_OK
        );
        let first = ctx.mem.read_u32(out_at).unwrap();
        let first_store = match ctx.state.com.get(first).map(|o| &o.state) {
            Some(ComState::Palette { store }) => *store,
            other => panic!("expected Palette state, got {other:?}"),
        };
        assert_eq!((RELEASE.run)(ctx, first, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(first).is_none(),
            "released unattached Palette must be reclaimed from ComTable"
        );

        let base_extent = ctx.state.heap.extent();
        let base_live = ctx.state.heap.live_bytes();

        // Creating a null-initialized palette must reuse the pooled 1 KiB store and zero it.
        assert_eq!(
            create_palette(ctx, dd, &[0x0000_0004, 0, out_at, 0]).unwrap(),
            S_OK
        );
        let second = ctx.mem.read_u32(out_at).unwrap();
        assert_eq!(second, first, "reclaimed Palette COM slot must be reused");
        let second_store = match ctx.state.com.get(second).map(|o| &o.state) {
            Some(ComState::Palette { store }) => *store,
            other => panic!("expected Palette state, got {other:?}"),
        };
        assert_eq!(
            second_store, first_store,
            "1 KiB palette slab must be reused"
        );
        assert_eq!(
            get_entries(ctx, second, &[0, 0, 256, dst_entries]).unwrap(),
            S_OK
        );
        assert!(
            ctx.mem
                .slice(dst_entries, 256 * 4)
                .unwrap()
                .iter()
                .all(|&b| b == 0),
            "reused null-initialized Palette store must be zeroed"
        );
        assert_eq!((RELEASE.run)(ctx, second, &[]).unwrap(), 0);

        for i in 0..1_200u32 {
            let entries = if i % 2 == 0 { src_entries } else { 0 };
            assert_eq!(
                create_palette(ctx, dd, &[0x0000_0004, entries, out_at, 0]).unwrap(),
                S_OK
            );
            let pal = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!((RELEASE.run)(ctx, pal, &[]).unwrap(), 0);
        }

        let extent_growth = ctx.state.heap.extent() - base_extent;
        let live_growth = ctx.state.heap.live_bytes().saturating_sub(base_live);
        assert_eq!(
            extent_growth, 0,
            "steady-state Palette churn must not grow heap"
        );
        assert!(
            live_growth <= 256 * 1024,
            "1,200 CreatePalette/Release cycles grew heap.live_bytes() by {live_growth} bytes"
        );

        // Oversized block from `heap.free` must return to `heap.free` (not orphan in `palette_pool`).
        let big = ctx.state.heap.alloc(4096);
        ctx.mem.write(big, &[0x77u8; 4096]).unwrap();
        assert!(ctx.state.heap.free(big));
        assert_eq!(
            create_palette(ctx, dd, &[0x0000_0004, 0, out_at, 0]).unwrap(),
            S_OK
        );
        let pal_in_pool = ctx.mem.read_u32(out_at).unwrap();
        assert_eq!(
            create_palette(ctx, dd, &[0x0000_0004, 0, out_at, 0]).unwrap(),
            S_OK
        );
        let pal_big = ctx.mem.read_u32(out_at).unwrap();
        let big_store = match ctx.state.com.get(pal_big).map(|o| &o.state) {
            Some(ComState::Palette { store }) => *store,
            other => panic!("expected Palette state, got {other:?}"),
        };
        assert_eq!(big_store, big);
        assert!(
            ctx.mem
                .slice(big_store, 256 * 4)
                .unwrap()
                .iter()
                .all(|&b| b == 0),
            "null-initialized Palette backed by recycled heap.free block must be zeroed"
        );
        assert_eq!((RELEASE.run)(ctx, pal_big, &[]).unwrap(), 0);
        assert_eq!(
            ctx.state.heap.alloc(4096),
            big,
            "oversized recycled heap block must return to heap.free rather than orphaning in palette_pool"
        );
        assert_eq!((RELEASE.run)(ctx, pal_in_pool, &[]).unwrap(), 0);
    });
}

#[test]
fn vertex_buffer_churn_bounds_heap_and_zeros_reused_slab() {
    use lego_legacy_core::com::{self, ComState, RELEASE, S_OK};
    use lego_legacy_core::guest::GuestMem;

    with_ctx(|ctx| {
        let d3d7 = com::create(ctx, &win32::d3d::D3D7, ComState::None).unwrap();
        let create_vb = method_run(&win32::d3d::D3D7, "CreateVertexBuffer");
        let lock_vb = method_run(&win32::d3d::VERTEX_BUFFER7, "Lock");

        // D3DFVF_XYZ | D3DFVF_DIFFUSE = 0x42 (16 bytes/vertex).
        let fvf = 0x42u32;
        let desc_at = ctx.state.heap.alloc(16);
        let out_at = ctx.state.heap.alloc(8);
        let ptr_at = ctx.state.heap.alloc(8);
        let size_at = ctx.state.heap.alloc(8);

        let write_desc = |ctx: &mut win32::Ctx<'_>, count: u32| {
            ctx.mem.write_u32(desc_at, 16).unwrap();
            ctx.mem.write_u32(desc_at + 4, 0).unwrap();
            ctx.mem.write_u32(desc_at + 8, fvf).unwrap();
            ctx.mem.write_u32(desc_at + 12, count).unwrap();
        };

        // Warm up with a 128-vertex (2048-byte) VB and dirty its memory.
        write_desc(ctx, 128);
        assert_eq!(create_vb(ctx, d3d7, &[desc_at, out_at, 0]).unwrap(), S_OK);
        let vb1 = ctx.mem.read_u32(out_at).unwrap();
        assert_eq!(lock_vb(ctx, vb1, &[0, ptr_at, size_at]).unwrap(), S_OK);
        let store1 = ctx.mem.read_u32(ptr_at).unwrap();
        let size1 = ctx.mem.read_u32(size_at).unwrap();
        assert_eq!(size1, 128 * 16);
        ctx.mem
            .write(store1, &vec![0xa5u8; size1 as usize])
            .unwrap();
        assert_eq!((RELEASE.run)(ctx, vb1, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(vb1).is_none(),
            "released VertexBuffer must be reclaimed from ComTable"
        );

        let base_extent = ctx.state.heap.extent();
        let base_live = ctx.state.heap.live_bytes();

        // A 96-vertex (1536-byte) request is within 2x of the pooled 2048-byte slab and must reuse + zero it.
        write_desc(ctx, 96);
        assert_eq!(create_vb(ctx, d3d7, &[desc_at, out_at, 0]).unwrap(), S_OK);
        let vb2 = ctx.mem.read_u32(out_at).unwrap();
        assert_eq!(vb2, vb1, "reclaimed VertexBuffer COM slot must be reused");
        assert_eq!(lock_vb(ctx, vb2, &[0, ptr_at, size_at]).unwrap(), S_OK);
        let store2 = ctx.mem.read_u32(ptr_at).unwrap();
        assert_eq!(
            store2, store1,
            "best-fit 2x vb_pool must reuse 2048-byte slab for 1536-byte VB"
        );
        assert!(
            ctx.mem
                .slice(store2, 96 * 16)
                .unwrap()
                .iter()
                .all(|&b| b == 0),
            "reused VertexBuffer store must be zeroed"
        );
        assert_eq!((RELEASE.run)(ctx, vb2, &[]).unwrap(), 0);

        for i in 0..1_200u32 {
            let count = if i % 2 == 0 { 128 } else { 96 };
            write_desc(ctx, count);
            assert_eq!(create_vb(ctx, d3d7, &[desc_at, out_at, 0]).unwrap(), S_OK);
            let vb = ctx.mem.read_u32(out_at).unwrap();
            assert_eq!((RELEASE.run)(ctx, vb, &[]).unwrap(), 0);
        }

        let extent_growth = ctx.state.heap.extent() - base_extent;
        let live_growth = ctx.state.heap.live_bytes().saturating_sub(base_live);
        assert_eq!(
            extent_growth, 0,
            "steady-state VertexBuffer churn must not grow heap"
        );
        assert!(
            live_growth <= 256 * 1024,
            "1,200 CreateVertexBuffer/Release cycles grew heap.live_bytes() by {live_growth} bytes"
        );
    });
}

#[test]
fn palette_live_surface_holder_guard_defers_reclaim_until_surface_release_or_detach() {
    use lego_legacy_core::com::{self, ComState, RELEASE, S_OK};
    use lego_legacy_core::guest::GuestMem;
    use lego_legacy_core::raster::PixelFormat;

    with_ctx(|ctx| {
        let dd = com::create(ctx, &win32::ddraw::DIRECT_DRAW, ComState::None).unwrap();
        let create_palette = method_run(&win32::ddraw::DIRECT_DRAW, "CreatePalette");
        let set_palette = method_run(&win32::ddraw::DIRECT_DRAW_SURFACE, "SetPalette");
        let out_at = ctx.state.heap.alloc(8);

        let make_surf = |ctx: &mut win32::Ctx<'_>, primary: bool| -> u32 {
            let (pixels, _) = ctx.state.surface_pool.alloc(&mut ctx.state.heap, 256);
            com::create(
                ctx,
                &win32::ddraw::DIRECT_DRAW_SURFACE,
                ComState::Surface {
                    pixels,
                    owned_pixels: true,
                    width: 16,
                    height: 16,
                    pitch: 16,
                    format: PixelFormat {
                        bits: 8,
                        masks: [0; 4],
                    },
                    primary,
                    caps: 0,
                    back: 0,
                    palette: 0,
                },
            )
            .unwrap()
        };
        let make_pal = |ctx: &mut win32::Ctx<'_>| -> (u32, u32) {
            assert_eq!(
                create_palette(ctx, dd, &[0x0000_0004, 0, out_at, 0]).unwrap(),
                S_OK
            );
            let pal = ctx.mem.read_u32(out_at).unwrap();
            let store = match ctx.state.com.get(pal).map(|o| &o.state) {
                Some(ComState::Palette { store }) => *store,
                other => panic!("expected Palette, got {other:?}"),
            };
            (pal, store)
        };

        // 1. Offscreen surface holds palette at refcount 0 until surface releases.
        let s1 = make_surf(ctx, false);
        let (p1, store1) = make_pal(ctx);
        assert_eq!(set_palette(ctx, s1, &[p1]).unwrap(), S_OK);
        assert_eq!((RELEASE.run)(ctx, p1, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(p1).is_some(),
            "palette bound to live surface must stay alive at refcount 0"
        );
        assert_eq!((RELEASE.run)(ctx, s1, &[]).unwrap(), 0);
        assert!(ctx.state.com.get(s1).is_none());
        assert!(
            ctx.state.com.get(p1).is_none(),
            "palette must reclaim once holding surface releases"
        );
        // Reclaimed 1 KiB store comes back on next CreatePalette.
        let (p1_reused, store1_reused) = make_pal(ctx);
        assert_eq!(store1_reused, store1);
        assert_eq!((RELEASE.run)(ctx, p1_reused, &[]).unwrap(), 0);

        // 2. Primary surface detaching via SetPalette(0) clears state.palette and reclaims palette.
        let primary = make_surf(ctx, true);
        let (p2, store2) = make_pal(ctx);
        assert_eq!(set_palette(ctx, primary, &[p2]).unwrap(), S_OK);
        assert_eq!(ctx.state.palette, store2);
        assert_eq!((RELEASE.run)(ctx, p2, &[]).unwrap(), 0);
        assert!(ctx.state.com.get(p2).is_some());
        assert_eq!(set_palette(ctx, primary, &[0]).unwrap(), S_OK);
        assert_eq!(ctx.state.palette, 0);
        assert!(
            ctx.state.com.get(p2).is_none(),
            "palette must reclaim once detached via SetPalette(0)"
        );

        // 3. Replacing a primary surface's palette reclaims the old unreferenced palette immediately.
        let (p3, store3) = make_pal(ctx);
        let (p4, store4) = make_pal(ctx);
        assert_eq!(store3, store2, "detached p2 slab reused for p3");
        assert_eq!(set_palette(ctx, primary, &[p3]).unwrap(), S_OK);
        assert_eq!(ctx.state.palette, store3);
        assert_eq!((RELEASE.run)(ctx, p3, &[]).unwrap(), 0);
        assert!(ctx.state.com.get(p3).is_some());
        assert_eq!(set_palette(ctx, primary, &[p4]).unwrap(), S_OK);
        assert_eq!(ctx.state.palette, store4);
        assert!(
            ctx.state.com.get(p3).is_none(),
            "old palette must reclaim when replaced on surface"
        );

        // 4. Shared across primary + offscreen: releasing primary clears state.palette but keeps
        //    palette alive until offscreen releases too.
        let off = make_surf(ctx, false);
        assert_eq!(set_palette(ctx, off, &[p4]).unwrap(), S_OK);
        assert_eq!((RELEASE.run)(ctx, p4, &[]).unwrap(), 0);
        assert_eq!((RELEASE.run)(ctx, primary, &[]).unwrap(), 0);
        assert_eq!(ctx.state.palette, 0);
        assert!(
            ctx.state.com.get(p4).is_some(),
            "palette must stay alive while offscreen surface still holds it"
        );
        assert_eq!((RELEASE.run)(ctx, off, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(p4).is_none(),
            "palette must reclaim once last surface holder releases"
        );
    });
}

#[test]
fn device3d_retained_device_stream_sample_dinput_and_d3drm_arrays_reclaim_com_slots() {
    use lego_legacy_core::com::{self, ComState, RELEASE, S_OK};

    with_ctx(|ctx| {
        // Warm up vtables first.
        let set_rs = method_run(&win32::d3d::DEVICE3, "SetRenderState");
        let frame = com::create(ctx, &win32::d3drm::FRAME, ComState::None).unwrap();

        let dev3d_warm =
            com::create(ctx, &win32::d3d::DEVICE3, ComState::Device3D { surface: 0 }).unwrap();
        assert_eq!(set_rs(ctx, dev3d_warm, &[22, 1]).unwrap(), S_OK);
        assert_eq!(ctx.state.immediate.device_summaries().len(), 1);
        assert_eq!((RELEASE.run)(ctx, dev3d_warm, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(dev3d_warm).is_none(),
            "Device3D COM face must reclaim"
        );
        assert!(
            ctx.state.immediate.device_summaries().is_empty(),
            "Device3D entry in immediate.devices must be removed on Release"
        );

        // RetainedDevice clears `names` and `set_values` and defers reclaim while a live Viewport holds it.
        let inner_dev =
            com::create(ctx, &win32::d3d::DEVICE2, ComState::Device3D { surface: 0 }).unwrap();
        let rm_dev = com::create(
            ctx,
            &win32::d3drm::DEVICE3,
            ComState::RetainedDevice {
                surface: 0,
                buffers: 1,
                immediate: inner_dev,
            },
        )
        .unwrap();
        ctx.state.names.insert(rm_dev, "rm-dev".to_owned());
        ctx.state.set_values.insert((rm_dev, "quality"), 0x18);
        let vp = com::create(
            ctx,
            &win32::d3drm::VIEWPORT2,
            ComState::Viewport {
                device: rm_dev,
                camera: 0,
                x: 0,
                y: 0,
                width: 64,
                height: 64,
                background: 0,
                immediate: 0,
            },
        )
        .unwrap();
        assert_eq!((RELEASE.run)(ctx, rm_dev, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(rm_dev).is_some(),
            "RetainedDevice held by live Viewport must stay alive at refcount 0"
        );
        assert_eq!((RELEASE.run)(ctx, vp, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(rm_dev).is_none(),
            "RetainedDevice COM face must reclaim once Viewport releases"
        );
        assert!(
            ctx.state.com.get(inner_dev).is_none(),
            "RetainedDevice Release must release its owned immediate Device3D"
        );
        assert!(
            !ctx.state.names.contains_key(&rm_dev),
            "RetainedDevice name must be cleared"
        );
        assert!(
            !ctx.state.set_values.contains_key(&(rm_dev, "quality")),
            "RetainedDevice set_values must be cleared"
        );

        let sample = com::create(
            ctx,
            &win32::amstream::DIRECT_DRAW_STREAM_SAMPLE,
            ComState::StreamSample {
                owner: 0,
                surface: 0,
            },
        )
        .unwrap();
        assert_eq!((RELEASE.run)(ctx, sample, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(sample).is_none(),
            "StreamSample COM face must reclaim"
        );

        let di_dev = com::create(
            ctx,
            &win32::dinput::DIRECT_INPUT_DEVICE,
            ComState::Device {
                keyboard: false,
                exclusive: true,
                acquired: true,
            },
        )
        .unwrap();
        assert_eq!((RELEASE.run)(ctx, di_dev, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(di_dev).is_none(),
            "DirectInput Device COM face must reclaim"
        );

        // Array and PickedArray (including PickedArray's owned FRAME_ARRAY).
        let frame_face = com::face(ctx, frame, &win32::d3drm::FRAME).unwrap();
        let arr = com::create(
            ctx,
            &win32::d3drm::FRAME_ARRAY,
            ComState::Array {
                items: vec![frame_face],
            },
        )
        .unwrap();
        let picked = com::create(
            ctx,
            &win32::d3drm::PICKED_ARRAY,
            ComState::PickedArray {
                picks: vec![(frame, arr, [0; 5])],
            },
        )
        .unwrap();
        assert_eq!((RELEASE.run)(ctx, picked, &[]).unwrap(), 0);
        assert!(
            ctx.state.com.get(picked).is_none(),
            "PickedArray COM face must reclaim"
        );
        assert!(
            ctx.state.com.get(arr).is_none(),
            "PickedArray Release must release and reclaim its owned FrameArray"
        );
        // Original frame keeps its creation refcount (1) after Array balanced the com::face ref.
        assert_eq!(ctx.state.com.get(frame).map(|o| o.refs), Some(1));

        // Steady-state churn across all of these COM types must reuse reclaimed slots with 0 heap growth.
        let base_extent = ctx.state.heap.extent();
        for _ in 0..200 {
            let d =
                com::create(ctx, &win32::d3d::DEVICE3, ComState::Device3D { surface: 0 }).unwrap();
            assert_eq!((RELEASE.run)(ctx, d, &[]).unwrap(), 0);
            let r = com::create(
                ctx,
                &win32::d3drm::DEVICE3,
                ComState::RetainedDevice {
                    surface: 0,
                    buffers: 1,
                    immediate: 0,
                },
            )
            .unwrap();
            assert_eq!((RELEASE.run)(ctx, r, &[]).unwrap(), 0);
            let s = com::create(
                ctx,
                &win32::amstream::DIRECT_DRAW_STREAM_SAMPLE,
                ComState::StreamSample {
                    owner: 0,
                    surface: 0,
                },
            )
            .unwrap();
            assert_eq!((RELEASE.run)(ctx, s, &[]).unwrap(), 0);
            let di = com::create(
                ctx,
                &win32::dinput::DIRECT_INPUT_DEVICE,
                ComState::Device {
                    keyboard: true,
                    exclusive: false,
                    acquired: false,
                },
            )
            .unwrap();
            assert_eq!((RELEASE.run)(ctx, di, &[]).unwrap(), 0);
            let a = com::create(
                ctx,
                &win32::d3drm::FRAME_ARRAY,
                ComState::Array { items: Vec::new() },
            )
            .unwrap();
            let p = com::create(
                ctx,
                &win32::d3drm::PICKED_ARRAY,
                ComState::PickedArray {
                    picks: vec![(frame, a, [0; 5])],
                },
            )
            .unwrap();
            assert_eq!((RELEASE.run)(ctx, p, &[]).unwrap(), 0);
        }
        assert_eq!(
            ctx.state.heap.extent(),
            base_extent,
            "reclaimed COM slots must be reused without growing guest heap"
        );
    });
}
