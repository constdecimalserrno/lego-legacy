# LEGO Alpha Team — Agent Playbook

Isometric 3D puzzle-action game (`LEGOATeam.exe`, NetImmerse / Direct3D 7). Place and orient Action Bricks, connect power/belts/ropes, and press **GO** to trigger chain reactions that clear Evil Orbs, rescue teammates, and unlock each room's exit.

## Quick Profile

- **Title key:** `lego-alpha-team` (`640×480`, 16-bit, **30 Hz** — `33.3 ms/flip`)
- **Mouse regime:** `RelativeSteered` (`virtual_span: [1280, 1280]`, `recenter: [320, 240]`, `start_corner: [0, 480]`, auto-scale `(2.0, 8/3)`)
- **Cadence:** `PauseToThink` (`settle_flips: 30`, `settle_ms: 1000`)
- **Safety:** `escape_quits: false` (`Escape` skips the intro from ~flip 440 onward and opens/closes the Mission Menu in normal missions like `M1010`/`M1020`, but is disabled during `M0010` Training Area — click Menu button `(61, 430)` there)
- **Save file:** `C:\Program Files\LEGO Media\LEGO Alpha Team\Game.txt` (6 adventure slots: `P1Space`..`P6Space`, `HighSpace`)
- **Control tools:** see [`docs/agent-play.md`](../../docs/agent-play.md) for `lego-ctrl` and `lego-mcp` usage

## Boot & Menu Flow

1. **Launch:** `lego-ctrl launch lego-alpha-team --mode step` (or `game_launch`).
2. **Intro movie (`LMIINTRO.AVI`):** First non-black frame at flip `5`; runs to flip `1631` untouched. The title ignores all input during the developer logo (~flips `0..439`). From **~flip 440 onward**, send `Escape` or a left click (`click_in_a_movie: 0x01`) to skip straight to the Direct3D Loading panel (`LOADING_PANEL` at flip `1633` unskipped).
3. **Attract screen ("Click the mouse to begin!"):** Send `Space` (`hold_flips: 2`) or a left click. Main Menu (`MM.PUZ`) is ready once settled (flip `2112` on an unskipped boot).
4. **Main Menu:** Vertical list centered horizontally — **Adventure** (top), **Single Missions** (second), **Options**, **Exit Game**.
   - **Adventure:** Opens **Select Adventure** (6 save slots). Pick slot 1 (`New`) → click the bottom-right `X` on the Adventure Briefing to start at the Training Area (`M0010`), or click the Menu button at `(61, 430)` (`Escape` is disabled in `M0010`) → **Skip Training Mission** to jump straight to Mission One (`M1010`) and write `Game.txt`.
   - **Single Missions:** Pick a mission from the list, then click the bottom-left checkmark to launch.

## `RelativeSteered` Mouse Controls (Critical)

`DDMouse` hides the OS cursor, draws a software cursor (`Cursor_Arrow.tga`), and calls `SetCursorPos(320, 240)` **every flip**. Each raw delivered pixel away from `(320, 240)` moves the drawn cursor by `1/1280` of the screen on both axes (`0.5 px` X, `0.375 px` Y on `640×480`).

- **Steer with `mouse_delta`:** `ControlHost` auto-scales `(dx, dy)` by `(2.0, 2.6667)` so `dx`/`dy` are in visual `640×480` pixels. The cursor starts at **`(0, 480)` (bottom-left)** on the front end.
- **Spread large moves across flips (avoid `#332` clamping):** One flip can only carry `±319` raw X (`±159.5` visual px) and `±239` raw Y (`±89.6` visual px) before clamping to `0..639 × 0..479`. Keep each flip under `±140 px` X and `±80 px` Y (e.g. `flips: 4..6` for cross-screen moves).
  ```sh
  lego-ctrl delta 320 -240 --flips 5
  ```
