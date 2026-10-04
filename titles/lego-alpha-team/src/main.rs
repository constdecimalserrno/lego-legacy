//! LEGO Alpha Team: the roster's third title.

use lego_legacy_core::clock::FlipCost;
use lego_legacy_core::roster::ALPHA_TEAM as ENTRY;
use lego_legacy_core::title::{Declared, Installed, WINDOWS_98_SE};
use lego_legacy_core::{run, Title};
use std::process::ExitCode;

/// What this title declares about the machine it runs on.
///
/// Every value here is a declared value: chosen for this title, carried with
/// its reason, never measured from the host.
pub const LEGO_ALPHA_TEAM: Title = Title {
    key: ENTRY.key,
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
    image: ENTRY.image,
    red_book: ENTRY.red_book,
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
        megabytes: 128,
        modes: &[(640, 480, 16)],
        computer: "LEGO-LEGACY",
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
};

fn main() -> ExitCode {
    run::main(&LEGO_ALPHA_TEAM)
}
