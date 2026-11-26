# Changelog: lego-ctrl

One heading per version, newest first. What each part of a tool's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-25

Initial zero-crate `lego-ctrl` CLI and shared HTTP/1.1 + UDS client library (#312):

- Pure `std` HTTP/1.1 client over TCP and Unix domain sockets with `Content-Length`,
  chunked transfer decoding, binary responses, and optional Bearer token auth.
- Live session discovery in `~/.local/state/lego-legacy/sessions/` with stale PID pruning,
  `--url` / `LEGO_CTRL_URL` / `--title` resolution, and `launch` helper waiting for control readiness.
- Full CLI surface for status, metadata, controls, goals, fps, watch URL, PNG screenshots,
  action primitives (including `click --settle-flips N`, #344, and `chord --lead-flips N`, #348), route/JSONL piping, read-only memory/CPU/Win32/file inspection,
  bookmarks/rewind/snapshot/restore/route export, agent state, and execution mode control.
  action primitives (including `click --settle-flips N`, #344), route/JSONL piping (`pipe --relative|--absolute|--no-wait`, `pipe --help`,
  explicit `flip +N` relative offsets with absolute bare `flip N` default), queue clearing
  (`clear` -> `POST /api/clear-queue`), read-only memory/CPU/Win32/file inspection,
  bookmarks/rewind/snapshot/restore/route export, agent state, and execution mode control (#346).
- Top-level `resume`, `pace <mult>` (dispatching to `POST /api/mode`), and raw `mouse-down` / `mouse-up` (`[<x> <y>] [--right] [--button left|right]`) subcommands (#352).
