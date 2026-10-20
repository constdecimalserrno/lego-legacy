//! What KERNEL32, USER32 and retained mode answer, called as a title calls
//! them but without one: each shim or method run on the stub's machine with
//! the arguments a title would push, and what it wrote and answered read back
//! (#114, #117, #118, #119, #120).

use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::Stop;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::raster::PixelFormat;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::win32::{self, user32::WINDOW, Ctx};
use lego_legacy_core::Title;

/// Where the tests put what they pass and what is written back.
const DATA: u32 = 0x0050_0000;
const OUT: u32 = 0x0050_1000;

fn machine() -> Machine {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    Machine::new(&Title::STUB, &mut Headless::new()).expect("a machine")
}

/// Call `dll!name` with `args`, as the boundary would.
fn call(machine: &mut Machine, dll: &str, name: &str, args: &[u32]) -> Result<u32, Stop> {
    let shim = win32::find(dll, name).unwrap_or_else(|| panic!("{dll}!{name} is written"));
    with_ctx(machine, |ctx| (shim.run)(ctx, args))
}

/// Call the method in `slot` of `interface` on `this`, as a guest calling
/// through the object's table would.
fn com(
    machine: &mut Machine,
    interface: &str,
    slot: usize,
    this: u32,
    args: &[u32],
) -> Result<u32, Stop> {
    let method = win32::com_method(interface, slot)
        .unwrap_or_else(|| panic!("{interface} has a slot {slot}"));
    assert_eq!(
        method.args as usize,
        args.len(),
        "{interface}::{} at slot {slot}",
        method.name
    );
    with_ctx(machine, |ctx| (method.run)(ctx, this, args))
}

fn with_ctx(
    machine: &mut Machine,
    run: impl FnOnce(&mut Ctx<'_>) -> Result<u32, Stop>,
) -> Result<u32, Stop> {
    let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
    let mut ctx = Ctx {
        cpu: &mut machine.cpu,
        mem: &mut machine.arena,
        state: &mut machine.state,
        title: &Title::STUB,
        log: &mut log,
        tree: &mut machine.tree,
        modules: &mut machine.modules,
        clock_millis: 0,
        boundary: &mut machine.boundary,
        call_guest: None,
        float_answer: None,
        answer_is_the_call_s: false,
    };
    run(&mut ctx)
}

fn text(machine: &Machine, at: u32) -> String {
    let bytes = machine.arena.slice(at, 64).expect("in the arena");
    let end = bytes.iter().position(|b| *b == 0).expect("terminated");
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[test]
fn a_setting_win_ini_does_not_have_is_the_callers_default_cut_to_fit() {
    let mut m = machine();
    m.arena.write(DATA, b"LEGO Island\0").expect("in the arena");
    m.arena
        .write(DATA + 16, b"MediaPath\0")
        .expect("in the arena");
    m.arena
        .write(DATA + 32, b"D:\\LEGO  \0")
        .expect("in the arena");
    let (section, key, default) = (DATA, DATA + 16, DATA + 32);

    // Trailing blanks are not part of the answer.
    let n = call(
        &mut m,
        "KERNEL32",
        "GetProfileStringA",
        &[section, key, default, OUT, 0x100],
    );
    assert_eq!(n, Ok(7));
    assert_eq!(text(&m, OUT), "D:\\LEGO");

    // Too little room: as much as fits, terminated, and that much answered.
    let n = call(
        &mut m,
        "KERNEL32",
        "GetProfileStringA",
        &[section, key, default, OUT, 4],
    );
    assert_eq!(n, Ok(3));
    assert_eq!(text(&m, OUT), "D:\\");

    // A null default is an empty string, which is what LEGO Island is given.
    let n = call(
        &mut m,
        "KERNEL32",
        "GetProfileStringA",
        &[section, key, 0, OUT, 0x100],
    );
    assert_eq!(n, Ok(0));
    assert_eq!(text(&m, OUT), "");

    // A null key asks for the section's names, which is not written.
    assert!(matches!(
        call(
            &mut m,
            "KERNEL32",
            "GetProfileStringA",
            &[section, 0, default, OUT, 0x100]
        ),
        Err(Stop::NotWritten { .. })
    ));
}

#[test]
fn a_window_style_and_instance_are_the_ones_it_was_made_with() {
    const GWL_STYLE: u32 = -16i32 as u32;
    const GWL_HINSTANCE: u32 = -6i32 as u32;
    const GWL_USERDATA: u32 = -21i32 as u32;
    const WS_CAPTION_SYSMENU: u32 = 0x00c8_0000;
    let mut m = machine();

    // No window yet, so nothing to ask about.
    assert!(call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_STYLE]).is_err());

    m.arena.write(DATA, b"LEGO\0").expect("in the arena");
    let made = call(
        &mut m,
        "USER32",
        "CreateWindowExA",
        &[
            0x0004_0000, // WS_EX_APPWINDOW
            DATA,
            DATA,
            WS_CAPTION_SYSMENU,
            0,
            0,
            641,
            481,
            0,
            0,
            0x0040_0000,
            0,
        ],
    );
    assert_eq!(made, Ok(WINDOW));
    assert_eq!(
        call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_STYLE]),
        Ok(WS_CAPTION_SYSMENU)
    );
    assert_eq!(
        call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_HINSTANCE]),
        Ok(0x0040_0000)
    );
    // Any other word is not answered.
    assert!(call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_USERDATA]).is_err());
}

