//! LEGO Rock Raiders: the roster's first title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::{run, Title};
use std::process::ExitCode;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
const ROCK_RAIDERS: Title = Title {
    key: "lego-rock-raiders",
    // UNMEASURED. This title's drawing has not been priced yet, and a made-up
    // price would be a fact the clock spends. #9 measures it, against the first
    // movie frame this title presents.
    flip_cost: FlipCost::Free,
};

fn main() -> ExitCode {
    run::main(&ROCK_RAIDERS)
}
