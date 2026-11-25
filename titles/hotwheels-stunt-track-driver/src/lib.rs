//! Hot Wheels Stunt Track Driver: the roster's second title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::HOT_WHEELS as ENTRY;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, Goal, Installed, Milestone, MouseMode, TitleGuide,
    WINDOWS_98_SE,
};
use lego_legacy_core::Title;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const HOT_WHEELS_STUNT_TRACK_DRIVER: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. Sixty a second is the roster's
    // baseline; this title has not yet been seen to ask for anything else.
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        windows: WINDOWS_98_SE,
        // A Pentium 166: what this title's own box asks for, and not the
        // roster's baseline. It matters more here than it does for a title
        // that sleeps: this one waits for its next frame by reading the tick
        // count in a loop, so the declared processor decides how much work a
        // frame of waiting costs, and a machine far faster than the one it
        // was written for spends its whole frame spinning.
        hz: 166_000_000,
        // And that Pentium is told about as itself: family 5, which has no
        // conditional moves. Measured over the race route: this title never
        // asks, by `cpuid` or by the flags (#104).
        processor: Processor::Pentium,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        // The primary tree's names alone, by #142's rule: Joliet is LEGO
        // Chess's alone, because Alpha Team's runs move with it, until the
        // maintainer decides (#144). Nothing in this title asks for it: read
        // through its Joliet tree, as Windows 98 SE's CDFS reads it, this
        // disc answers the same names but for case, and none of its runs
        // moved with it on (measured at 7509b49).
        joliet: false,
        computer: "LEGO-LEGACY",
        // None: no gate run asks for a library, and the one place it can,
        // its C runtime's message box, asks for USER32, which core writes (#146).
        absent_libraries: &[],
        // None, as every released title's machine has had since its routes were
        // recorded: C: and D: only.
        floppy: false,
        why: "Windows 98 SE is what this pressing shipped against and a Pentium 166 is what \
              its box asks for; the rest is the roster's baseline, unmeasured against this \
              title because it has not asked for any of it yet",
    },
    // No cabinet on this disc: the installer copies SETUP/HOT98 whole, and the
    // declared tree says so rather than unpacking anything.
    installed: Installed::Copied("SETUP/HOT98"),
    // What its installer leaves behind for it to find itself by: this title
    // reads both of these back on the way up, and builds every path to its own
    // data out of them.
    registry: &[
        (
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "InstallPath",
            "C:\\Program Files\\Mattel\\Stunt Track Driver",
        ),
        (
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "CDDATADIR",
            // The title puts a file's own name straight onto this one, so it
            // names the directory the data is in, not the one above it.
            "D:\\SETUP\\HOT98\\DATA",
        ),
    ],
    // Where this title's installer puts it. The title reads this path back out
    // of the registry as `InstallPath` under
    // `SOFTWARE\MATTEL\STUNT TRACK DRIVER\1.0`, so it has to be the same on
    // every run and on every host.
    install_dir: "C:\\Program Files\\Mattel\\Stunt Track Driver",
    // This title plays no movie at all, so there is nothing here for a click
    // to be translated inside of (#68). What gets past its splashes is Space,
    // measured — Escape quits the game outright and a click does nothing —
    // but its splashes and its menu are the same thing at the boundary, so
    // there is no window to scope a translation to and it is not made.
    click_in_a_movie: None,
    // One for one: this title steers its own pointer by what it is handed and
    // keeps none of it back, and a person played it at a window and said the
    // pointer and the mouse move as one (#37).
    pointer_speed: (1.0, 1.0),
};

