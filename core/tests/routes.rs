//! Routes, replayed and recorded, and the gate that says a run reproduces.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::{headless::Headless, Button, Frame, Host, HostError, Input, Mapping};
use lego_legacy_core::machine::{Finish, Machine, Outcome, ASK_BETWEEN_FLIPS_MILLIS, STACK_TOP};
use lego_legacy_core::route::Route;
use lego_legacy_core::runlog::RunLog;
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
        (0, Input::Down(Button::Left)),
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

    first.assert_same_machine(&second);
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
    host.queue(vec![
        (0, Input::Down(Button::Left)),
        (0, Input::Move { x: 999, y: 999 }),
    ]);
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

#[test]
fn a_route_naming_a_poll_the_run_never_reaches_says_so() {
    let dir = scratch();
    let path = dir.join("too-many-polls.route");
    // The stub guest presents once and exits, and polls nothing between.
    std::fs::write(&path, "flip 0 down\nflip 0 poll 1 up\n").expect("the route writes");

    let ran = testkit::run_route(&STUB, &path, 1_000);
    ran.assert_exited(stub::EXIT_CODE)
        .assert_logged("route undelivered 1 events after flip 1");
}

/// Space, as DirectInput numbers it.
const SPACE: u32 = 0x39;

fn space(down: bool) -> Input {
    Input::Key { code: SPACE, down }
}

/// What a run of [`waiting_for_space`] left: how it ended, its log, its
/// machine's digest, and what it recorded.
struct Waited {
    finish: Finish,
    log: String,
    digest: String,
    recorded: Route,
}

/// A guest that waits for Space to come up between flips, as LEGO Chess waits
/// after a movie it skipped (#215), run replaying `route` if there is one and
/// recording either way.
///
/// It has just finished its first frame, left for the machine to present as
/// a title's own `Flip` leaves one. Then it asks `GetAsyncKeyState(VK_SPACE)`
/// until the key is up, sleeping `nap` declared milliseconds after each ask
/// that finds it down, and exits with 7. Hand-assembled into the machine, as
/// threads.rs's guests are, calling through the boundary as a title's imports
/// do.
fn waiting_for_space(host: &mut dyn Host, route: Option<&str>, nap: u32, budget: u64) -> Waited {
    scratch();
    let mut machine = Machine::new(&STUB, host).expect("a machine");
    let ask = machine
        .boundary
        .bind_named("USER32.dll", "GetAsyncKeyState");
    let sleep = machine.boundary.bind_named("KERNEL32.dll", "Sleep");
    let exit = machine.boundary.bind_named("KERNEL32.dll", "ExitProcess");
    let call = |to: u32| [&[0xb8][..], &to.to_le_bytes(), &[0xff, 0xd0]].concat(); // mov eax, to; call eax
    let code = [
        // push VK_SPACE; GetAsyncKeyState
        &[0x68, 0x20, 0, 0, 0][..],
        &call(ask),
        // test eax, 0x8000; jz the exit, past the nap
        &[0xa9, 0x00, 0x80, 0x00, 0x00, 0x74, 14],
        // push nap; Sleep; jmp back to the ask
        &[0x68],
        &nap.to_le_bytes(),
        &call(sleep),
        &[0xeb, 0xdf],
        // push 7; ExitProcess
        &[0x68, 7, 0, 0, 0],
        &call(exit),
    ]
    .concat();
    const MAIN: u32 = 0x0040_0000;
    machine.arena.write(MAIN, &code).expect("the code fits");
    machine.cpu.eip = MAIN;
    machine.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
    machine.state.present = Some(lego_legacy_core::win32::Present {
        pixels: stub::SURFACE,
        width: 4,
        height: 4,
        pitch: 16,
        depth: 32,
    });
    machine.route = route.map(|text| Route::parse(text).expect("the route reads"));
    machine.recording = Some(Route::new());
    let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
    let finish = machine.run(host, &mut log, budget);
    Waited {
        finish,
        log: std::fs::read_to_string(log.dir().join("log")).expect("the log reads"),
        digest: machine.state_digest(),
        recorded: machine.recording.take().expect("it recorded"),
    }
}

/// A key let go at a poll between flips reaches a title waiting on it without
/// flipping (#215): Space, pressed at flip 0 and let go at the third poll
/// after it, is down for the guest's first two asks and up at its third. A
/// route of a flip and a poll replays exactly there, and a run records what it
/// was handed where it was handed it.
#[test]
fn a_key_let_go_at_a_poll_between_flips_reaches_a_title_waiting_on_it() {
    let route = "#lego-legacy-route 1\nflip 0 key Space down\nflip 0 poll 3 key Space up\n";
    let waited = waiting_for_space(&mut Headless::new(), Some(route), 0, 1_000_000);
    assert_eq!(
        waited.finish.outcome,
        Outcome::Exited { code: 7 },
        "{}",
        waited.log
    );
    assert_eq!(waited.finish.flips, 1);
    assert!(
        waited
            .log
            .lines()
            .any(|l| l == "input flip 0 poll 3 Key { code: 57, down: false }"),
        "{}",
        waited.log
    );
    assert_eq!(waited.recorded.to_text(), route);
}

