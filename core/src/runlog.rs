//! The run directory and its log.
//!
//! Every run gets its own directory holding one append-only text log. Inputs
//! and flips are always recorded, because a run whose inputs were not written
//! down cannot be replayed, and a wall that cannot be replayed is a bug report
//! nobody can act on. Everything heavier than that is behind `--trace`.

use crate::cli::Trace;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The first line of every log, so a file's format is knowable from its bytes.
pub const MAGIC: &str = "#lego-legacy-run 1";

/// Where run directories are made. `LEGO_LEGACY_RUNS` overrides it, which is
/// how tests keep their runs out of the working tree.
#[must_use]
pub fn runs_root() -> PathBuf {
    std::env::var_os("LEGO_LEGACY_RUNS").map_or_else(|| PathBuf::from("runs"), PathBuf::from)
}

/// One run's directory and its log.
pub struct RunLog {
    dir: PathBuf,
    file: File,
    trace: Trace,
}

impl RunLog {
    /// Make a run directory under [`runs_root`] and open its log.
    ///
    /// # Errors
    /// When the directory or the log cannot be created.
    pub fn create(title: &str, trace: Trace) -> std::io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let root = runs_root();
        fs::create_dir_all(&root)?;
        // A run directory is claimed by creating it, never by looking first:
        // two runs starting in the same second with the same pid (a test
        // binary's threads) would otherwise agree on a name and interleave
        // into one log.
        let mut n = 0;
        let dir = loop {
            let mut suffix = String::new();
            if n > 0 {
                let _ = write!(suffix, "-{n}");
            }
            let dir = root.join(format!("{stamp}-{}{suffix}", std::process::id()));
            match fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        };
        let mut file = File::create(dir.join("log"))?;
        writeln!(file, "{MAGIC}")?;
        writeln!(file, "title {title}")?;
        Ok(Self { dir, file, trace })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[must_use]
    pub fn trace(&self) -> Trace {
        self.trace
    }

    /// Write one line. A log that cannot be written is not worth stopping a run
    /// over, so a failure here is dropped rather than propagated: the run's
    /// own outcome is still reported through its exit status.
    pub fn line(&mut self, line: &str) {
        let _ = writeln!(self.file, "{line}");
    }

    /// Write one line only at or above a trace level. The line is made only
    /// when it will be written: a shim call is made hundreds of thousands of
    /// times a second, and formatting one that nobody asked for is most of
    /// what it costs.
    pub fn at(&mut self, level: Trace, line: impl FnOnce() -> String) {
        if self.trace >= level {
            self.line(&line());
        }
    }
}
