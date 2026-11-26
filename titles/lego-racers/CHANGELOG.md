# Changelog: lego-racers

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-26

On the roster (#369): `LEGORacers.exe` (`987,222` B, linked 2001-01-12, unwrapped, statically
linked CRT) out of the Masterpiece pressing's InstallShield 5 cabinet (`DATA1.HDR` + `DATA1.CAB`,
56 entries at the install root, `C:\Program Files\LEGO Media\Games\LEGO Racers`), beside
`GolDP.dll` (`413,696` B), `LEGO.JAM` (`19,642,024` B), 50 `.tun` ADPCM music tracks, and 3 intro
`.avi` movies. Neither pressing carries Red Book audio (`red_book: None`, `bank: &[]`). The
original 1999 pressing (`racers`) packs a SafeDisc v1 wrapper (`LEGORacers.exe` with `.txt` /
`.txt2` sections beside encrypted `LEGORacers.icd`) and is recognised and refused as
`Wrapped("SafeDisc")` per Golden Rule 6 and ADR-0006.

Declares the roster's baseline machine (Windows 98 SE, Pentium II 450, 128 MB, 640x480x16, 60 Hz
`FlipCost::Refresh(60)`, C:/D: without Joliet or a floppy drive) and initial `GUIDE: TitleGuide`
plus `titles/lego-racers/AGENTS.md` sourced from `MANUAL.PDF`, `README.TXT`, and `LEGORacers.exe`
disassembly. Started, its statically linked C runtime initializes and its window comes up before
stopping at its first wall, `USER32.dll!BringWindowToTop`, called from `0x0048ADF7` at cursor
50,273 with nothing presented; first-light Win32/D3D3 bringup follows in #370. Untagged until its
release.
