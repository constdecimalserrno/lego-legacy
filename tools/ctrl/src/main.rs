//! `lego-ctrl` CLI binary entry point (#312).

use lego_ctrl::cli::{execute_cli, parse_args};
use std::process::ExitCode;

fn main() -> ExitCode {
    let parsed = match parse_args(std::env::args().skip(1)) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("lego-ctrl: {err}");
            return ExitCode::FAILURE;
        }
    };

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdin_lock = stdin.lock();
    let mut stdout_lock = stdout.lock();

    match execute_cli(&parsed, &mut stdin_lock, &mut stdout_lock) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lego-ctrl: {err}");
            ExitCode::FAILURE
        }
    }
}
