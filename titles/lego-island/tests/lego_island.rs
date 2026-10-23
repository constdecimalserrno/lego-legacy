//! LEGO Island, run headless from the player's rip: the launcher finding it
//! or refusing it, its declared tree, its boot, and its committed routes to
//! first light and to its Information Center's idle loop, what the second is
//! heard to play, and each replayed to the same machine every time; and a
//! synthetic hand driving it through the host, out of the Information Center
//! and away on a bike, and the route its run recorded; and its saves, written
//! by a hand that signs the book and quits, saved again as the game closes
//! itself after its credits, and loaded by the next run over the same data
//! root; and one whole activity by hand, Nick Brick's pizza taken from the
//! Pizzeria to the gas station on the skateboard, and the game saved on the
//! way out; and the maintainer's own play, out on the bicycle to the beach,
//! replayed to where they closed the window; and a hand out of the door
//! holding the button still on the island until it walks.
//!
//! These need the player's own rip under `game-data/`. Without it the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_island::LEGO_ISLAND as ISLAND;
use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::{headless::Headless, Button, Input};
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
/// light, at 97,000,829, and a frame more.
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
    ran.assert_logged("flip 460 cursor 97000829 640x480x16");

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

/// The game sets its pointer's shape by posting its own window
/// `WM_ISLE_SETCURSOR`, which its window procedure answers with `SetCursor`
/// (#139): the hourglass as it starts, the arrow for a moment once its
/// scripts are open, the hourglass again, and no pointer at all for the intro.
/// Each reaches the host as the cursor `ISLE.EXE` holds, and the host's own
/// arrow comes back when the run ends.
#[test]
fn the_game_sets_its_pointers_shape_and_the_host_is_told() {
    let Some(ran) = first_light(&LIT) else { return };
    for line in [
        "pointer shape flip 2 0x30001",
        "pointer shape flip 406 0x30000",
        "pointer shape flip 409 0x30001",
        "pointer shape flip 456 0x0",
    ] {
        ran.assert_logged(line);
    }
    let shapes: Vec<_> = ran.host.shapes.iter().map(|(_, shape)| shape).collect();
    let [Some(hourglass), Some(arrow), Some(again), Some(none), None] = shapes[..] else {
        panic!("the host was told {} shapes", shapes.len());
    };
    assert_eq!(again, hourglass);
    let pixel = |c: &lego_legacy_core::host::Cursor, x: u32, y: u32| {
        c.pixels[((y * c.width + x) * 4) as usize..][..4].to_vec()
    };
    // The arrow points with its tip, black edged and white within; the
    // hourglass with its middle, its top a black bar.
    assert_eq!((arrow.width, arrow.height, arrow.hot), (32, 32, (1, 1)));
    assert_eq!(pixel(arrow, 0, 0), [0, 0, 0, 0], "clear about the tip");
    assert_eq!(pixel(arrow, 1, 1), [0, 0, 0, 255], "the edge");
    assert_eq!(pixel(arrow, 2, 2), [255, 255, 255, 255], "within");
    assert_eq!(
        (hourglass.width, hourglass.height, hourglass.hot),
        (32, 32, (16, 16))
    );
    assert_eq!(
        pixel(hourglass, 12, 5),
        [0, 0, 0, 255],
        "the top of the glass"
    );
    assert_eq!(pixel(hourglass, 0, 0), [0, 0, 0, 0]);
    // No pointer is a pointer that shows nothing.
    assert!(none.pixels.chunks(4).all(|p| p[3] == 0));
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
/// for the tests, and once more beside it for the determinism gate, that one
/// asking for `--audio`.
fn world(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    audio: bool,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("routes/information-center.route"),
        ),
        audio,
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
    let Some(ran) = world(&WORLD, false) else {
        return;
    };
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
    let Some(ran) = world(&WORLD, false) else {
        return;
    };
    ran.assert_logged("flip 5099 cursor 3952209794 640x480x16");
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

/// The flip the Information Center's music starts at (#123): the Infomaniac's
/// welcome over, the game plays `InformationCenter_Music` out of
/// `JUKEBOX.SI`, and it plays on under everything after.
const MUSIC: u64 = 3938;

/// LEGO Island is heard (#123): its intro, the Infomaniac, and the Information
/// Center's music, on the mix the route makes, block by block. Each sound in it
/// was held sample for sample against the wave it plays out of the disc's
/// scripts, at the volume the game gave it; #123's comment says how.
#[test]
fn the_route_is_heard_from_its_intro_to_the_information_centers_music() {
    let Some(ran) = world(&WORLD, false) else {
        return;
    };
    let mean = |r: &[f64]| r.iter().sum::<f64>() / r.len().max(1) as f64;

    // Nothing until the LEGO logo's sound starts, a flip before first light.
    assert!(
        ran.loudness(0, FIRST_LIGHT - 1).iter().all(|r| *r == 0.0),
        "silence before the intro"
    );
    // The intro's three waves out of `INTRO.SI`, one after another: the LEGO
    // logo's, Mindscape's, and the intro movie's.
    for (from, to) in [(FIRST_LIGHT, 870), (880, 1055), (1115, 3195)] {
        let heard = mean(&ran.loudness(from, to));
        assert!(heard > 2000.0, "flips {from} to {to} are heard: {heard:.0}");
    }
    // In the Information Center the Infomaniac welcomes the player, his speech
    // and the sounds his animation makes in 3D buffers about him.
    let welcome = mean(&ran.loudness(3241, MUSIC));
    assert!(welcome > 1000.0, "the welcome is heard: {welcome:.0}");

    // Then the music, under the Infomaniac at the game's own lowered volume,
    // in every block to the end of the route.
    let music = ran.loudness(MUSIC + 1, u64::MAX);
    assert!(
        music.len() > 900 && music.iter().all(|r| *r > 0.0),
        "the music is heard to the end: {} blocks",
        music.len()
    );
    // It is a two and a half minute wave streamed through a buffer of two
    // seconds: two seconds on, it is the next part, not the same two again.
    let mix = ran.host.audio_blocks.concat();
    let block = ran.host.audio_blocks[0].len();
    let at = block
        * ran
            .host
            .audio_flips
            .iter()
            .position(|flip| *flip >= MUSIC + 60)
            .expect("mixed this far");
    let fifth = |from: usize| mix.get(from..from + 8820).expect("mixed this far");
    let two_seconds = 2 * lego_legacy_core::sound::RATE as usize * 2;
    assert!(
        fifth(at) != fifth(at + two_seconds),
        "the music goes on past its buffer"
    );
}

#[test]
fn the_information_center_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    // The second replay runs beside the first rather than after it, and asks
    // for `--audio`. Headless, that reaches only the run log: whether a
    // device listens is nothing the guest can see, since every answer
    // DirectSound gives comes from the declared clock (#123).
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| world(&AGAIN, true));
        (
            world(&WORLD, false),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    second.assert_logged("audio true");
    first.assert_same_machine(second);
    assert!(
        first.host.frame(IDLE_LOOP).map(|f| &f.pixels)
            == second.host.frame(IDLE_LOOP).map(|f| &f.pixels),
        "including what it presented at flip {IDLE_LOOP}"
    );
    assert!(
        first.host.audio_blocks == second.host.audio_blocks,
        "and what it mixed"
    );
}

