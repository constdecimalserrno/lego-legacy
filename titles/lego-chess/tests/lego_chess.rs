//! LEGO Chess, run headless from the player's rip: the launcher finding it
//! or refusing it, its declared tree, and its boot as far as the machine
//! carries it, replayed to the same machine every time.
//!
//! These need the player's own rip under `game-data/`. Without it the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_chess::LEGO_CHESS as CHESS;
use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, hash, testkit, Title};

const ROOT: &str = CHESS.install_dir;

/// Enough instructions for the C runtime to start, the game to make its
/// window and its framework to come up as far as the machine carries it:
/// the boot stops at a wall well inside this (#147).
const BOOT_BUDGET: u64 = 1_000_000;

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&CHESS.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    // It lists the title fifth, after the four released ones.
    assert_eq!(
        roster::ROSTER.iter().position(|e| e.key == CHESS.key),
        Some(4)
    );
    if !scratch() {
        return;
    }
    // Which means it read the game out of MAIN.Z and found its bytes.
    assert_eq!(
        roster::look_for(&roster::LEGO_CHESS),
        Found::Runnable {
            pressing: "masterpiece-chess".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..CHESS.image
        },
        ..CHESS
    };
    let err = testkit::try_run_headless(&ABSENT, &Cli::default(), 1_000)
        .expect_err("there is no such pressing");

    assert!(
        err.contains("no-such-pressing"),
        "the refusal names the path: {err}"
    );
    assert!(
        err.contains("ripped from your own disc"),
        "and says what to do: {err}"
    );
}

#[test]
fn its_machine_reads_long_names_and_lacks_glide_and_the_opengl_plugin() {
    if !scratch() {
        return;
    }
    // Its bumper is named only in the disc's Joliet tree (#144), and the
    // machine reads the disc through it.
    let mut tree = testkit::tree(&CHESS);
    assert!(tree.lies("D:\\intro\\Chess Bumper01.bmp").is_some());
    let mut absent: Vec<&str> = CHESS
        .declared
        .absent_libraries
        .iter()
        .map(|&(name, _)| name)
        .collect();
    absent.sort_unstable();
    assert_eq!(absent, ["KLIB_OpenGL_PlugIn.dll", "glide2x.dll"]);
}

#[test]
fn its_disc_carries_the_label_its_cd_check_compares() {
    if !scratch() {
        return;
    }
    // Byte for byte, at 0x4054E2 (#145).
    let iso = disc::open_pressing(CHESS.image.disc_dir, CHESS.image.pressing)
        .expect("the pressing opens");
    assert_eq!(iso.label, "LEGOCHESS");
}

#[test]
fn the_declared_tree_is_main_z_expanded_with_the_game_s_strings_beside_it() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&CHESS);

    // The game and everything it ships, each expanded when it is read.
    let exe = tree
        .read(&format!("{ROOT}\\Lego Chess.exe"))
        .expect("the game is installed");
    assert_eq!(hash::sha256_hex(&exe), CHESS.image.sha256);
    for name in [
        "RCBOOK.BOK",
        "Advisors\\redking.avi",
        "profiles\\saves\\readme.txt",
    ] {
        assert!(
            tree.readable(&format!("{ROOT}\\{name}")),
            "{name} is installed"
        );
    }
    // Its renderers, as a title looking for them finds them, at their sizes.
    let mut plugins = tree.matching(ROOT, "KLIB_*.dll");
    plugins.sort();
    assert_eq!(
        plugins,
        [
            ("KLIB_DIRECTX_PLUGIN.DLL".to_owned(), 53_248),
            ("KLIB_GLIDE_PLUGIN.DLL".to_owned(), 61_440),
            ("KLIB_SOFTWARE_PLUGIN.DLL".to_owned(), 86_016),
        ]
    );

    // The game's own strings, copied out of the setup's library to the root.
    let text = tree
        .read(&format!("{ROOT}\\Text.txt"))
        .expect("its strings are installed");
    assert_eq!(
        hash::sha256_hex(&text),
        "7c7a5118710a1c0f700c7bd955c76403dfcca5dd05e6ce2708b8f4b492d86d3c"
    );

    // A Minimum install: the scenes' three archives stay on the disc and are
    // read where they lie there.
    for (name, len) in [
        ("PIRATES.RES", 59_352_329),
        ("WESTERN.RES", 69_566_827),
        ("TUTORIAL.RES", 41_677_288),
    ] {
        assert!(
            !tree.holds(&format!("{ROOT}\\{name}")),
            "{name} is not installed"
        );
        assert_eq!(tree.lies(&format!("D:\\{name}")).map(|(_, l)| l), Some(len));
    }
}

