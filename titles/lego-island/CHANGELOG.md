# Changelog: lego-island

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-10-19

On the roster (#113): `ISLE.EXE` from `DATA/disk`, copied whole as the Wise installer copies it,
on a declared machine of its own. Untagged until its release (#128).

`ISLE.EXE` opens its window (#114): it turns mouse trails off, makes a window with a caption,
asks `WIN.INI` for a media path it does not have, and its engine starts its sound and asks
DirectDraw for Direct3D 2 at instruction 432,045, which is #117's. The boot tests run to there.
