//! LEGOLAND: the roster's seventh title.

use lego_legacy_core::run;
use legoland::{GUIDE, LEGOLAND};
use std::process::ExitCode;

fn main() -> ExitCode {
    run::main_with_guide(&LEGOLAND, &GUIDE)
}