/// A person's hand, as a run in a window takes it: through the host, whose
/// `poll_input` hands the machine what was done since it was last asked, and
/// not through a route (#124). A route replaces the hand, and a whole class of
/// fault is only met by a hand: the old tree's LEGO Island "crashed right
/// away" for a person and never for a replay, because no route of it pressed
/// anything.
///
/// The pointer glides where it goes, a few pixels a flip as a mouse moves it,
/// rather than jumping as a route may; a button is held a couple of flips; a
/// key as long as it is held.
struct Hand {
    at: (i32, i32),
    flip: u64,
    done: Vec<(u64, Input)>,
}

/// How far a gliding pointer goes in a flip, on each axis.
const GLIDE: i32 = 16;

/// The keys the hand presses, as DirectInput numbers them.
const KEY_E: u32 = 0x12;
const KEY_I: u32 = 0x17;
const KEY_O: u32 = 0x18;
const KEY_S: u32 = 0x1f;
const KEY_G: u32 = 0x22;
const KEY_L: u32 = 0x26;
const KEY_UP: u32 = 0xc8;
const KEY_LEFT: u32 = 0xcb;
const KEY_RIGHT: u32 = 0xcd;

impl Hand {
    /// Where Windows leaves the pointer after a mode is set: the middle of
    /// the screen.
    fn new() -> Self {
        Self {
            at: (320, 240),
            flip: 0,
            done: Vec::new(),
        }
    }

    /// Nothing until this flip.
    fn wait(mut self, flip: u64) -> Self {
        self.flip = self.flip.max(flip);
        self
    }

    fn glide(mut self, to: (i32, i32)) -> Self {
        while self.at != to {
            let step = |from: i32, to: i32| from + (to - from).clamp(-GLIDE, GLIDE);
            self.at = (step(self.at.0, to.0), step(self.at.1, to.1));
            let (x, y) = self.at;
            self.done.push((self.flip, Input::Move { x, y }));
            self.flip += 1;
        }
        self
    }

    /// The left button pressed over a place, and let go two flips later.
    fn click(self, at: (i32, i32)) -> Self {
        let mut hand = self.glide(at);
        hand.done.push((hand.flip, Input::Down(Button::Left)));
        hand.done.push((hand.flip + 2, Input::Up(Button::Left)));
        hand.flip += 3;
        hand
    }

    /// A key held down for so many flips.
    fn hold(mut self, code: u32, flips: u64) -> Self {
        self.done.push((self.flip, Input::Key { code, down: true }));
        self.flip += flips;
        self.done
            .push((self.flip, Input::Key { code, down: false }));
        self.flip += 1;
        self
    }

    /// A key pressed and kept down while the hand goes on to other things, as
    /// a person keeps going forward while steering (#137).
    fn press(mut self, code: u32) -> Self {
        self.done.push((self.flip, Input::Key { code, down: true }));
        self
    }

    /// A key kept down by [`Hand::press`], let go.
    fn release(mut self, code: u32) -> Self {
        self.done
            .push((self.flip, Input::Key { code, down: false }));
        self
    }

    /// A host that hands the machine what this hand did, at the flips it did
    /// it, keeping the frames a test looks at.
    fn host(self, keep: &[u64]) -> Headless {
        let mut host = Headless::new().keeping(keep);
        host.queue(self.done);
        host
    }
}

/// A run taking its input from a hand through the host, recording what it
/// was given as a route, done once into its slot.
fn by_hand(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    hand: impl FnOnce() -> Hand,
    keep: &[u64],
    budget: u64,
) -> Option<&'static testkit::Ran> {
    slot.get_or_init(|| {
        if !scratch() {
            return None;
        }
        let cli = Cli {
            record: true,
            ..Cli::default()
        };
        Some(testkit::run_on(hand().host(keep), &ISLAND, &cli, budget))
    })
    .as_ref()
}

/// A restless hand from the first flip: the pointer goes round and round the
/// screen, and the button is pressed every 97 flips, through the boot, the
/// intro, and into the Information Center.
fn restless() -> Hand {
    let mut hand = Hand::new();
    while hand.flip < 900 {
        let now = hand.flip;
        let turn = now as f64 / 40.0;
        let to = (
            320 + (200.0 * turn.cos()) as i32,
            240 + (150.0 * turn.sin()) as i32,
        );
        hand = if now > 0 && now.is_multiple_of(97) {
            hand.click(to)
        } else {
            hand.glide(to)
        };
        hand = hand.wait(now + 1);
    }
    hand
}

/// Enough for the restless hand's clicks to skip the intro and reach the
/// Information Center: flip 800 is presented at 317 million.
const RESTLESS_BUDGET: u64 = 320_000_000;

/// The old tree's live crash, met and gone (#124). The hand's first press of
/// the button, at flip 98, under two declared seconds into the black countdown
/// before first light, has the game ask retained mode which of its 3D objects is under the
/// pointer, `IDirect3DRMViewport::Pick`. The old tree had no such method and
/// ended the run there, which is what a person saw as LEGO Island crashing
/// right away; this tree stopped at the same wall, at instruction 942,097,
/// until #124 answered it. No route had ever pressed a button.
#[test]
fn a_restless_hand_from_the_first_flip_meets_no_wall() {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let Some(ran) = by_hand(&RAN, restless, &[800], RESTLESS_BUDGET) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing a hand does stops the run"
    );
    // The hand's input came through the host: no route was replayed.
    assert!(!ran.log.contains("\nroute "), "no route was replayed");
    ran.assert_logged("input flip 98 Down(Left)");
    // Its clicks were taken: the intro is skipped, and the Information Center
    // is up at flip 800, more than two thousand flips before the intro ends
    // untouched, its plants' pots green either side of the stage.
    let frame = ran.host.frame(800).expect("kept");
    for (x, y) in [(180, 342), (472, 343)] {
        let [a, green, b] = pixel(frame, x, y);
        assert!(
            green > a && green > b,
            "a plant at {x},{y}: {:?}",
            [a, green, b]
        );
    }
}

