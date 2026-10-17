//! LEGO Rock Raiders, from the disc to its first import.
//!
//! These need the player's own rips under `game-data/`. Without them the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::headless::{Headless, Presented};
use lego_legacy_core::testkit::differing;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, testkit, Title};
use lego_rock_raiders::ROCK_RAIDERS;

/// The same title pointed at the pressing whose executable is wrapped.
const ROCK_RAIDERS_WRAPPED: Title = Title {
    image: EntryImage {
        pressing: "rock-raiders",
        sha256: "cf3ba432a382c2fa229333fd6f7663a0edcda4f9140813e5a8f2e5de2f860328",
        why: "the original pressing, whose executable is a SafeDisc wrapper",
        ..ROCK_RAIDERS.image
    },
    ..ROCK_RAIDERS
};

/// The boot path, run once and looked at by several tests.
///
/// It plays four and a half minutes of movie, retires four hundred million
/// instructions and unpacks a 79 MB archive on the way, which is worth doing
/// once rather than once per assertion. Its sounds load before its scene is
/// built, and the scene's lights come at about 352 million.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &ROCK_RAIDERS, &cli, &[], 400_000_000)
}

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    testkit::rips(&ROCK_RAIDERS.image)
}

#[test]
fn the_entry_image_loads_from_the_disc_and_stops_at_its_first_import() {
    if !scratch() {
        return;
    }
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    let ran = testkit::try_run_headless(&ROCK_RAIDERS, &cli, 100_000).expect("the rip loads");

    // The image loads and the guest starts asking the machine for things.
    // What it asks for last moves as the boundary grows, so what is held here
    // is that it got in and started calling, not where it stopped.
    ran.assert_logged("imports 145")
        .assert_logged("call KERNEL32.dll!GetVersion");

    assert!(
        ran.finish.cursor > 100,
        "the run should get past its own entry code, not stop at {}",
        ran.finish.cursor
    );
    assert!(
        ran.log.contains("base 0x00400000"),
        "the image lands where it asks to\nlog:\n{}",
        ran.log
    );
}

#[test]
fn a_wrapped_image_is_named_and_never_run() {
    if !scratch() {
        return;
    }
    let err = testkit::try_run_headless(&ROCK_RAIDERS_WRAPPED, &Cli::default(), 100_000)
        .expect_err("a SafeDisc wrapper is refused");

    assert!(
        err.contains("SafeDisc"),
        "the refusal names the scheme: {err}"
    );
    assert!(
        err.contains("never runs, decrypts or bypasses"),
        "the refusal says what it will not do: {err}"
    );
}

#[test]
fn a_pressing_that_is_not_the_declared_one_is_refused_by_its_own_digest() {
    if !scratch() {
        return;
    }
    const PATCHED: Title = Title {
        image: EntryImage {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..ROCK_RAIDERS.image
        },
        ..ROCK_RAIDERS
    };
    let err = testkit::try_run_headless(&PATCHED, &Cli::default(), 1_000)
        .expect_err("a title is the bytes it is made of");

    assert!(
        err.contains("62b67f5ba656c714"),
        "the refusal names what was found: {err}"
    );
    assert!(
        err.contains("0000000000000000"),
        "and what was declared: {err}"
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    scratch();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..ROCK_RAIDERS.image
        },
        ..ROCK_RAIDERS
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
fn the_disc_reads_in_the_order_the_image_records() {
    if !scratch() {
        return;
    }
    let mut iso = disc::open_pressing("rock-raiders", "masterpiece-rock-raiders")
        .expect("the pressing opens");
    assert_eq!(iso.label, "ROCKRAIDERS");

    let root = iso.list("").expect("the root directory reads");
    let names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"EXE") && names.contains(&"DATA"));
    // The order is the image's own, not sorted by us, and it is the same on
    // every run because it is written into the disc.
    let again = iso.list("").expect("the root directory reads twice");
    assert_eq!(root, again);

    let exe = iso.list("EXE").expect("a subdirectory reads");
    assert!(exe.iter().any(|e| e.name == "LEGORR.EXE"));

    let key = iso.read("DATA/CD.KEY").expect("a small file reads whole");
    assert_eq!(key.len(), 2, "the disc key is two bytes");
    assert!(iso.has("EXE/LEGORR.EXE").unwrap_or(false));
    assert!(!iso.has("EXE/NOTHERE.EXE").unwrap_or(true));
}

