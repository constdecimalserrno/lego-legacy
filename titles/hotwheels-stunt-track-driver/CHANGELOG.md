# Changelog: hotwheels-stunt-track-driver

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-18

Reclassified `GUIDE.mouse_mode` from `AbsoluteKept` to `MouseMode::SoftwareRelative { virtual_span: (640, 480), start_pos: (320, 240) }` to eliminate the post-race `SetCursorPos(320, 240)` cursor desync in `ControlHost`, `--cursor` screenshots, and `/watch` Human Take Over, updated `AGENTS.md` / `GUIDE` hazards accordingly, and added disc-gated headless + live loopback `/api/stream` WebSocket v2 cursor-alignment proof (`tests/cursor_alignment.rs`, pre-race, post-race, and first-click Green GO advance) (#329). Strictly outside guest state; no route moved.

Added `AGENTS.md` agent playbook and enriched static `GUIDE` (`controls`, `hazards`, `goals`, `save_files`) with verified `STUNT.EXE` DirectInput controls (`Up`/`Down`/`Left`/`Right`, `LeftShift`/`LeftControl` aerial stunts, `Space`, `P`/`Pause`, `C` crash/respawn, `F1` race resolution, `Escape` quit hazard), menu click hold/settle rules (`--hold-flips 5..6`), post-race `SetCursorPos(320, 240)` cursor offset caveat (#329), and concrete progression goals from Single Race and Custom Garage Gold Medal through Championship Tracks 1–6 and Secret Track `WIRE` Gold Medal (#334). Strictly outside guest state; no route moved.

Declares static `GUIDE: TitleGuide` metadata (`AbsoluteKept`, 8bpp, 60 Hz, `escape_quits: true` safety guard requiring `force: true` to send `Escape`, `MicroStep`, `Up`/`Left`/`Right`/`LeftShift`/`LeftControl` racing & stunt controls, Mattel `"TT"` `(270,100)` Track Select hotspot for Secret Track `WIRE`, First Race & Championship + `WIRE` Gold Medal goals, and `saves\onoff.dat`/`times.dat`/`wheel.dat`) and threads it through `run::main_with_guide` for `/api/meta`, `/api/controls`, and `/api/goals` (#311, core 1.2.0). Strictly outside guest state; no route moved.

Added `routes/secret-track.route` (#285): a deterministic route from a clean `--data-root` that
completes Championship Tracks 1–6 on the unmodified retail binary to unlock `saves\onoff.dat` (28
bytes, `01 00 00 00` × 6 + `00 00 00 00`), right-clicks the invisible Mattel "TT" logo hotspot on
Track Select at flip 17,889 to arm Secret Track 7 (`WIRE.JAM`, neon-green wireframe grid), drives
and stunts through `WIRE` to a 1:16 Bronze Medal finish (04 stunts, 0 crashes, writing
`saves\times.dat` at flip 20,756), and quits cleanly on Escape at flip 20,977 (`exited 1`, cursor
77,377,021,811, 20,980 flips, 656,816 declared ms, digest `4c032fc7…`, 309,495,794 clock-spin
instructions passed; persists `SAVES/ONOFF.DAT` 28 B, `SAVES/TIMES.DAT` 480 B, and
`SAVES/WHEEL.DAT` 240 B). Regression and side-by-side determinism
tests in `tests/hotwheels.rs` pin the `saves\onoff.dat` unlock bytes, all seven `saves\times.dat`
writes, non-uniform pairwise-distinct `WIRE` gameplay frames (including wireframe terrain checks),
clean exit, and bit-exact replay across independent data roots. Adding a committed route is a minor
reach per ADR-0007 (making this title's next release 1.1.0; the crate is already past its `v1.0.0`
tag and stays at `1.0.1` until release).

Took core 1.1.0, which the Windows (#107) and Linux (#108) backends joined, written but untested on those systems. No route moved.

Core's `GetVersion` answers as Windows 98 SE did, with the top bit that says it is not NT (#129).
The title's C runtime only stores the word, so no route takes a different turn, and every run's
digest moves; core's changelog lists them.

Took core 1.2.0 (#140), whose `DestroyWindow` sends the window `WM_DESTROY` and then
`WM_NCDESTROY`, as Microsoft documents, and whose `DefWindowProcA` destroys the window it is handed
`WM_CLOSE` for. The title destroys its window as it exits and hands both messages to
`DefWindowProcA`, so its two runs that exit, the escape route and `first-race.route`, end 54
instructions later, their flips, declared milliseconds and exit code unchanged; core's changelog
lists them. The maintainer blessed the move on 2026-10-21. Its run moved, so a person should drive
the first race again before this version is released.

Closing its window tells it, as Windows does (#140), where the run stopped at once without telling
it: it hears `WM_SYSCOMMAND` with `SC_CLOSE` and exits 1, as it always exits. A second close stops
the run at once. No committed route closes, so no run moves.

Took core 1.2.0's `WM_TIMER` (#138). The title sets a one-second timer as it starts and kills it
only as it quits, and its window procedure now hears it once a declared second: it latches a frame
rate nothing in the game reads. The game reads its tick count at 120 places, so its two runs that
exit move: `first-race.route` ends at 13,462,673,372 where it ended at 13,462,722,543, and the
escape route at 583,424,086 where it ended at 583,424,325, their flips, declared milliseconds and
exit code unchanged; core's changelog lists them. The race still finishes, with the same
`WHEEL.DAT` and `TIMES.DAT` written at the same flips. The maintainer blessed the move on
2026-10-21. Its runs moved again, so a person should drive the first race before its next release.

Its declared machine lacks no library (`absent_libraries`, #146): no gate run asks `LoadLibraryA` for one, so no route moves.

Took core 1.2.0's mouse (#141): a player clicks on the picture to give the title the mouse, and
Escape frees it, neither reaching the game, so the Escape that frees the mouse never quits; the
next one does. No route records either, so no route moved.

Declares core 1.2.0's `joliet` as `false` (#144), by #142's rule: Joliet is LEGO Chess's alone,
because Alpha Team's runs move with it, until the maintainer decides. Its disc is read by the
primary tree's names alone, as before, so no route moved. Nothing in this title asks for the
Joliet tree: read through it, the disc answers the same names but for case, and none of its runs
moved with it forced on, measured at `7509b49`. Core's `FindFirstFileA` on the CD is now a
wall; this title never searches.

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
stays until the title has left, as on Windows (#183, core 1.2.0). No route moved.

Built for an Intel Mac, it no longer reads garbage or crashes at any press on its window or a move
of the free mouse over it (#186, core 1.2.0): built and clippy-clean for Intel, and the struct path
run as x86_64 under Rosetta, not played on an Intel Mac. No route moved.

Declares no floppy drive (#145), core's new required `Declared::floppy` (unreleased; core's next
release is 2.0.0, #163), which is the machine it has always had: C: and D: only. Nothing it sees
moves.

Took core 1.2.0's activation (#196): its window procedure is told, after `WM_ACTIVATEAPP`, that its
window is active (`WM_ACTIVATE`) and has the keyboard (`WM_SETFOCUS`), as Windows tells it. Its two
runs that exit move, their flips, declared milliseconds and exit code unchanged: `first-race.route`
ends at 13,462,673,233 where it ended at 13,462,673,372, and the escape route at 583,424,170 where it
ended at 583,424,086. Its two runs held to a budget end in another state; core's changelog lists
the digests. Its procedure hands both to `DefWindowProcA`, and its first pump runs between flips 99
and 100, so the flips before it are unchanged. A move the maintainer blessed on 2026-10-22. A person
should drive the first race again on the MacBook before its next release.

Took core 1.2.0's input between flips (#215): at a window, what a person does while a frame stands
core's `ASK_BETWEEN_FLIPS_MILLIS` or more of declared time, as its boot and the race's finish do,
reaches it at the frame's next input poll rather than at its flip. Core's put-back ceiling, an ask
made before the host is told of a pointer put earlier in the frame, hardly reaches it: it puts its
pointer at 2 of the race's 3,903 flips, and reads its mouse's buffered moves all through a frame.
No route moved.

Took core 1.2.0's clock-spin pass (#217): none of its committed routes passes a spin, and no
route moved. The waits its routes reach poll as well as read the clock, so they run pass by pass as
before. Off its routes it has pure spins, which a run at a window would pass:
- its frame limiters at 0x4299F2 and 0x42C77B;
- its reveal waits at 0x42254F, 0x422B43 and 0x422E7E, whenever no music track is set.

They are untested by a route, held by the pass's proof alone, until #229 commits one. Its first
race, which passes nothing, takes 0.67 % more of the host's instructions on the Mac mini, 2,470.0
billion where it took 2,453.7.

Took core 1.2.0's `WM_CHAR` (#157). The title translates its keys, so its window procedure is now
handed a character after Escape and Space, 40 instructions each. Its two runs that exit move, their
flips, declared milliseconds and exit code unchanged: `first-race.route` ends at 13,462,673,313
where it ended at 13,462,673,233, and the escape route at 583,424,210 where it ended at 583,424,170.
The maintainer blessed the move on 2026-11-23. Its run moved, so a person should drive the first race
again before this version is released.

## 1.0.0 — 2026-10-18

Playable. `routes/first-race.route` drives the first race to the finish.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: the first
race, driven to the finish.