/// The run every boot test looks at, done once.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &CHESS, &cli, &[], BOOT_BUDGET)
}

#[test]
fn the_game_loads_out_of_main_z_and_makes_its_window() {
    let Some(ran) = boot() else { return };

    // Every member of MAIN.Z, and Text.txt beside them.
    ran.assert_logged("installed 882 files")
        .assert_logged("image Lego Chess.exe base 0x00400000")
        .assert_logged("imports 197")
        // Its C runtime, linked into it, starts first.
        .assert_logged("call KERNEL32.dll!GetVersion() from 0x00471300")
        // Then its framework registers its class and makes its window.
        .assert_logged("call USER32.dll!CreateWindowExA(0x0, 0x487ba8, 0x0, 0x86c80000");
}

/// Its framework reads where the pointer is confined as it starts, to put it
/// back as it leaves (#147); core's shim tests hold what it is told, the
/// whole screen.
#[test]
fn the_framework_reads_where_the_pointer_is_confined() {
    let Some(ran) = boot() else { return };

    assert!(
        ran.log
            .lines()
            .any(|l| l.starts_with("call USER32.dll!GetClipCursor(")
                && l.contains(" from 0x0045ac3c ")),
        "its framework asks where the pointer is confined\nlog:\n{}",
        ran.log
    );
}

/// Its input and its sound come up (#149). DirectInput's keyboard and mouse
/// are each set to `DISCL_NONEXCLUSIVE|DISCL_FOREGROUND` and acquired, which
/// the machine already answered. Its 3D listener is set all at once, and the
/// CD audio device it opens by number opens, with its data disc in the
/// drive, as Windows' does, and counts in tracks.
#[test]
fn its_input_and_sound_come_up() {
    let Some(ran) = boot() else { return };

    for device in ["keyboard", "mouse"] {
        ran.assert_logged(&format!("dinput {device} at "));
    }
    // Its framework sets each to the foreground, not exclusively, and acquires
    // it as it starts; its window procedure acquires them again whenever it
    // is told it is in front (0x0045C646), so only these are counted.
    let called = |call: &str, from: &str| {
        assert!(
            ran.log
                .lines()
                .any(|l| l.starts_with(call) && l.contains(&format!(" from {from} "))),
            "{call} from {from}\nlog:\n{}",
            ran.log
        );
    };
    let foreground = "call IDirectInputDeviceA::SetCooperativeLevel(0x10010, 0x6)";
    let acquire = "call IDirectInputDeviceA::Acquire()";
    for (call, from) in [
        (foreground, "0x00455e6c"),
        (acquire, "0x00455e9e"),
        (foreground, "0x00455f6e"),
        (acquire, "0x00455fad"),
    ] {
        called(call, from);
    }
    ran.assert_logged("call IDirectSound3DListener::SetAllParameters(")
        .assert_logged("mci command 0x0803 to device 516, flags 0x3102, answered 0")
        .assert_logged("mci command 0x080d to device 1, flags 0x402, answered 0");
}

/// Its plugin walk goes through all four (#148): the DirectX plugin gets
/// Direct3D 2 off DirectDraw's `IDirectDraw2` face, as DirectX 5 hands it
/// out, and lists its devices from it; the Glide plugin finds no
/// `glide2x.dll`; the Software plugin lists its own driver; and the OpenGL
/// plugin is not there.
#[test]
fn the_plugin_walk_gets_direct3d_2() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("com IDirectDraw4 handed back IDirect3D2")
        .assert_logged("call IDirect3D2::EnumDevices(")
        .assert_logged("library missing glide2x.dll")
        .assert_logged("proc address KLIB_Software_HookDll")
        .assert_logged("library missing KLIB_OpenGL_PlugIn.dll");
}

