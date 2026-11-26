//! LEGO Racers, run headless from the player's rip: the launcher finding the
//! Masterpiece pressing or refusing the SafeDisc-wrapped 1999 original, its
//! declared cabinet install tree, and its boot as far as the machine carries
//! it (#369).
//!
//! Disc tests need the player's own rips under `game-data/`. Without them the
//! disc tests print a skip line and pass, which is a gate that did not run
//! rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into
//! a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::roster::{self, Entry, Found};
use lego_legacy_core::route::Route;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{hash, pe, testkit, Title};
use lego_racers::LEGO_RACERS as RACERS;
use std::path::PathBuf;

const ROOT: &str = RACERS.install_dir;
const BOOT_BUDGET: u64 = 100_000;
const FIRST_LIGHT_BUDGET: u64 = 200_000_000;

fn route_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("routes")
        .join(name)
}

/// The default unflagged boot path, run once and shared by the boot assertions
/// and determinism gate (stops at DirectShow `CLSID_FilterGraph` until #372).
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &RACERS, &cli, &[], BOOT_BUDGET)
}

/// The `-novideo` first-light route to Main Menu and clean exit (#370).
fn first_light() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        guest_args: Some("-novideo".to_owned()),
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &RACERS,
        &cli,
        &[60, 300, 330],
        FIRST_LIGHT_BUDGET,
    )
}

/// LEGO Racers pointed at its original 1999 pressing, whose cabinet-packed
/// executable is wrapped in SafeDisc v1 (ADR-0006), still declaring the bytes
/// of the Masterpiece's (#369).
const RACERS_WRAPPED: Title = Title {
    image: testkit::LEGO_RACERS_WRAPPED,
    ..RACERS
};

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&RACERS.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    // It lists the title sixth, after the five earlier crates on the roster.
    assert_eq!(
        roster::ROSTER.iter().position(|e| e.key == RACERS.key),
        Some(5)
    );
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&roster::LEGO_RACERS),
        Found::Runnable {
            pressing: "masterpiece-racers".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..RACERS.image
        },
        ..RACERS
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
fn a_wrapped_image_is_named_and_never_run() {
    runs();
    if !testkit::rips(&RACERS_WRAPPED.image) {
        return;
    }
    let wrapped_entry = Entry {
        image: testkit::LEGO_RACERS_WRAPPED,
        ..roster::LEGO_RACERS
    };
    assert_eq!(
        roster::look_for(&wrapped_entry),
        Found::Wrapped {
            pressing: "racers".to_owned(),
            scheme: "SafeDisc".to_owned(),
        }
    );

    let err = testkit::try_run_headless(&RACERS_WRAPPED, &Cli::default(), 100_000)
        .expect_err("a SafeDisc wrapper is refused");

    assert!(
        err.contains("SafeDisc"),
        "the refusal names the scheme: {err}"
    );
    assert!(
        err.contains(&format!("{}racers.iso", std::path::MAIN_SEPARATOR)),
        "and the image it is in: {err}"
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
            ..RACERS.image
        },
        ..RACERS
    };
    let err = testkit::try_run_headless(&PATCHED, &Cli::default(), 1_000)
        .expect_err("a title is the bytes it is made of");

    assert!(
        err.contains("dcaea5ff"),
        "the refusal names what it found: {err}"
    );
}

#[test]
fn the_declared_tree_unpacks_the_masterpiece_cabinet() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&RACERS);
    assert_eq!(tree.declared_files().len(), 56);

    let exe = tree
        .read(&format!("{ROOT}\\LEGORacers.exe"))
        .expect("LEGORacers.exe is unpacked from DATA1.CAB");
    assert_eq!(exe.len(), 987_222);
    assert_eq!(hash::sha256_hex(&exe), RACERS.image.sha256);
    let exe_pe = pe::inspect(&exe).expect("LEGORacers.exe is a 32-bit PE");
    assert_eq!(pe::wrapper_scheme(&exe_pe.sections), None);
    assert_eq!(exe_pe.preferred_base, 0x0040_0000);
    assert_eq!(exe_pe.entry, 0x000b_ef2b);
    assert_eq!(exe_pe.size, 0x000f_f000);

    let goldp = tree
        .read(&format!("{ROOT}\\GolDP.dll"))
        .expect("GolDP.dll is unpacked from DATA1.CAB");
    assert_eq!(goldp.len(), 413_696);
    assert_eq!(
        hash::sha256_hex(&goldp),
        "cc0120eb92b1881a42ea0dcbb0a7824e0095cac0acf5d69878896005f345ea62"
    );
    let dll_pe = pe::inspect(&goldp).expect("GolDP.dll is a 32-bit PE");
    assert_eq!(pe::wrapper_scheme(&dll_pe.sections), None);
    assert_eq!(dll_pe.preferred_base, 0x1000_0000);
    assert_eq!(dll_pe.entry, 0x0004_9f97);
    assert_eq!(dll_pe.size, 0x0006_9000);

    let jam = tree
        .read(&format!("{ROOT}\\LEGO.JAM"))
        .expect("LEGO.JAM is unpacked from DATA1.CAB");
    assert_eq!(jam.len(), 19_642_024);
    assert_eq!(&jam[..4], b"LJAM");
    assert_eq!(
        hash::sha256_hex(&jam),
        "eb38feb756887e3c8ed087c38faf52302cbf427f7a753c989d8bda86724ac27d"
    );

    for name in ["theme.tun", "lmicmp.avi", "HVSCmp.avi", "introcmp.avi"] {
        assert!(
            tree.readable(&format!("{ROOT}\\{name}")),
            "{name} is declared in the cabinet"
        );
    }
}

