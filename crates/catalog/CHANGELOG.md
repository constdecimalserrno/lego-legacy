# Changelog: lego-legacy-catalog

One heading per version, newest first. What each part of a crate's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-28

Initial consumer-only title catalog crate (`crates/catalog`, #407), keeping `core` 100% free of hardcoded title declarations (Golden Rule 4) and strictly acyclic (`launcher -> catalog -> titles/* -> core`):

- Collects all 7 title crates (`lego-rock-raiders`, `hotwheels-stunt-track-driver`, `lego-alpha-team`, `lego-island`, `lego-chess`, `lego-racers`, `legoland`) via workspace `path =` dependencies (no `=version` pins on titles) and pins `lego-legacy-core = "=2.0.0"`.
- Exports `TITLES: &[&'static Title]` in roster order and re-exports `GM16` (`[EntryImage; 4]`) for cross-title consumers and integration tests.
- Hosts cross-title catalog, roster, cabinet, InstallShield 3, DLS synth, and real-disc shim integration tests migrated out of `core/tests/`.
