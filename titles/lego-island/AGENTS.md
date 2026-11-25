# LEGO Island — Agent Playbook

First-person 3D exploration and minigame adventure (`lego-island`, `ISLE.EXE` + `LEGO1.DLL`).
For generic `lego-ctrl` / `lego-mcp` tool usage, see [docs/agent-play.md](../../docs/agent-play.md).

## Session Baseline

| Property | Value |
| --- | --- |
| Display | `640×480×16`, `60 Hz` (`1 flip ≈ 16.67 ms` declared) |
| Mouse mode | `AbsoluteKept` — direct `(x, y)` clicks/drags (`0..639`, `0..479`); 3D scene picking via `IDirect3DRMViewport::Pick` |
| Cadence | `BurstThenPause` (`500 ms` ≈ `30 flips` per burst) |
| `escape_quits` | `false` — `Escape` returns to the Information Center (or opens the Quit confirmation inside it) |
| Grid overlay | `80×80` cells: columns `A..H` (`A=0..79` … `H=560..639`), rows `1..6` (`1=0..79` … `6=400..479`) |
| Saves | `<data-root>/profile/C/PROGRAM FILES/LEGO ISLAND/` (`Players.gsi`, `History.gsi`, `G0.GS`..`G8.GS`) |

Boot timeline (untouched): first light at flip `460`, Information Center at flip `3200`, idle loop at flip `5099` (`120`-flip cycle). Clicking the floor periodically from flip `98` skips the intro and reaches the Information Center by ~flip `800`.

## Controls

### Movement & Steering

Navigation arrows are read via DirectInput (`GetDeviceState`), with `Left`/`Right` cross-checked via `GetAsyncKeyState`.

| Input | Action | Notes |
| --- | --- | --- |
| `Up` / `Keypad8` | Walk forward / accelerate | Drives bicycle, skateboard, race car, ambulance, tow truck, jetski, dune buggy, helicopter |
| `Down` / `Keypad2` | Walk back / brake / reverse | Slows or backs up on foot and in vehicles |
| `Left` / `Right` (`Keypad4` / `Keypad6`) | Turn in place | ~8°/flip; **~11–12 flips ≈ 90° turn** |
| `chord Up,Left` / `chord Up,Right` | **Steer while moving** | **Required on skateboard & vehicles** to carve turns at speed without stalling |
| `chord LeftControl,Left` / `Right` | Sharp turn | `LeftControl` or `RightControl` gives 4× rotational acceleration / 2× deceleration |
| Left hold still (`>= 1050 ms` / ~63 flips) | Mouse walk-drag | Fires the 1-second `WM_TIMER` to enter continuous pointer-steered walking |

### Mouse & World Interaction (`AbsoluteKept`)

Keep normal clicks brief (`--hold-flips 2`, well under `1000 ms`) so they register as picks rather than starting walk-drag.

