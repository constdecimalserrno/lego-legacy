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
    /// Write a snapshot every this many flips, to inspect: only the stub
    /// resumes from one.
    pub snapshot_every: Option<u64>,
    /// Which flips to write into the run directory as pictures.
    pub screenshots: Screenshots,
    /// Where what the guest writes lands, in its `profile`, instead of beside
    /// the pressing.
    pub data_root: Option<PathBuf>,
    /// Drive the machine from a debugger on standard input.
    pub repl: bool,
    /// Run the stub guest instead of the title: the machine's own test guest.
    pub stub: bool,
    /// Print and log host-side speed and frame-time telemetry at run end (#296).
    pub fps_report: bool,
    /// Optional half-open flip window `<start>..<end>` for `--fps-report` (#296).
    pub fps_window: Option<(u64, u64)>,
    /// Serve the newline-delimited JSON control protocol on stdin/stdout (#308).
    pub control_stdio: bool,
    /// Serve the HTTP/1.1 control & introspection endpoint on `HOST:PORT` (#309).
    pub control_http: Option<String>,
    /// Serve the HTTP/1.1 control & introspection endpoint on a Unix socket (#309).
    pub control_unix: Option<PathBuf>,
    /// Optional Bearer token required for mutating `POST` control endpoints (#309).
    pub token: Option<String>,
    /// Initial control execution mode (`step` or `live`) (#308).
    pub control_mode: Option<crate::control::ExecMode>,
    /// Start paused on the first post-route flip (#308).
    pub paused: bool,
}

impl Cli {
    /// Whether any control-plane flag is active on this invocation (#308, #309).
    #[must_use]
    pub fn control_enabled(&self) -> bool {
        self.control_stdio
            || self.control_http.is_some()
            || self.control_unix.is_some()
            || self.control_mode.is_some()
            || self.paused
    }

    /// Whether an external agent/control transport is active (stdio, HTTP, or UDS).
    #[must_use]
    pub fn has_control_transport(&self) -> bool {
        self.control_stdio || self.control_http.is_some() || self.control_unix.is_some()
    }
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
  --snapshot-every <flips> write a snapshot this often, to inspect; only the
                           stub resumes from one
  --screenshot <flip,...>  write these flips as PNGs into the run directory,
                           as shots/<flip, ten digits>.png
  --screenshot-every <n>   write every nth flip as a PNG
  --data-root <dir>        keep what the guest writes in <dir>/profile, not
                           beside the rip
  --repl                   drive the machine from a debugger on stdin
  --stub                   run the machine's own stub guest
  --fps-report             report host speed and frame-time percentiles at run end
  --fps-window <a..b>      also report telemetry over flips [a, b)
  --control-stdio          serve JSONL control commands on stdin/stdout
  --control [HOST:PORT]    serve HTTP/1.1 control & introspection (default: 127.0.0.1:0)
  --control-unix <path>    serve HTTP/1.1 control & introspection on a Unix socket
  --token <secret>         require Bearer token on mutating HTTP POST routes
                           (also read from LEGO_LEGACY_TOKEN)
  --control-mode step|live initial control mode (default: step when control active)
  --paused                 start paused on the first post-route flip
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
        let mut it = args.into_iter().map(Into::into).peekable();
        while let Some(arg) = it.next() {
            if let Some(addr) = arg.strip_prefix("--control=") {
                if addr.is_empty() || !addr.contains(':') {
                    return Err(CliError::BadValue {
                        flag: "--control",
                        value: addr.to_owned(),
                    });
                }
                cli.control_http = Some(addr.to_owned());
                continue;
            }
            let mut value = |flag: &'static str| it.next().ok_or(CliError::MissingValue(flag));
            match arg.as_str() {
                "--headless" => cli.headless = true,
                "--record" => cli.record = true,
                "--audio" => cli.audio = true,
                "--repl" => cli.repl = true,
                "--stub" => cli.stub = true,
                "--fps-report" => cli.fps_report = true,
                "--control-stdio" => cli.control_stdio = true,
                "--control" => {
                    let addr = match it.peek() {
                        Some(next) if !next.starts_with('-') => {
                            let v = it.next().expect("peeked");
                            if v.is_empty() || !v.contains(':') {
                                return Err(CliError::BadValue {
                                    flag: "--control",
                                    value: v,
                                });
                            }
                            v
                        }
                        _ => "127.0.0.1:0".to_owned(),
                    };
                    cli.control_http = Some(addr);
                }
                "--control-unix" => {
                    let v = value("--control-unix")?;
                    if v.is_empty() {
                        return Err(CliError::BadValue {
                            flag: "--control-unix",
                            value: v,
                        });
                    }
                    cli.control_unix = Some(PathBuf::from(v));
                }
                "--token" => {
                    let v = value("--token")?;
                    if v.is_empty() {
                        return Err(CliError::BadValue {
                            flag: "--token",
                            value: v,
                        });
                    }
                    cli.token = Some(v);
                }
                "--paused" => cli.paused = true,
                "--control-mode" => {
                    let v = value("--control-mode")?;
                    cli.control_mode = Some(crate::control::ExecMode::parse(&v).ok_or(
                        CliError::BadValue {
                            flag: "--control-mode",
                            value: v,
                        },
                    )?);
                }
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
                "--fps-window" => {
                    let v = value("--fps-window")?;
                    let parsed = v
                        .split_once("..")
                        .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
                        .filter(|(a, b)| a < b)
                        .ok_or(CliError::BadValue {
                            flag: "--fps-window",
                            value: v,
                        })?;
                    cli.fps_window = Some(parsed);
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
            "--fps-report",
            "--fps-window",
            "5450..8500",
        ])
        .unwrap();
        assert_eq!(cli.screenshots.at, [4700, 5450]);
        assert_eq!(cli.screenshots.every, Some(300));
        assert!(Cli::parse(["--screenshot-every", "0"]).is_err());
        assert!(cli.headless && cli.record && cli.audio && cli.repl && cli.stub && cli.fps_report);
        assert_eq!(cli.fps_window, Some((5450, 8500)));
        assert_eq!(cli.trace, Trace::Insns);
        assert_eq!(cli.snapshot_every, Some(64));
        assert_eq!(cli.route.unwrap().to_str().unwrap(), "r.route");
        assert_eq!(cli.data_root.unwrap().to_str().unwrap(), "/tmp/dr");
    }

