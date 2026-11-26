# LEGO Racers (`lego-racers`)

Agent playbook for driving `LEGORacers.exe` (Masterpiece pressing) via `lego-ctrl` or `lego-mcp`. For generic CLI/MCP commands and session setup, see [`docs/agent-play.md`](../../docs/agent-play.md). Every control, coordinate, timing window, and save path below is empirically verified on the real retail disc.

## Runtime Facts

- **Display:** `640×480`, 16bpp, 60 Hz (`FlipCost::Refresh(60)` — verified at exact 60.0 Hz lap-timer cadence across Imperial Grand Prix with zero clock-spin stalls), `GolDP.dll` (DirectDraw 4 + Direct3D 3 Immediate Mode), declared Pentium II 450 MHz / Windows 98 SE.
- **Mouse regime:** `SoftwareRelative { virtual_span: (640, 480), start_pos: (0, 0) }` (`1.0×` scale, `recenter_every_flip: false`): `LEGORacers.exe` hides the OS cursor via `DISCL_EXCLUSIVE` DirectInput mouse acquisition (`GUID_SysMouse`, `SetCooperativeLevel(hwnd, 0x9)` + `Acquire()`), never calls `SetCursorPos`, and drives its `32×32` yellow `MENUDATA/ARROW.BMP` software cursor from buffered DirectInput `GetDeviceData` relative deltas starting at `(0, 0)`.
- **Intros & skipping:** `lmicmp.avi` (flips `0..236`, 237 frames) is unskippable; `hvscmp.avi` (flip `237+`) and `introcmp.avi` (flip `242+`) skip on `Space`, `Escape`, `Return`, or a left mouse click (`click_in_a_movie: Some(0x39)`). The LEGO Media splash fades into the animated 3D Captain Redbeard Main Menu by flip `547`.
- **Cadence:** `MicroStep { flips_per_step: 8 }` for racing steering and Turbo Start timing; use `wait-change` / `wait-settled` in menus.
- **Exit guard:** `escape_quits: false` — `Escape` opens the in-race `GAME PAUSED` menu or backs out of Circuit/Track Select (never hard-exits).

## Object of the Game & Progression Goals

Build custom LEGO drivers and karts, race through 6 Circuit Championships (4 races × 3 laps each) to defeat each Circuit Champion and unlock their brick sets, then beat Rocket Racer on Circuit 7 (`Rocket Racer Run`).

1. **Boot to Main Menu (`boot-main-menu`, FirstLight):** Clear intro movies (`lmicmp.avi`, `hvscmp.avi`, `introcmp.avi`) and reach the Main Menu (`BUILD`, `CIRCUIT RACE`, `SINGLE RACE`, `VERSUS RACE`, `TIME RACE`, `OPTIONS`, `QUIT`).
2. **Build Custom Driver & Car (`build-custom-racer`, Playable):** Enter `BUILD` → `NEW RACER`, assemble a driver (`MIX` or part rows), sign `Make License` (`type_text`), assemble a car (`QUICK BUILD` or custom brick placement in 3D Brick Builder), exit via the Exit Door `(68, 435)` → `FINISH`, persist `Save\0\LEGORac1`, and return to Main Menu.
3. **Win Single Race (`single-race-win`, Playable):** Enter `SINGLE RACE` on Imperial Grand Prix, nail Turbo Start (`Up` on `GO`), collect and trigger an upgraded power-up (`Return`), finish all 3 laps in 1st place, and persist best times to `Save\0\LEGORac1`.
4. **Win Circuit 1 (`circuit-1-redbeard`, Playable):** Win Captain Redbeard's 4-race circuit (Imperial Grand Prix, Dark Forest Dash, Magma Moon Marathon, Desert Adventure Dragway) on points (1st=30, 2nd=20, 3rd=10, 4th=3, 5th=2, 6th=1; survival cutoffs ≥10 after Race 1, ≥20 after Race 2, ≥30 after Race 3) to unlock Circuit 2 and Redbeard's Car Set.
5. **Defeat Rocket Racer (`rocket-racer-champion`, Credits):** Clear Circuits 1–6 (Redbeard, Kahuka, Basil, Johnny Thunder, Baron von Barron, Gypsy Moth) and defeat Rocket Racer on Circuit 7 (`Rocket Racer Run`) to win Rocket Racer's Car Set and reach Credits.

## Verified Controls (`MANUAL.PDF` & `LEGORacers.exe`)

