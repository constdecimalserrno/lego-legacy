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
        .assert_logged("installed 61 files");
}

/// An MSVC image reads and writes `fs:[0]` before it does anything else, and
/// every one of the twelve this title ships is an MSVC image. With `fs` flat
/// that address is guest zero; the declared thread information block is what
/// it is instead (#54).
#[test]
fn fs_is_based_at_the_declared_thread_information_block() {
    if !scratch() {
        return;
    }
    let mut host = lego_legacy_core::host::headless::Headless::new();
    let out = lego_legacy_core::repl::transcript(
        &ALPHA_TEAM,
        &Cli::default(),
        &mut host,
        "regs\nmem 0x00200000 0x38\nquit\n",
    )
    .expect("the title loads");

    assert!(out.contains("fs 0x0000 base 0x00200000"), "{out}");
    // The block itself, as the debugger dumped it.
    let bytes: Vec<u8> = out
        .lines()
        .filter(|l| l.contains("0x0020"))
        .flat_map(str::split_whitespace)
        .filter(|t| t.len() == 2)
        .filter_map(|b| u8::from_str_radix(b, 16).ok())
        .collect();
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!(word(0x00), 0xffff_ffff, "the chain ends where it starts");
    assert_eq!(word(0x04), lego_legacy_core::machine::STACK_TOP);
    assert_eq!(
        word(0x08),
        lego_legacy_core::machine::STACK_TOP - lego_legacy_core::machine::STACK_BYTES
    );
    assert_eq!(word(0x18), 0x0020_0000, "a block points at itself");
    assert_eq!(word(0x20), 0x0000_1234, "the declared process");
    assert_eq!(word(0x24), 0x0000_1000, "the declared thread");
    assert_eq!(word(0x30), 0x0020_1000, "the process block");
}

/// Everything this title ships beside its executable is loaded as guest code,
/// deepest first, and none of them lands where another one is (#55).
#[test]
fn every_binary_the_title_ships_is_loaded_in_the_order_it_has_to_attach() {
    if !scratch() {
        return;
    }
    let ran = testkit::run_headless(&ALPHA_TEAM, &Cli::default(), 200_000);
    let modules: Vec<&str> = ran
        .log
        .lines()
        .filter_map(|l| l.strip_prefix("module "))
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert_eq!(modules.len(), 11, "{modules:?}");

    // The C runtime is under everything, and the piece that reaches for the
    // most is on top.
    assert_eq!(modules.first(), Some(&"MSVCRT.dll"));
    assert_eq!(modules.last(), Some(&"LoadComp.dll"));
    // Each of these comes after what it imports.
    for (module, needs) in [
        ("MSVCP60.dll", "MSVCRT.dll"),
        ("NiMain.dll", "DDPackedDirFileLib.dll"),
        ("NiAnimation.dll", "NiMain.dll"),
        ("BehaviorComp.dll", "BaseClassComp.dll"),
        ("LoadComp.dll", "NiDirect3d.dll"),
    ] {
        let at = |name| modules.iter().position(|m| *m == name);
        assert!(
            at(module) > at(needs),
            "{module} attaches after {needs}: {modules:?}"
        );
    }

    // Nine of the twelve ask to be loaded at the same address, so eight of
    // them went somewhere else and every one is somewhere of its own.
    let mut bases: Vec<&str> = ran
        .log
        .lines()
        .filter_map(|l| l.split(" base ").nth(1))
        .map(|l| l.split_whitespace().next().unwrap_or(""))
        .collect();
    bases.sort_unstable();
    let count = bases.len();
    bases.dedup();
    assert_eq!(bases.len(), count, "no two images share an address");
}

/// The imports are bound to real addresses inside those binaries rather than
/// to places on the boundary: 328 of the entry image's own and the rest its
/// engine's (#55).
#[test]
fn the_names_the_title_imports_are_bound_to_the_code_that_exports_them() {
    if !scratch() {
        return;
    }
    let ran = testkit::run_headless(&ALPHA_TEAM, &Cli::default(), 200_000);
    ran.assert_logged("bound 2576 names");
    // The run is inside the C runtime's own code by the time it stops, which
    // it could only reach by being bound to it.
    assert!(
        ran.log.contains("called from 0x2"),
        "the wall's caller is inside a loaded module: {}",
        ran.log.lines().last().unwrap_or_default()
    );
}

/// A forwarded export stands for one somewhere else, and this loader does not
/// follow one. Nothing on this disc has one, and this is what says so: if a
/// pressing ever does, the following gets written (#55).
#[test]
fn nothing_this_title_ships_forwards_an_export() {
    if !scratch() {
        return;
    }
    let mut tree = tree();
    for name in [
        "NiMain.dll",
        "LoadComp.dll",
        "NiAnimation.dll",
        "NiDirect3d.dll",
        "NiCollision.dll",
        "NiA3dSound.dll",
        "BaseClassComp.dll",
        "BehaviorComp.dll",
        "LegoLogicComp.dll",
        "DDPackedDirFileLib.dll",
        "MSVCRT.DLL",
        "MSVCP60.DLL",
        "a3dapi.dll",
    ] {
        let bytes = tree
            .read(&format!("{ROOT}\\{name}"))
            .unwrap_or_else(|| panic!("{name} is declared"));
        let exports = lego_legacy_core::pe::exports(&bytes).expect("it is a PE image");
        assert!(
            exports.forwards.is_empty() && exports.forwards_by_ordinal.is_empty(),
            "{name} forwards {} exports",
            exports.forwards.len() + exports.forwards_by_ordinal.len()
        );
        assert!(!exports.by_name.is_empty(), "{name} exports something");
    }
}
