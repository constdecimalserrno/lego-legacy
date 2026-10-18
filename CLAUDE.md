# lego-legacy

## Agent skills

### Issue tracker

Issues live as GitHub issues on `constdecimalserrno/lego-legacy`, managed with the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage labels, unchanged: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.

## Versioning

Every crate and title is versioned and tagged on its own, meticulously, so each keeps its own log and takes patches individually (ADR-0007).

- **Changing a crate**: move its version by ADR-0007's meaning of major, minor and patch (once per release: a crate already past its last tag stays there), and write the change under that version's heading in the crate's `CHANGELOG.md`. A core bump changes each title's exact pin, so it moves every title too.
- **Releasing**: tag each crate whose version moved as `<crate>-v<version>`, and the workspace as `v<version>`. A patch to one title releases on its own.

`core/tests/versions.rs` fails the suite on a missed bump or a missing heading.
