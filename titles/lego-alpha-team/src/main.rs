//! LEGO Alpha Team: the roster's third title.

use lego_alpha_team::{GUIDE, LEGO_ALPHA_TEAM};
use lego_legacy_core::run;
use std::process::ExitCode;

fn main() -> ExitCode {
    run::main_with_guide(&LEGO_ALPHA_TEAM, &GUIDE)
}
