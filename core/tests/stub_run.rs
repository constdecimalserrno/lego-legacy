//! The run seam, exercised end to end on the machine's own guest.
//!
//! Every route test the roster grows goes through this door: run a title
//! headless within an instruction budget, then assert what it presented, what
//! it logged, and how it ended.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::Input;
use lego_legacy_core::{stub, testkit, Title};

const TEST_TITLE: Title = Title::STUB;

/// Keep run directories in the target scratch root, never in the working tree.
fn scratch() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_stub_guest_presents_one_frame_and_exits_within_budget() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&TEST_TITLE, &cli, 1_000);

    ran.assert_exited(stub::EXIT_CODE)
        .assert_flips(1)
        .assert_logged("flip 0 cursor")
        .assert_logged("end exited 3");

    assert_eq!(ran.finish.cursor, stub::RETIRED);
    assert_eq!(ran.host.frames.len(), 1);

    let frame = &ran.host.frames[0];
    assert_eq!((frame.width, frame.height), (4, 4));
    assert_eq!(frame.pixels.len(), 4 * 4 * 4);
    assert_eq!(
        u32::from_le_bytes([
            frame.pixels[0],
            frame.pixels[1],
            frame.pixels[2],
            frame.pixels[3]
        ]),
        stub::EXIT_CODE,
        "the stub writes its counter into the surface before presenting"
    );
}

#[test]
fn a_budget_smaller_than_the_guest_stops_the_run_without_a_frame() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&TEST_TITLE, &cli, 5);

    assert_eq!(ran.finish.outcome.to_string(), "budget exhausted");
    ran.assert_flips(0);
    assert!(ran.host.frames.is_empty());
}

#[test]
fn a_title_with_no_disc_behind_it_refuses_and_names_what_it_looked_for() {
    scratch();
    let err = testkit::try_run_headless(&TEST_TITLE, &Cli::default(), 1_000)
        .expect_err("the stub guest comes off no disc, so asking for its disc must refuse");
    assert!(
        err.contains("no rip at"),
        "the refusal names the path: {err}"
    );
}

#[test]
fn the_run_directory_holds_an_append_only_log_that_names_its_format() {
    scratch();
    let cli = Cli {
        stub: true,
        trace: Trace::Insns,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&TEST_TITLE, &cli, 1_000);

    assert!(ran.dir.join("log").is_file());
    let mut lines = ran.log.lines();
    assert_eq!(lines.next(), Some(lego_legacy_core::runlog::MAGIC));
    assert_eq!(lines.next(), Some("title stub"));
    assert_eq!(
        ran.log
            .lines()
            .filter(|l| l.starts_with("insn cursor "))
            .count(),
        stub::RETIRED as usize,
        "--trace insns records every instruction retired"
    );
}

#[test]
fn the_declared_clock_follows_the_cursor_and_not_the_host() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let a = testkit::run_headless(&TEST_TITLE, &cli, 1_000);
    let b = testkit::run_headless(&TEST_TITLE, &cli, 1_000);
    assert_eq!(a.finish.cursor, b.finish.cursor);
    assert_eq!(a.finish.declared_millis, b.finish.declared_millis);
}

#[test]
fn input_is_delivered_at_the_flip_it_names_and_always_recorded() {
    scratch();
    let mut host = lego_legacy_core::host::headless::Headless::new();
    host.queue(vec![
        (0, Input::Down),
        (0, Input::Move { x: 320, y: 213 }),
        (99, Input::Up),
    ]);

    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_on(host, &TEST_TITLE, &cli, 1_000);

    ran.assert_exited(stub::EXIT_CODE)
        .assert_logged("input flip 0 Down")
        .assert_logged("input flip 0 Move { x: 320, y: 213 }");
    assert_eq!(
        ran.host.undelivered(),
        [(99, Input::Up)],
        "an event naming a flip the run never reached is kept, not lost"
    );
}