#[test]
fn mouse_trails_go_off_and_nothing_else_is_answered() {
    const SPI_SETMOUSETRAILS: u32 = 0x005d;
    let mut m = machine();
    for off in [0, 1] {
        assert_eq!(
            call(
                &mut m,
                "USER32",
                "SystemParametersInfoA",
                &[SPI_SETMOUSETRAILS, off, 0, 0]
            ),
            Ok(1)
        );
    }
    // A trail of three is one this machine would have to draw.
    assert!(call(
        &mut m,
        "USER32",
        "SystemParametersInfoA",
        &[SPI_SETMOUSETRAILS, 3, 0, 0]
    )
    .is_err());
    // SPI_GETSCREENSAVEACTIVE, which no title has asked.
    assert!(call(
        &mut m,
        "USER32",
        "SystemParametersInfoA",
        &[0x0010, 0, OUT, 0]
    )
    .is_err());
}

/// SmartHeap, which LEGO Island's engine links in, finds the header of any
/// block it handed out by masking its address with `0xffff_0000`. That only
/// finds it if every region VirtualAlloc gives it starts on Windows' 64 KiB
/// allocation granularity (#114, #356 in the old tree).
#[test]
fn a_new_region_starts_on_the_allocation_granularity() {
    const MEM_RESERVE: u32 = 0x2000;
    const PAGE_NOACCESS: u32 = 0x01;
    let mut m = machine();
    // An odd size first, so the next region would start off the boundary if
    // nothing put it back on.
    for size in [0x1_2345, 0x17_0000, 0x5_0000] {
        let at = call(
            &mut m,
            "KERNEL32",
            "VirtualAlloc",
            &[0, size, MEM_RESERVE, PAGE_NOACCESS],
        )
        .expect("answered");
        assert_ne!(at, 0, "{size:#x} bytes are reserved");
        assert_eq!(at % 0x1_0000, 0, "{at:#x} is not on a 64 KiB boundary");
    }
}

/// A region says how much of it, from an address, is committed and how, and
/// how much only reserved, as SmartHeap in LEGO Island's engine asks (#118).
#[test]
fn a_region_says_how_much_of_it_is_committed() {
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_DECOMMIT: u32 = 0x4000;
    const MEM_RELEASE: u32 = 0x8000;
    const MEM_PRIVATE: u32 = 0x2_0000;
    const PAGE_NOACCESS: u32 = 0x01;
    const PAGE_READWRITE: u32 = 0x04;
    let mut m = machine();
    let base = call(
        &mut m,
        "KERNEL32",
        "VirtualAlloc",
        &[0, 0x1_0000, MEM_RESERVE, PAGE_NOACCESS],
    )
    .expect("answered");
    let commit = [base, 0x5000, MEM_COMMIT, PAGE_READWRITE];
    assert_eq!(call(&mut m, "KERNEL32", "VirtualAlloc", &commit), Ok(base));
    let mut query = |at: u32| {
        assert_eq!(
            call(&mut m, "KERNEL32", "VirtualQuery", &[at, OUT, 28]),
            Ok(28)
        );
        [0, 4, 8, 12, 16, 20, 24].map(|o| m.arena.read_u32(OUT + o).expect("in the arena"))
    };
    // Base, allocation base, its protection, size, state, protection, type.
    assert_eq!(
        query(base),
        [
            base,
            base,
            PAGE_NOACCESS,
            0x5000,
            MEM_COMMIT,
            PAGE_READWRITE,
            MEM_PRIVATE
        ]
    );
    // From inside a page, the run starts on that page.
    assert_eq!(
        query(base + 0x1234)[..5],
        [base + 0x1000, base, 1, 0x4000, MEM_COMMIT]
    );
    // What is only reserved runs to the region's end, with no protection.
    assert_eq!(query(base + 0x5000)[3..6], [0xb000, MEM_RESERVE, 0]);

    // A page given back ends the committed run before it.
    let decommit = [base + 0x4000, 0x1000, MEM_DECOMMIT];
    assert_eq!(call(&mut m, "KERNEL32", "VirtualFree", &decommit), Ok(1));
    assert_eq!(
        call(&mut m, "KERNEL32", "VirtualQuery", &[base, OUT, 28]),
        Ok(28)
    );
    assert_eq!(m.arena.read_u32(OUT + 12), Ok(0x4000));

    // A region let go of is no region at all.
    let release = [base, 0, MEM_RELEASE];
    assert_eq!(call(&mut m, "KERNEL32", "VirtualFree", &release), Ok(1));
    assert!(call(&mut m, "KERNEL32", "VirtualQuery", &[base, OUT, 28]).is_err());
}

