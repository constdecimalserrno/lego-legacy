# LEGOLAND (`legoland`)

Agent playbook for driving `Legoland.exe` (Masterpiece pressing) via `lego-ctrl` or `lego-mcp`. For generic CLI/MCP commands and session setup, see [`docs/agent-play.md`](../../docs/agent-play.md). Controls, coordinates, hazards, and save paths below are verified from primary sources (`Legoland.exe` disassembly, `Graphics1.res`, `Graphics2.res`, `Legoland.res`, `strings/stab.str`, and `readme.txt`); live boot flip timings will be added once first-light bringup (#394) lands.

## Runtime Facts

- **Display:** `640×480`, 16bpp RGB565 (8bpp fallback), pure 2D software rasterizer + Z-buffer compositor over DirectDraw2 (`FlipFullscreen` / `FlipWindowed` spin on a `28 ms` / `~35.7 Hz` floor via `timeGetTime()`/`GetTickCount()`; declared at the roster baseline `FlipCost::Refresh(60)` until first-light timing is measured), declared Pentium II 450 MHz / Windows 98 SE.
- **Mouse regime:** `SoftwareRelative { virtual_span: (640, 480), start_pos: (320, 240) }` (`1.0×` scale, `recenter_every_flip: false`): `Legoland.exe` imports no `GetCursorPos`/`SetCursorPos`/`ClipCursor`, blanks the OS pointer via `SetCursor(NULL)` + `DISCL_EXCLUSIVE` DirectInput `GUID_SysMouse` acquisition, initializes its software pointer once at `(320, 240)` (`0x4589a0`), integrates relative `(lX, lY)` deltas clamped to `[0..639, 0..479]`, and blits software cursor sprites (`rab over icon.lls`, `yes/no build.lls`, `erase it.lls`, `question it.lls`) with a `(0, 0)` top-left hotspot.
- **Intros & skipping:** Unflagged boot plays `lmi.avi` (`0x45956b`, skips only via `LControl+Q` or `RControl+Q`) followed by `Intro.avi` (`0x4595e7`, skips on `Space` or any mouse button). Committed routes must run unflagged (do not use `-nointro` / `-nomusic`).
- **Cadence:** `PauseToThink { settle_ms: 800, settle_flips: 16 }` — isometric park management and release-edge UI menus respond best to settled clicks and `wait-change` / `wait-settled`.
- **Exit guard:** `escape_quits: false` — `Escape` closes `PopUpInfo`, drops a held worker, opens Pause/Options in-game, backs out of sub-menus, or opens the Exit Yes/No dialog on Title Screen (never hard-exits).

## Object of the Game & Progression Goals

Build and manage themed LEGO parks across 5 Tutorials, 10 Miniland Campaign Levels, and Free Play: place rides, shops, power stations, and scenery, link paths to the park entrance, direct Mechanics and Gardeners, and pass Mr. Bimble's Park Inspector appraisals.

1. **Boot to Title Screen (`boot-main-menu`, FirstLight):** Clear `lmi.avi` and `Intro.avi`, create/select a profile on `Player Details` (`InitListProfiles`), click `Accept` `(562, 402)`, and reach the `Title Screen` with `Start Game` unlocked.
2. **Clear Tutorial 1 (`tutorial-1-first-park`, Playable):** Enter `Progress Map` → Tutorial Screen → Lesson 1 (`ObjList1.txt`, map `"one"`), place `SPACE TOWER RIDE`, drag-link `LEGO SHOP 1` with `Path`, demolish `LEGO SHOP 1` with `Erase`, and complete the lesson.
3. **Clear Level 1 San Francisco & Save/Reload (`level-1-san-francisco`, Playable):** Complete `ObjList6.txt` (Money Master, map `"GLONE"`) across LEGOLAND and Western themes while holding power, ride access, savings, and Inspector metrics, and verify `profiles\%dsave%d.sav` save/reload across a cold restart.
4. **Free Play Sandbox (`free-play-sandbox`, Playable):** Enter `Free Play` `(233, 82)` (`FreePlayTest.txt`), confirm all 4 theme tabs populate, place a ride, path, and power station, and exit cleanly.
5. **Complete All 10 Miniland Levels (`all-10-minilands`, Credits):** Clear Campaign Levels 1–10 (`ObjList6.txt`–`ObjList15.txt`) to reach the Credits and Certificate screen (never click `Print_On_Certificate.lls`).

## Verified Controls (`Legoland.exe` & `readme.txt`)

### Mouse & Camera Controls

| Input | Action | Verified behavior |
| --- | --- | --- |
| `Left click` (`LMB`) | Activate UI / Place / Drag / Pick Up | UI icons fire on **button release** (`test arg, 2`) after hover latches (`[0x6687d0]`). Places selected ride/scenery/power, drag-paints paths/roads/fences, erases in Erase mode, picks up/drops Mechanics and Gardeners, and opens `PopUpInfo` in Normal mode. |
| `Right click` (`RMB`) | Cancel Build / Erase / Zoom | Cancels active Build or Erase mode back to the Normal/Query cursor (`[0x8119b0] = 0`) and exits `LoadZoomer`. |
| `Middle click` (`MMB`) | Inspect (`PopUpInfo`) | Opens `PopUpInfo` directly on the hovered tile or attraction. |
| `Mouse Wheel` | Scroll Build Tray | Scrolls the active build/category tray (`DIMOUSESTATE.lZ` threshold `±60`). |
| `Left` / `Right` / `Up` / `Down` | Pan Park Map | Pans the isometric camera (`±8192` X, `±4096` Y) when no popup is open. |
| Screen-edge hover (`< 8 px`) | Edge Auto-Scroll | Moving within `8 px` of any screen edge (`x < 8`, `x > 631`, `y < 8`, `y > 471`) outside HUD icons scrolls the camera. |
| `Space` | Skip `Intro.avi` | Skips `Intro.avi` (mouse buttons also skip `Intro.avi`). |
| `LControl+Q` / `RControl+Q` | Skip `lmi.avi` | Skips the opening `lmi.avi` logo movie. |
| `Escape` | Back / Close Popup / Pause | Closes open `PopUpInfo`, drops a held worker, opens Pause/Options in-game, backs out of Options/Progress/Profile, or opens the Exit Yes/No dialog on Title Screen. |
| `Backspace` | Erase Tool / Text Edit | Deletes characters in text entry (`WM_CHAR`); switches to Erase/default cursor in-game. |

> **Absent controls (do not invent):** No camera rotation, no continuous zoom, no building/ride rotation, no in-game fast-forward key (use `--pace`), and no dedicated pause hotkey outside `Escape` / Options `(359, 445)`.

## Verified `640×480` Hotspots (`Legoland.exe` + `Graphics1.res`)

Safe Click centers are listed only where both top-left `(x, y)` and sprite `[w×h]` are directly verified; entries marked `verify visually` should be confirmed on screen once first light lands.

| Screen / Element | Asset / Purpose | Top-Left `(x, y)` | Size `[w×h]` | Safe Click `(x, y)` |
| --- | --- | --- | --- | --- |
| **Title:** Player Details | `Reg_on_Title.lls` | `(24, 113)` | `158×145` | `(103, 185)` |
| **Title:** Free Play | `Free_on_Title.lls` | `(154, 8)` | `158×148` | `(233, 82)` |
| **Title:** Start Game *(needs active profile)* | `New_on_Title.lls` | `(202, 312)` | — | verify visually |
| **Title:** Load Game | `Load_on_Title.lls` | `(25, 280)` | `158×158` | `(104, 359)` |
| **Title:** Movie / Adverts | `MovieOn_On_Title.lls` | `(270, 161)` | `105×90` | `(322, 206)` |
| **Title:** Exit Game | `Exit_On_Title.lls` | `(481, 19)` | `153×149` | `(557, 93)` |
| **Profile Details:** Slots `0..7` | `RegProfileON/Off_*.lls` | `(128, 134+38*i)` | — | verify visually |
| **Profile Details:** Accept | `Accept_On_Reg.lls` | `(495, 335)` | `135×135` | `(562, 402)` |
| **Progress Map:** Tutorial Screen toggle | `Tutorial_On_Progress.lls` | `(372, 365)` | — | verify visually |
| **Progress Map:** Accept / Play | `Accept_on_Progress.lls` | `(526, 367)` | `103×103` | `(577, 418)` |
| **Progress Map:** Go Back | `GoBack_on_Progress.lls` | `(520, 11)` | `112×112` | `(576, 67)` |
| **Tutorial Screen:** Accept | `Accept_On_Report.lls` | `(522, 364)` | `110×110` | `(577, 419)` |
| **Options:** Exit | `Exit_on_Options.lls` | `(480, 6)` | `153×149` | `(556, 80)` |
| **Options:** Save | `Save_on_Options.lls` | `(479, 189)` | `158×145` | `(558, 261)` |
| **Certificate:** Go Back | `GoBack_On_Certificate.lls` | `(12, 14)` | `110×110` | `(67, 69)` |
| **Certificate:** Print *(HAZARD)* | `Print_On_Certificate.lls` | `(519, 14)` | `110×110` | **DO NOT CLICK** |
| **HUD Control Tools:** Path / Query / Erase / Map / Options | Bottom action buttons | `(12..332, 418)` step `80` | `54×54` | `(39,445)`, `(119,445)`, `(199,445)`, `(279,445)`, `(359,445)` |
| **HUD Theme Tabs:** LEGOLAND / West / Castle / Adv | Bottom category tabs | `(8,379)`, `(105,379)`, `(202,379)`, `(299,379)` | — | verify visually |
| **HUD Advisor:** Briefing Phone | `BriefIcon2.lls` | `(522, 378)` | `100×100` | `(572, 428)` |

## Input Hazards & Workarounds

- **Windows Print Trap (`0x451660..0x451770`):** Clicking `Print_On_Certificate.lls` `(519, 14)` invokes `EnumPrintersA`/`CreateDCA`/`StartDocA` and attempts to minimize fullscreen DirectDraw. Never click Print in automated runs.
- **8 px Screen-Edge Auto-Scroll (`0x4614f0`):** Leaving the cursor inside `x < 8`, `x > 631`, `y < 8`, or `y > 471` over non-HUD pixels pans the park camera. Park the idle cursor inside `[64..576, 64..370]`.
- **Release-Edge & Hover-Lock UI:** UI icons trigger on button release (`test arg, 2`) and swap sprites on hover (`[0x6687d0]`); always settle hover (`settle_flips >= 1`) before clicking.
- **AVI Skip Asymmetry:** `lmi.avi` skips only via `LControl+Q` / `RControl+Q`, whereas `Intro.avi` skips on `Space` or mouse click.
- **Appraisal Failure (`APPRAISAL 10` → `Lose.txt`):** Leaving rides unlinked or unpowered, or missing Inspector food/scenery/visitor metrics, triggers repeated failed appraisals and closes the park.
- **Debug Cheats Forbidden:** Typed `:` cheat codes (`:COLDHARDCASH`, `:PRAISEME`, `:ILIKETOTRAVEL*`, etc.) exist in `0x473c10` and must never be used in genuine routes.

## Save Files

- **Save directory:** `C:\Program Files\LEGO Media\Games\LEGOLAND\profiles\` (`profiles\Profile%d.txt`, `profiles\%dsave%d.sav`, `profiles\%dsave%d.sh` with header `"00002 LEGOLAND Save Game V0.02 \x1a"`, persisted under `<data-root>/profile/C/PROGRAM FILES/LEGO MEDIA/GAMES/LEGOLAND/PROFILES/`).
