//! LEGO Island, run headless from the player's rip: the launcher finding it
//! or refusing it, its declared tree, its boot, and its committed routes to
//! first light and to its Information Center's idle loop, each replayed to the
//! same machine every time.
//!
//! These need the player's own rip under `game-data/`. Without it the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_island::LEGO_ISLAND as ISLAND;
use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::testkit;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::Title;

/// Enough instructions for both C runtimes to start, the game to read its
/// settings back and open its window (#114), its engine to make its Direct3D
/// 2 device and hand it to retained mode (#117), the game to count two
/// hundred frames down, open its scripts and start building its world out of
/// `WORLD.WDB` (#118), and retained mode to keep its first two meshes (#119).
/// The boot is traced, call by call, and a trace of the whole world costs
/// ten times any other run's log, so the world is [`WORLD_BUDGET`]'s.
const BOOT_BUDGET: u64 = 5_571_000;

/// Enough instructions for the game to build its world, about four hundred
/// meshes, out of `WORLD.WDB` (#119), play its intro, go into its Information
/// Center, whose actors retained mode draws itself (#120), and come round its
/// idle loop there once and a quarter (#121): flip 5249 is presented at
/// 3,964,530,295.
const WORLD_BUDGET: u64 = 3_965_000_000;

/// The Information Center's first frame: the stage painted, and the two
/// plants on it drawn by retained mode.
const INFORMATION_CENTER: u64 = 3200;

/// The Information Center's idle loop (#121): its first frame, the milestone
/// the route is held to.
///
/// The milestone is the loop's first frame rather than the room's, because the
/// room idles twice. The Infomaniac first stands by the right rail, and the
/// room loops there from flip 3905, but at flip 4937 he walks over beside the
/// desk by himself. From here he stays, and the loop holds for as long as
/// nobody touches it. It starts here and not before: the frame before it, his
/// last step, is not the one a period later.
const IDLE_LOOP: u64 = 5099;

/// How many frames the idle loop takes to come round, every one of them
/// counted: 88 still, then 32 in which the brick on the desk rocks. Sampled
/// every hundredth flip it reads as a loop of 600, lcm(100, 120), which is the
/// sampling's beat and not the game's.
const PERIOD: u64 = 120;

/// A frame of the loop with the brick on the desk tipped up.
const ROCKING: u64 = IDLE_LOOP + 30;

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&ISLAND.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&roster::LEGO_ISLAND),
        Found::Runnable {
            pressing: "lego-island".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..ISLAND.image
        },
        ..ISLAND
    };
    let err = testkit::try_run_headless(&ABSENT, &Cli::default(), 1_000)
        .expect_err("there is no such pressing");

    assert!(
        err.contains("no-such-pressing"),
        "the refusal names the path: {err}"
    );
    assert!(
        err.contains("ripped from your own disc"),
        "and says what to do: {err}"
    );
}

#[test]
fn the_declared_tree_holds_what_the_installer_would_have_copied() {
    if !scratch() {
        return;
    }
    let ran = testkit::try_run_headless(&ISLAND, &Cli::default(), 1_000).expect("it loads");

    // Every file under the disc's own DATA\disk, and nothing from the
    // installer, the autorun menu or the DirectX redistributable beside it.
    ran.assert_logged("installed 27 files");
}

/// The run every boot test looks at, done once.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &ISLAND, &cli, &[], BOOT_BUDGET)
}

#[test]
fn the_game_and_its_engine_load_from_the_disc_and_start_calling() {
    let Some(ran) = boot() else { return };

    // The game lands where it asks to, and the engine it ships beside it is
    // loaded as guest code (ADR-0002).
    ran.assert_logged("image DATA/disk/ISLE.EXE base 0x00400000")
        .assert_logged("imports 159")
        .assert_logged("module Lego1.dll base")
        // The engine's C runtime starts first, then the game's own.
        .assert_logged("call KERNEL32.dll!GetVersion() from 0x0040830c");
}

