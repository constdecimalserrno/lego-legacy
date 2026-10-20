//! LEGO Island, run headless from the player's rip: the launcher finding it
//! or refusing it, its declared tree, and its boot as far as the machine
//! carries it, replayed to the same machine every time.
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

/// Enough instructions for both C runtimes to start and the game to read
/// its settings back, and no more: what the game asks for after that is the
/// next ticket's (#114).
const BOOT_BUDGET: u64 = 100_000;

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
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else { return };
    let Some(second) = ({
        static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
        let cli = Cli {
            trace: Trace::Shims,
            ..Cli::default()
        };
        testkit::replay(&AGAIN, scratch, &ISLAND, &cli, &[], BOOT_BUDGET)
    }) else {
        return;
    };
    first.assert_same_machine(second);
}
