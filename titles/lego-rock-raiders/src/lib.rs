//! LEGO Rock Raiders: the roster's first title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::cpu::Processor;
use lego_legacy_core::roster::ROCK_RAIDERS as ENTRY;
use lego_legacy_core::title::{
    AgentCadence, ControlHint, Declared, Goal, Installed, Milestone, MouseMode, TitleGuide,
    WINDOWS_98_SE,
};
use lego_legacy_core::Title;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const ROCK_RAIDERS: Title = Title {
    key: ENTRY.key,
    version: env!("CARGO_PKG_VERSION"),
    // A flip waits for the declared display's next refresh, which is what
    // DirectDraw's Flip does unless a title asks otherwise, and this one does
    // not (#9). Sixty a second is what GetMonitorFrequency declares. Its movies
    // pace themselves by waiting for each frame, and every frame they ask for
    // lands on a refresh.
    flip_cost: FlipCost::Refresh(60),
    // What this title is, rather than what machine it runs on: the launcher
    // needs the same facts, so they live in the roster and are read from here.
    image: ENTRY.image,
    red_book: ENTRY.red_book,
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
};

/// Static playability metadata served via `/api/meta`, `/api/controls`, and
/// `/api/goals` (#311).
pub const GUIDE: TitleGuide = TitleGuide {
    display_name: "LEGO Rock Raiders",
    summary: "Real-time underground mining and base-building strategy game.",
    resolution: (640, 480),
    bpp: 16,
    refresh_hz: 60,
    mouse_mode: MouseMode::AbsoluteKept,
    escape_quits: false,
    skip_keys: &["Escape", "Space"],
    cadence: AgentCadence::BurstThenPause { burst_ms: 500 },
    controls: &[
        ControlHint {
            input: "Left click",
            action: "Select / interact",
            notes: "Select raiders, buildings, walls, or HUD icons; clicks skip intro movies directly.",
        },
        ControlHint {
            input: "F2+W",
            action: "Drill wall",
            notes: "Issues a drill order on the selected rock or dirt wall.",
        },
        ControlHint {
            input: "Minus / Equals",
            action: "Zoom camera",
            notes: "Zoom camera out (`Minus`) or in (`Equals`).",
        },
        ControlHint {
            input: "Left / Right",
            action: "Rotate camera",
            notes: "Rotate the 3D cavern camera counter-clockwise or clockwise.",
        },
        ControlHint {
            input: "Up / Down",
            action: "Tilt camera",
            notes: "Pitch the 3D cavern camera up or down.",
        },
        ControlHint {
            input: "Space / Escape",
            action: "Skip / menu back",
            notes: "Skip intro and briefing movies or back out of menus (`Escape` does not quit).",
        },
    ],
    hazards: &[
        "Avoid moving the pointer into the screen border (`x < 4 || x > 635 || y < 4 || y > 475`), which triggers continuous edge-scrolling away from the base.",
        "Front-end menu loops do not poll window close; closing there stops cleanly after core's 5-second grace window.",
    ],
    goals: &[Goal {
        id: "mission-1-driller-night",
        name: "Mission 1: Driller Night",
        milestone: Milestone::Playable,
        description: "Find the Tool Store, drill walls, and collect 5 Energy Crystals.",
    }],
    save_files: &["C:\\LEGORR\\DATA\\SAVES\\0.SAV"],
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
        assert_eq!(GUIDE.resolution, (640, 480));
        assert_eq!(GUIDE.bpp, 16);
        assert_eq!(GUIDE.refresh_hz, 60);
        for body in [
            format_meta_json(&ROCK_RAIDERS, &GUIDE),
            format_controls_json(&ROCK_RAIDERS, &GUIDE),
            format_goals_json(&ROCK_RAIDERS, &GUIDE),
        ] {
            let v = parse_json(&body).expect("valid JSON");
            assert_eq!(v.as_object().unwrap()["ok"].as_bool(), Some(true));
        }
    }
}
