//! LEGO Racers: the roster's sixth title (#369).

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::LEGO_RACERS as ENTRY;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, Goal, Installed, Milestone, MouseMode, TitleGuide,
    WINDOWS_98_SE,
};
use lego_legacy_core::Title;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_RACERS: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. Verified empirically across a
    // full Imperial Grand Prix lap (#373): the 3-2-1-GO countdown advances at
    // exact 60-flip (1.00 s) intervals and the in-race lap timer ticks at
    // 60.00 Hz with zero clock-spin stalls (`guest_fps = 60.0`, ~0.68–1.25 M
    // instructions/flip, well inside Pentium II 450's 7.5 M/flip budget).
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        // The roster's baseline: Masterpiece ships against Windows 98 SE and
        // DirectX 6.
        windows: WINDOWS_98_SE,
        // Pentium II 450 (450 MHz -> 7.5 M instructions per 60 Hz flip): a 3D
        // race flip retires ~0.68–1.25 M instructions with zero clock spins and
        // zero over-budget stalls on Imperial Grand Prix (#373).
        hz: lego_legacy_core::clock::DECLARED_HZ,
        processor: Processor::PentiumII,
        megabytes: 128,
        // 640x480 at sixteen bits: the default fullscreen mode used by
        // `LEGORacers.exe` and `GolDP.dll` unless CLI flags override it.
        modes: &[(640, 480, 16)],
        // Primary ISO-9660 tree alone by #142's rule: everything the title
        // installs lives inside `DATA1.HDR` / `DATA1.CAB` at the disc root.
        joliet: false,
        computer: "LEGO-LEGACY",
        // Default graphics flags load `GolDP.DLL` directly (0x48984a) without
        // probing `GolGlide.DLL`, `GolSoft.DLL`, or `GolD3D.DLL`.
        absent_libraries: &[],
        // C: and D: only; Masterpiece `LEGORacers.exe` performs no CD volume
        // or floppy-drive check.
        floppy: false,
        why: "Windows 98 SE and Pentium II 450, 128 MB, 640x480x16 at 60 Hz refresh, \
              and C:/D: without Joliet or a floppy drive; verified on Imperial Grand Prix \
              at exact 60.0 Hz lap-timer cadence with zero clock-spin stalls (#373)",
    },
    // Nothing of the game lies loose on the disc: `LEGORacers.exe`,
    // `GolDP.dll`, `LEGO.JAM`, all 50 `.tun` tracks, and all 3 intro `.avi`
    // files are packed inside the pressing's InstallShield 5 cabinet
    // (`DATA1.HDR` + `DATA1.CAB`, 56 entries at the install root).
    installed: Installed::Cabinet,
    // The binary's sole registry read (`0x414f40`) queries `LangID` (DWORD)
    // under `HKLM\Software\LEGO Media\Games\LEGO Racers\0.90.000` and defaults
    // cleanly to `0` (English) when the key is absent.
    registry: &[],
    // Where the official README and installer place it under Program Files.
    install_dir: "C:\\Program Files\\LEGO Media\\Games\\LEGO Racers",
    // `LEGORacers.exe`'s ActiveMovie loop skips skippable movies on `WM_KEYUP`;
    // translating mouse clicks during movies to Space (`0x39`) lets clicks skip too (#372).
    click_in_a_movie: Some(0x39),
    // One for one: in fullscreen `LEGORacers.exe` hides the OS cursor via
    // `DISCL_EXCLUSIVE` DirectInput mouse acquisition and accumulates buffered
    // `GUID_SysMouse` relative deltas at 1:1 scale (`0x4bc900` / `0x4bca40`).
    pointer_speed: (1.0, 1.0),
};

