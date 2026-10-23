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
mono, reads the sound's format and makes its DirectSound buffer for it, and the run stopped at #151's
next wall, `AVIFIL32.dll!AVIStreamGetFrameOpen`, called from 0x0044B325 at cursor 768,839 after two
flips.

It takes the logo's frames as it asks for them (#151, core 1.2.0), each a 16-bit DIB of 320x240,
and hands the first to its DirectX plugin, which takes its back buffer's DC to draw it on. The run
stopped at #151's next wall, `MSVFW32.dll!DrawDibOpen`, called from 0x4000408D at cursor 768,882
after two flips.

Its DirectX plugin draws the logo's first frame doubled onto its back buffer, 640x480 from the
frame's 320x240, through DrawDib on the buffer's own DC, and lets the DC go (#151, core 1.2.0). The
run stopped at #150's wall, `AVIFIL32.dll!AVIStreamStart`, called from 0x0044B3A1 at cursor 768,941,
as the game starts the logo's sound, still after two flips.

First light (#150, core 1.2.0). The logo's sound starts with its first frame: the game reads its
start and length, puts eleven chunks of it, 1,470 samples each, into its DirectSound buffer and
plays the buffer looping, then flips. The logo plays a frame a flip, and the first frame that is not
one colour is flip 7, at cursor 45,093,881: the logo's frame 5, the first sliver of a brick at the
top of the screen, 52 greys no brighter than #212121 in x 102 to 117 and y 0 to 5, black to the eye.
It is the bytes predicted from FFmpeg's planes of the same picture, and the old tree's default-boot
first light byte for byte. Before it come the movie player's two clears, two flips the DirectX
plugin fills black whatever was drawn, and the logo's own black frames 2 to 4. The brick a person
first notices comes at flip 10. The old tree's first light, a chessboard splash, came from refusing
the movie; here the movies play, and the splash is the bumper after the intro.
`routes/first-light.route` is committed with its determinism gate, two replays side by side to flip
10 and a frame more, 84 million instructions. The boot test now ends by its budget inside the logo's
first frame. A run with no route went on: the logo played its 280 frames to the end, and the run
stopped at #151's next wall, `AVIFIL32.dll!AVIStreamGetFrameClose`, called from 0x0044B605 at cursor
2,582,262,607 after 282 flips.

Its intro plays (#151, core 1.2.0). As each movie ends its streams and frame getter are let go of,
its file staying open as the game never releases one, and the next opens: the logo, flips 0 to 283,
its 280 frames between the player's two clears either side; Krisalis' logo, flips 284 to 325, its 38
frames with no sound, held two seconds after; and the intro, 320x200 doubled to 640x400 at y 40, its
499 frames with their sound, flips 328 to 826. `routes/intro.route` plays the first two through and
the intro's first dozen frames, 3.7 billion instructions, with its determinism gate. It ends there
for two reasons, which its comment gives: every gate replays a committed route twice, and the lead
holds this title's routes to about four billion instructions a run for that, where the intro played
whole is 8.6 billion; and it cannot skip the intro, since the game's skip is Space, and after a
movie it waits for Space to come up in a loop that presents no frame (0x0040D220), while a key comes
up only at a flip, a route's or a person's. So a person who presses Space to skip a movie at a
window hangs the game there for now; #215 is to deliver what a person does between flips, and then a
route can skip with Space. A run with no route goes on: the intro plays to its end and the run stops
in the bumper, at `IDirect3D2::CreateMaterial`, called from 0x400027A9 at cursor 8,613,581,999 after
829 flips, #210's. Headless and untraced on the Mac mini, the logo played in real time, with 0.7% to
spare: its frames take 9.2 million instructions each, 15.1 frames a second at 139 million a second,
against its 15. The intro did not: its frames take 10.1 million each, 13.7 frames a second at 138
million a second, 0.91 of real time. The MacBook is not measured.

Space skips its movies (#215, core 1.2.0). The game ends a movie it lets be skipped, Krisalis' logo,
the intro and the story movies, when Space is down at its check after a frame's flip (0x0044B47C),
and then waits for Space and the left button to come up (0x0040D220), presenting nothing, where it
waited for ever, since a key came up only at a flip. The wait reads its pump, its keyboard and its
mouse each turn, and core now delivers what a person does at a title's own polls between flips, so
Space let go there is seen. `routes/skip-krisalis.route` plays the logo through, presses Space at
Krisalis' logo's fifth frame, flip 290, and lets it go at the 300th poll after it, in the wait's
98th turn: Krisalis' two-second hold is over at once, the game goes on, and the intro opens after
flip 294, where with Krisalis' logo played through it opened after flip 327. Its frame 10, flip
305, is byte for byte the frame `intro.route` presents at flip 338. The route ends there, at 2,975
million instructions, with its determinism gate. At a window, Space should skip a movie without
hanging the game once it is let go and the frame has stood core's `ASK_BETWEEN_FLIPS_MILLIS`, which
a person has yet to try on the MacBook. By their code, its 16 waits for an arrow key to come up (0x457200) read
the keyboard the same way, so a key let go between flips reaches them too.

Its movies' clock spins are passed (#217, core 1.2.0). Between frames the movie player reads
`QueryPerformanceCounter` from 0x0044B57D, 38 instructions a pass and about 4,400 passes a declared
millisecond: 99.996 % of a frame's instructions. The machine retires the passes that would read the
same millisecond at once.
- Headless on the MacBook, from its kit, the intro had played at 0.72 to 0.80 of real time, and at
  its window at 0.55 to 0.76, its music stretched.
- On the Mac mini the logo and the intro now play at about 40 times real time. The whole intro, 58
  declared seconds, runs in 7.1 s, against 67.6.
- It lands on the same machine. `intro.route` ends on main's digest, with every flip at its cursor,
  the same mix and each block of it handed over at the same flip, as its new test pins them. A run
  with no route still stops in the bumper at cursor 8,613,581,999 after 829 flips.
  `skip-krisalis.route` played to its wall stops in the bumper too, at cursor 7,908,916,035 after
  796 flips, on main's digest, in 4.0 s.
- The boot's two waits and Krisalis' hold poll for input as they wait, so they are not passed, and
  run at about 0.65 of real time on the Mac mini. No sound plays through them. At a window core now
  takes the pace as real time at the movie's first hold that has to wait, where it stretched up to
  half a second of the movie's sound by the holds' pace.

Its bumper presents (#210, core 1.2.0). Past the intro the game loads `intro\Chess Bumper01.bmp`
and `02.bmp` off the disc into two 256x256 textures, which its DirectX plugin makes in system memory
over the game's own pixels and loads into copies in video memory, and draws both, the LEGO logo and
the word chess on its chequerboard, on white, through Direct3D 2: flips 829 and 830 on
a run with no route. The run then stops at `KERNEL32.dll!UnhandledExceptionFilter`, called from
0x0047562D at cursor 8,645,403,111 after 831 flips: the game reads through a null pointer at
0x0045672F, having found none of the room's files. It looks for them by relative names in
directories it sets by relative names too, which the machine does not yet resolve against its
current directory, and at boot it sets an empty one (`SetCurrentDirectoryA` from 0x0045691B at
cursor 260,327), which Windows refuses and the machine takes (#171).

Its strings load, and its room's files are found (#171, core 1.2.0). At boot its main restores its
saved directory before it has saved it, `SetCurrentDirectoryA` of an empty name from 0x0045691B at
cursor 260,327, which now fails as Windows NT fails it, Windows 98's answer being unmeasured: the
directory stays where the game was installed, and `text.txt` is found beside it, where every run
before looked for it at the root and missed. Every run moves by 23,170 instructions from there, its
flips unchanged: first light is flip 7 at cursor 45,117,051, the brick flip 10 at 72,784,709, the
intro route's flip 340 at 3,695,193,660, and `skip-krisalis.route`'s flip 291 at 2,706,399,675 and
its flip 305 at 2,971,352,071; the test of the intro's clock spins (#217) pins the intro route's new
digest and flip lines, its mix unchanged. Past the bumper the game sets its own folders by name,
`profiles`, `sounds` and `menus\room`, each now its install directory's child, which its install
holds, and loads what is in them. The run stops at #153's wall, `MSACM32.dll!acmStreamOpen`, called
from 0x0044FA83 at cursor 8,616,780,716 after 831 flips, as the game decodes `sounds\chimp.wav`, an
MS ADPCM sound.

Its sounds decode (#153, core 1.2.0): its sample loader (0x0044F950) opens an ACM stream from each
MS ADPCM sound to PCM, sizes it, and decodes it, sixteen bits a sample, which
`MSACM32.dll!acmStreamOpen` stopped at; a sound of more than half a megabyte it decodes to eight,
which is a wall. On the way to its room it decodes fifteen, twelve at 11,025 Hz and three at 22,050,
all in one channel, to sixteen bits, each to its end. The run no longer stops: past the bumper, on a
run with no route, it presents the New Player form's buttons and boxes on black at flip 832, at
cursor 8,729,002,651, and from flip 833 its room with the form on the desk, a third of whose letters
do not yet draw (#210), and waits there for a player, flipping.
