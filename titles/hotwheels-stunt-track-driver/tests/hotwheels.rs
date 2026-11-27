//! Hot Wheels Stunt Track Driver, run headless from the player's rip: its
//! declared tree, its boot to first light replayed to the same machine every
//! time, its quit key, and the committed route driven through its first race
//! to the finish.
//!
//! Everything here goes through the run seam on the headless host: what the
//! title presented, what it logged, and how the run ended.

use hotwheels_stunt_track_driver::HOT_WHEELS_STUNT_TRACK_DRIVER as HOT_WHEELS;
use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::testkit;

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    testkit::rips(&HOT_WHEELS.image)
}

/// Enough instructions to reach the splash and watch it fade up.
///
/// This title paces itself by reading the tick count in a loop rather than by
/// sleeping, so it retires about fifteen million instructions between frames
/// waiting for its next one to be due: a declared Pentium 166's worth of
/// spinning, and most of this budget.
const FIRST_LIGHT_BUDGET: u64 = 2_200_000_000;

/// The flip the title's own picture first reaches the screen at: the
/// developer's splash, fading up through a palette that starts black.
const FIRST_LIGHT: u64 = 101;

#[test]
fn the_entry_image_loads_from_the_disc_and_starts_calling() {
    if !scratch() {
        return;
    }
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    let ran = testkit::try_run_headless(&HOT_WHEELS, &cli, 100_000).expect("the rip loads");

    ran.assert_logged("imports 73")
        .assert_logged("base 0x00400000")
        .assert_logged("call KERNEL32.DLL!GetVersion");
}

#[test]
fn the_declared_tree_holds_what_the_installer_would_have_copied() {
    if !scratch() {
        return;
    }
    let ran = testkit::try_run_headless(&HOT_WHEELS, &Cli::default(), 100_000).expect("it loads");

    // Every file under the disc's own HOT98 directory, and nothing from the
    // installer or the DirectX redistributable beside it.
    ran.assert_logged("installed 246 files");
}

/// The run every first light test looks at, replayed once.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    testkit::replay(
        &RAN,
        scratch,
        &HOT_WHEELS,
        &Cli::default(),
        &[FIRST_LIGHT - 1, FIRST_LIGHT],
        FIRST_LIGHT_BUDGET,
    )
}

#[test]
fn the_title_is_told_its_application_is_the_one_in_front() {
    let Some(ran) = boot() else { return };

    // WM_ACTIVATEAPP. This title turns its own drawing on when it hears it and
    // off when it hears the other one: never told, it renders a whole race
    // into its own frame buffer and puts none of it on the screen (#38).
    ran.assert_logged("window procedure given message 0x001c");
}

#[test]
fn the_title_reaches_first_light_and_puts_its_own_picture_on_the_screen() {
    let Some(ran) = boot() else { return };

    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way to the splash stops the run"
    );

    // The title has only the screen to draw on: no flipping chain, so every
    // frame is a lock and a letting go of the primary surface.
    ran.assert_logged("surface 640x480x8 at")
        .assert_logged("dinput mouse at");

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

    // Before it, the screen is one colour: the picture is already drawn and
    // the palette it is read through is still black.
    let before = ran
        .host
        .frame(FIRST_LIGHT - 1)
        .expect("the frame before it is kept");
    assert!(
        before.is_uniform(),
        "the frame before first light should be a screen of one colour"
    );
}

#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else { return };
    let Some(second) = ({
        static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
        testkit::replay(
            &AGAIN,
            scratch,
            &HOT_WHEELS,
            &Cli::default(),
            &[],
            FIRST_LIGHT_BUDGET,
        )
    }) else {
        return;
    };

    first.assert_same_machine(second);
}

