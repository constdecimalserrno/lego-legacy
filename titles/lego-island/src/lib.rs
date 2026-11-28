//! LEGO Island: the roster's fourth title (#112).

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, EntryImage, Goal, Installed, Kept, Milestone, MouseMode,
    TitleGuide, WINDOWS_95,
};
use lego_legacy_core::Title;

/// Where this title's installer puts it: `%PROGRAM_FILES%\%MAINDIR%`, with
/// `MAINDIR` set to `LEGO Island`, read out of the Wise installer's own
/// script on the disc (`INSTALL.EXE`).
const INSTALL_DIR: &str = "C:\\Program Files\\LEGO Island";

/// The key the installer writes and the game reads its paths back from.
const KEY: &str = "HKLM\\SOFTWARE\\Mindscape\\LEGO Island";

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_ISLAND: Title = Title {
    key: "lego-island",
    name: "LEGO Island",
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. ADR-0003 gives a real title no
    // free frame, and this title has not been seen to author a rate of its
    // own: the old tree found no `rdtsc` in either image and no frame time in
    // its data. So it is the roster's baseline sixty, which is also what VGA's
    // own 640x480 timing refreshes at.
    flip_cost: FlipCost::Refresh(60),
    // Started as a player double-clicking it would start it, with its own
    // quoted path and nothing after it, and not with the command line its
    // settings launcher passes, `isle.exe /diskstream /script
    // \lego\scripts\isle\isle.si` (`LEGOIsle.exe` 0x40309b). Nothing reads
    // those arguments: neither `ISLE.EXE` nor `LEGO1.DLL` holds the string
    // `/diskstream`, and the one `/script` in `LEGO1.DLL` is what its own
    // script command passes to a program it starts (RVA 0x3e5aa), not
    // something it looks for. The old tree measured the same: the same
    // picture at flip 6,000 with them and without (#517 there). So golden
    // rule 2 asks nothing more of the command line, and the machine has no
    // field for one.
    image: EntryImage {
        disc_dir: "lego-island",
        pressing: "lego-island",
        // The game itself, not `LEGOIsle.exe` beside it: that is the settings
        // launcher, byte for byte the disc's `CONFIG.EXE`, and all it does
        // once its dialog closes is start this one (#113).
        path: "DATA/disk/ISLE.EXE",
        kept: Kept::Loose,
        sha256: "5cf57c284973fce9d14f5677a2e4435fd989c5e938970764d00c8932ed5128ca",
        why: "this title has one pressing and its executable is on the disc unwrapped, beside \
              the engine and data its Wise installer copies from the same directory; the \
              settings launcher LEGOIsle.exe, which only starts this one, is not run",
    },
    // One data track and no audio tracks: this title's music is in its own
    // scripts and in MIDI it plays itself.
    red_book: None,
    bank: &[],
    declared: Declared {
        // Windows 95 as it was sold, 4.00.950: what this pressing shipped
        // against, a year before Windows 98. Its box asks for it, its disc
        // carries DirectX 5 for it and a help file for its problems on it
        // (`W95TECHS.HLP`). The same rule the other titles' Windows 98 SE
        // comes from.
        windows: WINDOWS_95,
        // The roster's baseline Pentium II 450. The old tree saw this title
        // measure nothing: no `rdtsc` in either image. `LEGO1.DLL` holds one
        // `cpuid`, leaves 0 and 1 at RVA 0x9d1b9; whether it runs, and what
        // it picks from the answer, is unmeasured, and this value is the
        // baseline until it is.
        hz: lego_legacy_core::clock::DECLARED_HZ,
        processor: Processor::PentiumII,
        // The roster's baseline, and never asked: neither `ISLE.EXE` nor
        // `LEGO1.DLL` imports `GlobalMemoryStatus`, the one call the figure
        // reaches a title through.
        megabytes: 128,
        // 640x480 at sixteen bits: the depth this title picks when `Display
        // Bit Depth` is absent from the registry, which it is here. Its
        // constructor sets the sixteen-bit flag (`ISLE.EXE` 0x401064), and the
        // registry read at 0x402a9d only changes it for a value of 8 or 16.
        // Not yet seen asked for: the title has not set a mode on this
        // machine yet.
        modes: &[(640, 480, 16)],
        // The primary tree's names alone, by #142's rule: Joliet is LEGO
        // Chess's alone, because Alpha Team's runs move with it, until the
        // maintainer decides (#144). Nothing in this title asks for it: read
        // through its Joliet tree, as Windows 95's CDFS reads it, this
        // disc answers the same names but for case, and none of its runs
        // moved with it on (measured at 7509b49).
        joliet: false,
        computer: "LEGO-LEGACY",
        // None: no gate run asks for a library, and the two places it can,
        // the C runtimes' message boxes, ask for USER32, which core writes (#146).
        absent_libraries: &[],
        // None, as every released title's machine has had since its routes were
        // recorded: C: and D: only.
        floppy: false,
        why: "Windows 95 is what this pressing shipped against; the rest is the roster's \
              baseline, unmeasured against this title because it has not asked for any of it \
              yet",
    },
    // No cabinet on this disc: the Wise installer copies DATA\disk whole, and
    // the declared tree says so rather than unpacking anything. Beside it the
    // installer copies its own uninstaller, the disc's help file and
    // Mindscape's registration program, none of them the game's.
    installed: Installed::Copied("DATA/disk"),
    // What the installer writes under `SOFTWARE\Mindscape\LEGO Island`, read
    // out of its script: the three paths, and nothing else. The game asks for
    // sixteen values there, measured here as in the old tree (#321 there),
    // and takes its own compiled default for each one it does not find; the
    // other thirteen are the settings dialog's, which the installer never
    // writes and the launcher is not run to write.
    //
    // Each path has no trailing backslash: the game puts `\lego\…` straight
    // onto it, and a doubled separator is a different path on the declared
    // tree.
    registry: &[
        // Where the game finds its install: `LEGO\data\WORLD.WDB` and the rest.
        (KEY, "diskpath", INSTALL_DIR),
        // Where it finds its scripts: the disc in the declared drive. The
        // installer writes the directory it was run from, which is the disc's
        // root.
        (KEY, "cdpath", "D:"),
        // Where it keeps its saves: the install directory, as the installer
        // writes it.
        (KEY, "savepath", INSTALL_DIR),
    ],
    install_dir: INSTALL_DIR,
    // Not yet measured: no movie of this title's own has played on this
    // machine, so there is nothing a click could be translated inside of.
    click_in_a_movie: None,
    // One for one, because the pointer is never the title's to steer (#124).
    // `Draw Cursor` is absent from the registry, so the game takes its own
    // default, off, and leaves the pointer to Windows: it never hides it,
    // never puts it anywhere, and reads it only as the place each mouse
    // message carries. The host's pointer stays the person's, and a declared
    // speed only acts on a pointer a title has taken.
    pointer_speed: (1.0, 1.0),
    // Static playability metadata served via `/api/meta`, `/api/controls`, and
    // `/api/goals` (#311, #333).
    guide: TitleGuide {
        summary: "First-person 3D exploration, vehicle building, minigame adventure, and Brickster finale on LEGO Island.",
        resolution: (640, 480),
        bpp: 16,
        refresh_hz: 60,
        mouse_mode: MouseMode::AbsoluteKept,
        escape_quits: false,
        skip_keys: &["Space"],
        cadence: AgentCadence::BurstThenPause { burst_ms: 500 },
        controls: &[
            ControlHint {
                input: "Up / Down (or Keypad8 / Keypad2)",
                action: "Walk forward/back & vehicle throttle/reverse",
                notes: "Move on foot or accelerate/brake vehicles (bicycle, skateboard, race car, ambulance, tow truck, jetski, dune buggy, helicopter).",
            },
            ControlHint {
                input: "Left / Right (or Keypad4 / Keypad6)",
                action: "Turn in place",
                notes: "Rotates ~8 deg/flip on foot (~11-12 flips for a 90 deg turn). Hold LeftControl/RightControl with Left/Right for 4x sharper turning.",
            },
            ControlHint {
                input: "chord Up,Left / chord Up,Right",
                action: "Steer skateboard & vehicles while moving",
                notes: "Skateboard and vehicle steering at speed requires holding Up simultaneously with Left or Right.",
            },
            ControlHint {
                input: "Left click (quick, hold_flips=2)",
                action: "3D pick / interact / enter buildings & mount vehicles",
                notes: "Keep clicks well under 1000 ms to pick 3D objects, talk to NPCs, enter visible buildings, mount bikes, pick portraits, use paint/mood tools, or click the floor to skip intro/welcome.",
            },
            ControlHint {
                input: "Left hold still (>= 1050 ms / ~63 flips)",
                action: "Mouse walk-drag",
                notes: "Holding Left button still fires the 1-second `WM_TIMER` to enter continuous pointer-steered walking.",
            },
            ControlHint {
                input: "Left drag (step_px=16)",
                action: "Assemble vehicles in workshops",
                notes: "Drag the part matching the currently flashing red wireframe ghost from the shelf (or E2 preview) onto the ghost slot; LEGO1 enforces strict sequential order and returns out-of-order parts to the E2 preview on release.",
            },
            ControlHint {
                input: "Shelf Elevator B3 (95, 165) & Platform Lever H3 (598, 225)",
                action: "Scroll workshop shelves & rotate build platform (Dune Buggy)",
                notes: "Click the red Octan gas pump at (95, 165) to cycle hidden parts shelves until the part matching the flashing red ghost is visible; click the yellow lever at (598, 225) to rotate the build table.",
            },
            ControlHint {
                input: "Space",
                action: "Skip cutscene / dialog",
                notes: "Skips or cancels the active animation, cutscene, or dialog sequence.",
            },
            ControlHint {
                input: "Escape",
                action: "Return to Information Center / Quit prompt",
                notes: "Returns to the Information Center from the Island or activities; inside Info Center opens the Quit confirmation (does not immediately quit).",
            },
            ControlHint {
                input: "HUD (i) button A2 (38, 148)",
                action: "Warp to Information Center",
                notes: "Click the on-screen `(i)` HUD button at `(38, 148)` (or click the Info Center tower in 3D) to return inside.",
            },
            ControlHint {
                input: "F12",
                action: "Quicksave game",
                notes: "Immediately saves `G0.GS` and `History.gsi` whenever a player is signed into the Registration Book.",
            },
            ControlHint {
                input: "A-Z / Backspace + Tick (519, 135)",
                action: "Sign Registration Book",
                notes: "Click the blue book `(322, 294)`, type up to 7 letters, click green tick `(519, 135)` (or `(519, 163)` for row 2 saved player), then pick a portrait (Pepper `(322, 105)`, Nick `(372, 105)`).",
            },
            ControlHint {
                input: "Info Center arrows (28, 310) -> (35, 140) -> Door (330, 220)",
                action: "Exit to LEGO Island",
                notes: "Navigate from Info Center main room to Elevator room, then Revolving Door; door unlocks only after signing and picking a portrait.",
            },
            ControlHint {
                input: "Quit Door (590, 260) -> Yes (210, 400)",
                action: "Save & quit",
                notes: "Click the Information Center Quit Door `(590, 260)`, then the green Yes brick `(210, 400)` (or send clean `Close`) to save `G0.GS` + `History.gsi`, roll credits, and exit cleanly.",
            },
        ],
        hazards: &[
            "Revolving Door stays locked until you both sign the Registration Book AND click a character portrait (blue frame); a new name selects nobody by default.",
            "Skateboard and vehicle steering requires holding Up simultaneously (`chord Up,Left` / `chord Up,Right`); bare Left/Right stalls or rotates in place (~11-12 flips per 90 deg).",
            "Holding Left mouse button still for >= 1000 ms fires `WM_TIMER` and enters continuous walk-drag; keep normal pick clicks brief (`hold_flips=2`).",
            "Signing writes only `Players.gsi` (shifting old `G0.GS` to `G1.GS`); progress (`G0.GS`, `History.gsi`) is only persisted on `F12`, Quit Door -> Yes, switching players, or clean `Close` — never force-kill before saving.",
            "Each character delivers pizza to a different target: Pepper's Jail route is blocked by town roadworks (and triggers Act 2 once the Helicopter is built); pick Nick `(372, 105)` for the shortest direct delivery east+north to the Gas Station.",
            "Vehicle Build workshops enforce strict sequential part order (`IsNextPartToPlace`): only the part matching the currently flashing red wireframe ghost snaps on release (any other part returns to the E2 preview). Some required parts start off-shelf — click the Shelf Elevator (`ShelfUp_Ctl`, red Octan gas pump `(95, 165)` in Dune Buggy) to scroll shelves before dragging.",
            "Cutscenes, spoken lines, and room transitions span 100-600+ flips while the hourglass cursor shows; wait for frames to settle or use `Space` / quick floor clicks instead of queueing blind clicks.",
        ],
        goals: &[
            Goal {
                id: "information-center",
                name: "Sign in & exit Information Center",
                milestone: Milestone::FirstLight,
                description: "Skip intro, sign a player name in the Registration Book `(322, 294)`, pick a character portrait, and exit through the Revolving Door `(330, 220)`.",
            },
            Goal {
                id: "island-exploration",
                name: "Ride bicycle & explore LEGO Island",
                milestone: Milestone::Playable,
                description: "Mount the red bicycle outside the Information Center, steer roads using `Up` and `chord Up,Left`/`Up,Right`, and reach key island landmarks (Pizzeria, Gas Station, Hospital, Police Station, Beach, Race Track).",
            },
            Goal {
                id: "pizza-delivery",
                name: "Complete Pizza Delivery",
                milestone: Milestone::Playable,
                description: "Click the Pizzeria, click the pizza Papa Brickolini holds out, deliver it on the skateboard (Nick rides east then north to Nubby at the Gas Station for a Red score brick), and save.",
            },
            Goal {
                id: "island-minigames",
                name: "Complete Ambulance, Tow Truck, Race Track & Jetski Race",
                milestone: Milestone::Playable,
                description: "Finish the remaining island activities — Ambulance (Hospital), Tow Truck (Gas Station), 2-lap Race Track, and Jetski Race (Beach) — to earn Score Cube bricks.",
            },
            Goal {
                id: "vehicle-builds",
                name: "Build Race Car, Dune Buggy, Jetski & Helicopter",
                milestone: Milestone::Complete,
                description: "Assemble all four custom vehicles by dragging parts onto workshop slots; building the Helicopter unlocks the Brickster finale.",
            },
            Goal {
                id: "brickster-finale",
                name: "Stop the Brickster (Acts 2 & 3) & reach credits",
                milestone: Milestone::Credits,
                description: "As Pepper with the Helicopter built, deliver pizza to the Jail to trigger Act 2 (recover stolen bricks) and Act 3 (fly the Helicopter, throw pizzas/donuts to capture the Brickster), then roll credits.",
            },
        ],
        save_files: &["Players.gsi", "History.gsi", "G0.GS"],
    },
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{format_controls_json, format_goals_json, format_meta_json};

    #[test]
    fn title_guide_serializes_valid_json() {
        let guide = &LEGO_ISLAND.guide;
        assert!(!guide.is_empty());
        const { assert!(!LEGO_ISLAND.guide.escape_quits) };
        assert_eq!(guide.resolution, (640, 480));
        assert_eq!(guide.bpp, 16);
        assert_eq!(guide.refresh_hz, 60);
        for body in [
            format_meta_json(&LEGO_ISLAND),
            format_controls_json(&LEGO_ISLAND),
            format_goals_json(&LEGO_ISLAND),
        ] {
            let v = parse_json(&body).expect("valid JSON");
            assert_eq!(v.as_object().unwrap()["ok"].as_bool(), Some(true));
        }
        assert!(guide
            .controls
            .iter()
            .any(|c| c.input.contains("chord Up,Left")));
        assert!(guide.controls.iter().any(|c| c.input == "F12"));
        assert!(guide.controls.iter().any(|c| c.input.contains("(38, 148)")));
        assert!(guide.controls.iter().any(|c| c.input.contains("(95, 165)")));
        assert!(guide
            .hazards
            .iter()
            .any(|h| h.contains("flashing red") && h.contains("(95, 165)")));
        assert_eq!(guide.goals.len(), 6);
        assert!(guide
            .goals
            .iter()
            .any(|g| g.id == "brickster-finale" && g.milestone == Milestone::Credits));
    }
}
