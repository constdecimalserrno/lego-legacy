//! LEGO Rock Raiders: the roster's first title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::roster::ROCK_RAIDERS as ENTRY;
use lego_legacy_core::title::{Declared, Installed, WINDOWS_98_SE};
use lego_legacy_core::{run, Title};
use std::process::ExitCode;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const ROCK_RAIDERS: Title = Title {
    key: ENTRY.key,
    // A flip waits for the declared display's next refresh, which is what
    // DirectDraw's Flip does unless a title asks otherwise, and this one does
    // not (#9). Sixty a second is what GetMonitorFrequency declares. Its movies
    // pace themselves by waiting for each frame, and every frame they ask for
    // lands on a refresh.
    flip_cost: FlipCost::Refresh(60),
    // What this title is, rather than what machine it runs on: the launcher
    // needs the same facts, so they live in the roster and are read from here.
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        // The roster's baseline processor, which this title has never been
        // seen to measure or to wait on.
        hz: lego_legacy_core::clock::DECLARED_HZ,
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
        why: "Windows 98 SE is what this pressing shipped against and what its own installer \
              checks for; the rest is the roster's baseline, unmeasured against this title \
              because it has not asked for any of it yet",
    },
    // This pressing ships its data in an InstallShield cabinet, which the
    // declared tree unpacks from (#16).
    installed: Installed::Cabinet,
    // Its installer writes nothing this title reads back.
    registry: &[],
    // Where this title's installer puts it, spelled the way the guest will
    // read it back from the machine.
    install_dir: "C:\\LEGORR",
    // This title takes a click during a movie itself, measured (#67): it
    // lets go of the movie on the flip the click lands. Nothing to translate.
    click_in_a_movie: None,
    // One for one, and never used: this title's pointer is a place rather
    // than a distance, so the host hands it where the person's own pointer is
    // and there is nothing to steer (#31, #46).
    pointer_speed: (1.0, 1.0),
};

fn main() -> ExitCode {
    run::main(&ROCK_RAIDERS)
}
