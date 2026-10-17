//! LEGO Alpha Team, from the rip to first light (#50).
//!
//! Everything here goes through the run seam or the declared tree: this title
//! ships nothing of itself loose on its disc, so what it is made of only
//! exists once the cabinet has been read.

use lego_alpha_team::LEGO_ALPHA_TEAM as ALPHA_TEAM;
use lego_legacy_core::cli::Cli;
use lego_legacy_core::host::headless::{Headless, Presented};
use lego_legacy_core::install::Tree;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::testkit;
use lego_legacy_core::testkit::differing;

const ROOT: &str = ALPHA_TEAM.install_dir;

/// The whole of a 640x480 screen.
const SCREEN: (usize, usize, usize, usize) = (0, 0, 640, 480);

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    testkit::rips(&ALPHA_TEAM.image)
}

/// The declared tree over this title's cabinet, as the machine builds it.
fn tree() -> Tree {
    testkit::tree(&ALPHA_TEAM)
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

/// The digest a determinism gate compares sees inside the binaries this title
/// ships, which is where most of it lives (#55, #94).
#[test]
fn the_digest_sees_inside_the_binaries_the_title_ships() {
    use lego_legacy_core::guest::GuestMem;
    if !scratch() {
        return;
    }
    let mut host = Headless::new();
    let mut machine = Machine::new(&ALPHA_TEAM, &mut host).expect("a machine");
    machine.load_title(&ALPHA_TEAM).expect("the title loads");
    let before = machine.state_digest();
    let module = machine.modules.modules().last().expect("a module");
    let at = module.base + module.size - 1;
    let byte = machine.arena.slice(at, 1).expect("it is mapped")[0];
    machine.arena.write(at, &[!byte]).expect("it is mapped");
    assert_ne!(machine.state_digest(), before, "{} is hashed", module.name);
}

/// The debugger's `until wild` stops where the guest first runs outside its
/// own code. Every run starts in the loader's own program and goes from there
/// into each binary's `DllMain`, and both are the title's own (#55, #94).
#[test]
fn the_loader_and_the_binaries_it_attaches_are_not_wild() {
    if !scratch() {
        return;
    }
    let mut host = Headless::new();
    let mut machine = Machine::new(&ALPHA_TEAM, &mut host).expect("a machine");
    machine.load_title(&ALPHA_TEAM).expect("the title loads");
    machine
        .tree
        .set_profile(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("wild"));
    let mut log =
        lego_legacy_core::runlog::RunLog::create(ALPHA_TEAM.key, lego_legacy_core::cli::Trace::Off)
            .expect("a run directory");
    while machine.clock.cursor() < 200_000 {
        assert!(
            !machine.is_wild(),
            "wild at {:#010x} cursor {}",
            machine.cpu.eip,
            machine.clock.cursor()
        );
        assert_eq!(machine.tick(&mut host, &mut log), None, "the run goes on");
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

/// The flip the title's own picture is first on the screen at: the fifth
/// frame of its intro movie, which is the first one that is not black (#63).
const FIRST_LIGHT: u64 = 5;

/// The flip its own Loading panel is on the screen at, drawn through
/// Direct3D 7 — what #58 called first light, before the intro movie in front
/// of it was being presented at all (#63).
const LOADING_PANEL: u64 = 1633;

/// What it costs to get there. Most of it is the title reading four hundred
/// megabytes of archives and playing its intro movie.
const FIRST_LIGHT_BUDGET: u64 = 600_000_000;

/// The run that reaches first light, done once for every test that looks at
/// it.
fn boot() -> Option<&'static testkit::Ran> {
    static BOOT: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    testkit::replay(
        &BOOT,
        scratch,
        &ALPHA_TEAM,
        &Cli::default(),
        &[FIRST_LIGHT, LOADING_PANEL],
        FIRST_LIGHT_BUDGET,
    )
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
    // And its own Loading panel, the frame #58 was written about, is still
    // there behind the movie's sixteen hundred (#63).
    let panel = ran
        .host
        .frame(LOADING_PANEL)
        .expect("the Loading panel is kept");
    assert!(!panel.is_uniform(), "the Loading panel drew something");
    assert_eq!((panel.width, panel.height), (640, 480));
}

/// A click gets past the intro, because the machine hands the title the key
/// the title listens for there (#66).
#[test]
fn a_click_during_the_intro_is_the_key_the_title_takes() {
    if !scratch() {
        return;
    }
    // Measured before this landed: a click at flip 600 was ignored outright
    // and the movie ran to its own end at flip 1631. Escape is the one key
    // this intro takes, and only from about flip 440 on.
    let clicked = |at: u64| {
        let route = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("click-the-intro-{at}.route"));
        std::fs::write(
            &route,
            format!("#lego-legacy-route 1\nflip {at} down\nflip {} up\n", at + 2),
        )
        .expect("the route writes");
        let cli = Cli {
            route: Some(route),
            ..Cli::default()
        };
        let ran = testkit::run_on(Headless::new(), &ALPHA_TEAM, &cli, FIRST_LIGHT_BUDGET);
        let before = ran
            .log
            .split_once("lmiintro.avi let go of")
            .expect("the movie is let go of, on the click or on its own end")
            .0;
        before
            .lines()
            .filter_map(|l| l.strip_prefix("flip "))
            .filter_map(|l| l.split_whitespace().next())
            .filter_map(|n| n.parse::<u64>().ok())
            .next_back()
            .unwrap_or(0)
    };
    assert_eq!(clicked(600), 600, "a click past the logo ends the movie");
    // And the title's own rule is untouched: it refuses every key while its
    // developer's logo is up, so a click there is refused too. The machine
    // translates what a person did; it does not end the movie (ADR-0002).
    assert!(
        clicked(300) > 1600,
        "a click under the logo is still the title's to refuse"
    );
}

/// The same run twice over is the same run (#58).
#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else { return };
    let Some(second) = ({
        static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
        testkit::replay(
            &AGAIN,
            scratch,
            &ALPHA_TEAM,
            &Cli::default(),
            &[],
            FIRST_LIGHT_BUDGET,
        )
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

/// The flip the main menu is up and waiting at, before the route touches the
/// mouse.
const MENU: u64 = 2112;

/// A few flips after the route has clicked Single Missions, where the mission
/// list is up.
const MISSION_LIST: u64 = 2332;

/// Enough instructions for the committed route to reach the mission list and
/// wait in it. The front end costs about two and a half million instructions a
/// flip, all of it the title's own engine running as guest code.
const MENU_BUDGET: u64 = 5_400_000_000;

/// The committed route through the front end, replayed once into its slot.
fn through_the_menu(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    testkit::replay(
        slot,
        scratch,
        &ALPHA_TEAM,
        &route("menu.route"),
        &[MENU, MISSION_LIST],
        MENU_BUDGET,
    )
}

/// A committed route of this title's, to replay.
fn route(name: &str) -> Cli {
    Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("routes")
                .join(name),
        ),
        ..Cli::default()
    }
}

static MENU_FIRST: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
static MENU_SECOND: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// The route leaves the attract screen with a keystroke and picks Single
/// Missions with the pointer, and both reach the title's own window procedure
/// (#59).
#[test]
fn the_route_drives_the_front_end_from_the_keyboard_and_the_mouse() {
    let Some(ran) = through_the_menu(&MENU_FIRST) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing in the front end stops the run"
    );
    // This title reads its keyboard from its window and its pointer from
    // DirectInput's, and both arrive: `WM_KEYDOWN` and `WM_LBUTTONDOWN`, at
    // the procedure the title registered.
    ran.assert_logged("window procedure given message 0x0100")
        .assert_logged("window procedure given message 0x0201");

    let menu = ran.host.frame(MENU).expect("the menu is kept");
    let list = ran
        .host
        .frame(MISSION_LIST)
        .expect("the mission list is kept");
    assert_eq!((menu.width, menu.height), (640, 480));
    // Choosing an item put a different screen up: the mission list covers most
    // of the window the menu was in.
    let pixels = 640 * 480;
    assert!(
        differing(menu, list, SCREEN) > pixels / 4,
        "flip {MISSION_LIST} should be the mission list, not the menu"
    );
}

