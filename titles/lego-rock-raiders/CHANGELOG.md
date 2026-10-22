# Changelog: lego-rock-raiders

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-18

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
have freed while the title leaves (#175, core 1.2.0): the window still goes from the screen at once,
and only the object behind it is kept until the process ends. No route moved.

Switching to another Space on macOS frees a captured mouse, as losing the front does, and keys
pressed there no longer reach the game (#176, core 1.2.0). No route moved.

On macOS, quitting from the Dock, logging out or restarting closes it as the window's button does
(#178, core 1.2.0), where the process ended at once; a logout or restart is interrupted, and asked
again once the title has left. No route moved.

A long play on macOS no longer grows the process's memory (#180, core 1.2.0): what macOS hands
back for a while is let go every frame. No route moved.

Declares no floppy drive (#145), core's new required `Declared::floppy` (unreleased; core's next
release is 2.0.0, #163), which is the machine it has always had: C: and D: only. Nothing it sees
moves.

## 1.0.0 — 2026-10-18

Playable. A person played the first mission through on 2026-09-10, on an older build, and
`routes/first-mission.route` covers its first minute.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: to the level
select. A route through the whole first mission is #106.
