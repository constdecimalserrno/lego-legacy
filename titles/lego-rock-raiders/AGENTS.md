# LEGO Rock Raiders — Agent Playbook

Playable RTS guide for driving `lego-rock-raiders` via `lego-ctrl` or `lego-mcp` (see [docs/agent-play.md](../../docs/agent-play.md)). Every key, coordinate, and rule below is verified against retail `LegoRR.exe` and `Lego.cfg` (`LegoRR1.wad`).

## Runtime Profile

| Property | Value |
| --- | --- |
| Display | `640×480`, 16-bit, `60 Hz` (`1 flip = 16.67 ms` declared) |
| Mouse | `AbsoluteKept`, `pointer_speed = (1.0, 1.0)` — standard `move`, `click`, `drag` work directly |
| `Escape` | Safe (`escape_quits: false`): skips movies, backs out of menus, opens in-mission Pause Menu |
| Movie skip | Left-click, `Space`, or `Escape` (`click_in_a_movie: None`, handled natively) |
| Cadence | `BurstThenPause` (~`500 ms` / `30 flips` per burst) — observe/order while paused, step bursts to let Raiders work |
| Install & saves | `C:\LEGORR`; saves in `C:\LEGORR\DATA\SAVES\0.SAV`–`5.SAV` |

## Session Startup & Stepping Loop

1. Launch in `step` mode (`lego-ctrl launch lego-rock-raiders --mode step` or `game_launch`), then read `game_meta` for live `controls` and `goals`.
2. Skip the two intro movies (`lmi640.avi`, `intro.avi`) with `Space`, `Escape`, or a left-click, then `wait-change` through the loading screen until the Main Menu renders (flip `4603` if unskipped).
3. Click `Start Game` at `(320, 222)`, then step ~80 flips through the 3D rockfall until the Level Select cavern map settles (ready by flip `4700`).
4. On Level Select, un-scrolled `(155, 160)` is **`1. Moving Rock Raiders`** (Training Mission 1). To launch Campaign Mission 1 (**Driller Night!**), scroll the Level Select map down (`move 320 478`), return the pointer to the interior, and click the center cavern at `(315, 180)`; wait through the loading screen until the Mission Brief overlays the 3D cavern, then press `Space` to advance and dismiss each briefing page.
5. Loop in mission: **observe** (`screenshot --grid --cursor`) → **issue orders** (clicks or `chord F2,<Key>`) → **park pointer in safe interior `(320, 240)`** → **step 30–90 flips** → repeat until the mission reward screen appears.

Done when the current rung on the Progression Ladder holds (confirmed by screenshot, crystal bar, or reward screen).

## Controls & Verified Hotkeys

Use canonical `lego-legacy` key names (`core/src/route.rs`). Retail `Lego.cfg` sets `AllowDebugKeys FALSE` and `AllowEditMode FALSE`; debug keys (`Keypad8`/`Keypad9` speed, `F6`, `Tab`, `Ctrl+F`, `Ctrl+G`, `Keypad0`) are **inactive** in retail.

### Mouse

| Gesture | Effect |
| --- | --- |
| Left-click | Select Raider, vehicle, building, cavern wall/floor tile, or HUD icon; skips intro movies |
| Left-drag | Drag selection box to multi-select Rock Raiders |
| `LeftShift` / `RightShift` + click | Add or modify multi-unit selection |
| Right-click | Deselect current selection or cancel active placement/targeting mode |

### Camera (Top-Down View)

| Input | Effect |
| --- | --- |
| `Left` / `Right` | Rotate 3D cavern camera CCW / CW |
| `Up` / `Down` | Tilt camera pitch up / down (`0°`–`60°`) |
| `Minus` / `Equals` | Zoom camera out / in (`150`–`250` units) |
| Pointer in 4 px border | Pan cavern map (`x <= 3`, `x >= 636`, `y <= 3`, or `y >= 476`); also scrolls Level Select map vertically (e.g. `move 320 478`) |
| Click Radar (`16..167, 13..164`) | Jump camera immediately to clicked cavern location |
| Camera Widget (`410, 410`) | Zoom In `(438, 452)`, Zoom Out `(531, 451)`, Cycle Buildings / Home `(562, 449)` |

### Eye View (1st-Person) & Shoulder View (2nd-Person)

Select a Raider or Vehicle, switch view with `chord F2,1` (Eye View) or `chord F2,2` (Shoulder View), and return to overhead view with `chord F2,3` (Top View):

| Input | Effect |
| --- | --- |
| `Up` / `Down` | Move forward / backward |
| `Left` / `Right` | Turn left / right |
| `Z` / `X` | Strafe left / right |

### Standalone Retail Keys