/// The same route twice over is the same run (#59).
#[test]
fn the_menu_route_replays_to_the_same_machine_every_time() {
    let Some(first) = through_the_menu(&MENU_FIRST) else {
        return;
    };
    let Some(second) = through_the_menu(&MENU_SECOND) else {
        return;
    };
    assert_eq!(first.finish.cursor, second.finish.cursor);
    assert_eq!(first.finish.flips, second.finish.flips);
    assert_eq!(
        first.finish.declared_millis, second.finish.declared_millis,
        "the same declared time"
    );
    assert_eq!(first.digest, second.digest);
}

/// The flip the mission is up and playing at, a hundred flips after the route
/// has begun the training exercise.
const MISSION: u64 = 2882;

/// And three hundred and fifty flips of it later, which is where the mission
/// is asked to have gone on drawing.
const MISSION_LATER: u64 = 3232;

/// The flip the route clicks the movable Direction Tile at, which is when the
/// training's step 230a asks for it (#85).
const TILE_CLICKED: u64 = 7011;

/// Just before that click, and once the Unit Menu it opens is up.
const BEFORE_UNIT_MENU: u64 = 7004;
const UNIT_MENU: u64 = 7034;

/// The flip the route presses GO at, which is the training's last step: Dash
/// runs for the console (#61).
const GO: u64 = 7607;

