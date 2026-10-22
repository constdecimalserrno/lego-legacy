# Changelog: lego-chess

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-10-21

On the roster (#143): `Lego Chess.exe`, V2.06, out of the Masterpiece pressing's `MAIN.Z`, an
InstallShield 3 archive the declared tree expands whole into `C:\Program Files\LEGO Media\Games\LEGO
Chess`, with the game's own strings, `Text.txt`, copied out of `_SETUP.LIB` beside it. A Minimum
install: the three `.RES` archives stay on the disc and are read where they lie. The declared
machine is the roster's baseline: Windows 98 SE, 640x480 at sixteen bits, a flip on a sixty-hertz
refresh, and the baseline processor until #147 measures the game's clock waits. Its CD answers
long names from its Joliet tree (#144), since its bumper, `D:\intro\Chess Bumper01.bmp`, is named
only there. It has no `glide2x.dll`, for the machine has no 3dfx card, and no
`KLIB_OpenGL_PlugIn.dll`, which no pressing shipped: each is told it is not there, as Windows 9x
tells it (#146).

Started, its C runtime runs, its framework registers its class and makes its window, and the run
stopped at its first wall, `USER32.dll!GetClipCursor`, called from 0x0045AC3C at cursor 15,940 with
nothing presented, until #147 answered it; where it stops now is in the newest paragraphs below.
Untagged until its release (#162).

The declared machine has a floppy drive, A:, with no disk in it (#145), as a 1998 PC had. The game's
CD check (0x405420) tests bit 0 of `GetLogicalDrives` once and never shifts, so with no A: it looked
at no drive and would ask for its disc for ever; with one it asks every letter its type, A: only
that, and finds `CDFS` and `LEGOCHESS` on D:. The run stops before the check, and the boot reaches
its first DirectDraw call before the game calls the check at 0x4087AF, so #148's run is the first
to reach it. Until then core's shim tests stand
in: they make the check's calls as its code at 0x405420 makes them and find `D:\`. The rip's label
is tested to be `LEGOCHESS`, the label the check compares.

A member of `MAIN.Z` that will not expand, or expands to another size than its table gives, stops
the run at a wall naming it (#169, core 1.2.0), where the title was told the file was missing, and a
damaged KLIB plugin stopped at #146's wall, which said it was not the title's own. All 881 members
of a good rip expand, so the run is unchanged.

Its framework asks where the pointer is confined as it starts (#147), and is told the whole screen,
as Windows tells a program whose pointer is confined nowhere; it keeps the rectangle to hand back to
`ClipCursor` as it leaves. What `ClipCursor` is given is now kept and read back, as Windows does. It goes on to its DirectInput devices and its sound, and
the run stops at #149's first wall, the 3D listener's `SetAllParameters`, called from 0x0044FE17 at
cursor 214,521, still with nothing presented. Its boot test now runs to that wall, on a budget of a
million instructions where it was a hundred thousand.
