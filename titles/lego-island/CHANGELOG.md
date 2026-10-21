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

Its frames start and its world starts to load (#118), the way to first light. The engine lets go
of the whole screen it has just cleared at instruction 458,966, flip 0, and from then on copies
its back buffer onto the screen once a frame. After two hundred of those, counted by `ISLE.EXE`,
it opens its scripts where they lie on the disc, then `WORLD.WDB`, and makes its textures and its
first material through DirectX 5's retained mode. Every frame so far is black. At 5,560,671 it
asks retained mode for its first mesh, which is #119's; first light is past the world it builds.
The boot tests run to 5,560,600.

Its world is built and its intro plays (#119). Retained mode keeps the 400 meshes it makes out of
`WORLD.WDB`, with their 1,280 groups, and their boxes and clones. Past them, its 3D sounds set the
volume outside their cones (flip 368), and its viewport is forced to update (flip 455). Then the
bricks fade up and the intro plays, until 3,770,493,606, flip 3200, where retained mode clears its
viewport to draw the island, which is #120's. The traced boot tests run to 5,571,000, past its
first two meshes. The world runs untraced to 3,770,493,500, twice, for the determinism gate.

First light is reached (#118). With nothing pressed, by the committed `first-light.route`, the
game presents its first picture of its own at flip 460, instruction 97,000,222, 8,766 declared
milliseconds in: the first instant of its LEGO bricks fading up out of black, four colours all
within 8/255 of it. First light is the first frame that is not one colour; flip 0, the engine's
cleared screen, and the loop's flips 1 to 459 are all black. The route runs to 98,000,000, twice,
for the determinism gate.

Its Information Center is drawn (#120). At flip 3200 retained mode clears the viewport, the room
is painted over it, and retained mode draws the room's actors in front: the two plants on the
stage at once, and from flip 3241 the Infomaniac, lit and textured, walking about. The run goes on
into the Information Center's idle loop with no wall. The world runs to 3,800,000,000, flip 3437,
twice, and its test finds the plants green on the grey stage in the first frame.

Its Information Center is reached, and it holds its idle loop there (#121). With nothing pressed,
by the committed `information-center.route`, the Infomaniac walks in and talks, then stands by the
right rail while the room idles, and at flip 4937 walks over beside the info desk by himself. From
flip 5099, instruction 3,952,208,885, 181,983 declared milliseconds in, the room goes round one
loop of 120 frames, 88 still and 32 in which the brick on the desk rocks, for as long as nobody
touches it. The loop's first frame is the milestone. The world run is now this route, to
3,965,000,000 and 5,254 flips, twice, for the determinism gate, and its test finds the loop come
round a period later, without the frame before it. Nothing calls `SetTimer` on the way, nor by
instruction 5,000,000,000.

Its plants stand where the game draws them on Windows, in front of the stage's lip, now that core
projects with uniform scaling as d3drm does (#134), and the Infomaniac is in proportion, where he
was a third too wide for his height. The first-frame test looks for the plants' green pots there.
`information-center.route`'s end line is unchanged and its digest moves, which core's changelog
lists.

It is heard (#123), with nothing new in core: DirectSound already mixed what it plays. Measured
against the disc's scripts, the route's mix carries every wave the game plays, sample for sample,
at the volume the game gives it. The intro is three waves out of `INTRO.SI`, the LEGO logo's from flip 459,
Mindscape's from 878 and the intro movie's from 1112 to 3196, each streamed whole through a
two-second buffer. In the Information Center, the Infomaniac's speech and the sounds his
animation makes are 3D buffers placed about him, under a listener at the camera. From flip 3938
the room's music, `InformationCenter_Music` out of `JUKEBOX.SI`, plays on under him at the game's
own lowered volume, going round at its end. Its new test hears each part, and the determinism
gate's second replay asks for `--audio` and ends on the same machine and the same mix. No buffer
asks for position notifications, and every position the game reads comes from the declared clock.

It takes the keyboard and the mouse, and the old tree's live crash is gone (#124). The game reads
its keys through DirectInput's keyboard and the window's key messages, and the mouse only through
the window's messages; `Draw Cursor`'s default is off, so the pointer stays the person's and is
never taken. The old tree's live play ended at the first press of the button: the game asks
retained mode which of its 3D objects is under the pointer at every press, even in the black
seconds before first light, and the old tree had no `Pick`. No route had pressed a button, so no
replay saw it. This tree stopped at the same wall, and core now answers it.

A synthetic hand drives the game through the host, as a person's reaches a run in a window, and
not through a route: the pointer glides, a button is held two flips, a key as long as it is held.
A restless hand, circling the pointer and pressing every 97 flips from the first flip, meets no
wall; its clicks skip the intro, and by flip 800 it is in the Information Center. Before #124 its
first press, at flip 98, stopped the run at `IDirect3DRMViewport::Pick`.

The committed `first-ride.route` is what a second hand recorded: out of the Information Center
and away on a bike. In the Information Center it opens the registration book (the blue brick on
the desk), types LEGO on the keyboard, signs with the tick, and picks Pepper. The island lets
nobody out who has not signed and picked, and a new name picks nobody, so the book comes first.
Then the arrow to the elevator, the arrow to the revolving door, and the door: from flip 12100 the
island is seen through the doorway. Outside, Up walks a step, Left turns to face the red bicycle
by the door, a click at flip 13317 gets on it, and Up rides it down the road from flip 13600 to
13800. On the way it met four walls, each answered in core: `Pick`, the book's cursor copied with
a raster operation, signing it recolouring a light through `GetLights`, and `GetAsyncKeyState`,
which the game asks about the arrows. Its tests run the hand to 6,350,000,000 and flip 13,964 and
find the sky through the doorway, the handlebars on the bike, and the island going by. The route
is the one the hand's run recorded, and replayed beside that run it ends on the same machine and
presents the same frame. Its clicks are let go before the game's one-second drag timer is due,
which no title is yet sent (#138).

It shows its own pointers (#139). It loads its arrow, hourglass and no-entry sign out of
`ISLE.EXE`, and sets one by posting its own window `WM_ISLE_SETCURSOR`, which its window procedure
answers with `SetCursor`. Core now posts what was faked before and hands each shape to the host,
which draws it over the picture: the hourglass from flip 2, the arrow at 406, the hourglass at
409, no pointer for the intro from 456. The posted messages are dispatched as they are on Windows,
so every run that reaches the first one moves its digest, and first light and the idle loop come a
few hundred instructions later, at the same flips: first light at 97,000,702, the idle loop at
3,952,209,667. The ride's route is the one the hand's run records still.

It saves its game and loads it on the next run (#125). The saves go where `savepath` says, the
install directory, which in a run is its data root's `profile/C/PROGRAM FILES/LEGO ISLAND`; a
person's is `game-data/lego-island/profile`. Signing the registration book writes the players,
`Players.gsi`. The game itself, `G0.GS` for the first player, and the scores, `History.gsi`, are
written only once a name is signed: when the player leaves by the quit door and answers yes, and
when another name signs the book, both run here; and, by the game's code, when another name is
picked from the book and as its window closes. At boot it reads the players and the scores back;
a player's game is read when their name is picked from the book. They are written through the C
runtime's `CreateFileA`, `WriteFile` and `CloseHandle`, and the one call a save reached that core
did not answer was `MoveFileA`.

The committed `first-save.route` is what a hand recorded: `first-ride.route` as far as picking
Pepper, then the door by the right rail. The Infomaniac asks whether the player is sure, with a
green brick and a red one in front of him; the green one is yes. Signing writes `Players.gsi`
after flip 7113, yes writes `G0.GS` and `History.gsi` after flip 10627, and after flip 10941 the
credits roll. Its tests run the hand to 6,030,000,000, just past flip 11000, over an empty data
root, and find the three saves there: LEGO, his game with Pepper picked, and the scores. Replayed
over a data root of its own, the route ends on the same machine and saves the same bytes.

A second run over the hand's data root loads all three, and each shows:

- The book opens with LEGO already in it, on the row below the empty line a new name goes on,
  where the first run's book at the same flip had nobody (from `Players.gsi`).
- Picking LEGO there puts his name on the board, and Pepper's portrait is framed in blue though
  nothing in this run picked him (from `G0.GS`).
- A second name, ISLE, signs: `MoveFileA`, written in core for it, moves LEGO's game to `G1.GS`
  byte for byte, and ISLE's game is the first, numbered 1, the number the first run's scores
  saved for the next player (from `History.gsi`).

From here, per ADR-0007, a change that stops a save this version writes being read back is a
major version. Its save as its window closes is not reached yet (#140): closing the host's window
ends a run without the game being told, and when the credits end, at flip 17338, the game posts
its window `WM_CLOSE`, which `PostMessageA` stops at as a wall.
