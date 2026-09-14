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
        user: "LEGO-LEGACY",
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
};

fn main() -> ExitCode {
    run::main(&LEGO_ALPHA_TEAM)
}
