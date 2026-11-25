//! LEGO Island: the roster's fourth title.

use lego_island::{GUIDE, LEGO_ISLAND};
use lego_legacy_core::run;
use std::process::ExitCode;

fn main() -> ExitCode {
    run::main_with_guide(&LEGO_ISLAND, &GUIDE)
}
