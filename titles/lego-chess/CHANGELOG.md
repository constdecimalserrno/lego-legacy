# Changelog: lego-chess

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-10-21

On the roster (#143): `Lego Chess.exe`, V2.06, out of the Masterpiece pressing's `MAIN.Z`, an
InstallShield 3 archive the declared tree expands whole into `C:\Program Files\LEGO Media\Games\LEGO
Chess`, with the game's own strings, `Text.txt`, copied out of `_SETUP.LIB` beside it. A Minimum
install: the three `.RES` archives stay on the disc and are read where they lie. The declared
machine is the roster's baseline: Windows 98 SE, 640x480 at sixteen bits, a flip on a sixty-hertz
refresh, and the baseline processor until #147 measured the game's clock waits. Its CD answers
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
that, and finds `CDFS` and `LEGOCHESS` on D:. The game's main calls `GetSystemDefaultLangID`
before the check at 0x4087AF, so the first run past that call (#147) is the first to reach it. Until then core's shim tests stood
in: they make the check's calls as its code at 0x405420 makes them and find `D:\`. The rip's label
is tested to be `LEGOCHESS`, the label the check compares.

A member of `MAIN.Z` that will not expand, or expands to another size than its table gives, stops
the run at a wall naming it (#169, core 1.2.0), where the title was told the file was missing, and a
damaged KLIB plugin stopped at #146's wall, which said it was not the title's own. All 881 members
of a good rip expand, so the run is unchanged.

Its framework asks where the pointer is confined as it starts (#147), and is told the whole screen,
as Windows tells a program whose pointer is confined nowhere; it keeps the rectangle to hand back to
`ClipCursor` as it leaves. What `ClipCursor` is given is now kept and read back, as Windows does.
It goes on to its DirectInput devices and its sound, and the run stopped at #149's first wall, the
3D listener's `SetAllParameters`, called from 0x0044FE17 at cursor 214,521, still with nothing
presented. Its boot test now runs to the wall it stops at, on a budget of a million instructions
where it was a hundred thousand.

Its input and sound come up (#149). Its keyboard and mouse are set to the foreground, not
exclusively, and acquired, as the machine already answered. Its listener is set all at once, and
the CD audio device it opens by number opens with its data disc in the drive, as Windows' does, and
counts in tracks; it will be stopped and closed as the game leaves. Its music's stream will be
refused for want of a MIDI port, which it takes as no music, until #154. Its plugin walk then loads
the DirectX plugin, which asks DirectDraw for Direct3D 2 and, refused, calls through the pointer it
was not given: an access violation at 0x400014E1 that nothing took, so the run stopped at
`KERNEL32.dll!UnhandledExceptionFilter` at cursor 230,008.

DirectDraw now hands its DirectX plugin Direct3D 2 (#148), and the plugin lists the machine's device
from it. The walk goes on through all four plugins: the Glide plugin finds no `glide2x.dll`, the
Software plugin lists its own driver, and the OpenGL plugin is not there. The game's main starts,
takes its single-instance mutex, and stopped at `KERNEL32.dll!GetSystemDefaultLangID`, called from
0x004086E6 at cursor 260,309.

Its main goes on (#147): it is told the system's language is US English, finds its disc on D: with
no word to the player, which is #145's check passing at 0x004087AF, is told no screen saver is
enabled, so turns none off, and reads its three `.RES` archives from `D:\`. Its video setup's own
auto-select picks the DirectX plugin's Direct3D 2 device, as on a 1998 PC with a 3D card, takes the
screen at 640x480x16 and makes the device, and the run stopped at
`IDirect3DDevice2::EnumTextureFormats` at cursor 374,735.

The declared processor is a Pentium 166 (#147), family 5, where it was the roster's Pentium II 450:
the minimum the original pressing's 1998 UK manual gives, IB2G-CHE3, page 5, "CPU Pentium 166 MHz"
(archive.org item `LEGOChess-manual-EU-IB2G-CHE3`), and its later boxes repeat. The Masterpiece's own
manual and the 2002 US edition have no requirements page. The game waits by spinning on the clock,
half a second and then a second at boot and between its movies' frames, so the processor decides
what a second of waiting costs. Measured on a probe that skipped the movies: the two boot waits are
675 million instructions at 450 MHz, nine tenths of the boot, and 249 million at 166, and the boot
to its MIDI open took 4 seconds of the Mac mini's wall time where it took 8. No committed run reaches
the waits yet; their pin comes with #151, the first run to reach them.

Its DirectX plugin now lists the device's texture formats and makes it a viewport (#148), and its
framework takes the screen (#147): it confines the pointer to it, says a screen saver is running, as
programs of the time did to keep Alt+Tab away (Microsoft's Q161133, cited in core), and tells its own
window its size with `SendMessageA(WM_SIZE)`, where the run stopped, at cursor 376,609. Core's
`IDirectDraw2` is the object's `IDirectDraw4` face, whose `EnumDisplayModes` hands out DirectX 6's
124-byte `DDSURFACEDESC2` where DirectX 5 hands a 108-byte `DDSURFACEDESC`; the plugin lists modes
only through DirectDraw's first face, so nothing reaches that yet.

Its window is told its size, and is painted once its framework invalidates every window after
setting the mode (#147). The game's first movie clears the screen twice, its first two flips, and
its message pump takes what waits: the window is the active application's, is active and has the
keyboard (#196), which the pump waited for, and is painted; its window procedure acquires its
keyboard and mouse again as it is told it is in front. The game opens `intro\lmi.avi`, and the run
stopped at #151's first wall, `AVIFIL32.dll!AVIFileOpenA`, at cursor 767,882 after two flips.

Its first movie, the logo, opens through AVIFile (#151, core 1.2.0). The game reads its two streams'
headers, a video of 280 frames at 320x240 and 15 a second and its sound, PCM at 22,050 Hz in 8-bit
mono, reads the sound's format and makes its DirectSound buffer for it, and the run stops at #151's
next wall, `AVIFIL32.dll!AVIStreamGetFrameOpen`, called from 0x0044B325 at cursor 768,839 after two
flips.