/// A pixel of a kept frame: red, green and blue.
fn pixel(frame: &lego_legacy_core::host::headless::Presented, x: usize, y: usize) -> [u8; 3] {
    let at = (y * frame.width as usize + x) * 4;
    [2, 1, 0].map(|i| frame.pixels[at + i])
}

/// Where the ride presses, on the screen. In the Information Center: the blue
/// brick on the desk, which is the registration book, and the book's tick once
/// a name is written in it; Pepper's portrait, the third over the map; and the
/// arrow at the left, to the elevator. By the elevator, the arrow at its left,
/// to the door. The revolving door out. And outside, the red bicycle by the
/// door, once the walk has turned to face it.
const BOOK: (i32, i32) = (322, 294);
const TICK: (i32, i32) = (519, 135);
const PEPPER: (i32, i32) = (322, 105);
const TO_THE_ELEVATOR: (i32, i32) = (28, 310);
const TO_THE_DOOR: (i32, i32) = (35, 140);
const DOOR: (i32, i32) = (330, 220);
const BIKE: (i32, i32) = (70, 271);

/// In the Information Center, LEGO signs the registration book and picks
/// Pepper: where the ride and the first save both start.
fn signed_as_pepper() -> Hand {
    Hand::new()
        .wait(IDLE_LOOP + 50)
        .click(BOOK)
        .wait(7000)
        .hold(KEY_L, 2)
        .hold(KEY_E, 2)
        .hold(KEY_G, 2)
        .hold(KEY_O, 2)
        .wait(7100)
        .click(TICK)
        .wait(8500)
        .click(PEPPER)
}

/// Out of the Information Center and onto the island, and away on a bike
/// (#124).
fn ride() -> Hand {
    out_of_the_door()
        // Outside: a step forward, a turn left to face the bicycle, and on.
        .wait(13000)
        .hold(KEY_UP, 40)
        .wait(13100)
        .hold(KEY_LEFT, 140)
        .wait(13300)
        .click(BIKE)
        .wait(13600)
        .hold(KEY_UP, 200)
}

/// Out of the Information Center by its revolving door, which the island is
/// through a hundred flips later. The island will not let anyone out who has
/// not signed its book and picked who they are: the revolving door checks
/// both (`LEGO1.DLL` 0x10037b35), and a new name in the book picks nobody.
fn out_of_the_door() -> Hand {
    signed_as_pepper()
        .wait(10000)
        .click(TO_THE_ELEVATOR)
        .wait(11000)
        .click(TO_THE_DOOR)
        .wait(12000)
        .click(DOOR)
}

/// The flips the ride is looked at: by the revolving door, the island through
/// the doorway, on the bicycle, and away down the road on it.
const AT_THE_DOOR: u64 = 12000;
const OUTSIDE: u64 = 12100;
const ON_THE_BIKE: u64 = 13500;
const RIDING: u64 = 13800;

/// Enough for the ride to the end of its last key: flip 13900 is presented at
/// about 6.34 billion.
const RIDE_BUDGET: u64 = 6_350_000_000;

const RIDE_FRAMES: &[u64] = &[AT_THE_DOOR, OUTSIDE, ON_THE_BIKE, RIDING];

static RIDDEN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// The committed ride, replayed: what the hand did, as the run it did it in
/// recorded it.
fn ride_replayed(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(ride_route()),
        ..Cli::default()
    };
    testkit::replay(slot, scratch, &ISLAND, &cli, RIDE_FRAMES, RIDE_BUDGET)
}

fn ride_route() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/first-ride.route")
}

/// The ticket's bar (#124): by hand, through the host, out of the Information
/// Center onto the island and away on a bike, with no wall on the way. Each
/// press the island takes is one a person's hand would make, and two of them
/// met walls on the way: the book's blinking cursor is copied with a raster
/// operation, and signing it recolours a light that hangs on a frame.
#[test]
fn a_hand_walks_out_of_the_information_center_and_rides_a_bike() {
    let Some(ran) = by_hand(&RIDDEN, ride, RIDE_FRAMES, RIDE_BUDGET) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the ride stops the run"
    );
    let frame = |flip: u64| ran.host.frame(flip).expect("kept");
    let sky = |flip: u64| {
        let [r, g, b] = pixel(frame(flip), 320, 10);
        b > 200 && b > r + 100 && b > g + 30
    };
    // Inside, a grey ceiling; a hundred flips on, through the doorway, the
    // island's sky.
    assert!(!sky(AT_THE_DOOR), "inside, by the revolving door");
    assert!(sky(OUTSIDE), "outside, under the sky");
    // On the bike, its red handlebars and their stem across the foot of the
    // screen; and riding, the island going by above them.
    for flip in [ON_THE_BIKE, RIDING] {
        let [r, g, b] = pixel(frame(flip), 320, 400);
        assert!(r > 150 && g < 60 && b < 60, "the handlebars at flip {flip}");
    }
    assert!(
        testkit::differing(frame(ON_THE_BIKE), frame(RIDING), (0, 0, 640, 300)) > 30_000,
        "the island goes by"
    );
}

/// The committed route is the hand's own, as its run recorded it.
#[test]
fn the_committed_ride_is_what_the_hand_recorded() {
    let Some(ran) = by_hand(&RIDDEN, ride, RIDE_FRAMES, RIDE_BUDGET) else {
        return;
    };
    let recorded = lego_legacy_core::route::Route::load(&ran.dir.join("route"))
        .expect("the run recorded a route");
    let committed = lego_legacy_core::route::Route::load(&ride_route()).expect("it reads");
    assert_eq!(recorded, committed);
}

/// The determinism gate: the committed ride, replayed, ends on the machine
/// the hand's own run ended on, and presents what it presented.
#[test]
fn the_ride_replays_to_the_machine_the_hand_made() {
    static REPLAYED: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    // The replay runs beside the hand's run rather than after it.
    let (hand, replay) = std::thread::scope(|scope| {
        let replay = scope.spawn(|| ride_replayed(&REPLAYED));
        (
            by_hand(&RIDDEN, ride, RIDE_FRAMES, RIDE_BUDGET),
            replay.join().expect("the replay ran"),
        )
    });
    let (Some(hand), Some(replay)) = (hand, replay) else {
        return;
    };
    hand.assert_same_machine(replay);
    assert!(
        hand.host.frame(RIDING).map(|f| &f.pixels) == replay.host.frame(RIDING).map(|f| &f.pixels),
        "including what it presented at flip {RIDING}"
    );
}