#[test]
fn the_boot_path_plays_its_movies_and_goes_on_to_build_its_scene() {
    let Some(ran) = boot() else { return };

    // Past both movies and the scene, loading its levels' objects with the
    // loading screen up, and nothing on the way it asked for that is not here.
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "the boot path should run to its budget without a wall"
    );
    ran.assert_logged("dinput created")
        // It opens its keyboard before it makes its window; the pointer comes
        // later, through the window itself.
        .assert_logged("dinput keyboard")
        // Its window, its sound, and the display its own dialog chose.
        .assert_logged("window class LegoRR")
        .assert_logged("dsound created")
        .assert_logged("display mode 640x480x16")
        .assert_logged("primary")
        // The retained-mode device, and the device underneath it, which is
        // what this title draws with.
        .assert_logged("d3drm created")
        .assert_logged("d3drm device")
        .assert_logged("d3d immediate device")
        // Its two movies, played to the end.
        .assert_logged("movie C:\\LEGORR\\Data\\avi\\lmi640.avi opened: 640x480 IV50, 279 frames")
        .assert_logged("movie C:\\LEGORR\\Data\\avi\\intro.avi opened: 320x200 IV50, 3821 frames");
    assert!(
        !ran.log.contains("drawing into surface 0x00000000"),
        "the device draws into real guest memory"
    );
    assert!(
        ran.finish.flips > 279 + 3821,
        "both movies are presented frame by frame, not {} flips",
        ran.finish.flips
    );
    // Four minutes and a half of movie is four minutes and a half of the
    // declared clock, however fast the host decodes it.
    assert!(
        ran.finish.declared_millis > (279 * 1000 / 15) + (3821 * 1000 / 15),
        "the movies take their own time on the declared clock, not {} ms",
        ran.finish.declared_millis
    );
}

#[test]
fn the_scene_is_built_through_retained_mode_and_drawn_through_the_device_under_it() {
    let Some(ran) = boot() else { return };

    // Its camera's view of the level, full screen, and the radar's, in the
    // corner of the panel: the viewports a player sees, made on the device.
    ran.assert_logged("d3drm viewport 640x480 at 0,0")
        .assert_logged("d3drm viewport 151x151 at 16,13")
        // The frames it builds its scene from are placed and turned.
        .assert_logged("IDirect3DRMFrame3::SetPosition")
        .assert_logged("IDirect3DRMFrame3::SetOrientation")
        .assert_logged("IDirect3DRMFrame3::AddChild")
        // Its lights, and the objects it draws itself when the scene renders.
        .assert_logged("IDirect3DRM3::CreateLightRGB")
        .assert_logged("IDirect3DRM3::CreateUserVisual")
        // The device underneath retained mode, asked for as DirectX 6's,
        // which is how it makes the material it draws with.
        .assert_logged("com IDirect3DDevice2 handed back IDirect3DDevice3")
        .assert_logged("IDirect3D3::CreateMaterial")
        .assert_logged("IDirect3DMaterial3::GetHandle");
    assert!(
        !ran.log.contains("IDirect3DDevice2 was asked for b0ab3b60"),
        "the DirectX 6 device is never refused"
    );
}

#[test]
fn the_screen_mode_is_the_one_the_titles_own_dialog_chose() {
    let Some(ran) = boot() else { return };

    // Its setup dialog lists the modes the declared machine has and selects
    // the first; a person pressing Enter takes it. The title set 320x200 when
    // its dialog was skipped, which is not what anyone playing it saw.
    ran.assert_logged("ddraw modes: 640x480x16")
        .assert_logged("dialog default button pressed")
        .assert_logged("dialog ended with 1")
        .assert_logged("display mode 640x480x16");
}

/// Where the first movie's first picture that is not all one colour lands.
fn first_movie_picture(budget: u64) -> Option<(usize, testkit::Ran)> {
    if !scratch() {
        return None;
    }
    let ran =
        testkit::try_run_headless(&ROCK_RAIDERS, &Cli::default(), budget).expect("the rip loads");
    let first = ran
        .host
        .frames
        .iter()
        .position(|f| !f.is_uniform())
        .unwrap_or_else(|| panic!("no picture within the budget\n{:?}", ran));
    Some((first, ran))
}

#[test]
fn the_first_movie_frame_is_presented_at_the_same_flip_on_every_run() {
    // The logo's movie starts sixty million instructions in and plays through
    // in a million more: the waiting is declared time, not instructions.
    let budget = 61_000_000;
    let Some((first, ran)) = first_movie_picture(budget) else {
        return;
    };
    let Some((again, _)) = first_movie_picture(budget) else {
        return;
    };
    assert_eq!(first, again, "the first picture moved between two runs");
    // It is the movie's picture: presented after the movie was set running,
    // not something drawn before it.
    let line = |needle: &str| {
        ran.log
            .lines()
            .position(|l| l.starts_with(needle))
            .unwrap_or_else(|| panic!("no log line starts `{needle}`"))
    };
    let running = line("movie C:\\LEGORR\\Data\\avi\\lmi640.avi running");
    assert!(
        line(&format!("flip {first} ")) > running,
        "flip {first} was presented before the movie started"
    );
    let frame = &ran.host.frames[first];
    assert_eq!((frame.width, frame.height), (640, 480));
    assert!(
        first < 279,
        "the picture is one of the logo's 279 frames, not flip {first}"
    );
}

