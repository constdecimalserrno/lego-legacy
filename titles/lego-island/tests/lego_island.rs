//! LEGO Island, run headless from the player's rip: the launcher finding it
//! or refusing it, its declared tree, its boot, and its committed routes to
//! first light and to its Information Center's idle loop, what the second is
//! heard to play, and each replayed to the same machine every time; and a
//! synthetic hand driving it through the host, out of the Information Center
//! and away on a bike, and the route its run recorded.
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
/// `poll_input` hands the machine what was done since the last flip, and not
/// through a route (#124). A route replaces the hand, and a whole class of
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
const KEY_O: u32 = 0x18;
const KEY_G: u32 = 0x22;
const KEY_L: u32 = 0x26;
const KEY_UP: u32 = 0xc8;
const KEY_LEFT: u32 = 0xcb;

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

/// Out of the Information Center and onto the island, and away on a bike
/// (#124). The island will not let anyone out who has not signed its book
/// and picked who they are: the revolving door checks both (`LEGO1.DLL`
/// 0x10037b35), and a new name in the book picks nobody.
fn ride() -> Hand {
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
        .wait(10000)
        .click(TO_THE_ELEVATOR)
        .wait(11000)
        .click(TO_THE_DOOR)
        .wait(12000)
        .click(DOOR)
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