/// A module with no table of keys among its resources has none to load, as
/// LEGO Island's own image has none (#118); one that does stops the run.
#[test]
fn a_table_of_keys_is_looked_for_in_the_modules_resources() {
    const RT_ICON: u32 = 3;
    const RT_ACCELERATOR: u32 = 9;
    let mut m = machine();
    // Headers at 0x40, the resource directory's address at the third of the
    // optional header's data directories, and one resource type in it.
    let module = DATA;
    m.arena
        .write_u32(module + 0x3c, 0x40)
        .expect("in the arena");
    m.arena
        .write_u32(module + 0x40 + 24 + 96 + 16, 0x200)
        .expect("in the arena");
    m.arena
        .write_u32(module + 0x200 + 12, 1 << 16)
        .expect("in the arena");
    m.arena.write(OUT, b"AppAccel\0").expect("in the arena");

    m.arena
        .write_u32(module + 0x200 + 16, RT_ICON)
        .expect("in the arena");
    assert_eq!(
        call(&mut m, "USER32", "LoadAcceleratorsA", &[module, OUT]),
        Ok(0)
    );
    // ERROR_RESOURCE_TYPE_NOT_FOUND.
    assert_eq!(m.state.last_error, 1813);

    m.arena
        .write_u32(module + 0x200 + 16, RT_ACCELERATOR)
        .expect("in the arena");
    assert!(call(&mut m, "USER32", "LoadAcceleratorsA", &[module, OUT]).is_err());
}

/// DirectX 5's retained mode makes DirectX 5's texture and material (#118):
/// the texture is DirectX 6's as far as `GenerateMIPMap`, with a `Changed`
/// that takes two flags, and the material is DirectX 6's without its ambient
/// colour.
#[test]
fn directx_5_makes_its_own_textures_and_materials() {
    let mut m = machine();
    // IDirect3DRM2::CreateTextureFromSurface(surface, out), at 18.
    assert_eq!(com(&mut m, "IDirect3DRM2", 18, 0, &[0, OUT]), Ok(0));
    let texture = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(m.state.interface_of(texture), Some("IDirect3DRMTexture2"));
    let changed = win32::com_method("IDirect3DRMTexture2", 14).expect("declared");
    assert_eq!((changed.name, changed.args), ("Changed", 2));
    assert_eq!(
        win32::com_method("IDirect3DRMTexture2", 32).map(|m| m.name),
        Some("GenerateMIPMap")
    );
    assert!(win32::com_method("IDirect3DRMTexture2", 33).is_none());

    // IDirect3DRM2::CreateMaterial(power, out), at 13.
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 13, 0, &[10.0f32.to_bits(), OUT]),
        Ok(0)
    );
    let material = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(m.state.interface_of(material), Some("IDirect3DRMMaterial"));
    assert_eq!(
        win32::com_method("IDirect3DRMMaterial", 16).map(|m| m.name),
        Some("GetEmissive")
    );
    assert!(win32::com_method("IDirect3DRMMaterial", 17).is_none());
}

/// Three floats where `at` is.
fn vector(machine: &Machine, at: u32) -> [f32; 3] {
    [0, 4, 8].map(|o| f32::from_bits(machine.arena.read_u32(at + o).expect("in the arena")))
}

/// DirectX 5's frame is not DirectX 6's with a slot taken out (#117). From
/// `GetTextureTopology` at 33 up its slots sit one place past DirectX 6's,
/// from `SetTextureTopology` at 65 two places, and from `AddMoveCallback2`
/// at 69 three: a frame made through DirectX 5's retained mode is placed and
/// read back through the slots its own header gives those methods.
#[test]
fn a_directx_5_frame_answers_at_its_own_slots() {
    let mut m = machine();
    let frame3 = win32::interface_named("IDirect3DRMFrame3").expect("declared");
    for gone in [
        "GetTextureTopology",
        "SetTextureTopology",
        "AddMoveCallback2",
    ] {
        assert!(
            frame3.methods.iter().all(|method| method.name != gone),
            "DirectX 6's frame has no {gone}"
        );
    }

    // IDirect3DRM2::CreateFrame(no parent, out).
    assert_eq!(com(&mut m, "IDirect3DRM2", 4, 0, &[0, OUT]), Ok(0));
    let frame = m.arena.read_u32(OUT).expect("in the arena");

    // Slot 33 is GetTextureTopology, which is not written, and not
    // InverseTransform, which is DirectX 6's slot 33.
    assert_eq!(
        win32::com_method("IDirect3DRMFrame2", 33).map(|m| m.name),
        Some("GetTextureTopology")
    );
    assert!(com(&mut m, "IDirect3DRMFrame2", 33, frame, &[0, 0]).is_err());

    // SetPosition at 61 (DirectX 6's 60), read back by GetPosition at 24,
    // where both agree.
    let [x, y, z] = [1.0f32, 2.0, 3.0].map(f32::to_bits);
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 61, frame, &[0, x, y, z]),
        Ok(0)
    );
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 24, frame, &[0, OUT]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [1.0, 2.0, 3.0]);

    // Transform at 68 (DirectX 6's 66): the frame's origin is where it was
    // put.
    m.arena.write(DATA, &[0; 12]).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 68, frame, &[OUT, DATA]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [1.0, 2.0, 3.0]);

    // And InverseTransform at 34 (DirectX 6's 33) takes it back.
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 34, frame, &[OUT + 16, OUT]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT + 16), [0.0, 0.0, 0.0]);
}

