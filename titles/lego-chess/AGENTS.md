# LEGO Chess — Agent Playbook

Play full-rules 3D LEGO chess (Western, Pirates, or Classic Staunton) against the Oxford Softworks AI (difficulty `0..100`) or in hot-seat/Story/Tutorial modes. Captures in matched Western or Pirates sets play 3D slapstick cartoons; checkmate the enemy King to win.

## Session Setup

- **Display:** `640×480` 16-bit @ `60 Hz` (`SoftwareAbsolute`, `requires_poll_release: true`, `escape_quits: false`).
- **Cadence:** `PauseToThink` (`settle_ms: 1000`, `settle_flips: 20`). Launch with `--mode step` and a fresh `--data-root <dir>` for first-run profile setup (reuse the same `--data-root` to test save/reload).
- **Save files:** `profiles\prof1.lcp` (4,544 B profile) and `profiles\1save.lcs` (190,920 B, 10 slots; 8 shown in UI). Existing `prof1.lcp` skips New Player straight to `"Who's playing?"`.

## Click Semantics (Critical)

LEGO Chess uses two distinct mouse-handling loops. Pick the matching primitive every time:

| Target | Required primitive | CLI example | Why |
| --- | --- | --- | --- |
| UI buttons, green ticks, menus, dialogs, top HUD icons | `click_poll` (`hold_ms: 150`) | `lego-ctrl click 590 410 --poll --hold-ms 150` | Release loops spin waiting for button-up without flipping frames; standard `click` hangs. |
| 3D chessboard squares, expanded Rotate arrows, Throne Room world objects | standard `click` (`hold_flips: 3..6`) | `lego-ctrl click 356 308 --hold-flips 4` | Board hit-testing runs on presented flips; `click_poll` misfires or drops clicks on 3D squares. |

## 3D Camera & Board View Controls

| Action | Control | Notes |
| --- | --- | --- |
| Expand Rotate / Tilt HUD | `click_poll 320 60` (`--hold-ms 150`) | Top-center checkerboard icon (`"Rotate / tilt view"`). Reveals 4 red/yellow directional arrows. |
| Orbit left / right (HUD) | `click 282 60 --hold-flips 25` / `click 358 60 --hold-flips 25` | Hold with standard `click` while Rotate HUD is open. |
| Tilt up / down (HUD) | `click 320 24 --hold-flips 20` / `click 320 96 --hold-flips 20` | Hold with standard `click` while Rotate HUD is open. |
| Orbit / tilt (keyboard) | `chord LeftShift,Left` / `Right` / `Up` / `Down` (`--hold-flips 25`) | Active only while the Rotate HUD `(320, 60)` is expanded; ignored when collapsed. |
| Reset 3D camera | `click_poll 320 60` | Clicking the center icon again snaps back to default angle and closes the arrows. |
| Switch 3D → 2D view | `click_poll 440 60` | Knight icon. Opens flat orthographic 8×8 board; `!` Options moves to `(48, 200)` and Knight toggle moves to `(585, 200)`. |
| Switch 2D → 3D view | `click_poll 585 200` | Returns from 2D overhead board to 3D view. |

- Right-drag does not orbit the board; use the Rotate HUD `(320, 60)` arrows or `LeftShift+Arrow` (with Rotate open) for deterministic camera moves.
- Two-player (hot-seat) mode automatically rotates the 3D board 180° after each move so the side to move faces the bottom.

## Keyboard & Text Controls

