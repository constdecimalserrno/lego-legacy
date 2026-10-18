# Path to v1

**v1 is three titles _playable_** (CONTEXT.md: the first unit of play completes by a replaying
route, and a person drove it and said so), signed off by the maintainer on the release build:

| Title | Playable | Signed off |
|---|---|---|
| `lego-rock-raiders` | a person played the first mission through (2026-09-10); `routes/first-mission.route` covers its first minute, and a route through the whole mission is #106 | 2026-10-18 |
| `hotwheels-stunt-track-driver` | `routes/first-race.route`, the maintainer's play, drives the first race to the finish | 2026-10-18 |
| `lego-alpha-team` | `routes/first-mission.route` plays the Training Area to its end | 2026-10-18 |

Out of v1: `lego-racers`, the *complete* and *credits* milestones, goldens.

## After v1

- **#106** A route that plays Rock Raiders' first mission to its end. v1 ships with the
  one-minute route, by the maintainer's choice.

## Done on the way

### The sign-off plays — 2026-10-18

The maintainer played all three on `c770a73`, the release build, and signed each off: #62, #88,
#89. Hot Wheels' play, the first race to the finish, is its committed route (`0b682da`). Rock
Raiders' reached the level select, so its route through the whole mission waits on #106.

### Every crate versioned — 2026-10-18

- **#105** (ADR-0007) Each crate carries its own version, 1.0.0 at v1, and a `CHANGELOG.md`; a
  title and the launcher pin core exactly; the suite refuses a crate that changed since its tag
  without moving its version; a run's log names the versions that made it.

### Alpha Team by route — 2026-10-04

- **#85** (`af81a39`) The Unit Menu was never a machine bug: the training lets only its
  coach be picked until step 220a, and no route had taken its first step.
- **#61** (`d23364e`) `routes/first-mission.route` plays the Training Area to its end and
  back to the main menu (18.42 G instructions), with the determinism gate over all of it.
- **#87** (`3d282e6`, `d33e170`) `GetDateFormatA`/`GetTimeFormatA`: the wall was Select
  Adventure (main menu → *Adventure*) labelling a saved slot with its date.

### Alpha Team's narration — 2026-10-17

- **#92** (`cebf92d`) `IDirectSoundBuffer::Lock` hands out the part of a lock that runs past
  a buffer's end. Every line the coach finishes is heard whole (#90 measured 41% of his
  narration reaching the mix), the music streams play without a skip, and
  `routes/first-mission.route` was re-taken at his pace: back at the main menu at flip 7990
  (23.22 G instructions), with `m0010_020a` gated at least 310 flips after `m0010_010`.
- **#99** A volume, pan or frequency change leaves a voice and its play cursor on the sample
  they were on.

### Cleanup before launch — 2026-10-17

Nothing a title sees moved unless named here.

- **#90**, **#91**, **#93**, **#94**, **#97**: the underrun count is always printed, one
  declared date, six guest values corrected, one run loop, and docs that match the launcher.
- **#95** What nothing calls is gone, and each shim and method is one line.
- **#96** Each title's declaration lives in its own crate, and its tests test that one.
- **#98** The digest and a snapshot see every page the guest can write: Alpha Team's level
  images, the whole main stack, the thread and process blocks. Every digest moved.
- **#100** A damaged rip, a wrapped pressing or a refused save each say so; Cmd-Q and Cmd-W end
  the run.
- **#101** A disc test is gated on the rip it reads and fails when that rip is required.
- **#102** What was written twice is written once.
- **#103** Comments and docs say what the code does.
- **#104** `cpuid` and `GetSystemInfo` answer as each title's declared processor.

## Cutting the release

1. ~~#85, #61, #87 closed; `LEGO_LEGACY_REQUIRE_DISC=1 ./scripts/ci.sh` green.~~ Done 2026-10-04.
2. ~~The conformance corpus is fetched (`scripts/fetch-corpus.sh`) and
   `LEGO_LEGACY_REQUIRE_CORPUS=1` passes.~~ Done 2026-10-16.
3. ~~On the build after the cleanup, the maintainer plays all three: #62, #88, #89. The Hot
   Wheels play is committed as its route.~~ Done 2026-10-18; Rock Raiders' route is #106.
4. Tag the release commit `v1.0.0`, and each crate at its own (ADR-0007):
   `lego-legacy-core-v1.0.0`, `lego-legacy-launcher-v1.0.0`, `lego-rock-raiders-v1.0.0`,
   `hotwheels-stunt-track-driver-v1.0.0` and `lego-alpha-team-v1.0.0`.