/// In the Information Center, the quit door by the right rail, and the green
/// brick that answers yes when the Infomaniac asks whether to leave. In the
/// registration book, the tick beside the first name already written in it, a
/// row below the line a new name is written on.
const QUIT_DOOR: (i32, i32) = (590, 260);
const YES: (i32, i32) = (210, 400);
const FIRST_NAME: (i32, i32) = (519, 163);

/// LEGO signs the book and picks Pepper, then leaves by the quit door and
/// answers yes (#125). Signing writes the players, `Players.gsi`; yes saves
/// the game, `G0.GS`, and the scores, `History.gsi`, and the credits roll.
fn quit() -> Hand {
    signed_as_pepper()
        .wait(10000)
        .click(QUIT_DOOR)
        .wait(10600)
        .click(YES)
}

/// The flips the saves are looked at: the book open, nothing yet written in
/// it; the Information Center once the book is signed, before anyone is
/// picked; and the credits.
const BOOK_OPEN: u64 = 6600;
const SIGNED: u64 = 8200;
const CREDITS: u64 = 11000;
const QUIT_FRAMES: &[u64] = &[BOOK_OPEN, SIGNED, CREDITS];

/// Enough for the credits to start: flip 11000 is presented at about 6.03
/// billion.
const QUIT_BUDGET: u64 = 6_030_000_000;

/// A run that saved, and what it left in its data root as it left it, since a
/// later run over the same root changes what is there.
struct Saving {
    ran: testkit::Ran,
    root: std::path::PathBuf,
    saves: std::collections::BTreeMap<String, Vec<u8>>,
}

/// Every file in a data root's `savepath`, which is the install directory on
/// C:, by the name the profile keeps it under.
fn saves_in(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(root.join("profile/C/PROGRAM FILES/LEGO ISLAND"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|file| {
            let bytes = std::fs::read(file.path()).expect("a save reads");
            (file.file_name().to_string_lossy().into_owned(), bytes)
        })
        .collect()
}

/// A run to the credits, or past them by its budget, over a data root of its
/// own, empty to start with, done once into its slot.
fn saving(
    slot: &'static std::sync::OnceLock<Option<Saving>>,
    root: &str,
    host: impl FnOnce() -> Headless,
    cli: Cli,
    budget: u64,
) -> Option<&'static Saving> {
    slot.get_or_init(|| {
        if !scratch() {
            return None;
        }
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(root);
        let _ = std::fs::remove_dir_all(&root);
        let cli = Cli {
            data_root: Some(root.clone()),
            ..cli
        };
        let ran = testkit::run_on(host(), &ISLAND, &cli, budget);
        Some(Saving {
            saves: saves_in(&root),
            ran,
            root,
        })
    })
    .as_ref()
}

fn saved_by_hand() -> Option<&'static Saving> {
    static SAVED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    let cli = Cli {
        record: true,
        ..Cli::default()
    };
    saving(
        &SAVED,
        "first-save-by-hand",
        || quit().host(QUIT_FRAMES),
        cli,
        QUIT_BUDGET,
    )
}

fn save_route() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/first-save.route")
}

/// The flip a run had presented last when its log said something.
fn flip_of(ran: &testkit::Ran, said: &str) -> usize {
    let at = ran
        .log
        .find(said)
        .unwrap_or_else(|| panic!("`{said}` is logged"));
    ran.log[..at].matches("\nflip ").count() - 1
}

/// A name as the game saves it: seven letters, each numbered from A as 0, the
/// ones not used as -1.
fn name(letters: &str) -> Vec<u8> {
    let mut saved: Vec<i16> = letters.bytes().map(|c| i16::from(c - b'A')).collect();
    saved.resize(7, -1);
    saved
        .iter()
        .flat_map(|letter| letter.to_le_bytes())
        .collect()
}

/// The ticket's bar (#125): a route that saves leaves the game's saves in its
/// data root. The book is signed and the players are written; yes at the quit
/// door writes the game and the scores, and the credits roll.
#[test]
fn a_hand_signs_the_book_quits_and_leaves_the_saves_in_its_data_root() {
    let Some(saved) = saved_by_hand() else {
        return;
    };
    let ran = &saved.ran;
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way stops the run"
    );
    // The data root starts empty: at boot the game looks for its players and
    // its scores where it saves them, and finds neither.
    ran.assert_logged(r"file missing C:\Program Files\LEGO Island\Players.gsi")
        .assert_logged(r"file missing C:\Program Files\LEGO Island\History.gsi");
    let written = |save: &str| {
        flip_of(
            ran,
            &format!(r"file written C:\Program Files\LEGO Island\{save},"),
        )
    };
    assert_eq!(written("Players.gsi"), 7113, "pressing the tick");
    assert_eq!(written("History.gsi"), 10627, "letting go of yes");
    assert_eq!(written("G0.GS"), 10627);

    // One player, LEGO.
    let players = &saved.saves["PLAYERS.GSI"];
    assert_eq!(players[..2], [1, 0]);
    assert_eq!(players[2..], name("LEGO"));
    // The game's version, then LEGO's number, 0, the first act, and Pepper,
    // who is actor 1.
    assert_eq!(saved.saves["G0.GS"][..9], [0x0c, 0, 1, 0, 0, 0, 0, 0, 1]);
    // The scores, starting with the number the next player will be given.
    assert_eq!(saved.saves["HISTORY.GSI"][..2], [1, 0]);
    assert_eq!(saved.saves.len(), 3, "{:?}", saved.saves.keys());

    // The credits: black either side of their first picture.
    let credits = ran.host.frame(CREDITS).expect("kept");
    for x in [40, 600] {
        assert_eq!(pixel(credits, x, 300), [0, 0, 0], "the credits at {x}");
    }
}

/// The committed route is the hand's own, as its run recorded it.
#[test]
fn the_committed_first_save_is_what_the_hand_recorded() {
    let Some(saved) = saved_by_hand() else {
        return;
    };
    let recorded = lego_legacy_core::route::Route::load(&saved.ran.dir.join("route"))
        .expect("the run recorded a route");
    let committed = lego_legacy_core::route::Route::load(&save_route()).expect("it reads");
    assert_eq!(recorded, committed);
}

