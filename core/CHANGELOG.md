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

## 1.0.0 — 2026-10-18

The engine the three v1 titles run on: an x86 and x87 interpreter for the guest's code, the Win32
and DirectX calls those titles make answered in Rust, a declared machine per title that the title
cannot see past, and a macOS host reached at run time with no external crates (ADR-0001). A run is
replayed from a route, logs what it did, and stops at a named wall when a title asks for something
the machine does not have.
