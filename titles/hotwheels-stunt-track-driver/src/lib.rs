//! Hot Wheels Stunt Track Driver: the roster's second title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::HOT_WHEELS as ENTRY;
use lego_legacy_core::title::{Declared, Installed, WINDOWS_98_SE};
use lego_legacy_core::Title;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const HOT_WHEELS_STUNT_TRACK_DRIVER: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. Sixty a second is the roster's
    // baseline; this title has not yet been seen to ask for anything else.
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        // A Pentium 166: what this title's own box asks for, and not the
        // roster's baseline. It matters more here than it does for a title
        // that sleeps: this one waits for its next frame by reading the tick
        // count in a loop, so the declared processor decides how much work a
        // frame of waiting costs, and a machine far faster than the one it
        // was written for spends its whole frame spinning.
        hz: 166_000_000,
        // And that Pentium is told about as itself: family 5, which has no
        // conditional moves. Measured over the race route: this title never
        // asks, by `cpuid` or by the flags (#104).
        processor: Processor::Pentium,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        // The primary tree's names alone, by #142's rule: Joliet is LEGO
        // Chess's alone, because Alpha Team's runs move with it, until the
        // maintainer decides (#144). Nothing in this title asks for it: read
        // through its Joliet tree, as Windows 98 SE's CDFS reads it, this
        // disc answers the same names but for case, and none of its runs
        // moved with it on (measured at 7509b49).
        joliet: false,
        computer: "LEGO-LEGACY",
        // None: no gate run asks for a library, and the one place it can,
        // its C runtime's message box, asks for USER32, which core writes (#146).
        absent_libraries: &[],
        // None, as every released title's machine has had since its routes were
        // recorded: C: and D: only.
        floppy: false,
        why: "Windows 98 SE is what this pressing shipped against and a Pentium 166 is what \
              its box asks for; the rest is the roster's baseline, unmeasured against this \
              title because it has not asked for any of it yet",
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
    // This title plays no movie at all, so there is nothing here for a click
    // to be translated inside of (#68). What gets past its splashes is Space,
    // measured — Escape quits the game outright and a click does nothing —
    // but its splashes and its menu are the same thing at the boundary, so
    // there is no window to scope a translation to and it is not made.
    click_in_a_movie: None,
    // One for one: this title steers its own pointer by what it is handed and
    // keeps none of it back, and a person played it at a window and said the
    // pointer and the mouse move as one (#37).
    pointer_speed: (1.0, 1.0),
};
