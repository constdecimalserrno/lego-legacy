//! LEGO Rock Raiders, from the disc to its first import.
//!
//! These need the player's own rips under `game-data/`. Without them the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::title::{Declared, EntryImage, WINDOWS_98_SE};
use lego_legacy_core::{disc, testkit, Title};

/// The title as its own crate declares it.
const ROCK_RAIDERS: Title = Title {
    key: "lego-rock-raiders",
    flip_cost: FlipCost::Refresh(60),
    image: EntryImage {
        disc_dir: "rock-raiders",
        pressing: "masterpiece-rock-raiders",
        path: "EXE/LEGORR.EXE",
        sha256: "62b67f5ba656c71412764d343c6a955d218e3b61f0e0f4758ad6086e971e64d4",
        why: "the Masterpiece pressing ships the game unwrapped (ADR-0006)",
    },
    red_book: Some("rock-raiders"),
    declared: Declared {
        windows: WINDOWS_98_SE,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "the title crate's own values",
    },
    install_dir: "C:\\LEGORR",
};

/// The same title pointed at the pressing whose executable is wrapped.
const ROCK_RAIDERS_WRAPPED: Title = Title {
    key: "lego-rock-raiders",
    flip_cost: FlipCost::Free,
    image: EntryImage {
        disc_dir: "rock-raiders",
        pressing: "rock-raiders",
        path: "EXE/LEGORR.EXE",
        sha256: "cf3ba432a382c2fa229333fd6f7663a0edcda4f9140813e5a8f2e5de2f860328",
        why: "the original pressing, whose executable is a SafeDisc wrapper",
    },
    red_book: Some("rock-raiders"),
    declared: Declared {
        windows: WINDOWS_98_SE,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "the title crate's own values",
    },
    install_dir: "C:\\LEGORR",
};

/// The boot path, run once and looked at by several tests.
///
/// It plays four and a half minutes of movie, retires two hundred million
/// instructions and unpacks a 79 MB archive on the way, which is worth doing
/// once rather than once per assertion.
fn boot() -> Option<&'static testkit::Ran> {
    use std::sync::OnceLock;
    static RAN: OnceLock<Option<testkit::Ran>> = OnceLock::new();
    RAN.get_or_init(|| {
        if !scratch() {
            return None;
        }
        let cli = Cli {
            trace: Trace::Shims,
            ..Cli::default()
        };
        Some(testkit::try_run_headless(&ROCK_RAIDERS, &cli, 200_000_000).expect("the rip loads"))
    })
    .as_ref()
}

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
    std::env::set_var("LEGO_LEGACY_GAME_DATA", &root);

    let present = disc::pressing_path("rock-raiders", "masterpiece-rock-raiders").is_file();
    if !present {
        let required = std::env::var("LEGO_LEGACY_REQUIRE_DISC").is_ok_and(|v| v != "0");
        assert!(
            !required,
            "LEGO_LEGACY_REQUIRE_DISC is set but the rips are absent"
        );
        eprintln!(
            "skipping: no rips under {} (rip your own discs there)",
            root.display()
        );
    }
    present
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
    // tree when the file is in the pressing after all.
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
    let mut iso = disc::open_pressing("rock-raiders", "masterpiece-rock-raiders")
        .expect("the pressing opens");
    let header = iso.read("DATA1.HDR").expect("the cabinet's header");
    let mut tree = lego_legacy_core::install::Tree::new();
    tree.set_cabinet(&header, "DATA1.CAB", ROCK_RAIDERS.install_dir)
        .expect("the cabinet reads");
    tree.set_disc(iso);
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
