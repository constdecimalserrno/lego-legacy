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

    /// Fail unless another run ended as this one did: the same instructions
    /// retired, the same flips presented, the same declared time and the same
    /// machine. What a determinism gate holds two replays of a route to.
    ///
    /// # Panics
    /// When any of the four differ.
    pub fn assert_same_machine(&self, other: &Self) -> &Self {
        let ended = |r: &Self| (r.finish.cursor, r.finish.flips, r.finish.declared_millis);
        assert_eq!(
            ended(self),
            ended(other),
            "the same instructions, flips and declared time"
        );
        assert_eq!(
            self.digest, other.digest,
            "and the machine ends the same way"
        );
        self
    }

    /// The last flip the log names before it first carries this, or zero when
    /// it names none before it.
    ///
    /// # Panics
    /// When the log never carries it.
    #[must_use]
    pub fn flip_before(&self, needle: &str) -> u64 {
        self.log
            .split_once(needle)
            .unwrap_or_else(|| panic!("the log never names `{needle}`"))
            .0
            .lines()
            .filter_map(|l| l.strip_prefix("flip "))
            .filter_map(|l| l.split_whitespace().next())
            .filter_map(|n| n.parse::<u64>().ok())
            .next_back()
            .unwrap_or(0)
    }

    /// How loud each block mixed from one flip up to another is: the root
    /// mean square of its samples.
    #[must_use]
    pub fn loudness(&self, from: u64, to: u64) -> Vec<f64> {
        self.host
            .audio_blocks
            .iter()
            .zip(&self.host.audio_flips)
            .filter(|(_, flip)| (from..to).contains(*flip))
            .map(|(block, _)| {
                let power: f64 = block.iter().map(|s| f64::from(*s).powi(2)).sum();
                (power / block.len().max(1) as f64).sqrt()
            })
            .collect()
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
        // The stub guest is the one title with no image to load.
        stub: title.key == Title::STUB.key,
        ..Cli::default()
    };
    run_headless(title, &cli, budget)
}

/// Point a test at the checkout's rips, unless `LEGO_LEGACY_GAME_DATA`
/// already points somewhere else, and say whether this pressing's image is
/// among them.
///
/// Without the rips a disc test prints a skip line and passes, which is a gate
/// that did not run rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1`
/// turns the skip into a failure.
///
/// # Panics
/// When the image is absent and `LEGO_LEGACY_REQUIRE_DISC` is set.
#[must_use]
pub fn rips(image: &EntryImage) -> bool {
    point_at_rips();
    ripped(&disc::pressing_path(image.disc_dir, image.pressing))
}

/// As [`rips`], for a pressing's Red Book audio: the cue sheet beside its
/// image and the raw rip beside that.
///
/// # Panics
/// When either is absent and `LEGO_LEGACY_REQUIRE_DISC` is set.
#[must_use]
pub fn red_book_rips(disc_dir: &str, pressing: &str) -> bool {
    point_at_rips();
    let image = disc::pressing_path(disc_dir, pressing);
    ripped(&image.with_extension("cue")) && ripped(&image.with_extension("bin"))
}

/// Rock Raiders pointed at its original pressing, whose executable is wrapped
/// in SafeDisc (ADR-0006), still declaring the bytes of the Masterpiece's:
/// what a person who ripped the original where the Masterpiece belongs is
/// running, and what a test points the title at to see the wrapper named and
/// refused before anything is hashed (#100).
pub const ROCK_RAIDERS_WRAPPED: EntryImage = EntryImage {
    pressing: "rock-raiders",
    ..crate::roster::ROCK_RAIDERS.image
};

fn point_at_rips() {
    if std::env::var_os("LEGO_LEGACY_GAME_DATA").is_none() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
        std::env::set_var("LEGO_LEGACY_GAME_DATA", root);
    }
}

/// Whether one file of the rips is there; a skip line when it is not, and a
/// failure when `LEGO_LEGACY_REQUIRE_DISC` is set.
fn ripped(file: &Path) -> bool {
    let present = file.is_file();
    if !present {
        let required = std::env::var("LEGO_LEGACY_REQUIRE_DISC").is_ok_and(|v| v != "0");
        assert!(
            !required,
            "LEGO_LEGACY_REQUIRE_DISC is set but {} is absent",
            file.display()
        );
        eprintln!("skipping: no {} (rip your own discs there)", file.display());
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

/// The declared tree over a title's pressing, built as the machine builds it
/// ([`Tree::install`]).
///
/// # Panics
/// When the pressing will not open or install.
#[must_use]
pub fn tree(title: &Title) -> Tree {
    let iso = disc::open_pressing(title.image.disc_dir, title.image.pressing)
        .expect("the pressing opens");
    let mut tree = Tree::new();
    tree.install(iso, title).expect("the pressing installs");
    tree
}
