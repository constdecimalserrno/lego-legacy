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

## 1.0.0 — 2026-10-18

The engine the three v1 titles run on: an x86 and x87 interpreter for the guest's code, the Win32
and DirectX calls those titles make answered in Rust, a declared machine per title that the title
cannot see past, and a macOS host reached at run time with no external crates (ADR-0001). A run is
replayed from a route, logs what it did, and stops at a named wall when a title asks for something
the machine does not have.