/// Dash running across the blue floor for the tile, and the same floor once
/// the tile has turned him and he has gone up the steps to the console. The
/// camera does not move between the two.
const DASH_RUNNING: u64 = 7644;
const DASH_GONE: u64 = 7834;

/// The flip the title is back at its main menu, the training over (#61).
const MENU_AGAIN: u64 = 7990;

/// Enough instructions for the committed route to load the first mission,
/// play its training to its end and be back at the main menu: 23.22 billion
/// reaches flip 7990. The mission costs about three and a half million
/// instructions a flip: the title's own director, its actors and the
/// rasteriser under them.
const MISSION_BUDGET: u64 = 23_400_000_000;

/// The committed route into the first mission, replayed once into its slot.
fn into_the_mission(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    testkit::replay(
        slot,
        scratch,
        &ALPHA_TEAM,
        &route("first-mission.route"),
        &[
            MENU,
            MISSION,
            MISSION_LATER,
            BEFORE_UNIT_MENU,
            UNIT_MENU,
            DASH_RUNNING,
            DASH_GONE,
            MENU_AGAIN,
        ],
        MISSION_BUDGET,
    )
}

static MISSION_FIRST: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
static MISSION_SECOND: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// The route chooses the Alpha Team Training Area out of Single Missions,
/// loads it and plays it: the mission's own space, drawn by its own director,
/// and still drawing three hundred and fifty flips later (#61).
#[test]
fn the_route_loads_the_first_mission_and_plays_it() {
    let Some(ran) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing in the mission stops the run"
    );
    let mission = ran.host.frame(MISSION).expect("the mission is kept");
    let later = ran
        .host
        .frame(MISSION_LATER)
        .expect("the mission, later, is kept");
    assert_eq!((mission.width, mission.height), (640, 480));
    assert!(!mission.is_uniform(), "the mission drew something");
    // It is a moving picture: the director turns the camera around the space
    // and the actors in it are animated.
    let pixels = 640 * 480;
    let moved = differing(mission, later, SCREEN);
    assert!(
        moved > pixels / 200,
        "the mission stopped moving between flip {MISSION} and flip {MISSION_LATER}: \
         {moved} pixels of {pixels} differ"
    );
    // And it has been going for well over a declared minute by then.
    assert!(
        ran.finish.declared_millis > 60_000,
        "only {} declared milliseconds",
        ran.finish.declared_millis
    );
}

/// The same route twice over is the same run, from the boot through the
/// whole training and back to the main menu (#61).
#[test]
fn the_first_mission_route_replays_to_the_same_machine_every_time() {
    let Some(first) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    let Some(second) = into_the_mission(&MISSION_SECOND) else {
        return;
    };
    assert_eq!(first.finish.cursor, second.finish.cursor);
    assert_eq!(first.finish.flips, second.finish.flips);
    assert_eq!(first.finish.declared_millis, second.finish.declared_millis);
    assert_eq!(first.digest, second.digest);
}

