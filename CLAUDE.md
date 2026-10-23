# lego-legacy

## The golden rules come first

They outrank everything below and every other doc in this repo. Check each change against them before anything else.

@GOLDEN-RULES.md

## Agent skills

### Issue tracker

Issues live as GitHub issues on `constdecimalserrno/lego-legacy`, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage labels, unchanged: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## The host behaves the same for every title

The maintainer's rule, for consistency: the mouse and the window work the same way in every title.

- **The mouse** (#141): it is the person's until they click on the picture, and that click captures it without reaching the game. Escape frees it, also without reaching the game, and so does the window losing the front.
- **The window**: it opens at 640×480 and is resizable, and the picture scales to fit with its aspect kept and black bars.

A title never gets an exception of its own. A change to either is made once, in `core/src/host/`, for every title.

## Versioning

Every crate and title is versioned and tagged on its own, meticulously, so each keeps its own log and takes patches individually (ADR-0007).

- **Changing a crate**: move its version by ADR-0007's meaning of major, minor and patch (once per release: a crate already past its last tag stays there), and write the change under that version's heading in the crate's `CHANGELOG.md`. A core bump changes each title's exact pin, so it moves every title too.
- **Releasing**: tag each crate whose version moved as `<crate>-v<version>`, and the workspace as `v<version>`. A patch to one title releases on its own.

`core/tests/versions.rs` fails the suite on a missed bump or a missing heading.

## Ticket runbook

Every ticket runs the `ticket-runbook` skill (the maintainer's rule, 2026-10-21): a `task/<NN-slug>` branch named for its GitHub issue, and a pull request. The maintainer signs each pull request off and merges it, unless they have pre-approved an effort's pull requests, as they did LEGO Chess's on 2026-10-21. Then the lead merges each one by fast-forward once its gate and three reviews pass. These are its settings:

- **Service:** the play kit. `lego-ship` builds every binary in the workspace and publishes them in `play-kit/` (git-ignored). The maintainer's MacBook pulls that before every `lego` run; game data and saves stay there.
- **Gate (steps 4 and 6):** the maintainer's rule, 2026-10-22: "Save long CIs for version releases."
  - A pull request runs every step of `scripts/ci.sh` except the four released titles' route replays, which are about 21 of its 22 minutes:
    - `cargo fmt --all -- --check`;
    - `cargo clippy --workspace --all-targets -- -D warnings`;
    - `cargo test --workspace --release --exclude lego-rock-raiders --exclude hotwheels-stunt-track-driver --exclude lego-alpha-team --exclude lego-island`, with the discs;
    - the four released titles' own tests without their discs (`LEGO_LEGACY_GAME_DATA` pointed at an empty directory, `LEGO_LEGACY_REQUIRE_DISC=0`), so only their disc replays are skipped;
    - `cargo test -p lego-legacy-core --lib`.
  - The full `ci.sh`, with `LEGO_LEGACY_REQUIRE_DISC=1 LEGO_LEGACY_REQUIRE_CORPUS=1`, runs for a version release. It also runs for a change meant to move a released title's runs, whose moves it measures for the blessing.
- **Deploy (step 11):** `lego-ship`, alone, after the merge, at any time. A ship never touches a game already running, since the MacBook pulls only when a title starts.
- **Monitor (step 12):** a headless run, from the MacBook's kit, of a committed route of each title the change touches. It always takes a fresh `--data-root`, never the kit's live saves, since a route can make the game save. It must end on the same line as the last full `ci.sh` run of that route, or on the line a blessed change moved it to.
- **Freeze:** none.
- **Shared code that needs extra care:** `core/`, which every title runs on, above all `core/src/host/` (the host behaves the same for every title) and anything that can move a released title's runs. A move in Rock Raiders', Hot Wheels', Alpha Team's or LEGO Island's runs needs the maintainer's blessing before its merge. A move that only a version release's full `ci.sh` finds needs it before the tag.
- **Outages confirmed as expected, and who confirmed them:** none yet.

## Living doc

Our living doc: https://claude.ai/artifact/8VZiKCrrknBDVunxKQk7iB. It mirrors the GitHub issues, which stay the record. Its ticket files live in `.scratch/lego-legacy/` and render to the local mirror `.scratch/lego-legacy/living-doc.html`. Both stay in the maintainer's checkout, excluded from git because they name local paths.

After every ticket change, and for every new ticket (filed on GitHub first, then mirrored, or from the page's form), use the `living-doc` skill. It syncs the local mirror and the artifact, and respins the artifact when this link is unreachable after a Claude seat switch.
