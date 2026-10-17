# Path to v1

**v1 is three titles _playable_** (CONTEXT.md: the first unit of play completes by a replaying
route, and a person drove it and said so), signed off by the maintainer on the release build:

| Title | Playable | Signed off |
|---|---|---|
| `lego-rock-raiders` | a person played the first mission through; `routes/first-mission.route` covers its first minute and is re-taken whole from #88's play | 2026-09-10, on an older build |
| `hotwheels-stunt-track-driver` | a person drove the first race to the finish; `routes/first-race.route` covers its start and is re-taken whole from #89's play | 2026-09-11, on an older build |
| `lego-alpha-team` | by route — the training mission by `routes/first-mission.route`; awaiting #62 | — |

Out of v1: `lego-racers`, the *complete* and *credits* milestones, goldens.

## Open tickets

Every open ticket is on this path, and each is a play at a window by the maintainer.

### The sign-off plays

Each is played with `--record`, and each keeps its run directory: a recorded play that reaches
the end of its unit of play becomes that title's committed route.

- **#62** Play Alpha Team's first mission and say what is wrong. #86 (the drawn pointer
  matches the hand), #64 (the pointer is put back after the hand leaves the window), #65
  (front-end pace, 0.86×) and #75 (a slow host stretches the mix, and a run ends with
  `audio: the device ran dry N times`) are closed as fixed; this play is what confirms them.
- **#88** Play Rock Raiders' first mission to its end.
- **#89** Drive Hot Wheels' first race to the finish.

Play with `cargo run --release -p <title> -- --audio --record`, which builds first. A
`target/release/<title>` started by hand is stale whenever `CARGO_TARGET_DIR` sends builds
elsewhere.

## Done on the way

### Alpha Team by route — 2026-10-04

- **#85** (`0d118d2`) The Unit Menu was never a machine bug: the training lets only its
  coach be picked until step 220a, and no route had taken its first step.
- **#61** (`858f687`) `routes/first-mission.route` plays the Training Area to its end and
  back to the main menu (18.42 G instructions), with the determinism gate over all of it.
- **#87** (`1f2dddf`, `399d2fb`) `GetDateFormatA`/`GetTimeFormatA`: the wall was Select
  Adventure (main menu → *Adventure*) labelling a saved slot with its date.

### Alpha Team's narration — 2026-10-17

- **#92** (`5d3b8ac`) `IDirectSoundBuffer::Lock` hands out the part of a lock that runs past
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
3. On the build after the cleanup, the maintainer plays all three: #62, #88, #89. The Rock
   Raiders and Hot Wheels plays are committed as their titles' routes.
4. Tag `v1.0.0`.