/// Escape is this title's quit, not its skip (#68).
///
/// Measured while answering whether a click should get past the splashes:
/// Space skips them and Escape ends the game outright. Anyone tempted to
/// translate a click into Escape here — which is what the sibling title takes
/// (#66) — would be handing a player a quit for a click, so this holds the
/// measurement down.
#[test]
fn escape_is_this_titles_quit() {
    if !scratch() {
        return;
    }
    let route = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hot-wheels-escape.route");
    std::fs::write(
        &route,
        "#lego-legacy-route 1\nflip 150 key Escape down\nflip 155 key Escape up\n",
    )
    .expect("the route writes");
    let cli = Cli {
        route: Some(route),
        ..Cli::default()
    };
    let ran = testkit::run_headless(&HOT_WHEELS, &cli, FIRST_LIGHT_BUDGET);
    ran.assert_exited(1);
    assert!(
        ran.finish.flips < 200,
        "the title quits on Escape, at flip {} here",
        ran.finish.flips
    );
}

/// The flip the starting line is on the screen at, its lights still red.
const RACE_STARTS: u64 = 980;

/// Forty flips later, the lights green and the car away.
const RACE_RUNNING: u64 = 1020;

/// The flip the route presses Escape at, with the race's results up.
const ESCAPE: u64 = 3900;

/// Enough instructions for the committed route to race to the finish and
/// quit: 13.46 billion is spent by the time the title has gone. The boot and
/// the menus are under two billion of it; the race and its results cost
/// nearly four million instructions a flip.
const FIRST_RACE_BUDGET: u64 = 13_600_000_000;

/// The committed route through the first race, replayed once into its slot.
fn racing(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/first-race.route"),
        ),
        ..Cli::default()
    };
    testkit::replay(
        slot,
        scratch,
        &HOT_WHEELS,
        &cli,
        &[RACE_STARTS, RACE_RUNNING],
        FIRST_RACE_BUDGET,
    )
}

static RACE_FIRST: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
static RACE_SECOND: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

#[test]
fn the_route_reaches_a_race_and_the_race_is_a_moving_picture() {
    let Some(ran) = racing(&RACE_FIRST) else {
        return;
    };

    let line = ran
        .host
        .frame(RACE_STARTS)
        .expect("the run reaches the starting line inside its budget");
    let running = ran
        .host
        .frame(RACE_RUNNING)
        .expect("the run reaches forty flips past it");

    assert!(
        !line.is_uniform() && !running.is_uniform(),
        "a race that draws is not a screen of one colour"
    );
    assert_ne!(
        line.pixels, running.pixels,
        "a race that draws is a different picture forty flips later: the same one twice is \
         the title rendering into its own frame buffer and presenting none of it (#38)"
    );
}

/// The route drives the race to its finish and quits from its results, as
/// the person did (#89).
#[test]
fn the_route_finishes_the_race_and_quits_on_escape() {
    let Some(ran) = racing(&RACE_FIRST) else {
        return;
    };

    // The title writes the race's best time as it puts up the results, after
    // the finish.
    ran.assert_logged(r"file written C:\Program Files\Mattel\Stunt Track Driver\saves\times.dat");
    // And Escape is its quit (#68): the game is gone three flips after it.
    ran.assert_exited(1).assert_flips(ESCAPE + 3);
}

/// The same route twice over is the same run, from the boot through the
/// race to the quit (#89).
#[test]
fn the_first_race_route_replays_to_the_same_machine_every_time() {
    // The second replay runs beside the first rather than after it.
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| racing(&RACE_SECOND));
        (
            racing(&RACE_FIRST),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
}

/// The flip `secret-track.route` right-clicks the invisible Mattel "TT"
/// hotspot on Track Select, arming Secret Track 7 (`WIRE.JAM`).
const SECRET_RIGHT_CLICK: u64 = 17_889;

/// Track Select after right-clicking the Mattel "TT" hotspot.
const SECRET_TRACK_SELECT: u64 = 17_919;

/// `WIRE` starting grid: yellow starting lights and neon-green wireframe hills.
const WIRE_GRID: u64 = 18_125;

/// Mid-air upside-down barrel roll over `WIRE`'s wireframe valley.
const WIRE_STUNT: u64 = 18_460;

/// High-speed run through `WIRE`'s wireframe tunnel.
const WIRE_TUNNEL: u64 = 19_750;

