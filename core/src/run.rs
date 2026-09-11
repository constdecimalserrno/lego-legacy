//! The run seam: the one way a title is run.
//!
//! A title crate's `main` is a call to [`main`] and nothing else. Tests reach
//! the same seam through [`crate::testkit`], so what a test runs is what a
//! player runs.

use crate::cli::{Cli, CliError};
use crate::host::{headless::Headless, Host};
use crate::machine::{Finish, Machine, Outcome};
use crate::runlog::RunLog;
use crate::Title;
use std::process::ExitCode;

/// Where a run keeps what its guest writes: beside the pressing it was made
/// from, unless the run was told to keep it somewhere of its own, which is
/// how a test gives one run a machine nothing has been written on.
pub(crate) fn profile_dir(title: &Title, cli: &Cli) -> std::path::PathBuf {
    cli.data_root.as_ref().map_or_else(
        || crate::disc::profile_dir(title.image.disc_dir),
        |root| root.join("profile"),
    )
}

/// Run one title on one host, from a parsed command line.
///
/// # Errors
/// When the machine cannot be built, or the run cannot be set up. A guest that
/// stops at a wall is not an error here: it is an [`Outcome`].
pub fn run(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
    budget: u64,
) -> Result<(Finish, RunLog), Box<dyn std::error::Error>> {
    run_digested(title, cli, host, budget).map(|(finish, log, _)| (finish, log))
}

/// The run seam, with the digest a determinism gate compares.
///
/// # Errors
/// As [`run`].
pub fn run_digested(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
    budget: u64,
) -> Result<(Finish, RunLog, String), Box<dyn std::error::Error>> {
    let mut log = RunLog::create(title.key, cli.trace)?;
    log.line(&format!("headless {}", cli.headless));
    let mut machine = Machine::new(title, host)?;
    if cli.stub {
        machine.load_stub()?;
    } else {
        machine.load_title(title)?;
        if let Some(image) = &machine.image {
            log.line(&format!(
                "image {} base {:#010x} entry {:#010x} imports {}",
                title.image.path,
                image.base,
                image.base.wrapping_add(image.entry),
                image.imports.len()
            ));
        }
    }
    // A route is the run's only recorded input (ADR-0003).
    if let Some(path) = &cli.route {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("no route at {}: {e}", path.display()))?;
        let route =
            crate::route::Route::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        log.line(&format!("route {} events {}", path.display(), route.len()));
        machine.route = Some(route);
    }
    if cli.record {
        machine.recording = Some(crate::route::Route::new());
    }
    machine.screenshots = cli.screenshots.clone();
    machine.tree.set_profile(profile_dir(title, cli));

    // Snapshots are off unless asked for: scrubbability costs nothing until a
    // person wants it.
    let finish = if let Some(every) = cli.snapshot_every {
        run_with_snapshots(&mut machine, host, &mut log, budget, every)
    } else {
        machine.run(host, &mut log, budget)
    };

    // A run that stopped at a wall leaves behind everything needed to reach it
    // again: what was done, where it stopped, and the last snapshot before it.
    if let crate::machine::Outcome::Wall { what } = &finish.outcome {
        let dir = log.dir().to_path_buf();
        let mut report = format!(
            "#lego-legacy-crash 1\ntitle {}\nwall {what}\ncursor {}\nflips {}\n",
            title.key, finish.cursor, finish.flips
        );
        let mut route = crate::route::Route::new();
        for (flip, event) in &machine.input {
            route.push(*flip, *event);
        }
        report.push('\n');
        report.push_str(&route.to_text());
        let _ = std::fs::write(dir.join("crash"), report);
        if let Ok(bytes) = machine.snapshot() {
            let _ = std::fs::write(dir.join("crash.snapshot"), bytes);
        }
        log.line("crash written");
    }

    // What the route asked for after the run's last flip was never delivered,
    // and saying so is the difference between a route that ended and one that
    // was cut short.
    if let Some(route) = &machine.route {
        let undelivered = route.after(finish.flips.saturating_sub(1));
        if !undelivered.is_empty() {
            log.line(&format!(
                "route undelivered {} events after flip {}",
                undelivered.len(),
                finish.flips
            ));
        }
    }
    if let Some(recording) = &machine.recording {
        let path = log.dir().join("route");
        let _ = std::fs::write(&path, recording.to_text());
        log.line(&format!(
            "recorded {} events to {}",
            recording.len(),
            path.display()
        ));
    }
    let digest = machine.state_digest();
    log.line(&format!("digest {digest}"));
    Ok((finish, log, digest))
}

