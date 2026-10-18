# lego-legacy

An emulator that runs the original Windows releases of three late-90s games on a modern Mac,
from your own discs:

| Title | Status |
|---|---|
| LEGO Rock Raiders | **Playable**: a person has played the first mission through |
| Hot Wheels Stunt Track Driver | **Playable**: a person has driven the first race to the finish |
| LEGO Alpha Team | **Playable**: the Training Area plays to its end, and a person has signed it off |
| LEGO Racers | On the roster, not started |

It is not a reimplementation of any game and not a general PC emulator. It runs each title's own
retail executable: an x86 interpreter for the guest's code, and the Win32 and DirectX calls the
title makes answered in Rust. The machine it presents is declared per title (processor, clock,
display, drives, install state) and the title cannot see the host. When a title asks for
something the emulator does not have, the run stops at a named **wall** instead of guessing.

The vocabulary is in [`CONTEXT.md`](CONTEXT.md), the decisions behind it are in
[`docs/adr/`](docs/adr/), and what v1 is and what comes after it are in [`path-to-v1.md`](path-to-v1.md).
A comment tagged `ponytail:` marks a deliberate simplification with a known ceiling, and says
what the ceiling is and the way past it.

## What you need

- A Mac. The window, sound and input are macOS only (AppKit, Core Graphics and AudioToolbox,
  reached at run time). `--headless` needs no window and may build elsewhere; that is untested.
- Rust. `rust-toolchain.toml` pins 1.98.0 and rustup picks it up on its own. There are no
  dependencies: the workspace uses zero external crates, by design ([ADR-0001](docs/adr/0001-zero-crates-and-one-host-seam.md)).
- Your own discs. No game data is in this repository and none ever will be.

## Supplying your discs

Each disc is read as a 2048-byte-sector ISO-9660 image of its data track, placed at
`game-data/<disc>/<pressing>/<pressing>.iso` under the directory you run from (or under
`$LEGO_LEGACY_GAME_DATA`). Only the executable inside is checked, by its SHA-256, so any tool
that makes a plain ISO of the data track works (for example `dd if=/dev/rdiskN of=… bs=2048`
on an unmounted disc).

| Title | Image | Notes |
|---|---|---|
| Rock Raiders | `game-data/rock-raiders/masterpiece-rock-raiders/masterpiece-rock-raiders.iso` | The **Masterpiece** re-release. The original pressing's executable is wrapped in SafeDisc copy protection, which is recognised and refused, never bypassed ([ADR-0006](docs/adr/0006-a-title-may-be-two-pressings.md)). |
| Rock Raiders music (optional) | `game-data/rock-raiders/rock-raiders/rock-raiders.cue` and `.bin` | The original pressing's audio tracks, as one raw 2352-byte-sector `.bin` with its `.cue`. Without it the game runs without music. |
| Rock Raiders original (tests only) | `game-data/rock-raiders/rock-raiders/rock-raiders.iso` | The original pressing's data track. Needed only by the tests that check its wrapped executable is named and refused; the game never runs from it. |
| Hot Wheels Stunt Track Driver | `game-data/hotwheels-stunt-track-driver/hotwheels-stunt-track-driver/hotwheels-stunt-track-driver.iso` | |
| Alpha Team | `game-data/alpha-team/alpha-team/alpha-team.iso` | The original pressing. |

Running the launcher with no arguments lists each title and says whether its image was found
and is the right one. Saves land beside the rip, in `game-data/<disc>/profile/`.

## Playing

```sh
cargo build --release
cargo run --release -p lego-legacy-launcher            # lists the titles, asks which one
cargo run --release -p lego-rock-raiders -- --audio    # or start a title directly
```

Run from the repository root. Each run writes a directory under `runs/` holding its log (and,
with `--record`, the inputs you gave, as a route that replays the same run). A run that stops at a
wall also leaves a `wall` report there: what was asked for, where, and the inputs that reached it,
with a snapshot taken at the wall. Useful flags, all listed by `--help`:

| Flag | |
|---|---|
| `--audio` | let the sound reach your speakers (off by default) |
| `--record` | keep this run's inputs as a replayable route |
| `--route <file>` | replay a route; committed ones are in `titles/<title>/routes/` |
| `--screenshot <flip,…>` | write those frames as PNGs into the run directory |
| `--headless` | no window, uncapped |
| `--data-root <dir>` | keep saves in `<dir>/profile` instead of beside the rip |

While the window is in front and the pointer is over the picture, a title that draws its own
pointer takes the mouse; Cmd-Tab gives it back. Cmd-Q or Cmd-W ends the run. In Hot Wheels,
Space gets past the splash screens and Escape quits. If the host cannot keep up, the game runs
in slow motion with its sound stretched to match, rather than skipping.

## Tests

```sh
./scripts/ci.sh                      # formatting, clippy, the suite, then the unit tests with overflow checks
./scripts/fetch-corpus.sh            # the x86 conformance corpus (about 1.1 GB, fetched, never committed)
LEGO_LEGACY_REQUIRE_DISC=1 LEGO_LEGACY_REQUIRE_CORPUS=1 ./scripts/ci.sh
```

Tests that need a disc or the corpus skip when it is absent; the two variables turn a skip into
a failure. The disc tests replay the committed routes and check that every replay reaches the
same machine.

Each crate's version is in its own `Cargo.toml` and what changed is in the `CHANGELOG.md` beside
it; a crate that changed since its tag without moving its version fails the suite ([ADR-0007](docs/adr/0007-every-crate-carries-its-own-version.md)).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option, except the Indeo Video 5 decoder (`core/src/iv50.rs` and `core/src/iv50/`), which
is derived from FFmpeg's Indeo 5 decoder and is licensed under the
[GNU Lesser General Public License, version 2.1 or later](LICENSE-LGPL-2.1). See [NOTICE](NOTICE).

Much of this code was written with Claude, an AI assistant; commits it co-wrote say so.

## Not affiliated

This project is not affiliated with, endorsed by or sponsored by the LEGO Group, Mattel,
Microsoft or any other owner of the games or interfaces it works with. LEGO, Rock Raiders and
Alpha Team are trademarks of the LEGO Group. Hot Wheels and Stunt Track Driver are trademarks of
Mattel. Windows and DirectX are trademarks of Microsoft. Indeo, InstallShield, SafeDisc and A3D
belong to their respective owners. No game data, copyrighted game content or Microsoft binaries
are distributed here, and no copy protection is circumvented: you need your own discs.
