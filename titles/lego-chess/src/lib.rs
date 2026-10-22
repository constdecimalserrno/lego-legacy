//! LEGO Chess: the roster's fifth title (#142).

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::LEGO_CHESS as ENTRY;
use lego_legacy_core::title::{Declared, Installed, Kept, WINDOWS_98_SE};
use lego_legacy_core::Title;

/// The archive the roster keeps the game in, which the setup expands.
const ARCHIVE: &str = match ENTRY.image.kept {
    Kept::Imploded(archive) => archive,
    _ => panic!("LEGO Chess keeps its game imploded in an archive"),
};

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_CHESS: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. This title authors no frame
    // rate: no wait for a vertical blank was found in its code, and it moves
    // everything by the clock, through `timeGetSystemTime` and its own
    // millisecond timer. So it is the roster's baseline sixty.
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        // The roster's baseline. All this title asks of its Windows is
        // whether it is NT, through `GetVersionExA`: on NT it would skip its
        // DirectX plugin and read keys with `GetAsyncKeyState`.
        windows: WINDOWS_98_SE,
        // The roster's baseline Pentium II 450, for now. This title spins on
        // the clock, half a second and then a second at boot and between its
        // movies' frames, so what a declared second is worth in work decides
        // what those waits cost; #147 measures them and settles the value.
        // It never asks `cpuid`, and its two `rdtsc`s (0x42E1CE, 0x42E1E7) are
        // in one helper, 0x42E1C0, that nothing calls.
        hz: lego_legacy_core::clock::DECLARED_HZ,
        processor: Processor::PentiumII,
        // The roster's baseline, and never asked: the game imports no
        // `GlobalMemoryStatus`.
        megabytes: 128,
        // 640x480 at sixteen bits: the mode the game sets unless its own
        // configuration says otherwise, and nothing does on first run.
        modes: &[(640, 480, 16)],
        // Its disc's long names, as the declared CD answers them from its
        // Joliet tree (#144): after its intro the game opens `D:\intro\Chess Bumper01.bmp`, a name only
        // the Joliet tree has, the primary tree calling it `CHESSB~1.BMP`
        // (#144). By #142's rule Joliet is this title's alone until the
        // maintainer decides for every title.
        joliet: true,
        computer: "LEGO-LEGACY",
        // The two libraries its renderer list asks for and this machine has
        // not got, each told it is not there (NULL, 1157 on Windows 9x;
        // #146), so the game passes over them and its own auto-select picks
        // its DirectX plugin. Anything else it asks for that is neither
        // shipped nor written stops the run at a wall.
        absent_libraries: &[
            (
                "glide2x.dll",
                "3dfx's Glide, which its Glide plugin loads, and the declared machine has no \
                 3dfx card",
            ),
            (
                "KLIB_OpenGL_PlugIn.dll",
                "the OpenGL renderer its plugin table names, which neither pressing ever \
                 shipped",
            ),
        ],
        // A 1998 PC had a floppy drive, and the game's CD check (0x405420)
        // needs one: it tests bit 0 of `GetLogicalDrives`, A:, once and never
        // shifts, so with no A: it looks at no drive and asks for its disc
        // for ever. With one it reaches D: (#145). It asks A: only its type.
        floppy: true,
        why: "the roster's baseline, but for what the game itself needs: its disc's long names \
              read through Joliet, the two renderer libraries it asks for and no 1998 machine \
              had, and a floppy drive, without which its CD check looks at no drive. Of its \
              Windows it asks only whether it is NT, and the processor waits on #147's \
              measuring the clock waits it spends at boot",
    },
    // Nothing of the game is loose on its disc. Its setup expands MAIN.Z, an
    // InstallShield 3 archive, into the install directory whole, and copies
    // `Text.txt`, the game's own strings, out of its own library beside it;
    // without it the New Player form faults (#143). A Minimum install (#142):
    // the three `.RES` archives, 170 MB, stay on the disc, and the game reads
    // them from the drive it finds its disc in.
    installed: Installed::Expanded {
        archive: ARCHIVE,
        beside: &[("_SETUP.LIB", "Text.txt")],
    },
    // The game reads nothing its setup writes. The one key it opens is its
    // own `HKCU\Software\LEGO Chess`, for its video settings, and it writes
    // that itself.
    registry: &[],
    // Where its setup puts it: `PROGRAMFILES` and `LEGO Media\Games\LEGO
    // Chess`, read out of its script, `SETUP.INS`.
    install_dir: "C:\\Program Files\\LEGO Media\\Games\\LEGO Chess",
    // Not yet measured: no movie of this title's own has played on this
    // machine.
    click_in_a_movie: None,
    // One for one, not yet played at a window. The game hides the Windows
    // pointer (`ShowCursor(FALSE)`, 0x40ADA0) and draws its own, so the host
    // steers it by how far the mouse goes and this is the value that steers
    // it. It draws it at the place each `WM_MOUSEMOVE` carries, kept whole
    // (0x457830 stores its x and y), so a move should be drawn as far as it
    // went, as Hot Wheels' is (#37). It also puts the pointer at its camera
    // presets with `SetCursorPos` (0x457810, from 0x4113C0), which #157
    // measures.
    pointer_speed: (1.0, 1.0),
};
