# LEGO Racers (`lego-racers`)

Agent playbook for driving `LEGORacers.exe` (Masterpiece pressing) via `lego-ctrl` or `lego-mcp`. For generic CLI/MCP commands and session setup, see [`docs/agent-play.md`](../../docs/agent-play.md).

> **Status (`#369`, pre-first-light):** Controls, power-ups, circuit structure, and save paths below are verified from primary sources (`MANUAL.PDF`, `README.TXT`, `LEGO.JAM`, and `LEGORacers.exe` disassembly). Exact menu coordinates, initial cursor `start_pos`, and intro skip timing will be empirically calibrated once first-light bringup lands (`#370+`).

## Runtime Facts

- **Display:** `640×480`, 16bpp, 60 Hz (`FlipCost::Refresh(60)`, baseline until measured in `#370+`), `GolDP.dll` (DirectDraw 4 + Direct3D 3 Immediate Mode).
- **Mouse regime:** `SoftwareRelative { virtual_span: (640, 480), start_pos: (320, 240) }` (`1.0×` scale, `recenter_every_flip: false`): in fullscreen `LEGORacers.exe` hides the OS cursor (`ShowCursor(FALSE)`), never calls `SetCursorPos`, and drives its `32×32` `MENUDATA/ARROW.BMP` software cursor from buffered DirectInput `GetDeviceData` relative deltas.
- **Cadence:** `MicroStep { flips_per_step: 8 }` for racing steering and Turbo Start timing; use `wait-change` / `wait-settled` in menus.
- **Exit guard:** `escape_quits: false` — `Escape` opens the in-race `GAME PAUSED` menu or backs out of menus.

## Object of the Game & Progression Goals

Build custom LEGO drivers and karts, race through 6 Circuit Championships (4 races × 3 laps each) to defeat each Circuit Champion and unlock their brick sets, then beat Rocket Racer on Circuit 7 (`Rocket Racer Run`).

1. **Boot to Main Menu (`boot-main-menu`, FirstLight):** Clear intro movies (`lmicmp.avi`, `HVSCmp.avi`, `introcmp.avi`) and reach the Main Menu (`CIRCUIT RACE`, `SINGLE RACE`, `VERSUS RACE`, `TIME RACE`, `BUILD`, `OPTIONS`, `QUIT`).
2. **Build Custom Driver & Car (`build-custom-racer`, Playable):** Enter `BUILD` → `NEW RACER`, assemble a driver, sign `Make License` (`type_text`), build or Quick Build a car, save to `Save\LEGORac1`, and return to the Main Menu.
3. **Win Single Race (`single-race-win`, Playable):** Enter `SINGLE RACE` on Imperial Grand Prix, nail Turbo Start (`Up` on `GO`), use an upgraded power-up (`Return`), finish all 3 laps in 1st place, and persist best times to `Save\LEGORac1`.
4. **Win Circuit 1 (`circuit-1-redbeard`, Playable):** Win Captain Redbeard's 4-race circuit (Imperial Grand Prix, Dark Forest Dash, Magma Moon Marathon, Desert Adventure Dragway) on points (1st=30, 2nd=20, 3rd=10, 4th=3, 5th=2, 6th=1) to unlock Circuit 2 and Redbeard's Car Set.
5. **Defeat Rocket Racer (`rocket-racer-champion`, Credits):** Clear Circuits 1–6 (Redbeard, Kahuka, Basil, Johnny Thunder, Baron von Barron, Gypsy Moth) and defeat Rocket Racer on Circuit 7 (`Rocket Racer Run`) to reach Credits.

## Verified Controls (`MANUAL.PDF` & `LEGORacers.exe`)

### Driving, Drifting & Power-Ups (Default Player 1)

| Input | Action | Verified behavior & timing |
| --- | --- | --- |
| `Up` | Accelerate / **Turbo Start** | Hold to drive forward. Wait during `3-2-1` and press `Up` right as `GO` appears for a Turbo Start. |
| `Down` | Brake / Reverse | Slow down for hairpins or reverse out of walls. |
| `Left` / `Right` | Steer | Steer left/right; pulse in short `MicroStep` bursts on straights, hold into corners. |
| `Return` | **Activate Power-Up** | Fires/triggers held Red/Yellow/Blue/Green brick (not `Space`). Also confirms menu selections. |
| `Space` | **Power Slide** (Drift) | Hold `Up` + `Space` + `Left`/`Right` (`chord`) to drift through sharp corners. |
| `Up`+`Down`+`Space`+`Left`/`Right` | **Super Slide** | Pivot tightly in place; release `Down` or `Space` to launch forward. |
| `RightControl` | Change Camera View | Cycles chase / close / wide racing camera views. |
| `RightAlt` | Look Backwards | Hold to view behind the kart. |
| `M` | Toggle HUD | Cycles Speedometer → Course Map → Close-up Map → Off. |
| `Escape` | Pause Race / Menu Back | Opens `GAME PAUSED` (`CONTINUE`, `RESTART RACE`, `RESTART CIRCUIT`, `EXIT`) or backs out of menus. |

### Power-Up Bricks (`MANUAL.PDF` p. 9)

Pick up a colored brick, then up to 3 White Power Plus bricks to upgrade before pressing `Return`:

| Color | 0 White | +1 White | +2 White | +3 White |
| --- | --- | --- | --- | --- |
| **Red** (Projectile) | Cannon Ball | Grappling Hook | Lightning Wand | 3 Guided Rockets |
| **Yellow** (Hazard) | Oil Slick | Barrel of Gun Powder | Magnetic Trap | Mummy's Curse |
| **Blue** (Shield) | 5 s Shield | 6 s Shield | 8 s Shield (reflects + spins) | 10 s Shield (reflects + spins) |
| **Green** (Turbo) | Turbo Boost | Extra Turbo Boost | Flying Turbo Boost | Warp Turbo Boost |

### Car Builder (`BUILD` → `NEW RACER` / `EDIT RACER`)

| Input | Action |
| --- | --- |
| `Insert` / `PageUp` | Previous / Next Car Set |
| `Delete` / `PageDown` | Previous / Next LEGO Brick |
| `S` / `X` / `A` / `D` (or Arrow keys) | Orbit / tilt car camera |
| `Keypad1`–`4`, `Keypad6`–`9` | Move floating brick above chassis (8 directions) |
| `Keypad5` / Right-click brick | Rotate selected brick 90° |
| `KeypadPlus` (`+`) / Double left-click | Drop / connect brick onto car |
| `KeypadMinus` (`-`) | Undo last placed brick |
| Left-drag on brick / car | Position floating brick / orbit 3D car camera |

## Input Hazards & Workarounds

- **`Return` vs. `Space`:** `Return` activates power-ups; `Space` drifts (`Power Slide`).
- **Early throttle loses Turbo Start:** Wait for `GO` before pressing `Up`.
- **`VERSUS RACE` requires a controller:** 2-player split-screen is unavailable without an attached joystick (`MANUAL.PDF` p. 15).
- **Cursor calibration pending first light:** Verify `start_pos` and Main Menu button coordinates on first boot before committing route coordinates.

## Save Files & Reference Routes

- **Save file:** `C:\Program Files\LEGO Media\Games\LEGO Racers\Save\LEGORac1` (`15,872` B, persisted under `<data-root>/profile/C/Program Files/LEGO Media/Games/LEGO Racers/Save/LEGORac1`). Stores custom drivers/licenses/cars, circuit unlocks, trophies, and best times.
- **Committed reference routes:** None yet (first-light and race routes follow in `#370+`).