### Driving, Drifting & Power-Ups (Default Player 1)

| Input | Action | Verified behavior & timing |
| --- | --- | --- |
| `Up` | Accelerate / **Turbo Start** | Hold to drive forward. Wait during `3-2-1` (60-flip / 1.00 s intervals) and press `Up` right as `GO` appears for a Turbo Start. |
| `Down` | Brake / Reverse | Slow down for hairpins or reverse out of walls. |
| `Left` / `Right` | Steer | Steer left/right; pulse in short `MicroStep` bursts (4–8 flips) on straights, hold into corners. |
| `Return` | **Activate Power-Up** | Fires/triggers held Red/Yellow/Blue/Green brick (**not `Space`**). Also confirms menu selections. |
| `Space` | **Power Slide** (Drift) | Hold `Up` + `Space` + `Left`/`Right` (`chord`) to carve high-speed corners. |
| `Up`+`Down`+`Space`+`Left`/`Right` | **Super Slide** | Pivot tightly in place; release `Down` or `Space` to launch forward. |
| `RightControl` | Change Camera View | Cycles chase / close / wide racing camera views. |
| `RightAlt` | Look Backwards | Hold to view behind the kart. |
| `M` | Toggle HUD | Cycles Speedometer → Course Map → Close-up Map → Off. |
| `Escape` | Pause Race / Back | Opens `GAME PAUSED` (`CONTINUE`, `RESTART RACE`, `RESTART CIRCUIT`, `EXIT`) in-race or backs out of Circuit/Track Select. |

### Power-Up Bricks (`MANUAL.PDF` p. 9)

Pick up a spinning colored brick, then up to **3 White Power Plus bricks** to upgrade before pressing `Return`:

| Color | 0 White (Lv0) | +1 White (Lv1) | +2 White (Lv2) | +3 White (Lv3) |
| --- | --- | --- | --- | --- |
| **Red** (Projectile) | Cannon Ball | Grappling Hook | Lightning Wand | 3 Guided Rockets |
| **Yellow** (Hazard) | Oil Slick | Barrel of Gun Powder | Magnetic Trap | Mummy's Curse |
| **Blue** (Shield) | 5 s Shield | 6 s Shield | 8 s Shield (reflects + spins) | 10 s Shield (reflects + spins) |
| **Green** (Turbo) | Turbo Boost | Extra Turbo Boost | Flying Turbo Boost | Warp Turbo Boost (teleports forward) |

> **Track Shortcuts:** Firing a Red projectile at marked track targets (e.g., Imperial Grand Prix cannon/waterfall wall, Dark Forest Dash, Royal Knights Raceway) blasts open shortcuts.

### Car Builder (`BUILD` → `NEW RACER` / `EDIT RACER`)

| Input | Action |
| --- | --- |
| `Insert` / `PageUp` | Previous / Next Car Set (top slider) |
| `Delete` / `PageDown` | Previous / Next LEGO Brick (bottom slider) |
| `S` / `X` / `A` / `D` (or Arrow keys) | Orbit / tilt 3D car camera |
| `Keypad1`–`4`, `Keypad6`–`9` | Move floating brick above chassis (8 directions) |
| `Keypad5` / Right-click brick | Rotate selected brick 90° |
| `KeypadPlus` (`+`) / Double left-click | Drop / connect brick onto car (green = snaps, red = invalid) |
| `KeypadMinus` (`-`) | Undo last placed brick |
| Left-drag (`mouse_down` + `move` + `mouse_up`) | Grab floating brick or orbit 3D car camera |
| Click Exit Door `(68, 435)` | Leave 3D Brick Builder to the Finish / Save screen |

## Verified `640×480` Menu Hotspots & Navigation

Menus accept both mouse (`ARROW.BMP` + left-click) and keyboard (`Up`/`Down`/`Left`/`Right` + `Return`). Use coordinate `click` (default `settle_flips: 1`) so the 2-flip hover-arrival lands and clicks cleanly on the first click:

