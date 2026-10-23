# Changelog: lego-legacy-core

One heading per version, newest first. What each part of core's version means is in
[ADR-0007](../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.2.0 — 2026-10-21

What a person does reaches a title between flips too, at its own input polls (#215), where it
reached it only at a flip. A poll is a call that reads the keyboard, the mouse or the message queue,
and every reader core has written is one: `PeekMessageA`, `GetMessageA`, `WaitMessage`,
`GetCursorPos`, `GetAsyncKeyState`, and DirectInput's `GetDeviceState` and `GetDeviceData`, on every
thread and whatever they filter, each counted from the flip before it; a test fails on any USER32 or
DirectInput call named neither a poll nor not one, so a reader written later is classed as it is
written. A route keys an event between
flips as `flip N poll K`, delivered as the guest makes its Kth poll after flip N, before the poll
answers, or at the first point after it if the guest makes fewer. An event with no `poll` is the
flip's own, as every route's are, so every route reads as it did, and an older build refuses a
`poll` line, naming the line. A run at a window asks the host at every flip, as before, and between
flips at the first poll once the frame has stood `ASK_BETWEEN_FLIPS_MILLIS` of declared time, its
waits included, since the flip or since the host was last asked; it holds the pace there, as at a
flip, and records what it heard at that poll. A run replaying a route drains the host there and
drops what it has, as at a flip. The released titles meet the threshold only where a frame waits or
loads: Rock Raiders in the frame that ends a load and in its boot splash, Hot Wheels in its boot and
its race's finish; Alpha Team and LEGO Island poll at the start of their frames, before they wait.
An ask is made before the host is told of a pointer the title put earlier in that frame, so a title
that puts its pointer back every frame and polls a threshold into one would lose the move heard
there: a ceiling marked in `Machine::poll`, which Alpha Team alone of the released titles could meet,
in a load that polls late, and which is to be lifted before any shorter threshold. The headless host
has nothing to hand over between flips, its events for a flip going at the flip, and holds no pace,
and the polls are counted outside the digest, so no committed route moves: the full gate's lines for
the released titles are the last full gate's. `route undelivered` counts the events after the last
flip or poll a run reached, so a run that presents nothing counts its first flip's too. The polls
cost Hot Wheels' first race, which in its menus polls every seventy-odd instructions, 0.18% more of
the host's instructions on the Mac mini, 2,453.46 billion where it took 2,449.16. LEGO Chess,
which after a movie Space skipped waits for Space to come up and presents nothing while it waits,
gets past the wait. ADR-0003 says how input is delivered and keyed, and CONTEXT.md has **Poll**.

`AVIStreamGetFrameClose` lets a frame getter go, its DIB and its hold on its stream with it,
answering the references it has left, as Wine's `api.c` releases it, a getter holding its stream
from its open as Wine's `getframe.c` holds it; `AVIStreamRelease` counts a stream's reference out,
the file keeping the stream so that asking for it again hands out the same one, and one released
with none left answers -1 and is still there, as Wine's conformance test `test_default_data` finds
it on Windows; and `AVIFileExit` answers as `AVIFileInit` does (#151). All three were walls. LEGO
Chess lets go of each movie's streams and frame getter this way, its file staying open as the game
never releases one, and its intro's three movies play one after another. Rock Raiders imports
`AVIStreamRelease` and reaches a stream only through `AVIStreamOpenFromFileA`, still a wall; no
released title imports the other two; so no released run moves.

`AVIStreamStart` and `AVIStreamLength` answer a stream's start and length, and `AVIStreamRead` reads
a stream of fixed-size samples, a sound's, across its chunks, as Wine's `avifile.c`, read as
documentation, reads one, its samples its format's blocks as `AVIStreamInfoA` says; all three were
walls (#150). Both counts it writes are cleared first; a start before the stream or past its end is
`AVIERR_NODATA`; what is asked for stops at the stream's end; -1 samples is as many as fit, and with
no room is `AVIERR_BUFFERTOOSMALL`; no buffer is the counts alone; and what does not fit is
`AVIERR_BUFFERTOOSMALL`. A stream of whole chunks, a video's, is a wall, since no title reads one,
and so are a count below -1 and a stream whose chunks hold less than its header says, a damaged rip.
LEGO Chess fills its movies' DirectSound buffers through it, and with it reaches first light. No
released title imports any of the three, so no released run moves.

A DirectDraw surface's `GetDC` hands out a device context of its own, where it answered success and
wrote nothing, and `ReleaseDC` lets it go (#151): one at a time, whichever face of the surface is
asked, a second before the first is let go of answering `DDERR_DCALREADYCREATED` and leaving what it
was handed alone, as ddraw.h's error says and Wine's `surface.c`, read as documentation, answers; a
surface with none out answers `DDERR_NODC`, and one handed another DC than its own is a wall. With
its DC out a surface is locked, as Microsoft's GetDC page says: `Lock` answers `DDERR_SURFACEBUSY`,
as Wine's conformance test `test_getdc` finds Windows answering, and `Blt`, `BltFast` and `Flip`,
which nothing has observed, are walls. A screen with nothing behind it has its frame when its DC is
let go of, as letting go of a lock of the whole of it does. What draws on a DC is DrawDib:
`DrawDibOpen` hands out handles counted up from one, as Wine's `drawdib.c` counts them, so one let
go of is not handed out again; `DrawDibClose` answers TRUE, or FALSE for a handle it never gave; and
`DrawDibDraw` draws a plain bottom-up 16-bit DIB whole onto a surface's DC, each pixel repeated as
many times over as the destination is the DIB's size, from the bottom row up, and what falls off the
surface left out. Each channel is widened by repeating its top bits and narrowed to the surface's,
as Wine's DIB engine does, so a 555 DIB's green gets its sixth bit on a 565 surface from its own top
one. A top-down or empty DIB is refused with FALSE, as Microsoft's DrawDibDraw page says it "will
not draw inverted DIBs" and Wine's `test_DrawDib_sizeimage` finds an empty one refused; a flag, a
part of a DIB, a scale that is not whole, a codec, another depth or an indexed surface is a wall.
LEGO Chess's DirectX plugin draws each movie frame this way. No released title imports MSVFW32, and
no released title takes a surface's DC on a committed route: the full gate on this change moved none
of the released titles' runs.

LEGO Chess's movies give up their frames (#151). `AVIStreamGetFrameOpen` makes a getter of a video
stream's frames as a plain bottom-up DIB in the format the title asks for, 16 bits a pixel, which is
555, for its movies, or 24 for its Advisors (#152), at the movie's own size, the header asked for
followed by its bits; a stream that is not video gets none, as Wine's `getframe.c`, read as
documentation, refuses one, and the decompressor's own format, another depth or size, a colour
table, or a codec other than Indeo 5 is a wall, as is a picture too big for memory, which only a
damaged rip's header could say. `AVIStreamGetFrame` decodes the pictures in order up to the one
asked for, a null picture keeping the last, and answers the DIB with it in: the same frame again is
the DIB as it is, one before the last decoded starts again from the first, and one before the
stream's start or past its last is none. LEGO Chess's logo's frames 5 and 8 are the DIBs a reference
decoder's pictures give, FFmpeg 9.0.1's, turned into DIBs the same way, as the test beside them
records. Every open of a movie file shares one copy of its bytes, since the game opens each movie
anew every time it plays it and never lets go of a file. The conversion from YUV to RGB, BT.601 in
the integer form Microsoft documents and not FFmpeg's, moved from DirectShow's movies into `iv50.rs`
as `Picture::rgb`, unchanged, and both now call it; NOTICE says it is this project's own. Rock
Raiders imports both calls and reaches a stream only through `AVIStreamOpenFromFileA`, still a wall,
so no released run moves; its movies' tests and its first-light route end as they did.

LEGO Chess's movies open through AVIFile's C API (#151), which past `AVIFileInit` was a wall.
`AVIFileOpenA` opens a movie to read it: one that is not there, or on the empty floppy drive, gives
`AVIERR_FILEOPEN`, which LEGO Chess takes as a movie to skip; one that is there and is not a movie,
or holds fewer streams than its header says, which Wine fails to load, stops the run at its name, as
a damaged archive member does (#169); one that holds more is read, as Wine reads it, as the streams
its header counts; another mode, or a handler named, is a wall. `AVIFileInfoA` answers the main
header's flags and stream count and works the rest out from the streams as Wine's
`AVIFILE_UpdateInfo`, read as documentation, works them out: the first stream's time scale, the
longest stream's length in it, the streams' bytes a second added up, the biggest chunk as the
buffer, and the video's biggest frame as the size. `AVIFileGetStream` hands out a stream by number,
or by number of its type, the same object each time with a reference more, and `AVIERR_NODATA` past
the last. `AVIStreamInfoA` answers the stream's header as vfw.h's 56-byte `AVIStreamHeader` lays it
out but for three things it answers as Windows does: a video stream's empty frame is filled from its
picture's size, as Wine fills it, which twenty of LEGO Chess's twenty-two movies need, their
headers' 32-bit edges being empty read as 16-bit ones; the buffer is the stream's biggest chunk, and
none for none, as Wine's conformance test `test_default_data` finds Windows answering; and a sound's
sample size is its format's `nBlockAlign`, whatever its header says, as `test_ash1_corruption` finds
it. A sound's time scale is its header's, which Wine changes and no test observes Windows changing.
A stream is named by its `strn`, as its bytes, or as AVIFile names one with none, by the file's
name, its kind and its number of that kind, as two of LEGO Chess's movies kept the names their
authors' AVIFile gave them, `LMI.avi Video #1` and `WL1lose.avi Audio #1`. `AVIStreamAddRef` counts
a reference, and `AVIStreamReadFormat` answers how long the format is, or copies as much as fits and
says `AVIERR_BUFFERTOOSMALL` when that is not all of it, as Wine's does. The file and its streams
are COM objects whose own tables are walls. `avi.rs` reads the rest of each stream's header, its
name and the main header's flags and stream count, and what it read before is unchanged. Rock
Raiders imports `AVIStreamInfoA` and reaches a stream only through `AVIStreamOpenFromFileA`, still a
wall, so no released run moves.

A message sent to the title's own window is handed to its procedure, whose answer is
`SendMessageA`'s, as Microsoft documents a send to a window of the calling thread; it was a wall
(#147). `InvalidateRect` of the title's window, or of no window, which on this machine is the same
window, marks it with its background to erase, the erase forced with no window as Microsoft and Wine
say; a rectangle of its own is a wall. A marked window is handed `WM_PAINT` by `PeekMessageA` and
`GetMessageA` once nothing else waits, in `GetMessage`'s documented order, and sent it by
`UpdateWindow`, until `BeginPaint` unmarks it and says to draw the whole client area, its background
still to erase if that was asked for, or `DefWindowProcA` is handed the `WM_PAINT`, which validates
the region as Microsoft's WM_PAINT page says. `BeginPaint` with nothing due says nothing is to be
drawn, and `EndPaint` answers success. `UpdateWindow` of no window, or of one the machine never
made, fails, as Wine's conformance tests record Windows 9x doing; the title's window, the desktop and
a dialog and its controls are the machine's. A send from a thread other than the window's, or to a
window with no procedure here, is a wall. `GetMessageA` takes what `PeekMessageA` would and answers
zero for `WM_QUIT` and -1 with nowhere to put it; with nothing waiting it is a wall, since a wait for
a timer's hour or another thread's post is not written yet. A paint due and a screen saver said to
run are part of the machine's digest when there is one. LEGO Chess makes all of these calls. No
released title invalidates its window, sends its own window a message, or calls `BeginPaint`,
`EndPaint` or `GetMessageA`; Rock Raiders calls `UpdateWindow` and every title calls `PeekMessageA`,
and with no window marked both answer as before, so no released run moves. Windows also activates
and paints a window as it is first shown; #203 is to do that.

`IDirect3DDevice2::EnumTextureFormats` lists the declared device's texture formats, the ones
`IDirect3DDevice7`'s lists, each in a whole `DDSURFACEDESC` as d3dtypes.h has its callback take one
and Wine fills it, and `IDirect3D2::CreateViewport` makes a viewport, DirectX 5's face being DirectX
6's slot for slot as far as it goes, as d3d.h declares them; both were walls (#148). The viewport
answers to DirectX 5's and DirectX 3's names too. With no callback, either device's
`EnumTextureFormats` is refused with `DDERR_INVALIDPARAMS`, and a material, light or viewport made
inside another object with `CLASS_E_NOAGGREGATION`, as Wine refuses them. LEGO Chess's DirectX plugin
calls both as it makes its device; no released title passes either refusal's argument, so no
released run moves.

A title's window is told it is the active window and has the keyboard as Windows tells it (#196):
after `WM_ACTIVATEAPP` come `WM_ACTIVATE` with `WA_ACTIVE` and no window deactivated, and then
`WM_SETFOCUS` with no window that lost it, in Microsoft's order, posted with it as the window is made.
Windows sends them from `ShowWindow`, or a visible `CreateWindowExA`, with `WM_NCACTIVATE` between the
first two; core posts them at `CreateWindowExA`, and #203 is to send them. LEGO Chess's message pump
waits for `WM_SETFOCUS` (0x0045C626), and on a probe it spun for ever without it. Every
released title's window procedure now hears two more messages, so every run of a released title that
makes its window moves, a move the maintainer blessed on 2026-10-22; each title's changelog says how,
and the runs are listed here:

- hotwheels-stunt-track-driver, `first-race.route` (exits): cursor 13,462,673,372 → 13,462,673,233, digest `ca24a96c` → `4fc367fd`;
- hotwheels-stunt-track-driver, `hot-wheels-escape.route` (exits): cursor 583,424,086 → 583,424,170, digest `1448924f` → `929363be`;
- hotwheels-stunt-track-driver, a run with no route (held to 100,000): digest `c4eae7cb` → `bf20b397`;
- hotwheels-stunt-track-driver, a run with no route (held to 2,200,000,000): digest `cb56737b` → `638636af`;
- lego-alpha-team, `click-the-intro-300.route` (held to 600,000,000): digest `a71a75d7` → `f486059d`;
- lego-alpha-team, `click-the-intro-600.route` (held to 600,000,000): digest `00c2662d` → `2f818e98`;
- lego-alpha-team, `first-mission.route` (held to 23,400,000,000): flips 8,054 → 8,053, ms 331,967 → 331,938, digest `1b7d7349` → `dea8cb61`;
- lego-alpha-team, `menu.route` (held to 5,400,000,000): ms 139,471 → 139,469, digest `f34a24e8` → `ec35babc`;
- lego-alpha-team, a run with no route (held to 1,100,000,000): digest `1a08e772` → `da79732d`;
- lego-alpha-team, a run with no route (held to 2,000,000): digest `e37ef1e8` → `e8b61fb0`;
- lego-alpha-team, a run with no route (held to 600,000,000): digest `198706f8` → `80389ef1`;
- lego-alpha-team, `select-adventure.route` (held to 14,000,000,000): flips 4,454 → 4,455, ms 214,104 → 214,138, digest `87323b4d` → `9dbb0afd`;
- lego-island, `first-light.route` (held to 98,000,000): digest `e753e559` → `1295656d`;
- lego-island, `first-ride.route` (held to 6,350,000,000): digest `e08fb5c1` → `d4014002`;
- lego-island, `first-save.route` (held to 6,030,000,000): digest `cbc3eae1` → `b299dcd3`;
- lego-island, `first-save.route` (exits): cursor 13,071,129,606 → 13,071,129,733, digest `cce081c4` → `a8f05879`;
- lego-island, `information-center.route` (held to 3,965,000,000): digest `93ac795c` → `d74f0616`;
- lego-island, a run with no route (held to 320,000,000): digest `99ce5f30` → `8d7906a2`;
- lego-island, a run with no route (held to 4,490,000,000): digest `0851a9b9` → `9b73e5c5`;
- lego-island, a run with no route (held to 5,571,000): digest `da6e3642` → `a39bb4f2`;
- lego-island, a run with no route (held to 6,030,000,000): digest `cbc3eae1` → `b299dcd3`;
- lego-island, a run with no route (held to 6,067,000,000): digest `f108f2f8` → `4fc2975c`;
- lego-island, a run with no route (held to 6,350,000,000): digest `e08fb5c1` → `d4014002`;
- lego-island, a run with no route (exits): cursor 10,657,537,575 → 10,657,537,702, digest `3d447ba5` → `d65d776f`;
- lego-island, a run with no route (exits): cursor 4,998,298,712 → 4,998,298,839, digest `038ebb6b` → `895d407d`;
- lego-island, `pizza-delivery.route` (exits): cursor 10,657,537,575 → 10,657,537,702, digest `3d447ba5` → `d65d776f`;
- lego-island, `route` (exits): cursor 4,998,298,712 → 4,998,298,839, digest `038ebb6b` → `895d407d`;
- lego-rock-raiders, `click-the-intro.route` (held to 150,000,000): digest `b1c537af` → `0447048c`;
- lego-rock-raiders, `first-light.route` (held to 8,010,000,000): digest `322ab7a0` → `704487cb`;
- lego-rock-raiders, `first-mission.route` (held to 14,500,000,000): digest `4888818a` → `27d40ec6`;
- lego-rock-raiders, `level-select.route` (held to 8,035,000,000): digest `49e57d12` → `df1a6347`;
- lego-rock-raiders, a run with no route (held to 400,000,000): digest `6714ce40` → `e76c516d`;
- lego-rock-raiders, a run with no route (held to 61,000,000): digest `3e8b8a38` → `f53901b5`;
- lego-rock-raiders, `skip-the-intro.route` (held to 150,000,000): digest `315d3880` → `b0ab6356`
  (Rock Raiders' run held to 100,000 makes no window and is unchanged);
- lego-island, `maintainer-first-play.route`: replayed to 3,341,815,910 where it was held to 3,341,815,783, the same moment 127 instructions later, ending at flip 10,667 and 274,650 ms as before; digest `db7bd6b2` → `6b5eca98`.

LEGO Chess's run ends at the same wall, cursor and flips, its digest `52a34d88` → `7fcea288`: its
window's three messages wait in its queue, which it has not reached.

`GetSystemDefaultLangID` answers the declared locale, US English, as `GetUserDefaultLangID` does
(#147). `SystemParametersInfoA` answers two more actions, only as LEGO Chess asks them, anything
else being a wall: `SPI_GETSCREENSAVEACTIVE`, no, since the declared machine has no screen saver,
and fails with nowhere to say so, as Wine's `get_bool_entry` does; and `SPI_SETSCREENSAVERRUNNING`,
which Windows 9x keeps and programs of the time set to switch off Ctrl+Alt+Del and Alt+Tab
(Microsoft's Q161133), each setting handing back the one it replaced, which is [inferred]: Q161133
never reads it. `timeGetSystemTime`, which LEGO Chess was seen on a probe to wait on, hands
`timeGetTime`'s milliseconds back in an `MMTIME` that says `TIME_MS`; one smaller than twelve
bytes, or none, is a wall that names it. LEGO Chess calls all of them; no released title calls any,
so no released run moves.

DirectDraw's `IDirectDraw2` face, the one a DirectX 5 program holds and here the object's
`IDirectDraw4` face, hands out `IDirect3D2`, as any face of a COM object answers for any other and as
Wine's ddraw hands Direct3D 2 out from every DirectDraw face, where it refused it (#148). LEGO Chess's
DirectX plugin asks it for Direct3D 2 and calls `EnumDevices` through what it is given without
looking, so a refusal was an access violation in the plugin. LEGO Island asks the `IDirectDraw` face,
which already answered; no released title asks this one, so no released run moves.

The 3D listener's `SetAllParameters` sets every value at once from a `DS3DLISTENER`, as dsound.h
lays it out, dropping the velocity as `SetVelocity` does, where it was a wall (#149). Another size,
an apply that is neither immediate nor deferred, a distance factor not above zero, or a rolloff or
Doppler factor outside 0 to 10 is refused with `DSERR_INVALIDPARAM` and nothing kept, as DirectSound
documents. LEGO Chess sets its listener so at boot; Alpha Team's A3D (`a3dapi.dll` 0x1001FD34)
would, on a renderer none of its runs takes.

The `cdaudio` device opens for a title with no Red Book audio, its own data disc in the drive, as
Windows' mcicda opens it, where it was not ready: Wine's conformance tests, which run on Windows 9x
too, record the open succeeding with a data-only disc and with none (#149). Both faces of MCI share
the one open. `mciSendCommandA`, MCI's numeric face, opens the device by its type's number, the
first of its type only, sets its time format to tracks, minutes, seconds and frames, and stops and
closes it, each logged with its answer as the string face's are; any other command, device or
flag is a wall. Rock Raiders, the only released title that imports MCI, has Red Book audio, so its
open is the one it always was.

`midiStreamOpen` is refused with `MMSYSERR_BADDEVICEID`, the mapper included: the machine has no
MIDI port until #154 gives it one, and none after that whenever the player has no bank (ADR-0008).
`midiStreamOpen`'s own page lists that answer, and Wine's conformance tests record every Windows they
ran on answering it for the mapper with no port. A stream of other than one device, or with nowhere
to say which, is refused as invalid; each answer is logged. LEGO Island's `LEGO1.DLL` imports it,
behind a `midiOutGetNumDevs` that is still a wall, so no run reaches it.

No released run reaches any of the three, so none moves.

`GetClipCursor` answers where the pointer is confined, as Microsoft documents (#147): the
rectangle `ClipCursor` was last given, which it now keeps, cut to the screen, until it is given none,
or the declared screen, in the mode it is in, while it is confined nowhere; with nowhere to write it,
it fails. The host's pointer is not held to the rectangle; no title has been seen to need it. LEGO
Chess's framework asks as it starts. Only LEGO Chess imports `GetClipCursor`, and nothing reads what
`ClipCursor` keeps but it, so no released run moves.

`DestroyWindow` tells the window it destroys, as Microsoft documents, where it answered success and
told it nothing (#140). Its procedure is sent `WM_DESTROY` and then `WM_NCDESTROY`, whatever it
answers the first, through the program `call_each` writes, which no longer has to stop at a zero.
The window is then gone: the messages waiting for it go too, what a person does reaches no window,
`GetActiveWindow` answers none, and destroying it again, or destroying any other handle, fails with
`ERROR_INVALID_WINDOW_HANDLE`. `DefWindowProcA` destroys the window it is handed `WM_CLOSE` for, as
Microsoft documents; every other message is still answered zero. The messages Windows also sends a
window that is shown and in front as it is destroyed, hiding it and taking the front from it, are
not sent. Hot Wheels destroys its window as it exits, and its procedure hands both messages to
`DefWindowProcA`, so its two runs that exit move, each by 54 instructions, their flips, declared
milliseconds and exit code unchanged, a move the maintainer blessed on 2026-10-21:

- the escape route from 583,424,271 and `920d063e…` to 583,424,325 and `8bf567a9…`;
- `first-race.route` from 13,462,722,489 and `ca178965…` to 13,462,722,543 and `d48d8761…`.

LEGO Island's procedure does the same as it leaves after its credits, so each of its runs that
close themselves ends 58 instructions later, its flips unchanged: `first-save.route`, past its end,
at 13,071,129,606, and `pizza-delivery.route` at 10,657,537,575. No other title destroys its window
in a committed run.

Closing the window reaches the title as Windows delivers a close, and the run ends when the title
leaves (#140). It stopped the run at once, and the title was never told. This changes how every
title's run ends when a person closes it.

- Every way a person closes a run is counted, `Host::closes` where `Host::closed` said whether:
  the window's close button, Command-Q or -W, Control-Q, Alt-F4, and the run's terminal or
  console, by Ctrl-C, Ctrl-Break, `SIGTERM` or `SIGHUP`. A key held down repeating is one close.
- The first is delivered at the next flip, as everything a person does is, as `Input::Close`. A
  route records it as `flip N close`, so a replay closes where the person did. The title's window
  is told as Alt-F4 tells it: `WM_SYSCOMMAND` with `SC_CLOSE` is posted to it (Wine's `defwnd.c`,
  read as documentation). The close button sends the same from `DefWindowProc`, inside the
  title's own dispatch of the click.
- `DefWindowProcA` answers `SC_CLOSE` by sending the window `WM_CLOSE` and answering what the
  window answers, as Microsoft documents and `defwnd.c` does.
- `PostQuitMessage` posts `WM_QUIT` with its code, where it only said so in the log. The thread
  that asked is handed it after whatever was posted to it and before what a person did, whatever
  it filters for, as Wine's message server hands it out (`server/queue.c`). The digest hashes a
  quit asked for and not taken, only when there is one.
- The run ends with the title's own exit. A second close still stops it at once, wherever it is,
  as `stopped by the person`, as one close did before; so does a close before the title has a
  window to tell. A title told that has not left within `machine::CLOSE_GRACE_MILLIS`, five
  declared seconds, is stopped then, on the same instruction every time its route is replayed. A
  third Ctrl-C or signal is let be what it was, as the second was. A closing Windows console,
  which Windows gives five seconds, stops the run as a second close does.

What each title does when it is closed, by a route that replays a committed one and closes:

| Title, where | What it does | Instructions to its exit |
|---|---|---|
| Rock Raiders, in its first mission (flip 9000) | stops its CD audio, exits 0 | 1,500,815 |
| Rock Raiders, at its level select (flip 5000) | its window is destroyed and it runs on; stopped after the grace | never exits |
| Hot Wheels, in its first race (flip 1020) | exits 1, as it always exits | 13,835,553 |
| Alpha Team, at its menu (flip 2300) | posts its quit, writes `Game.txt`, shuts NetImmerse down, exits 0 | 1,062,668 |
| Alpha Team, in its first mission (flip 8000) | the same | 973,232 |
| LEGO Island, in its Information Center (flip 5200) | posts itself `WM_CLOSE`, exits 0 | 6,669,503 |
| LEGO Island, signed as LEGO with Pepper picked (flip 9000) | the same, saving `G0.GS` and `History.gsi` first | 8,858,439 |

None asks a question. Rock Raiders' front end runs loops of its own that never look at the flag its
window procedure sets on `WM_CLOSE` and `WM_DESTROY` (OpenLRR's `Main_WndProc` and its note on the
close button within menus, read as documentation), so on Windows too it would run on with its
window gone. No committed route closes, so no run moves: LEGO Island's
`maintainer-first-play.route` was recorded before a route kept a close, and ends on its budget. The
stub run `stopped.rs` stops with a signal, which has no window to tell, now logs the close as
`input flip 0 Close` and ends where and as it did. The Windows and X backends are compiled and
clippy-clean, not run.

Minimizing the window on macOS, or hiding the application, is not a close (#173). The macOS backend
took its window leaving the screen for its close button, and a window minimized into the Dock, or
one of an application hidden, is off the screen too (Apple's `isVisible`, `isMiniaturized` and
`hide(_:)`). So, read from the code (#6, `a8fc43a`) and Apple's documentation rather than seen at a
window, minimizing the window stopped the run in 1.0.0 and 1.1.0, ended the "close the window to
end" wait after a run, and with the close above would have told the title `SC_CLOSE`. Windows
answers a minimize with `SC_MINIMIZE` and an iconic window, never `SC_CLOSE`, and the Windows and X
backends never took one for a close. The decision was `host::closed`, with no `cfg`, until the
window's delegate took the close from the button itself (#183, below). On a minimize or a hide the
title is told nothing new: no `SC_MINIMIZE`, `WM_SIZE`, `WM_ACTIVATE` or `WM_ACTIVATEAPP` FALSE,
only the keys and buttons it was told are held let go, as for any window not in front (#141), and it
runs on, unseen but heard. Telling it as Windows does is #177. No route moves; a headless run has no
window.

The macOS window outlives its close button (#175). A window made in code is released when it closes,
by default (Apple's `isReleasedWhenClosed` and `close()`), and the backend went on asking it about
itself, setting its caption and drawing into its layer while the title left (#140), and closed it
again as the run ended: messages to an object it may have freed, read from the documentation and
never seen to crash. The window is now kept until the process ends, and since #183, below, its
button no longer takes it off the screen. No route moves; a headless run has no window.

A window on another Space is not in front on macOS, so switching Spaces frees a captured mouse, as
losing the front does, and the title hears no key there (#176). #141's front read whether the
window was on the screen, not minimized, and of the active application. A window on another Space
is still on the screen (Apple's `isVisible`), and, reasoned from the code and not seen at a window,
its application stays active when nothing else takes the front there: the mouse would stay
captured with the window out of sight, the pointer hidden and frozen, or LEGO Island's pulled back
into the window's place. The front now also asks `isOnActiveSpace`; `isVisible` and
`isMiniaturized` stay, because Apple says a window off the screen answers where it would go. The
decision is `host::in_front`, with no `cfg`, so every host's tests hold it. Coming back to the
Space, like coming back to the front, captures nothing until a click (#141). A key pressed while
the window is not in front is the title's no more, on every backend (`Capture::key`), as Windows
and X send such a window no keys and macOS sends them on to an application whose window is on
another Space, where Escape would have reached Hot Wheels and quit it; on macOS the keys the title
holds are let go when the window leaves the front as well as when it stops being the key window.
A known limit: Mission Control, App Exposé and Show Desktop change no Space, so a captured mouse
stays captured over them until Escape. No route moves; a headless run has no window.

The Dock's Quit, a logout and a restart reach the title on macOS as the window's close button does
(#178): one close, delivered at the next flip as `SC_CLOSE` and recorded in the route, and the run
ends when the title leaves, a second close or the five declared seconds still stopping one that
never does (#140). So does any other request to quit the application, Activity Monitor's Quit and
AppleScript's `quit` among them. They ask the application `terminate:`, which, with no delegate,
ended the process at once, inside `pump`'s `nextEventMatchingMask:`: the title was never told, the
run wrote no `end` line, and LEGO Island didn't save. The application now has a delegate, a class
made at run time through the Objective-C runtime's own `objc_allocateClassPair`, `class_addMethod`
and `objc_registerClassPair`, with no crate. Its `applicationShouldTerminate:` counts the close and
answers `NSTerminateCancel` (Apple, `terminate(_:)`), as SDL and GLFW do, read as documentation:
`NSTerminateLater` would have AppKit run its own loop until answered, with the title stopped inside
it. So a logout or a restart is interrupted, and the person asks for it again once the title has
left; that holds in the "close the window to end" wait too, which a quit ends. A logout also asks
the run's terminal to quit, and a terminal that hangs up the run without asking is a second close,
which stops the run before LEGO Island has saved. It also answers
`applicationSupportsSecureRestorableState:` yes, which spares the terminal AppKit's warning about
an application delegate without it. No route moves; a headless run has no window.

A Windows sign-out, shutdown or restart reaches the title as a close too (#184), where the window
handed `WM_QUERYENDSESSION` to `DefWindowProcW`, whose TRUE let the session end the process with the
title untold. The window holds the session while the title leaves: Windows wants the answer at once
and the title runs on the window's own thread, so it is held as Microsoft documents for a window
that learns only then that it must block ("Application Shutdown Changes in Windows Vista", usage
model 2): `ShutdownBlockReasonCreate` with "The game is closing.", then FALSE. Windows shows that on
its screen while the title leaves; whether the screen then goes on by itself or the person asks
again, Microsoft does not say. The process asks to be asked first, at the top of the "Application
reserved first shutdown range" (`SetProcessShutdownParameters(0x3FF, 0)`; every process starts at
0x280, and Windows asks from high to low), so its FALSE ends the round before Windows Terminal, the
default console on Windows 11, is asked: Terminal quits at any `WM_ENDSESSION`, and the title's
console closing would be a second close that stops it before it is done.

It is at most one close, unlike a quit on macOS, which counts each (#178): a sign-out after the
close button, Alt-F4 or Ctrl-C, or one asked again while the title leaves, adds none. Alt-F4 now
counts in `CLOSE_ASKED` with the rest, where the window kept its own count. The Restart Manager's
session end (`ENDSESSION_CLOSEAPP`) is answered TRUE and counts no close, as `DefWindowProcW`
answered it: its guidelines ask for TRUE and for shutting down only at `WM_ENDSESSION`, and the
`WM_CLOSE` it sends a window still there is the close. A forced one, Windows' "Sign out anyway",
"Shut down anyway" or "Restart anyway", or `ENDSESSION_CRITICAL`, cannot be refused: Windows ends
the process, the title untold and the run with no `end` line, as a forced sign-out (`EWX_FORCE`)
ends it without asking at all. So does a run that has answered none of its window's messages for
five seconds when the session ends, such as one loading without pumping: Windows ends it as not
responding, untold, as before. The reason is withdrawn as the window goes. `WM_ENDSESSION` is still
`DefWindowProcW`'s, which answers zero as documented.

X is unchanged. systemd-logind's `KillUserProcesses=` is set when systemd is built: upstream's
default is yes, but Debian, Ubuntu, Fedora and Arch build it no. There a logout sends no `SIGTERM`,
and the display going away ends the process through Xlib's I/O error handler, the title untold and
the run with no `end` line. Where it is yes, the session's `SIGTERM` is a close (#140), and so is a
desktop's `WM_DELETE_WINDOW`. A session manager asks only a client registered through libSM. No
route moves; a headless run has no window.

The macOS backend drains an autorelease pool every pump, so a long run no longer grows the
process's memory (#180). AppKit opens a pool for each turn of its own event loop, and this backend
runs its own loop, so nothing AppKit handed back autoreleased was ever let go: each event, the run
loop mode and the date asked for every pump, and each new caption (Apple, `NSAutoreleasePool`:
"If a pool is not available, autoreleased objects do not get released and you leak memory").
Measured by probes making the same calls with no window, a million times each: a pump with no
event leaked about 58 bytes, each event fetched about 0.9 KB, and the first probe's loop, which
also makes a caption and asks for the screens, about 110 bytes a pass, growing by 109 MB with no
pool and by 1.4 MB with one drained each pass. A pump runs twice a flip. The pool is an
`NSAutoreleasePool`, opened at every way into the backend that reaches AppKit, `open`, `pump`,
`present`, the title's pointer calls, opening and closing the audio, and `Drop`, and drained as
each ends, a panic's unwinding included. What the backend keeps across pumps is owned, never only
autoreleased: the window (#175), the delegate (#178), the title's pointer shape and the window's
layer, now retained by the backend; the view is the window's and the application AppKit's. No
route moves; a headless run has no window.

The macOS window's close button reaches the title through the window's delegate, and the window
stays on the screen until the title has left, as on Windows and X (#183). The button took the window
off the screen at once, and the backend read the close from the window leaving it (`host::closed`,
#173), so the title saved and left with nothing in view, and a close of a minimized window, AltTab's
for one, was never counted (reasoned, not seen at a window). #178's delegate now also answers
`windowShouldClose:`, which AppKit asks when the person clicks the button (Apple,
`windowShouldClose(_:)`): it counts one close and answers NO, so the window stays; it is closed when
the run ends, by `close`, which asks no one (Apple, `close()`). `host::closed` is gone, and nothing
reads a close from the window's visibility. Command-W and Command-Q are counted by the key as
before, so a key held down repeating is still one close, as Windows counts Alt-F4 by the key; sent
on as `performClose:`, each repeat would be another close, and the second would stop the run. No
route moves; a headless run has no window.

The macOS backend sends every message that answers a rectangle through `objc_msgSend_stret` on an
Intel Mac, so an x86_64 build no longer reads garbage or crashes the first time it asks a window,
its view or a screen for its `frame` (#186): at any press on the window, a move of the free mouse
over it, a steered move, or a pump with the mouse kept. On x86_64 a struct larger than two
eightbytes comes back through memory its caller provides (System V AMD64 psABI, 3.2.3), and the
runtime has its own entry point for that (`objc/message.h`); an `NSRect`, four doubles, is one, and
an `NSPoint` or `NSSize`, two, is not, nor is a `CGFloat`, as Apple's own compiler sends them. Arm64
has no such entry point and `objc_msgSend` answers any struct, so nothing changes on Apple silicon.
The decision is `host::stret`, with no `cfg`, so every host's tests hold it, and every struct-
returning send goes through `Runtime::send_struct`, which answers zeros for a message to nil, as
Apple's compiler does, since the stret entry point writes nothing then (the first screen is nil with
no display). Built and clippy-clean for `x86_64-apple-darwin`; core's `host` tests, the struct
path's included, pass as x86_64 under Rosetta on an Apple silicon Mac, and fail without the stret
entry point (a crash) or without the nil answer. Not played on an Intel Mac. No route moves; a
headless run has no window.

`WM_TIMER` is sent as Windows sends it (#138), where `SetTimer` and `KillTimer` answered success
and no timer ever came due. It moves Hot Wheels' runs, which are released and signed off, a move
the maintainer blessed on 2026-10-21; a person drives Hot Wheels' first race again before its next
release (golden rule 7).

- `SetTimer` keeps a window's timer: its number, its period, held between `USER_TIMER_MINIMUM`
  and `USER_TIMER_MAXIMUM`, and when it next comes due on the declared clock. Set again with the
  same number it is replaced, and comes due a whole period from then. It answers the number. A
  timer with no window, the thread's own, is a wall, and so is one for a window that is gone: no
  title sets either.
- `KillTimer` takes the timer away, and with it any message it had due. A timer that is not there
  is refused: zero, and `ERROR_INVALID_PARAMETER`. `DestroyWindow` takes the window's timers with
  it, as Microsoft documents, as it takes the messages waiting for it.
- `PeekMessageA` makes a timer's `WM_TIMER` only when the filter takes nothing else: after posted
  messages, `WM_QUIT`, input and `WM_PAINT` (which this machine never makes). A timer has one at
  most: its number in `wParam`, its callback in `lParam`, and the tick count it was taken at as
  its time. Looked at, it stays; taken, the timer comes due again on its own beat, the first whole
  period after it last came due that is still to come, so a title that was late hears it once. As
  with posted messages (#139), only the thread that made the window takes it.
- `DispatchMessageA` calls a timer's callback in place of the window procedure, with the window,
  `WM_TIMER`, the number and the tick count, as a `TimerProc` is called. No title sets one.
- Sources: Microsoft's documentation of `SetTimer`, `KillTimer`, `WM_TIMER`, `DestroyWindow` and
  `PeekMessage`'s order (sent, posted, input, `WM_PAINT`, `WM_TIMER`); Wine's `server/queue.c`
  (`set_win_timer`, `kill_win_timer`, `get_message`, `find_expired_timer`, `restart_timer`) and
  `dlls/win32u/message.c` (`NtUserSetTimer`, `NtUserDispatchMessage`), read as documentation.

LEGO Island sets a one-second timer at a press of the button with nothing under the pointer to
take it (`LEGO1.DLL` 0x1005cfca) and kills it at the release. Its `WM_TIMER`, with the button still
down and the pointer not moved past the drag threshold, starts a drag: a button held still on the
island walks, from a declared second after the press. Before, the island stood still under it.

Hot Wheels sets a one-second timer as it starts (0x004152bb) and kills it only as it quits, before
it destroys its window, so its window procedure now hears a `WM_TIMER` once a declared second, 117
times over `first-race.route`. Its handler (0x41a31c) copies the count of frames drawn since the
last one (counted at 0x41a5d9) into a word nothing in the program reads, zeroes the count, and
passes the message to `DefWindowProcA`: a frame rate that is never shown, 52 instructions a
second. Nothing in the game reads what it writes, but the game reads its tick count at 120 places,
and every read after the first message lands that much later. Its first race finishes as before:
the same race, the same `WHEEL.DAT` and `TIMES.DAT`, written at the same flips, 870 and 3728, and
the quit at flip 3903. Every 25th frame of it compared, the menus, the track, the camera, the race
clock and the meters are the same throughout; in 46 of the 157 the cars themselves, and the sparks
they throw, are drawn a pixel or two apart, at most 13,368 pixels a frame, from the drop onto the
starting line (flip 875) to 1100 and from 2475 to the finish.

The runs that move, against main at `5a775ac`, every other end line and digest unchanged, Rock
Raiders', Alpha Team's and every one of LEGO Island's committed routes among them:

- Hot Wheels' `first-race.route`: end cursor 13,462,722,543 to 13,462,673,372, the same flip and
  declared millisecond; digest `d48d8761…` to `ca24a96c…`.
- Hot Wheels' escape route, written by its test: end cursor 583,424,325 to 583,424,086; digest
  `8bf567a9…` to `1448924f…`.
- Hot Wheels' boot to 2,200,000,000: digest `62d6d90b…` to `cb56737b…`.
- LEGO Island's restless hand: digest `c029337f…` to `99ce5f30…`. Two of its releases, at flips
  488 and 682, kill a timer that is not there and are refused with `ERROR_INVALID_PARAMETER`,
  which `LEGO1.DLL`'s C runtime saves and puts back around its thread data (`GetLastError` at
  0x1008d5f2). Nothing else of the run moves.

`LoadLibraryA` answers as Windows does for a library the declared machine does not have, and stops
at a wall for one it cannot say (#146), where it handed back a module for any name.

- A library is found when the title ships it, in its install directory whatever case it is asked
  for in, or when core writes an entry point of it. A name with no extension is looked for with
  `.DLL` added, as Microsoft's `LoadLibraryA` page documents.
- `Declared` gains `absent_libraries`: the libraries a title's machine does not have, each with
  why. One of those is answered NULL, with the declared Windows' error: `ERROR_DLL_NOT_FOUND`
  (1157) on Windows 9x, the answer Wine's conformance test records for 9x (wine-1.0,
  `dlls/kernel32/tests/module.c`, `testLoadLibraryA_Wrong`: "Expected ERROR_MOD_NOT_FOUND or
  ERROR_DLL_NOT_FOUND (win9x)"), and `ERROR_MOD_NOT_FOUND` (126) on NT, as Microsoft documents. Every
  declared Windows today is 9x. The run logs `library missing <name>: <why>`.
- Anything else stops at a wall that names the library as neither the title's own, nor written,
  nor declared absent: a library of Microsoft's that core writes nothing of (SHELL32, say), which
  every Windows 98 has, and a library the title ships only somewhere other than its install
  directory, the one place a title's library is looked for (a path asked for is cut to its file
  name). Answering either "not found" would send a title down a fallback
  no player met (golden rule 2).
- A library's `.dll` is now matched whatever its case, where only `.dll` and `.DLL` were. This is
  `find`'s match too, so an import naming `DDraw.Dll` binds to its shims, and `GetProcAddress` on a
  handle `LoadLibraryA` gave for it finds them, where both missed before. No binary on the roster
  spells an extension so.
- A handle for a library core writes is known by the file it names, not the path it was asked
  for by, so `GetProcAddress` on one asked for by a path finds its shims, where it missed before.
  No run asks so.
- `GetProcAddress`'s and `FreeLibrary`'s own code is unchanged: a title given NULL does not call
  them.

Every released title declares no absent library: in every gate run it asks only for libraries it
ships and ones core writes. Only Alpha Team calls `LoadLibraryA` in a committed run, for thirteen
libraries it ships or core writes, so no run moves. LEGO Chess, which is not on `main` yet, will
declare two when #143 lands: `glide2x.dll`, since its declared machine has no 3dfx card, and
`KLIB_OpenGL_PlugIn.dll`, a renderer plugin it walks but never shipped. Each is then told it is not
there, where the game would have called a null hook and its Glide plugin gone on into null `gr*`
pointers.

`absent_libraries` is a required field every title had to add to build, so it makes core's next
release a major one (ADR-0007, as #163 settled). Core stays at 1.2.0 until that release.

The mouse is the person's until they click on the picture, the same in every title, by the
maintainer's rule of 2026-10-21 (#141). A title that draws its own pointer took the mouse as soon as
the person's pointer was over the picture with the window in front, and only Cmd-Tab gave it back;
LEGO Island never took it, and was handed every move over its picture.

- Until a click on the picture the title sees none of the mouse, no move and no button. That
  click captures it and is the host's, and so is its release: it would otherwise press whatever
  the title's own pointer was over. A click on a window behind others captures too, once the
  window has come to the front, which it must do within two pumps after the one the click came
  in or half a second, whichever is longer, so that neither a fast host nor a stall cuts it
  short; a window that comes to the front any later captures nothing. A press on the window's
  frame never captures (#49).
- Captured, every move and button is the title's. A title that draws its own pointer (Rock
  Raiders, Hot Wheels, Alpha Team) has the person's hidden and frozen and its own steered by how
  far the mouse goes, as when it was taken. One that shows the host's (LEGO Island, #139) keeps
  it on the picture, in the title's shape, where until then it was the host's arrow: by
  `ClipCursor` to the client area on Windows, by a grab confined to the window on X, and on
  macOS, which has neither, by putting it back at the edge it left by once a pump, after the
  window's place in front is settled, with the mouse joined to the pointer again straight after
  so that the warp holds no event back. So on macOS the pointer can stray past the edge for a
  frame, or for as long as the title takes to flip, before it is put back, and a pointer that
  left for a window now in front, or a window closed by its button or minimized, is not put back. Which of
  the two a captured mouse is follows the title's `ShowCursor`, whenever it changes, and a title
  the mouse comes to be kept for, by a click or by its `ShowCursor`, is told where the pointer is
  at the end of the pump, which no move has told it since. "The picture" here is the window's
  content, bars included, as for a click (#49).
- Escape frees it and is the host's, repeating and let go, so the Escape that frees the mouse
  never quits Hot Wheels; the next one, with the mouse free, reaches the title as any key does,
  and stays the title's, repeating and let go, if a click captures the mouse while it is held.
  The window leaving the front frees it too; only a click captures it again. Either way the title
  lets go of every button it was told is down, whose release may go to another window, or on X
  come to nobody once the grab is gone. The end of the run frees it for good, through
  `Host::free_mouse`, which the run calls where it called `show_pointer(true)`, so no click
  captures it while the window waits to be closed (#111).
- A button let go reaches the title only if its press did, as a key does, and only once, and a
  press on the window's frame let go on the picture is no longer a release the title never saw
  pressed.
- The window's caption says which it is, `<name> — click to play` or `<name> — Escape frees the
  mouse`, with a plain dash on X, whose `XStoreName` takes Latin-1, and the name alone once the run
  is over. The run log's `pointer flip` line says a title that draws its own pointer has a
  captured mouse steered.

The rule is one state machine, `host::pointer::Capture`, to which each backend hands its clicks,
its keys, the title's `ShowCursor` and, once a pump, whether it is in front, and which answers
what the host does with the mouse (`host::pointer::Mouse`: free, steered or kept on the picture);
nothing in it is a title's. No route records the click that captures, its release or the Escape
that frees, and a headless run has no window to capture, so no committed route moves and no run
does: LEGO Island's hands, which drive the machine through a headless host, record the routes
they recorded before. The macOS backend is compiled and its unit tests run, not played; the
Windows and X backends are compiled and clippy-clean, not run.

A General MIDI synth in core plays a DLS Level 1 bank into the mix on the declared clock (#155,
ADR-0008 1(a)). Nothing calls it yet: LEGO Chess's MIDI stream (#154) and the player's bank (#156)
will, and no title's run moves.

- `dls::Bank::parse` reads a bank, written from the MIDI Manufacturers Association's DLS Level 1
  specification (v1.1b, read with its errata): instruments, their regions of the keyboard, each
  region's wave through the pool table, its sample values (the region's own `wsmp`, else its wave's,
  else a one-shot at middle C) and its articulation (the region's own `art1`, else its instrument's,
  else the specification's defaults). It walks RIFF with `winmm`'s chunk finder.
- Each wave is read once, however many entries of the pool table point at it, and they share it. A
  bank that records more than it holds is refused: a chunk past what holds it, a count past its
  chunk, a `colh` or `insh` counting other than the instruments or regions that follow, or waves
  whose data, added up, is more than the bank's own bytes. So the synth holds at most twice a bank's
  size, an eight-bit wave widened to sixteen. Refused as well, as Level 1 does not have them or no
  bank played yet uses them: a wave that is not mono PCM at eight or sixteen bits, a `wsmp` of more
  than one loop or a loop of another type, a loop past its wave's end, and a key set not to move its
  pitch.
- `synth::Synth` plays it: note on and off, program change and bank select, modulation, volume, pan,
  expression, sustain, the pitch wheel and its range, fine and coarse tuning, reset all controllers,
  all notes off, all sound off and the mode messages that end every note, and drums on channel 10
  from the bank's own kits, their keys past General MIDI's 35 to 81 included. Volume and pitch
  envelopes, one LFO, velocity and key scaling and loops come from each region's articulation. Bank
  select and data entry follow MIDI 1.0: a high part clears its low part, and data entry goes to
  whichever of a registered or a non-registered parameter was chosen last. A system common message
  ends running status.
- What the specification leaves to the device is declared in `synth.rs` with its reasons: 32 voices,
  Windows 98 SE's, a finished voice freeing its place before a note looks for one, stolen by DLS
  Level 1's static channel priority, a released voice before a held one and the oldest first; a key
  struck again cutting in 15 ms its notes that have had no note off, unless their own region lets
  them overlap, a note the pedal keeps ringing on; linear interpolation; envelopes and the LFO moved
  every frame; the LFO swinging gain both ways, the specification's worked example taken over its
  prose; gain added up in centibels and stopped at 0 dB; a key group cut in 15 ms; a variation the
  bank lacks played as its capital tone; reset all controllers keeping bank select unless asked for
  every power-on value; no reverb or chorus.
- The mix plays the synth: `Mixer::open_synth` gives it one, and `Mixer::midi` hands it a short
  message, packed as `midiOutShortMsg` takes one, heard from the frame its declared time falls in.
  It says whether a synth heard it: without the player's bank there is no MIDI device to send
  through, and the device that sent a message none heard stops the run at a wall (#154).
- Every sample is integer arithmetic, but for a note let go in its attack, which finds its decibels
  with the same maths as the tables. Cents to a ratio, centibels to an amplitude and the LFO's sine
  read tables built once with `cpu::x87::maths`, whose IEEE operations round alike on every host,
  and a test pins the tables' hash and what a tiny bank written in the tests renders.
- The player's `GM16.DLS` (sha256 `3229b09b…`) is loose on both pressings of Rock Raiders and both
  of LEGO Racers, and the bank can come from either. Read off the Rock Raiders Masterpiece rip the
  gate already uses, a C major scale on the piano and the GS Shaker, key 82, sound and render to the
  same samples every time; without the rip those tests skip, and `LEGO_LEGACY_REQUIRE_DISC=1` fails
  them. LEGO Chess's four tunes, played through it with that bank, reach 40 to 55 voices at once
  with every release tail heard. At 32 they steal 620 times, 611 of them a release tail, and 66
  notes are never played, 62 of them on channel 16, the lowest priority.

The declared CD can answer long names from its Joliet tree, as Windows 95's CDFS does (#144).
LEGO Chess opens `D:\intro\Chess Bumper01.bmp` after its intro, a name only its disc's Joliet tree
holds; the primary tree calls the same file `INTRO/CHESSB~1.BMP`.

- `Iso::read_joliet` finds the supplementary volume descriptor whose escape sequences are `%/@`,
  `%/C` or `%/E` (ECMA-119 8.5.6, the Joliet specification) and answers names from its tree from
  then on: UCS-2 big-endian, `;1` stripped. `Iso::open` still reads the primary descriptor alone.
- The primary tree's 8.3 names are not answered on a disc read this way. Nothing public says
  whether Windows 95's CDFS answers them or 8.3 names of its own making, as NT's does, and the two
  can differ on the roster's own rips: Chess's `ereg` records `LEGO Chess_EZ.exe` as
  `LEGOCH~3.EXE` and `LEGO Chess_eReg.exe` as `LEGOCH~2.EXE`, where numbering them in the long
  names' order swaps the two. A name only the primary tree records is told it is not
  there, and the machine stops at a wall naming it at the boundary, or as the title loads (golden
  rule 2); a name neither tree records is not found. No title asks for one. A path that mixes a
  long directory name and an 8.3 name is in neither tree, so it is not found rather than a wall.
- A path through a file is no path, in either tree, and is not found: every name on the way but
  the last must be a directory's, and `Iso::list` lists only a directory. Before, a file's bytes
  were read as a directory's records.
- `Declared::joliet` says whether a title's machine reads its disc this way. `load_title` reads
  the title's image by the primary tree's names, as the launcher does, and then the disc as the
  title declares; `testkit::tree` reads it the same way. It is a field every title had to add to
  keep building, so core's next release is a major (ADR-0007, settled on #163); core stays at
  1.2.0, past its last tag, until then.
- `FindFirstFileA` on the CD is a wall, for every title, where it looked only at the declared
  install tree and found nothing, `ERROR_FILE_NOT_FOUND`. No title searches its CD, and what
  Windows answers for a disc read through its Joliet tree, the 8.3 names included, is not known;
  it is written when a title's wall names it (golden rule 4). A pattern relative to a directory on
  the CD walls too. No committed run reaches it: only Alpha Team searches, with patterns relative
  to its install directory on C:, and its only `_chdir`s sit behind `PackDir` and `LocaleDir`,
  which its declared `Config.txt` never sets.
- The gate needs LEGO Chess's rip whenever `LEGO_LEGACY_REQUIRE_DISC=1`: its bumper's test reads
  the Masterpiece pressing, `game-data/chess/masterpiece-chess`.

Every roster rip has a Joliet tree. Read through it, with `joliet` forced on, Rock Raiders', Hot
Wheels' and LEGO Island's runs do not move (all 29 of their run lines, measured at `7509b49`):
their discs answer the same names but for case. Alpha Team's do. Its original pressing's Joliet tree has
`Strings_en.txt`, the primary tree's `STR_0012.TXT`, the autorun launcher's string table, and
the title's `MSVCRT.dll` opens `D:\Strings_EN.txt` at boot, which is otherwise missing. Every
route moves, and every boot run past 2,000,000 instructions; the boots to 100,000, 200,000 and
20,000,000 instructions and the debugger's quit do not. Measured at `55a3105` (#138 moves none of
Alpha Team's runs, and it asks for neither of its two 8.3 names that differ):

| Alpha Team run | Off | On |
|---|---|---|
| `menu.route` | 2,341 flips, 139,471 ms, `f34a24e8…` | 2,341 flips, 139,470 ms, `101c533b…` |
| `click-the-intro-300.route` | 1,634 flips, 110,593 ms, `a71a75d7…` | 1,634 flips, 110,591 ms, `5070d423…` |
| `click-the-intro-600.route` | 605 flips, 41,938 ms, `00c2662d…` | 605 flips, 41,937 ms, `b2d82eff…` |
| `select-adventure.route` | 4,454 flips, 214,104 ms, `87323b4d…` | 4,454 flips, 214,102 ms, `254746d4…` |
| `first-mission.route` | 8,054 flips, 331,967 ms, `1b7d7349…` | 8,053 flips, 331,937 ms, `48dfc4dc…` |
| boot to 1,100,000,000 | 1,634 flips, 111,704 ms, `1a08e772…` | 1,634 flips, 111,703 ms, `624bd32f…` |
| boot to 600,000,000 | 1,634 flips, 110,593 ms, `198706f8…` | 1,634 flips, 110,592 ms, `17169231…` |
| boot to 2,000,000 | 0 flips, 4 ms, `e37ef1e8…` | 0 flips, 4 ms, `2d5f0520…` |

So, by #142's rule, `joliet` is declared per title: the four released titles declare `false`,
their runs unchanged, until the maintainer decides, and LEGO Chess declares `true` with its crate
(#143).

LEGO Chess joined the roster as its fifth entry, `roster::LEGO_CHESS`, with its own crate and in
progress (#143): `Lego Chess.exe` out of the Masterpiece pressing's `MAIN.Z`, no Red Book audio.
Nothing of that game is loose on its disc, so core reads the archives its setup expands:

- `is3`, InstallShield 3's `.Z` archive: its header, its table of directories and then files, and
  each file in the directory its own record names by index, not where the table lists it. A table
  whose record is too short for its own name is refused as damaged (`Is3Error::Damaged`). The
  original pressing's `MAIN.Z` lists its executable, a root file, last, and a reader that deals the
  records out to directories in turn misplaces 48 of its files; a unit test holds the rule over an
  archive listed that way.
- `explode`, PKWARE's Data Compression Library implode undone, written from Ben Rudiak-Gould's
  description of the format (comp.compression, 2001, placed by him in the public domain), its
  three tables his, code word by code word: plain or coded literals, and a dictionary of one, two
  or four kilobytes. `explode(bytes, expanded)` takes the size its table gives it and stops a
  stream as soon as it would pass it (`ExplodeError::TooLong`). His
  example, with the one offset Mark Adler's `blast` notes as wrong put right, explodes to
  `AIAIAIAIAIAIA`; all 881 members of `MAIN.Z` and the 7 of `_SETUP.LIB` expand to the sizes their
  tables give, and were checked once, off the suite, against the research's own expansion of
  them.
- `title::Installed::Expanded`: every member of an archive on the disc under the install directory,
  expanded when it is read, as a cabinet's entry is unpacked (`install::Tree::set_expanded`), and
  at the root members of the setup's other archives it copies there on their own: Chess's
  `Text.txt` out of `_SETUP.LIB`.
- `EntryImage::in_cabinet` is `EntryImage::kept`, a `title::Kept`: loose on the disc, in the
  pressing's cabinet, or imploded in an archive it names. `roster::look_for` and the machine read it
  alike.

LEGO Chess's machine reads its disc through its Joliet tree from its install on (#144), and is the
first to declare absent libraries (#146), Glide's runtime and its OpenGL plugin; its own changelog
says why. Its bumper and its plugin table were not reached then: the run stopped at
`GetClipCursor`, which #147 answers above; #149's run reaches the plugin table and #148's walks it
through.
`install::Tree::install` is now the one way a title's tree is built, by the machine and by
`testkit::tree` alike, where each had its own copy of the step and Chess's test a third.

No other title's route moves.

A title can declare a floppy drive, A:, with no disk in it (#145). `Declared` gains the required
field `floppy`, which every title crate had to add to build, so by ADR-0007 this makes core's next
release a major: this heading releases as 2.0.0 (#163). The drives are listed in one place,
`Ctx::drives`. `GetLogicalDrives` sets bit 0 for the floppy, which Microsoft documents as A:;
`GetLogicalDriveStringsA` names `A:\` first; and `GetDriveTypeA("A:\")` answers `DRIVE_REMOVABLE`
(2). `GetDriveTypeA` with no root still answers C:'s type whatever the current directory, since
Alpha Team saves its game only on that answer. Every call that names a place on A: is refused as on
an empty drive, through `Ctx::refuse_on_empty_floppy`:

- KERNEL32's `GetVolumeInformationA`, `CreateFileA`, `OpenFile`, `GetFileAttributesA`,
  `FindFirstFileA`, `DeleteFileA`, `CreateDirectoryA`, `MoveFileA` from or to A:,
  `SetCurrentDirectoryA`, `SetFileAttributesA` and `LoadLibraryA`;
- VERSION's `GetFileVersionInfoA`, WINMM's `mmioOpenA`, and the movie stream's
  `IAMMultiMediaStream::OpenFile`.

For a title that has set `SEM_FAILCRITICALERRORS` each fails with `ERROR_NOT_READY` (21); the movie
stream hands it back as `HRESULT_FROM_WIN32(ERROR_NOT_READY)`. For one that has not, Windows would show its box asking
for a disk (`GetVolumeInformationA`'s remarks), and this machine has no such box, so the run stops
at a wall that names it. Other drives the machine lacks are not refused yet (#171). Only LEGO Chess
declares the drive. Every released title and the stub declare none and answer as before, so no
route moves.

A member of the pressing's archive that will not come out stops the run at a wall naming the
archive, the image, the member and why (#169), where the guest was told the file was missing, a
file `GetFileAttributesA` and `FindFirstFileA` had just said was there. A damaged import was taken
at boot for one of Microsoft's, and `LoadLibraryA` of a damaged library stopped at #146's wall,
which named the title's own library as neither its own nor written. The tree answers both kinds of
archive in one place, `Tree::member`: a cabinet entry (Rock Raiders, Alpha Team) and an
InstallShield 3 member (LEGO Chess) whose span will not read or runs past its archive's end, whose
data will not decode, that is stored a way the reader was not taught, or that comes out another
size than its table gives. The executable the launcher hashes (`EntryImage::read`) and LEGO Chess's
`Text.txt` are read elsewhere, and still refuse with their readers' own errors. `Cabinet::unpack`
now refuses the wrong size as `Corrupt`, as `Member::expand` already did, an entry whose header
gives bytes it holds none of among them; it handed back whatever its chunks inflated to.
`Iso::read_range` refuses a span past the image's end as the image damaged, so a rip cut short says
it has to be ripped again. `Tree::wall` hands the machine this wall as it hands #144's, after the
call that met it. A good rip never comes here: measured once off the suite, every one of Rock
Raiders' 444 packed entries and Alpha Team's 57 unpacks to its header's size, and Alpha Team's
seven placeholders declare none; the suite expands all 881 of `MAIN.Z`'s members to their table's.
No route moves.

## 1.1.0 — 2026-10-21

What LEGO Island 1.0.0 runs on, from its roster entry to its saves: counted semaphores, the old
file API and files read where they lie on the image, waits on several handles, DirectX 5's
Direct3D and retained mode with meshes built, drawn, picked and projected with uniform scaling,
posted messages and a title's own pointer shapes, and `MoveFileA`. Beside it, the Windows (#107)
and Linux (#108) backends, run by machine and not yet played by a person; a run's wrap-up however
it is stopped; and answers truer to Windows for every title: x87's own NaNs, `GetVersion` as
Windows 9x answers it, a null pointer faulting, `RaiseException`'s context, and DirectX 6's
Direct3D for Rock Raiders. Each change below lists the released titles' runs it moved.

Two backends joined, each written and compiled for its operating system and untested there:

- Windows (#107): a window through user32 and gdi32, the pointer by macOS's rules, and sound
  through winmm's `waveOut`, loaded from System32 at run time, with kernel32's `VirtualAlloc` and
  `LoadLibraryExW` beside `mmap` and `dlopen`. Compiles for x86-64 and Arm64 Windows.
- Linux (#108): an X11 window with the Mac's pointer rules and ALSA for sound, both reached by
  `dlopen`. Compiles for x86-64 and aarch64 Linux.

The pointer, pace and audio feed the backends share moved out of the Mac's into `host::pointer`,
`host::pace` and `host::feed`.

Fixed once the Linux backend was first run, on x86-64 and aarch64 under Xvfb (#108):

- A resized window's bars kept pieces of the frame drawn just before the backend was told the new
  size, since the X server only paints the background where a window is uncovered. The bars are
  now cleared at every frame.
- A window destroyed by another client (`xdotool windowclose`) ended the run, and then Xlib's own
  error handler ended the process over the window that was gone. Errors that only say the window
  is gone are now let pass; every other X error still ends the process as Xlib's handler does.
- An x87 operation with no answer, such as `0/0`, kept the NaN the host's own arithmetic made: the
  unit's negative indefinite on an x86-64 host, a positive NaN on an Arm one. Rock Raiders stores
  one at instruction 356,515,652 of its boot, so its longer runs ended with different digests on
  macOS and on x86-64 Linux, breaking golden rule 1. Every x87 result now follows the unit's own
  rule (`x87::answer`): a NaN operand comes through quieted, and a NaN made from numbers is the
  indefinite. Seven of Rock Raiders' macOS digests move to the ones x86-64 Linux already gave.
- Control-Q ended the run without the guest seeing the Q pressed, but the Q let go still reached
  the guest and a recorded route as `Q up`. The Mac's Command-Q and Command-W and Windows' Alt-F4
  did the same with their Q, W and F4. Every backend's keys now pass through one rule
  (`host::key_change`): a key let go that the guest was never told was pressed goes no further.
  No route moves: a route replays what the guest was told, and none holds such a release.

The same rule now holds for the floats the shims make on the host (#109). Retained mode's maths
(`win32::scene`: matrices, vectors, rotations, inverses, the projection, and a viewport's
`Transform` and `InverseTransform`) and `IDirect3DRMViewport2::GetField` kept whatever NaN the host
made when a title passed an infinity, or numbers large enough to overflow, and the guest reads
those results back. Each now comes out by the unit's rule (`x87::answer_single`, beside `answer`),
from the operands it was made from. No route reaches one, so no digest moves. The rasteriser, the
sound mix and every other float a shim only compares, or turns into a whole number, were audited
and left alone: no NaN's bits reach the guest from them.

Fixed once the Windows backend was first run, on x86-64 in a VM (#107):

- A title is a console program, so that what it says reaches the terminal it was started from,
  and closing its console, or Ctrl-C or Ctrl-Break in it, ended the process with no wrap-up: no
  `end` line and no digest in the log. Each now ends the run as the window's close button does. A
  closing console gives a process five seconds, and the handler holds it open while the run writes
  up; a second Ctrl-C, with the run already ending, ends the process as before.
- `close the window to end` was printed after a run the person had ended by closing the window.
  It is printed only when there is a window left to close, on every host.
- A run that ended by itself, its route finished or its budget spent, kept the pointer while its
  window waited to be closed, if the title had taken it: on Windows clipped to one pixel and
  hidden, so a person had to Alt-Tab away to get it back. Every host now gives it back as soon as
  the run ends, before `close the window to end`, the way losing focus does, and does not take it
  again while the window waits, clicked or in front (#111).

A run stopped from its terminal got no wrap-up either, on any other host (#110). On macOS, Ctrl-C,
`SIGTERM` and the terminal closing (`SIGHUP`) put the pointer back and ended the process; on Linux
nothing heard them; and a headless run ended on Ctrl-C, on Windows too, with no `end` line and no
digest. A title's `main` now hears each of them, headless or in a window (`host::listen`), and
each ends the run as the window's close button does, with its `end` line and digest. The signal
handler only asks for the close, which every host's `closed` answers; a second signal, with the
run already ending, ends the process as before, putting the Mac's pointer back first. A signal
the run was started ignoring, as `nohup` starts it ignoring `SIGHUP`, stays ignored. The
debugger does not listen, so Ctrl-C there still ends it. No route moves. Such a run's `end`
line says `stopped by the person`, where it said `window closed`, since a run stopped from its
terminal may have no window.

LEGO Island joined the roster as its fourth entry, `roster::LEGO_ISLAND`, with its own crate and
in progress (#113): `DATA/disk/ISLE.EXE`, one pressing, no Red Book audio. `title::WINDOWS_95`
is the Windows it declares, packed as `WINDOWS_98_SE` is. No other title's route moves.

Counted semaphores and `Sleep(0)`, for LEGO Island's streamer threads (#115):

- `CreateSemaphoreA` and `ReleaseSemaphore` are answered as Win32 documents them. A wait on one
  takes one of its count. A release past its maximum fails with `ERROR_TOO_MANY_POSTS` and
  changes nothing, and the count before is written back only when a release succeeds and it was
  asked for. Counts out of range are `ERROR_INVALID_PARAMETER`, and a named semaphore is a wall
  until a title asks for one.
- A wait on a semaphore parks as one on an event or a mutex does (ADR-0005), and a release wakes
  whoever is parked on it, the first in the machine's order taking the count.
- A wait with no timeout that was woken and found another waiter had taken what woke it goes back
  down. It answered `WAIT_TIMEOUT`, which an endless wait never does.
- `Sleep(0)` with another thread ready gives that thread the rest of the turn, where the caller
  ran on to its next call. The order is the scheduler's, never the host's, so every run
  interleaves the same way.

`core/tests/threads.rs` proves each of these on hand-assembled guests, without a title. No route
moves.

The old file API LEGO Island reads its scripts and world through, for its `MxIOInfo` (#116):
`OpenFile`, `_hread`, `_hwrite`, `_llseek` and `_lclose`, with `GetFileSize` and `SetEndOfFile`.
They are faces over the handles `CreateFileA` makes, which `ReadFile`, `SetFilePointer` and
`CloseHandle` take as well.

- `OpenFile` reads, writes, makes (`OF_CREATE`, read and write as MS-DOS's create opens it) and
  asks (`OF_EXIST`, answered TRUE with nothing left open), with each `OF_SHARE_*` mode as the
  documentation maps it onto `CreateFileA`'s. It fills the `OFSTRUCT`: its length, whether the
  file is on a fixed disk, the MS-DOS error, and the path. A bare name is looked for where the
  title started, then where it is. A style that parses, deletes, verifies, prompts or reopens is a
  wall.
- `_hread` short-counts at the end of a file; `_hwrite` of nothing sets where the file ends, as
  MS-DOS's write does; each fails with `HFILE_ERROR` and `GetLastError` as Windows does.
- A second open of a file another handle does not share as it asks, through either door, is
  refused with `ERROR_SHARING_VIOLATION`.
- `SetFilePointer` past the end of a file opened only to read lands there, as Windows says, where
  it stopped at the end; a read there finds nothing. A move method that is not one is
  `ERROR_INVALID_PARAMETER`, where it was the end. A handle that is not a file is
  `ERROR_INVALID_HANDLE`, where `GetLastError` kept whatever it last said, which is a failed seek
  that reads as a success at `0xffffffff`. Hot Wheels' C runtime seeks its three standard streams
  as it exits and keeps the error it reads, so the digests of its two routes that exit move, their
  end lines unchanged: `first-race.route` from `189da6d5…` to `1f7255c4…`, and the escape route
  from `c8f1a71e…` to `5e33fc05…`. The maintainer accepted the move.
- A file on the pressing opened only to read is read where it lies on the image, a piece at a
  time, rather than held whole when it is opened (`win32::Contents`, `Tree::lies`, `Iso::lies`):
  Island's scripts are 534 MB, and `Isle\ISLE.SI` alone is 157 MB. What a title writes is held
  and reaches its profile as before.

Waits on several handles and on threads, as Win32 documents them, for LEGO Island's engine (#130):

- `WaitForMultipleObjects` waiting for any of its handles parks on all of them, and is woken by
  whichever is signalled, answering `WAIT_OBJECT_0` plus its place. It was parked on the first
  only, so a signal to any other never woke it.
- Waiting for all of them, which was a wall, takes none until it can take every one together, and
  then answers `WAIT_OBJECT_0`. A handle named twice is `ERROR_INVALID_PARAMETER`.
- A wait with a timeout that was woken and could not take what it waited for goes back down until
  its hour, as an endless one already did. It answered `WAIT_TIMEOUT` early.
- A thread's handle is signalled when the thread ends, by returning or by `ExitThread`, so a wait
  on it is over. `GetExitCodeThread` is written: `STILL_ACTIVE` until then, and what it ended with
  after.

`core/tests/threads.rs` proves each on hand-assembled guests, without a title. No route moves.

What LEGO Island asks for before Direct3D (#114), each written when its run stopped there:

- `SystemParametersInfoA` turns mouse trails off (`SPI_SETMOUSETRAILS` with 0 or 1), which this
  machine never draws. Any other action is a wall that names it.
- `AdjustWindowRectEx` leaves the rectangle as it is, as `AdjustWindowRect` already did: a
  frame is as wide as `GetSystemMetrics` says its border, caption and menu are, and the declared
  machine's are all zero.
- `GetProfileStringA` answers the caller's default, cut to fit and with trailing blanks left
  off: the declared machine has no `WIN.INI`, so no section holds the key. A null section or key,
  which lists names, is a wall.
- `GetWindowLongA` answers `GWL_STYLE` with the style `CreateWindowExA` was given. Any other
  index is a wall.

`core/tests/shims.rs` calls each one as a title would, without a title, and holds `VirtualAlloc`
to Windows' 64 KiB granularity, which SmartHeap finds its blocks' headers by. No route moves.

A traced or stepped run of a threaded title switches threads where its route's run does (#131),
as golden rule 1 asks. Under `--trace insns` threads switched at every instruction, and in the
debugger they never switched at the quantum, so what either showed was not what the route did.
Every run now keeps one schedule, `Machine::step`: a burst to the 65,536-instruction quantum or
the next call across the boundary, the switch, the call or instruction the next thread stands
on, and the next burst. A traced run takes its bursts an instruction at a time to write each one
down, and the debugger steps by the same schedule an instruction at a time, standing past any
switch, so a break at a thread's next instruction is met. The stub still runs an instruction a
turn: it has one thread and no boundary, so nothing is switched, and its `int`s are heard and a
snapshot can fall between any two instructions. `Machine::tick` is one instruction or call with no
schedule. `core/tests/threads.rs` runs two threads past the quantum plainly, traced and stepped,
and finds the same switches, cursor for cursor, and the same finish and digest. No route moves.

DirectX 5's Direct3D and retained mode, which LEGO Island builds the other way round from Rock
Raiders: immediate mode first, by itself, then retained mode over it (#117). Each was written
when its run stopped there, and every identifier is from Wine's `d3d.h` and `d3drm.h`:

- `IDirect3D2` comes off the DirectDraw object (`QueryInterface` for `{6aae1ec1-…}`).
  `EnumDevices` offers the declared machine's one device, the same one DirectX 7's offers, in
  DirectX 5's 0xcc-byte `D3DDEVICEDESC`: a hardware device by its colour model, with nothing
  emulated. `CreateDevice` makes an `IDirect3DDevice2` over the back buffer for that device; any
  other is a wall that names it.
- `IDirectDrawClipper::SetHWnd` clips to the part of the screen the window shows, which is all of
  it, as `GetClientRect` already said, so the engine's primary surface takes its clipper.
- `IDirect3DRM2` comes off the retained-mode object (`{4516ecc8-…}`). `CreateFrame` makes an
  `IDirect3DRMFrame2`; `CreateDeviceFromD3D` an `IDirect3DRMDevice2` over the device the title
  made, which it holds; `CreateViewport` an `IDirect3DRMViewport`; `CreateLightRGB` a light as
  DirectX 6's does. `SetDefaultTextureColors` and `SetDefaultTextureShades` are kept.
- `IDirect3DRMDevice2` is DirectX 6's device's first 39 slots, and the frame array's
  `IDirect3DRMFrame` is DirectX 5's frame's first 69, each sharing its table. The v1 viewport is
  DirectX 6's but for `Clear`, which takes no flags, and is a wall.
- `IDirect3DRMFrame2` is a table of its own, mapped by name: DirectX 6's frame drops
  `GetTextureTopology`, `SetTextureTopology` and `AddMoveCallback2`, so from slot 33 up the two
  sit one, two and then three places apart, where the old tree had measured only the first.
- A face DirectX 5's own faces are asked for that is not here is a wall naming the face and the
  identifier, rather than `E_NOINTERFACE`. Only LEGO Island holds them.
- `D3DRMCreateColorRGBA` is written.

`core/tests/shims.rs` places a frame through DirectX 5's slots and reads it back, and makes
colours. No other title's route moves.

`GetVersion` told every title it was on Windows NT (#129). The declared Windows was packed with
its top bit clear, which Microsoft documents as NT, and with the build in the high word, where
Windows 9x puts none: no real machine ever answered that, breaking golden rule 4.
`title::Windows` now holds what each Windows answered to `GetVersion` and `GetVersionExA`, with
the sources beside it: Windows 98 SE `0xC0000A04`, build `0x040A08AE` and `" A "`; Windows 95
`0xC0000004`, build `0x040003B6` and no CSD string.

No title takes a different turn, and every route's end line is unchanged. Each C runtime only
stores a copy of the word: Rock Raiders', Hot Wheels' Watcom one, Island's two, and the
`MSVCRT.DLL` and A3D that Alpha Team runs. The `GetVersionExA` callers, that `MSVCRT.DLL`, A3D
and NetImmerse's `LoadComp.dll`, read only the platform and the version, which did not change.
The one reader that branches on the top bit, SmartHeap's test for NT in `ISLE.EXE` and
`LEGO1.DLL`, which picks where shared memory may lie, is not reached by Island's boot. The copies
are guest memory, so the digest of every title's run moves, and no stub's:

- Rock Raiders: `first-light.route` `856f27b3…` to `d0504186…`, `first-mission.route`
  `e9edcf14…` to `5c74c51c…`, `level-select.route` `4fe12832…` to `82a0c841…`,
  `click-the-intro.route` `0c754229…` to `e71381ad…`, `skip-the-intro.route` `b4fa40cc…` to
  `ed8f2de4…`, and the runs with no route to 100,000 (`5bd29ec1…` to `6c54a6b2…`), 61,000,000
  (`2222ff17…` to `f820fc34…`) and 400,000,000 (`179ce83e…` to `f0cc0561…`).
- Hot Wheels: `first-race.route` `1f7255c4…` to `ca178965…`, the escape route `5e33fc05…` to
  `920d063e…`, and the runs to 100,000 (`b388bc29…` to `c4eae7cb…`) and 2,200,000,000
  (`7f753ad4…` to `62d6d90b…`).
- Alpha Team: `first-mission.route` `5fe9a0f0…` to `8ca987d2…`, `menu.route` `5a9a6b3c…` to
  `d237d472…`, `select-adventure.route` `3a6da532…` to `27585089…`, `click-the-intro-300.route`
  `02fd93af…` to `2ca490be…`, `click-the-intro-600.route` `405fa26d…` to `d67fb670…`, and the runs
  to 100,000 (`b37a2a14…` to `6bebf32f…`), 200,000 (`c0749233…` to `4f6172f6…`), 2,000,000
  (`a799b066…` to `e37ef1e8…`), 600,000,000 (`da5f48ed…` to `7b40ea29…`) and 1,100,000,000
  (`ff247d41…` to `920a2827…`).
- LEGO Island: its boot (measured to 435,000 `92babebb…` to `dfd1ac5e…`, before #117 moved its
  tests to 478,000), every call it logs the same, and the run to 1,000 `66dd6743…` to `20202003…`.

Accepted by the maintainer on 2026-10-20, with Windows 95's word as Wine's `0xC0000004`.

What LEGO Island asks between its retained mode and its world's first mesh (#118), each written
when its run stopped there:

- `GetWindowLongA(GWL_HINSTANCE)` answers the instance `CreateWindowExA` was given.
- `LoadAcceleratorsA` looks for a table of keys where Windows does, among the resources the
  module's own headers point to. `ISLE.EXE`'s are cursors, icons and its version, so it answers
  none, with `ERROR_RESOURCE_TYPE_NOT_FOUND`. A module that holds a table stops the run.
- `VirtualQuery` answers from what `VirtualAlloc` and `VirtualFree` now write down: each region's
  start, size and protection, and which of its pages are committed and how. It gives the run of
  like pages from an address, as SmartHeap asks before it gives pages back; an address outside
  every region stops the run. Nothing a title reads changes from the writing down.
- `IDirect3DRM2::CreateTextureFromSurface` makes DirectX 5's `IDirect3DRMTexture2`: DirectX 6's
  texture as far as `GenerateMIPMap`, but for `Changed`, which takes two flags.
- `IDirect3DRM2::CreateMaterial` makes DirectX 5's `IDirect3DRMMaterial`: DirectX 6's material
  without its ambient colour.

`core/tests/shims.rs` asks all five. No other title's route moves.

Rock Raiders is handed DirectX 6's Direct3D, as Windows 98 with DirectX 6 hands it (#132). Its
mode dialog asks its DirectDraw for `IDirect3D3` to list the devices it can draw with. This
machine refused, so the list said the title's own "Error: DirectX6 not installed." and the title
went on with no device, reading its flags through a null pointer at `0x004784be`. That read gave 0
until the bottom of the address space faulted, below.

- `IDirectDraw4::QueryInterface` answers `IID_IDirect3D3` (Wine's `d3d.h`) with the same object's
  `IDirect3D3`.
- `IDirect3D3::EnumDevices` offers the declared machine's one device, the `Direct3D HAL` that
  DirectX 5 and 7 already enumerate, in DirectX 6's `D3DDEVICEDESC`. That is 0xfc bytes, as Wine's
  `d3dcaps.h` has it: DirectX 5's fields, then the texture repeat, anisotropy and blend stages that
  DirectX 7's description gives. One enumeration serves DirectX 5 and 6.

The title now lists the device and hands its identifier to `IDirect3DRM3::CreateDeviceFromSurface`.
It asks `SetSceneFogMethod` for table fog rather than vertex fog. This machine's retained mode
takes neither further, and every hundredth frame of `first-mission.route` to flip 9,100 is the
same picture as before. Every end line is unchanged. The digest of each run past the dialog,
about 39.7 million instructions in, moves:

- `first-light.route` `d0504186…` to `322ab7a0…`, `first-mission.route` `5c74c51c…` to
  `74b1031b…`, `level-select.route` `82a0c841…` to `49e57d12…`, `click-the-intro.route`
  `e71381ad…` to `b1c537af…`, `skip-the-intro.route` `ed8f2de4…` to `315d3880…`.
- The runs with no route to 61,000,000 (`f820fc34…` to `3e8b8a38…`) and 400,000,000
  (`f0cc0561…` to `6714ce40…`).

The run to 100,000 does not reach the dialog and does not move. No other title's route moves.
Accepted by the maintainer on 2026-10-20.

The bottom of a title's address space answers as Windows 9x's does, rather than reading as zero
(#132). LEGO Island read through a null pointer at address 8 and carried on with a 0 (#114).

- A read or a write in the first 4 KiB is an access violation (`0xC0000005`): Windows 9x's
  Win32 data selector refuses those addresses so that a null pointer faults (Pietrek, *Windows 95
  System Programming Secrets*). The record says whether it was a write and which address, as
  `EXCEPTION_RECORD`'s documentation has it. It goes to the guest's own handler chain through the
  dispatch `RaiseException` already used (`kernel32::dispatch`).
- A handler that carries on (`ExceptionContinueExecution`) gets the processor back as the context
  now says, every register and the flags, at its `Eip`. If nothing on the chain takes it, the run
  stops at a wall naming the access, the address and where it faulted.
- The rest of the first megabyte is MS-DOS's memory, which Windows 9x lets a Win32 process read
  and write, and code run from the bottom is not refused there either. This machine has neither,
  so each is a wall that names the address.
- A shim handed a pointer down there stops at a wall that says so, where it read or wrote
  whatever was there. A borrow of no bytes is always allowed, since Alpha Team's rasteriser
  borrows none at a null texture. The arena's bounds check is one subtraction longer
  (`Arena::with_floor`). The conformance corpus's arena, a 386 in real mode, has no floor.

`core/tests/access_violation.rs` proves it on hand-assembled guests, handled and unhandled.

Retained mode keeps the meshes LEGO Island builds its world from (#119), about four hundred of
them out of `WORLD.WDB`, each written when its run stopped there:

- `IDirect3DRM2::CreateMesh` makes the same `IDirect3DRMMesh` as DirectX 6's. The scene keeps
  each mesh's groups (`scene::Mesh`, `scene::Group`).
- `AddGroup` copies a group's faces and numbers it in turn. `SetVertices` copies its
  `D3DRMVERTEX`es, as the guest's bits, out of the guest's array, which Island reuses for the
  next group. Faces of differing sizes, a vertex past the group's end, or a group the mesh lacks
  are a wall or `D3DRMERR_BADVALUE`; every group at once (`D3DRMGROUP_ALLGROUPS`) is a wall.
- `SetGroupColor`, `SetGroupColorRGB`, `SetGroupMapping` and `SetGroupQuality` keep what they are
  given. `SetGroupMaterial` and `SetGroupTexture` keep a reference to what the group is drawn
  with and let go of what it was drawn with before.
- `GetBox` answers the least and greatest corner of every vertex in the mesh, picked by
  comparing. A vertex never set, or a mesh with none, is a wall.
- `Clone`, as `IID_IDirect3DRMMesh` and nothing else, makes a mesh of its own with the same
  groups.
- Nothing reads a group back yet: Island never calls `GetGroup`, `GetVertices`, `GetGroupCount`
  or a group's getters, and they stay walls.

Past the meshes, the same run stopped at two more:

- `IDirectSound3DBuffer::SetConeOutsideVolume` answers as done. A cone starts all the way round
  and nothing narrows one (`SetConeAngles` is a wall), so the volume outside it is never heard.
- `IDirect3DRMViewport::ForceUpdate` has nothing to copy, since a viewport here draws straight
  into its device's surface. Part of a viewport outside it is a wall.

Island now runs to 3,770,493,606, flip 3200, where its viewport's `Clear` is #120's.
`core/tests/shims.rs` builds meshes and forces an update. No other title's route moves.

Retained mode draws its own meshes, lit and textured (#120), which is how LEGO Island draws its
Information Center's actors. Each was written when Island's run stopped there:

- DirectX 5's `IDirect3DRMViewport::Clear` takes no flags and clears everything, as Wine's d3drm
  forwards it (`D3DRMCLEAR_ALL`): the viewport's part of the surface to its camera's scene's
  background, as DirectX 6's already did for Rock Raiders, and now the depth buffer under it too
  (`d3d::clear_depth`).
- `Render` draws a mesh hung on a frame itself, group by group, through the immediate-mode
  device underneath, with the projection and the rasteriser Rock Raiders' user visuals already
  draw through (`d3d::rasterise`, taken out of `DrawPrimitive` unchanged). The device's quality
  is the most a group's is drawn with: flat or Gouraud, lit or not; points, wireframe and Phong
  are walls. A face is in front when its corners go round clockwise, and one turned away is not
  drawn. A lit vertex takes the group's colour from the scene's lights and the material's
  emissive colour, and a material's highlight (Direct3D's specular term, with the viewer as far
  away as can be) is added over the texture. A group's colour with alpha is blended over what is
  behind it. A scene of both meshes and user visuals is a wall.
- A frame can be a visual of another, which draws its tree again wherever that frame is: Island
  hangs every model on a frame of its own and adds that frame to the one that places it.
- `IDirect3DRM2::CreateTextureFromSurface` keeps the surface, with a reference to it, and a
  texture's texels are read from the surface each time it is drawn, never copied. So
  `IDirect3DRMTexture2::Changed` has nothing to do.
- `CreateMaterial` keeps its power, and `SetSpecular` and `SetEmissive` their colours. A
  material starts with a white highlight, as Wine's d3drm makes one (`scene::Material`).
- `IDirect3DRMMesh::GetGroupTexture` hands back DirectX 3's `IDirect3DRMTexture`, as the header
  has it, which Island asks for `{120f30c0-1629-11d0-941c-0080c80cfa7b}`: that is
  `IID_IDirect3DRMTexture2` in Wine's `d3drmobj.h`, the same texture's DirectX 5 face.
- A null reference frame is the world, whatever the frame hangs from. It was a wall for a frame
  with a parent.

`SetField` is the half-width of the view at the front plane, which the projection already was:
Island's front plane is a tenth of a unit away, where Rock Raiders' a unit away hid it.

The projection still stretches the field to the viewport's own edges, fitted to Rock Raiders
(#29). Retained mode's documented default is uniform scaling, and Wine's tests hold a native
viewport to one scale across and down; Island's minifigures look a third too wide without it.
Rock Raiders never calls `SetUniformScaling`, so the two titles cannot be told apart, and uniform
scaling would move every Rock Raiders route. That is left to the maintainer.

Island goes on into its Information Center and its idle loop, past 19 billion instructions with no
wall. `core/tests/shims.rs` clears and draws a lit mesh, and a textured one whose surface changes,
and puts a frame in the world through a null reference. No other title's route moves.

`RaiseException`'s `CONTEXT` had `EFlags` and `Esp` one field late, at 0xc4 and 0xc8 (#133).
`winnt.h`'s i386 `CONTEXT` (Wine's `include/winnt.h`) has `EFlags` at 0xc0, `Esp` at 0xc4 and
`SegSs` at 0xc8, so a handler reading the stack pointer out of it read the flags, breaking golden
rule 4. A fault's context already had them there. The one writer in `kernel32::dispatch` now puts
them there for both.

Only Alpha Team raises through `RaiseException`: its C++ throws, `0xe06d7363`, eight of them
before its 20 millionth instruction. No handler on its chain reads or writes any byte of the
context: watched on `menu.route` to 45.7 billion instructions, no guest instruction touches one.
So no title takes a different turn and every end line is unchanged. The contexts are on the heap
the digest hashes, so the digest of every Alpha Team run that throws moves:

- `first-mission.route` `8ca987d2…` to `1b7d7349…`, `menu.route` `d237d472…` to `f34a24e8…`,
  `select-adventure.route` `27585089…` to `87323b4d…`, `click-the-intro-300.route` `2ca490be…` to
  `a71a75d7…`, `click-the-intro-600.route` `d67fb670…` to `00c2662d…`.
- The runs with no route to 600,000,000 (`7b40ea29…` to `198706f8…`) and 1,100,000,000
  (`920a2827…` to `1a08e772…`).

The runs to 100,000, 200,000 and 2,000,000 throw nothing and do not move, and no other title's
run or stub's moves. `core/tests/access_violation.rs` reads both fields back from a handler
`RaiseException` called. Accepted by the maintainer on 2026-10-20.

Retained mode projects with uniform scaling, as d3drm does by default (#134). A viewport's front
plane, its field or `SetPlane`'s, reaches the edges of the viewport's longer side, and the shorter
side shows as much of it as the same scale fits, so a square stays square: a field of 0.5 on 640
by 480 is half a unit across and 0.375 up. Since #29 the plane was stretched to all four edges.

- Retained mode's documentation: uniform scaling scales "the viewing volume into the larger
  window dimension" (`GetUniformScaling`), and "the viewing angle is calculated using half the
  height or half the width of the front clipping plane, whichever is larger" (Viewing Frustum).
  Wine's tests hold native retained mode's own viewport underneath to one scale, the longer
  side's half.
- Frames of both titles on Windows agree. Rock Raiders' Chief in its retail Mission Brief is as
  tall as this draws him, where the stretch drew him a quarter shorter, his head wider than tall. LEGO
  Island's two plants stand on the same pixels as in the game on Windows 10 with its own d3drm,
  in front of the stage's lip, where the stretch stood them eighteen pixels higher.
- #29 fitted the stretch to an earlier emulator's frames, which stretched too. The dark walls
  and black edges it saw were its spot lights' halved cones, fixed in the same change.
- `SetUniformScaling(FALSE)`, which scales the view into the window across and up apart, is a
  wall: no title asks for it. `core/tests/shims.rs` holds it to that.

Every end line is unchanged. The digests of the runs that draw a viewport wider than it is tall
move:

- Rock Raiders' `first-mission.route` `74b1031b…` to `4888818a…`.
- LEGO Island's `information-center.route`, `2f2d5411…` to `69bd9a2e…`.

Rock Raiders' other runs, its menus and level select included, and every other title's, do not
move. Accepted by the maintainer on 2026-10-20; Rock Raiders' first mission is played again by a
person before its next release (golden rule 7).

What LEGO Island asks for once a hand is on it (#124), each written when a run driven by a
synthetic hand through the host stopped there:

- `IDirect3DRMViewport::Pick`: every mesh the viewport draws over a place on the device, nearest
  first, each found where `Render` would draw it, cut to the view volume and culled as it is, and
  picked once at its nearest face. `IDirect3DRMPickedArray` hands each one out with its visual,
  an `IDirect3DRMFrameArray` of the frames from its scene's root down to the one it hangs on (a
  frame hung as a visual among them), and its `D3DRMPICKDESC`: face, group, and where in the scene
  the place picked is. The game asks at every press of the button and takes the first frame below
  the root that carries its own data as what was pressed. This was the old tree's live crash: it
  had no `Pick`, and the first press ended the run, even in the black seconds before first light,
  which a person saw as the game crashing right away and no route, which never pressed a button,
  could see. This tree stopped at the same wall, at instruction 942,097, flip 102.
- `IDirect3DRMFrame2::GetLights` and `IDirect3DRMLightArray`: the lights hung on a frame as they
  are now. Signing the registration book recolours a light through it.
- `IDirectDrawSurface::Blt` with `DDBLT_ROP` and the raster operation `SRCCOPY`, which is a copy;
  any other operation is a wall. The registration book draws its blinking cursor this way.
- `GetAsyncKeyState`, as Windows 95 answers it: the top bit while a key is down, the bottom bit if
  it went down since it was last asked about, which asking clears; no left key told from a right
  one; the two buttons as keys too. The game asks about the arrows before it believes
  DirectInput's word that they are held.

`ComState::FrameArray` is now `ComState::Array`, a frame array's frames or a light array's
lights. Still not sent: `WM_TIMER` (#138). LEGO Island sets a timer at every press outside and
kills it at the release, and a button held still for a second starts a drag on Windows with the
timer's message; Hot Wheels sets a one-second timer as it starts, so sending the message moves
every Hot Wheels run, which is the maintainer's to decide. Nor is LEGO Island's own
`PostMessageA` to itself, which sets its pointer's shape (#139). The joystick's `joyGetPosEx` and
`joyGetDevCapsA` are not reached: `UseJoystick` is not declared and the game's default is off.

No other title's run moves.

`PostMessageA` posts, where it answered success and posted nothing, which golden rule 4 forbids
(#139). The message waits in the queue of the thread that made the window, stamped with the
declared time and the pointer as it is posted, and `PeekMessage` takes it as Windows documents:
after whatever was sent or posted before it, before every key and mouse message, however long those
have waited. Only the thread that made the window takes its messages. A message below `WM_USER`,
or to any window but the title's own, is a wall: which of the system's carry pointers, and so
cannot be posted, is not written. No title calls `PostThreadMessageA`.

And a title's pointer shapes reach the host (#139). `LoadCursorA` loads a cursor out of a module's
own resources, its `RT_GROUP_CURSOR` and the `RT_CURSOR` the group names, once, as `LoadCursor`
shares what it loads; a group of more than one cursor, or a cursor that is not paletted, is a
wall. Windows' own arrow is answered with the handle every title had before. `SetCursor` sets the
shape and answers the one before; a handle that is not a cursor is a wall. At the flip the host is
told, through a new `Host::pointer_shape` whose default does nothing: macOS sets an `NSCursor` of
it over the picture while the window is in front, Windows makes it the window class's cursor
while the window is in front, and X defines it on the window while the window has the keyboard.
Each gives the arrow back on focus loss, and every host is given it back when the run ends (#111).
The headless host records what it was told. The Windows and X backends are compiled and their
bitmaps tested on every host, not run.

LEGO Island posts itself `WM_ISLE_SETCURSOR`, 21 times on its ride, and its window procedure answers
with `SetCursor`: the hourglass as it starts, the arrow, the hourglass again, no pointer for the
intro, and the arrow and the hourglass as it loads and is clicked. Its runs move, and only its:

- `first-light.route` `47ad2868…` to `e753e559…`; first light is still flip 460, now at
  instruction 97,000,702 where it was 97,000,222.
- `information-center.route` `69bd9a2e…` to `93ac795c…`; its idle loop still starts at flip
  5099, now at 3,952,209,667 where it was 3,952,208,885.
- `first-ride.route`, and the hand's run it was recorded from, `cf079381…` to `e08fb5c1…`. The
  route is unchanged: the hand's run records the same one.
- The restless hand `b172d1ec…` to `c029337f…`, and the boot `31cce8ba…` to `da6e3642…`.

Every run ends on the instruction, flip and declared millisecond it ended on before. Rock Raiders,
Hot Wheels and Alpha Team never post a message, set a cursor or load one but Windows' arrow, and
none of their runs moves.

`MoveFileA`, which LEGO Island reaches when a second name signs its registration book (#125): it
moves every player's saved game down a slot, `G0.GS` to `G1.GS` and on, before it saves the new
player's as the first. A file the guest wrote moves within the profile, where the next run finds
it. A move from nothing fails with `ERROR_FILE_NOT_FOUND`, and one onto a path already there, the
guest's or the pressing's, with `ERROR_ALREADY_EXISTS`, as on Windows; the game carries on past
either. A move the host refuses is told to the person as a refused save is, and the guest is told
it moved. Moving a file a handle holds open, one of the pressing's own, or anything onto the disc
in D: is a wall. `core/tests/old_file_api.rs` holds each. The game writes its saves through its C
runtime, `CreateFileA`, `WriteFile` and `CloseHandle`, and reads them back through `CreateFileA`,
all answered already; its `_hwrite` and `SetEndOfFile` are its scripts' (#116), and a save does not
reach them. No title's run moves.

`PostMessageA` posts `WM_CLOSE`, the first of the system's own messages it posts (#140). It carries
no pointer, so Windows posts it as it posts a title's own. LEGO Island closes its own window this
way when its credits are over (`LegoOmni::CloseMainWindow`), which stopped its runs at a wall at
flip 17338. Its window procedure now takes it: the game is deleted, its `Close` saves the game and
the scores once more, the same bytes the quit door saved, and it leaves its loop, destroys its
window and exits 0, at instruction 13,071,129,548, flip 17341. Every other message below `WM_USER`
is still a wall. No other title posts one, and no committed route reaches Island's, so no run
moves.

## 1.0.0 — 2026-10-18

The engine the three v1 titles run on: an x86 and x87 interpreter for the guest's code, the Win32
and DirectX calls those titles make answered in Rust, a declared machine per title that the title
cannot see past, and a macOS host reached at run time with no external crates (ADR-0001). A run is
replayed from a route, logs what it did, and stops at a named wall when a title asks for something
the machine does not have.