/// Static playability metadata served via `/api/meta`, `/api/controls`, and
/// `/api/goals` (#311).
pub const GUIDE: TitleGuide = TitleGuide {
    display_name: "Hot Wheels: Stunt Track Driver",
    summary: "Arcade 3D stunt racing across household tabletop tracks with aerial tricks and a secret wireframe track.",
    resolution: (640, 480),
    bpp: 8,
    refresh_hz: 60,
    mouse_mode: MouseMode::AbsoluteKept,
    escape_quits: true,
    skip_keys: &["Space"],
    cadence: AgentCadence::MicroStep { flips_per_step: 8 },
    controls: &[
        ControlHint {
            input: "Up",
            action: "Throttle / Rocket Start",
            notes: "Accelerate (also `Keypad8`/`7`/`9`); hold only when the start light turns green for a Rocket Start.",
        },
        ControlHint {
            input: "Down",
            action: "Brake / reverse",
            notes: "Brake before tight turns or reverse to recover (also `Keypad2`/`1`/`3`).",
        },
        ControlHint {
            input: "Left / Right",
            action: "Steer",
            notes: "Steer left or right along the track (also `Keypad4`/`6`); pulse in short bursts to avoid oversteer.",
        },
        ControlHint {
            input: "LeftShift",
            action: "Barrel roll",
            notes: "Hold mid-air off ramps for a barrel roll (also `LeftAlt`/`PageDown`); release in time to land upright.",
        },
        ControlHint {
            input: "LeftControl",
            action: "Flip",
            notes: "Hold mid-air off ramps for a flip (also `Delete`); release in time to land upright.",
        },
        ControlHint {
            input: "Space",
            action: "Skip / confirm",
            notes: "Skip splashes and intro (~flip 285, hold ~5-9 flips); also acts as left-click confirm in menus.",
        },
        ControlHint {
            input: "Left click (--hold-flips 5..6)",
            action: "Select menu / wheel / workbench / Green GO",
            notes: "Move cursor and settle first, then hold 5-6 flips: Main Menu Single Race `~(546,413)`, Championship `~(552,102)`, Track/Car Green GO `~(583,430)`, Car Select wheel, and Custom Garage workbench pieces.",
        },
        ControlHint {
            input: "Right click (270, 100)",
            action: "Unlock Secret Track WIRE",
            notes: "Right-click (`--hold-flips 5`) the invisible Mattel \"TT\" hotspot at `(270, 100)` on Track Select after clearing Championship Tracks 1-6, then click Green GO. Visually verify cursor alignment if returning from a race (#329).",
        },
        ControlHint {
            input: "P / Pause",
            action: "Pause race",
            notes: "Toggle in-race pause safely without quitting.",
        },
        ControlHint {
            input: "C",
            action: "Reset / crash car",
            notes: "Forces an in-race crash/respawn and increments the crash counter; use only if stuck.",
        },
        ControlHint {
            input: "F1",
            action: "Cycle race resolution",
            notes: "In menus, cycles `640x480` / `320x240` / `320x200`; keep at `640x480` during normal runs.",
        },
        ControlHint {
            input: "Escape",
            action: "Immediate exit (HAZARD)",
            notes: "NEVER press `Escape` unless intending immediate process exit (`exits 1`); blocked unless `force: true`.",
        },
    ],
    hazards: &[
        "NEVER press `Escape` unless intending immediate exit: this title quits unconditionally on `Escape` (guarded by `escape_quits: true`; requires `force: true`). Use `P` or `Pause` to pause a race.",
        "Menu clicks need `move` + settle and `--hold-flips 5` (or `5..6`); 1-flip clicks are frequently missed by DirectInput menu polling.",
        "Post-race `SetCursorPos(320, 240)` cursor offset (#329): on race finish `STUNT.EXE` re-centers the Windows cursor while its internal DirectInput hand cursor integrates relative deltas; screenshot and steer with `delta` or offset-adjusted moves after any race.",
        "Hold `Up` only on the green light for Rocket Start; revving on red/yellow loses launch speed.",
        "Release `LeftShift` / `LeftControl` early enough in mid-air to land wheels down; landing inverted causes a crash.",
    ],
    goals: &[
        Goal {
            id: "first-race",
            name: "Single Race (Track 1: Living Room)",
            milestone: Milestone::Playable,
            description: "Complete Single Race / Track 1 (`HOUS`) cleanly and persist `saves\\times.dat`.",
        },
        Goal {
            id: "custom-garage-gold",
            name: "Custom Garage Stunt Track Gold Medal",
            milestone: Milestone::Playable,
            description: "Open the Custom Garage Stunt Track Builder, place stunt pieces on the workbench, click Green GO, and earn a Gold Medal.",
        },
        Goal {
            id: "aerial-stunts",
            name: "Clean Aerial Stunts",
            milestone: Milestone::Playable,
            description: "Execute mid-air barrel rolls (`LeftShift`) and flips (`LeftControl`) off ramps and land upright without crashing.",
        },
        Goal {
            id: "championship-tracks-1-6",
            name: "Championship Tracks 1-6",
            milestone: Milestone::Complete,
            description: "Clear Championship Tracks 1-6 (Living Room `HOUS`, Bedroom `TOYS`, Greenhouse `ATRI`, Attic `ATIC`, Sandbox `SAND`, Basement `BASE`) so `saves\\onoff.dat` holds all 6 unlock flags.",
        },
        Goal {
            id: "secret-track-wire-gold",
            name: "Secret Track WIRE Gold Medal",
            milestone: Milestone::Complete,
            description: "With Tracks 1-6 unlocked, right-click the Mattel \"TT\" hotspot at `(270, 100)` on Track Select to arm Secret Track `WIRE` (`WIRE.JAM`) and earn a Gold Medal (`<= 1:10`).",
        },
    ],
    save_files: &[
        "saves\\onoff.dat",
        "saves\\times.dat",
        "saves\\wheel.dat",
        "saves\\track0.dat",
        "saves\\tracks.dat",
    ],
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{
        format_controls_json, format_goals_json, format_meta_json, Action, Command, ControlBus,
        ControlConfig,
    };

    #[test]
    fn title_guide_serializes_valid_json_and_guards_escape() {
        assert!(!GUIDE.is_empty());
        const { assert!(GUIDE.escape_quits) };
        assert_eq!(GUIDE.bpp, 8);
        assert_eq!(GUIDE.refresh_hz, 60);

        for body in [
            format_meta_json(&HOT_WHEELS_STUNT_TRACK_DRIVER, &GUIDE),
            format_controls_json(&HOT_WHEELS_STUNT_TRACK_DRIVER, &GUIDE),
            format_goals_json(&HOT_WHEELS_STUNT_TRACK_DRIVER, &GUIDE),
        ] {
            let v = parse_json(&body).expect("valid JSON");
            assert_eq!(v.as_object().unwrap()["ok"].as_bool(), Some(true));
        }

        let controls_json = parse_json(&format_controls_json(
            &HOT_WHEELS_STUNT_TRACK_DRIVER,
            &GUIDE,
        ))
        .expect("valid controls JSON");
        let controls = controls_json.as_object().unwrap()["controls"]
            .as_array()
            .unwrap();
        for required in [
            "Up",
            "Down",
            "LeftShift",
            "LeftControl",
            "P / Pause",
            "Escape",
        ] {
            assert!(
                controls
                    .iter()
                    .any(|c| c.as_object().unwrap()["input"].as_str() == Some(required)),
                "missing control hint `{required}`"
            );
        }
        let hazards = controls_json.as_object().unwrap()["hazards"]
            .as_array()
            .unwrap();
        assert!(
            hazards
                .iter()
                .any(|h| h.as_str().unwrap_or("").contains("#329")),
            "missing #329 post-race cursor offset hazard"
        );
        assert!(
            hazards
                .iter()
                .any(|h| h.as_str().unwrap_or("").contains("--hold-flips 5")),
            "missing menu click hold-flips hazard"
        );

        let goals_json = parse_json(&format_goals_json(&HOT_WHEELS_STUNT_TRACK_DRIVER, &GUIDE))
            .expect("valid goals JSON");
        let goals = goals_json.as_object().unwrap()["goals"].as_array().unwrap();
        for required_id in [
            "first-race",
            "custom-garage-gold",
            "aerial-stunts",
            "championship-tracks-1-6",
            "secret-track-wire-gold",
        ] {
            assert!(
                goals
                    .iter()
                    .any(|g| g.as_object().unwrap()["id"].as_str() == Some(required_id)),
                "missing goal `{required_id}`"
            );
        }

        let bus = ControlBus::new(ControlConfig {
            title: HOT_WHEELS_STUNT_TRACK_DRIVER.key,
            guide: &GUIDE,
            ..ControlConfig::default()
        });
        let blocked = bus.dispatch(Command::Act {
            action: Action::KeyHold {
                key: "Escape".into(),
                hold_flips: 1,
                hold_ms: None,
            },
            auto_step: false,
        });
        assert!(!blocked.ok, "Escape must be blocked without force");

        let allowed = bus.dispatch_forced(
            Command::Act {
                action: Action::KeyHold {
                    key: "Escape".into(),
                    hold_flips: 1,
                    hold_ms: None,
                },
                auto_step: false,
            },
            true,
        );
        assert!(allowed.ok, "Escape must succeed with force=true");
    }
}
