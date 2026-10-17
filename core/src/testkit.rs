//! Running a title headless from a test.
//!
//! This is the run seam with the assertions a route test needs around it. Tests
//! go through here rather than reaching into the machine, so what they hold is
//! the behaviour a player would see: what was presented, what was logged, and
//! how the run ended.

use crate::cli::Cli;
use crate::disc;
use crate::host::headless::{Headless, Presented};
use crate::install::Tree;
use crate::machine::{Finish, Outcome};
use crate::title::EntryImage;
use crate::Title;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A finished headless run, with everything it produced.
///
/// Printed as its outcome and its log, so a test that expected a refusal and
/// got a run says which run it got.
pub struct Ran {
    pub finish: Finish,
    /// What the machine looked like when it stopped: the same route on the
    /// same build must produce the same value.
    pub digest: String,
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

/// A directory of this run's own, under the scratch root the tests run in,
/// for whatever the guest writes.
fn scratch_data_root() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let at = crate::runlog::runs_root().join(format!(
        "data-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&at);
    at
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
pub fn run_headless(title: &'static Title, cli: &Cli, budget: u64) -> Ran {
    run_on(Headless::new(), title, cli, budget)
}

/// Run a title on a headless host that has already been given something:
/// input queued for the flips it will reach, for instance.
///
/// # Panics
/// When the run could not be set up at all, which is a broken test rather than
/// a guest outcome.
#[must_use]
pub fn run_on(host: Headless, title: &'static Title, cli: &Cli, budget: u64) -> Ran {
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
pub fn try_run_headless(title: &'static Title, cli: &Cli, budget: u64) -> Result<Ran, String> {
    try_run_on(Headless::new(), title, cli, budget)
}

fn try_run_on(
    mut host: Headless,
    title: &'static Title,
    cli: &Cli,
    budget: u64,
) -> Result<Ran, String> {
    let headless = Cli {
        headless: true,
        // Every test run starts from a machine with nothing written on it:
        // what one run's guest saves is not what the next one boots with,
        // and two runs side by side do not write over each other.
        data_root: cli.data_root.clone().or_else(|| Some(scratch_data_root())),
        ..cli.clone()
    };
    let (finish, log, digest) =
        crate::run::run_digested(title, &headless, &mut host, budget).map_err(|e| e.to_string())?;
    let dir = log.dir().to_path_buf();
    drop(log);
    let text = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
    Ok(Ran {
        finish,
        digest,
        host,
        log: text,
        dir,
    })
}

/// Run a title headless against a route, within an instruction budget.
///
/// What every route test goes through: a route on disk, a named state to
/// reach, and a budget to reach it in.
///
/// # Panics
/// When the route cannot be read, or the run could not be set up.
#[must_use]
pub fn run_route(title: &'static Title, route: &std::path::Path, budget: u64) -> Ran {
    let cli = Cli {
        route: Some(route.to_path_buf()),
        stub: true,
        ..Cli::default()
    };
    run_headless(title, &cli, budget)
}

/// Point a test at the checkout's rips, and say whether this pressing is
/// among them.
///
/// Without the rips a disc test prints a skip line and passes, which is a gate
/// that did not run rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1`
/// turns the skip into a failure.
///
/// # Panics
/// When the pressing is absent and `LEGO_LEGACY_REQUIRE_DISC` is set.
#[must_use]
pub fn rips(image: &EntryImage) -> bool {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
    std::env::set_var("LEGO_LEGACY_GAME_DATA", &root);

    let present = disc::pressing_path(image.disc_dir, image.pressing).is_file();
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

/// A run done once into its slot, keeping the frames the tests look at, for
/// every test that looks at it; `None` when `rips` says the pressing is not
/// there.
///
/// A gate that holds a run to replaying gives it a second slot.
///
/// # Panics
/// When the run could not be set up at all.
pub fn replay(
    slot: &'static OnceLock<Option<Ran>>,
    rips: impl FnOnce() -> bool,
    title: &'static Title,
    cli: &Cli,
    keep: &[u64],
    budget: u64,
) -> Option<&'static Ran> {
    slot.get_or_init(|| {
        if !rips() {
            return None;
        }
        Some(run_on(Headless::new().keeping(keep), title, cli, budget))
    })
    .as_ref()
}

/// How many pixels two frames of the same size disagree on, within a
/// rectangle given as its left, top, right and bottom.
#[must_use]
pub fn differing(
    a: &Presented,
    b: &Presented,
    (x0, y0, x1, y1): (usize, usize, usize, usize),
) -> usize {
    let stride = a.width as usize;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * stride + x) * 4))
        .filter(|at| a.pixels[*at..at + 3] != b.pixels[*at..at + 3])
        .count()
}

/// The declared tree over a title's cabinet, as the machine builds it.
///
/// # Panics
/// When the pressing or its cabinet will not read.
#[must_use]
pub fn tree(title: &Title) -> Tree {
    let mut iso = disc::open_pressing(title.image.disc_dir, title.image.pressing)
        .expect("the pressing opens");
    let header = iso.read("DATA1.HDR").expect("the cabinet's header reads");
    let mut tree = Tree::new();
    tree.set_cabinet(&header, "DATA", title.install_dir)
        .expect("the cabinet reads");
    tree.set_disc(iso);
    tree
}