- **Confirm cursor visually before every click:** Take a screenshot (`lego-ctrl screenshot --grid` or `game_observe`) after steering and verify the drawn software cursor sits on the button or brick before clicking.
- **Click in place (omit `x`/`y`):** Pass `{"action":"click","hold_flips":2}` (or `lego-ctrl click 320 240 --hold-flips 2`, which has zero offset from recenter). Passing non-center `x`/`y` to `click` or using `move`/`drag` injects an absolute `Input::Move` that `DDMouse` reads as a sudden relative jump off target.
- **Drag movable Action Bricks via piped route:** Steer onto the brick, verify visually, then pipe a `#lego-legacy-route 1` sequence holding Left button while emitting small offsets from `(320, 240)` and releasing `up` on the target pad (pattern in [`routes/first-mission.route`](routes/first-mission.route#L145-L162)). Use explicit `flip +N` offsets (e.g. `flip +0 down`, `flip +4 move 304 219`, `flip +12 up`) for relative scheduling from the current flip, or bare `flip N` for absolute flip numbers (matching committed `.route` files; pass `--relative` only if piping bare `flip 0`..`flip N` offsets). Run `lego-ctrl clear` (`POST /api/clear-queue`) if you ever need to cancel queued pipe events cleanly. Verified piped relative-drag scaling is `0.5 px/raw_x` and `0.375 px/raw_y`; carry Action Bricks at `<= 8 screen px/flip` (`= 16 raw px/flip` in X) so the brick stays attached to the pointer.
- **Right-drag orbits the 3D camera:** Holding Right button (`down right` .. relative moves .. `up right`) orbits the mission camera around the puzzle room.

## Controls & HUD Layout (`640×480`)

### Verified Keyboard Controls

| Input | Action | Notes |
| --- | --- | --- |
| `Space` | Go / Stop | Starts simulation (`GO`) or stops & resets agents/props (`Stop`); also advances the attract screen. Ignored during the intro movie. |
| `Escape` | Skip intro / Mission Menu (normal missions) | Skips intro movie from ~flip `440` onward; opens/closes the Mission Menu in normal missions (`M1010`/`M1020`) or cancels an active connection. Disabled during `M0010` Training Area (click Menu button `(61, 430)` instead). Never quits (`escape_quits: false`). |
| `Tab` / `Shift+Tab` | Next / Prev camera preset | Cycles preset mission camera angles (`NextCam` / `PrevCam`). |
| `R` | Rotate counter-clockwise (90° CCW) | Rotates the selected Action Brick **90° counter-clockwise (CCW)**, not clockwise. Wait `~0.5s` (`~15` flips) to settle after clicking a tile to open its Unit Menu before sending `R`. |
| `Z` | Zoom camera out | Zooms out cleanly (`PageDown` does not zoom out). Camera keys recenter the cursor. |
| `Q` / `W` | Orbit camera | Orbits the 3D mission camera left / right. Camera keys recenter the cursor. |
| `Up` / `Down` / `Left` / `Right` | Pan camera | Pans the 3D mission camera while held. Camera keys recenter the cursor. |
| `A` / `S` / `X` / `PageUp` | Additional camera axes | Fine camera zoom / pitch / elevation keys read by `PE0300.DLL` (`PageDown` does not zoom out; camera keys recenter the cursor). |

> No built-in fast-forward hotkey exists in `LEGOATeam.exe`; control simulation speed with `lego-ctrl mode live <pace>` or `step` mode.

### Bottom HUD Bar (`y ≈ 428`)

- **Planning mode (before `GO`):**
  - **Menu (book):** `(61, 430)` — pauses mission (`Keep Playing`, `Start Over`, `Skip Training Mission` in training, `Options`, `Return to HQ`, `Exit Game`); required during `M0010` Training Area where `Escape` is disabled
  - **`PrevCam`:** `(215, 428)` | **`Nav_InOut` (zoom):** `(283, 428)` | **`Nav_Pivot` (8-way orbit):** `(348, 428)` | **`NextCam`:** `(412, 428)`
  - **`GO / Stop`:** `(573, 428)`
- **Action mode (while simulation is running):**
  - **Team follow-cams (`y ≈ 428`):** Flex `T0016` `(170)`, Cam `T0014` `(220)`, Charge `T0012` `(270)`, Radia `T0015` `(320)`, Crunch `T0013` `(370)`, Dash `T0011` `(419)`
  - **`ICam` (Longview):** `(470, 428)` | **`SmartCam` toggle:** `(534, 433)` | **`Stop`:** `(573, 428)`
- **Screen-edge camera scroll:** Moving the software cursor within `1/40` of any screen edge (`x <= 16`, `x >= 624`, `y <= 12`, `y >= 468`) pans the view.

### Action Bricks, Unit Menu & Specialists

- **Unit Menu:** Short left-click + release on a movable Action Brick opens its floating Unit Menu directly above it (wait `~0.5s` / `~15` flips to settle before pressing `R` to rotate 90° CCW): **`RotateCW`**, **`RotateCCW`**, **`Info`** (magnifier — zooms in and reads unit description; click unit again to exit), **`Hint`** (lightbulb — uses a mission hint if available), plus unit-specific controls:
  - **Toggle button** on Horizontal/Vertical Mirrors (`U0200`/`U0132`) and Teeter-Totters (`U0027`).
  - **Attach buttons** for **Belts** (red wheels: Motor ↔ Belt-driven device / Belt Box), **Electric Cords** (blue sockets: Generator ↔ Electric device), and **Ropes** (orange loops: Tugger/Object ↔ Rope Lock / Rope Guide).
- **Common Action Bricks:** Direction Tile (`U0179`, turns walking agents), Jump Tile (`U0177`, launches upward), Stairs (`U0164`), Slide (`U0033`), Trampoline (`U0023`), Platform (`U0016`, stackable), See-Saw (`U0206`) & Teeter-Totter (`U0027`), Puncher (`U0026`), Turbo Tube (`U0184`), Remote Control Bomb (`U0197a`) + matching Plunger Detonator (`U0197b`, flashes when in range), Laser (`U0198`/`U0204`) + Mirrors (`U0200`/`U0132`), Catapult (`U0017`), Antigravity Device (`U0188`). Fixed **Training Tiles (`U0205`)** cannot be moved.
- **Specialists (6 playable + 1 HQ coach):**
  - **Dash** (`T0011`, Motion — Diving Board `U0004`, tiles/trampolines)
  - **Charge** (`T0012`, Electrical — Portable Elevator `U0186`, generators/cords/antigravity)
  - **Crunch** (`T0013`, Explosives — Rolling Bomb `U0057`, remote bombs/detonators)
  - **Cam** (`T0014`, Motors — Ejector Seat `U0201`, motors/belts)
  - **Radia** (`T0015`, Lasers — Laser Launcher `U0022`, lasers/mirrors)
  - **Flex** (`T0016`, Ropes — Helmet Hanger `U0209`, ropes/guides/catapults)
  - **Tee Vee** (`T0001`) — HQ coordinator & Training Area coach (not deployed onto puzzle tiles).

## Puzzle Loop

1. **Inspect:** Read the mission briefing goal, pan/cycle cameras (`Tab`, `PrevCam`/`NextCam`, right-drag), locate agents, Evil Orbs, enemies (Guards, Troopers, Zombies, Lurchers), Trouble Tubes (trapped teammates), and door locks (flashing Warning Lights show remaining locks).
2. **Plan & place:** Drag movable Action Bricks onto floor pads/tiles (`<= 8 screen px/flip` = `16 raw px/flip` at `0.5 px/raw_x`, `0.375 px/raw_y`), click to open their Unit Menu (wait `~0.5s` settle) to rotate (`RotateCW`/`RotateCCW` icons, or `R` for 90° CCW) or wire Belts/Cords/Ropes.
3. **Execute:** Press `Space` or click `GO` `(573, 428)`. Watch agents move automatically; grabbing all **Evil Orbs** clears zombies/guards blocking locks, pulling levers frees trapped teammates, and tripping all locks opens the exit door.
4. **Iterate:** If a chain reaction misses or an agent hits a hazard (`Team Member Down!`), press `Space` / click `Stop` `(573, 428)` (or `Try Again`) to reset all units to their setup positions, adjust brick positions/orientations, and re-run until `We did it!` appears.

## Hazards

- **Cursor clamping & drift (`#332`):** Single-flip `mouse_delta` beyond `±159.5` X / `±89.6` Y visual px clips silently; absolute `move`/`drag` or `click` with non-`(320, 240)` coordinates jumps the pointer off target. Always use multi-flip `mouse_delta`, verify the drawn cursor in a screenshot, and click in place.
- **Accidental edge-scrolling:** Parking the cursor in the outer `16 px` left/right or `12 px` top/bottom margin continuously pans the camera. Move back toward center after edge-panning.
- **30 Hz timing:** Every second of declared time is 30 flips (`33.3 ms/flip`), not 60.
- **Camera keys recenter the cursor:** Pressing `Z` (zoom out; `PageDown` does not zoom out), `Q`/`W` (orbit), or arrow keys (pan) recenters the cursor — re-verify cursor position visually before clicking.
- **Training Area script & `Escape` locks:** In `M0010`, `Escape` is disabled (click the Menu button at `(61, 430)` instead; `Escape` works in normal missions `M1010`/`M1020`). Tee Vee (`T0001`) speaks each step and flashes the single permitted target; nothing else can be picked until step `220a` unlocks Direction Tile `U0179`, and each step only advances after the voice line finishes whole (`#85`, `#92`).
- **Intro lock-out:** Clicks and keys before ~flip `440` are ignored by the game (`#66`).
- **Display options:** Keep `640×480` 16-bit in Options (higher resolutions or 32-bit BPP break display output; `Game.txt` stores the override).

## Progression Ladder & Agent Goals

1. **`boot-main-menu` (`first_light`):** Boot `lego-alpha-team`, skip or wait past `LMIINTRO.AVI`, clear the attract screen, and reach Main Menu (`MM.PUZ`). Done when Main Menu options (`Adventure`, `Single Missions`) are visible.
2. **`training-area` (`playable`):** Complete `M0010` (*Alpha Team Training Area*) — follow Tee Vee's steps, drag Direction Tile `U0179` onto the blue pad, rotate via Unit Menu, press `GO` so Dash reaches the console, and return to HQ.
3. **`adventure-m1010` (`playable`):** Start Adventure slot 1 (skipping or finishing training), place and orient Direction Tiles in **Mission One (`M1010`)**, press `GO`, guide Dash to the Plunger Lock, and clear the mission (`We did it!`).
4. **`adventure-m1020-compound-1` (`complete`):** Complete **Mission Two (`M1020`, *On the Right Path*)** using its 4 Direction Tiles, then clear Ogel Island (`M1010`–`M1140`), rescuing **Crunch** (`M1070`) and **Radia** (`M1140`).
5. **`full-adventure` (`credits`):** Complete Goo Caverns (`M2010`–`M2160`, rescue **Charge** & **Cam**), Deep Sea Orb Factory (`M3010`–`M3100`, rescue **Flex** & shut off D.O.O.M.), and Arctic Command Base (`M4010`–`M4070`, stop the Boggle Rocket) to reach **The Victory Party! (`Credits`)**.

## Reference Files

- Title declaration & `GUIDE`: [`src/lib.rs`](src/lib.rs)
- Verified headless routes: [`routes/menu.route`](routes/menu.route), [`routes/first-mission.route`](routes/first-mission.route), [`routes/select-adventure.route`](routes/select-adventure.route)
- Integration tests & frame constants: [`tests/alpha_team.rs`](tests/alpha_team.rs)
