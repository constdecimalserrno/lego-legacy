//! LEGOLAND, run headless from the player's rip: the launcher finding the
//! Masterpiece pressing, its declared InstallShield 3 `main.z` tree and Joliet
//! `.res` access, its first-light route through `Player Details` -> `"ADA"` ->
//! `Title Screen` -> clean exit, determinism, and `LLSNAP v2` cold snapshot
//! restore parity (#393, #394).
//!
//! Disc tests need the player's own rip under `game-data/`. Without it the
//! disc tests print a skip line and pass, which is a gate that did not run
//! rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into
//! a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::route::Route;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, hash, pe, testkit, Title};
use legoland::LEGOLAND;
use std::path::PathBuf;

const ROOT: &str = LEGOLAND.install_dir;
const UNFLAGGED_BOOT_BUDGET: u64 = 50_000_000;
const FIRST_LIGHT_BUDGET: u64 = 2_000_000_000;

fn route_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("routes")
        .join(name)
}

/// Unflagged cold boot: loads `Legoland.exe`, reads PE `RT_VERSION`, mounts
/// `.res` volumes over Joliet, initializes DirectDraw/DirectInput/DirectSound,
/// and starts decoding `D:\lmi.avi` (#394).
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Off,
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &LEGOLAND,
        &cli,
        &[0, 1],
        UNFLAGGED_BOOT_BUDGET,
    )
}

/// First-light route (`--guest-args -nointro` until #395 adds IMA ADPCM 0x0011
/// for `Intro.avi`): boots to `Player Details`, creates profile `"ADA"`,
/// clicks `Accept`, renders `Title Screen`, and exits cleanly via
/// `Exit Game -> Yes` (#394).
fn first_light() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        guest_args: Some("-nointro".to_owned()),
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &LEGOLAND,
        &cli,
        &[5, 32, 70, 94],
        FIRST_LIGHT_BUDGET,
    )
}

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&LEGOLAND.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    // It lists the title seventh, after the six earlier crates on the roster.
    assert_eq!(
        roster::ROSTER.iter().position(|e| e.key == LEGOLAND.key),
        Some(6)
    );
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&roster::LEGOLAND),
        Found::Runnable {
            pressing: "masterpiece-legoland".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..LEGOLAND.image
        },
        ..LEGOLAND
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
fn a_pressing_that_is_not_the_declared_one_is_refused_by_its_own_digest() {
    if !scratch() {
        return;
    }
    const PATCHED: Title = Title {
        image: EntryImage {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..LEGOLAND.image
        },
        ..LEGOLAND
    };
    let err = testkit::try_run_headless(&PATCHED, &Cli::default(), 1_000)
        .expect_err("a title is the bytes it is made of");

    assert!(
        err.contains("c50865b6"),
        "the refusal names what it found: {err}"
    );
}

#[test]
fn its_disc_carries_the_label_joliet_volumes_and_absent_indeo_library() {
    assert_eq!(LEGOLAND.declared.absent_libraries.len(), 1);
    assert_eq!(LEGOLAND.declared.absent_libraries[0].0, "Ir50_32.dll");

    if !scratch() {
        return;
    }
    let iso = disc::open_pressing(LEGOLAND.image.disc_dir, LEGOLAND.image.pressing)
        .expect("the pressing opens");
    assert_eq!(iso.label, "LEGOLAND");

    let mut tree = testkit::tree(&LEGOLAND);
    for (name, len) in [
        ("Legoland.res", 16_424_086),
        ("Graphics1.res", 19_962_774),
        ("Graphics2.res", 120_357_911),
    ] {
        assert!(
            !tree.holds(&format!("{ROOT}\\{name}")),
            "{name} stays on D:\\ rather than installed locally"
        );
        assert_eq!(
            tree.lies(&format!("D:\\{name}")).map(|(_, l)| l),
            Some(len),
            "{name} is readable via Joliet at its declared size"
        );
    }
    assert!(
        tree.wall().is_none(),
        "Joliet `.res` lookups must not raise a primary-tree wall"
    );
}

#[test]
fn the_declared_tree_unpacks_main_z_and_verifies_legoland_exe() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&LEGOLAND);
    assert_eq!(tree.declared_files().len(), 331);

    let exe = tree
        .read(&format!("{ROOT}\\Legoland.exe"))
        .expect("Legoland.exe is unpacked from main.z");
    assert_eq!(exe.len(), 802_857);
    assert_eq!(hash::sha256_hex(&exe), LEGOLAND.image.sha256);

    let exe_pe = pe::inspect(&exe).expect("Legoland.exe is a 32-bit PE");
    assert_eq!(pe::wrapper_scheme(&exe_pe.sections), None);
    assert_eq!(exe_pe.preferred_base, 0x0040_0000);
    assert_eq!(exe_pe.entry, 0x000a_08cd);
    assert_eq!(exe_pe.size, 0x0043_6000);
    assert!(
        pe::version_resource(&exe).is_some(),
        "Legoland.exe carries an RT_VERSION resource"
    );

    for name in [
        "Lego.TTF",
        "strings\\stab.str",
        "IMusic\\Band19.bnd",
        "zbuffers\\balloonz.bnv",
    ] {
        assert!(
            tree.readable(&format!("{ROOT}\\{name}")),
            "{name} is unpacked from main.z"
        );
    }
}

