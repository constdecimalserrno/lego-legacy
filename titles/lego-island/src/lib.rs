//! LEGO Island: the roster's fourth title (#112).

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::LEGO_ISLAND as ENTRY;
use lego_legacy_core::title::{Declared, Installed, WINDOWS_95};
use lego_legacy_core::Title;

/// Where this title's installer puts it: `%PROGRAM_FILES%\%MAINDIR%`, with
/// `MAINDIR` set to `LEGO Island`, read out of the Wise installer's own
/// script on the disc (`INSTALL.EXE`).
const INSTALL_DIR: &str = "C:\\Program Files\\LEGO Island";

/// The key the installer writes and the game reads its paths back from.
const KEY: &str = "HKLM\\SOFTWARE\\Mindscape\\LEGO Island";

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_ISLAND: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. ADR-0003 gives a real title no
    // free frame, and this title has not been seen to author a rate of its
    // own: the old tree found no `rdtsc` in either image and no frame time in
    // its data. So it is the roster's baseline sixty, which is also what VGA's
    // own 640x480 timing refreshes at.
    flip_cost: FlipCost::Refresh(60),
    // Started as a player double-clicking it would start it, with its own
    // quoted path and nothing after it, and not with the command line its
    // settings launcher passes, `isle.exe /diskstream /script
    // \lego\scripts\isle\isle.si` (`LEGOIsle.exe` 0x40309b). Nothing reads
    // those arguments: neither `ISLE.EXE` nor `LEGO1.DLL` holds the string
    // `/diskstream`, and the one `/script` in `LEGO1.DLL` is what its own
    // script command passes to a program it starts (RVA 0x3e5aa), not
    // something it looks for. The old tree measured the same: the same
    // picture at flip 6,000 with them and without (#517 there). So golden
    // rule 2 asks nothing more of the command line, and the machine has no
    // field for one.
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        // Windows 95 as it was sold, 4.00.950: what this pressing shipped
        // against, a year before Windows 98. Its box asks for it, its disc
        // carries DirectX 5 for it and a help file for its problems on it
        // (`W95TECHS.HLP`). The same rule the other titles' Windows 98 SE
        // comes from.
        windows: WINDOWS_95,
        // The roster's baseline Pentium II 450. The old tree saw this title
        // measure nothing: no `rdtsc` in either image. `LEGO1.DLL` holds one
        // `cpuid`, leaves 0 and 1 at RVA 0x9d1b9; whether it runs, and what
        // it picks from the answer, is unmeasured, and this value is the
        // baseline until it is.
        hz: lego_legacy_core::clock::DECLARED_HZ,
        processor: Processor::PentiumII,
        // The roster's baseline, and never asked: neither `ISLE.EXE` nor
        // `LEGO1.DLL` imports `GlobalMemoryStatus`, the one call the figure
        // reaches a title through.
        megabytes: 128,
        // 640x480 at sixteen bits: the depth this title picks when `Display
        // Bit Depth` is absent from the registry, which it is here. Its
        // constructor sets the sixteen-bit flag (`ISLE.EXE` 0x401064), and the
        // registry read at 0x402a9d only changes it for a value of 8 or 16.
        // Not yet seen asked for: the title has not set a mode on this
        // machine yet.
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        // None: no gate run asks for a library, and the two places it can,
        // the C runtimes' message boxes, ask for USER32, which core writes (#146).
        absent_libraries: &[],
        why: "Windows 95 is what this pressing shipped against; the rest is the roster's \
              baseline, unmeasured against this title because it has not asked for any of it \
              yet",
    },
    // No cabinet on this disc: the Wise installer copies DATA\disk whole, and
    // the declared tree says so rather than unpacking anything. Beside it the
    // installer copies its own uninstaller, the disc's help file and
    // Mindscape's registration program, none of them the game's.
    installed: Installed::Copied("DATA/disk"),
    // What the installer writes under `SOFTWARE\Mindscape\LEGO Island`, read
    // out of its script: the three paths, and nothing else. The game asks for
    // sixteen values there, measured here as in the old tree (#321 there),
    // and takes its own compiled default for each one it does not find; the
    // other thirteen are the settings dialog's, which the installer never
    // writes and the launcher is not run to write.
    //
    // Each path has no trailing backslash: the game puts `\lego\…` straight
    // onto it, and a doubled separator is a different path on the declared
    // tree.
    registry: &[
        // Where the game finds its install: `LEGO\data\WORLD.WDB` and the rest.
        (KEY, "diskpath", INSTALL_DIR),
        // Where it finds its scripts: the disc in the declared drive. The
        // installer writes the directory it was run from, which is the disc's
        // root.
        (KEY, "cdpath", "D:"),
        // Where it keeps its saves: the install directory, as the installer
        // writes it.
        (KEY, "savepath", INSTALL_DIR),
    ],
    install_dir: INSTALL_DIR,
    // Not yet measured: no movie of this title's own has played on this
    // machine, so there is nothing a click could be translated inside of.
    click_in_a_movie: None,
    // One for one, because the pointer is never the title's to steer (#124).
    // `Draw Cursor` is absent from the registry, so the game takes its own
    // default, off, and leaves the pointer to Windows: it never hides it,
    // never puts it anywhere, and reads it only as the place each mouse
    // message carries. The host's pointer stays the person's, and a declared
    // speed only acts on a pointer a title has taken.
    pointer_speed: (1.0, 1.0),
};
