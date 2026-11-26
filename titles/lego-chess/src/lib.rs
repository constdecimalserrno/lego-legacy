//! LEGO Chess: the roster's fifth title (#142).

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::LEGO_CHESS as ENTRY;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, Goal, Installed, Kept, Milestone, MouseMode, TitleGuide,
    WINDOWS_98_SE,
};
use lego_legacy_core::Title;

/// The archive the roster keeps the game in, which the setup expands.
const ARCHIVE: &str = match ENTRY.image.kept {
    Kept::Imploded(archive) => archive,
    _ => panic!("LEGO Chess keeps its game imploded in an archive"),
};

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_CHESS: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise. This title authors no frame
    // rate: no wait for a vertical blank was found in its code, and it moves
    // everything by the clock, through `timeGetSystemTime` and its own
    // millisecond timer. So it is the roster's baseline sixty.
    flip_cost: FlipCost::Refresh(60),
    image: ENTRY.image,
    red_book: ENTRY.red_book,
    declared: Declared {
        // The roster's baseline. All this title asks of its Windows is
        // whether it is NT, through `GetVersionExA`: on NT it would skip its
        // DirectX plugin and read keys with `GetAsyncKeyState`.
        windows: WINDOWS_98_SE,
        // A Pentium 166: what this title asks for, in the "Minimum
        // Specifications" of its original pressing's 1998 UK manual,
        // IB2G-CHE3, page 5 ("CPU Pentium 166 MHz"; archive.org item
        // `LEGOChess-manual-EU-IB2G-CHE3`), and on the backs of its later
        // boxes. The Masterpiece's own manual and the 2002 US edition have no
        // requirements page. It matters here as it
        // does for Hot Wheels: this title waits by spinning on the clock,
        // half a second and then a second at boot (0x00408ABA, 0x00408AE3)
        // and between its movies' frames, so the declared processor decides
        // how much work a second of waiting costs. Measured on a probe that
        // skipped the movies (#147): the two waits are 675 million
        // instructions at the baseline 450 MHz, nine tenths of the boot, and
        // 249 million at 166. A committed run first reaches them with #151.
        hz: 166_000_000,
        // And that Pentium is told about as itself, family 5. It never asks
        // `cpuid`, and its two `rdtsc`s (0x42E1CE, 0x42E1E7) are in one
        // helper, 0x42E1C0, that nothing calls.
        processor: Processor::Pentium,
        // The roster's baseline, and never asked: the game imports no
        // `GlobalMemoryStatus`.
        megabytes: 128,
        // 640x480 at sixteen bits: the mode the game sets unless its own
        // configuration says otherwise, and nothing does on first run.
        modes: &[(640, 480, 16)],
        // Its disc's long names, as the declared CD answers them from its
        // Joliet tree (#144): after its intro the game opens `D:\intro\Chess Bumper01.bmp`, a name only
        // the Joliet tree has, the primary tree calling it `CHESSB~1.BMP`
        // (#144). By #142's rule Joliet is this title's alone until the
        // maintainer decides for every title.
        joliet: true,
        computer: "LEGO-LEGACY",
        // The two libraries its renderer list asks for and this machine has
        // not got, each told it is not there (NULL, 1157 on Windows 9x;
        // #146), so the game passes over them and its own auto-select picks
        // its DirectX plugin. Anything else it asks for that is neither
        // shipped nor written stops the run at a wall.
        absent_libraries: &[
            (
                "glide2x.dll",
                "3dfx's Glide, which its Glide plugin loads, and the declared machine has no \
                 3dfx card",
            ),
            (
                "KLIB_OpenGL_PlugIn.dll",
                "the OpenGL renderer its plugin table names, which neither pressing ever \
                 shipped",
            ),
        ],
        // A 1998 PC had a floppy drive, and the game's CD check (0x405420)
        // needs one: it tests bit 0 of `GetLogicalDrives`, A:, once and never
        // shifts, so with no A: it looks at no drive and asks for its disc
        // for ever. With one it reaches D: (#145). It asks A: only its type.
        floppy: true,
        why: "the roster's baseline, but for what the game itself needs: its disc's long names \
              read through Joliet, the two renderer libraries it asks for and no 1998 machine \
              had, and a floppy drive, without which its CD check looks at no drive. Of its \
              Windows it asks only whether it is NT. Its processor is the Pentium 166 its own \
              manual asks for, since it waits by spinning on the clock",
    },
    // Nothing of the game is loose on its disc. Its setup expands MAIN.Z, an
    // InstallShield 3 archive, into the install directory whole, and copies
    // `Text.txt`, the game's own strings, out of its own library beside it;
    // without it the New Player form faults (#143). A Minimum install (#142):
    // the three `.RES` archives, 170 MB, stay on the disc, and the game reads
    // them from the drive it finds its disc in.
    installed: Installed::Expanded {
        archive: ARCHIVE,
        beside: &[("_SETUP.LIB", "Text.txt")],
    },
    // The game reads nothing its setup writes. The one key it opens is its
    // own `HKCU\Software\LEGO Chess`, for its video settings, and it writes
    // that itself.
    registry: &[],
    // Where its setup puts it: `PROGRAMFILES` and `LEGO Media\Games\LEGO
    // Chess`, read out of its script, `SETUP.INS`.
    install_dir: "C:\\Program Files\\LEGO Media\\Games\\LEGO Chess",
    // Not yet measured: no movie of this title's own has played on this
    // machine.
    click_in_a_movie: None,
    // One for one, not yet played at a window. The game hides the Windows
    // pointer (`ShowCursor(FALSE)`, 0x40ADA0) and draws its own, so the host
    // steers it by how far the mouse goes and this is the value that steers
    // it. It draws it at the place each `WM_MOUSEMOVE` carries, kept whole
    // (0x457830 stores its x and y), so a move should be drawn as far as it
    // went, as Hot Wheels' is (#37). It puts the pointer itself only for Tab
    // in a game (0x40F5F0), stepping it through six of its buttons with
    // `SetCursorPos` (0x457810, from 0x4113C0), and reads it back only from
    // `WM_MOUSEMOVE`, which core does not post at a `SetCursorPos` (#157, #235).
    pointer_speed: (1.0, 1.0),
};

