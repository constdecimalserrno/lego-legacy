# The golden rules

lego-legacy is a preservation effort: it lets anyone play late-90s games again from their
original discs, unchanged, on any computer, where even period Windows emulators struggle, on a
Mac most of all. It is a framework of per-title emulators, not a Windows emulator.

These rules outrank everything else in the repo: the glossary, the ADRs, every ticket and every
other doc. When two pull against each other, the higher one wins.

1. **Everything is deterministic, and the machine is a state machine we control completely.**
   Every input a game sees is declared: the clock, instruction speed, processor, date, display,
   drives and thread order, never read from the host. The same route reaches the same state on
   any host, and any run can be paused, stepped, snapshotted and replayed. Why: a preserved run
   must be reproducible exactly, by anyone, years from now.
2. **The game runs exactly as it shipped.** Its own code from its own disc runs unchanged;
   nothing in it is patched, replaced or faked. Whatever the emulator lacks stops the run at a
   named wall, never a guess. Why: this is preservation, not a remake.
3. **Extras sit on top, never inside.** Mods, cheats, trainers, editors and TAS tools are welcome
   as a separate layer over a working core, added or removed with no effort, and with no change
   to how the unmodded game runs.
4. **A per-title emulator, true to Windows.** Each title gets only the Windows and DirectX
   behaviour it uses, written when a wall names it, and answered the way real Windows answers:
   from public documentation, headers and observed behaviour, never Microsoft's code. Refuse
   rather than fake success. Core is keyed to no title; what is title-specific is declared data
   in that title's crate.
5. **Runs anywhere, depends on nothing.** Zero external crates, and the host operating system
   sits behind one seam, so a new platform is one new backend. Why: the games must outlive any
   one operating system.
6. **Bring your own disc.** The player supplies an image of their own disc; no game data is ever
   distributed; copy protection is recognised and refused, never bypassed. We do not condone
   piracy.
7. **Nothing is released until it is proven three ways.** Before a title's version is released:
   the full CI pass, a full headless run of a level or the game by route, and a person playing
   it and signing off, on the exact build that is tagged.
8. **Everything is written down so anyone can pick it up.** Every crate is versioned and tagged
   on its own, and releases, changelogs, tickets and pull requests carry verbose notes: enough
   that a person or an agent can fork the project and carry on without asking.
