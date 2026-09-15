//! Hot Wheels Stunt Track Driver, from the rip to first light (#33).
//!
//! Everything here goes through the run seam on the headless host: what the
//! title presented, what it logged, and how the run ended.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::disc;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::roster::HOT_WHEELS as ENTRY;
use lego_legacy_core::testkit;
use lego_legacy_core::title::{Declared, Installed, Title, WINDOWS_98_SE};

/// The title as its own crate declares it.
const HOT_WHEELS: Title = Title {
    key: ENTRY.key,
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        hz: 166_000_000,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
        user: "LEGO-LEGACY",
        why: "the title crate's own, repeated here because a test crate cannot depend on a \
              binary",
    },
    installed: Installed::Copied("SETUP/HOT98"),
    registry: &[
        (
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "InstallPath",
            "C:\\Program Files\\Mattel\\Stunt Track Driver",
        ),
        (
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "CDDATADIR",
            "D:\\SETUP\\HOT98\\DATA",
        ),
    ],
    install_dir: "C:\\Program Files\\Mattel\\Stunt Track Driver",
    click_in_a_movie: None,
    pointer_speed: (1.0, 1.0),
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
        eprintln!(
            "skipping: no rips under {} (rip your own discs there)",
            root.display()
        );
    }
    present
}

/// Enough instructions to reach the splash and watch it fade up.
///
/// This title paces itself by reading the tick count in a loop rather than by
/// sleeping, so it retires about fifteen million instructions between frames
/// waiting for its next one to be due: a declared Pentium II's worth of
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
    use std::sync::OnceLock;
    static RAN: OnceLock<Option<testkit::Ran>> = OnceLock::new();
    RAN.get_or_init(|| {
        if !scratch() {
            return None;
        }
        let host = Headless::new().keeping(&[FIRST_LIGHT - 1, FIRST_LIGHT]);
        Some(testkit::run_on(
            host,
            &HOT_WHEELS,
            &Cli::default(),
            FIRST_LIGHT_BUDGET,
        ))
    })
    .as_ref()
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
        use std::sync::OnceLock;
        static AGAIN: OnceLock<Option<testkit::Ran>> = OnceLock::new();
        AGAIN
            .get_or_init(|| {
                if !scratch() {
                    return None;
                }
                Some(testkit::run_headless(
                    &HOT_WHEELS,
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
const RACE_STARTS: u64 = 7200;

/// Forty flips later, with the car away down the track.
const RACE_RUNNING: u64 = 7240;

/// Enough to replay the committed route through the race's first frames.
///
/// Most of it is the boot and the menus; the race itself is the last half
/// billion. It is the price of a gate on a title that only reaches its own
/// play through seven thousand flips of a person's own clicking.
const FIRST_RACE_BUDGET: u64 = 11_000_000_000;

/// The committed route to a race, replayed once.
fn racing() -> Option<&'static testkit::Ran> {
    use std::sync::OnceLock;
    static RAN: OnceLock<Option<testkit::Ran>> = OnceLock::new();
    RAN.get_or_init(|| {
        if !scratch() {
            return None;
        }
        let cli = Cli {
            route: Some(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../titles/hotwheels-stunt-track-driver/routes/first-race.route"),
            ),
            ..Cli::default()
        };
        let host = Headless::new().keeping(&[RACE_STARTS, RACE_RUNNING]);
        Some(testkit::run_on(host, &HOT_WHEELS, &cli, FIRST_RACE_BUDGET))
    })
    .as_ref()
}

#[test]
fn the_route_reaches_a_race_and_the_race_is_a_moving_picture() {
    let Some(ran) = racing() else { return };

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
