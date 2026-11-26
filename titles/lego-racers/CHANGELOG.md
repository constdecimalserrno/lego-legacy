# Changelog: lego-racers

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-26

Calibrated `TitleGuide`, complete `AGENTS.md` playbook, and real-disc headless + `/watch` WS v2 cursor alignment proof (#374): sets `GUIDE.mouse_mode` to `MouseMode::SoftwareRelative { virtual_span: (640, 480), start_pos: (0, 0) }`, updates `GUIDE` controls, hazards, goals, and unit tests for `/api/{meta,controls,goals}` and `ControlBus`, completes `titles/lego-racers/AGENTS.md` with empirically verified `640×480` menu hotspots, racing tactics, power-up matrix, Car Builder hotkeys, `Save\0\LEGORac1` persistence notes, and reference routes, and adds disc-gated `titles/lego-racers/tests/cursor_alignment.rs` proving `<= 2 px` `MENUDATA/ARROW.BMP` tip alignment headlessly, on `--cursor` crosshairs, and over live `/api/stream?fps=60` WebSocket v2 (`pointer_shown == false`) across Main Menu, `/watch` Human Take Over Circuit Select, Build Menu, and post-race return to Main Menu.

Playable Car Builder, `Save\0\LEGORac1` persistence, Imperial Grand Prix Single Race, measured `FlipCost::Refresh(60)`, and committed deterministic routes (#373): commits `routes/build-racer.route` (unflagged boot -> `BUILD` -> `NEW RACER` -> `MIX` driver -> `MAKE LICENSE` `"ADA"` -> `QUICK BUILD` -> 3D Brick Builder -> DirectInput relative mouse click on Exit Door -> `FINISH` -> persists `Save\0\LEGORac1` (`1,792` B, `LR\x01\x80\x04`, UTF-16LE `"ADA"`) and cleanly reloads on a second cold boot) and `routes/first-race.route` (unflagged boot -> `SINGLE RACE` -> Imperial Grand Prix -> 3-2-1-GO -> Turbo Start -> full 3D Lap 1 with Red Cannonball fire, Blue Shield activation, White Power-Plus upgrade, and Power Slides -> `LAP 2` (`BEST 1:10:41`) -> Pause -> `EXIT` -> Main Menu -> clean exit 0 at 6,371 flips). Verifies exact 60.0 Hz lap-timer cadence with zero clock-spin stalls (`FlipCost::Refresh(60)` at `4.06x` uncapped pace / `243.7` host FPS across the 72 s race lap), raises `BOOT_BUDGET` to `200_000` for real DirectInput `EnumObjects` 104-key keyboard enumeration, and adds real-disc integration tests proving `Save\0\LEGORac1` save/reload persistence, non-uniform 3D race rendering + non-silent circuit audio, side-by-side determinism (`assert_same_machine`), and bit-exact mid-race `LLSNAP v2` cold snapshot/restore parity at flip 2620.

Unflagged DirectShow intro movies (`lmicmp.avi`, `hvscmp.avi`, `introcmp.avi`) through Main Menu (#372): sets `click_in_a_movie: Some(0x39)` (Space `WM_KEYUP` skip for mouse clicks during movies), updates `routes/first-light.route` to run unflagged without `-novideo` (playing unskippable `lmicmp.avi`, skipping `hvscmp.avi` via Space and `introcmp.avi` via click, reaching the animated 3D Captain Redbeard Main Menu with live `theme.tun` music and exiting 0 at 581 flips), and adds real-disc integration tests verifying full unflagged playback of all 3 intro AVIs (including 1,080 scalable-mode IV50 frames of `introcmp.avi`), non-silent PCM intro + Main Menu audio, exact replay determinism, and bit-exact `LLSNAP v2` cold snapshot round-trips both mid-movie (flip 100) and on Main Menu (flip 555).

Deterministic `.tun` ADPCM music streaming on the declared clock via `WINMM!timeSetEvent` / `timeKillEvent` (#371): real-disc integration tests in `titles/lego-racers/tests/lego_racers.rs` prove non-silent stereo PCM (`theme.tun`, 28,672 samples, RMS 5,216.4, peak 27,562 across 11 active blocks on `first-light.route`), fresh decoded PCM past the 2.04 s (`0x2c000` B) circular DirectSound buffer wrap (`26` 100 ms periodic callbacks over 461 flips, `22,045/22,050` distinct samples vs. lap 1), bit-exact audio determinism across independent runs, clean `timeKillEvent` teardown, and bit-exact `LLSNAP v2` cold snapshot/restore parity including audio.

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
