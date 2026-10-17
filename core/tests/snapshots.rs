//! Snapshots: a cache a run can be resumed from, and never evidence.

use lego_legacy_core::cli::Cli;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::{stub, testkit, Title};

const STUB: Title = Title::STUB;

fn scratch() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

/// Build a stub machine and run it to a cursor.
fn machine_at(cursor: u64) -> (Machine, Headless) {
    scratch();
    let mut host = Headless::new();
    let mut machine = Machine::new(&STUB, &mut host).expect("a machine");
    machine.load_stub().expect("the stub loads");
    let mut log =
        lego_legacy_core::runlog::RunLog::create("stub", lego_legacy_core::cli::Trace::Off)
            .expect("a run directory");
    while machine.clock.cursor() < cursor {
        if machine.tick(&mut host, &mut log).is_some() {
            break;
        }
    }
    (machine, host)
}

#[test]
fn resuming_from_the_middle_ends_where_running_straight_through_does() {
    // Run to a cursor, snapshot, then resume into a fresh machine and finish
    // there. The two must end identically: that is the whole promise a
    // snapshot makes.
    let (mut early, _host) = machine_at(6);
    let bytes = early.snapshot().expect("the machine snapshots");

    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let straight = testkit::run_headless(&STUB, &cli, 1_000);

    scratch();
    let mut host = Headless::new();
    let mut resumed = Machine::new(&STUB, &mut host).expect("a machine");
    resumed.load_stub().expect("the stub loads");
    resumed.resume(&bytes).expect("the snapshot is ours");
    let mut log =
        lego_legacy_core::runlog::RunLog::create("stub", lego_legacy_core::cli::Trace::Off)
            .expect("a run directory");
    let finish = resumed.run(&mut host, &mut log, 1_000);

    assert_eq!(
        finish.cursor, straight.finish.cursor,
        "the same instructions retired"
    );
    assert_eq!(
        finish.outcome, straight.finish.outcome,
        "and it ended the same way"
    );
    assert_eq!(
        resumed.state_digest(),
        straight.digest,
        "and the machine looks the same"
    );
}

/// The digest sees every page the guest can write: the main thread's whole
/// stack, not only what is above where it stands, and the thread block and
/// the process block above that (#98).
#[test]
fn the_digest_sees_the_whole_stack_and_the_blocks_above_it() {
    use lego_legacy_core::guest::GuestMem;
    use lego_legacy_core::machine::{PEB, STACK_BYTES, STACK_TOP, TIB};
    let (mut machine, _host) = machine_at(0);
    // Standing where a title's main thread starts. The stub's own surface is
    // at the bottom of the stack, so the stack is written a page above that,
    // and just below where it stands.
    machine.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
    for at in [
        STACK_TOP - STACK_BYTES + 0x1000,
        STACK_TOP - 17,
        TIB,
        PEB + 0xfff,
    ] {
        let before = machine.state_digest();
        machine.arena.write(at, &[1]).expect("it is mapped");
        assert_ne!(machine.state_digest(), before, "{at:#010x} is hashed");
    }
}

#[test]
fn a_snapshot_from_another_title_is_refused_by_name() {
    let (mut machine, _host) = machine_at(4);
    let bytes = machine.snapshot().expect("the machine snapshots");

    // The same bytes, offered to a machine running something else.
    const OTHER: Title = Title {
        key: "not-the-stub",
        ..Title::STUB
    };
    scratch();
    let mut host = Headless::new();
    let mut other = Machine::new(&OTHER, &mut host).expect("a machine");
    other.load_stub().expect("the stub loads");
    let err = other
        .resume(&bytes)
        .expect_err("this snapshot is not that machine's");
    let text = err.to_string();
    assert!(
        text.contains("different title"),
        "the refusal names which part: {text}"
    );
}

#[test]
fn something_that_is_not_a_snapshot_is_refused_before_anything_is_read() {
    scratch();
    let mut host = Headless::new();
    let mut machine = Machine::new(&STUB, &mut host).expect("a machine");
    machine.load_stub().expect("the stub loads");
    let err = machine
        .resume(b"this is not a snapshot")
        .expect_err("nor is it one");
    assert!(err.to_string().contains("not a snapshot"), "{err}");
}

#[test]
fn snapshots_are_written_on_the_cadence_that_was_asked_for() {
    scratch();
    let cli = Cli {
        stub: true,
        snapshot_every: Some(1),
        ..Cli::default()
    };
    let ran = testkit::run_headless(&STUB, &cli, 1_000);
    ran.assert_exited(stub::EXIT_CODE)
        .assert_logged("snapshot flip-1.snapshot");
    assert!(
        ran.dir.join("flip-1.snapshot").is_file(),
        "the snapshot is in the run directory"
    );
    // Taking them goes through the one run loop, so the run is the same
    // machine as one that took none (ADR-0005, #94).
    let none = Cli {
        snapshot_every: None,
        ..cli
    };
    let plain = testkit::run_headless(&STUB, &none, 1_000);
    assert_eq!(ran.finish, plain.finish, "the same run");
    assert_eq!(ran.digest, plain.digest, "and the machine looks the same");
}

#[test]
fn nothing_is_written_when_nothing_was_asked_for() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&STUB, &cli, 1_000);
    let snapshots = std::fs::read_dir(&ran.dir)
        .expect("the run directory reads")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".snapshot"))
        .count();
    assert_eq!(
        snapshots, 0,
        "scrubbability costs nothing until it is asked for"
    );
}

/// Only a wall writes a crash report. The stub guest never reaches one, so
/// what is held here is the other half: a run cut short by its budget, which
/// is not a wall, claims no crash.
#[test]
fn a_budget_running_out_writes_no_crash_report() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&STUB, &cli, 6);
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted
    );
    assert!(
        !ran.dir.join("crash").is_file(),
        "a budget running out is not a crash"
    );
}
