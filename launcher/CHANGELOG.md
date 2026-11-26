# Changelog: lego-legacy-launcher

One heading per version, newest first. What each part of the launcher's version means is in
[ADR-0007](../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.2.0 — 2026-10-21

Lists LEGO Racers as the sixth title via `core::roster::ROSTER` and starts it as `lego-legacy lego-racers` when its `masterpiece-racers` rip is present (#369).

Took core 1.2.0: its window changes (#140) change nothing the launcher does, since it uses core
only to find and check the rips and starts each title as a program of its own, and its roster
lists LEGO Chess (#143).

Lists LEGO Chess as the fifth title and starts it, as `lego-legacy lego-chess`, when its rip is
there (#143). It is in progress: it stops at a wall long before it is playable.

Says where LEGO Chess's music comes from (#156), as it says where Rock Raiders' does (ADR-0006):
"music is played with GM16.DLS, from the masterpiece-racers pressing", the first of LEGO Racers' and
Rock Raiders' pressings that holds it; or that the music cannot be played, and why: what a rip that is
there held instead, or that none of the four is ripped. The title starts either way.

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
