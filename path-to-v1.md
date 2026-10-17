# Path to v1

**v1 is three titles _playable_** (CONTEXT.md: the first unit of play completes by a replaying
route, and a person drove it and said so), signed off by the maintainer on the release build:

| Title | Playable | Signed off |
|---|---|---|
| `lego-rock-raiders` | yes — first mission by `routes/first-mission.route` | 2026-09-10, on an older build |
| `hotwheels-stunt-track-driver` | yes — first race by `routes/first-race.route` | 2026-09-11, on an older build |
| `lego-alpha-team` | by route — the training mission by `routes/first-mission.route`; awaiting #62 | — |

Out of v1: `lego-racers`, the *complete* and *credits* milestones, goldens.

## Open tickets

Every open ticket is on this path. Anything else was closed on 2026-10-04 (#40, #50, #68).

### Alpha Team by route — done 2026-10-04

- **#85** (`0d118d2`) The Unit Menu was never a machine bug: the training lets only its
  coach be picked until step 220a, and no route had taken its first step.
- **#61** (`858f687`) `routes/first-mission.route` plays the Training Area to its end and
  back to the main menu (18.42 G instructions), with the determinism gate over all of it.
- **#87** (`1f2dddf`, `399d2fb`) `GetDateFormatA`/`GetTimeFormatA`: the wall was Select
  Adventure (main menu → *Adventure*) labelling a saved slot with its date.

### Alpha Team's narration — before the sign-off play

- **#92** Every narration line loses every other second in the mix, because
  `IDirectSoundBuffer::Lock` never hands out the half of a buffer that wraps.
  #90 measured it: 41% of the training's narration reaches the mix, and the
  device is not to blame (it runs dry at loads, not under the coach). The fix
  re-takes `routes/first-mission.route`, whose steps follow the coach's voice.

### Cleanup before launch — nothing a title sees moves

- **#95** What nothing calls is gone, and each shim and method is one line.
- **#96** Each title's declaration lives in its own crate, and its tests test that one.

### The sign-off play — needs the maintainer at a window

- **#62** Play Alpha Team's first mission and say what is wrong. It closes these, which are
  all fixed and waiting only on a person:
  - **#86** the drawn pointer matches the hand (`56dd545`, `417a7ba`)
  - **#64** the pointer is put back after the hand leaves the window (`cb71023`)
  - **#65** front-end pace, now 0.86× (was 0.42×)
  - **#75** a slow host stretches the mix instead of starving the device; a run now ends
    with `audio: the device ran dry N times`, which is the number to read

Play with `cargo run --release -p lego-alpha-team -- --audio --record`, which builds first. A
`target/release/<title>` started by hand is stale whenever `CARGO_TARGET_DIR` sends builds
elsewhere.

## Cutting the release

1. ~~#85, #61, #87 closed; `LEGO_LEGACY_REQUIRE_DISC=1 ./scripts/ci.sh` green.~~ Done 2026-10-04.
2. The conformance corpus is fetched (`scripts/fetch-corpus.sh`) and
   `LEGO_LEGACY_REQUIRE_CORPUS=1` passes: without it those tests skip silently.
3. On that build, the maintainer plays all three — #62 for Alpha Team; a first mission for
   Rock Raiders (#88) and a first race for Hot Wheels (#89), since both were signed off before the
   pointer, pace and audio changes for Alpha Team touched the shared host.
4. Tag `v1.0.0`.