/// D3DRMCreateColorRGBA, which LEGO Island's engine imports (#117): alpha in
/// the top byte, then red, green and blue, each fraction truncated to a byte
/// and held between none and all.
#[test]
fn a_colour_from_four_fractions_puts_alpha_on_top() {
    let mut m = machine();
    let mut colour = |rgba: [f32; 4]| {
        call(
            &mut m,
            "d3drm",
            "D3DRMCreateColorRGBA",
            &rgba.map(f32::to_bits),
        )
    };
    assert_eq!(colour([1.0, 0.5, 0.0, 0.25]), Ok(0x3fff_7f00));
    assert_eq!(colour([-1.0, 2.0, 0.999, 1.0]), Ok(0xff00_fffe));
}

/// A mesh LEGO Island builds its world from (#119), made through DirectX 5's
/// retained mode, with `groups` groups of four vertices and two triangles.
fn mesh(m: &mut Machine, groups: u32) -> u32 {
    // IDirect3DRM2::CreateMesh(out), at 5.
    assert_eq!(com(m, "IDirect3DRM2", 5, 0, &[OUT]), Ok(0));
    let mesh = m.arena.read_u32(OUT).expect("in the arena");
    for (at, index) in [0, 1, 2, 2, 1, 3].into_iter().enumerate() {
        m.arena
            .write_u32(DATA + 4 * at as u32, index)
            .expect("in the arena");
    }
    for id in 0..groups {
        // AddGroup(vertices, faces, vertices a face, face data, out), at 14.
        assert_eq!(
            com(m, "IDirect3DRMMesh", 14, mesh, &[4, 2, 3, DATA, OUT]),
            Ok(0)
        );
        assert_eq!(m.arena.read_u32(OUT), Ok(id), "groups are numbered in turn");
    }
    mesh
}

/// Four `D3DRMVERTEX`es at `at`, each placed at `corner` times its number
/// and with its number in every other field.
fn vertices(m: &mut Machine, at: u32, corner: [f32; 3]) -> Vec<[u32; 9]> {
    (0..4u32)
        .map(|n| {
            let mut v = [n; 9];
            for i in 0..3 {
                v[i] = (corner[i] * n as f32).to_bits();
            }
            for (i, word) in v.iter().enumerate() {
                m.arena
                    .write_u32(at + 36 * n + 4 * i as u32, *word)
                    .expect("in the arena");
            }
            v
        })
        .collect()
}

