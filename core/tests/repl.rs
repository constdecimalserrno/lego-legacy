//! The debugger, driven from a script.
//!
//! Everything the debugger prints is plain text with one fact per line, which
//! is what makes it checkable without a terminal.

use lego_legacy_core::cli::Cli;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Outcome;
use lego_legacy_core::{repl, stub, Title};

const STUB: Title = Title::STUB;

fn scratch() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

fn drive(script: &str) -> String {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let mut host = Headless::new();
    repl::transcript(&STUB, &cli, &mut host, script).expect("the stub guest loads")
}

#[test]
fn quitting_before_the_guest_stops_ends_the_run_as_a_quit() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let mut input = std::io::Cursor::new(b"step\nquit\n".to_vec());
    let finish = repl::drive(
        &STUB,
        &cli,
        &mut Headless::new(),
        &mut input,
        &mut Vec::new(),
    )
    .expect("the stub guest loads");
    assert_eq!(finish.outcome, Outcome::Quit, "not a window closing");
}

#[test]
fn stepping_moves_the_cursor_one_instruction_at_a_time() {
    let out = drive("step\nstep\nstep 3\nquit\n");
    let cursors: Vec<&str> = out
        .lines()
        .filter(|l| l.contains("cursor "))
        .map(|l| l.trim_start_matches("> "))
        .collect();
    // The line before any command is where the machine starts.
    assert!(
        cursors[0].starts_with("cursor 0"),
        "starts at zero: {:?}",
        cursors
    );
    assert!(
        cursors[1].starts_with("cursor 1"),
        "one step: {:?}",
        cursors
    );
    assert!(
        cursors[2].starts_with("cursor 2"),
        "two steps: {:?}",
        cursors
    );
    assert!(
        cursors[3].starts_with("cursor 5"),
        "three more: {:?}",
        cursors
    );
}

#[test]
fn the_registers_come_out_one_fact_per_line() {
    let out = drive("step 2\nregs\nquit\n");
    // Two instructions in, the stub has loaded both its counters.
    assert!(out.contains("eax 0x00000000"), "transcript:\n{out}");
    assert!(out.contains("ecx 0x00000003"), "transcript:\n{out}");
    assert!(
        out.lines().any(|l| l.starts_with("eip 0x00400")),
        "transcript:\n{out}"
    );
    assert!(out.contains("eflags "), "transcript:\n{out}");
}

#[test]
fn memory_comes_out_as_bytes_at_the_address_asked_for() {
    let out = drive("mem 0x400000 8\nquit\n");
    // The stub guest's first instruction is a mov of a four-byte immediate.
    assert!(
        out.contains("0x00400000 b8 00 00 00 00 b9 03 00"),
        "transcript:\n{out}"
    );
}

#[test]
fn a_break_stops_on_the_instruction_it_names() {
    // The stub's loop starts at its eleventh byte.
    let out = drive("break 0x40000a\ncontinue\nregs\nquit\n");
    assert!(out.contains("break at 0x0040000a"), "transcript:\n{out}");
    assert!(
        out.contains("break 0x0040000a"),
        "the run stopped there:\n{out}"
    );
    assert!(
        out.contains("cursor 2 eip 0x0040000a"),
        "two instructions in:\n{out}"
    );
}

#[test]
fn until_flip_stops_when_the_guest_has_presented() {
    let out = drive("until flip 1\nquit\n");
    let stopped = out
        .lines()
        .find(|l| l.contains("flips 1"))
        .expect("the run reached a flip");
    assert!(
        stopped.contains(&format!("cursor {}", stub::RETIRED - 1)),
        "at {stopped}"
    );
}

#[test]
fn continuing_runs_the_guest_to_its_own_end() {
    let out = drive("continue\nquit\n");
    assert!(out.contains("stopped: exited 3"), "transcript:\n{out}");
    assert!(
        out.contains(&format!("cursor {}", stub::RETIRED)),
        "transcript:\n{out}"
    );
}

#[test]
fn what_is_typed_here_is_not_what_the_guest_reads() {
    // The debugger's own words are not input: a run driven from it presents
    // the same frame as one that was not.
    let out = drive("step 100\nquit\n");
    assert!(out.contains("stopped: exited 3"), "transcript:\n{out}");
    assert!(
        !out.contains("input flip"),
        "nothing typed reached the guest:\n{out}"
    );
}

#[test]
fn a_command_nobody_defined_is_refused_by_name() {
    let out = drive("frobnicate\nhelp\nquit\n");
    assert!(
        out.contains("no such command: frobnicate"),
        "transcript:\n{out}"
    );
    assert!(
        out.contains("until flip <n>"),
        "help lists the commands:\n{out}"
    );
}
