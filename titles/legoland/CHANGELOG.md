# Changelog: legoland

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-27

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
