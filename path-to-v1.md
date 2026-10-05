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

### Alpha Team to playable — in order

1. **#85** A click on a unit does not open its Unit Menu. The click reaches
   `DDMouse::ProcessMouseClick` and is never asked what was clicked; this is #61's wall.
2. **#61** The first mission is played to its end by a committed route, inside a budget,
   with the determinism gate over it.

### Alpha Team, alongside

- **#87** `KERNEL32!GetDateFormatA` is a wall a click in the front end reaches (a screen
  showing a date, under *Single Missions*). A player can hit it, so it ships fixed.

### The sign-off play — needs the maintainer at a window

- **#62** Play Alpha Team's first mission and say what is wrong. It closes these, which are
  all fixed and waiting only on a person:
  - **#86** the drawn pointer matches the hand (`56dd545`, `417a7ba`)
  - **#64** the pointer is put back after the hand leaves the window (`cb71023`)
  - **#65** front-end pace, now 0.86× (was 0.42×)
  - **#75** a slow host stretches the mix instead of starving the device; a run now ends
    with `audio: the device ran dry N times`, which is the number to read

Play with `cargo run --release -p lego-alpha-team -- --audio --record`. Never run
`target/release/<title>` in the tree: builds land in `CARGO_TARGET_DIR`, and that path is stale.

## Cutting the release

1. #85, #61, #87 closed; `LEGO_LEGACY_REQUIRE_DISC=1 ./scripts/ci.sh` green.
2. The conformance corpus is fetched (`scripts/fetch-corpus.sh`) and
   `LEGO_LEGACY_REQUIRE_CORPUS=1` passes: without it those tests skip silently.
3. On that build, the maintainer plays all three — #62 for Alpha Team; a first mission for
   Rock Raiders and a first race for Hot Wheels, since both were signed off before the
   pointer, pace and audio changes for Alpha Team touched the shared host.
4. Tag `v1.0.0`.