/// `WIRE` results screen: 1:16 Bronze Medal, 04 stunts, 0 crashes, fireworks.
const WIRE_RESULTS: u64 = 20_976;

/// The flip `secret-track.route` presses Escape at, quitting from `WIRE`'s
/// results screen three flips later.
const SECRET_ESCAPE: u64 = 20_977;

const SECRET_TRACK_KEPT: [u64; 5] = [
    SECRET_TRACK_SELECT,
    WIRE_GRID,
    WIRE_STUNT,
    WIRE_TUNNEL,
    WIRE_RESULTS,
];

/// Enough instructions for `secret-track.route` to complete Championship
/// Tracks 1-6, unlock and finish `WIRE`, and quit on Escape: 77.38 billion
/// retired by exit at flip 20,980.
const SECRET_TRACK_BUDGET: u64 = 78_000_000_000;

struct SecretTrackRun {
    ran: testkit::Ran,
    onoff_path: std::path::PathBuf,
    onoff_dat: Vec<u8>,
    times_dat: Vec<u8>,
    wheel_dat: Vec<u8>,
}

/// The host path logged by the last `file written` line matching `suffix`.
fn last_written(ran: &testkit::Ran, suffix: &str) -> std::path::PathBuf {
    let path = ran
        .log
        .lines()
        .filter(|l| l.starts_with("file written ") && l.contains(suffix))
        .filter_map(|l| l.split_once(", to ").map(|(_, p)| p))
        .next_back()
        .unwrap_or_else(|| panic!("no `file written` line for {suffix}"));
    std::path::PathBuf::from(path)
}

/// Count bright neon-green pixels (`WIRE`'s wireframe colour) in a BGRA frame.
fn neon_green_pixels(frame: &lego_legacy_core::host::headless::Presented) -> usize {
    frame
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| px[1] > 180 && px[2] < 60 && px[0] < 60)
        .count()
}

/// Replay a Championship + `WIRE` route from a fresh `--data-root` once into
/// `slot`, reading back the persisted save files before removing the scratch root.
fn run_wire_route(
    slot: &'static std::sync::OnceLock<Option<SecretTrackRun>>,
    route_rel: &str,
    kept: &[u64],
    tag: &str,
) -> Option<&'static SecretTrackRun> {
    slot.get_or_init(|| {
        if !scratch() {
            return None;
        }
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("hotwheels-wire-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cli = Cli {
            route: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(route_rel)),
            data_root: Some(root.clone()),
            ..Cli::default()
        };
        let ran = testkit::run_on(
            lego_legacy_core::host::headless::Headless::new().keeping(kept),
            &HOT_WHEELS,
            &cli,
            SECRET_TRACK_BUDGET,
        );
        let onoff_path = last_written(&ran, r"saves\onoff.dat");
        let times_path = last_written(&ran, r"saves\times.dat");
        let wheel_path = last_written(&ran, r"saves\wheel.dat");
        let onoff_dat = std::fs::read(&onoff_path).expect("saves/onoff.dat reads");
        let times_dat = std::fs::read(&times_path).expect("saves/times.dat reads");
        let wheel_dat = std::fs::read(&wheel_path).expect("saves/wheel.dat reads");
        let _ = std::fs::remove_dir_all(&root);
        Some(SecretTrackRun {
            ran,
            onoff_path,
            onoff_dat,
            times_dat,
            wheel_dat,
        })
    })
    .as_ref()
}