#[test]
fn the_game_finds_the_three_paths_its_installer_wrote_and_nothing_else() {
    let Some(ran) = boot() else { return };
    let missing = |name: &str| {
        ran.log.contains(&format!(
            "registry value missing HKLM\\SOFTWARE\\Mindscape\\LEGO Island\\{name}"
        ))
    };
    for name in ["diskpath", "cdpath", "savepath"] {
        assert!(!missing(name), "{name} is declared and read back");
    }
    // The settings dialog's values are not written, and the game takes its
    // own default for each: 16 bits for the display among them.
    for name in ["Full Screen", "Display Bit Depth", "3D Device ID"] {
        assert!(missing(name), "{name} is left to the game's default");
    }
}

#[test]
fn the_game_opens_its_window_and_its_engine_asks_for_direct3d_2() {
    let Some(ran) = boot() else { return };

    // Mouse trails off, a window with a caption, and WIN.INI asked for a
    // media path it does not have (#114).
    ran.assert_logged("call USER32.dll!SystemParametersInfoA(0x5d, 0x0, 0x0, 0x0)")
        .assert_logged("window class Lego Island MainNoM App procedure 0x00401d20")
        .assert_logged("641x481 at 0,0")
        .assert_logged("call KERNEL32.dll!GetProfileStringA(")
        // The engine starts its sound, then looks for a display to draw on
        // with Direct3D, and asks for it by the second interface's name.
        .assert_logged("dsound primary format 11025 Hz, 2 channels, 16 bits")
        .assert_logged("ddraw modes: 640x480x16, the only one declared")
        .assert_logged("com IDirectDraw handed back IDirect3D2");
}

#[test]
fn the_engine_makes_its_own_direct3d_2_device_and_hands_it_to_retained_mode() {
    let Some(ran) = boot() else { return };

    // Immediate mode first, made by the engine itself (#117): the declared
    // machine's one device, the screen's own mode, and the device over the
    // back buffer.
    ran.assert_logged("d3d2 enumerate: one device, and it rasterises")
        .assert_logged("display mode 640x480x16")
        .assert_logged("d3d2 device at")
        // Then DirectX 5's retained mode over it, and a viewport the size of
        // the screen.
        .assert_logged("com IDirect3DRM handed back IDirect3DRM2")
        .assert_logged("d3drm device at")
        .assert_logged("d3drm viewport 640x480 at 0,0");
    assert!(
        !ran.log.contains("and refused"),
        "every face the engine asks for is one it is given"
    );
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing in Direct3D or retained mode stops the run"
    );
}

/// The frames the game presents before its world is built, all of them black
/// (#118): the screen its engine clears, then its back buffer copied onto the
/// screen once a frame while it counts two hundred of them down to its
/// scripts.
#[test]
fn the_game_counts_its_frames_down_to_its_scripts_and_starts_its_world() {
    let Some(ran) = boot() else { return };

    // Its window's instance, and no table of keys in its own image.
    ran.assert_logged("call USER32.dll!GetWindowLongA(0x10010, 0xfffffffa)")
        .assert_logged("call USER32.dll!LoadAcceleratorsA(0x400000, 0x4100bc)");

    // The engine lets go of the whole screen it has just cleared, which is a
    // frame: the screen is what a person would see, and there is no flipping
    // chain behind it. Every frame after is a copy of the whole back buffer
    // onto the screen.
    ran.assert_logged("flip 0 cursor 458966 640x480x16")
        .assert_logged("call IDirectDrawSurface::Blt(0x1ffcb0, 0x20b6cab8, 0x1ffca0, 0x0, 0x1ffcc0) on 0x20ad6aa8")
        .assert_logged("flip 1 cursor 644157 640x480x16");
    assert!(
        ran.host.frames.iter().all(|f| f.is_uniform()),
        "nothing is drawn before the world is built"
    );

    // After two hundred frames of its own, flips 1 to 200, its scripts are
    // opened where they lie on the disc, and then its world database from
    // the install. The count is the game's (`ISLE.EXE` 0x4017cb), and that
    // it lands on flip 200 says the cleared screen before them is no frame of
    // its loop's.
    let opened = |name: &str| {
        let line = ran
            .log
            .find(name)
            .unwrap_or_else(|| panic!("{name} is opened"));
        ran.log[..line].matches("\nflip ").count()
    };
    assert_eq!(opened("file opened D:\\lego\\scripts\\isle\\isle.si"), 201);
    ran.assert_logged("file opened C:\\Program Files\\LEGO Island\\lego\\data\\world.wdb")
        // Its textures through DirectX 5's retained mode, and SmartHeap asking
        // how much of a region it reserved is committed.
        .assert_logged("call IDirect3DRM2::CreateTextureFromSurface(")
        .assert_logged("call KERNEL32.dll!VirtualQuery(0x20d50000, ")
        .assert_logged("call IDirect3DRM2::CreateMaterial(0x41200000, ");
    assert_eq!(ran.finish.flips, 203);
}