/// An event keyed to a flip still waits for that flip (#215): a release keyed
/// to the next flip never reaches a title that waits for it without flipping.
#[test]
fn a_key_let_go_at_the_next_flip_never_reaches_a_title_waiting_between_flips() {
    let waited = waiting_for_space(
        &mut Headless::new(),
        Some("flip 0 key Space down\nflip 1 key Space up\n"),
        0,
        100_000,
    );
    assert_eq!(
        waited.finish.outcome,
        Outcome::BudgetExhausted,
        "{}",
        waited.log
    );
}

/// A tap, a key pressed and let go at one flip, is never seen by a title that
/// asks whether the key is down now: both are set down before it asks, and it
/// finds the key as it would have had nobody touched it. #215 leaves it so.
#[test]
fn a_tap_at_one_flip_is_never_seen_by_a_title_asking_whether_the_key_is_down() {
    let tapped = waiting_for_space(
        &mut Headless::new(),
        Some("flip 0 key Space down\nflip 0 key Space up\n"),
        0,
        100_000,
    );
    let untouched = waiting_for_space(&mut Headless::new(), None, 0, 100_000);
    assert_eq!(tapped.finish.outcome, Outcome::Exited { code: 7 });
    assert_eq!(tapped.finish.cursor, untouched.finish.cursor);
}

/// A person at a window, as a host hands over what they did: Space pressed as
/// the first frame comes up, and let go by the next time the host is asked.
struct Person {
    headless: Headless,
    asked: usize,
}

impl Person {
    fn new() -> Self {
        Self {
            headless: Headless::new(),
            asked: 0,
        }
    }
}

impl Host for Person {
    fn present(&mut self, frame: &Frame<'_>) {
        self.headless.present(frame);
    }

    fn poll_input(&mut self, _flip: u64) -> Vec<Input> {
        self.asked += 1;
        match self.asked {
            1 => vec![space(true)],
            2 => vec![space(false)],
            _ => Vec::new(),
        }
    }

    fn audio(&mut self, _samples: &[i16]) {}

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        self.headless.map(len)
    }
}

/// What a person did between flips reaches a title waiting on it, once the
/// frame has stood the declared threshold (#215): the host is asked at the
/// first poll past it, and the release is delivered and recorded at that
/// poll. The recording, replayed headless, ends on the same machine (rule 1).
#[test]
fn a_person_who_lets_go_between_flips_is_heard_at_the_next_poll_and_replays_there() {
    let nap = u32::try_from(ASK_BETWEEN_FLIPS_MILLIS).expect("small") + 50;
    let mut person = Person::new();
    let heard = waiting_for_space(&mut person, None, nap, 1_000_000);
    assert_eq!(
        heard.finish.outcome,
        Outcome::Exited { code: 7 },
        "{}",
        heard.log
    );
    assert_eq!(person.asked, 2, "at the flip, and at the poll past it");
    let route = heard.recorded.to_text();
    assert_eq!(
        route,
        "#lego-legacy-route 1\nflip 0 key Space down\nflip 0 poll 2 key Space up\n"
    );

    let replayed = waiting_for_space(&mut Headless::new(), Some(&route), nap, 1_000_000);
    let ended = |w: &Waited| (w.finish.cursor, w.finish.flips, w.finish.declared_millis);
    assert_eq!(ended(&replayed), ended(&heard));
    assert_eq!(replayed.digest, heard.digest, "the same machine");
}

/// The host is asked between flips only at the first poll the frame has stood
/// the declared threshold by (#215): a frame that stands less is never asked,
/// and in one that naps just short of it each time, the third poll is the
/// first past it.
#[test]
fn the_host_is_asked_between_flips_only_once_a_frame_has_stood_the_threshold() {
    let mut person = Person::new();
    let quick = waiting_for_space(&mut person, None, 0, 1_000_000);
    assert_eq!(quick.finish.outcome, Outcome::BudgetExhausted);
    assert_eq!(person.asked, 1, "only at the flip: two milliseconds stood");

    let mut person = Person::new();
    let nap = u32::try_from(ASK_BETWEEN_FLIPS_MILLIS).expect("small") - 1;
    let short = waiting_for_space(&mut person, None, nap, 1_000_000);
    assert_eq!(short.finish.outcome, Outcome::Exited { code: 7 });
    assert!(
        short
            .recorded
            .to_text()
            .ends_with("\nflip 0 poll 3 key Space up\n"),
        "{}",
        short.recorded.to_text()
    );
}

/// Asked between flips, the headless host hands over nothing of a later
/// flip's (#215): a release queued for flip 1 waits there for it, so no test
/// that queues input moves.
#[test]
fn the_headless_host_asked_between_flips_keeps_a_later_flip_s_input_for_it() {
    let mut host = Headless::new();
    host.queue(vec![(0, space(true)), (1, space(false))]);
    let nap = u32::try_from(ASK_BETWEEN_FLIPS_MILLIS).expect("small") + 50;
    let waited = waiting_for_space(&mut host, None, nap, 100_000);
    assert_eq!(
        waited.finish.outcome,
        Outcome::BudgetExhausted,
        "{}",
        waited.log
    );
    assert!(waited.finish.declared_millis > 1_000, "it was asked often");
    assert!(!waited.log.contains(" poll "), "{}", waited.log);
    assert_eq!(host.undelivered(), [(1, space(false))]);
}
