//! LEGOLAND: the roster's seventh title (#393).

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::LEGOLAND as ENTRY;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, Goal, Installed, Kept, Milestone, MouseMode, TitleGuide,
    WINDOWS_98_SE,
};
use lego_legacy_core::Title;

/// The archive the roster keeps the game in, which setup expands into the
/// install directory.
const ARCHIVE: &str = match ENTRY.image.kept {
    Kept::Imploded(archive) => archive,
    _ => panic!("LEGOLAND keeps its game imploded in an archive"),
};

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGOLAND: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // `Legoland.exe` paces frames with a 28 ms (~35.7 Hz) `timeGetTime()` floor
    // (`0x4661d0`), matching 36 Hz (`27.78 ms`/flip) with zero over-budget drift (#396).
    flip_cost: FlipCost::Refresh(36),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        // The roster's baseline: LEGOLAND (2000) and its Masterpiece pressing
        // target Windows 98 SE and DirectX 6/7.
        windows: WINDOWS_98_SE,
        hz: lego_legacy_core::clock::DECLARED_HZ,
        processor: Processor::PentiumII,
        megabytes: 128,
        // 640x480 at sixteen bits (RGB565): what `InitScreen` (`0x463f14`)
        // requests via DirectDraw2 `SetDisplayMode(640, 480, 16, 0, 0)`.
        modes: &[(640, 480, 16)],
        // Mandatory: 965 of 1,355 paths on disc differ between ISO-9660 8.3
        // and Joliet, including `Legoland.res`, `Graphics1.res`,
        // `Graphics2.res`, `Speech\*.wav`, and mixed-case `.avi` files.
        joliet: true,
        computer: "LEGO-LEGACY",
        // `Legoland.exe` calls `LoadLibraryA("Ir50_32.dll")` (`0x45965e`) and
        // immediately passes the handle to `FreeLibrary` (`0x4596ea`) with
        // zero `GetProcAddress` calls; all video decoding runs through
        // `AVIFIL32!AVIStreamGetFrame*`.
        absent_libraries: &[(
            "Ir50_32.dll",
            "Intel Indeo Video 5 runtime probed by LoadLibraryA (0x45965e) with zero \
             GetProcAddress calls; video decoding runs through AVIFIL32",
        )],
        // C: and D: only; the CD check (`0x450f30`) scans all 32 bits of
        // `GetLogicalDrives()` for `DRIVE_CDROM` (`"CDFS"`, `"LEGOLAND"`) and
        // needs no floppy drive.
        floppy: false,
        why: "Windows 98 SE and Pentium II 450, 128 MB, 640x480x16 at 36 Hz matching the \
              guest's 28 ms (~35.7 Hz) frame floor (0x4661d0), C:/D: with Joliet for \
              .res/Speech/AVI long names and no floppy drive, and Ir50_32.dll absent (#393, #396)",
    },
    // `Legoland.exe` and its 330 companion files (`Lego.TTF`, `strings\stab.str`,
    // `IMusic\*`, `zbuffers\*`, `RollerCoaster\*`, `AD_*.avi`) live inside
    // InstallShield 3 `main.z`. `Legoland.res`, `Graphics1.res`, `Graphics2.res`,
    // `Speech\*.wav`, and fullscreen `.avi` movies stay on `D:\` (`beside: &[]`).
    installed: Installed::Expanded {
        archive: ARCHIVE,
        beside: &[],
    },
    // `Legoland.exe` imports zero `Reg*` functions (`ADVAPI32.dll` imports only
    // `GetUserNameA`).
    registry: &[],
    // Where `SETUP.INS` installs the game under Program Files.
    install_dir: "C:\\Program Files\\LEGO Media\\Games\\LEGOLAND",
    // `Intro.avi` accepts mouse buttons directly alongside `Space` (`0x4595e7`).
    click_in_a_movie: None,
    // One for one: `Legoland.exe` hides the OS pointer (`SetCursor(NULL)` +
    // `DISCL_EXCLUSIVE` `GUID_SysMouse`) and integrates DirectInput relative
    // `(lX, lY)` deltas at 1:1 scale once `SPI_GETMOUSE` returns zero accel.
    pointer_speed: (1.0, 1.0),
};

