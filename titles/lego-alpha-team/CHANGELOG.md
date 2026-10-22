# Changelog: lego-alpha-team

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-18

Took core 1.1.0, which the Windows (#107) and Linux (#108) backends joined, written but untested on those systems. No route moved.

Core's `GetVersion` answers as Windows 98 SE did, with the top bit that says it is not NT (#129).
The title's runtime only stores the word, and what it and its engine read of `GetVersionExA` is
unchanged, so no route takes a different turn, and every run's digest moves; core's changelog
lists them.

Core's `RaiseException` puts `EFlags` and `Esp` in the context where `winnt.h` has them (#133).
No handler on the title's chain reads its C++ throws' context, so no route takes a different
turn, and the digest of every run that throws moves; core's changelog lists them.

Took core 1.2.0 (#140), in which closing its window tells it, as Windows does, where the run
stopped at once without telling it: at its menu or in a mission it posts its quit, writes
`Game.txt`, shuts NetImmerse down and exits 0. A second close stops the run at once. No committed
route closes, and the title never destroys its window in one, so no route moved.

Its declared machine lacks no library (`absent_libraries`, #146): in every gate run it asks `LoadLibraryA` only for libraries it ships and the two of Microsoft's core writes, so no route moves.

Took core 1.2.0's mouse (#141): a player clicks on the picture to give the title the mouse, and
Escape frees it, neither reaching the game. No route records either, so no route moved.

Declares core 1.2.0's `joliet` as `false` (#144), by #142's rule: Joliet is LEGO Chess's alone,
because this title's runs move with it, until the maintainer decides. Its disc is read by the
primary tree's names alone, as before, so no route moved. Read through its Joliet tree, as Windows
98 SE's CDFS reads it, the disc also answers `D:\Strings_EN.txt`, which the primary tree calls
`STR_0012.TXT` and the title's `MSVCRT.dll` opens at boot, and every route moves, and every boot
run past 2,000,000 instructions: `first-mission.route` ends at 8,053 flips and 331,937 declared
milliseconds where it ends at 8,054 and 331,967. Core's changelog lists each run. Core's
`FindFirstFileA` on the CD is now a wall; this title searches only relative to its install
directory on C:, and no route moves.

Minimizing its window on macOS, or hiding the application, no longer closes it (#173, core 1.2.0):
it runs on unseen and is shown again on restore. No route moved.

## 1.0.0 — 2026-10-18

Playable. `routes/first-mission.route` plays the Training Area to its end and back to the main
menu, taken at the pace of the coach's narration (#61, #92).

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: the Training
Area.