| Key | Effect |
| --- | --- |
| `Space` | Skip intro movies; advance/dismiss Mission Brief & tutorial prompts; toggle overhead task/status bubbles in gameplay |
| `Escape` | Skip intro movies; back out of menus; open in-mission **Paused Menu** (`Continue Game`, `Options`, `Advanced Options`, `Restart Mission`, `Quit`) |
| `P` | Toggle gameplay pause directly without opening the Paused Menu |
| `S` | Toggle Sound Effects on/off |
| `M` | Toggle Red Book CD Music on/off |

**Game Speed:** Adjusted only through the Pause Menu (`Escape` or Top Panel Options `(458, 12)` → `Options` → `Game Speed` slider `0..5`, or `Advanced Options` → `Automatic Game Speed`).

### Contextual HUD Chords (`F2` + Key)

`LegoRR.exe` (`0x0041C370`) gates **all** `Interface_MenuItem_*` hotkeys behind holding **`F2`**. Send them as `chord F2,<Key>` (2+ flips) while the relevant unit, tile, or menu is selected — or left-click the matching icon on the right sidebar.

| Context | Chords (`F2` + Key) |
| --- | --- |
| Base / Root Menu | `F2+M` Teleport Raider · `F2+C` Buildings menu · `F2+V` Small Vehicles · `F2+A` Large Vehicles · `F2+B` Back |
| Cavern Wall / Floor | `F2+W` Drill Wall · `F2+G` Cancel Drill · `F2+R` Reinforce Wall · `F2+E` Dynamite / Electric Fence · `F2+Q` Clear Rubble / Geology Scan / Dam / Attack · `F2+P` Build Power Path / Repair Erosion · `F2+D` Remove Path / Cancel Construction |
| Selected Raider / Vehicle | `F2+P` Pick Up / Load · `F2+U` Drop / Unload / Upgrade Vehicle · `F2+D` Enter/Exit Vehicle / Docks · `F2+J` Upgrade Raider · `F2+C` Feed (Canteen) · `F2+I` Get Tool menu · `F2+O` Train Skill menu · `F2+T` Track Object · `F2+1`/`2`/`3` Eye / Shoulder / Top View |
| Selected Building | `F2+P` Power On/Off · `F2+J` Upgrade Building · `F2+R` Repair · `F2+H` Make Primary Teleporter · `F2+C` Eject Crystal · `F2+O` Eject Ore |
| Tool Submenu (`F2+I`) | `F2+D` Drill · `F2+S` Shovel · `F2+H` Hammer · `F2+N` Spanner · `F2+F` Freezer · `F2+L` Laser · `F2+P` Pusher · `F2+B` Sonic Blaster |
| Train Submenu (`F2+O`) | `F2+D` Driver · `F2+E` Engineer · `F2+G` Geologist · `F2+P` Pilot · `F2+S` Sailor · `F2+X` Explosives |
| Build Submenu (`F2+C`) | `F2+1` Support Station · `F2+2` Power Station · `F2+3` Ore Refinery · `F2+4` Teleport Pad · `F2+6` Docks · `F2+7` Canteen · `F2+8` Super Teleport · `F2+9` Tool Store · `F2+0` Mining Laser · `F2+Minus` Upgrade Station · `F2+Equals` Geological Center |
| Dismiss (Caution!) | `F2+S` Teleport Raider back to Explorer · `F2+D` Teleport Building or Vehicle back to Explorer |

### Verified 640×480 HUD & Menu Hotspots

| Element | Screen Rect / Center (`x, y`) |
| --- | --- |
| Main Menu: `Start Game` | `(320, 222)` (`Load` ~`y=260`, `Training` ~`y=300`, `Credits` ~`y=340`, `Quit` ~`y=380`) |
| Level Select: Training Mission 1 (**1. Moving Rock Raiders**) | `(155, 160)` (un-scrolled top-left cavern) |
| Level Select: Mission 1 (**Driller Night!**) | Scroll down (`move 320 478`), then center cavern `(315, 180)`; Back button `(6..46, 436..476)` |
| Radar Viewport | `(16..167, 13..164)` (`151×151`); Toggle `(187, 49)` |
| Radar Buttons | Map View `(32, 183)` · Tagged View `(68, 183)` · Zoom In `(108, 183)` · Zoom Out `(144, 183)` |
| Top Panel (`330, 0`) | Action Stations `(421, 12)` · Options `(458, 12)` · Priorities `(495, 12)` |
| Priorities Panel (`398, 50` when open) | Up-priority arrows reorder jobs; `X` toggles job off; Reset `(418, 251)` |
| Crystal / Ore Sidebar (`584, 0`) | Ore icon `(600, 462)` · Energy Crystal bar & icon `(625, 462)` |
| Message / Briefing Panel | Next/OK `(380, 447)` · Repeat `(380, 462)` · InfoDock Goto `(39, 376)` · Close `(19, 376)` |