/// What retained mode keeps of a mesh (#119): each group's faces and
/// vertices copied as they were given, so a title that reuses its scratch
/// for the next group changes nothing, and what each group is drawn with.
#[test]
fn a_mesh_keeps_its_groups_as_they_were_given() {
    let mut m = machine();
    let mesh = mesh(&mut m, 2);
    let given = vertices(&mut m, DATA + 0x100, [1.0, 2.0, 3.0]);
    // SetVertices(group, first, count, vertices), at 15: all four, then the
    // scratch is written over.
    let set = |m: &mut Machine, args: [u32; 4]| com(m, "IDirect3DRMMesh", 15, mesh, &args);
    assert_eq!(set(&mut m, [0, 0, 4, DATA + 0x100]), Ok(0));
    vertices(&mut m, DATA + 0x100, [9.0, 9.0, 9.0]);
    m.arena.write_u32(DATA, 7).expect("in the arena");
    let group = |m: &Machine, id: usize| m.state.scene.meshes[&mesh].groups[id].clone();
    let kept = group(&m, 0);
    assert_eq!(
        kept.vertices,
        given.into_iter().map(Some).collect::<Vec<_>>()
    );
    assert_eq!((kept.per_face, kept.indices), (3, vec![0, 1, 2, 2, 1, 3]));
    // Only the vertices it names are set. Past the group's end, or a group
    // the mesh does not have, is a bad value.
    assert_eq!(set(&mut m, [1, 3, 1, DATA + 0x100]), Ok(0));
    assert_eq!(group(&m, 1).vertices.iter().flatten().count(), 1);
    assert_eq!(group(&m, 1).vertices[3].map(|v| v[3]), Some(0));
    assert_eq!(set(&mut m, [1, 3, 2, DATA + 0x100]), Ok(0x8876_0316));
    assert_eq!(set(&mut m, [2, 0, 1, DATA + 0x100]), Ok(0x8876_0316));

    // SetGroupColor and SetGroupColorRGB, at 16 and 17, set one colour;
    // SetGroupMapping and SetGroupQuality, at 18 and 19, are kept as given.
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 16, mesh, &[0, 0x80ff_0000]),
        Ok(0)
    );
    let [r, g, b] = [0.0, 1.0, 0.0f32].map(f32::to_bits);
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 17, mesh, &[1, r, g, b]),
        Ok(0)
    );
    assert_eq!(com(&mut m, "IDirect3DRMMesh", 18, mesh, &[0, 4]), Ok(0));
    assert_eq!(com(&mut m, "IDirect3DRMMesh", 19, mesh, &[0, 0x88]), Ok(0));
    let (first, second) = (group(&m, 0), group(&m, 1));
    assert_eq!(
        (first.colour, first.mapping, first.quality),
        (Some(0x80ff_0000), Some(4), Some(0x88))
    );
    assert_eq!(
        (second.colour, second.mapping, second.quality),
        (Some(0xff00_ff00), None, None)
    );
    // Every group at once has not been asked for.
    assert!(matches!(
        com(&mut m, "IDirect3DRMMesh", 16, mesh, &[u32::MAX, 0]),
        Err(Stop::NotWritten { .. })
    ));

    // SetGroupMaterial and SetGroupTexture, at 20 and 21: the group holds a
    // reference to each, and lets go of one it is no longer drawn with.
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 13, 0, &[1.0f32.to_bits(), OUT]),
        Ok(0)
    );
    let material = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(com(&mut m, "IDirect3DRM2", 18, 0, &[0, OUT]), Ok(0));
    let texture = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 20, mesh, &[0, material]),
        Ok(0)
    );
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 21, mesh, &[1, texture]),
        Ok(0)
    );
    assert_eq!(
        (group(&m, 0).material, group(&m, 1).texture),
        (material, texture)
    );
    // AddRef, at 1, answers how many references there are now.
    assert_eq!(com(&mut m, "IDirect3DRMMaterial", 1, material, &[]), Ok(3));
    assert_eq!(com(&mut m, "IDirect3DRMMesh", 20, mesh, &[0, 0]), Ok(0));
    assert_eq!(com(&mut m, "IDirect3DRMMaterial", 1, material, &[]), Ok(3));
    assert_eq!(group(&m, 0).material, 0);
}

/// A mesh's box holds every vertex of every group, and a clone is a mesh of
/// its own with the same groups (#119).
#[test]
fn a_mesh_has_a_box_round_its_vertices_and_clones_to_a_mesh_of_its_own() {
    let mut m = machine();
    let mesh = mesh(&mut m, 2);
    // GetBox(box), at 13: not while a vertex has nowhere to be.
    let bounds = |m: &mut Machine, of: u32| com(m, "IDirect3DRMMesh", 13, of, &[OUT]);
    assert!(matches!(bounds(&mut m, mesh), Err(Stop::NotWritten { .. })));
    vertices(&mut m, DATA + 0x100, [1.0, -2.0, 0.5]);
    vertices(&mut m, DATA + 0x200, [-1.0, 4.0, 0.25]);
    for (id, at) in [(0, DATA + 0x100), (1, DATA + 0x200)] {
        assert_eq!(
            com(&mut m, "IDirect3DRMMesh", 15, mesh, &[id, 0, 4, at]),
            Ok(0)
        );
    }
    assert_eq!(bounds(&mut m, mesh), Ok(0));
    assert_eq!(vector(&m, OUT), [-3.0, -6.0, 0.0]);
    assert_eq!(vector(&m, OUT + 12), [3.0, 12.0, 1.5]);

    // Clone(outer, interface, out), at 3, as IID_IDirect3DRMMesh.
    let iid = [
        0x01, 0x0d, 0xa8, 0xa3, 0x12, 0x6e, 0xcf, 0x11, 0xac, 0x4a, 0x00, 0x00, 0xc0, 0x38, 0x25,
        0xa1,
    ];
    m.arena.write(DATA, &iid).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 3, mesh, &[0, DATA, OUT]),
        Ok(0)
    );
    let clone = m.arena.read_u32(OUT).expect("in the arena");
    assert_ne!(clone, mesh);
    assert_eq!(m.state.interface_of(clone), Some("IDirect3DRMMesh"));
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 16, clone, &[0, 0xff12_3456]),
        Ok(0)
    );
    let colour = |m: &Machine, of: u32| m.state.scene.meshes[&of].groups[0].colour;
    assert_eq!(
        (colour(&m, mesh), colour(&m, clone)),
        (None, Some(0xff12_3456))
    );
    assert_eq!(bounds(&mut m, clone), Ok(0));
    assert_eq!(vector(&m, OUT + 12), [3.0, 12.0, 1.5]);
    // As an aggregate, or as anything else, is a wall.
    assert!(matches!(
        com(&mut m, "IDirect3DRMMesh", 3, mesh, &[OUT, DATA, OUT]),
        Err(Stop::NotWritten { .. })
    ));
    m.arena.write(DATA, &[0; 16]).expect("in the arena");
    assert!(matches!(
        com(&mut m, "IDirect3DRMMesh", 3, mesh, &[0, DATA, OUT]),
        Err(Stop::NotWritten { .. })
    ));
}

