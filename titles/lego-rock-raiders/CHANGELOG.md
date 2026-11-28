# Changelog: lego-rock-raiders

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-18

Takes `lego-legacy-core` `=2.0.0` (#407): folds `name`, `bank`, `image`, `red_book`, and `guide` inline onto `ROCK_RAIDERS`, drops the standalone `GUIDE` constant and `core::roster` import, moves the wrapped-disc pressing fixture into `tests/rock_raiders.rs`, and calls `run::main(&ROCK_RAIDERS)` / `ControlRig::in_process(&ROCK_RAIDERS, ...)`. Strictly outside guest state; no route moved.

Migrates `tests/cursor_alignment.rs` onto `lego_legacy_core::testkit::spectator` (`ControlRig::in_process`, draining `SpectatorClient`, `crosshair_center`, `fraction_diff`), deleting duplicated PNG decoding, crosshair detection, HTTP/1.1, RFC 6455 WS v2 / XOR-RLE, and RAII rig boilerplate while preserving every Main Menu, Level Select, Training Mission 1, and Priorities HUD alignment assertion (#411). Strictly outside guest state; no route moved.

Adds deterministic Campaign Mission 1 ("Driller Night!") playability TAS route `routes/mission-1-complete.route` (cold boot -> drill cavern walls -> discover Tool Store -> teleport Raiders -> collect 5 Energy Crystals -> Mission Completed summary at 93% score -> save Slot 1 `0.SAV`/`0.DAT`/`0.OSF` -> advance to Level Select with Mission 2 unlocked -> clean `ExitProcess(0)`) with regression and `assert_same_machine` determinism tests in `tests/rock_raiders.rs` (#297, #298). Zero emulator walls needed; active 3D gameplay window `[1245..12014]` measured at `3.36x` real-time pace (`201.9` host FPS, `60.0` guest FPS, `p50=4.70ms`, `p95=7.95ms`, `p99=11.19ms`, `max=26.20ms`). No existing route moved.

Reclassifies `GUIDE.mouse_mode` from `AbsoluteKept` to `SoftwareAbsolute { requires_poll_release: false }` (matching `LegoRR.exe`'s `ShowCursor(FALSE)` + DirectDraw software cursor) and adds a disc-gated headless + loopback `/api/stream` WebSocket v2 cursor alignment test (#331). Strictly outside guest state; no route moved.

Documents `LegoRR.exe` (`0x0041C370`) sidebar slide-in settlement (~15 flips before `0x004DF1F8` bit `0x80` accepts `F2+<Key>` chords) and tile-context selection for `F2+W`/`F2+Q` in `AGENTS.md` and `GUIDE` (#348). Strictly outside guest state; no route moved.

Fixes Level Select Mission 1 coordinates in `AGENTS.md` and `GUIDE` (#351): un-scrolled `(155, 160)` is `1. Moving Rock Raiders` (Training Mission 1), while Campaign Mission 1 `Driller Night!` requires scrolling down (`move 320 478`) to the center cavern at `(315, 180)`. Strictly outside guest state; no route moved.

Adds `AGENTS.md` agent playbook and enriches static `GUIDE: TitleGuide` (`controls`, `hazards`, `goals`) with verified `LegoRR.exe` / `Lego.cfg` camera, radar, Eye/Shoulder View, contextual `F2+<key>` HUD chords, Priorities/Top Panel hotspots, retail pause/audio keys, RTS hazards, and a 5-step progression ladder (#336). Strictly outside guest state; no route moved.

Declares static `GUIDE: TitleGuide` metadata (`AbsoluteKept`, 60 Hz, `BurstThenPause`, `F2+W`/camera controls, border edge-scroll hazard, Mission 1 "Driller Night" goal, and `C:\LEGORR\DATA\SAVES\0.SAV`) and threads it through `run::main_with_guide` for `/api/meta`, `/api/controls`, and `/api/goals` (#311, core 1.2.0). Strictly outside guest state; no route moved.

Took core 1.1.0, which the Windows (#107) and Linux (#108) backends joined, written but untested on those systems. No route moved.

The wrapped-pressing test looks for the image's name after the host's own path separator, so it
passes on Windows too (#107).

Core's `GetVersion` answers as Windows 98 SE did, with the top bit that says it is not NT (#129).
The title's C runtime only stores the word, so no route takes a different turn, and every run's
digest moves; core's changelog lists them. The declared-machine test reads the build from
`dwBuildNumber`'s low word.

Core answers the `IDirect3D3` the title's mode dialog asks its DirectDraw for, and enumerates the
declared machine's one device through it (#132). The dialog had listed "Error: DirectX6 not
installed.", and the title went on with no device and read its flags through a null pointer. That
gave 0 until core made the bottom of the address space fault as Windows 9x does. No route takes a
different turn, the frames compared are the same pictures, and every end line is unchanged. The seven runs past the dialog
move their digests: core's changelog lists them, and the maintainer accepted them. The
screen-mode test also checks for the enumeration.

Core's retained mode projects with uniform scaling, as d3drm does (#134). The level is drawn a
third taller than the stretch drew it, showing less of it up and down, and the Chief stands as
tall and as narrow as in the retail game's Mission Brief. No route takes a different turn and every
end line is unchanged; `first-mission.route`'s digest moves, which core's changelog lists. Accepted by
the maintainer on 2026-10-20; the first mission is played again by a person before this release.

Took core 1.2.0 (#140), in which closing its window tells it, as Windows does, where the run
stopped at once without telling it. In a mission it stops its CD audio and exits 0. Its front end's
screens run loops of their own that never look at the flag its window procedure sets, so there it
runs on with its window gone until core stops the run five declared seconds later; a second close
stops it at once. No committed route closes, and the title never destroys its window in one, so no
route moved.

Its declared machine lacks no library (`absent_libraries`, #146): no gate run asks `LoadLibraryA` for one, so no route moves.

Took core 1.2.0's mouse (#141): a player clicks on the picture to give the title the mouse, and
Escape frees it, neither reaching the game. No route records either, so no route moved.

Declares core 1.2.0's `joliet` as `false` (#144), by #142's rule: Joliet is LEGO Chess's alone,
because Alpha Team's runs move with it, until the maintainer decides. Its disc is read by the
primary tree's names alone, as before, so no route moved. Nothing in this title asks for the
Joliet tree: read through it, the disc answers the same names but for case, and none of its runs
moved with it forced on, measured at `7509b49`. Core's `FindFirstFileA` on the CD is now a
wall; this title searches only its install directory on C:.

Minimizing its window on macOS, or hiding the application, no longer closes it (#173, core 1.2.0):
it is told nothing new beyond the keys and buttons it held let go, runs on unseen but heard, and is
shown again on restore. No route moved.

Closing its window on macOS with its button no longer leaves the run messaging a window macOS may
have freed while the title leaves (#175, core 1.2.0): the object behind the window is kept until the
process ends. No route moved.

Switching to another Space on macOS frees a captured mouse, as losing the front does, and keys
pressed there no longer reach the game (#176, core 1.2.0). No route moved.

On macOS, quitting from the Dock, logging out or restarting closes it as the window's button does
(#178, core 1.2.0), where the process ended at once; a logout or restart is interrupted, and asked
again once the title has left. No route moved.

On Windows, signing out, shutting down or restarting closes it as the window's button does (#184,
core 1.2.0), where the process ended at once; Windows holds the sign-out, shutdown or restart on
its screen, saying "The game is closing.", while the title leaves, and whether it then goes on by
itself, Microsoft does not say. "Sign out anyway" there ends it at once. No route moved.

A long run on macOS no longer grows the process's memory (#180, core 1.2.0): what macOS hands back
to be let go later is now let go at every pump, twice a flip. No route moved.

Closing its window on macOS with its button no longer takes the window off the screen at once: it
stays until the title has left, as on Windows, or, at its front end, which never leaves, until the
run is stopped five declared seconds later (#183, core 1.2.0). No route moved.

Built for an Intel Mac, it no longer reads garbage or crashes at any press on its window or a move
of the free mouse over it (#186, core 1.2.0): built and clippy-clean for Intel, and the struct path
run as x86_64 under Rosetta, not played on an Intel Mac. No route moved.

Declares no floppy drive (#145), core's new required `Declared::floppy` (unreleased; core's next
release is 2.0.0, #163), which is the machine it has always had: C: and D: only. Nothing it sees
moves.

Core's declared tree stops the run at a wall naming the file when a rip's cabinet holds a file that
will not unpack, or unpacks to another size than its header gives (#169, core 1.2.0). The title was told the
file was missing and ran on. A good rip unpacks every file, so no route moves.

Took core 1.2.0's activation (#196): its window is told, after `WM_ACTIVATEAPP`, that it is active
(`WM_ACTIVATE`) and has the keyboard (`WM_SETFOCUS`), as Windows tells it. Its message loop drops
`WM_ACTIVATEAPP` before its procedure sees it (the filter at 0x004782D0), so these two are the
first messages its procedure is handed on a committed route: it answers `WM_ACTIVATE` itself, with
0, and hands `WM_SETFOCUS` to `DefWindowProcA`. Every flip comes 131 instructions later, so every
run that makes its window ends in another state at the same instruction, flip and declared
millisecond; core's changelog lists the digests. A move the maintainer blessed on 2026-10-22. A
person should play it on the MacBook before its next release.

Took core 1.2.0's input between flips (#215): at a window, what a person does while a frame stands
core's `ASK_BETWEEN_FLIPS_MILLIS` or more of declared time reaches it at the frame's next input poll
rather than at its flip. That is the frame that ends a load, which polls at its end, just before its
flip, and each of the 35 frames of its boot splash, which `Sleep(100)` before they poll. No route
moved.

Took core 1.2.0's clock-spin pass (#217): every loop round its `timeGetTime` reads also sleeps,
pumps its messages or calls out, so nothing of it is passed, and no route moved. Its first mission,
by its committed test, takes 0.34 % more of the host's instructions on the Mac mini, 3,298.4 billion
where it took 3,287.1.

Took core 1.2.0's `WM_CHAR` (#157). The title translates its keys, so its window procedure is now
handed a character after Space and Escape. `first-mission.route`, which presses Space twice, and the
test's skip of the intro with Escape each end at the same cursor, flips and declared time with
another digest. The maintainer blessed the move on 2026-11-23.

## 1.0.0 — 2026-10-18

Playable. A person played the first mission through on 2026-09-10, on an older build, and
`routes/first-mission.route` covers its first minute.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: to the level
select. A route through the whole first mission is #106.