/// Static playability metadata served via `/api/meta`, `/api/controls`, and
/// `/api/goals` (#311, #369, #374).
///
/// Controls, progression, menu hotspots, cursor calibration, and save paths
/// are verified against `MANUAL.PDF`, `README.TXT`, `LEGO.JAM` string tables,
/// `LEGORacers.exe` disassembly, and live real-disc runs.
pub const GUIDE: TitleGuide = TitleGuide {
    display_name: "LEGO Racers",
    summary: "3D LEGO kart racing with custom brick-built cars and drivers, stackable color + Power Plus brick power-ups, 6 circuits across 12 tracks (3 standard, 3 mirrored), Time Race vs Veronica Voltage, and Circuit 7 Rocket Racer Run.",
    resolution: (640, 480),
    bpp: 16,
    refresh_hz: 60,
    mouse_mode: MouseMode::SoftwareRelative {
        virtual_span: (640, 480),
        start_pos: (0, 0),
    },
    escape_quits: false,
    skip_keys: &["Space", "Escape", "Return"],
    cadence: AgentCadence::MicroStep { flips_per_step: 8 },
    controls: &[
        ControlHint {
            input: "Up",
            action: "Accelerate / Turbo Start",
            notes: "Hold to drive forward. For Turbo Start, wait during the 3-2-1 countdown and press `Up` right as `GO` appears (`MANUAL.PDF` p. 6).",
        },
        ControlHint {
            input: "Down",
            action: "Brake / Reverse",
            notes: "Slow down for tight turns or reverse out of walls (`MANUAL.PDF` p. 5).",
        },
        ControlHint {
            input: "Left / Right",
            action: "Steer Left / Right",
            notes: "Steer the kart; pulse in short `MicroStep` bursts on straights and hold into corners (`MANUAL.PDF` p. 5).",
        },
        ControlHint {
            input: "Return",
            action: "Activate Power-Up / Menu Confirm",
            notes: "Fires or activates the currently held Red (projectile), Yellow (hazard), Blue (shield), or Green (turbo) brick, upgraded by up to 3 White Power Plus bricks (Lv0-Lv3: Red Cannon Ball/Grappling Hook/Lightning Wand/3 Guided Rockets; Yellow Oil Slick/Gun Powder/Magnetic Trap/Mummy's Curse; Blue 5s/6s/8s/10s Shield; Green Turbo/Extra/Flying/Warp Turbo; `MANUAL.PDF` pp. 5, 9). Do NOT confuse with `Space`.",
        },
        ControlHint {
            input: "Space",
            action: "Power Slide (Drift)",
            notes: "Hold `Up` + `Space` + `Left`/`Right` (`chord`) to carve high-speed drifts; add `Down` (`Up` + `Down` + `Space` + `Left`/`Right`) for a Super Slide pivot (`MANUAL.PDF` pp. 7-8).",
        },
        ControlHint {
            input: "RightControl",
            action: "Change Camera View",
            notes: "Cycles chase / close / wide racing camera angles (`MANUAL.PDF` p. 5).",
        },
        ControlHint {
            input: "RightAlt",
            action: "Reverse Camera View (Look Backwards)",
            notes: "Hold to look behind the kart (`MANUAL.PDF` p. 5).",
        },
        ControlHint {
            input: "M",
            action: "Toggle HUD Map / Speedometer",
            notes: "Cycles Speedometer -> Course Map -> Close-up Map -> Off (`MANUAL.PDF` pp. 5, 10-11).",
        },
        ControlHint {
            input: "Escape",
            action: "Pause Race / Menu Back",
            notes: "Opens the in-race `GAME PAUSED` menu (`CONTINUE`, `RESTART RACE`, `RESTART CIRCUIT`, `EXIT`) or backs out of Circuit/Track Select (`MANUAL.PDF` p. 5). In `BUILD MENU`, use `MAIN MENU` `(78, 435)` or arrow keys + `Return`.",
        },
        ControlHint {
            input: "Left click / Menu navigation",
            action: "Select menu buttons (`640x480` hotspots)",
            notes: "Main Menu: `BUILD` `(65, 135)`, `CIRCUIT RACE` `(107, 195)`, `SINGLE RACE` `(101, 235)`, `TIME RACE` `(91, 315)`, `OPTIONS` `(79, 375)`, `QUIT` `(58, 435)`. Circuit Select: `OK` `(48, 395)`, `MAIN MENU` `(78, 435)`. 3D Car Builder Exit Door: `(68, 435)`. Coordinate `click` (default `settle_flips: 1`) lands and clicks on the first click; arrows + `Return` also work.",
        },
        ControlHint {
            input: "Car Builder hotkeys & mouse drag",
            action: "Build custom LEGO car (`BUILD` -> `NEW RACER` / `EDIT RACER`)",
            notes: "`Insert`/`PageUp` cycle Car Set, `Delete`/`PageDown` cycle LEGO Brick, `S`/`X`/`A`/`D` (or arrows) orbit camera, `Keypad1`-`4`/`6`-`9` move floating brick, `Keypad5` / right-click rotates 90 deg, `KeypadPlus` / double left-click drops brick, `KeypadMinus` undoes, left-drag (`mouse_down`/`move`/`mouse_up`) moves bricks or orbits car (`MANUAL.PDF` p. 22).",
        },
    ],
    hazards: &[
        "`Return` (Enter) activates power-ups and `Space` performs a Power Slide; pressing `Space` when expecting to fire a power-up triggers a drift instead.",
        "Holding `Up` during the 3-2-1 countdown loses the Turbo Start; wait until `GO` appears before accelerating.",
        "`VERSUS RACE` (2-player split-screen) requires a joystick/game controller attached (`MANUAL.PDF` p. 15) and is grayed out with keyboard/mouse alone.",
        "`MENUDATA/ARROW.BMP` starts at `(0, 0)` (and resets to `(0, 0)` after exiting a race), clamps inside `[0..632, 0..472]`, and runs `Menu::OnMouseMove` (`0x48b590`) before adding `(dx, dy)` (`0x4b1e70`): use coordinate `click` (default `settle_flips: 1`) rather than a single-flip `move` followed by an in-place click.",
        "In `BUILD MENU`, `Escape` does not return to Main Menu — click `MAIN MENU` `(78, 435)` or navigate with arrows + `Return`. Saves write to `Save\\0\\LEGORac1` (slots `0..15`) and flush on clean exit.",
    ],
    goals: &[
        Goal {
            id: "boot-main-menu",
            name: "Boot to Main Menu",
            milestone: Milestone::FirstLight,
            description: "Clear the intro movies (`lmicmp.avi`, `HVSCmp.avi`, `introcmp.avi`) and land on the Main Menu with `CIRCUIT RACE`, `SINGLE RACE`, and `BUILD` visible.",
        },
        Goal {
            id: "build-custom-racer",
            name: "Build Custom Driver & Car",
            milestone: Milestone::Playable,
            description: "Enter `BUILD` -> `NEW RACER`, customize a driver, sign the license (`type_text`), assemble a car (Quick Build or custom brick placement), save to `Save\\0\\LEGORac1`, and return to the Main Menu.",
        },
        Goal {
            id: "single-race-win",
            name: "Win Single Race (Imperial Grand Prix)",
            milestone: Milestone::Playable,
            description: "Enter `SINGLE RACE` on Imperial Grand Prix, nail Turbo Start on `GO`, collect and trigger an upgraded power-up (`Return`), finish all 3 laps in 1st place, and persist best times to `Save\\0\\LEGORac1`.",
        },
        Goal {
            id: "circuit-1-redbeard",
            name: "Win Circuit 1 (Captain Redbeard)",
            milestone: Milestone::Playable,
            description: "Win the 4-race Circuit 1 championship (Imperial Grand Prix, Dark Forest Dash, Magma Moon Marathon, Desert Adventure Dragway) on points to defeat Captain Redbeard and unlock Circuit 2 plus Redbeard's Car Set.",
        },
        Goal {
            id: "rocket-racer-champion",
            name: "Defeat Rocket Racer (Circuit 7 -> Credits)",
            milestone: Milestone::Credits,
            description: "Clear Circuits 1-6 (Redbeard, Kahuka, Basil, Johnny Thunder, Baron von Barron, Gypsy Moth) and beat Rocket Racer on Circuit 7 (`Rocket Racer Run`) to win the campaign and reach Credits.",
        },
    ],
    save_files: &["Save\\0\\LEGORac1"],
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{
        format_controls_json, format_goals_json, format_meta_json, Action, Command, ControlBus,
        ControlConfig, ExecMode, InputOwner,
    };

    #[test]
    fn title_guide_serializes_valid_json_and_declares_calibrated_metadata() {
        assert!(!GUIDE.is_empty());
        const { assert!(!GUIDE.escape_quits) };
        assert_eq!(
            GUIDE.mouse_mode,
            MouseMode::SoftwareRelative {
                virtual_span: (640, 480),
                start_pos: (0, 0),
            }
        );
        assert_eq!(GUIDE.cadence, AgentCadence::MicroStep { flips_per_step: 8 });

        let meta = parse_json(&format_meta_json(&LEGO_RACERS, &GUIDE)).expect("valid meta JSON");
        let meta_obj = meta.as_object().unwrap();
        assert_eq!(meta_obj["ok"].as_bool(), Some(true));
        assert_eq!(meta_obj["title"].as_str(), Some("lego-racers"));
        let guide = meta_obj["guide"].as_object().unwrap();
        let mm = guide["mouse_mode"].as_object().unwrap();
        assert_eq!(mm["kind"].as_str(), Some("software_relative"));
        assert_eq!(mm["recenter_every_flip"].as_bool(), Some(false));
        assert_eq!(
            mm["virtual_span"].as_array().unwrap()[0].as_u64(),
            Some(640)
        );
        assert_eq!(
            mm["virtual_span"].as_array().unwrap()[1].as_u64(),
            Some(480)
        );
        assert_eq!(mm["start_pos"].as_array().unwrap()[0].as_i64(), Some(0));
        assert_eq!(mm["start_pos"].as_array().unwrap()[1].as_i64(), Some(0));

        let controls =
            parse_json(&format_controls_json(&LEGO_RACERS, &GUIDE)).expect("valid controls JSON");
        let ctrl_obj = controls.as_object().unwrap();
        assert_eq!(ctrl_obj["ok"].as_bool(), Some(true));
        assert_eq!(ctrl_obj["escape_quits"].as_bool(), Some(false));
        let hints = ctrl_obj["controls"].as_array().unwrap();
        assert!(hints.len() >= 10);
        for required in [
            "Up",
            "Down",
            "Left / Right",
            "Return",
            "Space",
            "RightControl",
            "RightAlt",
            "M",
            "Escape",
        ] {
            assert!(
                hints
                    .iter()
                    .any(|h| h.as_object().unwrap()["input"].as_str() == Some(required)),
                "missing required control hint `{required}`"
            );
        }
        assert!(ctrl_obj["hazards"].as_array().unwrap().len() >= 4);

        let goals = parse_json(&format_goals_json(&LEGO_RACERS, &GUIDE)).expect("valid goals JSON");
        let goals_obj = goals.as_object().unwrap();
        assert_eq!(goals_obj["ok"].as_bool(), Some(true));
        let goal_list = goals_obj["goals"].as_array().unwrap();
        assert_eq!(goal_list.len(), 5);
        let ids: Vec<&str> = goal_list
            .iter()
            .map(|g| g.as_object().unwrap()["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "boot-main-menu",
                "build-custom-racer",
                "single-race-win",
                "circuit-1-redbeard",
                "rocket-racer-champion",
            ]
        );
        assert_eq!(
            goals_obj["save_files"].as_array().unwrap()[0].as_str(),
            Some("Save\\0\\LEGORac1")
        );
    }

    #[test]
    fn control_bus_initializes_at_origin_and_allows_escape_without_force() {
        let bus = ControlBus::new(ControlConfig {
            title: LEGO_RACERS.key,
            mode: ExecMode::Step,
            owner: InputOwner::Agent,
            pointer_speed: LEGO_RACERS.pointer_speed,
            flip_cost: LEGO_RACERS.flip_cost,
            guide: &GUIDE,
            ..ControlConfig::default()
        });
        let st = bus.status();
        assert_eq!(st.pointer_pos, (0, 0));
        assert_eq!(st.hw_pointer_pos, (320, 240));
        assert!(!st.pointer_shown);

        let resp = bus.dispatch(Command::Act {
            action: Action::KeyHold {
                key: "Escape".to_owned(),
                hold_flips: 2,
                hold_ms: None,
            },
            auto_step: false,
        });
        assert!(
            resp.ok,
            "escape_quits=false must allow Escape without force=true: {:?}",
            resp.error
        );
    }
}
