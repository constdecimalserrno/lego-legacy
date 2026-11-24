//! The run directory and its log.
//!
//! Every run gets its own directory holding one append-only text log. Inputs
//! and flips are always recorded, because a run whose inputs were not written
//! down cannot be replayed, and a wall that cannot be replayed is a bug report
//! nobody can act on. Everything heavier than that is behind `--trace`.

use crate::cli::Trace;
use std::cell::RefCell;
use std::fmt::{self, Write as _};
use std::fs::{self, File};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The first line of every log, so a file's format is knowable from its bytes.
pub const MAGIC: &str = "#lego-legacy-run 1";

const LOG_BUFFER_CAPACITY: usize = 64 * 1024;

/// Where run directories are made. `LEGO_LEGACY_RUNS` overrides it, which is
/// how tests keep their runs out of the working tree.
#[must_use]
pub fn runs_root() -> PathBuf {
    std::env::var_os("LEGO_LEGACY_RUNS").map_or_else(|| PathBuf::from("runs"), PathBuf::from)
}

/// One run's directory and its log.
pub struct RunLog {
    dir: PathBuf,
    file: RefCell<BufWriter<File>>,
    trace: Trace,
}

impl RunLog {
    /// Make a run directory under [`runs_root`] and open its log.
    ///
    /// # Errors
    /// When the directory or the log cannot be created.
    pub fn create(title: &str, trace: Trace) -> std::io::Result<Self> {
        Self::create_in(&runs_root(), title, trace)
    }

    fn create_in(root: &Path, title: &str, trace: Trace) -> std::io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        fs::create_dir_all(root)?;
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
        let mut file =
            BufWriter::with_capacity(LOG_BUFFER_CAPACITY, File::create(dir.join("log"))?);
        writeln!(file, "{MAGIC}")?;
        writeln!(file, "title {title}")?;
        Ok(Self {
            dir,
            file: RefCell::new(file),
            trace,
        })
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        if let Ok(mut file) = self.file.try_borrow_mut() {
            let _ = file.flush();
        }
        &self.dir
    }

    #[must_use]
    pub fn trace(&self) -> Trace {
        self.trace
    }

    /// Flush buffered log lines to disk.
    pub fn flush(&mut self) {
        let _ = self.file.get_mut().flush();
    }

    /// Write one line. A log that cannot be written is not worth stopping a run
    /// over, so a failure here is dropped rather than propagated: the run's
    /// own outcome is still reported through its exit status.
    pub fn line(&mut self, line: &str) {
        let file = self.file.get_mut();
        let _ = writeln!(file, "{line}");
        if line.starts_with("end ") || line.starts_with("wall ") || line.starts_with("digest ") {
            let _ = file.flush();
        }
    }

    /// Write one formatted line directly into the buffered log without
    /// allocating an intermediate [`String`].
    pub fn line_fmt(&mut self, args: fmt::Arguments<'_>) {
        let _ = writeln!(self.file.get_mut(), "{args}");
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

impl Drop for RunLog {
    fn drop(&mut self) {
        let _ = self.file.get_mut().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_fmt_matches_line_and_flushes_on_end_wall_dir_and_drop() {
        let base = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!("lego-legacy-runlog-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let mut log = RunLog::create_in(&root, "test-title", Trace::Off).expect("create run log");
        log.line_fmt(format_args!("flip {} cursor {} 640x480x16", 7, 12345));
        // Reading via `log.dir()` flushes buffered lines before Drop.
        let path = log.dir().join("log");
        let after_dir = fs::read_to_string(&path).expect("read after dir");
        assert!(after_dir.contains("flip 7 cursor 12345 640x480x16"));

        // `end ` and `wall ` lines flush immediately even without calling `dir()`.
        log.line("end exit 0 cursor 12345 flips 8 declared-ms 100");
        let after_end = fs::read_to_string(&path).expect("read after end");
        assert!(after_end.contains("end exit 0 cursor 12345 flips 8 declared-ms 100"));

        log.line("wall report written");
        let after_wall = fs::read_to_string(&path).expect("read after wall");
        assert!(after_wall.contains("wall report written"));

        // Subsequent buffered line flushes on Drop.
        log.line_fmt(format_args!("trailing {}", 99));
        drop(log);
        let final_text = fs::read_to_string(&path).expect("read after drop");
        assert!(final_text.contains("trailing 99"));

        let _ = fs::remove_dir_all(&root);
    }
}