/// Every line the training's coach says, in order: a line ending in `a` asks
/// for something, and the `b` after it is said once it has been done.
const COACH: &[&str] = &[
    "010", "020a", "020b", "030a", "030b", "040a", "040b", "050a", "050b", "060a", "060b", "070a",
    "070b", "080a", "080b", "090a", "090b", "100a", "100b", "110a", "110b", "120a", "120b", "130a",
    "130b", "140a", "150", "160", "170a", "170b", "180a", "180b", "190a", "190b", "195a", "195b",
    "200a", "200b", "210", "220a", "220b", "230a", "230b", "240a", "240b", "250a", "250b", "260a",
    "260b", "270a", "275", "280a", "280b",
];

/// The first mission is played to its end (#61).
///
/// The title shows each of the coach's lines as a subtitle, which it looks
/// for as a loose file before reading it out of `Locale_en.pac`, and the
/// machine logs each look. So the log carries the training, line by line, as
/// the title itself steps through it. The last, 280b, is said when Dash
/// reaches the console; the title then asks for Dash's victory line
/// (`T0011_VICTORY`), puts up "Destination: Alpha Team Headquarters", and
/// loads the main menu's own space (`MM.PUZ`).
#[test]
fn the_route_plays_the_training_to_its_end_and_the_title_goes_back_to_its_menu() {
    let Some(ran) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing in the training stops the run"
    );
    let said: Vec<&str> = ran
        .log
        .lines()
        .filter_map(|l| l.split("\\m0010_").nth(1))
        .filter_map(|l| l.strip_suffix("_EN.dds, asked from 0x400c8646"))
        .collect();
    assert_eq!(said, COACH, "the coach says every line, in order");

    // The last step is GO, and what comes of it is the title's own ending.
    let after_go = ran
        .log
        .split_once(&format!("input flip {GO} Down(Left)"))
        .expect("the route presses GO")
        .1;
    let at = |what: &str| {
        after_go
            .find(what)
            .unwrap_or_else(|| panic!("after GO, the title never asks for {what}"))
    };
    assert!(
        at("m0010_280b") < at("T0011_VICTORY") && at("T0011_VICTORY") < at("MM.PUZ"),
        "the training ends, Dash says his victory line, and then the main menu loads"
    );

    // And the screen is the main menu the route left: the same picture, but
    // for the characters' idle animation.
    let pixels = 640 * 480;
    let menu = ran.host.frame(MENU).expect("the menu is kept");
    let again = ran
        .host
        .frame(MENU_AGAIN)
        .expect("the menu, again, is kept");
    let mission = ran.host.frame(MISSION).expect("the mission is kept");
    // Measured: 23,571 pixels apart, where the mission is most of the screen
    // away from either.
    assert!(
        differing(menu, again, SCREEN) < pixels / 10,
        "flip {MENU_AGAIN} should be the main menu: {} pixels differ from flip {MENU}",
        differing(menu, again, SCREEN)
    );
    assert!(differing(mission, again, SCREEN) > pixels / 2);
}

/// The flip at which the title asked for one of the coach's lines.
fn asked_at(ran: &testkit::Ran, line: &str) -> u64 {
    let wanted = format!("\\m0010_{line}_EN.dds");
    let mut flip = 0;
    for l in ran.log.lines() {
        if let Some(n) = l
            .strip_prefix("flip ")
            .and_then(|l| l.split(' ').next())
            .and_then(|n| n.parse().ok())
        {
            flip = n;
        } else if l.contains(&wanted) {
            return flip;
        }
    }
    panic!("the title never asks for m0010_{line}");
}

/// The coach is heard out (#92).
///
/// The training waits for each of his lines to finish before it asks for
/// the next, so a line cut short in the mix is a training that goes on
/// early. His first, `m0010_010`, is 10.31 seconds long, which is 309 flips
/// at the title's thirty. While `Lock` handed out only the part of a lock
/// before a buffer's end, the voice's one-second stream buffer lost a second
/// of every lap and the next line was asked 173 flips after it. Measured
/// since: 313.
#[test]
fn the_training_waits_for_the_coach_to_finish_his_first_line() {
    let Some(ran) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    let waited = asked_at(ran, "020a") - asked_at(ran, "010");
    assert!(
        waited >= 310,
        "m0010_020a was asked {waited} flips after m0010_010"
    );
}

