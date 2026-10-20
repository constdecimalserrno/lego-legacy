# Changelog: lego-legacy-core

One heading per version, newest first. What each part of core's version means is in
[ADR-0007](../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.1.0 — 2026-10-18

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

## 1.0.0 — 2026-10-18

The engine the three v1 titles run on: an x86 and x87 interpreter for the guest's code, the Win32
and DirectX calls those titles make answered in Rust, a declared machine per title that the title
cannot see past, and a macOS host reached at run time with no external crates (ADR-0001). A run is
replayed from a route, logs what it did, and stops at a named wall when a title asks for something
the machine does not have.
