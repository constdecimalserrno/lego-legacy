//! LEGO Rock Raiders: the roster's first title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::title::{Declared, EntryImage, WINDOWS_98_SE};
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
    declared: Declared {
        windows: WINDOWS_98_SE,
        // A Pentium II era machine with 128 MB: what the box asked for, and
        // more than this title ever allocates. The figure only reaches the
        // guest through GlobalMemoryStatus, and a title that sized its caches
        // from the host's memory would size them differently on every host.
        megabytes: 128,
        // The screen this title asks for. It renders at one size and scales
        // nothing itself; the window does the scaling (#6).
        modes: &[(640, 480, 16)],
        // Declared rather than the host's own name, for the same reason as
        // everything else here: a recorded run must not depend on the machine
        // it was recorded on.
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "Windows 98 SE is what this pressing shipped against and what its own installer \
              checks for; the rest is the roster's baseline, unmeasured against this title \
              because it has not asked for any of it yet",
    },
    // Where this title's installer puts it, spelled the way the guest will
    // read it back from the machine.
    install_dir: "C:\\LEGORR",
};

fn main() -> ExitCode {
    run::main(&ROCK_RAIDERS)
}
