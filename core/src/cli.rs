//! The command line, parsed once for every title.
//!
//! Every title binary takes the same flags, because a player who learns one
//! has learnt all three supported titles' and a route that runs on one reads
//! the same on another.

use std::fmt;
use std::path::PathBuf;

/// How much a run writes to its log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trace {
    /// Inputs, flips and the run's outcome. Always written.
    #[default]
    Off,
    /// Every call across the boundary.
    Shims,
    /// Every instruction retired.
    Insns,
}

impl Trace {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "shims" => Some(Self::Shims),
            "insns" => Some(Self::Insns),
            _ => None,
        }
    }
}

/// What the person running the emulator asked for.
#[derive(Clone, Debug, Default)]
pub struct Cli {
    /// Run with no window: uncapped, and every frame recorded rather than
    /// shown.
    pub headless: bool,
    /// Replay this route.
    pub route: Option<PathBuf>,
    /// Write what was done into the run directory as a route.
    pub record: bool,
    /// Let mixed audio reach a host device. Mixing runs either way.
    pub audio: bool,
    pub trace: Trace,
    /// Write a snapshot every this many flips.
    pub snapshot_every: Option<u64>,
    /// Which flips to write into the run directory as pictures.
    pub screenshots: Screenshots,
    /// Where what the guest writes lands, instead of beside the pressing.
    pub data_root: Option<PathBuf>,
    /// Drive the machine from a debugger on standard input.
    pub repl: bool,
    /// Run the stub guest instead of the title: the machine's own test guest.
    pub stub: bool,
}

/// The flips a run writes as PNG files: named ones, and every so many.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Screenshots {
    pub at: Vec<u64>,
    pub every: Option<u64>,
}

impl Screenshots {
    #[must_use]
    pub fn wants(&self, flip: u64) -> bool {
        self.at.contains(&flip) || self.every.is_some_and(|n| flip.is_multiple_of(n))
    }
}

/// A command line this emulator will not act on.
#[derive(Debug)]
pub enum CliError {
    /// A flag nothing defines. Refused rather than ignored: a misspelled flag
    /// that runs anyway is a run that did not do what was asked.
    Unknown(String),
    /// A flag that takes a value, given none.
    MissingValue(&'static str),
    /// A value the flag does not accept.
    BadValue { flag: &'static str, value: String },
    /// `--help` was asked for. Not a failure.
    Help,
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(a) => write!(f, "unknown argument `{a}`"),
            Self::MissingValue(flag) => write!(f, "`{flag}` needs a value"),
            Self::BadValue { flag, value } => write!(f, "`{flag}` does not take `{value}`"),
            Self::Help => write!(f, "{USAGE}"),
        }
    }
}

impl std::error::Error for CliError {}

pub const USAGE: &str = "\
usage: <title> [flags]

  --headless               no window; uncapped; frames recorded, not shown
  --route <file>           replay a route
  --record                 write this run's inputs into the run directory
  --audio                  let mixed audio reach a host device
  --trace off|shims|insns  how much the run log keeps (default: off)
  --snapshot-every <flips> write a snapshot this often
  --screenshot <flip,...>  write these flips as PNGs into the run directory,
                           as shots/<flip, ten digits>.png
  --screenshot-every <n>   write every nth flip as a PNG
  --data-root <dir>        keep what the guest writes here, not beside the rip
  --repl                   drive the machine from a debugger on stdin
  --stub                   run the machine's own stub guest
  -h, --help               this";

impl Cli {
    /// Parse the arguments a title binary was given, without the program name.
    ///
    /// # Errors
    /// On any argument this emulator does not define, or a flag given a value
    /// it does not accept.
    pub fn parse<I, S>(args: I) -> Result<Self, CliError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut cli = Self::default();
        let mut it = args.into_iter().map(Into::into);
        while let Some(arg) = it.next() {
            let mut value = |flag: &'static str| it.next().ok_or(CliError::MissingValue(flag));
            match arg.as_str() {
                "--headless" => cli.headless = true,
                "--record" => cli.record = true,
                "--audio" => cli.audio = true,
                "--repl" => cli.repl = true,
                "--stub" => cli.stub = true,
                "-h" | "--help" => return Err(CliError::Help),
                "--route" => cli.route = Some(PathBuf::from(value("--route")?)),
                "--data-root" => cli.data_root = Some(PathBuf::from(value("--data-root")?)),
                "--trace" => {
                    let v = value("--trace")?;
                    cli.trace = Trace::parse(&v).ok_or(CliError::BadValue {
                        flag: "--trace",
                        value: v,
                    })?;
                }
                "--screenshot" => {
                    let v = value("--screenshot")?;
                    for flip in v.split(',') {
                        cli.screenshots.at.push(flip.trim().parse().map_err(|_| {
                            CliError::BadValue {
                                flag: "--screenshot",
                                value: v.clone(),
                            }
                        })?);
                    }
                }
                "--screenshot-every" => {
                    let v = value("--screenshot-every")?;
                    cli.screenshots.every = Some(v.parse().ok().filter(|n| *n > 0).ok_or(
                        CliError::BadValue {
                            flag: "--screenshot-every",
                            value: v,
                        },
                    )?);
                }
                "--snapshot-every" => {
                    let v = value("--snapshot-every")?;
                    cli.snapshot_every = Some(v.parse().map_err(|_| CliError::BadValue {
                        flag: "--snapshot-every",
                        value: v,
                    })?);
                }
                _ => return Err(CliError::Unknown(arg)),
            }
        }
        Ok(cli)
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, CliError, Trace};

    #[test]
    fn an_unknown_flag_is_refused_by_name() {
        let err = Cli::parse(["--headles"]).unwrap_err();
        assert!(matches!(err, CliError::Unknown(a) if a == "--headles"));
    }

    #[test]
    fn every_flag_parses() {
        let cli = Cli::parse([
            "--headless",
            "--route",
            "r.route",
            "--record",
            "--audio",
            "--trace",
            "insns",
            "--snapshot-every",
            "64",
            "--data-root",
            "/tmp/dr",
            "--repl",
            "--stub",
            "--screenshot",
            "4700,5450",
            "--screenshot-every",
            "300",
        ])
        .unwrap();
        assert_eq!(cli.screenshots.at, [4700, 5450]);
        assert_eq!(cli.screenshots.every, Some(300));
        assert!(Cli::parse(["--screenshot-every", "0"]).is_err());
        assert!(cli.headless && cli.record && cli.audio && cli.repl && cli.stub);
        assert_eq!(cli.trace, Trace::Insns);
        assert_eq!(cli.snapshot_every, Some(64));
        assert_eq!(cli.route.unwrap().to_str().unwrap(), "r.route");
        assert_eq!(cli.data_root.unwrap().to_str().unwrap(), "/tmp/dr");
    }

    #[test]
    fn a_flag_that_wants_a_value_and_gets_none_is_refused() {
        assert!(matches!(
            Cli::parse(["--trace"]).unwrap_err(),
            CliError::MissingValue("--trace")
        ));
        assert!(matches!(
            Cli::parse(["--trace", "everything"]).unwrap_err(),
            CliError::BadValue {
                flag: "--trace",
                ..
            }
        ));
    }
}
