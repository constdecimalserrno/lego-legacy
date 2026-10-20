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

## 1.0.0 — 2026-10-18

Playable. A person played the first mission through on 2026-09-10, on an older build, and
`routes/first-mission.route` covers its first minute.

Signed off 2026-10-18 on `c770a73`, on the release build, played by the maintainer: to the level
select. A route through the whole first mission is #106.