/// What a title binary's `main` calls.
///
/// Reads the process arguments, chooses a host, runs, and reports. The exit
/// status is the guest's own when it ended the run itself.
#[must_use]
pub fn main(title: &'static Title) -> ExitCode {
    let cli = match Cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(CliError::Help) => {
            println!("{}", crate::cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            eprintln!("{}", crate::cli::USAGE);
            return ExitCode::FAILURE;
        }
    };
    if cli.repl {
        return debugger(title, &cli);
    }
    if !cli.headless {
        return windowed(title, &cli);
    }
    let mut host = Headless::new();
    match run(title, &cli, &mut host, u64::MAX) {
        Ok((finish, log)) => {
            println!("{}: {}", title.key, finish.outcome);
            println!(
                "cursor {} flips {} declared-ms {} log {}",
                finish.cursor,
                finish.flips,
                finish.declared_millis,
                log.dir().join("log").display()
            );
            match finish.outcome {
                Outcome::Exited { code } => {
                    ExitCode::from(u8::try_from(code & 0xff).unwrap_or(0xff))
                }
                _ => ExitCode::FAILURE,
            }
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Run a title in a window on the host's screen.
///
/// The window stays open after the guest has finished, because a frame that
/// flashes past is not something a person can look at.
#[cfg(target_os = "macos")]
fn windowed(title: &'static Title, cli: &Cli) -> ExitCode {
    use crate::host::mac::MacWindow;

    // The window opens at the declared machine's own screen size; the guest's
    // frame is scaled into it, its aspect kept, and the guest is never told.
    let mut host = match MacWindow::open(title.key, 640, 480) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lego-legacy: no window: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The mix runs either way; `--audio` only lets it be heard, and a Mac with
    // no output to open runs silent rather than not at all.
    if cli.audio {
        if let Err(e) = host.open_audio() {
            eprintln!("lego-legacy: --audio: running silent, no audio device: {e}");
        }
    }
    let ran = run(title, cli, &mut host, u64::MAX);
    host.close_audio();
    match ran {
        Ok((finish, log)) => {
            println!("{}: {}", title.key, finish.outcome);
            println!(
                "cursor {} flips {} declared-ms {} log {}",
                finish.cursor,
                finish.flips,
                finish.declared_millis,
                log.dir().join("log").display()
            );
            if finish.flips > 0 {
                println!("close the window to end");
                while !host.closed() {
                    host.pump();
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            match finish.outcome {
                Outcome::Exited { code } => {
                    ExitCode::from(u8::try_from(code & 0xff).unwrap_or(0xff))
                }
                _ => ExitCode::FAILURE,
            }
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn windowed(_title: &'static Title, _cli: &Cli) -> ExitCode {
    eprintln!("lego-legacy: this operating system has no window backend yet; run with --headless");
    ExitCode::FAILURE
}

/// Drive a title from the debugger on standard input.
fn debugger(title: &'static Title, cli: &Cli) -> ExitCode {
    let mut host = crate::host::headless::Headless::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout();
    match crate::repl::drive(title, cli, &mut host, &mut input, &mut output) {
        Ok(finish) => {
            println!("{}: {}", title.key, finish.outcome);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Run, writing a snapshot every so many flips.
fn run_with_snapshots(
    machine: &mut crate::machine::Machine,
    host: &mut dyn Host,
    log: &mut RunLog,
    budget: u64,
    every: u64,
) -> Finish {
    let dir = log.dir().to_path_buf();
    let mut next_at = every.max(1);
    let outcome = loop {
        if machine.clock.cursor() >= budget {
            break Outcome::BudgetExhausted;
        }
        if machine.flips() >= next_at {
            match machine.snapshot() {
                Ok(bytes) => {
                    let name = format!("flip-{}.snapshot", machine.flips());
                    let _ = std::fs::write(dir.join(&name), bytes);
                    log.line(&format!(
                        "snapshot {name} cursor {}",
                        machine.clock.cursor()
                    ));
                }
                Err(fault) => log.line(&format!("snapshot refused: {fault}")),
            }
            next_at += every.max(1);
        }
        if let Some(outcome) = machine.tick(host, log) {
            break outcome;
        }
    };
    machine.finish(outcome)
}