| Input | Action | Timing / Caveat |
| --- | --- | --- |
| `Space` (poll-release) | Skip Krisalis' logo, intro movie, Story cutscenes, 3D capture cartoons | `lego-ctrl key Space --hold-flips 3 --hold-ms 150`. Opening LEGO Media logo (`lmi.avi`, flips `0..283`) is unskippable. |
| `Escape` (poll-release) | Open Resign dialog mid-match / back out of menus | `lego-ctrl key Escape --hold-flips 3 --hold-ms 150`. Never quits process. Confirm resign at `(360, 232)` (`click_poll`). |
| `F1` | Open in-game Settings (`Graphics` / `Sound` / `Gameplay`) | `lego-ctrl key F1 --hold-flips 3 --hold-ms 150`. Toggle `Show cartoons`, `Instant move`, `Default board view`, `Show notation`. (`Detail Level`, `Show Reflections`, and time limits are locked mid-match.) |
| `Left` / `Right` / `Up` / `Down` + `Return` | Move board square highlight & select/place piece | Works on the board in a match; per `README.TXT`, keyboard arrows cannot open or select menu items. |
| `Tab` | Cycle 6 game-screen HUD buttons | Known quirk (#235): calls `SetCursorPos`, which does not update the drawn cursor until `WM_MOUSEMOVE`. Prefer explicit coordinates. |
| `Delete` | Delete highlighted profile on `"Who's playing?"` | Prompts confirmation before deleting. |
| `type_text` | Type player name or save slot label | `lego-ctrl type "Ada" --flip-gap 4`. Text boxes enforce `>= 50 ms` declared-time `WM_CHAR` debounce (4+ flips @ 60 Hz) and key-up between chars. Clear default `....` save labels with 4× `key Backspace --hold-flips 3`. |

## Screen Maps & Verified Coordinates (`640×480`)

### Boot & Profile Register
- **New Player form** (empty `--data-root`, flip `~307+`): name box is focused automatically; type name (`--flip-gap 4`), then `click_poll 576 420` (green tick).
- **"Who's playing?"**: highlighted player enters Throne Room via `click_poll 576 420` (green tick).

### Throne Room Hub
The room camera pans toward the pointer, shifting what sits under it. Always `move`, `wait 25`, observe, then click.

| Object | Location | Destination |
| --- | --- | --- |
| Chess Table | `(385, 285)` after camera settles (`click --hold-flips 2`) | **Quick Game** setup |
| Red Curtains | Back wall behind King | **Story Game** (Wild West & Pirates campaigns) |
| Open Book on Pedestal | Mid-right behind table | **How to Play** (King's Study: Basic, Advanced, Tactics) |
| Green Crystal Ball | Far right on stand | **Settings** (Graphics, Sound, Gameplay) |
| Wooden Trapdoor | Bottom-center floor | **Quit Game** |
| Scrapbook & Blue Scissors | Bottom-left floor | **Scrapbook** (replay unlocked capture cartoons & certificates) |
| Treasure Chest | Mid-left behind King | **Load Game** |
| Window | Top-left | **Network Game** |
| Quill & Inkpot | Far-left throne armrest (pan camera left) | **Sign Register / Switch Player** |

### Quick Game Setup (all `click_poll`)
- **Choose Theme `(320, 70)`:** cycles **Western** (default) → **Pirates** (1 click) → **Classic Staunton** (2 clicks); auto-matches both piece sets.
- **Choose Background `(320, 200)`:** cycles unlocked arenas for the chosen theme.
- **Piece Sets `(115, 85)` White / `(525, 85)` Black:** mixing sets or using Classic suppresses 3D capture cartoons.
- **Player Toggle `(120, 180)` White / `(520, 180)` Black:** Human vs Computer AI (default: White Human, Black Computer).
- **AI Difficulty:** slider bar `x=140..500, y=310`, fine-tune `-` `(100, 310)` / `+` `(540, 310)`. Range `0..100` (default `25` plays random legal moves 75% of turns; `100` runs full alpha-beta search, ~1–5 declared seconds out of book).
- **Game Type `(320, 400)`:** Standard Chess vs Pawns mini-games.
- **Cancel `(50, 410)`** / **Start Match `(590, 410)`** (green tick).

### In-Match HUD (all buttons `click_poll`)
- **3D Top Row:** White Advisor `(60, 75)` | `!` Options `(200, 60)` | Rotate/Tilt `(320, 60)` | 2D/3D Knight `(440, 60)` | Black Advisor `(580, 75)`.
- **2D Side Row:** `!` Options `(48, 200)` | 3D Knight `(585, 200)` | Advisors stay at `(60, 75)` / `(580, 75)`; Rotate icon hidden.
- **Advisor Help Menu** (`click_poll 60 75` on White's turn; click again to close):
  - Pointer Style `(275, 190)` | Show Legal Moves `(365, 190)` | Hint `(275, 260)` | Undo `(365, 260)`.
  - Flags beside Advisor portraits drop down their poles as material is lost.
- **`!` Options Menu** (`click_poll 200 60` in 3D or `(48, 200)` in 2D):
  - Set Up Board (2D editor; remove pieces via wastebin, not off-board), **Save Game `(282, 258)`**, **Load Game `(360, 258)`**, Swap Sides, Resign. Confirm Save/Load green tick: `(392, 194)`.
- **Match End Dialogs:** Checkmate green tick `(320, 232)` → Throne Room; Resign confirm green tick `(360, 232)` → Quick Game.

## Board Square Coordinates

### Default 3D Camera (White at bottom, `640×480`)
Perspective foreshortens ranks upward and tapers files inward:

| Rank | Center `y` | Verified square `(x, y)` anchors |
| --- | --- | --- |
| Rank 1 | `349` | `d1=(282,349)`, `f1=(435,349)`, `g1=(511,349)` |
| Rank 2 | `308` | `d2=(284,308)`, `e2=(356,308)` |
| Rank 3 | `272` | `d3=(287,272)`, `e3=(353,272)`, `f3=(419,272)` |
| Rank 4 | `241` | `c4=(228,241)`, `e4=(351,241)` |
| Rank 5 | `214` | `h5=(521,214)` |
| Rank 6 | `~196` | interpolate between Rank 5 and Rank 7 |
| Rank 7 | `180` | `f7=(396,180)` |
| Rank 8 | `~165` | top back row just below HUD icons |

### 2D Overhead Board (`click_poll 440 60`)
Exact uniform 8×8 grid spanning `x=144..496`, `y=24..344` (`44×40` px per square):
- `x(file) = 166 + 44 * file_index` (`a=0 .. h=7`)
- `y(rank) = 44 + 40 * (8 - rank)` (`rank 8` top, `rank 1` bottom)

Switch to 2D whenever exact square targeting matters, then return to 3D via `click_poll 585 200` for capture cartoons and camera views.

## Hazards & Recovery

- **Clean shutdown only outside a match:** `Lego Chess.exe` ignores `WM_QUIT` mid-match and hits the 5 s close grace. Resign (`Escape` poll-release → `click_poll 360 232`) or finish via Checkmate green tick (`click_poll 320 232`) before sending `close`.
- **Click-mode mismatch:** Using standard `click` on UI buttons hangs; using `click_poll` on 3D board squares misfires.
- **Capture cartoons missing:** Verify `Show cartoons` is enabled (`F1` → Graphics) and both sides use the same Western or Pirates piece set (mismatched sets and Classic Staunton skip cartoons).
- **AI turn wait:** After a legal White move, wait for piece walk animation + AI think time + Black piece animation (`wait-settled` or `wait 60..150`; up to ~300 flips at difficulty `100` out of book) before clicking the next square.

## Concrete Agent Goals

1. **Create Profile & Enter Throne Room (`playable`):** Skip Krisalis' logo and intro with `Space` (`--hold-ms 150`), type a player name (`--flip-gap 4`), confirm `(576, 420)` twice, and reach the Throne Room with the King visible.
2. **Play 3D Chess vs AI & Trigger Capture Cartoons (`playable`):** Open Quick Game `(385, 285)` → green tick `(590, 410)`, play legal moves (`click --hold-flips 4`), execute a capture in matched Western or Pirates sets, and observe the 3D capture cartoon (or skip with `Space` poll-release).
3. **Inspect 3D Camera & Toggle 2D/3D View (`playable`):** Expand Rotate/Tilt `(320, 60)`, orbit with `(282, 60)` / `LeftShift+Left`, tilt with `(320, 24)`, snap back via `(320, 60)`, toggle 2D via `(440, 60)`, and return to 3D via `(585, 200)`.
4. **Checkmate AI in Western & Pirates Themes (`complete`):** Defeat the Computer AI by checkmate in default Western and in Pirates (1 click on Theme `(320, 70)`), reaching `"Checkmate! Congratulations, you win!"` and returning to the Throne Room via `(320, 232)`.
5. **Save & Reload Match Across Runs (`complete`):** Open `!` Options `(200, 60)` → Save Game `(282, 258)`, clear `....` (4× `Backspace`), type a label (`--flip-gap 4`), confirm `(392, 194)`, resign cleanly (`Escape` → `(360, 232)`), relaunch with the same `--data-root`, reload via Options → Load Game `(360, 258)` → `(392, 194)`, and play another legal move on the restored board.
