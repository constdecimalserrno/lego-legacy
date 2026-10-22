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

/// Enough instructions for the C runtime to start and the game to make its
/// window, and no more: what it asks for after that is the next tickets'
/// (#144–#147).
const BOOT_BUDGET: u64 = 100_000;

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
    // The label the game's CD check compares, byte for byte, at 0x4054E2
    // (#145).
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