/// The determinism gate: the committed first save, replayed over a data root
/// of its own, ends on the machine the hand's run ended on and saves what it
/// saved, byte for byte.
#[test]
fn the_first_save_replays_to_the_machine_the_hand_made_and_saves_the_same() {
    static REPLAYED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(save_route()),
        ..Cli::default()
    };
    // The replay runs beside the hand's run rather than after it.
    let (hand, replay) = std::thread::scope(|scope| {
        let replay = scope.spawn(|| {
            saving(
                &REPLAYED,
                "first-save-replayed",
                || Headless::new().keeping(QUIT_FRAMES),
                cli,
                QUIT_BUDGET,
            )
        });
        (saved_by_hand(), replay.join().expect("the replay ran"))
    });
    let (Some(hand), Some(replay)) = (hand, replay) else {
        return;
    };
    hand.ran.assert_same_machine(&replay.ran);
    assert!(hand.saves == replay.saves, "and the same saves");
}

/// Enough for the credits to finish and the game to close itself and leave:
/// it is gone by 13.08 billion.
const CLOSED_BUDGET: u64 = 13_100_000_000;

/// The ticket's bar (#140): when its credits are over the game closes its own
/// window by posting itself `WM_CLOSE` (`LegoOmni::CloseMainWindow`). Its
/// window procedure answers by deleting the game, whose `Close` saves the game
/// and the scores once more, and then it leaves. The committed first save,
/// replayed past its end, ends with the game's own exit rather than at a wall,
/// having saved as it closed what it saved at the quit door.
#[test]
fn after_its_credits_the_game_closes_itself_and_saves_as_it_goes() {
    static CLOSED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(save_route()),
        ..Cli::default()
    };
    let (hand, closed) = std::thread::scope(|scope| {
        let closed = scope.spawn(|| {
            saving(
                &CLOSED,
                "first-save-closed",
                Headless::new,
                cli,
                CLOSED_BUDGET,
            )
        });
        (saved_by_hand(), closed.join().expect("the run ran"))
    });
    let (Some(hand), Some(closed)) = (hand, closed) else {
        return;
    };
    let ran = &closed.ran;
    ran.assert_exited(0);
    let close = "window procedure given message 0x0010";
    assert_eq!(
        ran.flip_before(close),
        17339,
        "the close, after the credits"
    );
    let (_, after) = ran.log.split_once(close).expect("the close is logged");
    for save in ["G0.GS", "History.gsi"] {
        assert!(
            after.contains(&format!(
                r"file written C:\Program Files\LEGO Island\{save},"
            )),
            "{save} is saved as the game closes"
        );
    }
    assert!(closed.saves == hand.saves, "the same saves as at the door");
}

/// LEGO signs the book and picks Pepper, and then the person closes the
/// window, in the Information Center, never having left by the quit door.
fn closed_by_hand() -> Hand {
    let mut hand = signed_as_pepper().wait(CLOSED_AT);
    hand.done.push((hand.flip, Input::Close));
    hand
}

/// The flip the person closes the window at, and enough for the game to
/// have left after it: it is gone by 5 billion.
const CLOSED_AT: u64 = 9000;
const CLOSED_BY_HAND_BUDGET: u64 = 5_010_000_000;

/// The ticket's bar (#140): the person's close reaches the game as Windows
/// delivers it, `WM_SYSCOMMAND` with `SC_CLOSE`. The game posts itself
/// `WM_CLOSE`, as its procedure answers the close button, and its `Close`
/// saves the game and the scores, which nothing else had saved, and it
/// leaves. The run ends with its exit, not where the host heard the close.
/// The close is in the route the run recorded, at the flip the person closed
/// at, and that route, replayed, ends the same way and saves the same.
#[test]
fn a_person_closing_the_window_is_told_and_the_game_saves_and_leaves() {
    static BY_HAND: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    static REPLAYED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    let Some(hand) = saving(
        &BY_HAND,
        "closed-by-hand",
        || closed_by_hand().host(&[]),
        Cli {
            record: true,
            ..Cli::default()
        },
        CLOSED_BY_HAND_BUDGET,
    ) else {
        return;
    };
    let ran = &hand.ran;
    ran.assert_exited(0);
    let close = format!("input flip {CLOSED_AT} Close");
    let (_, after) = ran.log.split_once(&close).expect("the close is delivered");
    assert!(after.contains("window procedure given message 0x0112"));
    assert!(after.contains("window procedure given message 0x0010"));
    for save in ["G0.GS", "History.gsi"] {
        assert!(
            ran.log
                .matches(&format!(
                    r"file written C:\Program Files\LEGO Island\{save},"
                ))
                .count()
                == 1
                && after.contains(&format!(
                    r"file written C:\Program Files\LEGO Island\{save},"
                )),
            "{save} is saved once, as the game closes"
        );
    }
    // LEGO's game: the version, his number, 0, and the first act.
    assert_eq!(hand.saves["G0.GS"][..8], [0x0c, 0, 1, 0, 0, 0, 0, 0]);

    let recorded = ran.dir.join("route");
    let route = lego_legacy_core::route::Route::load(&recorded).expect("recorded");
    assert_eq!(route.at(CLOSED_AT), [Input::Close]);
    let Some(replay) = saving(
        &REPLAYED,
        "closed-replayed",
        Headless::new,
        Cli {
            route: Some(recorded),
            ..Cli::default()
        },
        CLOSED_BY_HAND_BUDGET,
    ) else {
        return;
    };
    hand.ran.assert_same_machine(&replay.ran);
    assert!(hand.saves == replay.saves, "and the same saves");
}

/// The next run over the same data root (#125): the book opened, LEGO picked
/// from the names already in it, and then a second player, ISLE, signing it.
fn again() -> Hand {
    Hand::new()
        .wait(IDLE_LOOP + 50)
        .click(BOOK)
        .wait(6700)
        .click(FIRST_NAME)
        .wait(8300)
        .click(BOOK)
        .wait(9500)
        .hold(KEY_I, 2)
        .hold(KEY_S, 2)
        .hold(KEY_L, 2)
        .hold(KEY_E, 2)
        .wait(9600)
        .click(TICK)
}

/// Enough for ISLE's signing to be saved, at flip 9613, and a few flips more.
const AGAIN_BUDGET: u64 = 4_490_000_000;

