//! LEGOLAND, run headless from the player's rip: the launcher finding the
//! Masterpiece pressing, its declared InstallShield 3 `main.z` tree and Joliet
//! `.res` access, and its boot to the exact first wall (#393).
//!
//! Disc tests need the player's own rip under `game-data/`. Without it the
//! disc tests print a skip line and pass, which is a gate that did not run
//! rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into
//! a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, hash, pe, testkit, Title};
use legoland::LEGOLAND;

const ROOT: &str = LEGOLAND.install_dir;
const BOOT_BUDGET: u64 = 100_000;

/// The boot path, run once and shared by the boot assertions and determinism gate.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &LEGOLAND, &cli, &[], BOOT_BUDGET)
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
fn the_entry_image_loads_from_main_z_and_reaches_its_first_wall() {
    let Some(ran) = boot() else {
        return;
    };
    ran.assert_logged("image Legoland.exe")
        .assert_logged("base 0x00400000")
        .assert_logged("entry 0x004a08cd")
        .assert_logged("imports 201")
        .assert_logged("installed 331 files")
        .assert_logged("wall VERSION.dll!GetFileVersionInfoSizeA from 0x00458845 cursor 15622");
    assert_eq!(ran.finish.cursor, 15_622);
    assert_eq!(ran.finish.flips, 0);
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
    let second = testkit::run_headless(&LEGOLAND, &cli, BOOT_BUDGET);
    first.assert_same_machine(&second);
}
