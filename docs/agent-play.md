# Agent Play & Systemwide Control Tools

`lego-ctrl` (CLI) and `lego-mcp` (stdio JSON-RPC 2.0 MCP server) are zero-crate workspace binaries wrapping `lego-legacy`'s HTTP/UDS control and introspection plane (#309, #312).

## Building & Installing

`lego-ship` discovers every binary in the workspace and publishes release builds into `play-kit/` (pulled automatically on the MacBook via `lego`):

```sh
lego-ship
ln -sf /Volumes/Tabletop/Games/lego-legacy/play-kit/lego-ctrl ~/.local/bin/lego-ctrl
ln -sf /Volumes/Tabletop/Games/lego-legacy/play-kit/lego-mcp  ~/.local/bin/lego-mcp
```

## `lego-ctrl` CLI

Target resolution order: `--url <URL>` → `LEGO_CTRL_URL` → latest live session in `~/.local/state/lego-legacy/sessions/*.json` (optionally filtered by `--title <name>`, with stale PIDs pruned automatically). Pass `--token <TOK>` or set `LEGO_LEGACY_TOKEN` when the server requires Bearer auth on mutating POSTs.

```sh
# Launch a title headless with HTTP control (or pass --window)
lego-ctrl launch lego-chess --mode step
lego-ctrl sessions
lego-ctrl watch                              # prints http://.../watch

# Observe
lego-ctrl status
lego-ctrl meta && lego-ctrl controls && lego-ctrl goals
lego-ctrl fps --window 0..600
lego-ctrl screenshot --grid --cursor -o frame.png
lego-ctrl screenshot --crop 160,120,320,240 -o zoom.png

# Drive inputs & pipe routes / JSONL
lego-ctrl click 320 240                      # 1-flip settle between move and down (--settle-flips 0 to opt out)
lego-ctrl click 320 240 --poll --hold-flips 2
lego-ctrl move 120 80
lego-ctrl delta -12 8
lego-ctrl drag 100 100 300 240
lego-ctrl key Space --hold-flips 3
lego-ctrl chord LeftControl,A --hold-flips 2
lego-ctrl type "PLAYER1"
lego-ctrl wait 30
lego-ctrl wait-change 60
lego-ctrl wait-settled 120
lego-ctrl act '{"action":"click","x":320,"y":240,"hold_flips":1}'
lego-ctrl pipe titles/lego-island/routes/boot.route
cat tasks.jsonl | lego-ctrl pipe -

# Read-only introspection (zero guest mutation)
lego-ctrl mem 0x00400000 64 --fmt hex
lego-ctrl mem-map
lego-ctrl mem-search --ascii "LEGO"
lego-ctrl cpu
lego-ctrl win32
lego-ctrl files profile
lego-ctrl read-file profile SAVE0.DAT -o save0.bin

# Checkpoints, routes, agent scratchpad & mode
lego-ctrl bookmark before-boss
lego-ctrl rewind before-boss
lego-ctrl snapshot /Volumes/Tabletop/.tmp/run.llsnap
lego-ctrl restore  /Volumes/Tabletop/.tmp/run.llsnap
lego-ctrl export-route -o recorded.route
lego-ctrl goal "Complete Mission 1"
lego-ctrl thought "Navigating menu cursor to Start"
lego-ctrl hint "Try the left button"
lego-ctrl mode live 1.0
lego-ctrl mode pause
lego-ctrl mode shared
lego-ctrl mode close
```

## `lego-mcp` Stdio MCP Server

`lego-mcp` speaks newline-delimited JSON-RPC 2.0 over `stdin`/`stdout` (logs strictly to `stderr`) and exposes 9 tools:

| Tool | Purpose |
| --- | --- |
| `game_sessions` | Discover live running sessions |
| `game_launch` | Launch a title with `--control` and wait for readiness |
| `game_meta` | Fetch `/api/meta`, `/api/controls`, `/api/goals` |
| `game_observe` | Fetch `/api/status` + `/api/frame.json` + native PNG `ImageContent` (`grid`/`cursor`/`crop`) |
| `game_act` | Execute actions, batches, `step`/`wait`, or `pipe` routes + optional PNG `ImageContent` |
| `game_inspect` | Read-only `mem`, `mem_map`, `mem_search`, `cpu`, `win32`, `files`, `file_read`, `fps` |
| `game_goal` | Update `goal`, `subgoals`, `thought`, `commentary`, `notepad`, `user_hint` on `/watch` |
| `game_checkpoint` | `bookmark`, `list_bookmarks`, `rewind`, `snapshot`, `restore`, `export_route` |
| `game_mode` | `step`/`live`, `pause`/`resume`, `pace`, `owner` (`agent`/`human`/`shared`), `close` |

### Antigravity Setup (`~/.gemini/antigravity/mcp_config.json`)

```json
{
  "mcpServers": {
    "lego-legacy": {
      "command": "/Users/constdecimalserrno/.local/bin/lego-mcp",
      "args": []
    }
  }
}
```

### Claude Code Setup (`~/.claude.json` or CLI)

```sh
claude mcp add --scope user lego-legacy -- ~/.local/bin/lego-mcp
```

Or add to `~/.claude.json` under `"mcpServers"`:

```json
{
  "mcpServers": {
    "lego-legacy": {
      "command": "/Users/constdecimalserrno/.local/bin/lego-mcp",
      "args": []
    }
  }
}
```

## Mac mini ↔ `mbp` Tailscale Watching & Control

Bind the control server on all interfaces with a Bearer token so `mbp` can watch `/watch` in a browser or drive the run remotely over Tailscale:

```sh
# On Mac mini (e.g. Tailscale IP 100.x.y.z):
export LEGO_LEGACY_TOKEN="$(openssl rand -hex 16)"
lego-ctrl launch lego-chess --bind 0.0.0.0:7777 --mode live --token "$LEGO_LEGACY_TOKEN"

# On mbp:
# 1. Watch live in browser:
open "http://<mac-mini-tailscale-ip>:7777/watch"

# 2. Control remotely with lego-ctrl or lego-mcp:
export LEGO_CTRL_URL="http://<mac-mini-tailscale-ip>:7777"
export LEGO_LEGACY_TOKEN="<same-token>"
lego-ctrl status
lego-ctrl screenshot --grid --cursor -o remote.png
```
