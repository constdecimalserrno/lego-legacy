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

## 1.0.0 — 2026-10-18

Playable. `routes/first-mission.route` plays the Training Area to its end and back to the main
menu, taken at the pace of the coach's narration (#61, #92).

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: the Training
Area.
