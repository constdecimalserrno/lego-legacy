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