| Screen | Element | Verified Center (`x, y`) | Notes |
| --- | --- | --- | --- |
| Main Menu | `BUILD` | `(65, 135)` | Opens Build Menu (default keyboard highlight) |
| Main Menu | `CIRCUIT RACE` | `(107, 195)` | Opens Circuit Select (`CIRCUIT NUMBER: 1`, Redbeard) |
| Main Menu | `SINGLE RACE` | `(101, 235)` | Opens Track Select (Imperial Grand Prix first) |
| Main Menu | `VERSUS RACE` | `(101, 275)` | Grayed out without a joystick (`MANUAL.PDF` p. 15) |
| Main Menu | `TIME RACE` | `(91, 315)` | Race Veronica Voltage's ghost car |
| Main Menu | `OPTIONS` | `(79, 375)` | Game / Video / Audio / Controls / Language / Credits |
| Main Menu | `QUIT` | `(58, 435)` | Prompts `ARE YOU SURE YOU WANT TO QUIT?` |
| Circuit Select | `OK` | `(48, 395)` | Confirms selected circuit (`Return` also works) |
| Circuit Select | `MAIN MENU` | `(78, 435)` | Returns to Main Menu (`Escape` also works) |
| Build Menu | `NEW RACER` | `(90, 152)` | Starts Driver Builder → Make License → Build Car |
| Build Menu | `MAIN MENU` | `(78, 435)` | Returns to Main Menu (`Escape` does **not** back out here) |
| 3D Brick Builder | Exit Door | `(68, 435)` | Bottom-left exit door to Finish / Save screen |

## Input Hazards & Workarounds

- **`Return` vs. `Space`:** `Return` activates power-ups; `Space` drifts (`Power Slide`).
- **Early throttle loses Turbo Start:** Wait during `3-2-1` and hold `Up` right when `GO` appears.
- **`VERSUS RACE` requires a controller:** 2-player split-screen stays grayed out without an attached joystick (`MANUAL.PDF` p. 15).
- **1-frame `Menu::OnMouseMove` lag (`0x48b8de` before `0x4b1e70`):** `LEGORacers.exe` hit-tests menu hover against the prior frame's cursor coordinates before adding `(dx, dy)`. Use coordinate `click` (default `settle_flips: 1` automatically emits a 1-px hover-arrival step before clicking) rather than a single-flip `move` followed by an in-place click.
- **Cursor origin, clamping, and post-race reset:** `ARROW.BMP` starts at `(0, 0)`, clamps inside `[0..632, 0..472]` (`640 - 8`, `480 - 8`), and resets to `(0, 0)` when exiting a race back to Main Menu (`Unacquire()` → `MenuManager::InitCursor` → `Acquire()`). `ControlHost` tracks this automatically when `pointer_shown` re-acquires (`true -> false`), keeping `status.pointer_pos` and `/watch` locked within `<= 2 px` across menus and post-race returns.
- **`Escape` in `BUILD MENU`:** Unlike Circuit Select and in-race Pause, `Escape` does not exit `BUILD MENU` — click `MAIN MENU` `(78, 435)` or select it with arrows + `Return`.
- **Save flush timing:** `Save\0\LEGORac1` is written via `CloseHandle` on clean shutdown (close or `QUIT` → `YES`) after finishing a build or race.

## Save Files & Reference Routes

- **Save file:** `C:\Program Files\LEGO Media\Games\LEGO Racers\Save\0\LEGORac1` (slots `0..15`, persisted under `<data-root>/profile/C/PROGRAM FILES/LEGO MEDIA/GAMES/LEGO RACERS/SAVE/0/LEGORAC1`). Header (`1,152` B, magic `LR\x01\x80\x04`) + `640` B per custom driver/license/car (`1,792` B with 1 custom racer), plus circuit unlocks, trophies, and best times.
- **Committed reference routes:**
  - [`routes/first-light.route`](routes/first-light.route): unflagged cold boot through all 3 DirectShow intro movies (`lmicmp.avi`, `hvscmp.avi`, `introcmp.avi`) to animated 3D Main Menu and clean exit (`581` flips).
  - [`routes/build-racer.route`](routes/build-racer.route): unflagged cold boot → `BUILD` → `NEW RACER` → `MIX` → `MAKE LICENSE` `"ADA"` → `QUICK BUILD` → 3D Brick Builder → Exit Door `(68, 435)` → `FINISH` → persists `Save\0\LEGORac1` (`1,792` B) and exits cleanly (`1,501` flips).
  - [`routes/first-race.route`](routes/first-race.route): unflagged cold boot → `SINGLE RACE` → Imperial Grand Prix → Turbo Start → full 3D Lap 1 (Red Cannonball, Blue Shield, White Power Plus, Power Slides) → `LAP 2` → Pause → `EXIT` → Main Menu → clean exit (`6,371` flips).
