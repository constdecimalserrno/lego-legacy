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
use lego_legacy_core::roster::{self, Entry, Found};
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{hash, pe, testkit, Title};
use lego_racers::LEGO_RACERS as RACERS;

const ROOT: &str = RACERS.install_dir;
const BOOT_BUDGET: u64 = 100_000;

/// The boot path, run once and shared by the boot assertions and determinism gate.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &RACERS, &cli, &[], BOOT_BUDGET)
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
        .assert_logged("installed 56 files");
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