- **3D World Picking (quick left click):** Enter visible buildings from up close or down the road (Pizzeria, Hospital, Gas Station, Police Station, Information Center tower), mount vehicles (red bicycle, skateboard), talk to NPCs, accept quests (click Papa Brickolini's pizza), and trigger world props.
- **Skip Intro / Welcome:** Quick click anywhere on the floor (e.g. `(600, 450)`) or press `Space`.
- **Vehicle Build Workshops (`drag`):** Left-drag pieces from the shelf onto highlighted build slots (`--step-px 16`). Click colour bricks (`Red`, `Blue`, `Yellow`, `Green`, `Black`, `Gray`) and decal/mood buttons to customise; click the Exit button to leave.
- **Info Center Controls:** Click the blue book (Registration Book), character portraits (select active citizen), left/right room arrows, Elevator top button (Observation Deck — changes sky colour & launches paper planes), Score Room History Book, radio/map props, and Quit Door.

### Keyboard Shortcuts (`ISLE.EXE` / `LEGO1.DLL`)

| Key | Effect |
| --- | --- |
| `Space` | Skip or cancel current cutscene, animation, or dialog |
| `Escape` | Return to the Information Center from the Island/activities; inside Info Center, opens the Quit confirmation (never quits immediately) |
| `F12` | Instant quicksave (`G0.GS` + `History.gsi`) whenever a player is signed in |
| `A`–`Z`, `Backspace` | Type/edit player name (up to 7 uppercase letters) in the Registration Book |
| `LeftShift` / `RightShift` | Toggle mouse auto-drag mode |

## Verified Hotspots (`640×480`)

| Location | Target | Grid | Coords `(x, y)` |
| --- | --- | --- | --- |
| Island HUD | `(i)` Return to Information Center | `A2` | `(38, 148)` |
| Info Center (Main) | Registration Book (blue brick on desk) | `E4` | `(322, 294)` |
| Info Center (Main) | Pepper portrait (3rd above map) | `E2` | `(322, 105)` |
| Info Center (Main) | Nick Brick portrait (4th above map) | `E2` | `(372, 105)` |
| Info Center (Main) | Left arrow -> Elevator room | `A4` | `(28, 310)` |
| Info Center (Main) | Quit Door (by right rail) | `H4` | `(590, 260)` |
| Info Center (Quit prompt) | Green Yes brick (save & exit) | `C6` | `(210, 400)` |
| Registration Book | Green tick — sign new name (row 1) | `G2` | `(519, 135)` |
| Registration Book | Green tick — load first saved player (row 2) | `G3` | `(519, 163)` |
| Elevator Room | Left arrow -> Revolving Door | `A2` | `(35, 140)` |
| Revolving Door | Exit to LEGO Island | `E3` | `(330, 220)` |
| Outside Info Center | Red bicycle (after stepping out & turning left) | `A4` | `(70, 271)` |

## Object of the Game & Progression Ladder

Explore LEGO Island as any of the five playable citizens (**Pepper**, **Mama**, **Papa**, **Nick**, **Laura**), complete the five island activities and four custom vehicle builds to earn Red/Yellow/Blue score bricks in the Score Cube, and complete the **Brickster Finale** (Acts 2 & 3).

### Hub Loop

1. **Sign In:** Click Registration Book `(322, 294)` -> type 1–7 letters -> click tick `(519, 135)` -> click a character portrait (selected portrait frames in blue).
2. **Exit to Island:** Left arrow `(28, 310)` -> Elevator arrow `(35, 140)` -> Revolving Door `(330, 220)`.
3. **Explore & Play:** Walk or ride (`Up`, `chord Up,Left`/`Up,Right`), click buildings/NPCs to launch activities or workshops.
4. **Return & Save:** Click HUD `(i)` `A2 (38, 148)`, click the Info Center tower, or press `Escape`. Save with `F12`, or exit via Quit Door `(590, 260)` -> Green Yes `(210, 400)` (or `lego-ctrl mode close`).

### Agent Goals (in order)

1. **`information-center` (First Light):** Skip intro, sign the Registration Book, select a character portrait, and exit through the Revolving Door onto LEGO Island.
2. **`island-exploration` (Playable):** Mount the red bicycle outside the Information Center, steer along the roads using `chord Up,Left` / `chord Up,Right`, and visit the town landmarks (Pizzeria, Gas Station, Hospital, Police Station, Beach, Race Track).
3. **`pizza-delivery` (Playable):** Click the Pizzeria, click the pizza Papa Brickolini holds out, and deliver it on the skateboard (as **Nick**, ride east then north to Nubby at the Gas Station for a Red score brick in `G0.GS`), then save.
4. **`island-minigames` (Playable):** Complete the remaining island jobs — **Ambulance** (Hospital), **Tow Truck** (Gas Station), **2-lap Race Track**, and **Jetski Race** (Beach) — to fill out the Score Cube.
5. **`vehicle-builds` (Complete):** Assemble all four vehicles (**Race Car**, **Dune Buggy**, **Jetski**, **Helicopter**) in their workshops by dragging parts onto slots. Building the **Helicopter** unlocks the finale.
6. **`brickster-finale` (Credits):** As **Pepper** with the Helicopter built, deliver a pizza to the Brickster at the Jail to launch **Act 2** (recover stolen island bricks) and **Act 3** (fly the Helicopter, throw pizzas to distract the Brickster and donuts to speed up Nick & Laura until capture) and reach the credits.

## Hazards & Quirks

- **Revolving Door stays locked until signed & picked:** Entering a new name leaves no character selected; always click a portrait (verify blue frame) before walking to the Revolving Door.
- **Vehicle steering stalls without `Up`:** On the skateboard or in vehicles, use `chord Up,Left` / `chord Up,Right` instead of bare `Left`/`Right`.
- **Walk-drag hijack (`>= 1000 ms` click):** Keep interaction clicks at `--hold-flips 2`. If a long hold triggers walk-drag, release and click cleanly once.
- **Save persistence & clean data roots:** Signing writes only `Players.gsi` (and shifts old `G0.GS` -> `G1.GS`). Progress (`G0.GS`, `History.gsi`) saves only on `F12`, Quit Door -> Green Yes, switching players, or clean `Close` (`lego-ctrl mode close`). Launch with a fresh `--data-root` for reproducible runs so saved rows don't offset Registration Book ticks.
- **Pepper's Pizza Delivery roadblock:** Pepper's pizza destination is the Jail; town roadworks block the direct road (forcing a long detour around the Police Station — and triggering Act 2 once the Helicopter is built). Pick **Nick** for the shortest clean delivery test (straight east + north to the Gas Station).
- **Vehicle Build drag precision:** Observe with `screenshot --grid --cursor` before each drag, use smooth drags (`--step-px 16`) from part center to highlighted slot, and re-verify placement on the next frame before moving to the next part.
- **Cutscene & loading pacing:** Transitions and spoken lines take 100–600+ flips while the cursor shows an hourglass. Wait for frames to settle (`wait-settled`) or press `Space` / quick floor click rather than queuing blind clicks during transitions.

## Quick `lego-ctrl` Recipe (Sign In & Steer)

```sh
lego-ctrl launch lego-island --mode step
lego-ctrl screenshot --grid --cursor -o frame.png
lego-ctrl click 322 294 --hold-flips 2          # Registration Book
lego-ctrl wait-settled 120
lego-ctrl type "LEGO"
lego-ctrl click 519 135 --hold-flips 2          # Sign (green tick)
lego-ctrl click 372 105 --hold-flips 2          # Pick Nick (or 322 105 for Pepper)
lego-ctrl chord Up,Left --hold-flips 11         # ~90° left turn while moving
lego-ctrl key F12 --hold-flips 2                # Quicksave G0.GS + History.gsi
```