/// Static playability metadata served via `/api/meta`, `/api/controls`, and
/// `/api/goals` (#311).
pub const GUIDE: TitleGuide = TitleGuide {
    display_name: "LEGO Chess",
    summary: "3D themed LEGO chess (Western, Pirates, Classic Staunton) with 3D capture cartoons, rotatable board camera, 2D/3D view toggle, tutorials, and Oxford Softworks AI.",
    resolution: (640, 480),
    bpp: 16,
    refresh_hz: 60,
    mouse_mode: MouseMode::SoftwareAbsolute {
        requires_poll_release: true,
    },
    escape_quits: false,
    skip_keys: &["Space"],
    cadence: AgentCadence::PauseToThink {
        settle_ms: 1000,
        settle_flips: 20,
    },
    controls: &[
        ControlHint {
            input: "click_poll (--poll --hold-ms 150)",
            action: "Click UI buttons, green ticks, menus, HUD icons, dialogs & Throne Room Chess Table",
            notes: "Required on UI controls and the Throne Room Chess Table (385,285) because button release loops spin without presenting frames (standard click hangs in the poll-release loop and spins +155s declared_ms). Do NOT use on 3D chessboard squares.",
        },
        ControlHint {
            input: "click --hold-flips 3..6",
            action: "Select & move pieces on 3D chessboard squares",
            notes: "Standard flip-held click (NOT click_poll). On default 3D camera, rank centers compress upward: Rank 1 y=349, Rank 2 y=308, Rank 3 y=272, Rank 4 y=241, Rank 5 y=214, Rank 7 y=180.",
        },
        ControlHint {
            input: "click_poll 320 60 -> HUD arrows or LeftShift+Arrow",
            action: "Rotate & tilt 3D board camera (click 320 60 again to reset)",
            notes: "Expand Rotate/Tilt HUD at (320,60), then hold standard click (15..30 flips) on arrows Left (282,60), Right (358,60), Up (320,24), Down (320,96), or hold chord LeftShift+Left/Right/Up/Down while expanded.",
        },
        ControlHint {
            input: "click_poll 440 60 (3D->2D) / click_poll 585 200 (2D->3D)",
            action: "Toggle 2D overhead board & 3D board view",
            notes: "2D view shows an exact uniform 8x8 grid (x=144..496, y=24..344; 44x40 px squares) and moves ! Options to (48,200) and the 3D Knight toggle to (585,200).",
        },
        ControlHint {
            input: "Space (hold_ms 150 poll release)",
            action: "Skip intro, Story cutscenes & 3D capture cartoons",
            notes: "Release Space between flips (`--hold-flips 3 --hold-ms 150`) so the post-movie key-up wait (`0x0040D220`) does not stall. Opening LEGO Media logo (flips 0..283) cannot be skipped.",
        },
        ControlHint {
            input: "type_text (--flip-gap 4)",
            action: "Type player profile & save slot names",
            notes: "Text boxes require >= 50 ms declared time (4+ flips at 60 Hz) and key-up between characters (`WM_CHAR` debounce). Clear default `....` save labels with 4x Backspace.",
        },
        ControlHint {
            input: "Escape (hold_ms 150 poll release)",
            action: "Open Resign dialog mid-match / menu back",
            notes: "Opens \"Are you sure you want to resign?\" (confirm green tick at (360,232) via click_poll) or backs out of menus; never quits the process.",
        },
        ControlHint {
            input: "F1 (hold_ms 150)",
            action: "Open in-game Settings menu",
            notes: "Toggle Show cartoons, Instant move, Default board view, and Show notation mid-match (Detail Level, Show Reflections, and timers are locked mid-match).",
        },
        ControlHint {
            input: "Arrows + Return / Tab / Delete",
            action: "Board square navigation / HUD button cycle / profile delete",
            notes: "Arrows move the board highlight and Return selects/places pieces (cannot open menus per README.TXT). Tab steps 6 HUD buttons via SetCursorPos (software cursor lags until WM_MOUSEMOVE, #235). Delete removes highlighted profile on register.",
        },
        ControlHint {
            input: "Throne Room & Match HUD hotspots",
            action: "Navigate hub, Quick Game setup, Advisor Help & ! Options",
            notes: "Throne Room (move + wait 25 flips for camera pan): Chess Table (385,285, click_poll --hold-ms 150)=Quick Game, Curtains=Story, Book=Tutorial, Crystal Ball=Settings, Trapdoor=Quit, Scrapbook/Scissors=Scrapbook, Chest=Load, Quill/Inkpot=Switch Player. Quick Game: Theme (320,70) cycles Western->Pirates->Classic, Black AI/Human (520,180), Start (590,410). Match 3D HUD: White Advisor Help (60,75) [Pointer/Moves/Hint/Undo], ! Options (200,60) [Save (282,258), Load (360,258), confirm (392,194)].",
        },
    ],
    hazards: &[
        "UI buttons, menus, dialogs, the Throne Room Chess Table (385,285), and Space skips require poll-release (`click_poll` / `--hold-ms 150`) because release loops spin without flipping (standard click on the Chess Table hangs and spins +155s declared_ms); 3D chessboard squares require standard `click --hold-flips 3..6` (NOT `click_poll`).",
        "Never send `close` mid-match (the game ignores `WM_QUIT` and hits the 5 s close grace); resign via Escape -> green tick (360,232) or finish via checkmate green tick (320,232) to return to Quick Game / Throne Room first.",
        "Default 3D camera foreshortens ranks sharply (Rank 1 y=349, Rank 2 y=308, Rank 3 y=272, Rank 4 y=241, Rank 5 y=214, Rank 7 y=180); toggle 2D view at (440,60) for uniform 44x40 px squares (return to 3D at (585,200)).",
        "Throne Room camera tracks the mouse pointer — move to an object, wait ~25 flips for the pan to settle, observe, then click.",
        "Text boxes enforce a >= 50 ms declared-time `WM_CHAR` debounce with key-up between characters (`type --flip-gap 4`); clear default `....` save labels with 4x Backspace before typing.",
        "3D capture cartoons require `Show cartoons` enabled (`F1` -> Graphics) and matched Western or Pirates piece sets (mixing sets or Classic Staunton suppresses cartoons).",
    ],
    goals: &[
        Goal {
            id: "profile-and-throne-room",
            name: "Create profile & enter Throne Room",
            milestone: Milestone::Playable,
            description: "Skip Krisalis' logo and intro with Space (poll-release), create a player profile on the New Player form (`type --flip-gap 4`), confirm (576,420) twice, and enter the Throne Room.",
        },
        Goal {
            id: "quick-game-checkmate",
            name: "Play 3D Quick Game vs AI with capture cartoons to checkmate",
            milestone: Milestone::Playable,
            description: "Click the Throne Room Chess Table (385,285) with click_poll (--hold-ms 150) -> Start (590,410), play legal 3D chess moves (`click --hold-flips 4`), trigger a 3D capture cartoon, and win by checkmate.",
        },
        Goal {
            id: "camera-and-view-inspection",
            name: "Orbit/tilt 3D camera & toggle 2D/3D view",
            milestone: Milestone::Playable,
            description: "Expand Rotate/Tilt at (320,60), rotate and tilt the 3D board via its HUD arrows or LeftShift+Arrow, snap back via (320,60), toggle 2D overhead view at (440,60), and return to 3D at (585,200).",
        },
        Goal {
            id: "western-and-pirates-ai-checkmate",
            name: "Checkmate Computer AI in Western & Pirates themes",
            milestone: Milestone::Complete,
            description: "Win by checkmate against the Computer AI in both Western (default) and Pirates (1 click on Theme (320,70)) with matched sets and capture animations enabled.",
        },
        Goal {
            id: "save-and-reload-match",
            name: "Save & reload match across runs",
            milestone: Milestone::Complete,
            description: "Save an in-progress match via ! Options (200,60) -> Save Game (282,258) (`profiles\\1save.lcs`), resign cleanly, relaunch over the same `--data-root`, reload via Load Game (360,258), and continue playing.",
        },
    ],
    save_files: &["profiles\\prof1.lcp", "profiles\\1save.lcs"],
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
        assert_eq!(
            GUIDE.mouse_mode,
            MouseMode::SoftwareAbsolute {
                requires_poll_release: true
            }
        );
        assert!(
            GUIDE.controls.iter().any(|c| {
                c.input.contains("click_poll")
                    && c.notes.contains("Do NOT use on 3D")
                    && c.notes.contains("385,285")
            }),
            "click_poll hint must cover UI and Throne Room Chess Table (385,285) and exclude 3D squares (#351)"
        );
        assert!(
            GUIDE.controls.iter().any(|c| {
                c.input.contains("hold-flips 3..6")
                    && c.action.contains("3D chessboard")
                    && !c.action.contains("Throne Room")
            }),
            "standard flip-held click hint must cover 3D chessboard squares without Throne Room (#351)"
        );
        assert!(GUIDE
            .controls
            .iter()
            .any(|c| c.input.contains("320 60") && c.input.contains("LeftShift+Arrow")));
        assert!(GUIDE
            .controls
            .iter()
            .any(|c| c.input.contains("440 60") && c.input.contains("585 200")));
        assert!(GUIDE.goals.len() >= 5);
        for body in [
            format_meta_json(&LEGO_CHESS, &GUIDE),
            format_controls_json(&LEGO_CHESS, &GUIDE),
            format_goals_json(&LEGO_CHESS, &GUIDE),
        ] {
            let v = parse_json(&body).expect("valid JSON");
            assert_eq!(v.as_object().unwrap()["ok"].as_bool(), Some(true));
        }
    }
}
