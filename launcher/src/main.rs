//! `lego-legacy`: the one front door.
//!
//! It says which titles are here, which of them can be started, and why the
//! rest cannot. Everything past that belongs to the title's own binary: the
//! launcher forwards what it was given and gets out of the way.

use lego_legacy_core::roster::{self, Found};
use std::io::{BufRead, Write};
use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // A title named on the command line is started; nothing else is asked.
    if let Some(first) = args.first() {
        if let Some(entry) = roster::ROSTER.iter().find(|e| e.key == first) {
            return start(entry.key, &args[1..]);
        }
        if first == "--help" || first == "-h" {
            println!("usage: lego-legacy [<title>] [flags…]");
            println!("       with no title, the titles that are here are listed");
            println!("\nflags are passed to the title's own binary, unchanged");
            return ExitCode::SUCCESS;
        }
        eprintln!("lego-legacy: no supported title is called `{first}`");
        list();
        return ExitCode::FAILURE;
    }

    let found = list();
    let ready: Vec<usize> = found
        .iter()
        .enumerate()
        .filter(|(_, f)| matches!(f, Found::Runnable { .. }))
        .map(|(i, _)| i)
        .collect();
    if ready.is_empty() {
        println!("\nNothing can be started yet. Rip your own discs into game-data/ first.");
        return ExitCode::FAILURE;
    }

    print!("\nWhich one? ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return ExitCode::FAILURE;
    }
    let choice: usize = match line.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= roster::ROSTER.len() => n - 1,
        _ => {
            eprintln!("lego-legacy: not one of the numbers above");
            return ExitCode::FAILURE;
        }
    };
    let entry = roster::ROSTER[choice];
    match &found[choice] {
        Found::Runnable { .. } => start(entry.key, &[]),
        other => {
            eprintln!("lego-legacy: {} cannot be started: {other}", entry.name);
            ExitCode::FAILURE
        }
    }
}

/// Say what is here, and hand back what was found for each title.
fn list() -> Vec<Found> {
    println!("lego-legacy");
    let mut found = Vec::new();
    // The names line up against the longest one on the roster, whatever joins
    // it later.
    let column = roster::ROSTER
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(0);
    for (i, entry) in roster::ROSTER.iter().enumerate() {
        let what = roster::look_for(entry);
        println!("  {}. {:<column$} {what}", i + 1, entry.name);
        // The music may be on a disc the image is not (ADR-0006), and a title
        // starts without it and plays none, so a missing one is said here.
        if let Some(pressing) = entry.red_book {
            match lego_legacy_core::disc::open_red_book(entry.image.disc_dir, pressing) {
                Err(e) => {
                    println!(
                        "     music comes from the {pressing} pressing, and cannot be played: {e}"
                    );
                }
                Ok(_) if pressing != entry.image.pressing => {
                    println!("     music comes from the {pressing} pressing");
                }
                Ok(_) => {}
            }
        }
        found.push(what);
    }
    found
}

/// Start a title's own binary, passing on whatever we were given.
fn start(key: &str, flags: &[String]) -> ExitCode {
    // The title's binary sits beside this one, which is where cargo puts it
    // and where an install would too; on Windows it is `<title>.exe` (#107).
    let beside = std::env::current_exe().ok().and_then(|p| {
        p.parent()
            .map(|d| d.join(format!("{key}{}", std::env::consts::EXE_SUFFIX)))
    });
    let program = match beside {
        Some(path) if path.is_file() => path,
        _ => std::path::PathBuf::from(key),
    };
    match Command::new(&program).args(flags).status() {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(e) => {
            eprintln!("lego-legacy: could not start {}: {e}", program.display());
            ExitCode::FAILURE
        }
    }
}
