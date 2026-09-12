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
    // Nothing the title imports from one of its own binaries is a wall: a name
    // that did not bind would stop the run and say which binary it wanted.
    for library in ["NiMain.dll!", "LoadComp.dll!", "MSVCP60.dll!"] {
        assert!(
            !ran.log.contains(library),
            "no name goes unbound to {library}: {}",
            ran.log.lines().last().unwrap_or_default()
        );
    }
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

/// The C runtime this title's disc ships runs as guest code, so everything it
/// calls into the kernel is a call this machine has to answer. It gets all the
/// way through its own startup and into the title's `main` (#56).
#[test]
fn the_c_runtime_the_disc_ships_gets_what_it_calls() {
    if !scratch() {
        return;
    }
    let ran = testkit::run_headless(&ALPHA_TEAM, &Cli::default(), 2_000_000);
    let end = ran
        .log
        .lines()
        .find(|l| l.starts_with("end "))
        .unwrap_or_default();
    for library in ["MSVCRT.dll!", "MSVCP60.dll!", "KERNEL32.dll!"] {
        assert!(
            !end.contains(library),
            "the run no longer stops inside the runtime or the kernel: {end}"
        );
    }
    // It reaches the title's own code, which goes looking for its archives.
    ran.assert_logged("find first *.pac");
}

/// The title asks DirectDraw for DirectX 7's face of itself, takes Direct3D 7
/// off that, enumerates a device, creates it, and draws (#57).
#[test]
fn the_title_brings_up_a_directx_7_device_and_presents_through_it() {
    if !scratch() {
        return;
    }
    // Far enough to get its renderer up and its first frames out; the run
    // carries on past this and is #58's.
    let ran = testkit::run_headless(&ALPHA_TEAM, &Cli::default(), 600_000_000);
    // Its own words for what happened, which is the strongest thing there is
    // to assert: the title says whether it liked the device.
    ran.assert_logged("Device open succeeded")
        .assert_logged("d3d7 enumerate: one device, and it rasterises")
        .assert_logged("d3d7 z buffers: one, sixteen bits deep")
        .assert_logged("d3d7 texture formats: 6 of them")
        .assert_logged("d3d7 viewport 640x480 at 0,0");
    assert!(
        !ran.log.contains("Render create failed"),
        "the renderer comes up: {}",
        ran.log
            .lines()
            .filter(|l| l.contains("Render create"))
            .collect::<Vec<_>>()
            .join(" / ")
    );
    assert!(ran.finish.flips >= 1, "it presents a frame of its own");
}

/// The archives this title reads everything out of come out of the cabinet
/// byte for byte. Their own hashes are here because the cabinet carries an
/// MD5 of each expanded file and those were checked against these (#58).
#[test]
fn the_archives_unpack_to_exactly_what_the_cabinet_says_they_are() {
    if !scratch() {
        return;
    }
    let mut tree = tree();
    for (name, bytes, digest) in [
        (
            "Bhvr.pac",
            4_532_456,
            "244725abd7fe5b6b3f7200ccc3bee90174ccad6c1eb0fead1cdd861824d1eddc",
        ),
        (
            "Fin.pac",
            20_642_530,
            "1b93b964d12a0114ba25c3712a62b8c8fe5d685d954b07b58b43e6b737c777e1",
        ),
        (
            "Locale_en.pac",
            82_323_720,
            "61156b4fd3d4bb269ac8b009d542f192b989deef96bf35a0d280f36162aabaa6",
        ),
    ] {
        let unpacked = tree
            .read(&format!("{ROOT}\\{name}"))
            .unwrap_or_else(|| panic!("{name} unpacks"));
        assert_eq!(unpacked.len(), bytes, "{name} is as long as it says");
        assert_eq!(
            lego_legacy_core::hash::sha256_hex(&unpacked),
            digest,
            "{name} is what it says it is"
        );
    }
}

/// The title carries its own PE loader: it unpacks a level's code out of its
/// archives and asks for the address that code was linked for. Every one of
/// them was linked for the same address, so only the first can have it, and a
/// machine that says yes to all of them lays each image over the last. The
/// refusal is what sends the loader down its own relocating path (#61).
#[test]
fn a_second_image_asking_for_a_taken_address_is_refused_and_relocated() {
    if !scratch() {
        return;
    }
    // Far enough for the title to load a second level image over the first.
    let ran = testkit::run_headless(&ALPHA_TEAM, &Cli::default(), 1_100_000_000);
    ran.assert_logged("address 0x10000000 is already taken");
    assert!(
        !ran.log.contains("abnormal program termination"),
        "the C runtime no longer gives up on a frame handler reached through \
         code that has been overwritten"
    );
}

/// The flip the title's own picture is first on the screen at: its Loading
/// panel, drawn through Direct3D 7 (#58).
const FIRST_LIGHT: u64 = 1;

/// What it costs to get there. Most of it is the title reading four hundred
/// megabytes of archives and playing its intro movie.
const FIRST_LIGHT_BUDGET: u64 = 600_000_000;

/// The run that reaches first light, done once for every test that looks at
/// it.
fn boot() -> Option<&'static testkit::Ran> {
    use std::sync::OnceLock;
    static BOOT: OnceLock<Option<testkit::Ran>> = OnceLock::new();
    BOOT.get_or_init(|| {
        if !scratch() {
            return None;
        }
        Some(testkit::run_headless(
            &ALPHA_TEAM,
            &Cli::default(),
            FIRST_LIGHT_BUDGET,
        ))
    })
    .as_ref()
}

/// First light: the first frame the title presents under its own control
/// (#58).
#[test]
fn the_title_presents_a_frame_of_its_own() {
    let Some(ran) = boot() else { return };
    let lit = ran
        .host
        .frames
        .iter()
        .find(|f| !f.is_uniform())
        .expect("the title puts a picture on the screen");
    assert_eq!(
        lit.flip, FIRST_LIGHT,
        "first light moved: it is now flip {}",
        lit.flip
    );
    assert_eq!((lit.width, lit.height), (640, 480));
}

/// The same run twice over is the same run (#58).
#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else { return };
    let Some(second) = ({
        use std::sync::OnceLock;
        static AGAIN: OnceLock<Option<testkit::Ran>> = OnceLock::new();
        AGAIN
            .get_or_init(|| {
                if !scratch() {
                    return None;
                }
                Some(testkit::run_headless(
                    &ALPHA_TEAM,
                    &Cli::default(),
                    FIRST_LIGHT_BUDGET,
                ))
            })
            .as_ref()
    }) else {
        return;
    };
    assert_eq!(first.finish.cursor, second.finish.cursor);
    assert_eq!(first.finish.flips, second.finish.flips);
    assert_eq!(
        first.finish.declared_millis, second.finish.declared_millis,
        "the same declared time"
    );
    assert_eq!(
        first.digest, second.digest,
        "and the machine ends the same way"
    );
}
