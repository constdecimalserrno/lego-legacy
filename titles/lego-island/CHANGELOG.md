# Changelog: lego-island

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-10-19

On the roster (#113): `ISLE.EXE` from `DATA/disk`, copied whole as the Wise installer copies it,
on a declared machine of its own. Untagged until its release (#128).

`ISLE.EXE` opens its window (#114): it turns mouse trails off, makes a window with a caption,
asks `WIN.INI` for a media path it does not have, and its engine starts its sound and asks
DirectDraw for Direct3D 2 at instruction 432,045, which is #117's.

Its engine makes its Direct3D 2 device and hands it to retained mode (#117): it enumerates the
declared machine's one device, sets the screen to 640x480x16, makes a device over its back buffer,
then DirectX 5's retained mode over that, with frames, a viewport the size of the screen and three
lights. At instruction 478,302 it asks its window for the instance it was made with, which is
#118's. The boot tests run to 478,000.

Core's `GetVersion` answers as Windows 95 did, with the top bit that says it is not NT (#129).
Both C runtimes only store the word, so the boot calls what it called, and its digest moves.
SmartHeap's test for NT in `ISLE.EXE` and `LEGO1.DLL`, not yet reached, now finds Windows 95.