#[test]
fn the_entry_image_loads_from_the_cabinet_and_starts_calling() {
    let Some(ran) = boot() else {
        return;
    };
    ran.assert_logged("image LEGORacers.exe")
        .assert_logged("base 0x00400000")
        .assert_logged("imports 147")
        .assert_logged("installed 56 files")
        .assert_logged("ole32.dll!CoCreateInstance: class e436ebb3-524f-11ce-9f53-0020af0ba770");
    assert!(
        ran.finish.cursor > 100,
        "the run should get past its own entry code, not stop at {}",
        ran.finish.cursor
    );
}

#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else {
        return;
    };
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    let second = testkit::run_headless(&RACERS, &cli, BOOT_BUDGET);
    first.assert_same_machine(&second);
}

#[test]
fn first_light_reaches_main_menu_and_exits_cleanly() {
    let Some(ran) = first_light() else {
        return;
    };
    ran.assert_exited(0)
        .assert_flips(331)
        .assert_logged("load library GolDP.DLL")
        .assert_logged("d3d3 device at")
        .assert_logged("winmm timer 1:")
        .assert_logged("winmm timer 1 killed");

    // Flip 60 is the LEGO Media splash; flips 300 and 330 are the animated 3D
    // Captain Redbeard Main Menu. All three are non-uniform, and the 3D scene
    // changes from splash -> menu and animates across Main Menu flips.
    let splash = ran.host.frame(60).expect("flip 60 kept");
    let menu_start = ran.host.frame(300).expect("flip 300 kept");
    let menu_end = ran.host.frame(330).expect("flip 330 kept");
    assert_eq!((menu_start.width, menu_start.height), (640, 480));
    assert!(!splash.is_uniform(), "splash frame 60 is non-uniform");
    assert!(
        !menu_start.is_uniform(),
        "main menu flip 300 is non-uniform"
    );
    assert!(!menu_end.is_uniform(), "main menu flip 330 is non-uniform");
    assert_ne!(
        splash.pixels, menu_start.pixels,
        "splash transitions cleanly into main menu"
    );
    assert_ne!(
        menu_start.pixels, menu_end.pixels,
        "3D main menu animates across flips 300..330"
    );
}

#[test]
fn first_light_replays_to_the_same_machine_every_time() {
    let Some(first) = first_light() else {
        return;
    };
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        guest_args: Some("-novideo".to_owned()),
        ..Cli::default()
    };
    let second = testkit::run_headless(&RACERS, &cli, FIRST_LIGHT_BUDGET);
    first.assert_same_machine(&second);
}

#[test]
fn first_light_main_menu_snapshot_round_trips_bit_exact() {
    let Some(baseline) = first_light() else {
        return;
    };
    let route = Route::load(&route_path("first-light.route")).expect("load first-light.route");
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);
    let profile_a = data_root.join("profile-a");
    let profile_b = data_root.join("profile-b");

    // 1. Run to flip 305 (live on Main Menu) and capture LLSNAP v2.
    let mut host_a = Headless::new();
    let mut machine_a = Machine::new(&RACERS, &mut host_a).expect("machine");
    machine_a.load_title(&RACERS).expect("load title");
    machine_a.state.guest_args = "-novideo".to_owned();
    machine_a.route = Some(route.clone());
    machine_a.tree.set_profile(profile_a);
    let mut log_a = RunLog::create("lego-racers-snap-a", Trace::Off).expect("log");
    let dir_a = log_a.dir().to_path_buf();
    while machine_a.flips() < 305 {
        assert!(
            machine_a.tick(&mut host_a, &mut log_a).is_none(),
            "should reach flip 305 without ending"
        );
    }
    let snap = machine_a.snapshot_v2().expect("capture LLSNAP v2");

    // 2. Cold-restore into a fresh Machine and finish to flip 330 close.
    let mut host_b = Headless::new();
    let mut machine_b = Machine::new(&RACERS, &mut host_b).expect("machine");
    machine_b.load_title(&RACERS).expect("load title");
    machine_b.state.guest_args = "-novideo".to_owned();
    machine_b.route = Some(route);
    machine_b.tree.set_profile(profile_b);
    machine_b.restore_v2(&snap).expect("cold restore LLSNAP v2");
    let mut log_b = RunLog::create("lego-racers-snap-b", Trace::Off).expect("log");
    let dir_b = log_b.dir().to_path_buf();
    let finish_b = machine_b.run(&mut host_b, &mut log_b, FIRST_LIGHT_BUDGET);

    assert_eq!(finish_b.outcome, baseline.finish.outcome);
    assert_eq!(finish_b.cursor, baseline.finish.cursor);
    assert_eq!(finish_b.flips, baseline.finish.flips);
    assert_eq!(finish_b.declared_millis, baseline.finish.declared_millis);
    assert_eq!(machine_b.state_digest(), baseline.digest);

    drop(log_a);
    drop(log_b);
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
    let _ = std::fs::remove_dir_all(data_root);
}
