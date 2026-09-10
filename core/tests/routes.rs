//! Routes, replayed and recorded, and the gate that says a run reproduces.

use lego_legacy_core::cli::Cli;
use lego_legacy_core::host::{headless::Headless, Input};
use lego_legacy_core::{stub, testkit, Title};
use std::path::PathBuf;

const STUB: Title = Title::STUB;

fn scratch() -> PathBuf {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

#[test]
fn a_route_is_delivered_at_the_flip_it_names() {
    let dir = scratch();
    let path = dir.join("delivered.route");
    std::fs::write(
        &path,
        "# the stub guest presents once\nflip 0 down\nflip 0 move 12 34\n",
    )
    .expect("the route writes");

    let ran = testkit::run_route(&STUB, &path, 1_000);
    ran.assert_exited(stub::EXIT_CODE)
        .assert_logged("input flip 0 Down")
        .assert_logged("input flip 0 Move { x: 12, y: 34 }")
        .assert_logged("route ");
}

#[test]
fn a_route_naming_a_flip_the_run_never_reaches_says_so() {
    let dir = scratch();
    let path = dir.join("too-long.route");
    std::fs::write(&path, "flip 0 down\nflip 99 up\n").expect("the route writes");

    let ran = testkit::run_route(&STUB, &path, 1_000);
    ran.assert_logged("route undelivered 1 events after flip 1");
}

#[test]
fn a_route_that_cannot_be_read_names_the_line() {
    let dir = scratch();
    let path = dir.join("broken.route");
    std::fs::write(&path, "flip 0 down\nflip 1 sideways\n").expect("the route writes");

    let cli = Cli {
        route: Some(path.clone()),
        stub: true,
        ..Cli::default()
    };
    let err = testkit::try_run_headless(&STUB, &cli, 1_000).expect_err("this route is not one");
    assert!(err.contains("line 2"), "the refusal names the line: {err}");
    assert!(
        err.contains("no such event"),
        "and what was wrong with it: {err}"
    );
}

#[test]
fn a_recorded_run_writes_a_route_that_replays_as_itself() {
    scratch();
    // A run that took input writes down what it took.
    let mut host = Headless::new();
    host.queue(vec![
        (0, Input::Down),
        (
            0,
            Input::Key {
                code: 0x01,
                down: true,
            },
        ),
    ]);
    let cli = Cli {
        stub: true,
        record: true,
        ..Cli::default()
    };
    let recorded = testkit::run_on(host, &STUB, &cli, 1_000);
    recorded.assert_exited(stub::EXIT_CODE);

    let written = recorded.dir.join("route");
    let text = std::fs::read_to_string(&written).expect("the recorder wrote a route");
    assert!(text.starts_with("#lego-legacy-route 1"), "route:\n{text}");
    assert!(text.contains("flip 0 down"), "route:\n{text}");
    assert!(text.contains("flip 0 key Escape down"), "route:\n{text}");

    // And replaying it produces the same run.
    let replayed = testkit::run_route(&STUB, &written, 1_000);
    assert_eq!(
        replayed.digest, recorded.digest,
        "a recording replays as itself"
    );
}

#[test]
fn the_same_route_twice_leaves_the_machine_in_the_same_state() {
    let dir = scratch();
    let path = dir.join("gate.route");
    std::fs::write(&path, "flip 0 move 5 7\nflip 0 down\n").expect("the route writes");

    let first = testkit::run_route(&STUB, &path, 1_000);
    let second = testkit::run_route(&STUB, &path, 1_000);

    assert_eq!(
        first.finish.cursor, second.finish.cursor,
        "the same instructions retired"
    );
    assert_eq!(
        first.finish.flips, second.finish.flips,
        "the same frames presented"
    );
    assert_eq!(
        first.digest, second.digest,
        "and the machine ends the same way"
    );
    assert_eq!(
        first.host.frames[0].pixels, second.host.frames[0].pixels,
        "including the pixels it presented"
    );
}

#[test]
fn a_run_that_replays_takes_no_input_from_the_person_at_the_keyboard() {
    let dir = scratch();
    let path = dir.join("only-this.route");
    std::fs::write(&path, "flip 0 up\n").expect("the route writes");

    // Someone leaning on the keyboard while a route replays changes nothing.
    let mut host = Headless::new();
    host.queue(vec![(0, Input::Down), (0, Input::Move { x: 999, y: 999 })]);
    let cli = Cli {
        stub: true,
        route: Some(path),
        ..Cli::default()
    };
    let ran = testkit::run_on(host, &STUB, &cli, 1_000);

    ran.assert_logged("input flip 0 Up");
    assert!(
        !ran.log.contains("999"),
        "what the person did while a route replayed did not reach the guest\nlog:\n{}",
        ran.log
    );
}
