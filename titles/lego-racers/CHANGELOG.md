# Changelog: lego-racers

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-26

First light to rendered Main Menu under `--guest-args -novideo` (#370): commits
`routes/first-light.route` (`flip 330 close`, exiting 0 cleanly at 331 flips after the LEGO Media
splash fades into the animated 3D Captain Redbeard Main Menu) alongside real-disc integration tests
proving non-uniform splash + animated 3D Main Menu rendering, exact replay determinism
(`assert_same_machine`), and bit-exact `LLSNAP v2` cold snapshot round-trip at flip 305. Default
unflagged boot stops deterministically at `ole32.dll!CoCreateInstance` for DirectShow
`CLSID_FilterGraph` (owned by #372); multimedia timer callback dispatch for `.tun` music follows in
#371. Observed software cursor (`MENUDATA/ARROW.BMP`) starting at `(0, 0)` on Main Menu for #374.

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