#[test]
fn a_movie_skipped_stops_being_heard() {
    if !scratch() {
        return;
    }
    // Escape during the intro: the title lets go of the movie there, without
    // stopping it first, and goes on to load.
    const SKIPPED_AT: u64 = 1000;
    let route = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("skip-the-intro.route");
    std::fs::write(
        &route,
        format!(
            "#lego-legacy-route 1\nflip {SKIPPED_AT} key Escape down\nflip {} key Escape up\n",
            SKIPPED_AT + 2
        ),
    )
    .expect("the route writes");
    let cli = Cli {
        route: Some(route),
        ..Cli::default()
    };
    let ran = testkit::run_on(Headless::new(), &ROCK_RAIDERS, &cli, 150_000_000);

    ran.assert_logged("movie C:\\LEGORR\\Data\\avi\\intro.avi let go of");
    let heard = |from: u64, to: u64| {
        ran.host
            .audio_blocks
            .iter()
            .zip(&ran.host.audio_flips)
            .filter(|(_, flip)| (from..to).contains(*flip))
            .any(|(block, _)| block.iter().any(|s| *s != 0))
    };
    assert!(
        heard(SKIPPED_AT - 10, SKIPPED_AT),
        "the intro is heard until it is skipped"
    );
    // What was mixed before the skip reaches the host a few flips on; after
    // that, the loading that follows is silent.
    let after = SKIPPED_AT + 4;
    assert!(ran.finish.flips > after, "the run goes on past the skip");
    assert!(
        !heard(after, u64::MAX),
        "a skipped movie's sound stops with it"
    );
}

#[test]
fn a_click_skips_a_movie_the_way_escape_does() {
    if !scratch() {
        return;
    }
    // #67 asked whether a click gets past this title's intro. It already
    // does: the title takes a click during a movie exactly as it takes
    // Escape, so the machine has nothing to translate. Measured headless —
    // a click at flip 1000 lets go of `intro.avi` at flip 1001, where the
    // same run with nobody clicking plays it to flip 4134.
    const CLICKED_AT: u64 = 1000;
    let route = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("click-the-intro.route");
    std::fs::write(
        &route,
        format!(
            "#lego-legacy-route 1\nflip {CLICKED_AT} down\nflip {} up\n",
            CLICKED_AT + 2
        ),
    )
    .expect("the route writes");
    let cli = Cli {
        route: Some(route),
        ..Cli::default()
    };
    let ran = testkit::run_on(Headless::new(), &ROCK_RAIDERS, &cli, 150_000_000);

    ran.assert_logged("movie C:\\LEGORR\\Data\\avi\\intro.avi let go of");
    let before = ran
        .log
        .split_once("intro.avi let go of")
        .expect("the movie was let go of")
        .0;
    let flips_before = before
        .lines()
        .filter_map(|l| l.strip_prefix("flip "))
        .filter_map(|l| l.split_whitespace().next())
        .filter_map(|n| n.parse::<u64>().ok())
        .next_back()
        .unwrap_or(0);
    assert!(
        (CLICKED_AT..CLICKED_AT + 8).contains(&flips_before),
        "the click is what ended the movie, not its own end: flip {flips_before}"
    );
}

#[test]
fn a_movies_sound_is_mixed_whether_or_not_anyone_is_listening() {
    let Some((_, ran)) = first_movie_picture(61_000_000) else {
        return;
    };
    // `--audio` is off, and the mix still runs: the logo's sound track is in
    // the blocks the host was handed.
    assert!(!ran.host.audio_blocks.is_empty(), "no audio was mixed");
    assert!(
        ran.host
            .audio_blocks
            .iter()
            .any(|block| block.iter().any(|s| *s != 0)),
        "the mix was silence"
    );
}

#[test]
fn the_disc_in_the_declared_drive_is_the_pressing_itself() {
    let Some(ran) = boot() else { return };

    // The title checks its own disc by opening a file on D:, which the
    // declared machine answers out of the pressing in its drive.
    ran.assert_logged("call KERNEL32.dll!CreateFileA");
    assert!(
        !ran.log.contains("file missing D:\\Data\\cd.key"),
        "the disc check should find the pressing's own key file\nlog:\n{}",
        ran.log
    );
}