fn assert_wire_run<'a>(
    run: &'a SecretTrackRun,
    kept: &[u64],
    min_stunt_green: usize,
    escape_flip: u64,
) -> Vec<&'a lego_legacy_core::host::headless::Presented> {
    let ran = &run.ran;
    const ONOFF_LINE: &str =
        r"file written C:\Program Files\Mattel\Stunt Track Driver\saves\onoff.dat, 28 bytes";
    const TIMES_LINE: &str =
        r"file written C:\Program Files\Mattel\Stunt Track Driver\saves\times.dat, 480 bytes";
    ran.assert_logged(ONOFF_LINE).assert_logged(TIMES_LINE);
    assert_eq!(
        ran.log.lines().filter(|l| l.contains(ONOFF_LINE)).count(),
        2,
        "saves\\onoff.dat is written after Track 3 and Track 6"
    );
    assert_eq!(
        run.onoff_dat.as_slice(),
        [1u32, 1, 1, 1, 1, 1, 0]
            .map(u32::to_le_bytes)
            .as_flattened(),
        "all 6 Championship tracks unlocked in saves\\onoff.dat"
    );
    assert_eq!(run.times_dat.len(), 480, "saves\\times.dat is 480 bytes");
    assert_eq!(run.wheel_dat.len(), 240, "saves\\wheel.dat is 240 bytes");
    assert_eq!(
        ran.log.lines().filter(|l| l.contains(TIMES_LINE)).count(),
        7,
        "saves\\times.dat is written once per finished race (6 Championship + WIRE)"
    );
    let (_, after_right_click) = ran
        .log
        .split_once(&format!("input flip {SECRET_RIGHT_CLICK} Down(Right)"))
        .expect("the route right-clicks the Mattel TT hotspot on Track Select");
    assert!(
        after_right_click.contains(TIMES_LINE),
        "WIRE finishes and writes saves\\times.dat after the right-click unlock"
    );

    let frames: Vec<_> = kept
        .iter()
        .map(|&flip| {
            let frame = ran
                .host
                .frame(flip)
                .unwrap_or_else(|| panic!("flip {flip} is kept"));
            assert_eq!((frame.width, frame.height), (640, 480));
            assert!(!frame.is_uniform(), "flip {flip} is not a uniform screen");
            frame
        })
        .collect();
    for (i, a) in frames.iter().enumerate() {
        for b in &frames[i + 1..] {
            assert_ne!(
                a.pixels, b.pixels,
                "flips {} and {} must show distinct pictures",
                a.flip, b.flip
            );
        }
    }

    assert_eq!(
        neon_green_pixels(frames[0]),
        0,
        "Track Select has no WIRE neon-green terrain"
    );
    assert!(
        neon_green_pixels(frames[1]) > 1_000,
        "WIRE starting grid shows neon-green wireframe hills: {} px",
        neon_green_pixels(frames[1])
    );
    assert!(
        neon_green_pixels(frames[2]) > min_stunt_green,
        "WIRE stunt frame shows neon-green wireframe terrain: {} px",
        neon_green_pixels(frames[2])
    );

    ran.assert_exited(1).assert_flips(escape_flip + 3);
    frames
}

