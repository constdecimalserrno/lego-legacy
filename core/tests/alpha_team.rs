//! LEGO Alpha Team, from the rip to first light (#50).
//!
//! Everything here goes through the run seam or the declared tree: this title
//! ships nothing of itself loose on its disc, so what it is made of only
//! exists once the cabinet has been read.

use lego_legacy_core::cli::Cli;
use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::disc;
use lego_legacy_core::install::Tree;
use lego_legacy_core::roster::ALPHA_TEAM as ENTRY;
use lego_legacy_core::testkit;
use lego_legacy_core::title::{Declared, Installed, Title, WINDOWS_98_SE};

const ROOT: &str = "C:\\Program Files\\LEGO Media\\LEGO Alpha Team";

/// The title as its own crate declares it.
const ALPHA_TEAM: Title = Title {
    key: ENTRY.key,
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        hz: lego_legacy_core::clock::DECLARED_HZ,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "the title crate's own, repeated here because a test crate cannot depend on a \
              binary",
    },
    installed: Installed::Cabinet,
    registry: &[(
        "HKLM\\SOFTWARE\\LEGO Media\\LEGO Alpha Team",
        "Version",
        "1.0",
    )],
    install_dir: ROOT,
};

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
    std::env::set_var("LEGO_LEGACY_GAME_DATA", &root);

    let present = disc::pressing_path(ENTRY.image.disc_dir, ENTRY.image.pressing).is_file();
    if !present {
        let required = std::env::var("LEGO_LEGACY_REQUIRE_DISC").is_ok_and(|v| v != "0");
        assert!(
            !required,
            "LEGO_LEGACY_REQUIRE_DISC is set but the rips are absent"
        );
        eprintln!("skipping: no rips under {}", root.display());
    }
    present
}

/// The declared tree over this title's cabinet, as the machine builds it.
fn tree() -> Tree {
    let mut iso = disc::open_pressing(ENTRY.image.disc_dir, ENTRY.image.pressing)
        .expect("the pressing is there");
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let mut tree = Tree::new();
    tree.set_cabinet(&header, "DATA", ROOT)
        .expect("the cabinet reads");
    tree.set_disc(iso);
    tree
}

/// The title's executable, its engine and its archives all come out of the
/// cabinet: nothing of this title is a file on its disc (#53).
#[test]
fn the_declared_tree_is_the_cabinet() {
    if !scratch() {
        return;
    }
    let mut tree = tree();
    for name in [
        "LEGOATeam.exe",
        "NiMain.dll",
        "LoadComp.dll",
        "NiDirect3d.dll",
        "NiA3dSound.dll",
        "DDPackedDirFileLib.dll",
        "MSVCRT.DLL",
        "MSVCP60.DLL",
        "a3dapi.dll",
        "Fin.pac",
        "Map.pac",
    ] {
        assert!(tree.holds(&format!("{ROOT}\\{name}")), "{name} is declared");
    }
    // The three the installer would have written rather than copied are named
    // by the header and hold nothing (#52).
    assert_eq!(tree.read(&format!("{ROOT}\\Config.txt")), Some(Vec::new()));
    assert_eq!(tree.read(&format!("{ROOT}\\Game.txt")), Some(Vec::new()));
}

/// The cabinet names `LMap.pac` twice, once per install size. The declared
/// machine had the Typical install, so the hi-res lightmaps are what is behind
/// that name (#53).
#[test]
fn the_declared_install_is_the_typical_one() {
    if !scratch() {
        return;
    }
    let lightmaps = tree()
        .read(&format!("{ROOT}\\LMap.pac"))
        .expect("the lightmaps are declared");
    assert_eq!(lightmaps.len(), 53_475_662, "the hi-res lightmaps");
}

/// The image comes out of the cabinet, is hashed there, and runs: the wall it
/// reaches is the C runtime the disc ships, which is the next ticket's (#53).
#[test]
fn the_title_starts_from_its_cabinet_and_walls_by_name() {
    if !scratch() {
        return;
    }
    testkit::run_headless(&ALPHA_TEAM, &Cli::default(), 100_000)
        .assert_logged("image LEGOATeam.exe")
        .assert_logged("imports 328")
        .assert_logged("installed 61 files")
        .assert_wall("MSVCRT.dll!");
}

/// An MSVC image installs an exception frame on `fs:[0]` before it does
/// anything else. With `fs` flat that is guest address zero; with the declared
/// thread information block it is the block's own first word (#54).
#[test]
fn the_first_thing_the_title_does_is_push_a_frame_onto_its_thread_block() {
    if !scratch() {
        return;
    }
    let mut host = lego_legacy_core::host::headless::Headless::new();
    let out = lego_legacy_core::repl::transcript(
        &ALPHA_TEAM,
        &Cli::default(),
        &mut host,
        // Eight instructions in, the prologue has pushed its frame and
        // pointed the chain head at it.
        "until cursor 8\nregs\nmem 0x00200000 8\nquit\n",
    )
    .expect("the title loads");

    // The chain head is the frame the guest just pushed, which is on its own
    // stack rather than at the arena's bottom.
    let head = out
        .lines()
        .find(|l| l.contains("00200000"))
        .expect("the block reads");
    assert!(
        head.contains("dc ff 1f 00"),
        "the chain head points at the frame on the stack: {head}"
    );
    // Which is also where esp is: the frame the prologue pushed.
    assert!(out.contains("esp 0x001fffdc"), "{out}");
    // And what it read out of the block first was the end of the chain, so
    // the read came from the block and not from the arena's bottom.
    assert!(out.contains("eax 0xffffffff"), "{out}");
}
