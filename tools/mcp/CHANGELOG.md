# Changelog: lego-mcp

One heading per version, newest first. What each part of a tool's version means is in
[ADR-0007](../../docs/adr/0007-every-crate-carries-its-own-version.md).

## 0.1.0 — 2026-11-25

Initial zero-crate `lego-mcp` stdio JSON-RPC 2.0 MCP server (#312):

- Pure `std` newline-delimited JSON-RPC 2.0 server on `stdin`/`stdout` (logs strictly to `stderr`),
  implementing `initialize`, `notifications/initialized`, `ping`, `tools/list`, and `tools/call`.
- Exposes 9 game control & introspection tools (`game_sessions`, `game_launch`, `game_meta`,
  `game_observe`, `game_act` including `click` `settle_flips` #344 and `chord` `lead_flips` #348, `game_inspect`, `game_goal`, `game_checkpoint`, `game_mode`),
  `game_observe`, `game_act` (including `click` `settle_flips` #344, `clear_queue`, and absolute-by-default `pipe` with `flip +N`
  relative syntax #346), `game_inspect`, `game_goal`, `game_checkpoint`, `game_mode`),
  including native MCP `ImageContent` (`image/png`, base64) screenshots with grid/cursor/crop overlays.
- Exposes raw `mouse_down` / `mouse_up` in the `game_act` tool schema (#352).