## Core RTS Loop & Tech Tree

**Objective:** Mine underground caverns on Planet U to hit each mission's Energy Crystal quota, base-building target, or vehicle/Raider rescue goal before running out of oxygen or losing too many crystals.

1. **Teleport & Equip:** Click the Tool Store (or press `F2+M`) to teleport Rock Raiders down. Raiders grab drills, shovels, and hammers automatically from the Tool Store as jobs demand.
2. **Drill Walls:** Select Dirt or Loose Rock walls and drill (`F2+W` or Drill icon). Hard Rock requires Dynamite (Raider trained in Explosives at upgraded Tool Store) or a drill vehicle; Solid Rock cannot be drilled.
3. **Clear Rubble & Collect:** Clear rubble (`F2+Q`) to open paths and yield Ore. Raiders collect exposed Energy Crystals and Ore autonomously according to the **Priorities Panel** — push **Collect Crystals** or **Drill/Clear** upward when Raiders stall on lower-priority chores.
4. **Build Power Paths & Base:** `OnlyBuildOnPaths TRUE` — lay Power Paths (`F2+P`) on cleared floor first; every new building must connect to a Power Path. Follow the `Lego.cfg` dependency chain:
   - `Tool Store` (upgrade Lv1, 5 Ore) → `Teleport Pad` → `Power Station` (powers adjacent buildings via crystals)
   - `Power Station` (Lv1) unlocks `Support Station` (restores cavern oxygen + trains Drivers/Geologists), `Ore Refinery`, `Upgrade Station`, `Geological Center`
   - `Support Station` (Lv1) unlocks `Mining Laser` and `Super Teleport` (large vehicles); `Docks` unlocks water vehicles
5. **Survive Hazards:**
   - **Landslides:** Reinforce shaky walls (`F2+R`) to prevent cave-ins that bury paths and injure Raiders.
   - **Oxygen:** Missions with `OxygenRate > 0` (starting in `Level04` / `Level08`+) drain air faster with every active Raider — build a `Support Station` early and avoid over-teleporting Raiders.
   - **Creatures & Erosion:** Repair lava erosion (`F2+P` on eroding tiles) and use Action Stations / Electric Fences / Sonic Blasters against Rock Monsters and Slimy Slugs so stolen crystals do not trigger `CrystalFailure`.

## Hazards & Guardrails

- **Border Edge-Scroll (`x <= 3 || x >= 636 || y <= 3 || y >= 476`):** Park the pointer in the interior (e.g. `(320, 240)`) after every HUD or Level Select edge-scroll move; leaving it in the 4 px border continuously pans the camera off base.
- **No Bare Action Keys:** Sending bare `W`, `Q`, `R`, `E`, or `M` will not drill, clear, reinforce, dynamite, or teleport (bare `M` toggles music; bare `S` toggles SFX). Always use `chord F2,<Key>` or click the HUD icon.
- **Dismiss Chords (`F2+S` / `F2+D`):** Do not send `F2+S` with a Raider selected or `F2+D` with a building/vehicle selected unless intentionally teleporting them back to the L.M.S. Explorer.
- **Clean Shutdown:** Front-end menu loops do not poll window close (hitting core's 5 s grace timeout). Close from inside a mission or accept the 5 s stop on front-end screens.
- **Focus Loss Pauses (`LoseFocusAndPause TRUE`):** In windowed runs, focus loss pauses the game; prefer headless `step` mode for unattended agent play.

## Progression Ladder & Goals

| Goal ID | Milestone | Completion Criterion |
| --- | --- | --- |
| `boot-to-level-select` | `first_light` | Skip intros, click `Start Game` `(320, 222)`, wait past 3D rockfall to the Level Select cavern map |
| `launch-mission-1` | `first_light` | Scroll Level Select down (`move 320 478`), click Campaign Mission 1 (**Driller Night!**) at center cavern `(315, 180)` (un-scrolled `(155, 160)` is Training Mission 1 *1. Moving Rock Raiders*), dismiss Mission Brief with `Space`, confirm active 3D cavern play with Raiders moving |
| `mission-1-driller-night` | `playable` | Drill cavern walls (`F2+W`), locate the Tool Store, clear rubble, and collect all 5 Energy Crystals in **Driller Night!** |
| `advance-to-mission-2` | `playable` | Clear Mission 1's reward screen, save slot, and launch the next unlocked mission: **The Path To Power** (`Level05`, build Power Station via upgraded Tool Store, Power Paths & Teleport Pad) or **Rubble Trouble!** (`Level03`, reinforce walls, clear rubble, collect 5 crystals) |
| `campaign-complete` | `complete` | Complete all 25 campaign missions (`Level01` **Driller Night!** through `Level25` **Rocky Horror**) with profile saves in `C:\LEGORR\DATA\SAVES\` |
