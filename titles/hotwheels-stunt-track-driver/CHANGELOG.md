# Changelog: hotwheels-stunt-track-driver

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-18

Took core 1.1.0, which the Windows (#107) and Linux (#108) backends joined, written but untested on those systems. No route moved.

Core's `GetVersion` answers as Windows 98 SE did, with the top bit that says it is not NT (#129).
The title's C runtime only stores the word, so no route takes a different turn, and every run's
digest moves; core's changelog lists them.

## 1.0.0 — 2026-10-18

Playable. `routes/first-race.route` drives the first race to the finish.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: the first
race, driven to the finish.
