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
