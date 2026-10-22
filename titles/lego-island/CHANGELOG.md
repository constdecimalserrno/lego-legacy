# Changelog: lego-island

One heading per version, newest first. What each part of a title's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.0.1 — 2026-10-21

Took core 1.2.0 (#140), whose `DestroyWindow` tells the window it is going: as the game leaves, its
window procedure is sent `WM_DESTROY` and `WM_NCDESTROY` and hands both to `DefWindowProcA`, so its
own close ends 58 instructions later, its flips unchanged: `first-save.route`, past its end, at
13,071,129,606, and `pizza-delivery.route` at 10,657,537,575. `maintainer-first-play.route` ends on
its budget, before the game closes, and does not move.

A person closing the window is answered as on Windows, with core's close (#140), so it saves the
game as the quit door does, where 1.0.0 lost what was played since the last save. The game hears
`WM_SYSCOMMAND` with `SC_CLOSE`, posts itself `WM_CLOSE`, saves as it closes and leaves. A hand
that signs the book as LEGO, picks Pepper and closes the window at flip 9000, never having taken
the quit door, leaves `G0.GS` and `History.gsi` behind, written as the game closed, and exits 0 at
4,998,298,712; the route its run recorded ends `flip 9000 close`, and replayed it ends on the same
machine and saves the same bytes. In the Information Center with nobody signed, the game leaves
without saving.

A button held still on the island walks (#138), now that core 1.2.0 sends `WM_TIMER`, where 1.0.0
stood still under it. At a press with nothing under the pointer to take it the game sets a
one-second timer (`LEGO1.DLL` 0x1005cfca), and its message, with the button still down and the
pointer not moved past the drag threshold, starts a drag. A new hand goes out of the door as the
ride does, presses above the middle of the screen at flip 12400 and holds still for 100 flips. Its
test runs it to 6,067,000,000: the island stands still for sixty flips, a declared second; the one
`WM_TIMER` comes after flip 12462, the drag killing its timer; and from there the island goes by.
The ride's hand is now that hand out of the door and the ride on from it, the same events, so
`first-ride.route` does not move. No committed route moves, since none holds the button still for
a second: `maintainer-first-play.route` still ends on its budget with the digest the maintainer's
run ended with, `db7bd6b2…`. The restless hand's digest moves, `c029337f…` to `99ce5f30…`: two of
its releases kill a timer that is not there, which is refused with `ERROR_INVALID_PARAMETER`, and
the C runtime keeps that error.

Its declared machine lacks no library (`absent_libraries`, #146): no gate run asks `LoadLibraryA` for one, so no route moves.

Took core 1.2.0's mouse (#141): a player clicks on the picture to give the title the mouse, and
Escape frees it, neither reaching the game. Until that click the island sees no move or button and
the pointer is the host's arrow; after it the pointer, in the title's shape, is kept on the
picture (on macOS, put back each pump, twice a frame). No route records either, so no route moved.

Declares core 1.2.0's `joliet` as `false` (#144), by #142's rule: Joliet is LEGO Chess's alone,
because Alpha Team's runs move with it, until the maintainer decides. Its disc is read by the
primary tree's names alone, as before, so no route moved. Nothing in this title asks for the
Joliet tree: read through it, the disc answers the same names but for case, and none of its runs
moved with it forced on, measured at `7509b49`. Core's `FindFirstFileA` on the CD is now a
wall; this title never searches.

Minimizing its window on macOS, or hiding the application, no longer closes it (#173, core 1.2.0):
it is told nothing new beyond the keys and buttons it held let go, runs on unseen but heard, and is
shown again on restore. No route moved.

Closing its window on macOS with its button no longer leaves the run messaging a window macOS may
have freed while the title leaves (#175, core 1.2.0): the object behind the window is kept until the
process ends. No route moved.

Switching to another Space on macOS frees a captured mouse, as losing the front does, and keys
pressed there no longer reach the game (#176, core 1.2.0). No route moved.

On macOS, quitting from the Dock closes it as the window's button does, so it saves (#178, core
1.2.0), where the process ended at once. A logout or restart closes it too, and is interrupted, to
be asked again once it has left; it saves then unless something else closes it first, such as its
terminal hanging up as the logout quits it. No route moved.

On Windows, signing out, shutting down or restarting closes it as the window's button does, so it
saves (#184, core 1.2.0), where the process ended at once; Windows holds the sign-out, shutdown or
restart on its screen, saying "The game is closing.", while it leaves, and whether it then goes on
by itself, Microsoft does not say. It saves unless something ends it first: "Sign out anyway" (or
its shutdown and restart twins) there, or a long load that has answered no window message for five
seconds. No route moved.

A long run on macOS no longer grows the process's memory (#180, core 1.2.0): what macOS hands back
to be let go later is now let go at every pump, twice a flip. No route moved.

Closing its window on macOS with its button no longer takes the window off the screen at once: it
stays until the game has left, as on Windows (#183, core 1.2.0), so it saves in view. No route
moved.

Declares no floppy drive (#145), core's new required `Declared::floppy` (unreleased; core's next
release is 2.0.0, #163), which is the machine it has always had: C: and D: only. Nothing it sees
moves.

## 1.0.0 — 2026-10-21

Playable (#128). `routes/pizza-delivery.route` plays one whole activity, Nick Brick's pizza from
the Pizzeria to Nubby at the gas station, from the intro to the quit door's save and the game's
own exit (#137). `routes/maintainer-first-play.route` is the maintainer's own play, and replayed
headless it ends on the same machine as their run and saves the same players (#126).

Signed off 2026-10-21 on `b90b1d7`, played by the maintainer at a window with sound on their
MacBook: the book signed, the observation deck, Pepper picked and out on the bicycle to the beach.
The code this release runs is that build's; since then only its tests, routes, docs and version
changed.

Not yet: closing the window ends the run without the game being told, so whatever was played since
its last save is lost; leave by the Information Center's quit door and answer yes to save (#140).
A button held still does not start a drag by itself, since no `WM_TIMER` is sent yet (#138).

What 1.0.0 holds, as it was reached:

On the roster (#113): `ISLE.EXE` from `DATA/disk`, copied whole as the Wise installer copies it,
on a declared machine of its own.

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

Its own close after the credits is answered (#140): core's `PostMessageA` posts `WM_CLOSE`, and the
game's window procedure deletes the game, whose `Close` saves the game and the scores once more,
the same bytes the quit door saved. The game then leaves its loop, destroys its window and exits 0.
`first-save.route`, replayed past its end over a data root of its own, ends with the game's own
exit, at instruction 13,071,129,548 and flip 17341, where it stopped at the wall; a new test holds
it.
Closing the host's window still ends a run without the game being told.

It plays one whole activity by hand (#137): Nick Brick's pizza, from the Pizzeria to Nubby at the
gas station on the skateboard. No core change was needed; nothing on the way walls. The committed
`pizza-delivery.route` is what a synthetic hand recorded through the host. It clicks the intro
through, signs the book as LEGO and picks Nick, the fourth portrait, then goes out of the door and
rides the bicycle down to the foot of the plaza. The Pizzeria is clicked from there at flip 10137,
where it shows just over the handlebars; the game cuts to it, and Papa Brickolini brings a pizza
out. Clicking the pizza while he holds it out takes the job, and from flip 12447 Nick is on the
skateboard beside the Pizzeria with the pizza on its dashboard. Holding Up, with Left for two
turns, it goes east and north into the gas station's road, where at flip 13085 Nubby comes and
takes the pizza. The Information Center's tower, clicked from there, goes back inside; the quit
door saves at flip 15327; the credits roll; and the game closes itself and exits 0 at instruction
10,657,537,517, flip 22041.

Its save says the pizza got there: Nick, actor 4, is the player in `G0.GS`, and his pizza mission
scores red, delivered inside a hundred declared seconds, with nobody else's taken. Its tests run
the hand over an empty data root and look at its frames on the bicycle, on the skateboard, at the
handover and back inside, and at the credits. Replayed over an empty data root of its own, the
route ends on the same machine, presents the same handover and saves the same bytes.

Nick is picked, not Pepper. Each player's pizza goes somewhere of their own, and Pepper's goes to
the jail. The road there from the town has road works across it, two workers and their fences,
for the whole 350 declared seconds he has; the other way, down the ramp from the Information
Center's plaza and round the police station, is more than four times as long as Nick's.

Outdoors the game runs at 2.3 declared seconds a wall second on this machine headless, frames
converted and presented included: a person's window has twice what it needs.

A person has played it (#126). The maintainer played it by hand at a window on their MacBook,
with `--audio --record` on `b90b1d7`, and signed it off on 2026-10-21. The committed
`maintainer-first-play.route` is the route that run recorded, as Hot Wheels' `first-race.route`
is the maintainer's own play, with a header saying what was done. The intro is clicked through
and the book signed as OO; the play goes round the Information Center's rooms and up to the
observation deck, whose controls turn the sky pink, orange, dark and back to a light orange. Back in the main
room it picks Pepper, goes out of the revolving door, gets on the red bicycle and rides it down to
the beach, where the window was closed at instruction 3,341,815,783, flip 10667, 274,650 declared
milliseconds in. No activity is started; `pizza-delivery.route` is the one played through.

A route keeps what a person did, not the window closing, so the gate holds the replay to that
instruction by its budget. Replayed headless on another Mac, it ends at the same instruction,
flip and declared time, with the digest the MacBook's run ended with, `db7bd6b2…`, and the same
`Players.gsi`: the end line says `budget exhausted` where the play's said `stopped by the person`.
Their play began from an empty profile, so a replay over an empty data root changes nothing, and
closing the window does not tell the game, so the players are all it saved. Its tests find Pepper
picked, the bicycle at the beach under the sky the deck left, and two replays side by side ending
on the same machine with the same saves.