#[test]
fn unflagged_boot_loads_fonts_resources_and_plays_lmi_avi() {
    let Some(ran) = boot() else {
        return;
    };
    ran.assert_logged("image Legoland.exe")
        .assert_logged("base 0x00400000")
        .assert_logged("entry 0x004a08cd")
        .assert_logged("imports 201")
        .assert_logged("installed 331 files")
        .assert_logged("gdi AddFontResourceA: Lego.ttf -> family \"Lego\"")
        .assert_logged("movie D:\\lmi.avi opened through AVIFile: 2 streams");
    assert!(
        ran.finish.flips >= 2,
        "unflagged boot should decode and present lmi.avi frames"
    );
}

#[test]
fn first_light_creates_ada_profile_reaches_title_screen_and_exits_cleanly() {
    let Some(ran) = first_light() else {
        return;
    };
    ran.assert_exited(0)
        .assert_flips(105)
        .assert_logged("gdi AddFontResourceA: Lego.ttf -> family \"Lego\"")
        .assert_logged("com: d2ac2890-b39b-11d1-8704-00600893b1bd is not registered")
        .assert_logged("movie AD_Blink.avi opened through AVIFile: 1 streams");

    let details = ran.host.frame(5).expect("Player Details flip 5 kept");
    let typed = ran.host.frame(32).expect("ADA typed flip 32 kept");
    let title = ran.host.frame(70).expect("Title Screen flip 70 kept");
    let exit_dlg = ran.host.frame(94).expect("Exit dialog flip 94 kept");
    for (name, frame) in [
        ("Player Details", details),
        ("ADA typed", typed),
        ("Title Screen", title),
        ("Exit dialog", exit_dlg),
    ] {
        assert_eq!((frame.width, frame.height), (640, 480), "{name} resolution");
        assert!(!frame.is_uniform(), "{name} renders non-uniform pixels");
    }
    assert_ne!(
        details.pixels, typed.pixels,
        "typing ADA visibly updates Player Details"
    );
    assert_ne!(
        typed.pixels, title.pixels,
        "Accept transitions from Player Details to Title Screen"
    );
    assert_ne!(
        title.pixels, exit_dlg.pixels,
        "Exit Game opens the Exit confirmation dialog"
    );
}

#[test]
fn first_light_replays_to_the_same_machine_every_time() {
    let Some(first) = first_light() else {
        return;
    };
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        guest_args: Some("-nointro".to_owned()),
        ..Cli::default()
    };
    let second = testkit::run_headless(&LEGOLAND, &cli, FIRST_LIGHT_BUDGET);
    first.assert_same_machine(&second);
}

fn assert_snapshot_round_trip(baseline: &testkit::Ran, label: &str, snap_flip: u64) {
    let route = Route::load(&route_path("first-light.route")).expect("load first-light.route");
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("legoland-snap-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);

    let mut host_a = Headless::new();
    let mut machine_a = Machine::new(&LEGOLAND, &mut host_a).expect("machine");
    machine_a.state.guest_args = "-nointro".to_owned();
    machine_a.load_title(&LEGOLAND).expect("load title");
    machine_a.route = Some(route.clone());
    machine_a.tree.set_profile(data_root.join("a"));
    let mut log_a = RunLog::create(&format!("legoland-snap-{label}-a"), Trace::Off).expect("log");
    let dir_a = log_a.dir().to_path_buf();
    while machine_a.flips() < snap_flip {
        assert!(
            machine_a
                .step(&mut host_a, &mut log_a, FIRST_LIGHT_BUDGET, true)
                .is_none(),
            "should reach {label} flip {snap_flip} without ending"
        );
    }
    let snap = machine_a.snapshot_v2().expect("capture LLSNAP v2");

    // Copy any profile files written before the snapshot into B's isolated
    // profile directory (cold restore semantics: guest disk + snapshot state).
    let prof_a = data_root.join("a");
    let prof_b = data_root.join("b");
    if prof_a.exists() {
        copy_dir_all(&prof_a, &prof_b).expect("copy profile for cold restore");
    }

    let mut host_b = Headless::new();
    let mut machine_b = Machine::new(&LEGOLAND, &mut host_b).expect("machine");
    machine_b.state.guest_args = "-nointro".to_owned();
    machine_b.load_title(&LEGOLAND).expect("load title");
    machine_b.route = Some(route);
    machine_b.tree.set_profile(prof_b);
    machine_b.restore_v2(&snap).expect("cold restore LLSNAP v2");
    let mut log_b = RunLog::create(&format!("legoland-snap-{label}-b"), Trace::Off).expect("log");
    let dir_b = log_b.dir().to_path_buf();
    let finish_b = machine_b.run(&mut host_b, &mut log_b, FIRST_LIGHT_BUDGET);

    assert_eq!(finish_b.outcome, baseline.finish.outcome, "{label} outcome");
    assert_eq!(finish_b.cursor, baseline.finish.cursor, "{label} cursor");
    assert_eq!(finish_b.flips, baseline.finish.flips, "{label} flips");
    assert_eq!(
        finish_b.declared_millis, baseline.finish.declared_millis,
        "{label} declared_millis"
    );
    assert_eq!(machine_b.state_digest(), baseline.digest, "{label} digest");
    host_a.audio_blocks.extend(host_b.audio_blocks);
    assert_eq!(
        host_a.audio_blocks, baseline.host.audio_blocks,
        "{label} mixed PCM audio before + after LLSNAP v2 restore is bit-exact"
    );

    drop(log_a);
    drop(log_b);
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
    let _ = std::fs::remove_dir_all(&data_root);
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

#[test]
fn first_light_player_details_and_title_screen_snapshots_round_trip_bit_exact() {
    let Some(baseline) = first_light() else {
        return;
    };
    for (label, snap_flip) in [("player-details", 20u64), ("title-screen", 70u64)] {
        assert_snapshot_round_trip(baseline, label, snap_flip);
    }
}
