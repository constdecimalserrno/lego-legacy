//! LEGO Rock Raiders: the roster's first title.

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
pub const ROCK_RAIDERS: Title = Title {
    key: "lego-rock-raiders",
    name: "LEGO Rock Raiders",
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, which is what
    // DirectDraw's Flip does unless a title asks otherwise, and this one does
    // not (#9). Sixty a second is what GetMonitorFrequency declares. Its movies
    // pace themselves by waiting for each frame, and every frame they ask for
    // lands on a refresh.
    flip_cost: FlipCost::Refresh(60),
    image: EntryImage {
        disc_dir: "rock-raiders",
        pressing: "masterpiece-rock-raiders",
        // Spelled as the cabinet's header names it, which is the name the
        // installer gives it and the one the title takes its own name, its
        // archives' and its window class's from (#93). The disc spells it in
        // capitals and is read without regard to case, as Windows reads one.
        path: "EXE/LegoRR.exe",
        kept: Kept::Loose,
        sha256: "62b67f5ba656c71412764d343c6a955d218e3b61f0e0f4758ad6086e971e64d4",
        why: "the original pressing's EXE/LEGORR.EXE is a 280 KB SafeDisc wrapper beside a \
              737 KB LEGORR.ICD holding the encrypted game; the Masterpiece pressing ships \
              that same game unwrapped, with an empty .ICD left behind (ADR-0006)",
    },
    // The original pressing carries three audio tracks and the Masterpiece
    // pressing carries none, so the music comes off the disc the executable
    // does not (ADR-0006).
    red_book: Some("rock-raiders"),
    bank: &[],
    declared: Declared {
        windows: WINDOWS_98_SE,
        // The roster's baseline processor, which this title has never been
        // seen to measure or to wait on.
        hz: lego_legacy_core::clock::DECLARED_HZ,
        // Nor to ask what it is: no `cpuid` and no toggled ID flag over any
        // committed route (#104).
        processor: Processor::PentiumII,
        // A Pentium II era machine with 128 MB: what the box asked for, and
        // more than this title ever allocates. The figure only reaches the
        // guest through GlobalMemoryStatus, and a title that sized its caches
        // from the host's memory would size them differently on every host.
        megabytes: 128,
        // The screen this title asks for. It renders at one size and scales
        // nothing itself; the window does the scaling (#6).
        modes: &[(640, 480, 16)],
        // The primary tree's names alone, by #142's rule: Joliet is LEGO
        // Chess's alone, because Alpha Team's runs move with it, until the
        // maintainer decides (#144). Nothing in this title asks for it: read
        // through its Joliet tree, as Windows 98 SE's CDFS reads it, this
        // disc answers the same names but for case, and none of its runs
        // moved with it on (measured at 7509b49).
        joliet: false,
        // Declared rather than the host's own name, for the same reason as
        // everything else here: a recorded run must not depend on the machine
        // it was recorded on.
        computer: "LEGO-LEGACY",
        // None: no gate run asks for a library, and the one place it can,
        // its C runtime's message box, asks for USER32, which core writes (#146).
        absent_libraries: &[],
        // None, as every released title's machine has had since its routes were
        // recorded: C: and D: only.
        floppy: false,
        why: "Windows 98 SE is what this pressing shipped against and what its own installer \
              checks for; the rest is the roster's baseline, unmeasured against this title \
              because it has not asked for any of it yet",
    },
    // This pressing ships its data in an InstallShield cabinet, which the
    // declared tree unpacks from (#16).
    installed: Installed::Cabinet,
    // Its installer writes nothing this title reads back.
    registry: &[],
    // Where this title's installer puts it, spelled the way the guest will
    // read it back from the machine.
    install_dir: "C:\\LEGORR",
    // This title takes a click during a movie itself, measured (#67): it
    // lets go of the movie on the flip the click lands. Nothing to translate.
    click_in_a_movie: None,
    // One for one, and never used: this title's pointer is a place rather
    // than a distance, so the host hands it where the person's own pointer is
    // and there is nothing to steer (#31, #46).
    pointer_speed: (1.0, 1.0),
    // Static playability metadata served via `/api/meta`, `/api/controls`, and
    // `/api/goals` (#311, #336).
    guide: TitleGuide {
        summary: "Real-time underground mining and base-building strategy game: teleport Raiders, drill caverns, clear rubble, build Power Paths and HQ structures, and hit each mission's Energy Crystal or construction objective.",
        resolution: (640, 480),
        bpp: 16,
        refresh_hz: 60,
        mouse_mode: MouseMode::SoftwareAbsolute {
            requires_poll_release: false,
        },
        escape_quits: false,
        skip_keys: &["Escape", "Space"],
        cadence: AgentCadence::BurstThenPause { burst_ms: 500 },
        controls: &[
            ControlHint {
                input: "Left click / Left drag",
                action: "Select / interact / box-select",
                notes: "Select raiders, vehicles, buildings, walls, floor tiles, or HUD icons; drag to box-select Raiders; clicks skip intro movies directly (`Start Game` at `(320, 222)`; un-scrolled Level Select `(155, 160)` is Training Mission 1 `1. Moving Rock Raiders`, while Campaign Mission 1 `Driller Night!` requires scrolling down via `(320, 478)` to center cavern `(315, 180)`).",
            },
            ControlHint {
                input: "Right click",
                action: "Deselect / cancel mode",
                notes: "Clears current selection or cancels active building placement and targeting modes.",
            },
            ControlHint {
                input: "LeftShift / RightShift + click",
                action: "Multi-select units",
                notes: "Hold Shift while left-clicking to add or modify a multi-Raider selection.",
            },
            ControlHint {
                input: "Left / Right",
                action: "Rotate camera (or turn in 1st/2nd person)",
                notes: "Rotates the 3D cavern camera CCW/CW in Top View, or turns the controlled Raider/Vehicle in Eye/Shoulder View.",
            },
            ControlHint {
                input: "Up / Down",
                action: "Tilt camera (or move in 1st/2nd person)",
                notes: "Pitches the 3D cavern camera up/down (0-60 deg) in Top View, or drives forward/backward in Eye/Shoulder View.",
            },
            ControlHint {
                input: "Z / X",
                action: "Strafe in Eye / Shoulder View",
                notes: "Strafes the controlled unit left (`Z`) or right (`X`) while in 1st-person Eye View or 2nd-person Shoulder View.",
            },
            ControlHint {
                input: "Minus / Equals",
                action: "Zoom camera",
                notes: "Zooms Top View camera out (`Minus`) or in (`Equals`); also clickable on the bottom-right Camera Control widget at `(410, 410)`.",
            },
            ControlHint {
                input: "Radar click (16..167, 13..164)",
                action: "Jump camera / radar view",
                notes: "Left-click inside the top-left 151x151 radar to jump camera to that cavern spot; buttons below toggle Map View, Tagged View, Zoom In/Out, and Radar collapse.",
            },
            ControlHint {
                input: "Border move (x<=3 || x>=636 || y<=3 || y>=476)",
                action: "Edge-scroll map",
                notes: "Pans the cavern camera (and scrolls the Level Select map vertically, e.g. `(320, 478)` to reach Campaign Mission 1 `Driller Night!` at `(315, 180)`); return pointer to `(320, 240)` to stop scrolling.",
            },
            ControlHint {
                input: "F2+W / F2+Q / F2+R / F2+E",
                action: "Drill / Clear Rubble / Reinforce / Dynamite",
                notes: "Contextual wall and floor orders: select a drillable wall or rubble tile (not a Raider), wait ~15 flips for the right sidebar to finish sliding in, then send `chord F2,<Key>` (or click the right-sidebar icon). `F2+G` cancels drilling; `F2+E` also places Electric Fences.",
            },
            ControlHint {
                input: "F2+M / F2+C / F2+V / F2+A / F2+P",
                action: "Teleport Raider / Build menus / Power Path",
                notes: "`F2+M` teleports a Raider down; `F2+C` Buildings menu (`F2+1`..`Equals`); `F2+V`/`F2+A` Small/Large Vehicles; `F2+P` builds Power Path, picks up, or toggles building power; `F2+B` Back.",
            },
            ControlHint {
                input: "F2+J / F2+O / F2+I / F2+U / F2+D",
                action: "Upgrade / Train / Tools / Unload / Enter-Exit",
                notes: "`F2+J` upgrades Raider/building; `F2+O` Train Skill submenu; `F2+I` Get Tool submenu; `F2+U` drop/unload/upgrade vehicle; `F2+D` enter/exit vehicle or cancel construction.",
            },
            ControlHint {
                input: "F2+1 / F2+2 / F2+3 / F2+T",
                action: "Eye View / Shoulder View / Top View / Track",
                notes: "Switches camera onto the selected Raider or Vehicle (`F2+1` 1st-person, `F2+2` 2nd-person), returns to overhead Top View (`F2+3`), or locks tracking (`F2+T`).",
            },
            ControlHint {
                input: "Top Panel (330, 0) & Priorities (495, 12)",
                action: "Action Stations / Options / AI Priorities",
                notes: "Click Action Stations `(421, 12)`, Options/Pause `(458, 12)`, or Priorities `(495, 12)` to reorder or disable autonomous Raider jobs (Collect Crystals, Drill, Clear Rubble, Construction, Reinforce).",
            },
            ControlHint {
                input: "Space",
                action: "Skip / dismiss briefing / toggle bubbles",
                notes: "Skips intro movies, advances/dismisses Mission Brief and tutorial prompts, and toggles overhead task/status bubbles during gameplay.",
            },
            ControlHint {
                input: "Escape / P",
                action: "Pause Menu / quick pause",
                notes: "`Escape` skips movies, backs out of menus, or opens the in-mission Paused Menu (contains `Options` -> `Game Speed` slider `0..5`); `P` toggles gameplay pause directly. `Escape` does not quit.",
            },
            ControlHint {
                input: "S / M",
                action: "Toggle SFX / Music",
                notes: "Bare `S` toggles Sound Effects on/off; bare `M` toggles Red Book CD music on/off.",
            },
        ],
        hazards: &[
            "Keep the pointer inside the safe interior (`4..=635`, `4..=475`) when idle; entering the 4 px screen border (`x <= 3 || x >= 636 || y <= 3 || y >= 476`) triggers continuous edge-scrolling away from base.",
            "All contextual HUD action hotkeys require holding `F2` (`chord F2,<Key>`) after the right sidebar finishes sliding in (~15 flips after selecting a tile/unit), or clicking the sidebar icon; bare letter keys do not issue orders (bare `M` toggles music, bare `S` toggles SFX).",
            "Avoid `F2+S` with a Raider selected or `F2+D` with a building/vehicle selected unless intentionally teleporting them back to the L.M.S. Explorer.",
            "Retail `Lego.cfg` disables debug keys (`AllowDebugKeys FALSE`): `Keypad8`/`Keypad9` game-speed keys, `F6`, `Tab`, `Ctrl+F`, and `Ctrl+G` do not work; change Game Speed via Pause Menu -> Options.",
            "Buildings require adjacent Power Paths (`OnlyBuildOnPaths TRUE`); unstable walls trigger landslides unless reinforced (`F2+R`); oxygen-limited missions fail at 0% air unless a Support Station is built.",
            "Front-end menu loops do not poll window close; closing there stops cleanly after core's 5-second grace window.",
        ],
        goals: &[
            Goal {
                id: "boot-to-level-select",
                name: "Boot to Level Select",
                milestone: Milestone::FirstLight,
                description: "Skip intro movies, click Start Game at (320, 222), and wait past the 3D rockfall transition to the Level Select cavern map.",
            },
            Goal {
                id: "launch-mission-1",
                name: "Launch Mission 1: Driller Night",
                milestone: Milestone::FirstLight,
                description: "Scroll the Level Select map down via (320, 478), return pointer to interior, click Campaign Mission 1 (Driller Night!) at center cavern (315, 180) (un-scrolled (155, 160) is Training Mission 1: 1. Moving Rock Raiders), dismiss the Mission Brief pages with Space, and reach active 3D cavern gameplay.",
            },
            Goal {
                id: "mission-1-driller-night",
                name: "Complete Mission 1: Driller Night",
                milestone: Milestone::Playable,
                description: "Drill through cavern walls (F2+W or Drill icon) to locate the Tool Store, clear rubble, and collect all 5 Energy Crystals.",
            },
            Goal {
                id: "advance-to-mission-2",
                name: "Advance to Mission 2+ (The Path To Power / Rubble Trouble)",
                milestone: Milestone::Playable,
                description: "Advance past Mission 1's reward screen, save progress, and launch the next unlocked cavern (Level05 The Path To Power: build Power Station via upgraded Tool Store, Power Paths & Teleport Pad; or Level03 Rubble Trouble!: reinforce walls, clear rubble, collect 5 crystals).",
            },
            Goal {
                id: "campaign-complete",
                name: "Complete All 25 Campaign Missions",
                milestone: Milestone::Complete,
                description: "Clear every mission from Level01 Driller Night! through Level25 Rocky Horror with profile saves in C:\\LEGORR\\DATA\\SAVES\\.",
            },
        ],
        save_files: &["C:\\LEGORR\\DATA\\SAVES\\0.SAV"],
    },
};

