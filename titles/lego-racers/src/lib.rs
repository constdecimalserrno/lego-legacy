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
    // Flip does unless a title asks otherwise. Sixty a second is the roster's
    // baseline; this title has not yet been seen to author another rate (#370).
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        // The roster's baseline: Masterpiece ships against Windows 98 SE and
        // DirectX 6.
        windows: WINDOWS_98_SE,
        // The roster's baseline Pentium II 450 until first-light bringup (#370)
        // measures whether the title spins on the clock.
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
        why: "Windows 98 SE and the roster's baseline Pentium II 450, 128 MB, 640x480x16, \
              and C:/D: without Joliet or a floppy drive; unmeasured against the title until \
              first-light bringup (#370)",
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
    // Not yet measured: ActiveMovie intro playback waits on bringup (#370+).
    click_in_a_movie: None,
    // One for one: in fullscreen `LEGORacers.exe` hides the OS cursor and
    // accumulates buffered DirectInput `GUID_SysMouse` relative deltas at 1:1
    // scale (`0x4bc900` / `0x4bca40`). Empirical cursor calibration follows in
    // first-light bringup.
    pointer_speed: (1.0, 1.0),
};

/// Static playability metadata served via `/api/meta`, `/api/controls`, and
/// `/api/goals` (#311, #369).
///
/// Controls, progression, and save paths are sourced from `MANUAL.PDF`,
/// `README.TXT`, `LEGO.JAM` string tables, and `LEGORacers.exe` disassembly.
/// Cursor `start_pos`, menu hotspots, and intro skip timing will be empirically
/// verified once first light lands (#370+).
pub const GUIDE: TitleGuide = TitleGuide {
    display_name: "LEGO Racers",
    summary: "3D LEGO kart racing with custom brick-built cars and drivers, stackable color + Power Plus brick power-ups, 6 circuits across 12 tracks (3 standard, 3 mirrored), Time Race vs Veronica Voltage, and Circuit 7 Rocket Racer Run.",
    resolution: (640, 480),
    bpp: 16,
    refresh_hz: 60,
    mouse_mode: MouseMode::SoftwareRelative {
        virtual_span: (640, 480),
        start_pos: (320, 240),
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
            notes: "Fires or activates the currently held Red (projectile), Yellow (hazard), Blue (shield), or Green (turbo) brick, upgraded by up to 3 White Power Plus bricks (`MANUAL.PDF` pp. 5, 9). Do NOT confuse with `Space`.",
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
            notes: "Opens the in-race `GAME PAUSED` menu (`CONTINUE`, `RESTART RACE`, `RESTART CIRCUIT`, `EXIT`) or backs out of menus (`MANUAL.PDF` p. 5).",
        },
        ControlHint {
            input: "Car Builder hotkeys & mouse drag",
            action: "Build custom LEGO car (`BUILD` -> `NEW RACER` / `EDIT RACER`)",
            notes: "`Insert`/`PageUp` cycle Car Set, `Delete`/`PageDown` cycle LEGO Brick, `S`/`X`/`A`/`D` (or arrows) orbit camera, `Keypad1`-`4`/`6`-`9` move floating brick, `Keypad5` / right-click rotates 90 deg, `KeypadPlus` / double left-click drops brick, `KeypadMinus` undoes, left-drag moves bricks or orbits car (`MANUAL.PDF` p. 22).",
        },
    ],
    hazards: &[
        "`Return` (Enter) activates power-ups and `Space` performs a Power Slide; pressing `Space` when expecting to fire a power-up triggers a drift instead.",
        "Holding `Up` during the 3-2-1 countdown loses the Turbo Start; wait until `GO` appears before accelerating.",
        "`VERSUS RACE` (2-player split-screen) requires a joystick/game controller attached (`MANUAL.PDF` p. 15) and is unavailable with keyboard/mouse alone.",
        "In fullscreen, `LEGORacers.exe` hides the OS cursor (`ShowCursor(FALSE)`), never calls `SetCursorPos`, and drives its software arrow via 100% buffered DirectInput `GetDeviceData` relative deltas; initial `start_pos` and exact menu hotspots await empirical first-light calibration (#370+).",
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
            description: "Enter `BUILD` -> `NEW RACER`, customize a driver, sign the license (`type_text`), assemble a car (Quick Build or custom brick placement), save to `Save\\LEGORac1`, and return to the Main Menu.",
        },
        Goal {
            id: "single-race-win",
            name: "Win Single Race (Imperial Grand Prix)",
            milestone: Milestone::Playable,
            description: "Enter `SINGLE RACE` on Imperial Grand Prix, nail Turbo Start on `GO`, collect and trigger an upgraded power-up (`Return`), finish all 3 laps in 1st place, and persist best times to `Save\\LEGORac1`.",
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
    save_files: &["Save\\LEGORac1"],
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{format_controls_json, format_goals_json, format_meta_json};

    #[test]
    fn title_guide_serializes_valid_json() {
        assert!(!GUIDE.is_empty());
        const { assert!(!GUIDE.escape_quits) };
        for body in [
            format_meta_json(&LEGO_RACERS, &GUIDE),
            format_controls_json(&LEGO_RACERS, &GUIDE),
            format_goals_json(&LEGO_RACERS, &GUIDE),
        ] {
            let v = parse_json(&body).expect("valid JSON");
            assert_eq!(v.as_object().unwrap()["ok"].as_bool(), Some(true));
        }
    }
}
