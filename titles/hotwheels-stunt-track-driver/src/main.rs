//! Hot Wheels Stunt Track Driver: the roster's second title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::roster::HOT_WHEELS as ENTRY;
use lego_legacy_core::title::{Declared, Installed, WINDOWS_98_SE};
use lego_legacy_core::{run, Title};
use std::process::ExitCode;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const HOT_WHEELS_STUNT_TRACK_DRIVER: Title = Title {
    key: ENTRY.key,
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. Sixty a second is the roster's
    // baseline; this title has not yet been seen to ask for anything else.
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "Windows 98 SE is what this pressing shipped against; the rest is the roster's \
              baseline, unmeasured against this title because it has not asked for any of it \
              yet",
    },
    // No cabinet on this disc: the installer copies SETUP/HOT98 whole, and the
    // declared tree says so rather than unpacking anything.
    installed: Installed::Copied("SETUP/HOT98"),
    // What its installer leaves behind for it to find itself by: this title
    // reads both of these back on the way up, and builds every path to its own
    // data out of them.
    registry: &[
        (
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "InstallPath",
            "C:\\Program Files\\Mattel\\Stunt Track Driver",
        ),
        (
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "CDDATADIR",
            // The title puts a file's own name straight onto this one, so it
            // names the directory the data is in, not the one above it.
            "D:\\SETUP\\HOT98\\DATA",
        ),
    ],
    // Where this title's installer puts it. The title reads this path back out
    // of the registry as `InstallPath` under
    // `SOFTWARE\MATTEL\STUNT TRACK DRIVER\1.0`, so it has to be the same on
    // every run and on every host.
    install_dir: "C:\\Program Files\\Mattel\\Stunt Track Driver",
};

fn main() -> ExitCode {
    run::main(&HOT_WHEELS_STUNT_TRACK_DRIVER)
}
