//! LEGO Chess: the roster's fifth title.

use lego_chess::{GUIDE, LEGO_CHESS};
use lego_legacy_core::run;
use std::process::ExitCode;

fn main() -> ExitCode {
    run::main_with_guide(&LEGO_CHESS, &GUIDE)
}