/// The ticket's bar (#125): the next run over the data root loads all three
/// saves, and each shows. The book opens with LEGO already in it, from the
/// players. Picked, LEGO's game is loaded, and Pepper is who the player is,
/// though nothing in this run picked him. A second name moves LEGO's game down
/// a slot, through `MoveFileA`, and is given the number the scores saved.
#[test]
fn the_next_run_loads_the_saves_and_a_second_name_moves_the_first_down_a_slot() {
    let Some(first) = saved_by_hand() else {
        return;
    };
    let cli = Cli {
        data_root: Some(first.root.clone()),
        ..Cli::default()
    };
    let ran = testkit::run_on(
        again().host(&[BOOK_OPEN, SIGNED]),
        &ISLAND,
        &cli,
        AGAIN_BUDGET,
    );
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way stops the run, a second name included"
    );
    for save in ["Players.gsi", "History.gsi"] {
        assert!(
            !ran.log.contains(&format!(
                r"file missing C:\Program Files\LEGO Island\{save}"
            )),
            "{save} is found"
        );
    }
    let at = |ran: &testkit::Ran, flip: u64, x: usize, y: usize| {
        pixel(ran.host.frame(flip).expect("kept"), x, y)
    };

    // Both runs have the book open at the same flip, after the same presses.
    // This one has LEGO's yellow letters on the row below the empty line;
    // the first run's book there was green, with nobody in it.
    let yellow = |[r, g, b]: [u8; 3]| r > 200 && g > 150 && b < 50;
    assert!(yellow(at(&ran, BOOK_OPEN, 346, 153)), "LEGO is in the book");
    assert!(!yellow(at(&first.ran, BOOK_OPEN, 346, 153)));

    // Back in the Information Center with LEGO's name on the board, Pepper's
    // portrait is framed in blue: he is who LEGO was. At the same flip the
    // first run had signed and picked nobody yet.
    let framed = |pixel: [u8; 3]| pixel == [0, 0, 255];
    assert!(framed(at(&ran, SIGNED, 304, 105)), "Pepper is picked");
    assert!(!framed(at(&first.ran, SIGNED, 304, 105)));

    // ISLE signs: LEGO's game moves to the second slot as it was saved, and
    // ISLE's is the first, numbered 1 by the scores the first run saved, in
    // the first act, with nobody picked.
    ran.assert_logged(
        r"file moved C:\Program Files\LEGO Island\G0.GS to C:\Program Files\LEGO Island\G1.GS",
    );
    let saves = saves_in(&first.root);
    assert!(saves["G1.GS"] == first.saves["G0.GS"], "LEGO's game");
    assert_eq!(saves["G0.GS"][..9], [0x0c, 0, 1, 0, 1, 0, 0, 0, 0]);
    let players = &saves["PLAYERS.GSI"];
    assert_eq!(players[..2], [2, 0]);
    assert_eq!(players[2..], [name("ISLE"), name("LEGO")].concat());
    assert_eq!(saves["HISTORY.GSI"][..2], [2, 0]);
}

/// Where the delivery presses (#137). Anywhere on the floor of the
/// Information Center, which clicks the intro and the Infomaniac's welcome
/// through. Nick Brick's portrait, the fourth over the map. The Pizzeria, its
/// red and white awning just over the handlebars, seen from the foot of the
/// plaza below the Information Center; the pizza Papa Brickolini holds out;
/// and the Information Center's tower, seen from the gas station.
const ANYWHERE: (i32, i32) = (600, 450);
const NICK: (i32, i32) = (372, 105);
const PIZZERIA: (i32, i32) = (330, 268);
const PIZZA: (i32, i32) = (290, 284);
const THE_TOWER: (i32, i32) = (363, 185);

/// One whole activity by hand (#137): Nick Brick takes a pizza from the
/// Pizzeria to Nubby at the gas station, comes back, and saves.
///
/// The book is signed as the ride signs it, but Nick is picked: each player's
/// pizza goes somewhere of their own (`PizzaMissionState`), and Pepper's goes
/// to the jail, past road works whose two workers and fences close the road
/// there from the town for as long as he carries it, where Nick's is a
/// straight road and a turn. Out of the door, a step, a turn to the bicycle, and the
/// road down to the foot of the plaza; a turn to the town, and the Pizzeria is
/// clicked from there: anywhere it is seen. Papa Brickolini brings the pizza
/// out, and clicking it while he holds it out takes the job, so the game puts
/// Nick on the skateboard beside the Pizzeria with the pizza on its
/// dashboard. East along the road with the key held down, north at the
/// crossroads, and into the gas station's road, where Nubby comes for it.
/// Then the tower of the Information Center, clicked from there, goes back
/// inside, and the quit door saves the game.
fn delivery() -> Hand {
    let mut hand = Hand::new();
    for flip in (98..800).step_by(100) {
        hand = hand.wait(flip).click(ANYWHERE);
    }
    hand.wait(949)
        .click(BOOK)
        .wait(2800)
        .hold(KEY_L, 2)
        .hold(KEY_E, 2)
        .hold(KEY_G, 2)
        .hold(KEY_O, 2)
        .wait(2900)
        .click(TICK)
        .wait(4300)
        .click(NICK)
        .wait(5800)
        .click(TO_THE_ELEVATOR)
        .wait(6800)
        .click(TO_THE_DOOR)
        .wait(7800)
        .click(DOOR)
        // Outside, turning a key's 8 degrees a frame.
        .wait(8800)
        .hold(KEY_UP, 40)
        .wait(8900)
        .hold(KEY_LEFT, 5)
        .wait(9100)
        .click(BIKE)
        .wait(9400)
        .hold(KEY_UP, 600)
        .wait(10050)
        .hold(KEY_RIGHT, 8)
        .wait(10120)
        .click(PIZZERIA)
        .wait(11884)
        .click(PIZZA)
        // On the skateboard, facing the sea: east, and north.
        .wait(12526)
        .hold(KEY_LEFT, 11)
        .press(KEY_UP)
        .wait(12825)
        .hold(KEY_LEFT, 11)
        .wait(13300)
        .release(KEY_UP)
        // Delivered, and facing the Information Center.
        .wait(14250)
        .hold(KEY_RIGHT, 11)
        .wait(14300)
        .click(THE_TOWER)
        .wait(14700)
        .click(QUIT_DOOR)
        .wait(15300)
        .click(YES)
}

/// The flips the delivery is looked at: on the bicycle; Papa Brickolini
/// holding the pizza out; on the skateboard; Nubby with the pizza in his
/// hands; back in the Information Center; and the credits.
const DOWN_TO_THE_PLAZA: u64 = 9700;
const HELD_OUT: u64 = 11880;
const SKATING: u64 = 12800;
const HANDED_OVER: u64 = 13500;
const BACK_INSIDE: u64 = 14600;
const LAST_CREDITS: u64 = 16000;
const DELIVERY_FRAMES: &[u64] = &[
    DOWN_TO_THE_PLAZA,
    HELD_OUT,
    SKATING,
    HANDED_OVER,
    BACK_INSIDE,
    LAST_CREDITS,
];