/// Static playability metadata served via `/api/meta`, `/api/controls`, and
/// `/api/goals` (#393, #396).
///
/// Controls, cursor calibration, hotspots, hazards, and progression goals are
/// sourced from `Legoland.exe` disassembly, `Graphics1.res`, and `readme.txt`.
pub const GUIDE: TitleGuide = TitleGuide {
    display_name: "LEGOLAND",
    summary: "Isometric 640x480 theme-park management sim across 5 tutorials, 10 Miniland levels, and Free Play: build themed attractions, link entrance paths, supply power, manage Mechanics and Gardeners, and pass Mr. Bimble's Park Inspector appraisals.",
    resolution: (640, 480),
    bpp: 16,
    refresh_hz: 36,
    mouse_mode: MouseMode::SoftwareRelative {
        virtual_span: (640, 480),
        start_pos: (320, 240),
    },
    escape_quits: false,
    skip_keys: &["Space"],
    cadence: AgentCadence::PauseToThink {
        settle_ms: 800,
        settle_flips: 16,
    },
    controls: &[
        ControlHint {
            input: "Left click",
            action: "Activate UI / Place / Drag Path / Pick Up Worker",
            notes: "UI icons fire on button release after hover latches. Places selected rides/scenery/power, drag-paints paths/roads/fences, erases in Erase mode, picks up/drops Mechanics and Gardeners, and opens `PopUpInfo` in Normal mode.",
        },
        ControlHint {
            input: "Right click",
            action: "Cancel Build / Erase / Exit Zoom",
            notes: "Cancels active Build or Erase mode back to the Normal/Query cursor (`[0x8119b0] = 0`) and exits `LoadZoomer`.",
        },
        ControlHint {
            input: "Middle click",
            action: "Inspect Tile / Object (`PopUpInfo`)",
            notes: "Opens `PopUpInfo` directly on the hovered tile or attraction.",
        },
        ControlHint {
            input: "Mouse Wheel",
            action: "Scroll Build Tray",
            notes: "Scrolls the active build/category tray (`DIMOUSESTATE.lZ` vs `±60`).",
        },
        ControlHint {
            input: "Left / Right / Up / Down",
            action: "Pan Isometric Park Map",
            notes: "Pans the park camera (`±8192` X, `±4096` Y) when no popup is open. No camera rotation or continuous zoom exists.",
        },
        ControlHint {
            input: "Space",
            action: "Skip `Intro.avi`",
            notes: "Skips `Intro.avi` (mouse clicks also skip `Intro.avi`). `lmi.avi` skips only via `LControl+Q` / `RControl+Q`.",
        },
        ControlHint {
            input: "LControl+Q / RControl+Q",
            action: "Skip `lmi.avi`",
            notes: "Skips the opening `lmi.avi` logo movie (`0x4767b7`).",
        },
        ControlHint {
            input: "Escape",
            action: "Close Popup / Drop Worker / Pause Options / Back",
            notes: "Never hard-quits (`escape_quits: false`): closes `PopUpInfo`, drops a held worker, opens Pause/Options in-game, backs out of sub-menus, or opens the Exit Yes/No dialog on Title Screen.",
        },
        ControlHint {
            input: "HUD Control Tools (`640x480`)",
            action: "Path / Query / Erase / Map / Options & Briefing",
            notes: "Bottom action buttons (`54x54`): Path `(39, 445)`, Query `(119, 445)`, Erase `(199, 445)`, Map `(279, 445)`, Options `(359, 445)`. Advisor Phone `(572, 428)`. Title Screen: Player Details `(103, 185)`, Free Play `(233, 82)`, Load `(104, 359)`, Exit `(557, 93)`.",
        },
    ],
    hazards: &[
        "Windows Print Trap (`0x451660`): clicking `Print_On_Certificate.lls` `(519, 14)` calls `EnumPrintersA`/`CreateDCA`/`StartDocA` and attempts to minimize fullscreen DirectDraw. Never click Print.",
        "8 px Screen-Edge Auto-Scroll (`0x4614f0`): hovering within `x < 8`, `x > 631`, `y < 8`, or `y > 471` outside HUD icons pans the camera. Park idle cursor inside `[64..576, 64..370]`.",
        "Release-edge & hover-lock UI: UI icons fire on button release (`test arg, 2`) and swap sprites on hover (`[0x6687d0]`); settle hover (`settle_flips >= 1`) before clicking.",
        "AVI skip asymmetry: `lmi.avi` skips only via `LControl+Q` / `RControl+Q`, while `Intro.avi` skips on `Space` or mouse click.",
        "Appraisal failure (`APPRAISAL 10`): unlinked paths, unpowered rides, or unmet Inspector food/scenery/visitor targets cause repeated failed appraisals and close the park (`Lose.txt`).",
    ],
    goals: &[
        Goal {
            id: "boot-main-menu",
            name: "Boot to Title Screen",
            milestone: Milestone::FirstLight,
            description: "Clear `lmi.avi` and `Intro.avi`, create/select a profile on `Player Details`, click `Accept` `(562, 402)`, and reach the `Title Screen` with `Start Game` unlocked.",
        },
        Goal {
            id: "tutorial-1-first-park",
            name: "Clear Tutorial 1",
            milestone: Milestone::Playable,
            description: "Enter Tutorial Lesson 1 (`ObjList1.txt`), place `SPACE TOWER RIDE`, drag-link `LEGO SHOP 1` to the entrance path, demolish `LEGO SHOP 1` with Erase, and complete the lesson.",
        },
        Goal {
            id: "level-1-san-francisco",
            name: "Clear Level 1 (San Francisco) & Save/Reload",
            milestone: Milestone::Playable,
            description: "Complete Campaign Level 1 (`ObjList6.txt`, Money Master) across LEGOLAND and Western themes, save to `profiles\\%dsave%d.sav`, and verify cold-restart reload.",
        },
        Goal {
            id: "free-play-sandbox",
            name: "Free Play Sandbox",
            milestone: Milestone::Playable,
            description: "Enter Free Play `(233, 82)`, verify all 4 theme tabs populate, place a ride, path, and power station, and exit cleanly.",
        },
        Goal {
            id: "all-10-minilands",
            name: "Complete All 10 Miniland Levels (-> Credits)",
            milestone: Milestone::Credits,
            description: "Clear Campaign Levels 1-10 (`ObjList6.txt`-`ObjList15.txt`) to reach the Credits and Certificate screen without clicking Print.",
        },
    ],
    save_files: &["profiles\\"],
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{format_controls_json, format_goals_json, format_meta_json};

    #[test]
    fn title_guide_serializes_valid_json_and_declares_initial_metadata() {
        assert!(!GUIDE.is_empty());
        const { assert!(!GUIDE.escape_quits) };
        assert_eq!(GUIDE.refresh_hz, 36);

        let meta = parse_json(&format_meta_json(&LEGOLAND, &GUIDE)).expect("valid meta JSON");
        let meta_obj = meta.as_object().unwrap();
        assert_eq!(meta_obj["ok"].as_bool(), Some(true));
        assert_eq!(meta_obj["title"].as_str(), Some("legoland"));
        let mm = meta_obj["guide"].as_object().unwrap()["mouse_mode"]
            .as_object()
            .unwrap();
        assert_eq!(mm["kind"].as_str(), Some("software_relative"));
        assert_eq!(mm["recenter_every_flip"].as_bool(), Some(false));
        assert_eq!(mm["start_pos"].as_array().unwrap()[0].as_i64(), Some(320));
        assert_eq!(mm["start_pos"].as_array().unwrap()[1].as_i64(), Some(240));

        let controls =
            parse_json(&format_controls_json(&LEGOLAND, &GUIDE)).expect("valid controls JSON");
        let ctrl_obj = controls.as_object().unwrap();
        assert_eq!(ctrl_obj["ok"].as_bool(), Some(true));
        assert_eq!(ctrl_obj["escape_quits"].as_bool(), Some(false));
        assert!(!ctrl_obj["controls"].as_array().unwrap().is_empty());
        assert!(!ctrl_obj["hazards"].as_array().unwrap().is_empty());

        let goals = parse_json(&format_goals_json(&LEGOLAND, &GUIDE)).expect("valid goals JSON");
        let goals_obj = goals.as_object().unwrap();
        assert_eq!(goals_obj["ok"].as_bool(), Some(true));
        assert_eq!(goals_obj["goals"].as_array().unwrap().len(), 5);
        assert_eq!(
            goals_obj["save_files"].as_array().unwrap()[0].as_str(),
            Some("profiles\\")
        );
    }
}