/// The game's main asks the system's language (#147), finds its disc on D:
/// with no word to the player (#145's check, 0x004087AF), is told no screen
/// saver is enabled, and reads its three archives from the disc it found.
/// Its video setup's own auto-select then picks the DirectX plugin's
/// Direct3D 2 device on this machine with a 3D card, takes the screen and
/// makes the device.
#[test]
fn the_game_s_main_finds_its_disc_and_picks_direct3d_2() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("call KERNEL32.dll!GetSystemDefaultLangID() from 0x004086e6")
        .assert_logged("call USER32.dll!SystemParametersInfoA(0x10, 0x0, ")
        .assert_logged("file opened D:\\pirates.res")
        .assert_logged("file opened D:\\tutorial.res")
        .assert_logged("file opened D:\\western.res")
        .assert_logged("call IDirectDraw::SetDisplayMode(0x280, 0x1e0, 0x10)")
        .assert_logged("call IDirect3D2::CreateDevice(");
    assert!(
        !ran.log.contains("MessageBoxA"),
        "the disc is found with no word to the player\nlog:\n{}",
        ran.log
    );
}

/// Its DirectX plugin lists the device's texture formats and makes it a
/// viewport (#148), and its framework then takes the screen: it confines the
/// pointer to it and says a screen saver is running, as programs of the time
/// did to keep Alt+Tab away (#147).
#[test]
fn the_device_gets_its_formats_and_viewport_and_the_framework_takes_the_screen() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("d3d2 texture formats: 6 of them")
        .assert_logged("call IDirect3D2::CreateViewport(")
        .assert_logged("call USER32.dll!ClipCursor(")
        .assert_logged("call USER32.dll!SystemParametersInfoA(0x61, 0x1, ");
}

/// Its framework tells its own window its size, and invalidates every window
/// once the mode is set (#147). The game's first movie clears the screen
/// twice, its first two frames, and its pump takes what is waiting: the
/// window is the active application's, is active, has the keyboard (#196)
/// and is painted. Then it opens its first movie.
#[test]
fn the_window_is_told_its_size_painted_and_focused_and_the_first_movie_opens() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("call USER32.dll!SendMessageA(0x10010, 0x5, 0x0, 0x1e00280)")
        .assert_logged("call USER32.dll!InvalidateRect(0x0, 0x0, 0x1)")
        .assert_logged("call USER32.dll!BeginPaint(0x10010, ")
        .assert_logged("flip 1 cursor ");
    let told: Vec<&str> = ran
        .log
        .lines()
        .filter(|l| l.starts_with("window procedure given message "))
        .collect();
    for message in ["0x001c", "0x0006", "0x0007", "0x000f"] {
        assert!(
            told.iter().any(|l| l.ends_with(message)),
            "its window is given {message}\nlog:\n{}",
            ran.log
        );
    }
}

/// Where the boot stops: the wall the machine names next, which each ticket
/// that answers one moves on.
#[test]
fn the_boot_stops_at_the_wall_it_meets_next() {
    let Some(ran) = boot() else { return };

    // #151's: its first movie, through AVIFile, after its first two frames.
    ran.assert_flips(2)
        .assert_logged("end wall: AVIFIL32.dll!AVIFileOpenA");
}

#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else { return };
    let Some(second) = ({
        static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
        let cli = Cli {
            trace: Trace::Shims,
            ..Cli::default()
        };
        testkit::replay(&AGAIN, scratch, &CHESS, &cli, &[], BOOT_BUDGET)
    }) else {
        return;
    };
    first.assert_same_machine(second);
}

// The machine has the floppy drive its CD check needs: the check at 0x405420
// looks at no drive without A: (#145). Core's shim tests run its calls on a
// machine with one and find D:.
const _: () = assert!(CHESS.declared.floppy);
