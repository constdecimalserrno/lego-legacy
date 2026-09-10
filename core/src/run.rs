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
    let finish = machine.run(host, &mut log, budget);
    Ok((finish, log))
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
        eprintln!("lego-legacy: the debugger is not built yet (#14)");
        return ExitCode::FAILURE;
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
    match run(title, cli, &mut host, u64::MAX) {
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
