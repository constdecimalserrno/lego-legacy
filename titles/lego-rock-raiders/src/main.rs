//! LEGO Rock Raiders: the roster's first title.

use lego_legacy_core::run;
use lego_rock_raiders::ROCK_RAIDERS;
use std::process::ExitCode;

fn main() -> ExitCode {
    run::main(&ROCK_RAIDERS)
}