#[test]
fn a_second_flow_of_control_is_a_wall_rather_than_a_thread_that_never_runs() {
    // ADR-0005: a machine that grew a thread the host scheduled would stop
    // replaying. Nothing on the roster has needed one, so asking for one stops
    // the run and says so.
    assert!(
        lego_legacy_core::win32::find("KERNEL32.dll", "CreateThread").is_some(),
        "the entry point is declared, so that asking for it is a wall with a reason \
         rather than an import nobody wrote"
    );
}

#[test]
fn what_the_machine_declares_is_what_the_guest_reads_back() {
    if !scratch() {
        return;
    }
    // The declared machine is the title crate's own, and nothing in it comes
    // from the host.
    assert_eq!(ROCK_RAIDERS.declared.megabytes, 128);
    assert_eq!(ROCK_RAIDERS.declared.computer, "LEGO-LEGACY");
    assert_eq!(
        ROCK_RAIDERS.declared.windows & 0xff,
        4,
        "Windows 98 is major version four"
    );
    assert_eq!(
        (ROCK_RAIDERS.declared.windows >> 8) & 0xff,
        10,
        "and minor version ten"
    );
    assert_eq!(
        ROCK_RAIDERS.declared.windows >> 16,
        2222,
        "build 2222 is Second Edition"
    );
    assert!(
        !ROCK_RAIDERS.declared.why.is_empty(),
        "a declared value carries its reason"
    );
}

#[test]
fn the_titles_own_archives_are_on_the_declared_machines_disc() {
    let Some(ran) = boot() else { return };

    // Before the cabinet was read, this title opened its ten archives, found
    // none, and gave up. Now it finds the two the pressing ships and goes on
    // to load its own art out of them.
    for shipped in ["LegoRR0.wad", "LegoRR1.wad"] {
        assert!(
            !ran.log.contains(&format!("file missing {shipped}")),
            "{shipped} is in the pressing's cabinet and should have been found"
        );
    }
    // It also looks for files the pressing does not ship: eight more archives,
    // and, while it loads its objects, textures in an object's own folder
    // before it falls back to the shared ones. A miss is only a gap in the
    // tree when the file is in the pressing after all. The tree already holds
    // everything in the cabinet and on the disc, which is where a miss was
    // looked for, so the one place a shipped file can still hide is inside
    // the title's own archives.
    let shipped = wad_names();
    let gaps: Vec<&str> = ran
        .log
        .lines()
        .filter_map(|l| l.strip_prefix("file missing "))
        .filter(|path| !(2..10).any(|n| path.ends_with(&format!("LegoRR{n}.wad"))))
        .filter(|path| {
            let inside = path
                .strip_prefix("C:\\LEGORR\\Data\\")
                .or_else(|| path.strip_prefix("D:\\Data\\"))
                .unwrap_or(path);
            shipped.contains(&inside.to_ascii_lowercase())
        })
        .collect();
    assert!(gaps.is_empty(), "the tree does not hold: {gaps:?}");
    assert!(
        ran.finish.cursor > 10_000_000,
        "loading its data takes tens of millions of instructions, not {}",
        ran.finish.cursor
    );
}

/// Every file name in the pressing's archives, in lower case.
fn wad_names() -> std::collections::BTreeSet<String> {
    let mut tree = testkit::tree(&ROCK_RAIDERS);
    let mut names = std::collections::BTreeSet::new();
    for wad in ["LegoRR0.wad", "LegoRR1.wad"] {
        let bytes = tree
            .read(&format!("{}\\{wad}", ROCK_RAIDERS.install_dir))
            .expect("the archive is installed");
        // "WWAD", a count, then that many NUL-terminated names.
        let count = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mut rest = &bytes[8..];
        for _ in 0..count {
            let end = rest
                .iter()
                .position(|b| *b == 0)
                .expect("a terminated name");
            names.insert(String::from_utf8_lossy(&rest[..end]).to_ascii_lowercase());
            rest = &rest[end + 1..];
        }
    }
    names
}

/// The flip the title first presents its main menu at, which #12 calls Rock
/// Raiders' first light. #1's letter, the first frame the title presents after
/// its intro movies, is the loading screen before it at flip 4135.
const MENU: u64 = 4603;

