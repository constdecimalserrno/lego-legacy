# Hot Wheels: Stunt Track Driver (`hotwheels-stunt-track-driver`)

Agent playbook for driving `STUNT.EXE` via `lego-ctrl` or `lego-mcp`. For generic CLI/MCP commands and session setup, see [`docs/agent-play.md`](../../docs/agent-play.md).

## Runtime Facts

- **Display:** `640×480`, 8bpp, 60 Hz (`FlipCost::Refresh(60)`), declared Pentium 166 MHz.
- **Mouse regime:** `SoftwareRelative { virtual_span: (640, 480), start_pos: (320, 240) }` (`1.0×` scale, `recenter_every_flip: false`): `STUNT.EXE` hides the OS cursor and steers its red-arrow software cursor via buffered DirectInput relative deltas; `ControlHost` keeps visual cursor tracking aligned across post-race `SetCursorPos(320, 240)` transitions (`#329`).
- **Cadence:** `MicroStep { flips_per_step: 8 }` during races for tight steering and stunt timing; use `wait-change` / `wait-settled` in menus.
- **Exit guard:** `escape_quits: true` — `Escape` immediately terminates `STUNT.EXE` (`exits 1`) and is blocked by the control plane unless `force: true`.

## Object of the Game & Progression Goals

Race Hot Wheels cars across 6 household tabletop tracks, build and race custom stunt tracks in the Garage, land mid-air stunts without crashing, and unlock the hidden 7th wireframe track (`WIRE`).

Work through goals in order; each ends on a verifiable state or save write:

1. **Single Race (`first-race`, Playable):** Enter Single Race (Track 1, Living Room / `HOUS`), complete the race cleanly, and persist `saves\times.dat` (480 B).
2. **Custom Garage Track Gold Medal (`custom-garage-gold`, Playable):** Open the Custom Garage Stunt Track Builder, place stunt track pieces onto the workbench slots, click the Green GO arrow, and earn a Gold Medal on the custom track.
3. **Aerial Stunts (`aerial-stunts`, Playable):** Launch off ramps, execute mid-air barrel rolls (`LeftShift`) and flips (`LeftControl`), and release in time to land upright with zero crashes.
4. **Championship Tracks 1–6 (`championship-tracks-1-6`, Complete):** Complete all 6 Championship tracks in sequence — 1 Living Room (`HOUS`), 2 Bedroom (`TOYS`), 3 Greenhouse (`ATRI`), 4 Attic (`ATIC`), 5 Sandbox (`SAND`), 6 Basement (`BASE`). Done when `saves\onoff.dat` (28 B) holds six `01 00 00 00` unlock dwords plus a trailing zero dword (written after Track 3 and Track 6).
5. **Secret Track `WIRE` Gold Medal (`secret-track-wire-gold`, Complete):** With Tracks 1–6 unlocked, open Track Select, right-click the invisible Mattel `"TT"` hotspot at `(270, 100)`, click Green GO to launch Secret Track 7 (`WIRE.JAM`, neon-green wireframe grid), and finish with a Gold Medal (`<= 1:10`).

## Verified Controls (`STUNT.EXE` DirectInput)

### Driving & Rocket Start

| Input | Action | Verified behavior & timing |
| --- | --- | --- |
| `Up` (`Keypad8`/`7`/`9`) | Throttle | Hold **only** when the start light turns green (~flip 1000 on Track 1) for a **Rocket Start**. Revving on red/yellow loses launch speed. |
| `Down` (`Keypad2`/`1`/`3`) | Brake / reverse | Slow down before tight turns or recover off-line. |
| `Left` / `Right` (`Keypad4`/`6`) | Steer | Steer left / right along the track; pulse in short `MicroStep` bursts (1–8 flips) to avoid oversteer. |

### Aerial Stunts

| Input | Action | Verified behavior & timing |
| --- | --- | --- |
| `LeftShift` (`LeftAlt`, `PageDown`) | Barrel roll | Hold mid-air off ramps (often with `Left`/`Right`) to spin a barrel roll; release early enough to land wheels down. |
| `LeftControl` (`Delete`) | Flip | Hold mid-air off ramps to perform a flip; release early enough to land upright. |

Landing inverted or sideways triggers a crash and costs race time.

### Menus, Race Utility & Exit