    #[test]
    fn fps_window_rejects_missing_or_malformed_ranges() {
        assert!(matches!(
            Cli::parse(["--fps-window"]).unwrap_err(),
            CliError::MissingValue("--fps-window")
        ));
        for bad in ["100", "100..", "..200", "200..100", "100..100", "a..b"] {
            assert!(
                matches!(
                    Cli::parse(["--fps-window", bad]).unwrap_err(),
                    CliError::BadValue {
                        flag: "--fps-window",
                        ..
                    }
                ),
                "expected refusal for `{bad}`"
            );
        }
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

    #[test]
    fn control_flags_parse_and_validate() {
        let cli = Cli::parse(["--control-stdio", "--control-mode", "live", "--paused"]).unwrap();
        assert!(cli.control_enabled());
        assert!(cli.control_stdio);
        assert_eq!(cli.control_mode, Some(crate::control::ExecMode::Live));
        assert!(cli.paused);

        let step_cli = Cli::parse(["--control-mode", "step"]).unwrap();
        assert!(step_cli.control_enabled());
        assert_eq!(step_cli.control_mode, Some(crate::control::ExecMode::Step));

        assert!(matches!(
            Cli::parse(["--control-mode"]).unwrap_err(),
            CliError::MissingValue("--control-mode")
        ));
        assert!(matches!(
            Cli::parse(["--control-mode", "turbo"]).unwrap_err(),
            CliError::BadValue {
                flag: "--control-mode",
                ..
            }
        ));

        let http_default = Cli::parse(["--control", "--headless"]).unwrap();
        assert!(http_default.control_enabled());
        assert!(http_default.has_control_transport());
        assert_eq!(http_default.control_http.as_deref(), Some("127.0.0.1:0"));
        assert!(http_default.headless);

        let http_explicit = Cli::parse([
            "--control",
            "0.0.0.0:7777",
            "--control-unix",
            "/tmp/lego.sock",
            "--token",
            "s3cr3t",
        ])
        .unwrap();
        assert_eq!(http_explicit.control_http.as_deref(), Some("0.0.0.0:7777"));
        assert_eq!(
            http_explicit.control_unix.as_deref(),
            Some(std::path::Path::new("/tmp/lego.sock"))
        );
        assert_eq!(http_explicit.token.as_deref(), Some("s3cr3t"));

        let http_eq = Cli::parse(["--control=127.0.0.1:8080"]).unwrap();
        assert_eq!(http_eq.control_http.as_deref(), Some("127.0.0.1:8080"));

        assert!(Cli::parse(["--control", "badaddr"]).is_err());
        assert!(Cli::parse(["--control-unix"]).is_err());
        assert!(Cli::parse(["--token"]).is_err());
    }
}
