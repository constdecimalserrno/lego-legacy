# Changelog: hotwheels-stunt-track-driver

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-18

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

## 1.0.0 — 2026-10-18

Playable. `routes/first-race.route` drives the first race to the finish.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: the first
race, driven to the finish.
