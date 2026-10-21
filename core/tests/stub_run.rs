//! The run seam, exercised end to end on the machine's own guest.
//!
//! Every route test the roster grows goes through this door: run a title
//! headless within an instruction budget, then assert what it presented, what
//! it logged, and how it ended.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::host::{Button, Frame, Host, HostError, Input, Mapping};
use lego_legacy_core::{run, stub, testkit, Title};

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
    assert!(lines.next().is_some_and(|l| l.starts_with("version ")));
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
fn the_run_log_names_the_title_crate_and_core_versions_that_made_it() {
    scratch();
    const VERSIONED: Title = Title {
        version: "9.8.7",
        ..Title::STUB
    };
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&VERSIONED, &cli, 1_000);
    let core = env!("CARGO_PKG_VERSION");
    assert_eq!(
        ran.log.lines().nth(2),
        Some(format!("version 9.8.7 core {core}").as_str()),
        "the header's third line names the title's crate, then core"
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
        (0, Input::Down(Button::Left)),
        (0, Input::Move { x: 320, y: 213 }),
        (99, Input::Up(Button::Left)),
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
        [(99, Input::Up(Button::Left))],
        "an event naming a flip the run never reached is kept, not lost"
    );
}

/// The headless host, with a device that kept up.
struct Heard(Headless);

impl Host for Heard {
    fn present(&mut self, frame: &Frame<'_>) {
        self.0.present(frame);
    }

    fn poll_input(&mut self, flip: u64) -> Vec<Input> {
        self.0.poll_input(flip)
    }

    fn audio(&mut self, samples: &[i16]) {
        self.0.audio(samples);
    }

    fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
        self.0.map(len)
    }

    fn audio_underruns(&self) -> Option<u64> {
        Some(0)
    }
}

/// A run a device heard says how often it ran dry, zero included, and a run
/// nobody heard says nothing (#90): zero is what puts a complaint about the
/// sound on the mix rather than on the host.
#[test]
fn a_run_a_device_heard_says_it_never_ran_dry_and_one_nobody_heard_says_nothing() {
    scratch();
    let cli = Cli {
        stub: true,
        data_root: Some(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("heard")),
        ..Cli::default()
    };
    let (_, log) =
        run::run(&TEST_TITLE, &cli, &mut Heard(Headless::new()), 1_000).expect("the stub runs");
    let dir = log.dir().to_path_buf();
    drop(log);
    let text = std::fs::read_to_string(dir.join("log")).expect("the log is there");
    assert!(
        text.lines()
            .any(|l| l == "audio: the device ran dry 0 times"),
        "{text}"
    );

    let unheard = testkit::run_headless(&TEST_TITLE, &cli, 1_000);
    assert!(
        !unheard.log.contains("audio: the device"),
        "{}",
        unheard.log
    );
}

/// A run that has ended gives the pointer back before anything else, so a
/// window waiting to be closed holds nothing of it (#111), not even the
/// shape the title gave it (#139).
#[test]
fn a_run_that_has_ended_gives_the_pointer_back() {
    /// The headless host, with a window's record of whether the title has
    /// the pointer hidden.
    struct Held(Headless, bool);
    impl Host for Held {
        fn present(&mut self, frame: &Frame<'_>) {
            self.0.present(frame);
        }
        fn poll_input(&mut self, flip: u64) -> Vec<Input> {
            self.0.poll_input(flip)
        }
        fn audio(&mut self, samples: &[i16]) {
            self.0.audio(samples);
        }
        fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
            self.0.map(len)
        }
        fn show_pointer(&mut self, show: bool) {
            self.1 = !show;
        }
        fn pointer_shape(&mut self, shape: Option<&lego_legacy_core::host::Cursor>) {
            self.0.pointer_shape(shape);
        }
    }

    scratch();
    let cli = Cli {
        stub: true,
        data_root: Some(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("held")),
        ..Cli::default()
    };
    let mut host = Held(Headless::new(), true);
    run::run(&TEST_TITLE, &cli, &mut host, 1_000).expect("the stub runs");
    assert!(!host.1, "the pointer is still the run's after it ended");
    assert!(
        matches!(host.0.shapes.last(), Some((_, None))),
        "and it is the host's own arrow"
    );
}

/// A host that cannot keep pace says so, and the run says it back for a
/// title's own frames as it does for the stub's: those are presented by
/// another path, and it never asked (#75, #94).
#[test]
fn a_title_s_frame_on_a_slow_host_logs_its_pace() {
    struct Slow(Headless);
    impl Host for Slow {
        fn present(&mut self, frame: &Frame<'_>) {
            self.0.present(frame);
        }
        fn poll_input(&mut self, flip: u64) -> Vec<Input> {
            self.0.poll_input(flip)
        }
        fn audio(&mut self, samples: &[i16]) {
            self.0.audio(samples);
        }
        fn map(&mut self, len: usize) -> Result<Mapping, HostError> {
            self.0.map(len)
        }
        fn pace(&mut self, _declared_nanos: u64) -> f64 {
            0.5
        }
    }

    scratch();
    let mut host = Slow(Headless::new());
    let mut machine =
        lego_legacy_core::machine::Machine::new(&TEST_TITLE, &mut host).expect("a machine");
    machine.load_stub().expect("the stub loads");
    // What a title's own `Flip` leaves for the machine to present.
    machine.state.present = Some(lego_legacy_core::win32::Present {
        pixels: stub::SURFACE,
        width: 4,
        height: 4,
        pitch: 16,
        depth: 32,
    });
    let mut log =
        lego_legacy_core::runlog::RunLog::create("stub", Trace::Off).expect("a run directory");
    assert_eq!(machine.tick(&mut host, &mut log), None);
    let text = std::fs::read_to_string(log.dir().join("log")).expect("the log reads");
    assert!(text.contains("flip 0 cursor 0 4x4x32"), "{text}");
    assert!(text.contains("pace 0.50x"), "{text}");
}