/// Enough for the credits to finish and the game to close itself: it is gone
/// by 10.66 billion.
const DELIVERY_BUDGET: u64 = 10_700_000_000;

fn delivered_by_hand() -> Option<&'static Saving> {
    static DELIVERED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    let cli = Cli {
        record: true,
        ..Cli::default()
    };
    saving(
        &DELIVERED,
        "pizza-delivery-by-hand",
        || delivery().host(DELIVERY_FRAMES),
        cli,
        DELIVERY_BUDGET,
    )
}

fn delivery_route() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/pizza-delivery.route")
}

/// Each player's pizza mission as the game saves it, Pepper, Mama, Papa, Nick
/// and Laura in turn: four numbers each, the last two the score of its latest
/// delivery and its best, 0 for none and 3 for red, the fastest.
fn pizza_missions(game: &[u8]) -> Vec<[i16; 4]> {
    let name = b"PizzaMissionState";
    let at = game
        .windows(name.len())
        .position(|w| w == name)
        .expect("the pizza missions are saved")
        + name.len();
    game[at..at + 40]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|m| [0, 2, 4, 6].map(|i| i16::from_le_bytes([m[i], m[i + 1]])))
        .collect()
}

/// The ticket's bar (#137): one activity from its start to its end by hand,
/// through the host, and saved. Nothing on the way stops the run: the game
/// ends it itself, closing after its credits. What it saved says the pizza got
/// there: Nick's delivery is red, inside a hundred declared seconds.
#[test]
fn a_hand_takes_nicks_pizza_to_the_gas_station_comes_back_and_saves() {
    let Some(delivered) = delivered_by_hand() else {
        return;
    };
    let ran = &delivered.ran;
    ran.assert_exited(0);
    let frame = |flip: u64| ran.host.frame(flip).expect("kept");
    let red = |[r, g, b]: [u8; 3]| r > 200 && g < 60 && b < 60;

    // Down the road to the plaza's foot under the island's sky, the bicycle's
    // red handlebars across the foot of the screen.
    let [r, g, b] = pixel(frame(DOWN_TO_THE_PLAZA), 320, 10);
    assert!(b > 200 && b > r + 100 && b > g + 30, "the sky");
    assert!(
        red(pixel(frame(DOWN_TO_THE_PLAZA), 320, 400)),
        "the bicycle"
    );
    // On the skateboard the pizza is on its dashboard; at the gas station it
    // is in Nubby's hands, and gone from the dashboard.
    assert!(
        red(pixel(frame(SKATING), 90, 360)),
        "the pizza on the dashboard"
    );
    assert!(
        red(pixel(frame(HANDED_OVER), 196, 276)),
        "the pizza with Nubby"
    );
    assert!(
        !red(pixel(frame(HANDED_OVER), 90, 360)),
        "and not on the dashboard"
    );
    // Back inside, the Information Center's plants either side of its stage;
    // after the quit door, the credits.
    for (x, y) in [(180, 342), (472, 343)] {
        let [a, green, b] = pixel(frame(BACK_INSIDE), x, y);
        assert!(green > a && green > b, "a plant at {x},{y}");
    }
    for x in [40, 600] {
        assert_eq!(pixel(frame(LAST_CREDITS), x, 300), [0, 0, 0], "the credits");
    }

    // The players when the book is signed, the game and the scores at the quit
    // door, and both again as the game closes.
    let written = |save: &str| {
        flip_of(
            ran,
            &format!(r"file written C:\Program Files\LEGO Island\{save},"),
        )
    };
    assert_eq!(written("Players.gsi"), 2913, "pressing the tick");
    assert_eq!(written("History.gsi"), 15327, "letting go of yes");
    assert_eq!(written("G0.GS"), 15327);
    assert_eq!(delivered.saves["PLAYERS.GSI"][2..], name("LEGO"));
    // LEGO's game, in the first act, with Nick, actor 4, as the player; his
    // pizza delivered red, and nobody else's taken.
    let game = &delivered.saves["G0.GS"];
    assert_eq!(game[..9], [0x0c, 0, 1, 0, 0, 0, 0, 0, 4]);
    let missions = pizza_missions(game);
    assert_eq!(missions[3][2..], [3, 3], "Nick's, red: {missions:?}");
    for other in [0, 1, 2, 4] {
        assert_eq!(missions[other][2..], [0, 0], "{missions:?}");
    }
}

/// The committed route is the hand's own, as its run recorded it.
#[test]
fn the_committed_delivery_is_what_the_hand_recorded() {
    let Some(delivered) = delivered_by_hand() else {
        return;
    };
    let recorded = lego_legacy_core::route::Route::load(&delivered.ran.dir.join("route"))
        .expect("the run recorded a route");
    let committed = lego_legacy_core::route::Route::load(&delivery_route()).expect("it reads");
    assert_eq!(recorded, committed);
}

/// The determinism gate: the committed delivery, replayed over an empty data
/// root of its own, ends on the machine the hand's run ended on, presents the
/// pizza handed over as it did, and saves what it saved, byte for byte.
#[test]
fn the_delivery_replays_to_the_machine_the_hand_made_and_saves_the_same() {
    static REPLAYED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(delivery_route()),
        ..Cli::default()
    };
    // The replay runs beside the hand's run rather than after it.
    let (hand, replay) = std::thread::scope(|scope| {
        let replay = scope.spawn(|| {
            saving(
                &REPLAYED,
                "pizza-delivery-replayed",
                || Headless::new().keeping(DELIVERY_FRAMES),
                cli,
                DELIVERY_BUDGET,
            )
        });
        (delivered_by_hand(), replay.join().expect("the replay ran"))
    });
    let (Some(hand), Some(replay)) = (hand, replay) else {
        return;
    };
    hand.ran.assert_same_machine(&replay.ran);
    assert!(
        hand.ran.host.frame(HANDED_OVER).map(|f| &f.pixels)
            == replay.ran.host.frame(HANDED_OVER).map(|f| &f.pixels),
        "including what it presented at flip {HANDED_OVER}"
    );
    assert!(hand.saves == replay.saves, "and the same saves");
}

/// The maintainer's own play (#126), recorded at a window on their MacBook and
/// signed off: the book signed as OO, up to the observation deck, Pepper
/// picked, and out on the bicycle to the beach, where they closed the window.
fn played_route() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/maintainer-first-play.route")
}

/// Pepper picked, his portrait framed in blue; and the last frame before the
/// window was closed, on the bicycle at the beach.
const PICKED: u64 = 8900;
const AT_THE_BEACH: u64 = 10666;

