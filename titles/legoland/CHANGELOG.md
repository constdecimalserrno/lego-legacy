# Changelog: legoland

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-27

Takes `lego-legacy-core` `=2.0.0` (#407): folds `name`, `bank` (`&GM16`), `image`, `red_book`, and `guide` inline onto `LEGOLAND`, drops the standalone `GUIDE` constant and `core::roster` import, and calls `run::main(&LEGOLAND)`. Strictly outside guest state; no route moved.

Playable (#396): calibrates `FlipCost::Refresh(36)` and `GUIDE.refresh_hz = 36` to match `Legoland.exe`'s `28 ms` (`~35.7 Hz`) frame floor (`0x4661d0`, yielding `35.0–35.3` guest FPS vs `36.0` intended FPS with `0.0%` over-budget active gameplay); commits unflagged, 100% legitimate routes `routes/tutorial-1.route` (`946` flips, `28.31 s` declared: Tutorial 1 `ObjList1.txt` completion + Park Map + `Options -> Save` `profiles\1save1.sav` + Finish door + clean exit `0`) and `routes/level-1.route` (`31,715` flips, `914.25 s` declared: full fresh-profile campaign progression through Tutorials 1–5 including `Spider.avi` and `Western.avi` FMVs -> Progress Map -> Campaign Level 1 San Francisco `ObjList6.txt` with all 6 food types, all 9 scenery varieties / 50+ scenery, full Driving School road loop with Pumps & Zebra Crossing, 100% power & ride access, `Options -> Save` `profiles\1save1.sav` `177,797` B, Mr. Bimble 8/8 100% green Appraisal victory -> Paris Level 2 unlocked -> clean exit `0`); adds real-disc integration tests in `tests/legoland.rs` covering Tutorial 1 + Level 1 completion, `profiles\1save1.sav` / `Profile1.txt` cold second-boot reload persistence, replay determinism (`assert_same_machine` + bit-exact audio & save bytes), and mid-park `LLSNAP v2` cold snapshot/restore parity; updates `titles/legoland/AGENTS.md` with verified UI hotspots and `FlipCost::Refresh(36)`. Untagged until its release.

DirectMusic interactive soundtrack (#399): plays `IMusic/` (`30` `.sgt` segments, `212` `.sty` styles, and bands) on the declared clock through `core::dmusic` + `core::synth` using Roland's `GM16.DLS` (`roster::LEGOLAND.bank = &GM16`, ADR-0008), with clean `REGDB_E_CLASSNOTREG` silent fallback when `GM16.DLS` is absent. Verified audible `"themeintro"` (`Segtheme1.sgt`) playback on `first-light.route` (`Player Details` and `Title Screen`), bit-exact PCM audio determinism across replays, and bit-exact `LLSNAP v2` cold snapshot/restore parity (including lazy `.sty` rehydration from `Tree`) in `tests/legoland.rs`. Untagged until its release.

Media & audio playback (#395): updates `routes/first-light.route` to run completely unflagged (145 flips, 5.484 declared s): plays `D:\lmi.avi` (skipped via `LeftControl+Q`) -> `D:\Intro.avi` (`IV50` + `0x0011` IMA ADPCM stereo audio, skipped via `Space`) -> `Player Details` -> `"ADA"` -> `Title Screen` -> `Exit Game -> Yes` -> clean `ExitProcess(0)`. Adds real-disc tests in `tests/legoland.rs` verifying non-silent PCM RMS during `Intro.avi` (`0x0011`) and post-intro UI/MS ADPCM audio, all 6 `0x0011` IMA ADPCM FMVs, `Speech\*.wav` (`0x0002` MS ADPCM), animating `AD_Blink.avi` (`112×96` `IV50` advisor overlay), full 587-frame `California.avi` (`320×200` `IV32`) decoding with mid-stream `LLSNAP v2` continuity, replay determinism (`assert_same_machine`), and bit-exact `LLSNAP v2` cold snapshot/restore parity mid-`Intro.avi` (flip 20), on `Player Details` (flip 55), and on `Title Screen` (flip 110). Untagged until its release.

First light (#394): boots headlessly (`--guest-args -nointro` until #395 adds IMA ADPCM `0x0011` for `Intro.avi`; unflagged boot already plays `lmi.avi` cleanly) through `Player Details` -> creates profile `"ADA"` (`profiles\Profile1.txt`) -> `Accept` -> rendered 640×480×16 `Title Screen` -> `Exit Game` -> `Yes` -> clean `ExitProcess(0)` (`routes/first-light.route`, 105 flips). Verified crisp `Lego.TTF` TrueType GDI text, animated `AD_Blink.avi` advisor orb, `rab over icon.lls` cursor at `(320, 240)`, side-by-side determinism (`assert_same_machine`), and bit-exact `LLSNAP v2` cold snapshot/restore parity at both `Player Details` and `Title Screen` in `tests/legoland.rs`. Untagged until its release.

On the roster (#393): `Legoland.exe` (`802,857` B, MSVC 6.0, linked 2000-04-07, unwrapped,
statically linked CRT) out of the Masterpiece pressing's InstallShield 3 archive (`main.z`, 331
members expanded into `C:\Program Files\LEGO Media\Games\LEGOLAND`), beside `Legoland.res`,
`Graphics1.res`, `Graphics2.res`, `Speech\*.wav` (1,266 MS ADPCM WAVs), and fullscreen `.avi`
movies read from `D:\` through Joliet. Neither pressing carries Red Book audio (`red_book: None`,
`bank: &[]`).

Declares the roster's baseline machine (Windows 98 SE, Pentium II 450, 128 MB, 640x480x16, 60 Hz
`FlipCost::Refresh(60)`, C:/D: with Joliet and no floppy drive, `registry: &[]`, and
`absent_libraries: &[("Ir50_32.dll", ...)]`) and initial `GUIDE: TitleGuide` plus
`titles/legoland/AGENTS.md` sourced from `Legoland.exe` disassembly, `Graphics1.res`, and
`readme.txt`. Started, its statically linked C runtime initializes before stopping at its first
wall, `VERSION.dll!GetFileVersionInfoSizeA`, called from `0x00458845` at cursor 15,622 with nothing
presented; first-light Win32 bringup follows in #394. Untagged until its release.
