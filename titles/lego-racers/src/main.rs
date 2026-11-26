//! LEGO Racers: the roster's sixth title.

use lego_legacy_core::run;
use lego_racers::{GUIDE, LEGO_RACERS};
use std::process::ExitCode;

fn main() -> ExitCode {
    run::main_with_guide(&LEGO_RACERS, &GUIDE)
}
