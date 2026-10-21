# Changelog: lego-legacy-launcher

One heading per version, newest first. What each part of the launcher's version means is in
[ADR-0007](../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.1.0 — 2026-10-21

It lists and starts four titles, the fourth LEGO Island, playable at its 1.0.0, and runs on core
1.1.0, whose Windows and Linux backends are run by machine and not yet played by a person.

Took core 1.1.0, which the Windows (#107) and Linux (#108) backends joined, written but untested on those systems, and finds a title's `<title>.exe` beside itself on Windows.

Lists LEGO Island as the fourth title and starts it, as `lego-legacy lego-island`, when its rip is
there (#113).

## 1.0.0 — 2026-10-18

`lego-legacy`, the one front door. With no arguments it lists the three titles, says for each
whether its rip is here and is the right one, or is wrapped, damaged, the wrong bytes or missing,
and where its music comes from, then starts the one picked. Given a title's name, it starts that
title's own binary with the flags it was given.
