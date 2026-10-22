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

Took core 1.2.0's input between flips (#215): at a window, what a person does while a frame stands a
tenth of a second or more, as its boot and the race's finish do, reaches it at the frame's next input
poll rather than at its flip. No route moved.

## 1.0.0 — 2026-10-18

Playable. `routes/first-race.route` drives the first race to the finish.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: the first
race, driven to the finish.