/// Enough instructions to reach the menu and a couple of hundred flips of it:
/// the movies, the loading, and the menu waiting for a person. The loading
/// takes the title's own texture conversion, which runs now that its 8-bit
/// images are the 8-bit surfaces it asked for (#18), about 115 million
/// instructions, and its sounds, which it loads now that DirectSound keeps a
/// listener (#19), about 230 million more.
const FIRST_LIGHT_BUDGET: u64 = 8_010_000_000;

/// The flip the pointer has been on Start Game long enough to be drawn.
const HOVERED: u64 = 4615;

/// A few flips after the route has clicked in an empty part of the menu.
const CLICKED: u64 = 4630;

/// The whole of a 640x480 screen.
const SCREEN: (usize, usize, usize, usize) = (0, 0, 640, 480);

/// More than half the screen changes when the title puts up a new one.
const A_NEW_SCREEN: usize = 640 * 480 / 2;

/// Less than a twentieth changes when all that moved is the pointer and what
/// it was over.
const THE_SAME_SCREEN: usize = 640 * 480 / 20;

/// The committed route, replayed, keeping the frames the tests look at.
///
/// Two slots, so the determinism gate's two runs can go side by side with
/// the tests that look at one of them.
fn replay(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    following(
        slot,
        "first-light.route",
        &[MENU - 1, MENU, HOVERED, CLICKED],
        FIRST_LIGHT_BUDGET,
    )
}

static FIRST: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
static SECOND: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

#[test]
fn the_route_reaches_first_light_where_the_title_presents_its_menu() {
    let Some(ran) = replay(&FIRST) else { return };

    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way to the menu stops the run"
    );
    let menu = ran.host.frame(MENU).expect("the menu is presented");
    let loading = ran.host.frame(MENU - 1).expect("the frame before it");
    assert_eq!((menu.width, menu.height), (640, 480));
    assert!(
        !menu.is_uniform(),
        "the menu is a picture, not a cleared screen"
    );
    // A different screen, not the loading screen drawn again: most of the
    // picture changes between them.
    assert!(
        differing(loading, menu, SCREEN) > A_NEW_SCREEN,
        "flip {MENU} should be a new screen, not the loading screen"
    );
    // And from then on, the title is waiting in its menu: a flip every few
    // tens of thousands of instructions, where loading took millions between
    // flips.
    // ponytail: measured under 50,000 a flip in the menu; loading takes
    // millions.
    const WAITING: u64 = 200_000;
    let cursors: Vec<(u64, u64)> = ran
        .log
        .lines()
        .filter_map(|l| {
            let mut w = l.strip_prefix("flip ")?.split_whitespace();
            let flip = w.next()?.parse().ok()?;
            let cursor = w.nth(1)?.parse().ok()?;
            Some((flip, cursor))
        })
        // The flip after the menu comes up loads its ambience from its file,
        // and is not the menu waiting yet.
        .filter(|(flip, _)| (MENU + 1..=HOVERED).contains(flip))
        .collect();
    assert!(cursors.len() > 10, "the menu is presented flip after flip");
    for pair in cursors.windows(2) {
        assert!(
            pair[1].1 - pair[0].1 < WAITING,
            "flip {} took {} instructions, which is not a menu waiting",
            pair[1].0,
            pair[1].1 - pair[0].1
        );
    }
}

#[test]
fn a_persons_pointer_reaches_the_menu_and_lights_up_what_it_is_over() {
    let Some(ran) = replay(&FIRST) else { return };

    ran.assert_logged("input flip 4610 Move { x: 320, y: 222 }");
    let before = ran.host.frame(MENU).expect("the menu is kept");
    let after = ran.host.frame(HOVERED).expect("the hovered frame is kept");
    // Start Game is drawn in another colour with the pointer on it, and the
    // pointer itself has moved there from the corner.
    // Start Game's lettering, lit, covers more than this many pixels.
    const LIT: usize = 500;
    let start_game = (200, 200, 440, 240);
    assert!(
        differing(before, after, start_game) > LIT,
        "Start Game should light up under the pointer"
    );
    // The rest of the menu is left as it was.
    let quit_game = (200, 360, 440, 400);
    assert_eq!(
        differing(before, after, quit_game),
        0,
        "only what the pointer is over changes"
    );
}

#[test]
fn a_persons_click_reaches_the_title_through_its_window_procedure() {
    let Some(ran) = replay(&FIRST) else { return };

    // The pointer's move is read with GetCursorPos as well as sent; the button
    // arrives only as the window's messages.
    ran.assert_logged("window procedure given message 0x0201");
    ran.assert_logged("window procedure given message 0x0202");
    let menu = ran.host.frame(MENU).expect("the menu is kept");
    let clicked = ran.host.frame(CLICKED).expect("the clicked frame is kept");
    assert!(
        differing(menu, clicked, SCREEN) < THE_SAME_SCREEN,
        "a click on nothing leaves the title in its menu"
    );
}

