//! LEGO Alpha Team: the roster's third title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, EntryImage, Goal, Installed, Kept, Milestone, MouseMode,
    TitleGuide, WINDOWS_98_SE,
};
use lego_legacy_core::Title;

/// What this title declares about itself and the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_ALPHA_TEAM: Title = Title {
    key: "lego-alpha-team",
    name: "LEGO Alpha Team",
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, as DirectDraw's
    // Flip does unless a title asks otherwise — and this title does not ask:
    // it calls `SetDisplayMode(640, 480, 16, 0, 0)`, a zero refresh being
    // "whatever the machine runs at", and over a whole run into its first
    // mission it never calls `GetMonitorFrequency`, `GetScanLine`,
    // `WaitForVerticalBlank` or `GetVerticalBlankStatus`, and flips with no
    // flags at all (#82).
    //
    // So the rate is a declared value like the processor and the memory, and
    // the title's own code names it: `DDCaps::MakeRecs` in LoadComp writes
    // 0x3d088889 — a thirtieth of a second — into the field
    // `DDCaps::GetRecFrameRate` hands out, on every run and under every
    // branch. That is CONTEXT.md's **intended rate**: the rate this title
    // authored as its target. Thirty, not the roster's baseline sixty.
    //
    // Measured in a window, over the same forty seconds of the committed menu
    // route on the same host: at sixty the pace is 0.52x and the picture is
    // 31 frames a second; at thirty the pace is **0.86x** and the picture is
    // 26. The second is a thirty-frame game dropping a few frames; the first
    // is a sixty-frame game running at half speed, which is what #65's
    // reporter heard and saw.
    flip_cost: FlipCost::Refresh(30),
    image: EntryImage {
        disc_dir: "alpha-team",
        pressing: "alpha-team",
        // A name in the pressing's cabinet, not a path on its disc: this title
        // ships nothing of itself loose (#53).
        path: "LEGOATeam.exe",
        kept: Kept::Cabinet,
        sha256: "36f77aa72b9345af2667f8e2c2395318bdbc9ac7649d378684a6bb0179eecc01",
        why: "both of this title's pressings carry the same cabinet byte for byte, so the \
              original is taken and the Masterpiece re-release buys nothing but an EA \
              registration tree, a PDF manual and menu fonts the game never reads (ADR-0006)",
    },
    // Neither pressing has an audio track: this title's music is in Audio.pac
    // and Ambient.pac, inside its own cabinet.
    red_book: None,
    bank: &[],
    declared: Declared {
        windows: WINDOWS_98_SE,
        // The roster's baseline Pentium II 450. This title does measure the
        // machine itself, in `DDCaps::CalcProcSpeed`, and `DDCaps::MakeRecs`
        // does pick what it would draw from the number it gets — shadows and
        // space animation above 350 MHz, texture animation above 250 — but
        // **nothing reads what it picks** (#81). The six recommendations land
        // in `DDCaps`' own fields, `GetRecShadows` and its five siblings hand
        // them out, and those getters have no call site in the only binary
        // that has them and are imported by nothing else the disc ships.
        //
        // Measured rather than argued: the menu route at 450, 350, 300 and
        // 233 MHz, and the first-mission route at 450 and 233, put the same
        // picture on the screen at the same flip — the same lighting, the
        // same textures, the same models. Only the animation's phase differs,
        // because a slower declared processor means more declared time passes
        // in a frame. The pace does not move either: 0.87x at 233 MHz against
        // 0.86x at 450, because the declared display's refresh is what a flip
        // costs the clock (#82) and the instructions are the small part of it.
        //
        // So this value is the roster's baseline because it is the roster's
        // baseline, and for no reason of this title's own. There is no quality
        // to be bought by declaring less, and none to be lost.
        hz: lego_legacy_core::clock::DECLARED_HZ,
        // And it is told it is that Pentium II. `NiMain` asks `cpuid` once at
        // boot, with no ID flag test first, for the extended leaves and then
        // leaf 1, and sorts the machine into a class by MMX and conditional
        // moves (#104). The class picks its transform routines, and every
        // class but MMX with the streaming extensions picks the same ones;
        // this machine is not told it has MMX.
        processor: Processor::PentiumII,
        megabytes: 128,
        modes: &[(640, 480, 16)],
        // The primary tree's names alone, by #142's rule: Joliet is LEGO
        // Chess's alone, because this title's runs move with it, until the
        // maintainer decides (#144). Read through its Joliet tree, as Windows
        // 98 SE's CDFS reads it, this disc also answers `D:\Strings_EN.txt`,
        // which the primary tree calls `STR_0012.TXT` and the title's runtime
        // opens at boot, and every route moves, and every boot run past
        // 2,000,000 instructions (measured at 55a3105; #138 moves none of
        // this title's runs).
        joliet: false,
        computer: "LEGO-LEGACY",
        // None: in every gate run it asks only for libraries it ships and the
        // two of Microsoft's core writes, ADVAPI32 and KERNEL32 (#146).
        absent_libraries: &[],
        // None, as every released title's machine has had since its routes were
        // recorded: C: and D: only.
        floppy: false,
        why: "Windows 98 SE is what this pressing shipped against, and the rest is the \
              roster's baseline; the processor is the one value this title reads back as a \
              measurement of its own rather than asking for it",
    },
    // Nothing of this title is loose on its disc: its executable, its engine
    // DLLs and all eight of its archives are inside the pressing's cabinet
    // (#52), and the declared tree unpacks them from there.
    installed: Installed::Cabinet,
    // What its installer leaves behind. The title's own executable opens this
    // key with `RegOpenKeyExA` and reads it back; the version is what the
    // installer's own script writes there.
    registry: &[(
        "HKLM\\SOFTWARE\\LEGO Media\\LEGO Alpha Team",
        "Version",
        "1.0",
    )],
    // Where this title's installer puts it: the shell folder its own script
    // names, under Program Files.
    install_dir: "C:\\Program Files\\LEGO Media\\LEGO Alpha Team",
    // Escape is the one key this title's intro takes, measured (#66): a
    // click and Space are ignored outright, Escape lets go of the movie from
    // about flip 440 on. A person who clicks to get past the intro meant
    // that key, so the machine hands it over.
    click_in_a_movie: Some(0x01),
    // Twice across and eight thirds down, because this title's own pointer
    // moves by a **1280th of the screen per pixel it is handed, on both
    // axes**, and a 1280th of the screen is half a pixel across and three
    // eighths of one down. Left at one for one, a person's hand and the
    // pointer they are watching part company immediately: the pointer covers
    // half the ground across, less than that down, so a straight diagonal
    // comes out at 37 degrees rather than 45, and every push into the edge of
    // the screen throws away a different amount of the correspondence. That
    // is what a person means by the cursor not matching the mouse (#86).
    //
    // Measured twice, two ways, on the first mission's own screen rather than
    // argued from the disassembly — which reads a 640th and is wrong by two:
    //
    //   - the drawn pointer, out of screenshots: 100 delivered pixels across
    //     move it 50, 20 move 10, 280 move 140; 100 down move it 37 (37.5,
    //     floored), 200 down move 75. Twenty deliveries of 1 pixel move it 10
    //     across and 7 down, so the title carries the fraction and loses
    //     nothing;
    //   - the title's own pointer, read out of guest memory at
    //     `DDMouse::xPositionNDC`: a delivered 100 across moves it 0.078125,
    //     which is 100/1280 exactly, and a delivered 100 down moves it the
    //     same. `DDUIManager::m_MouseX` is that same number, so what is drawn
    //     and what is hit-tested share it and there is nothing else to scale.
    //
    // 1280/640 and 1280/480. Its `DDMouse::sensitivity` is never called by
    // anything the disc ships (`SetSensitivity` has no call site in any of the
    // twelve binaries), and this title reads no Windows mouse setting at all —
    // no `SystemParametersInfo`, no mouse key in the registry — so there is
    // nothing to answer differently and what is handed over is the only lever.
    pointer_speed: (1280.0 / 640.0, 1280.0 / 480.0),
    // Static playability metadata served via `/api/meta`, `/api/controls`, and
    // `/api/goals` (#311, #337).
    guide: TitleGuide {
        summary: "Isometric 3D puzzle-action game: place and orient Action Bricks, connect belts/cords/ropes, and press GO so specialists clear Evil Orbs, rescue teammates, and unlock each room's exit.",
        resolution: (640, 480),
        bpp: 16,
        refresh_hz: 30,
        mouse_mode: MouseMode::RelativeSteered {
            virtual_span: (1280, 1280),
            recenter: (320, 240),
            start_corner: (0, 480),
        },
        escape_quits: false,
        skip_keys: &["Escape", "Space"],
        cadence: AgentCadence::PauseToThink {
            settle_ms: 1000,
            settle_flips: 30,
        },
        controls: &[
            ControlHint {
                input: "mouse_delta / move",
                action: "Steer software cursor",
                notes: "`DDMouse` re-centers Win32 cursor to `(320, 240)` every flip on a `1280x1280` virtual span (`2.0x` X, `8/3x` Y); cursor starts at `(0, 480)` bottom-left (`(0, 479)` in `status.pointer_pos`). `ControlHost` tracks `visual_cursor_pos` independently of per-flip `SetCursorPos(320, 240)`, auto-chunks large `mouse_delta` moves across recenter flips, and translates `move`/`click`/`drag` targets into scaled relative deltas.",
            },
            ControlHint {
                input: "Left click",
                action: "Click UI / open Unit Menu",
                notes: "Pass target `x`/`y` (auto-translated to multi-flip relative deltas) or omit `x`/`y` to click in place. Clicks menu/HUD buttons; clicking and releasing a movable Action Brick opens its Unit Menu (`RotateCW`, `RotateCCW`, `Info`, `Hint`, plus Belt/Cord/Rope attach or Mirror toggle — wait ~0.5s / ~15 flips to settle before pressing `R` for 90 deg CCW). Clicks during the intro translate to `Escape` (`click_in_a_movie`).",
            },
            ControlHint {
                input: "Left hold + relative drag",
                action: "Move Action Brick",
                notes: "Hold Left on a movable brick (Direction Tile, Jump Tile, Stairs, Slide, Trampoline, Platform, Mirror, See-Saw, Bomb) and pipe small relative moves from `(320, 240)` using verified scaling `0.5 px/raw_x`, `0.375 px/raw_y` (carry Action Bricks at `<= 8 screen px/flip` = `16 raw px/flip`) onto a valid pad, then release.",
            },
            ControlHint {
                input: "Right hold + relative drag",
                action: "Orbit 3D camera",
                notes: "Hold Right button (`down right`) and sweep with relative moves to orbit the mission camera around the puzzle space.",
            },
            ControlHint {
                input: "Screen edges (<= 16 px X / <= 12 px Y)",
                action: "Edge-scroll camera",
                notes: "Moving the software cursor within 1/40th of any screen edge pans the mission camera in that direction.",
            },
            ControlHint {
                input: "HUD bar (y=428)",
                action: "Menu / Camera / Specialists / GO-Stop",
                notes: "Planning: Menu `(61,430)` (required in `M0010` where `Escape` is disabled), `PrevCam (215)`, `Nav_InOut (283)`, `Nav_Pivot (348)`, `NextCam (412)`, `GO/Stop (573)`. Running: follow-cams Flex `(170)`, Cam `(220)`, Charge `(270)`, Radia `(320)`, Crunch `(370)`, Dash `(419)`, `ICam (470)`, `SmartCam (534,433)`, `Stop (573)`.",
            },
            ControlHint {
                input: "Space",
                action: "Go / Stop & attract screen",
                notes: "Toggles simulation execution (`GO`) and resets agents/props back to setup (`Stop`); also advances past the post-intro attract screen. Ignored during the intro movie.",
            },
            ControlHint {
                input: "Tab / Shift+Tab",
                action: "NextCam / PrevCam",
                notes: "Cycles mission camera presets forward (`Tab`) or backward (`Shift+Tab`).",
            },
            ControlHint {
                input: "R",
                action: "RotateCCW (90 deg CCW)",
                notes: "Rotates the selected Action Brick 90 degrees counter-clockwise (CCW, not CW). Wait ~0.5s (~15 flips) settle after clicking a tile to open its Unit Menu before pressing `R`.",
            },
            ControlHint {
                input: "Z / Q / W / Up / Down / Left / Right",
                action: "Zoom out / orbit / pan camera",
                notes: "`Z` zooms out cleanly (`PageDown` does not zoom out), `Q`/`W` orbit, and arrow keys pan the 3D mission camera while held (`A`/`S`/`X`/`PageUp` adjust fine camera axes). Camera keys recenter the cursor, so re-verify cursor position before clicking.",
            },
            ControlHint {
                input: "Escape",
                action: "Skip intro / Mission Menu (normal missions)",
                notes: "Skips `LMIINTRO.AVI` from ~flip 440 onward (refused during the opening logo); opens/closes the Mission Menu (`Keep Playing`, `Start Over`, `Return to HQ`) in normal missions (`M1010`/`M1020`) or cancels active selection. Disabled during `M0010` Training Area (click Menu button at `(61, 430)` instead). Never quits.",
            },
        ],
        hazards: &[
            "Raw piped `#lego-legacy-route 1` `move` events are interpreted as relative deltas from `(320, 240)` because `DDMouse` re-centers every flip (`0.5 px/raw_x`, `0.375 px/raw_y`); via the control API (`move`/`click`/`drag`/`mouse_delta`), `ControlHost` tracks `visual_cursor_pos` from `(0, 479)` and chunks large deltas automatically (#327, #332).",
            "Leaving the software cursor within `16 px` of left/right or `12 px` of top/bottom edges continuously scrolls the mission camera.",
            "Camera keys (`Z` zoom out, `Q`/`W` orbit, arrow keys pan) internally warp `DDMouse` to `(320, 192)` in-mission without calling Win32 `SetCursorPos`; `PageDown` does not zoom out (use `Z`). Re-verify cursor position visually before clicking after using keyboard camera controls.",
            "Intended presentation rate is 30 Hz (33.3 ms per flip), not 60 Hz; size waits in 30-flip (1 s) multiples. No built-in fast-forward key exists; use control-host pace instead.",
            "Intro movie (`LMIINTRO.AVI`) refuses all keys and clicks before ~flip 440; `Space` is ignored for the entire intro (use `Escape` or click from ~flip 440).",
            "In Training Area (`M0010`), `Escape` is disabled (click Menu button `(61, 430)` instead; `Escape` works in normal missions `M1010`/`M1020`), and Tee Vee (`T0001`) locks picking to only the flashing step target after each spoken line finishes whole (Direction Tile `U0179` unlocks at step `220a`).",
            "Keep display Options at `640x480` 16-bit; raising resolution or switching to 32-bit BPP breaks display output (`Game.txt` stores the override).",
        ],
        goals: &[
            Goal {
                id: "boot-main-menu",
                name: "Boot to Main Menu",
                milestone: Milestone::FirstLight,
                description: "Boot `lego-alpha-team`, skip `LMIINTRO.AVI` from ~flip 440 (or wait to flip 1631), clear the attract screen with `Space`, and reach Main Menu (`MM.PUZ`).",
            },
            Goal {
                id: "training-area",
                name: "Training Area (M0010)",
                milestone: Milestone::Playable,
                description: "Complete `M0010` following Tee Vee's steps: cycle cameras, drag Direction Tile `U0179` onto the blue pad, rotate via Unit Menu, press `GO` so Dash reaches the console, and return to HQ.",
            },
            Goal {
                id: "adventure-m1010",
                name: "Adventure Mission 1: Mission One (M1010)",
                milestone: Milestone::Playable,
                description: "Open Select Adventure slot 1 (skip or finish training), place and orient Direction Tiles to guide Dash to the Plunger Lock in `M1010`, press `GO`, and clear the mission.",
            },
            Goal {
                id: "adventure-m1020-compound-1",
                name: "Mission 2+ & Ogel Island (M1020-M1140)",
                milestone: Milestone::Complete,
                description: "Complete Mission 2 (`M1020`, On the Right Path) with 4 Direction Tiles and progress through Ogel Island (`M1010`-`M1140`), rescuing Crunch (`M1070`) and Radia (`M1140`).",
            },
            Goal {
                id: "full-adventure",
                name: "Stop the Boggle Rocket (M4070 -> Credits)",
                milestone: Milestone::Credits,
                description: "Clear Goo Caverns (`M2010`-`M2160`, rescue Charge & Cam), Deep Sea Orb Factory (`M3010`-`M3100`, rescue Flex & shut off D.O.O.M.), and Arctic Command Base (`M4010`-`M4070`) to reach Credits.",
            },
        ],
        save_files: &["C:\\Program Files\\LEGO Media\\LEGO Alpha Team\\Game.txt"],
    },
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{
        format_controls_json, format_goals_json, format_meta_json, Action, Command, ControlConfig,
        ControlHost, ExecMode, InputOwner,
    };
    use lego_legacy_core::host::headless::Headless;
    use lego_legacy_core::host::{Host, Input};

    #[test]
    fn title_guide_serializes_valid_json_and_auto_scales_relative_steered() {
        let guide = &LEGO_ALPHA_TEAM.guide;
        assert!(!guide.is_empty());
        const { assert!(!LEGO_ALPHA_TEAM.guide.escape_quits) };
        assert_eq!(guide.refresh_hz, 30);
        let (sx, sy) = guide.relative_steered_scale().expect("steered scale");
        assert!((sx - 2.0).abs() < 1e-12);
        assert!((sy - (1280.0 / 480.0)).abs() < 1e-12);
        assert!(guide.controls.len() >= 10);
        assert!(guide.hazards.len() >= 5);
        assert!(guide.goals.len() >= 4);

        let r_hint = guide
            .controls
            .iter()
            .find(|c| c.input == "R")
            .expect("R hotkey hint");
        assert!(
            r_hint.action.contains("CCW")
                && !r_hint.action.contains("RotateCW")
                && r_hint.notes.contains("0.5s"),
            "R hotkey must document 90 deg CCW and ~0.5s Unit Menu settle (#351)"
        );

        let cam_hint = guide
            .controls
            .iter()
            .find(|c| c.input.contains("Z") && c.input.contains("Q"))
            .expect("camera key hint");
        assert!(
            cam_hint.notes.contains("PageDown` does not")
                && cam_hint.notes.contains("recenter"),
            "camera key hint must note Z zooms out (PageDown does not) and camera keys recenter cursor (#351)"
        );

        let esc_hint = guide
            .controls
            .iter()
            .find(|c| c.input == "Escape")
            .expect("Escape hint");
        assert!(
            esc_hint.notes.contains("M0010") && esc_hint.notes.contains("(61, 430)"),
            "Escape hint must note M0010 disables Escape and requires Menu button (61, 430) (#351)"
        );

        let drag_hint = guide
            .controls
            .iter()
            .find(|c| c.input.contains("relative drag") && c.action.contains("Action Brick"))
            .expect("Action Brick relative drag hint");
        assert!(
            drag_hint.notes.contains("0.5 px/raw_x")
                && drag_hint.notes.contains("0.375 px/raw_y")
                && drag_hint.notes.contains("<= 8 screen px/flip")
                && drag_hint.notes.contains("16 raw px/flip"),
            "relative drag hint must document verified scaling and speed cap (#351)"
        );

        for body in [
            format_meta_json(&LEGO_ALPHA_TEAM),
            format_controls_json(&LEGO_ALPHA_TEAM),
            format_goals_json(&LEGO_ALPHA_TEAM),
        ] {
            let v = parse_json(&body).expect("valid JSON");
            assert_eq!(v.as_object().unwrap()["ok"].as_bool(), Some(true));
        }

        let goals_json = parse_json(&format_goals_json(&LEGO_ALPHA_TEAM)).unwrap();
        let goal_ids: Vec<&str> = goals_json.as_object().unwrap()["goals"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|g| g.as_object()?.get("id")?.as_str())
            .collect();
        assert!(goal_ids.contains(&"boot-main-menu"));
        assert!(goal_ids.contains(&"training-area"));
        assert!(goal_ids.contains(&"adventure-m1010"));
        assert!(goal_ids.contains(&"adventure-m1020-compound-1"));

        let mut host = ControlHost::new(
            Headless::new(),
            ControlConfig {
                title: &LEGO_ALPHA_TEAM,
                mode: ExecMode::Live,
                owner: InputOwner::Agent,
                ..ControlConfig::default()
            },
        );
        assert!(
            host.bus()
                .dispatch(Command::Act {
                    action: Action::MouseDelta {
                        dx: 20.0,
                        dy: -15.0,
                        flips: 1,
                        scale_x: None,
                        scale_y: None,
                    },
                    auto_step: false,
                })
                .ok
        );
        assert_eq!(host.poll_input(0), [Input::Move { x: 360, y: 200 }]);
    }
}