#[cfg(test)]
mod tests {
    use super::*;
    use lego_legacy_core::control::json::parse_json;
    use lego_legacy_core::control::{format_controls_json, format_goals_json, format_meta_json};
    use std::collections::BTreeSet;

    #[test]
    fn title_guide_serializes_valid_json() {
        let guide = &ROCK_RAIDERS.guide;
        assert!(!guide.is_empty());
        const { assert!(!ROCK_RAIDERS.guide.escape_quits) };
        assert_eq!(guide.resolution, (640, 480));
        assert_eq!(guide.bpp, 16);
        assert_eq!(guide.refresh_hz, 60);
        assert_eq!(
            guide.mouse_mode,
            MouseMode::SoftwareAbsolute {
                requires_poll_release: false,
            }
        );
        assert!(guide.controls.len() >= 10);
        assert!(guide.hazards.len() >= 4);
        assert!(guide.goals.len() >= 4);

        let mut goal_ids = BTreeSet::new();
        for goal in guide.goals {
            assert!(!goal.id.is_empty());
            assert!(!goal.name.is_empty());
            assert!(!goal.description.is_empty());
            assert!(goal_ids.insert(goal.id), "duplicate goal id {}", goal.id);
        }
        assert!(goal_ids.contains("mission-1-driller-night"));
        let launch_m1 = guide
            .goals
            .iter()
            .find(|g| g.id == "launch-mission-1")
            .expect("launch-mission-1 goal");
        assert!(
            launch_m1.description.contains("(320, 478)")
                && launch_m1.description.contains("(315, 180)")
                && launch_m1.description.contains("Moving Rock Raiders"),
            "launch-mission-1 must document scroll to (315, 180) and distinguish Training Mission 1 at (155, 160)"
        );
        let click_hint = guide
            .controls
            .iter()
            .find(|c| c.input.contains("Left click"))
            .expect("Left click control hint");
        assert!(
            click_hint.notes.contains("(320, 478)")
                && click_hint.notes.contains("(315, 180)")
                && click_hint.notes.contains("Moving Rock Raiders"),
            "Left click hint must distinguish Training Mission 1 at (155, 160) from Campaign Mission 1 at (315, 180) (#351)"
        );

        for hint in guide.controls {
            assert!(!hint.input.is_empty());
            assert!(!hint.action.is_empty());
            assert!(!hint.notes.is_empty());
        }

        let meta_v = parse_json(&format_meta_json(&ROCK_RAIDERS)).expect("valid meta JSON");
        let meta_obj = meta_v.as_object().unwrap();
        assert_eq!(meta_obj["ok"].as_bool(), Some(true));
        let meta_mm = meta_obj["guide"].as_object().unwrap()["mouse_mode"]
            .as_object()
            .unwrap();
        assert_eq!(meta_mm["kind"].as_str(), Some("software_absolute"));
        assert_eq!(meta_mm["requires_poll_release"].as_bool(), Some(false));

        let controls_v =
            parse_json(&format_controls_json(&ROCK_RAIDERS)).expect("valid controls JSON");
        let controls_obj = controls_v.as_object().unwrap();
        assert_eq!(controls_obj["ok"].as_bool(), Some(true));
        let ctrl_mm = controls_obj["mouse_mode"].as_object().unwrap();
        assert_eq!(ctrl_mm["kind"].as_str(), Some("software_absolute"));
        assert_eq!(ctrl_mm["requires_poll_release"].as_bool(), Some(false));

        let goals_v = parse_json(&format_goals_json(&ROCK_RAIDERS)).expect("valid goals JSON");
        assert_eq!(goals_v.as_object().unwrap()["ok"].as_bool(), Some(true));
    }
}