/// The flip the level select has come in by, after Start Game is clicked.
const LEVEL_SELECT: u64 = 4700;

/// A flip in the fall of rocks between the menu and the level select.
const ROCKFALL: u64 = 4648;

/// A flip well after the level select came in, with nothing clicked since.
const WAITING_IN_THE_LEVEL_SELECT: u64 = 4750;

/// A flip with the level select scrolled part of the way down, the crosses on
/// two caverns passing its top edge.
const SCROLLED_LEVEL_SELECT: u64 = 4785;

/// Enough instructions for the level select and a couple of hundred flips of
/// it: the rockfall is fifty flips after the menu, and the level select waits
/// at about a quarter of a million instructions a flip, long enough for the
/// menu's ambience to go past the end of the buffer it is streamed through.
const LEVEL_SELECT_BUDGET: u64 = FIRST_LIGHT_BUDGET + 25_000_000;

/// A committed route, replayed once into its slot to a budget, keeping the
/// frames the tests look at.
fn following(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    route: &str,
    keep: &[u64],
    budget: u64,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("routes")
                .join(route),
        ),
        ..Cli::default()
    };
    testkit::replay(slot, scratch, &ROCK_RAIDERS, &cli, keep, budget)
}

/// The committed route to the level select.
fn to_the_level_select(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    following(
        slot,
        "level-select.route",
        &[
            MENU,
            ROCKFALL,
            LEVEL_SELECT,
            WAITING_IN_THE_LEVEL_SELECT,
            SCROLLED_LEVEL_SELECT,
        ],
        LEVEL_SELECT_BUDGET,
    )
}

static LEVEL_SELECT_FIRST: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
static LEVEL_SELECT_SECOND: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// What share of a rectangle's pixels differ from the one to their right.
fn busy(frame: &Presented, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> f64 {
    let stride = frame.width as usize;
    let pixel = |x: usize, y: usize| &frame.pixels[(y * stride + x) * 4..(y * stride + x) * 4 + 3];
    let pairs = (y0..y1).flat_map(|y| (x0..x1 - 1).map(move |x| (x, y)));
    let (differ, all) = pairs.fold((0, 0), |(d, a), (x, y)| {
        (d + usize::from(pixel(x, y) != pixel(x + 1, y)), a + 1)
    });
    differ as f64 / all.max(1) as f64
}

#[test]
fn clicking_start_game_brings_rocks_down_in_3d_and_the_level_select_up() {
    let Some(ran) = to_the_level_select(&LEVEL_SELECT_FIRST) else {
        return;
    };

    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing between the menu and the level select stops the run"
    );
    let menu = ran.host.frame(MENU).expect("the menu is kept");
    let rockfall = ran.host.frame(ROCKFALL).expect("the rockfall is kept");
    let level_select = ran
        .host
        .frame(LEVEL_SELECT)
        .expect("the level select is kept");
    let later = ran
        .host
        .frame(WAITING_IN_THE_LEVEL_SELECT)
        .expect("the level select, later, is kept");

    // A new screen, and the title waits in it.
    assert!(
        differing(menu, level_select, SCREEN) > A_NEW_SCREEN,
        "flip {LEVEL_SELECT} should be the level select, not the menu"
    );
    assert!(
        differing(level_select, later, SCREEN) < THE_SAME_SCREEN,
        "the level select stays up"
    );

    // On the way, rocks come down in front of the level select: where a big
    // one is, the frame is neither the menu nor the level select, and it is
    // textured, most neighbouring pixels differing where a flat or smoothly
    // shaded triangle would leave them the same.
    let big_rock = (120, 120, 240, 240);
    let area = 120 * 120;
    assert!(
        differing(level_select, rockfall, big_rock) > area * 9 / 10
            && differing(menu, rockfall, big_rock) > area * 9 / 10,
        "flip {ROCKFALL} should have a rock in front of both screens"
    );
    assert!(
        busy(rockfall, big_rock) > 0.3,
        "a rock is textured, and a flat colour is not"
    );
}

#[test]
fn scrolling_the_level_select_draws_the_crosses_passing_its_top_edge() {
    let Some(ran) = to_the_level_select(&LEVEL_SELECT_FIRST) else {
        return;
    };
    // The crosses' yellow tape in the top forty rows: a blit reaching above
    // the screen draws what of it is still on the screen (#27), where it once
    // drew nothing at all.
    let scrolled = ran
        .host
        .frame(SCROLLED_LEVEL_SELECT)
        .expect("the scrolled level select is kept");
    let tape = scrolled.pixels[..40 * scrolled.width as usize * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|[b, g, r, _]| *r > 180 && *g > 150 && *b < 80)
        .count();
    assert!(
        tape > 300,
        "flip {SCROLLED_LEVEL_SELECT}: {tape} pixels of tape"
    );
}

