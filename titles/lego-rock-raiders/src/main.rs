//! LEGO Rock Raiders: the roster's first title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{run, Title};
use std::process::ExitCode;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const ROCK_RAIDERS: Title = Title {
    key: "lego-rock-raiders",
    // UNMEASURED. This title's drawing has not been priced yet, and a made-up
    // price would be a fact the clock spends. #9 measures it, against the first
    // movie frame this title presents.
    flip_cost: FlipCost::Free,
    image: EntryImage {
        disc_dir: "rock-raiders",
        pressing: "masterpiece-rock-raiders",
        path: "EXE/LEGORR.EXE",
        sha256: "62b67f5ba656c71412764d343c6a955d218e3b61f0e0f4758ad6086e971e64d4",
        why: "the original pressing's EXE/LEGORR.EXE is a 280 KB SafeDisc wrapper beside a \
              737 KB LEGORR.ICD holding the encrypted game; the Masterpiece pressing ships \
              that same game unwrapped, with an empty .ICD left behind (ADR-0006)",
    },
    // The original pressing carries three audio tracks and the Masterpiece
    // pressing carries none, so the music comes off the disc the executable
    // does not (ADR-0006).
    red_book: Some("rock-raiders"),
};

fn main() -> ExitCode {
    run::main(&ROCK_RAIDERS)
}
