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