/// Where the maintainer closed the window: their run stopped at instruction
/// 3,341,815,783, flip 10667, 274,650 declared milliseconds in. The route was
/// recorded before a route kept the window closing (#140), so the replay is
/// held to that moment by its budget. The game's window procedure has since
/// been told its window is active and has the keyboard as Windows tells it
/// (#196), which takes its procedure 127 instructions, so the
/// same moment is 127 instructions later: a move the maintainer blessed on
/// 2026-10-22.
const PLAYED_BUDGET: u64 = 3_341_815_783 + 127;

/// The maintainer's play replayed over an empty data root of its own, done
/// once into its slot.
fn played(
    slot: &'static std::sync::OnceLock<Option<Saving>>,
    root: &str,
) -> Option<&'static Saving> {
    let cli = Cli {
        route: Some(played_route()),
        ..Cli::default()
    };
    saving(
        slot,
        root,
        || Headless::new().keeping(&[PICKED, AT_THE_BEACH]),
        cli,
        PLAYED_BUDGET,
    )
}

static PLAYED: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();

/// The ticket's bar (#126): a person's play is the committed route, as Hot
/// Wheels' first race is. Replayed, it ends where the maintainer's run ended on
/// their MacBook, at the same moment, 127 instructions later (#196), the same
/// flip and declared time, the budget exhausted where theirs was stopped by the
/// person. Their play began from an
/// empty profile, so a replay over an empty data root starts from what theirs
/// did: it changes nothing. Their window closing did not tell the game, then,
/// so the players are all it saved, as theirs were.
#[test]
fn the_maintainers_play_replays_to_where_they_closed_the_window() {
    let Some(played) = played(&PLAYED, "maintainer-first-play") else {
        return;
    };
    let ran = &played.ran;
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way stops the run\nlog:\n{}",
        ran.log
    );
    assert_eq!(
        (
            ran.finish.cursor,
            ran.finish.flips,
            ran.finish.declared_millis
        ),
        (PLAYED_BUDGET, 10667, 274_650),
        "where the maintainer closed the window"
    );
    let frame = |flip: u64| ran.host.frame(flip).expect("kept");

    // The book signed as OO, one player, and nothing else saved.
    assert_eq!(
        flip_of(
            ran,
            r"file written C:\Program Files\LEGO Island\Players.gsi,"
        ),
        1678,
        "pressing the tick"
    );
    assert_eq!(
        played.saves.keys().collect::<Vec<_>>(),
        ["PLAYERS.GSI"],
        "the players and nothing else"
    );
    assert_eq!(played.saves["PLAYERS.GSI"][..2], [1, 0], "one player");
    assert_eq!(played.saves["PLAYERS.GSI"][2..], name("OO"));
    // Pepper's portrait, the third, framed in blue, with the others in gold.
    assert_eq!(pixel(frame(PICKED), 304, 105), [0, 0, 255], "Pepper");
    // At the beach on the bicycle, its red stem under the handlebars, and the
    // sky the colour the observation deck left it, a light orange where the
    // island's own is blue.
    let [r, g, b] = pixel(frame(AT_THE_BEACH), 320, 410);
    assert!(r > 200 && g < 60 && b < 60, "the bicycle");
    let [r, _, b] = pixel(frame(AT_THE_BEACH), 320, 10);
    assert!(r > b + 60, "the sky the deck left");
}

/// The determinism gate: the maintainer's play twice over, each over an empty
/// data root of its own, ends on the same machine, presents the same beach and
/// saves the same players.
#[test]
fn the_maintainers_play_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<Saving>> = std::sync::OnceLock::new();
    // The second replay runs beside the first rather than after it.
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| played(&AGAIN, "maintainer-first-play-again"));
        (
            played(&PLAYED, "maintainer-first-play"),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.ran.assert_same_machine(&second.ran);
    assert!(
        first.ran.host.frame(AT_THE_BEACH).map(|f| &f.pixels)
            == second.ran.host.frame(AT_THE_BEACH).map(|f| &f.pixels),
        "including what it presented at flip {AT_THE_BEACH}"
    );
    assert!(first.saves == second.saves, "and the same saves");
}

/// Where the held hand holds the button: above the middle of the screen,
/// where nothing of the island's own is drawn to be pressed.
const HELD_AT: (i32, i32) = (320, 120);

/// The flip the held hand presses the button at, outside, and the flip it
/// lets go at.
const PRESSED: u64 = 12400;
const LET_GO: u64 = PRESSED + 100;

/// Out of the door, and the button pressed on the island and held there
/// without moving.
fn held() -> Hand {
    let mut hand = out_of_the_door().wait(12300).glide(HELD_AT).wait(PRESSED);
    hand.done.push((PRESSED, Input::Down(Button::Left)));
    hand.done.push((LET_GO, Input::Up(Button::Left)));
    hand.flip = LET_GO + 1;
    hand
}

/// Enough for the held hand to let go: flip 12505 is presented at 6.067
/// billion.
const HELD_BUDGET: u64 = 6_067_000_000;

/// A button held still on the island walks (#138). At the press the game
/// sets a one-second timer (`LEGO1.DLL` 0x1005cfca), and the timer's
/// `WM_TIMER`, a declared second later with the button still down and the
/// pointer not moved past the drag threshold, starts a drag, which is how a
/// person walks by holding the button. A declared second is sixty flips here,
/// counted from when the game takes the press. Before #138 no `WM_TIMER` was
/// sent and the island stood still under the held button until it was let
/// go.
#[test]
fn a_button_held_still_for_a_declared_second_starts_a_walk() {
    static HELD: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let still = PRESSED + 60;
    let Some(ran) = by_hand(&HELD, held, &[PRESSED + 5, still, LET_GO], HELD_BUDGET) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing the held hand does stops the run"
    );
    // One timer's message, a declared second after the press: the drag it
    // starts kills the timer, so there is no second.
    let timer = "window procedure given message 0x0113";
    assert_eq!(ran.log.matches(timer).count(), 1, "one WM_TIMER");
    let at = ran.flip_before(timer);
    assert!(
        (still..=still + 3).contains(&at),
        "WM_TIMER after flip {at}"
    );
    let frame = |flip: u64| ran.host.frame(flip).expect("kept");
    // Held for a second, the island has not moved; held on, it goes by.
    assert!(
        frame(PRESSED + 5).pixels == frame(still).pixels,
        "still for a second"
    );
    assert!(
        testkit::differing(frame(still), frame(LET_GO), (0, 0, 640, 480)) > 100_000,
        "the walk"
    );
}
