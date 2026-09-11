//! LEGO Alpha Team: the roster's third title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::roster::ALPHA_TEAM as ENTRY;
use lego_legacy_core::title::{Declared, Installed, WINDOWS_98_SE};
use lego_legacy_core::{run, Title};
use std::process::ExitCode;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_ALPHA_TEAM: Title = Title {
    key: ENTRY.key,
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. Sixty a second is the roster's
    // baseline; this title has not yet been seen to ask for anything else.
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        // The roster's baseline Pentium II 450, and it matters here: this
        // title measures the machine itself, in `DDCaps::CalcProcSpeed`, and
        // `DDCaps::MakeRecs` picks what it will draw from the number it gets.
        // A machine declared slower than the one the title shipped against
        // would have it choose a quality nobody asked it to.
        hz: lego_legacy_core::clock::DECLARED_HZ,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "Windows 98 SE is what this pressing shipped against, and the rest is the \
              roster's baseline; the processor is the one value this title reads back as a \
              measurement of its own rather than asking for it",
    },
    // Nothing of this title is loose on its disc: its executable, its engine
    // DLLs and all eight of its archives are inside the pressing's cabinet
    // (#52), and the declared tree unpacks them from there.
    installed: Installed::Cabinet,
    // What its installer leaves behind. The title's own executable opens this
    // key with `RegOpenKeyExA` and reads it back; the version is what the
    // installer's own script writes there.
    registry: &[(
        "HKLM\\SOFTWARE\\LEGO Media\\LEGO Alpha Team",
        "Version",
        "1.0",
    )],
    // Where this title's installer puts it: the shell folder its own script
    // names, under Program Files.
    install_dir: "C:\\Program Files\\LEGO Media\\LEGO Alpha Team",
};

fn main() -> ExitCode {
    run::main(&LEGO_ALPHA_TEAM)
}