/// Dash runs, and his shadow runs with him (#61).
///
/// `DDSpace::FormShadow` lays a shadow under each actor that makes one, on
/// whatever is beneath it, textured with `ShadowMap.tga` and blended
/// `D3DBLEND_ZERO`, `D3DBLEND_INVSRCCOLOR`: it darkens what it covers. Under
/// a throwaway probe 37 flips after GO, which was flip 6010 before #92 and is
/// 7644 now, Dash's shadow darkened 570 pixels of the blue floor, the
/// rectangle here among them and clear of his legs. By flip 7834 he is at
/// the console, and with the camera exactly where it was, that floor is
/// lighter again.
#[test]
fn dash_runs_for_the_console_and_his_shadow_goes_with_him() {
    let Some(ran) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    let running = ran.host.frame(DASH_RUNNING).expect("Dash running is kept");
    let gone = ran.host.frame(DASH_GONE).expect("Dash gone is kept");
    let at = |x: usize, y: usize| (y * 640 + x) * 4;
    let light = |f: &Presented, i: usize| {
        f.pixels[i..i + 3]
            .iter()
            .map(|&c| u32::from(c))
            .sum::<u32>()
    };

    // The camera has not moved: the floor at the bottom left is the same to
    // the pixel.
    let still = differing(running, gone, (0, 200, 300, 388));
    assert_eq!(still, 0, "the camera moved");

    // Dash moved, and where he was the picture is not what it is now.
    let moved = differing(running, gone, (380, 60, 460, 150));
    assert!(moved > 80 * 90 / 10, "Dash is not where he was: {moved}");

    // And the floor under him is in his shadow. Measured: 318 of these 320
    // pixels darker, and the other two, in a corner past its edge, the same.
    let shadowed = (130..140)
        .flat_map(|y| (400..432).map(move |x| at(x, y)))
        .filter(|&i| light(running, i) < light(gone, i))
        .count();
    assert!(
        shadowed > 320 * 9 / 10,
        "the floor under Dash is not in shadow: {shadowed} of 320 pixels darker"
    );
}

/// A click on the movable Direction Tile opens its Unit Menu, once the
/// training has let the tile be picked (#85).
///
/// The training is a script in `Pe0300.dll`, the title's behaviour library.
/// Its coach, a figure in the teleporter of actor type `T0001`, says each step
/// and flashes what to use, and until step 220a it is the only thing in the
/// space that can be picked: the click handler at `0x10057c60` looks the
/// picked actor's type up in the training's set at `0x1009f418`, which holds
/// `T0001` alone, and drops anything else at `0x10057d7f`. Step 220a puts
/// `U0179`, the movable tile, into that set and asks for it to be carried
/// onto the blue pad; 230a asks for it to be clicked. The click selects it
/// (`0x100674f0`), and letting go builds the Unit Menu (`0x10031cd0`, called
/// from `0x1005823e`). The route takes every step the way the coach asks.
#[test]
fn a_click_on_the_movable_tile_opens_its_unit_menu() {
    let Some(ran) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    // The menu's buttons are made the first time it opens: the title asks for
    // their textures then, and at no point before.
    let (before, after) = ran
        .log
        .split_once(&format!("input flip {TILE_CLICKED} Down(Left)"))
        .expect("the route clicks the tile");
    assert!(
        !before.contains("RotateCW.tga"),
        "nothing builds the Unit Menu before the tile is clicked"
    );
    assert!(
        after.contains("RotateCW.tga") && after.contains("Hint.tga"),
        "the click on the tile builds its Unit Menu"
    );
    // And it is drawn, above the tile: its panel covers 329..489 across and
    // 118..179 down, where from one frame to the next about one pixel in forty
    // changes by itself.
    let shut = ran
        .host
        .frame(BEFORE_UNIT_MENU)
        .expect("the frame before is kept");
    let open = ran.host.frame(UNIT_MENU).expect("the frame after is kept");
    let changed = differing(shut, open, (329, 118, 489, 179));
    let panel = (179 - 118) * (489 - 329);
    assert!(
        changed > panel * 3 / 4,
        "the Unit Menu is not on the screen: {changed} of its {panel} pixels changed"
    );
}

