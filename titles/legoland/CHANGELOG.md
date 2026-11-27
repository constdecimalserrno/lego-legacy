# Changelog: legoland

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-27

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