| Input | Action | Verified behavior & timing |
| --- | --- | --- |
| `Space` | Skip / confirm | Skips Mattel Media / Semi Logic splashes and intro (~flip 285, hold ~5–9 flips); also acts as left-click confirm in menus (`LeftControl` confirms, `LeftAlt` right-clicks). |
| `Left click` | Menu / workbench select | First `move` to target and settle, then click with `--hold-flips 5` (or `5..6`). |
| `Right click (270, 100)` | Arm Secret Track `WIRE` | On Track Select after clearing Tracks 1–6, right-click the Mattel `"TT"` logo hotspot, then click Green GO. |
| `P` or `Pause` | Pause race | Toggles in-race pause safely without quitting. |
| `C` | Reset / crash car | Forces an in-race crash/respawn and increments the crash counter; use only if hopelessly stuck. |
| `F1` | Cycle race resolution | In menus, cycles `640×480` / `320×240` / `320×200`; keep at `640×480` during normal play. |
| `Escape` | **Immediate quit (HAZARD)** | Unconditionally terminates `STUNT.EXE` (`exits 1`) in 3 flips. Use `P`/`Pause` to pause; pass `force: true` only to end the run. |

## Menu & Screen Flow

1. **Boot → Main Menu:** First light at flip 101; send `Space` around flip 285 (`--hold-flips 9`) to enter the Main Menu.
2. **Main Menu:**
   - **Single Race / Track Select:** Crossed flags, bottom-right `~(546, 413)`.
   - **Championship:** Top-right `~(552, 102)`.
   - **Custom Garage Stunt Track Builder:** Workbench icon; place track pieces onto the workbench and click the Green GO arrow to race.
3. **Track Select:** Pick an unlocked track (or right-click Mattel `"TT"` at `(270, 100)` for `WIRE` after Tracks 1–6), then click the Green GO arrow at bottom-right `~(580, 420)`–`(583, 430)`.
4. **Car Select Wheel:** Click a car spoke on the wheel (e.g. `~(498, 373)`), then click the Green GO arrow `~(583, 430)` to drop onto the starting grid.
5. **Post-Race Results:** Click (`--hold-flips 5`) to advance past results and trophy screens into the next race or back to the menu.

## Input Hazards & Workarounds

- **`Escape` immediately quits:** Always pause with `P`/`Pause` and navigate with menu buttons. `ControlHost` blocks `Escape` unless `force: true`.
- **1-flip clicks drop in menus:** Always `move` onto the button first (let the cursor settle 1–2 flips) and click with `--hold-flips 5` (or `5..6`). Every reliable menu click in `routes/first-race.route` and `routes/secret-track.route` uses 5–6 flip holds.
- **Post-race `SetCursorPos(320, 240)` transitions (`#329`):** On race entry/exit, `STUNT.EXE` calls `SetCursorPos(320, 240)` while its internal software cursor continues integrating buffered DirectInput relative deltas. Because `GUIDE.mouse_mode` is `SoftwareRelative`, `ControlHost`, `--cursor`, and `/watch` Take Over preserve visual cursor alignment automatically across races (standard `move`/`click` coordinates stay aligned pre- and post-race).
- **Early throttle breaks Rocket Start:** Wait until the starting light switches green before holding `Up`.
- **Over-rotating aerial stunts crashes:** Start `LeftShift`/`LeftControl` immediately off the lip of a ramp and release before descent so the wheels level out for touchdown.

## Save Files & Reference Routes

- **Saves (under `C:\Program Files\Mattel\Stunt Track Driver\saves\`):**
  - `saves\onoff.dat` (28 B): track unlock flags (`01 00 00 00` × 6 + `00 00 00 00` once Tracks 1–6 are cleared).
  - `saves\times.dat` (480 B): per-track best times and medal records, written on every race finish.
  - `saves\wheel.dat` (240 B): Car Select wheel state.
  - `saves\track0.dat`–`saves\track3.dat`, `saves\tracks.dat`: saved Custom Garage tracks.
- **Committed reference routes:**
  - [`routes/first-race.route`](routes/first-race.route): boot → Track 1 finish (`saves\times.dat`) → `Escape` quit.
  - [`routes/secret-track.route`](routes/secret-track.route): clean profile → Championship Tracks 1–6 → Mattel `"TT"` right-click unlock → `WIRE` finish → `Escape` quit.
  - [`routes/wire-gold-medal.route`](routes/wire-gold-medal.route): clean profile → Championship Tracks 1–6 → Mattel `"TT"` right-click unlock → `WIRE` 1:08 Gold Medal (`0` crashes, `54` stunts) → `Escape` quit.
