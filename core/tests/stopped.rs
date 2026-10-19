//! A run stopped from its terminal, as Ctrl-C or `SIGTERM` stops one, still
//! ends with its `end` line and digest (#110).
//!
//! A test binary of its own, because a signal is the whole process's: every
//! run beside this one would be ended by it too. It is `SIGTERM` that is
//! raised, which one handler hears with Ctrl-C's `SIGINT` and the terminal's
//! `SIGHUP`: a suite started in the background is deaf to `SIGINT`, and a
//! signal the process was started deaf to stays so.
#![cfg(unix)]

use lego_legacy_core::cli::Cli;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::host::{Frame, Host, HostError, Input, Mapping};
use lego_legacy_core::machine::Outcome;
use lego_legacy_core::{run, Title};

extern "C" {
    fn raise(sig: i32) -> i32;
}

/// The headless host, with the run told to stop as its first frame is
/// presented.
struct Stopped(Headless);

impl Host for Stopped {
    fn present(&mut self, frame: &Frame<'_>) {
        self.0.present(frame);
        // Safety: libc's own, raising SIGTERM at this process.
        assert_eq!(unsafe { raise(15) }, 0, "SIGTERM raised");
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
}

/// The stub guest presents a frame and exits. A signal at that frame ends
/// the run there instead, as the window's close button would, and the run
/// still writes its wrap-up; the process carries on.
#[test]
fn a_run_stopped_by_a_signal_writes_its_end_line_and_digest() {
    let scratch = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::env::set_var("LEGO_LEGACY_RUNS", scratch);
    lego_legacy_core::host::listen();
    let cli = Cli {
        stub: true,
        headless: true,
        data_root: Some(scratch.join("stopped")),
        ..Cli::default()
    };
    let (finish, log, digest) =
        run::run_digested(&Title::STUB, &cli, &mut Stopped(Headless::new()), u64::MAX)
            .expect("the stub runs");
    let dir = log.dir().to_path_buf();
    drop(log);
    let text = std::fs::read_to_string(dir.join("log")).expect("the log is there");

    assert_eq!(finish.outcome, Outcome::Closed, "{text}");
    assert_eq!(finish.flips, 1, "{text}");
    let end = format!(
        "end window closed cursor {} flips 1 declared-ms",
        finish.cursor
    );
    assert!(text.lines().any(|l| l.starts_with(&end)), "{text}");
    assert!(
        text.lines().any(|l| l == format!("digest {digest}")),
        "{text}"
    );
}
