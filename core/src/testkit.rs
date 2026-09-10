//! Running a title headless from a test.
//!
//! This is the run seam with the assertions a route test needs around it. Tests
//! go through here rather than reaching into the machine, so what they hold is
//! the behaviour a player would see: what was presented, what was logged, and
//! how the run ended.

use crate::cli::Cli;
use crate::host::headless::Headless;
use crate::machine::{Finish, Outcome};
use crate::Title;
use std::path::PathBuf;

/// A finished headless run, with everything it produced.
///
/// Printed as its outcome and its log, so a test that expected a refusal and
/// got a run says which run it got.
pub struct Ran {
    pub finish: Finish,
    pub host: Headless,
    /// The run log, as text.
    pub log: String,
    pub dir: PathBuf,
}

impl std::fmt::Debug for Ran {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "run ended {} after {} instructions",
            self.finish.outcome, self.finish.cursor
        )
    }
}

impl Ran {
    /// Fail unless the guest ended the run itself with this code.
    ///
    /// # Panics
    /// When the run ended any other way.
    pub fn assert_exited(&self, code: u32) -> &Self {
        assert_eq!(
            self.finish.outcome,
            Outcome::Exited { code },
            "run ended {} instead\nlog:\n{}",
            self.finish.outcome,
            self.log
        );
        self
    }

    /// Fail unless the run stopped at a wall whose description contains
    /// `needle`.
    ///
    /// # Panics
    /// When the run ended any other way, or at a different wall.
    pub fn assert_wall(&self, needle: &str) -> &Self {
        match &self.finish.outcome {
            Outcome::Wall { what } => assert!(
                what.contains(needle),
                "wall was `{what}`, which does not name `{needle}`"
            ),
            other => panic!("run ended {other} instead of a wall naming `{needle}`"),
        }
        self
    }

    /// Fail unless the run presented this many frames.
    ///
    /// # Panics
    /// When it presented a different number.
    pub fn assert_flips(&self, flips: u64) -> &Self {
        assert_eq!(self.finish.flips, flips, "log:\n{}", self.log);
        self
    }

    /// Fail unless the run log carries this line.
    ///
    /// # Panics
    /// When no line of the log contains it.
    pub fn assert_logged(&self, needle: &str) -> &Self {
        assert!(
            self.log.lines().any(|l| l.contains(needle)),
            "no log line names `{needle}`\nlog:\n{}",
            self.log
        );
        self
    }
}

/// Run a title headless within an instruction budget.
///
/// The run directory goes under `LEGO_LEGACY_RUNS`, which a test sets to its
/// own scratch directory so no run lands in the working tree.
///
/// # Panics
/// When the run could not be set up at all, which is a broken test rather than
/// a guest outcome.
#[must_use]
pub fn run_headless(title: &Title, cli: &Cli, budget: u64) -> Ran {
    run_on(Headless::new(), title, cli, budget)
}

/// Run a title on a headless host that has already been given something:
/// input queued for the flips it will reach, for instance.
///
/// # Panics
/// When the run could not be set up at all, which is a broken test rather than
/// a guest outcome.
#[must_use]
pub fn run_on(host: Headless, title: &Title, cli: &Cli, budget: u64) -> Ran {
    try_run_on(host, title, cli, budget)
        .unwrap_or_else(|e| panic!("could not run {}: {e}", title.key))
}

/// Run a title headless and hand back the refusal if it would not start.
///
/// A guest that stops at a wall is a [`Ran`]; an image the emulator will not
/// run at all, or a rip that is not there, is the error.
///
/// # Errors
/// When the run could not be set up: no rip, the wrong pressing, or an image
/// this emulator refuses.
pub fn try_run_headless(title: &Title, cli: &Cli, budget: u64) -> Result<Ran, String> {
    try_run_on(Headless::new(), title, cli, budget)
}

fn try_run_on(mut host: Headless, title: &Title, cli: &Cli, budget: u64) -> Result<Ran, String> {
    let headless = Cli {
        headless: true,
        ..cli.clone()
    };
    let (finish, log) =
        crate::run::run(title, &headless, &mut host, budget).map_err(|e| e.to_string())?;
    let dir = log.dir().to_path_buf();
    drop(log);
    let text = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
    Ok(Ran {
        finish,
        host,
        log: text,
        dir,
    })
}