fn assert_wire_replays_identical(
    route_rel: &str,
    kept: &[u64],
    first_slot: &'static std::sync::OnceLock<Option<SecretTrackRun>>,
    second_slot: &'static std::sync::OnceLock<Option<SecretTrackRun>>,
    first_tag: &str,
    second_tag: &str,
) {
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| run_wire_route(second_slot, route_rel, kept, second_tag));
        (
            run_wire_route(first_slot, route_rel, kept, first_tag),
            second.join().expect("the second WIRE replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.ran.assert_same_machine(&second.ran);
    for &flip in kept {
        assert_eq!(
            first.ran.host.frame(flip).map(|f| &f.pixels),
            second.ran.host.frame(flip).map(|f| &f.pixels),
            "flip {flip} presented identically on both replays"
        );
    }
    assert_ne!(
        first.onoff_path, second.onoff_path,
        "two replays, two data roots"
    );
    assert_eq!(
        first.onoff_dat, second.onoff_dat,
        "identical saves\\onoff.dat"
    );
    assert_eq!(
        first.times_dat, second.times_dat,
        "identical saves\\times.dat"
    );
    assert_eq!(
        first.wheel_dat, second.wheel_dat,
        "identical saves\\wheel.dat"
    );
}

static SECRET_FIRST: std::sync::OnceLock<Option<SecretTrackRun>> = std::sync::OnceLock::new();
static SECRET_SECOND: std::sync::OnceLock<Option<SecretTrackRun>> = std::sync::OnceLock::new();

#[test]
fn the_secret_track_route_unlocks_and_finishes_wire() {
    let Some(run) = run_wire_route(
        &SECRET_FIRST,
        "routes/secret-track.route",
        &SECRET_TRACK_KEPT,
        "first",
    ) else {
        return;
    };
    assert_wire_run(run, &SECRET_TRACK_KEPT, 20_000, SECRET_ESCAPE);
}

#[test]
fn the_secret_track_route_replays_to_the_same_machine_every_time() {
    assert_wire_replays_identical(
        "routes/secret-track.route",
        &SECRET_TRACK_KEPT,
        &SECRET_FIRST,
        &SECRET_SECOND,
        "first",
        "second",
    );
}

const WIRE_GOLD_STUNT: u64 = 18_960;
const WIRE_GOLD_RESULTS: u64 = 20_875;
const WIRE_GOLD_ESCAPE: u64 = 20_876;

const WIRE_GOLD_KEPT: [u64; 5] = [
    SECRET_TRACK_SELECT,
    WIRE_GRID,
    WIRE_GOLD_STUNT,
    WIRE_TUNNEL,
    WIRE_GOLD_RESULTS,
];

/// Byte offset of Track 7 (`WIRE`) records in `saves\times.dat` (12 tracks x 5
/// `(seconds: u32, car_index: u32)` entries = 480 bytes).
const WIRE_TIMES_OFFSET: usize = 280;

/// Stock Track 7 (`WIRE`) Gold Medal time (`1:11`, 71 s), displaced to Silver
/// (`entry[1]`) when beaten strictly (`<= 1:10`).
const WIRE_STOCK_GOLD_SECS: u32 = 71;

static WIRE_GOLD_FIRST: std::sync::OnceLock<Option<SecretTrackRun>> = std::sync::OnceLock::new();
static WIRE_GOLD_SECOND: std::sync::OnceLock<Option<SecretTrackRun>> = std::sync::OnceLock::new();

#[test]
fn the_wire_gold_medal_route_unlocks_and_earns_gold_on_wire() {
    let Some(run) = run_wire_route(
        &WIRE_GOLD_FIRST,
        "routes/wire-gold-medal.route",
        &WIRE_GOLD_KEPT,
        "gold-first",
    ) else {
        return;
    };
    let frames = assert_wire_run(run, &WIRE_GOLD_KEPT, 10_000, WIRE_GOLD_ESCAPE);

    // Track 7 entry[0] = (68 s / 1:08, car10 index 9), shifting stock 71 s Gold to entry[1].
    assert_eq!(
        &run.times_dat[WIRE_TIMES_OFFSET..WIRE_TIMES_OFFSET + 12],
        [68u32, 9, WIRE_STOCK_GOLD_SECS]
            .map(u32::to_le_bytes)
            .as_flattened(),
        "WIRE Gold Medal persists 1:08 (68 s, car10) at entry[0] and displaces stock 1:11 to entry[1]"
    );

    // BGRA red channel (`pixels[i + 2]`) across the Results screen crash-icon row
    // (`x = 195..540, y = 310..360`): empty orange-yellow sunburst holds <100 dark px (0 crashes).
    let results = frames[4];
    let w = results.width as usize;
    let mut crash_dark = 0usize;
    for y in 310..360 {
        for x in 195..540 {
            if results.pixels[(y * w + x) * 4 + 2] < 180 {
                crash_dark += 1;
            }
        }
    }
    assert!(
        crash_dark < 100,
        "WIRE Gold Medal results screen crash row must be empty (0 crashes): {crash_dark} dark px"
    );
}

#[test]
fn the_wire_gold_medal_route_replays_to_the_same_machine_every_time() {
    assert_wire_replays_identical(
        "routes/wire-gold-medal.route",
        &WIRE_GOLD_KEPT,
        &WIRE_GOLD_FIRST,
        &WIRE_GOLD_SECOND,
        "gold-first",
        "gold-second",
    );
}
