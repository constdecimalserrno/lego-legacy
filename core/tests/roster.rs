//! What the launcher finds when it goes looking.

use lego_legacy_core::roster::{self, Entry, Found};
use lego_legacy_core::testkit;
use lego_legacy_core::title::EntryImage;

fn scratch() -> bool {
    testkit::rips(&roster::ROCK_RAIDERS.image)
}

#[test]
fn a_pressing_that_is_there_and_right_is_ready_to_start() {
    if !scratch() {
        return;
    }
    let found = roster::look_for(&roster::ROCK_RAIDERS);
    assert_eq!(
        found,
        Found::Runnable {
            pressing: "masterpiece-rock-raiders".to_owned()
        },
        "the Masterpiece pressing holds the unwrapped game"
    );
    assert!(found.to_string().contains("ready"));
}

#[test]
fn a_wrapped_pressing_is_named_rather_than_offered() {
    if !scratch() {
        return;
    }
    // The same title pointed at the pressing whose executable is wrapped.
    let wrapped = Entry {
        key: "lego-rock-raiders",
        name: "LEGO Rock Raiders",
        image: EntryImage {
            pressing: "rock-raiders",
            ..roster::ROCK_RAIDERS.image
        },
        red_book: None,
    };
    let found = roster::look_for(&wrapped);
    match &found {
        Found::Wrapped { scheme, pressing } => {
            assert_eq!(scheme, "SafeDisc");
            assert_eq!(pressing, "rock-raiders");
        }
        other => panic!("found {other} instead of naming the wrapper"),
    }
    assert!(found.to_string().contains("will not be run"));
}

#[test]
fn a_pressing_that_is_not_there_says_where_it_looked() {
    if !scratch() {
        return;
    }
    let absent = Entry {
        key: "lego-rock-raiders",
        name: "LEGO Rock Raiders",
        image: EntryImage {
            pressing: "no-such-pressing",
            ..roster::ROCK_RAIDERS.image
        },
        red_book: None,
    };
    match roster::look_for(&absent) {
        Found::Absent { looked_at } => {
            assert!(
                looked_at.contains("no-such-pressing"),
                "looked at {looked_at}"
            );
        }
        other => panic!("found {other} instead of saying where it looked"),
    }
}

#[test]
fn a_pressing_holding_different_bytes_is_refused_by_its_digest() {
    if !scratch() {
        return;
    }
    let patched = Entry {
        key: "lego-rock-raiders",
        name: "LEGO Rock Raiders",
        image: EntryImage {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..roster::ROCK_RAIDERS.image
        },
        red_book: None,
    };
    match roster::look_for(&patched) {
        Found::WrongBytes { found, .. } => {
            assert!(
                found.starts_with("62b67f5b"),
                "it says what it found: {found}"
            );
        }
        other => panic!("found {other} instead of refusing the bytes"),
    }
}

#[test]
fn every_title_on_the_roster_can_be_looked_for() {
    // A title joins the roster when its own crate does, so the launcher never
    // offers something that cannot be started.
    assert!(!roster::ROSTER.is_empty());
    for entry in roster::ROSTER {
        assert!(!entry.key.is_empty() && !entry.name.is_empty());
        assert_eq!(
            entry.image.sha256.len(),
            64,
            "{} names its image by bytes",
            entry.key
        );
        assert!(
            !entry.image.why.is_empty(),
            "{} says why that pressing",
            entry.key
        );
    }
}

/// Alpha Team's image is inside its pressing's cabinet rather than on the
/// disc, and the launcher has to read it from there to know the bytes (#53).
#[test]
fn an_image_inside_the_cabinet_is_found_and_hashed_like_any_other() {
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&roster::ALPHA_TEAM),
        Found::Runnable {
            pressing: "alpha-team".to_owned()
        }
    );
}

/// The same title pointed at an image the cabinet does not hold says where it
/// looked rather than starting something else (#53).
#[test]
fn an_image_the_cabinet_does_not_hold_is_absent() {
    if !scratch() {
        return;
    }
    let absent = Entry {
        key: "lego-alpha-team",
        name: "LEGO Alpha Team",
        image: EntryImage {
            path: "NoSuchThing.exe",
            ..roster::ALPHA_TEAM.image
        },
        red_book: None,
    };
    match roster::look_for(&absent) {
        Found::Absent { looked_at } => assert!(
            looked_at.contains("NoSuchThing.exe"),
            "looked at {looked_at}"
        ),
        other => panic!("found {other} instead of saying where it looked"),
    }
}