/// IDirect3DRMViewport::ForceUpdate (#119): what retained mode draws here is
/// in the device's surface already, so a part of the viewport to copy to the
/// screen has nothing to do, and a part outside it is a wall.
#[test]
fn a_viewport_forced_to_update_has_nothing_to_copy() {
    let mut m = machine();
    // IDirect3DRM2::CreateViewport(device, camera, x, y, width, height, out),
    // at 20.
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 20, 0, &[0, 0, 0, 0, 640, 480, OUT]),
        Ok(0)
    );
    let viewport = m.arena.read_u32(OUT).expect("in the arena");
    // ForceUpdate(left, top, right, bottom), at 23.
    let force =
        |m: &mut Machine, area: [u32; 4]| com(m, "IDirect3DRMViewport", 23, viewport, &area);
    assert_eq!(force(&mut m, [0, 0, 640, 480]), Ok(0));
    assert_eq!(force(&mut m, [301, 12, 400, 90]), Ok(0));
    assert!(matches!(
        force(&mut m, [0, 0, 641, 480]),
        Err(Stop::NotWritten { .. })
    ));
}

/// Call the method of `interface` called `name` on `this`.
fn method(
    m: &mut Machine,
    interface: &str,
    name: &str,
    this: u32,
    args: &[u32],
) -> Result<u32, Stop> {
    let slot = win32::interface_named(interface)
        .and_then(|i| i.methods.iter().position(|method| method.name == name))
        .unwrap_or_else(|| panic!("{interface} has {name}"));
    com(m, interface, slot, this, args)
}

/// Call a method that makes something, and hand back what it made.
fn made(m: &mut Machine, interface: &str, name: &str, this: u32, args: &[u32]) -> u32 {
    let mut args = args.to_vec();
    args.push(OUT);
    assert_eq!(method(m, interface, name, this, &args), Ok(0), "{name}");
    m.arena.read_u32(OUT).expect("in the arena")
}

fn floats<const N: usize>(values: [f32; N]) -> [u32; N] {
    values.map(f32::to_bits)
}

/// A drawing surface's pixels, 64 by 48 at sixteen bits, and a texture's.
const PIXELS: u32 = 0x0052_0000;
const TEXELS: u32 = 0x0052_8000;

fn surface(m: &mut Machine, pixels: u32, width: u32, height: u32) -> u32 {
    with_ctx(m, |ctx| {
        win32::State::make_com(
            ctx,
            &win32::ddraw::DIRECT_DRAW_SURFACE,
            win32::ComState::Surface {
                pixels,
                width,
                height,
                pitch: width * 2,
                format: PixelFormat::RGB565,
                primary: false,
                back: 0,
                palette: 0,
            },
        )
        .map_err(Stop::Fault)
    })
    .expect("a surface")
}

