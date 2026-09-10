# lego-legacy

A zero-crate Rust emulator that runs the original retail binaries of four late-90s Win32/DirectX
titles. Not a reimplementation, and not a general PC emulator: the machine it presents is tailored
to each title it supports.

## Language

### Titles

**Title**:
One supported game, identified by the bytes of its entry binary.
_Avoid_: game, ROM, image, guest as a synonym

**Roster**:
The exact set of four titles the emulator supports: `lego-rock-raiders`,
`hotwheels-stunt-track-driver`, `lego-racers`, `lego-alpha-team`. Support is per title, never
inferred from a hardware standard.
_Avoid_: compatibility list, catalogue, platform support, the collection

**Pressing**:
One physical release of a title, original retail or Masterpiece re-release, with its own
executable. A title has one or two, and they are not interchangeable.
_Avoid_: disc, rip, edition, version, release as a synonym for title

**Wrapped image**:
An entry image whose real code is encrypted behind copy protection, shipped
beside the encrypted remainder. Recognised and named, never run, decrypted or
bypassed.
_Avoid_: SafeDisc as a verb, protected, packed

**Red Book audio**:
A pressing's soundtrack, recorded as audio tracks on the disc itself rather
than as files. A title declares which pressing its tracks come from, which
need not be the pressing its entry image comes from.
_Avoid_: CD audio, soundtrack, music as a synonym

**Game data**:
A pressing's own bytes, ripped from the player's disc into `game-data/<title>/<pressing>/`. Never
committed; its absence is a refusal, not a fallback.
_Avoid_: assets, resources, ROM

**Core**:
The engine shared across every title on the roster; nothing in it is keyed to a title.
_Avoid_: platform, framework, common, system

**Launcher**:
The one front door: finds the pressings a player has and starts a title's own binary.
_Avoid_: runner, wrapper, menu, play script

### Milestones, in order

**First light**:
The first frame a title presents under its own control.
_Avoid_: boots, runs, works

**Playable**:
The title's first unit of play (mission, race, level) completes by a replaying route, and a
person drove it from the keyboard and said so.
_Avoid_: works, done, boots

**Complete**:
Every unit of play the title ships is completable from the keyboard, saves included. The bar
every roster title must reach.
_Avoid_: fully playable, finished, done, 100%

**Credits**:
A person reaches the title's credits in a normal session. The goal beyond complete.
_Avoid_: beaten, finished, done

### Speed is five separate things

**Throughput**:
Guest instructions retired per wall-second. A property of the engine.
_Avoid_: speed, performance, MIPS as a bare synonym

**Pace**:
Game-seconds per wall-second. 1.0× is real time.
_Avoid_: speed, game speed, real-time factor

**Frame rate**:
Frames the guest presents per wall-second.
_Avoid_: fps as a synonym for pace or throughput

**Presentation rate**:
Frames the guest presents per game-second; what the guest believes it draws at.
_Avoid_: frame rate, fps, draw rate

**Intended rate**:
The frame rate a title itself authors as its target, read from the title's own data. A title
without one is recorded as unreached, never defaulted.
_Avoid_: target fps, native fps, cap

### The declared machine

**Declared machine**:
The whole virtual PC the emulator asserts to a title: processor, clock, graphics, drives, install
state. Chosen per title, never measured from the host.
_Avoid_: virtual machine, emulated hardware

**Declared value**:
A chosen input the guest reads as fact, carried with the reason for that value and no other.
_Avoid_: config, setting, default, constant

**Declared clock**:
The declared machine's own sense of elapsed time, derived from instructions retired and declared
costs. The guest cannot reach the host's wall clock.
_Avoid_: virtual clock, emulated timer, game clock, host timebase

**Host**:
The real machine and operating system the emulator runs on; everything the guest cannot see.
_Avoid_: platform, native, system

**Shim**:
The Rust implementation of one guest-visible entry point at the boundary.
_Avoid_: stub, hook, HLE function

**Wall**:
A named stop where the guest asked the machine for something it does not have. A run ends at a
wall; it never continues past one.
_Avoid_: crash, unimplemented, TODO, stub

### Runs and evidence

**Run**:
One execution of a title, live or replayed, with its own run directory holding its log.
_Avoid_: session, launch, play

**Flip**:
One presentation of a frame by the guest. The unit a route is keyed to.
_Avoid_: frame as a count, vsync, tick

**Cursor**:
The count of instructions the machine has retired; names one moment in a run.
_Avoid_: timestamp, tick, PC

**Route**:
An input script, authored or recorded, that drives a title from boot to a named state.
_Avoid_: test script, macro, TAS as a noun, session log

**Snapshot**:
The whole machine's state at one cursor, kept so a run can resume there. A cache, never evidence,
never committed.
_Avoid_: save state, rung, checkpoint

**Golden**:
A committed bundle of hashes fixing the emulator's observable output for one route.
_Avoid_: snapshot, baseline, fixture

**Blessing**:
The maintainer's own act of accepting a captured golden as correct. Nobody else's, and no test's.
_Avoid_: approving, signing off, validating
