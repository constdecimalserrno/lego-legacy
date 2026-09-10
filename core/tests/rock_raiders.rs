//! LEGO Rock Raiders, from the disc to its first import.
//!
//! These need the player's own rips under `game-data/`. Without them the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, testkit, Title};

/// The title as its own crate declares it.
const ROCK_RAIDERS: Title = Title {
    key: "lego-rock-raiders",
    flip_cost: FlipCost::Free,
    image: EntryImage {
        disc_dir: "rock-raiders",
        pressing: "masterpiece-rock-raiders",
        path: "EXE/LEGORR.EXE",
        sha256: "62b67f5ba656c71412764d343c6a955d218e3b61f0e0f4758ad6086e971e64d4",
        why: "the Masterpiece pressing ships the game unwrapped (ADR-0006)",
    },
    red_book: Some("rock-raiders"),
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
};

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

    // The first thing this title's C runtime asks the machine for.
    ran.assert_wall("KERNEL32.dll!GetVersion")
        .assert_logged("imports 145")
        .assert_logged("shim KERNEL32.dll!GetVersion");

    assert!(
        ran.finish.cursor > 0 && ran.finish.cursor < 100,
        "the run should reach its first import within a few dozen instructions, not {}",
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