/// The first meshes of the world (#119): fourteen vertices and twelve
/// triangles in one group, then a mesh of two groups, the second numbered as
/// retained mode numbered it.
#[test]
fn retained_mode_keeps_the_meshes_the_world_is_built_of() {
    let Some(ran) = boot() else { return };
    ran.assert_logged("call IDirect3DRM2::CreateMesh(")
        .assert_logged("call IDirect3DRMMesh::AddGroup(0xe, 0xc, 0x3, ")
        .assert_logged("call IDirect3DRMMesh::SetVertices(0x0, 0x0, 0xe, ")
        .assert_logged("call IDirect3DRMMesh::SetGroupQuality(0x0, 0x88)")
        .assert_logged("call IDirect3DRMMesh::SetVertices(0x1, 0x0, 0x15, ");
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted
    );
}

/// First light (#118): the flip the game's own picture first reaches the
/// screen at, the first instant of its LEGO bricks fading up out of black.
///
/// First is decided as every title's is: the first frame that is not one
/// colour. Flip 0, the engine letting go of the screen it has just cleared, is
/// a present, but it is black and outside `ISLE.EXE`'s loop; flips 1 to 459
/// are the loop copying a back buffer that is still black. The loop's frames
/// are numbered from flip 1, so this is its 460th whether flip 0 is counted or
/// not.
const FIRST_LIGHT: u64 = 460;

/// Enough instructions for the game to build its world and present first
/// light, at 97,000,222, and a frame more.
const FIRST_LIGHT_BUDGET: u64 = 98_000_000;

/// The committed route to first light, replayed, keeping first light and the
/// frame before it. Two slots, so the determinism gate's two runs can go side
/// by side.
fn first_light(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/first-light.route"),
        ),
        ..Cli::default()
    };
    testkit::replay(
        slot,
        scratch,
        &ISLAND,
        &cli,
        &[FIRST_LIGHT - 1, FIRST_LIGHT],
        FIRST_LIGHT_BUDGET,
    )
}

static LIT: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

#[test]
fn the_route_reaches_first_light_as_the_bricks_start_to_fade_up() {
    let Some(ran) = first_light(&LIT) else { return };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way to first light stops the run"
    );
    ran.assert_logged("flip 460 cursor 97000222 640x480x16");

    let lit = ran
        .host
        .frames
        .iter()
        .find(|f| !f.is_uniform())
        .expect("the game puts a picture on the screen");
    assert_eq!(
        lit.flip, FIRST_LIGHT,
        "first light moved: it is now flip {}",
        lit.flip
    );
    assert_eq!((lit.width, lit.height), (640, 480));
    // The fade's first instant: every pixel within 8/255 of black.
    assert!(
        lit.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[..3].iter().all(|c| *c <= 8)),
        "first light is the bricks barely up out of black"
    );
    let before = ran
        .host
        .frame(FIRST_LIGHT - 1)
        .expect("the frame before it is kept");
    assert_eq!(before.pixels[..3], [0, 0, 0], "and before it, black");
}