fn pixel(m: &Machine, x: u32, y: u32) -> u32 {
    let at = PIXELS + y * 128 + x * 2;
    let bytes = m.arena.slice(at, 2).expect("in the arena");
    u32::from(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    PixelFormat::RGB565.encode([r, g, b, 255])
}

/// What LEGO Island draws its world through (#117, #120): an immediate-mode
/// device it made on a surface of its own, 64 by 48 here, and DirectX 5's
/// retained mode over that at `quality`; a scene, blue behind, and a camera
/// at its middle looking down z, seen through a viewport over the whole
/// device whose front plane is 0.1 away and whose field there is 0.05 either
/// side: a half-width of a half at every unit of distance.
fn stage(m: &mut Machine, quality: u32) -> (u32, u32) {
    let surface = surface(m, PIXELS, 64, 48);
    let immediate = with_ctx(m, |ctx| {
        win32::State::make_com(
            ctx,
            &win32::d3d::DEVICE2,
            win32::ComState::Device3D { surface },
        )
        .map_err(Stop::Fault)
    })
    .expect("a device");
    let device = made(m, "IDirect3DRM2", "CreateDeviceFromD3D", 0, &[0, immediate]);
    assert_eq!(
        method(m, "IDirect3DRMDevice2", "SetQuality", device, &[quality]),
        Ok(0)
    );
    let scene = made(m, "IDirect3DRM2", "CreateFrame", 0, &[0]);
    let camera = made(m, "IDirect3DRM2", "CreateFrame", 0, &[scene]);
    let viewport = made(
        m,
        "IDirect3DRM2",
        "CreateViewport",
        0,
        &[device, camera, 0, 0, 64, 48],
    );
    let view = |m: &mut Machine, name: &str, value: f32| {
        assert_eq!(
            method(m, "IDirect3DRMViewport", name, viewport, &[value.to_bits()]),
            Ok(0)
        );
    };
    view(m, "SetFront", 0.1);
    view(m, "SetBack", 10.0);
    view(m, "SetField", 0.05);
    let blue = floats([0.0, 0.0, 1.0]);
    assert_eq!(
        method(
            m,
            "IDirect3DRMFrame2",
            "SetSceneBackgroundRGB",
            scene,
            &blue
        ),
        Ok(0)
    );
    (viewport, scene)
}

/// A mesh of one group: a square a unit across, facing back down z at the
/// camera, wound clockwise as the camera sees it, and white; textured left
/// to right and top to bottom.
fn square(m: &mut Machine, quality: u32) -> u32 {
    let mesh = made(m, "IDirect3DRM2", "CreateMesh", 0, &[]);
    for (at, index) in [2, 3, 1, 2, 1, 0].into_iter().enumerate() {
        m.arena
            .write_u32(DATA + 4 * at as u32, index)
            .expect("in the arena");
    }
    assert_eq!(
        made(m, "IDirect3DRMMesh", "AddGroup", mesh, &[4, 2, 3, DATA]),
        0
    );
    for (n, [x, y, u, v]) in [
        [-0.5, -0.5, 0.0, 1.0],
        [0.5, -0.5, 1.0, 1.0],
        [-0.5, 0.5, 0.0, 0.0],
        [0.5, 0.5, 1.0, 0.0],
    ]
    .into_iter()
    .enumerate()
    {
        let vertex = floats([x, y, 0.0, 0.0, 0.0, -1.0, u, v]);
        for (i, word) in vertex.into_iter().chain([0]).enumerate() {
            m.arena
                .write_u32(DATA + 0x100 + 36 * n as u32 + 4 * i as u32, word)
                .expect("in the arena");
        }
    }
    let group = |m: &mut Machine, name: &str, args: &[u32]| {
        assert_eq!(
            method(m, "IDirect3DRMMesh", name, mesh, args),
            Ok(0),
            "{name}"
        );
    };
    group(m, "SetVertices", &[0, 0, 4, DATA + 0x100]);
    group(m, "SetGroupQuality", &[0, quality]);
    group(m, "SetGroupMapping", &[0, 4]);
    let white = floats([1.0, 1.0, 1.0]);
    group(m, "SetGroupColorRGB", &[0, white[0], white[1], white[2]]);
    mesh
}

/// Hang a frame in the scene `distance` in front of the camera, and the
/// mesh on a frame of its own that is that frame's visual, as LEGO Island
/// shares its models between the places it puts them.
fn hang(m: &mut Machine, scene: u32, mesh: u32, distance: f32) -> u32 {
    let place = made(m, "IDirect3DRM2", "CreateFrame", 0, &[scene]);
    let at = floats([0.0, 0.0, distance]);
    assert_eq!(
        method(
            m,
            "IDirect3DRMFrame2",
            "SetPosition",
            place,
            &[scene, at[0], at[1], at[2]]
        ),
        Ok(0)
    );
    let model = made(m, "IDirect3DRM2", "CreateFrame", 0, &[0]);
    assert_eq!(
        method(m, "IDirect3DRMFrame2", "AddVisual", model, &[mesh]),
        Ok(0)
    );
    assert_eq!(
        method(m, "IDirect3DRMFrame2", "AddVisual", place, &[model]),
        Ok(0)
    );
    place
}

fn draw(m: &mut Machine, viewport: u32, scene: u32) {
    assert_eq!(
        method(m, "IDirect3DRMViewport", "Clear", viewport, &[]),
        Ok(0)
    );
    assert_eq!(
        method(m, "IDirect3DRMViewport", "Render", viewport, &[scene]),
        Ok(0)
    );
}

/// What LEGO Island's retained mode does to draw its world (#120). DirectX
/// 5's Clear takes no flags and clears the viewport to its scene's
/// background and its depth buffer to as far as it goes; Render draws every
/// mesh in the scene where its frame puts it, lit by the scene's lights.
///
/// The field is the half-width at the front plane, not at a unit away: with
/// the front plane at 0.1 and a field of 0.05 there, a square a unit across
/// two units away covers the middle half of the viewport. Read as the
/// half-width at a unit away, it would be twenty times as wide and cover
/// all of it.
#[test]
fn a_viewport_clears_to_its_scene_and_draws_its_meshes_lit_where_they_are() {
    let mut m = machine();
    // Flat, lit and solid: the device's quality and the group's.
    let (viewport, scene) = stage(&mut m, 0x88);
    let mesh = square(&mut m, 0x88);
    let place = hang(&mut m, scene, mesh, 2.0);
    // A quarter of white all round, and half of white shining down z, full
    // on the square's face.
    let quarter = floats([0.25; 3]);
    let half = floats([0.5; 3]);
    let ambient = made(
        &mut m,
        "IDirect3DRM2",
        "CreateLightRGB",
        0,
        &[0, quarter[0], quarter[1], quarter[2]],
    );
    let sun = made(
        &mut m,
        "IDirect3DRM2",
        "CreateLightRGB",
        0,
        &[3, half[0], half[1], half[2]],
    );
    assert_eq!(
        method(&mut m, "IDirect3DRMFrame2", "AddLight", scene, &[ambient]),
        Ok(0)
    );
    assert_eq!(
        method(&mut m, "IDirect3DRMFrame2", "AddLight", scene, &[sun]),
        Ok(0)
    );

    draw(&mut m, viewport, scene);
    // A light's colour is kept in bytes: a quarter is 63 of 255 and a half
    // 127, which together are 190.
    let (lit, sky) = (rgb(190, 190, 190), rgb(0, 0, 255));
    for x in [17, 32, 46] {
        assert_eq!(pixel(&m, x, 24), lit, "{x} across is the square, lit");
    }
    for x in [14, 49] {
        assert_eq!(
            pixel(&m, x, 24),
            sky,
            "{x} across is the scene's background"
        );
    }
    assert_eq!(pixel(&m, 32, 2), sky);

    // Moved back, the square is smaller and farther than what was drawn
    // before: drawn only because Clear took the depth buffer back too.
    let at = floats([0.0, 0.0, 3.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "SetPosition",
            place,
            &[scene, at[0], at[1], at[2]]
        ),
        Ok(0)
    );
    draw(&mut m, viewport, scene);
    assert_eq!(pixel(&m, 32, 24), lit);
    assert_eq!(pixel(&m, 17, 24), sky);
}

/// A group's texture is DirectX 3's face of it, which LEGO Island asks for
/// DirectX 5's face, `{120f30c0-1629-11d0-941c-0080c80cfa7b}` as Wine's
/// `d3drmobj.h` names `IID_IDirect3DRMTexture2`; and it is drawn from its
/// surface as the surface is when it is drawn, so changing the surface's
/// pixels changes it, and `Changed` has nothing to do (#120).
#[test]
fn a_texture_is_drawn_from_its_surface_as_the_surface_is_now() {
    let mut m = machine();
    // Unlit: each pixel is the texel under it.
    let (viewport, scene) = stage(&mut m, 0x80);
    let mesh = square(&mut m, 0x80);
    hang(&mut m, scene, mesh, 2.0);
    // Two texels: red, then green.
    let (red, green, blue) = (rgb(255, 0, 0), rgb(0, 255, 0), rgb(0, 0, 255));
    let texels = |m: &mut Machine, left: u32, right: u32| {
        m.arena
            .write_u32(TEXELS, right << 16 | left)
            .expect("in the arena");
    };
    texels(&mut m, red, green);
    let image = surface(&mut m, TEXELS, 2, 1);
    let texture = made(
        &mut m,
        "IDirect3DRM2",
        "CreateTextureFromSurface",
        0,
        &[image],
    );
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMMesh",
            "SetGroupTexture",
            mesh,
            &[0, texture]
        ),
        Ok(0)
    );
    draw(&mut m, viewport, scene);
    assert_eq!((pixel(&m, 20, 24), pixel(&m, 44, 24)), (red, green));

    // GetGroupTexture hands back DirectX 3's face of the same texture.
    let older = made(&mut m, "IDirect3DRMMesh", "GetGroupTexture", mesh, &[0]);
    assert_eq!(m.state.interface_of(older), Some("IDirect3DRMTexture"));
    assert_eq!(m.state.identity(older), m.state.identity(texture));
    let iid = [
        0xc0, 0x30, 0x0f, 0x12, 0x29, 0x16, 0xd0, 0x11, 0x94, 0x1c, 0x00, 0x80, 0xc8, 0x0c, 0xfa,
        0x7b,
    ];
    m.arena.write(DATA, &iid).expect("in the arena");
    let newer = made(
        &mut m,
        "IDirect3DRMTexture",
        "QueryInterface",
        older,
        &[DATA],
    );
    assert_eq!(m.state.interface_of(newer), Some("IDirect3DRMTexture2"));

    texels(&mut m, blue, red);
    assert_eq!(
        method(&mut m, "IDirect3DRMTexture2", "Changed", newer, &[1, 0]),
        Ok(0)
    );
    draw(&mut m, viewport, scene);
    assert_eq!((pixel(&m, 20, 24), pixel(&m, 44, 24)), (blue, red));
}

/// A null reference frame is the world (#120): a frame hung from one moved
/// ten across is put where it is asked to be in the world, and read back
/// from there, whatever its parent.
#[test]
fn a_null_reference_frame_is_the_world() {
    let mut m = machine();
    let parent = made(&mut m, "IDirect3DRM2", "CreateFrame", 0, &[0]);
    let moved = floats([10.0, 0.0, 0.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "SetPosition",
            parent,
            &[0, moved[0], moved[1], moved[2]]
        ),
        Ok(0)
    );
    let child = made(&mut m, "IDirect3DRM2", "CreateFrame", 0, &[parent]);
    let at = floats([1.0, 2.0, 3.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "SetPosition",
            child,
            &[0, at[0], at[1], at[2]]
        ),
        Ok(0)
    );
    assert_eq!(
        method(&mut m, "IDirect3DRMFrame2", "GetPosition", child, &[0, OUT]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [1.0, 2.0, 3.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "GetPosition",
            child,
            &[parent, OUT]
        ),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [-9.0, 2.0, 3.0]);
}