#[test]
fn the_sounds_the_title_plays_are_mixed_on_the_declared_clock() {
    let Some(ran) = to_the_level_select(&LEVEL_SELECT_FIRST) else {
        return;
    };
    // How loud each mixed block is, for the blocks mixed between two flips.
    let loudness = |from: u64, to: u64| -> Vec<f64> {
        ran.host
            .audio_blocks
            .iter()
            .zip(&ran.host.audio_flips)
            .filter(|(_, flip)| (from..to).contains(*flip))
            .map(|(block, _)| {
                let power: f64 = block.iter().map(|s| f64::from(*s).powi(2)).sum();
                (power / block.len().max(1) as f64).sqrt()
            })
            .collect()
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;

    // Flips before and after the menu's ambience starts, and while the rocks
    // come down.
    const LOADING_ENDS: u64 = 4590;
    const AMBIENCE_PLAYS: u64 = MENU + 10;
    const CLICKED_AT: u64 = 4620;
    const ROCKS_FALL: u64 = 4630;
    // Silence is below this; the ambience measured about 1300.
    const HEARD: f64 = 500.0;

    // The movies are long over by the end of the loading, and nothing plays.
    let loading = loudness(LOADING_ENDS, MENU);
    assert!(!loading.is_empty() && loading.iter().all(|r| *r == 0.0));
    // The menu loops its own ambience, from a DirectSound buffer.
    let menu = loudness(AMBIENCE_PLAYS, CLICKED_AT);
    assert!(
        !menu.is_empty() && menu.iter().all(|r| *r > HEARD),
        "the menu's ambience is heard: {menu:?}"
    );
    // And the rocks coming down are louder than it.
    let rockfall = loudness(ROCKS_FALL, LEVEL_SELECT);
    assert!(
        mean(&rockfall) > 2.0 * mean(&menu),
        "the rockfall is heard over the menu: {} against {}",
        mean(&rockfall),
        mean(&menu)
    );

    // The ambience is a minute and a half of sound streamed through a buffer
    // three seconds long, the title writing each next part into the part
    // already played (#23). Three seconds on, in the level select, what is
    // heard is the next part rather than the first three seconds again.
    const STREAM_BUFFER_FRAMES: usize = 3 * lego_legacy_core::sound::RATE as usize;
    let mix = ran.host.audio_blocks.concat();
    let block = ran.host.audio_blocks[0].len();
    let menu_at = block
        * ran
            .host
            .audio_flips
            .iter()
            .position(|flip| *flip >= AMBIENCE_PLAYS)
            .expect("mixed in the menu");
    // A fifth of a second of stereo samples from a point in the mix.
    let heard = |from: usize| mix.get(from..from + 8820).expect("mixed this far");
    let later = heard(menu_at + STREAM_BUFFER_FRAMES * 2);
    assert!(later.iter().any(|s| *s != 0), "the ambience is still heard");
    assert!(
        heard(menu_at) != later,
        "the ambience goes on past its buffer's first three seconds"
    );
}

#[test]
fn the_level_select_route_replays_to_the_same_machine_every_time() {
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| to_the_level_select(&LEVEL_SELECT_SECOND));
        (
            to_the_level_select(&LEVEL_SELECT_FIRST),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    assert_eq!(
        (
            first.finish.cursor,
            first.finish.flips,
            first.finish.declared_millis
        ),
        (
            second.finish.cursor,
            second.finish.flips,
            second.finish.declared_millis
        )
    );
    assert_eq!(first.digest, second.digest, "the machine ends the same way");
    for flip in [ROCKFALL, LEVEL_SELECT] {
        assert_eq!(
            first.host.frame(flip).map(|f| &f.pixels),
            second.host.frame(flip).map(|f| &f.pixels),
            "and presents flip {flip} the same way"
        );
    }
    assert!(
        first.host.audio_blocks == second.host.audio_blocks,
        "and mixes the same"
    );
}

#[test]
fn the_first_light_route_replays_to_the_same_machine_every_time() {
    // The second replay runs beside the first rather than after it.
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| replay(&SECOND));
        (
            replay(&FIRST),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    assert_eq!(first.finish.cursor, second.finish.cursor);
    assert_eq!(first.finish.flips, second.finish.flips);
    assert_eq!(
        first.finish.declared_millis, second.finish.declared_millis,
        "the same declared time"
    );
    assert_eq!(
        first.digest, second.digest,
        "and the machine ends the same way"
    );
    assert_eq!(
        first.host.frame(HOVERED).map(|f| &f.pixels),
        second.host.frame(HOVERED).map(|f| &f.pixels),
        "including what it presented"
    );
    assert!(
        first.host.audio_blocks == second.host.audio_blocks,
        "and what it mixed"
    );
}

/// A flip on the loading screen, after the cavern was clicked.
const LOADING_THE_MISSION: u64 = 4750;

/// A flip with the Mission Brief up over the level.
const MISSION_BRIEF: u64 = 5350;

/// A flip in the mission, the brief dismissed, its tutorial waiting.
const IN_THE_MISSION: u64 = 5450;

/// A minute of the mission, in flips.
const A_MINUTE_OF_THE_MISSION: u64 = 60 * 60;

/// Enough instructions for the mission's load, its brief, and a minute of it
/// playing, at about 1.4 million instructions a flip with its raiders at work.
const FIRST_MISSION_BUDGET: u64 = 14_500_000_000;

/// The committed route into the first mission.
fn to_the_first_mission(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    following(
        slot,
        "first-mission.route",
        &[LOADING_THE_MISSION, MISSION_BRIEF, IN_THE_MISSION],
        FIRST_MISSION_BUDGET,
    )
}

static FIRST_MISSION_FIRST: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
static FIRST_MISSION_SECOND: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

#[test]
fn choosing_the_first_mission_loads_it_and_plays_it_in_3d_with_its_red_book_audio() {
    let Some(ran) = to_the_first_mission(&FIRST_MISSION_FIRST) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing from the level select to a minute into the mission stops the run"
    );
    assert!(
        ran.finish.flips > IN_THE_MISSION + A_MINUTE_OF_THE_MISSION,
        "the budget reaches a minute into the mission: {} flips",
        ran.finish.flips
    );
    let loading = ran
        .host
        .frame(LOADING_THE_MISSION)
        .expect("the loading screen is kept");
    let brief = ran.host.frame(MISSION_BRIEF).expect("the brief is kept");
    let mission = ran.host.frame(IN_THE_MISSION).expect("the mission is kept");

    // Where the level's lava and water are, once the brief has gone: drawn in
    // 3D, textured, and nothing like the loading screen.
    let level = (250, 150, 450, 250);
    let area = 200 * 100;
    assert!(
        busy(mission, level) > 0.3,
        "flip {IN_THE_MISSION} should show the level, textured"
    );
    assert!(
        differing(loading, mission, level) > area * 9 / 10,
        "and not the loading screen"
    );
    // Behind the brief, above it, the level is already drawn: the cavern's
    // rock, which the loading screen does not have there.
    let above_the_brief = (160, 40, 520, 120);
    assert!(
        differing(loading, brief, above_the_brief) > 360 * 80 / 2,
        "flip {MISSION_BRIEF} should have the level behind its brief"
    );

    // The mission starts its Red Book audio, when the pressing is there to
    // play it from, and the drive says it is playing when the title asks.
    if disc::open_red_book("rock-raiders", "rock-raiders").is_ok() {
        ran.assert_logged("mci \"play cdaudio from");
        ran.assert_logged("mci \"status cdaudio mode\" answered Ok(\"playing\")");
    }
    let heard = ran
        .host
        .audio_blocks
        .iter()
        .zip(&ran.host.audio_flips)
        .filter(|(_, flip)| **flip > IN_THE_MISSION)
        .any(|(block, _)| block.iter().any(|s| *s != 0));
    assert!(heard, "the mission is heard");
}

#[test]
fn the_first_mission_route_replays_to_the_same_machine_every_time() {
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| to_the_first_mission(&FIRST_MISSION_SECOND));
        (
            to_the_first_mission(&FIRST_MISSION_FIRST),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    assert_eq!(
        (
            first.finish.cursor,
            first.finish.flips,
            first.finish.declared_millis
        ),
        (
            second.finish.cursor,
            second.finish.flips,
            second.finish.declared_millis
        )
    );
    assert_eq!(first.digest, second.digest, "the machine ends the same way");
    for flip in [MISSION_BRIEF, IN_THE_MISSION] {
        assert!(
            first.host.frame(flip).map(|f| &f.pixels) == second.host.frame(flip).map(|f| &f.pixels),
            "and presents flip {flip} the same way"
        );
    }
    assert!(
        first.host.audio_blocks == second.host.audio_blocks,
        "and mixes the same"
    );
}