#[test]
fn the_first_light_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    // The second replay runs beside the first rather than after it.
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| first_light(&AGAIN));
        (
            first_light(&LIT),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    assert_eq!(
        first.host.frame(FIRST_LIGHT).map(|f| &f.pixels),
        second.host.frame(FIRST_LIGHT).map(|f| &f.pixels),
        "including what it presented"
    );
}

/// The committed route to the Information Center's idle loop, replayed: once
/// for the tests, and once more beside it for the determinism gate.
fn world(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("routes/information-center.route"),
        ),
        ..Cli::default()
    };
    testkit::replay(
        slot,
        scratch,
        &ISLAND,
        &cli,
        &[
            INFORMATION_CENTER - 1,
            INFORMATION_CENTER,
            IDLE_LOOP - 1,
            IDLE_LOOP,
            ROCKING,
            IDLE_LOOP + PERIOD - 1,
            IDLE_LOOP + PERIOD,
            ROCKING + PERIOD,
        ],
        WORLD_BUDGET,
    )
}

static WORLD: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// Its four hundred meshes kept, its 3D sounds placed (their cones' outside
/// volume set) and its viewport forced to update, the game's world is built
/// and its intro plays on the screen (#119); and then its Information Center
/// is drawn, its viewport cleared, the room painted over it, and its actors
/// drawn in front by retained mode, lit and textured (#120).
#[test]
fn the_game_builds_its_world_plays_its_intro_and_draws_its_information_center() {
    let Some(ran) = world(&WORLD) else { return };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing stops the run in its Information Center"
    );
    assert_eq!(ran.finish.flips, 5254);
    assert!(
        ran.host.frames.iter().any(|f| !f.is_uniform()),
        "the intro is on the screen"
    );

    // Before the Information Center, the screen is cleared black; in its
    // first frame the pots of the plants either side of the stage are green,
    // where the painted room under them is not: retained mode's own meshes,
    // standing where the game draws them on Windows (#134).
    let frame = |flip: u64| ran.host.frame(flip).expect("kept");
    assert!(frame(INFORMATION_CENTER - 1).is_uniform());
    let first = frame(INFORMATION_CENTER);
    for (x, y) in [(180, 342), (472, 343)] {
        let at = (y * first.width as usize + x) * 4;
        let [a, green, b] = [0, 1, 2].map(|i| first.pixels[at + i]);
        assert!(
            green > a && green > b,
            "a plant at {x},{y}: {:?}",
            [a, green, b]
        );
    }
}

/// The milestone: from flip 5099 on, with the Infomaniac beside the desk, the
/// room goes round the same 120 frames.
#[test]
fn the_route_reaches_the_information_centers_idle_loop_and_it_holds() {
    let Some(ran) = world(&WORLD) else { return };
    ran.assert_logged("flip 5099 cursor 3952208885 640x480x16");
    let frame = |flip: u64| &ran.host.frame(flip).expect("kept").pixels;

    // Its first frame, and one with the brick tipped up, come round again a
    // period later; and the brick does move.
    for flip in [IDLE_LOOP, ROCKING] {
        assert!(
            frame(flip) == frame(flip + PERIOD),
            "flip {flip} comes round again at {}",
            flip + PERIOD
        );
    }
    assert!(frame(IDLE_LOOP) != frame(ROCKING), "the brick rocks");
    // And it starts at 5099: the Infomaniac's last step up to the desk is not
    // in it.
    assert!(
        frame(IDLE_LOOP - 1) != frame(IDLE_LOOP + PERIOD - 1),
        "the loop starts at flip {IDLE_LOOP}"
    );
}

#[test]
fn the_information_center_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    // The second replay runs beside the first rather than after it.
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| world(&AGAIN));
        (world(&WORLD), second.join().expect("the second replay ran"))
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    assert!(
        first.host.frame(IDLE_LOOP).map(|f| &f.pixels)
            == second.host.frame(IDLE_LOOP).map(|f| &f.pixels),
        "including what it presented at flip {IDLE_LOOP}"
    );
}
