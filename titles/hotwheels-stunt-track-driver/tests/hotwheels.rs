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
