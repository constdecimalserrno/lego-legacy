# The emulation boundary is the DirectX and Win32 API

A title's own binaries (its executable and the DLLs it ships) run as guest x86. Microsoft's DLLs
(KERNEL32, USER32, MSVCRT, DDRAW, D3DIM, D3DRM, DSOUND, DINPUT, and vendor audio such as A3D) are
written in Rust as shims at their export tables. Run-versus-write is priced per binary: a binary
that the title's own disc ships may run as guest code when that is cheaper than writing it,
whoever wrote it. No binary from the host, and none from a redistributable this emulator would
have to install, is ever run.

## Considered options

- **Running the DirectX redistributable the discs ship**: pushes the boundary down to the display
  driver and kernel, a wider and undocumented surface; rejected.

## Amended for Alpha Team (#51)

This said "no Microsoft binary is ever required or run" alongside the priced-per-binary sentence,
and LEGO Alpha Team is the title where the two stop agreeing. Its cabinet ships `MSVCRT.DLL` and
`MSVCP60.DLL`, and its ten engine DLLs import 284 names from them — 128 from MSVCRT, 156 from
MSVCP60, the second lot almost entirely `std::basic_string` and the C++ exception machinery. None
of that is the title's own code, and writing it in Rust is the largest single piece of work the
title would ask for.

So the priced-per-binary sentence is the rule, and the absolute is gone. What it costs: the loader
that maps, relocates, binds and initialises a PE that was never a shim (#55), which this title
needs anyway for `NiMain.dll` and the nine beside it. What it does not buy: any right to reach for
the host's own `msvcrt.dll`, or to run a redistributable off the disc. The bytes that run are the
bytes the player's own pressing carries, hashed like any other.

`a3dapi.dll` is the same question and is not settled here. It ships in the same cabinet, and
whether it runs or is shimmed at its COM interfaces is decided by the ticket that reaches sound
(#60), which records which was taken and why.

## What a person did, and what the machine may not decide (#66)

A person asked to click past what plays before a game does. Alpha Team's intro takes Escape and
nothing else, and refuses even that for its first thirty seconds while a developer's logo is up.
Two different things were on the table, and only one of them is inside this boundary.

**A click handed over as the key the title listens for is a translation, and it is allowed.** The
person pressed something; the machine says which button. Which key, per title, is a declared value
(`Title::click_in_a_movie`), carried with the measurement that chose it. What the title does with
the key is still entirely the title's own rule — Alpha Team ignores it for thirty seconds, and it
goes on ignoring it.

**Ending a movie the title did not end is not allowed.** Answering `MS_S_ENDOFSTREAM` early, or
letting go of a stream nobody let go of, is the machine lying to the guest about what its own disc
did: the movie is the title's data, played at the title's request, and how long it runs is the
title's decision, not an API this boundary is drawn at. So the unskippable thirty seconds stay.
There is no flag for it, because a flag is the decision taken with a default rather than not taken:
if a person later wants those thirty seconds cut, that is a new amendment here with its own reason,
not a switch that was left lying around.

## Consequences

- A binary that runs as guest code is declared, per title, like everything else the machine is
  told: which file, from which pressing, and the reason running it was cheaper than writing it.
- The shim tables stay the boundary for everything the disc does not ship. A title that imports
  KERNEL32 still meets Rust, because no disc on the roster ships KERNEL32.
- An import no shim implements is still a wall. A DLL the tree does not hold is still a refusal.