/// Alpha Team is heard (#60).
///
/// Its sound is Aureal A3D, which it reaches through `CoCreateInstance` and
/// nothing else: the class is served by `a3dapi.dll` out of its own cabinet,
/// run as guest code, and what that mixes in software reaches this machine as
/// ordinary DirectSound buffers.
#[test]
fn the_title_is_heard_through_the_a3d_its_own_disc_ships() {
    let Some(ran) = into_the_mission(&MISSION_FIRST) else {
        return;
    };
    // How loud each mixed block is, for the blocks mixed between two flips.
    let blocks = |from: u64, to: u64| -> Vec<f64> {
        ran.host
            .audio_blocks
            .iter()
            .zip(&ran.host.audio_flips)
            .filter(|(_, flip)| (from..to).contains(*flip))
            .map(|(block, _)| {
                let power: f64 = block.iter().map(|s| f64::from(*s).powi(2)).sum();
                (power / block.len().max(1) as f64).sqrt()
            })
            .collect()
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    // Silence is zero; the menu and the mission measured in the thousands.
    const HEARD: f64 = 300.0;

    // The menu has its own music under it, long after the intro movie is over.
    let menu = blocks(MENU - 80, MENU + 20);
    assert!(
        mean(&menu) > HEARD,
        "the menu is heard: {:.0} over {} blocks",
        mean(&menu),
        menu.len()
    );
    // And so does the mission, which is a different piece.
    let mission = blocks(MISSION, MISSION_LATER);
    assert!(
        mean(&mission) > HEARD,
        "the mission is heard: {:.0} over {} blocks",
        mean(&mission),
        mission.len()
    );

    // `DDStreamingAudioManager` streams rather than plays: it writes on into a
    // buffer that is already playing, and a stream that starves leaves a gap.
    // Every fifty flips from the menu to the end of the run carries blocks.
    for at in (MENU - 80..MISSION_LATER).step_by(50) {
        let window = blocks(at, at + 50);
        assert!(
            !window.is_empty() && mean(&window) > HEARD,
            "the sound stops between flip {at} and flip {}: {window:?}",
            at + 50
        );
    }
}

/// The Select Adventure screen as the route first opens it, every slot New.
const SLOTS_NEW: u64 = 2340;

/// The same screen opened again once an adventure is saved, with the first
/// slot labelled with its date.
const SLOT_SAVED: u64 = 4140;

/// Enough instructions for the committed route to start an adventure, skip
/// its training mission, go back to HQ and open Select Adventure again: at the
/// title's thirty, 14 billion reaches flip 4454. Since its loading voice was
/// heard whole (#92), Mission One starts 176 flips later than it did, still
/// well before the route's click at 3562, and every flip the route and this
/// test name is where it was.
const SELECT_ADVENTURE_BUDGET: u64 = 14_000_000_000;

/// A saved adventure is labelled on Select Adventure with the date it was
/// saved on, and the machine writes that date as Windows 98 writes one for
/// US English (#87).
///
/// This was a wall a person could reach: `KERNEL32!GetDateFormatA` was not
/// written, and the first time the screen had a saved adventure to show, the
/// run stopped there.
#[test]
fn a_saved_adventure_is_labelled_with_the_date_it_was_saved_on() {
    if !scratch() {
        return;
    }
    let ran = testkit::run_on(
        Headless::new().keeping(&[SLOTS_NEW, SLOT_SAVED]),
        &ALPHA_TEAM,
        &route("select-adventure.route"),
        SELECT_ADVENTURE_BUDGET,
    );
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way stops the run"
    );
    // The title asks for the short date and the time without its seconds, in
    // the locale the machine declares, for the moment it saved at: the
    // declared clock's 1999-10-29, three minutes in.
    ran.assert_logged(r#"date written "10/29/99" for locale 0x409 flags 0x1"#)
        .assert_logged(r#"time written "12:03 AM" for locale 0x409 flags 0x2"#);

    // And the screen shows it. The first slot was New and is now the saved
    // adventure; the third was New and still is.
    let before = ran.host.frame(SLOTS_NEW).expect("the empty slots are kept");
    let after = ran.host.frame(SLOT_SAVED).expect("the saved slot is kept");
    let differing_in =
        |left: usize, top: usize| differing(before, after, (left, top, left + 120, top + 90));
    // Measured: 10,694 of the first slot's 10,800 pixels, and none of the
    // third's.
    let (first, third) = (differing_in(122, 84), differing_in(399, 84));
    assert!(first > 10_800 / 2, "the first slot is still New: {first}");
    assert!(third < 10_800 / 50, "the third slot changed: {third}");
}
