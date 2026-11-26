# Changelog: lego-legacy-core

One heading per version, newest first. What each part of core's version means is in
[ADR-0007](../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.2.0 — 2026-10-21

Direct3D 3 Immediate Mode, DirectInput 2 `Poll`, and Win32 boot shims for LEGO Racers first light (`core/src/{cli,run,com,machine}.rs`, `core/src/win32/{mod,advapi32,ddraw,d3d,dinput,gdi32,kernel32,user32,winmm}.rs`, #370), all strictly generic and true to Windows with zero title-keyed behavior and exhaustive `LLSNAP v2` `Snap` coverage:

- **Win32 boot shims:** Adds `ADVAPI32!RegOpenKeyA`, `DDRAW!DirectDrawCreateClipper`, `KERNEL32!SetPriorityClass`, `KERNEL32!SetEnvironmentVariableA` (with case-insensitive per-process overrides/deletions in `State::environment` honored by `GetEnvironmentVariableA` and `GetEnvironmentStringsA`/`W`), `USER32!{BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, EnumDisplaySettingsA, ChangeDisplaySettingsA, GetUpdateRect, FillRect}`, `GDI32!{CreateSolidBrush, DeleteObject}`, `WNDCLASSA.cbWndExtra` byte storage across `SetWindowLongA`/`GetWindowLongA` (readable through `WM_DESTROY`/`WM_NCDESTROY`), and `WINMM!{timeSetEvent, timeKillEvent}` timer ID registration/cancellation (periodic callback dispatch owned by #371).
- **Direct3D 3 IM & COM:** Adds `IDirectDraw -> IDirect3D3` `QueryInterface` routing in `com::FACES`, implements `IDirect3D3::CreateDevice` (slot 8), `IDirect3DDevice3::GetCaps` (slot 3), `IDirect3DDevice3::EnumTextureFormats` (slot 8), `D3DTSS_ADDRESS (12)` propagation to `ADDRESSU (13)`/`ADDRESSV (14)`, and title-described viewport clipping for pre-transformed `XYZRHW` vertices on `IDirect3DDevice3`.
- **DirectInput 2:** Extends `IDirectInputDeviceA` vtable through `IDirectInputDevice2A` slots 18–26 (`CreateEffect`..`SendDeviceData`) with `Poll` at slot 25 (`+0x64`) returning `DI_OK`, classified cleanly in `machine::tests::NOT_POLLS`.
- **Guest command-line arguments:** Adds `--guest-args <args>` on `Cli` threaded into `State::guest_args` and `GetCommandLineA`/`W` (unchanged when empty), snapshotted in `LLSNAP v2` alongside `environment`, `window_extra`, and `mm_timers`.

LEGO Racers joins the roster as its sixth entry (`roster::LEGO_RACERS` at `ROSTER[5]`, #369):

- **`core/src/roster.rs` & `core/src/testkit.rs`:** Declares `roster::LEGO_RACERS` (`key: "lego-racers"`, `pressing: "masterpiece-racers"`, `path: "LEGORacers.exe"`, `Kept::Cabinet`, `red_book: None`, `bank: &[]`) and `testkit::LEGO_RACERS_WRAPPED` pointing at the SafeDisc v1 wrapped 1999 `racers` pressing, with cabinet SafeDisc refusal coverage in `core/tests/roster.rs`.
- **`core/src/control/meta.rs`:** Adds `"lego-racers"` to `forbidden_keys` in the Golden Rule 4 guard test so `core/` control-plane code remains keyed to no title.

Control API ergonomics (#352) strictly outside guest state (`core/src/control/{bus,http,json,stdio,timeline}.rs`):

- **Compact `POST /api/agent-state` ack:** `Command::SetAgentState` now returns `ResponseData::None` (avoiding cloning the 200-entry `action_feed` on every patch), and `POST /api/agent-state` returns a compact JSON ack (`{"ok":true,"flip":...,"mode":...,"owner":...,"paused":...}`) without `action_feed` bloat while `GET /api/agent-state` stays unchanged.
- **Raw `mouse_down` & `mouse_up` actions:** Adds `Action::MouseDown` and `Action::MouseUp` (with optional `x`/`y` move-before-event respecting `#327` relative/absolute `MouseMode` and `button`: `"left"` | `"right"`, default `"left"`) across `Timeline`, `POST /api/act`, and `stdio` JSONL, integrating cleanly with `#346` `held_buttons` and `clear-queue` cleanup.

Control timeline chord prefix lead stagger (#348) strictly outside guest state (`core/src/control/{timeline,json}.rs`):

- **Default 1-flip prefix lead/trail on multi-key `Action::Chord`:** Adds `lead_flips` (default `1` via JSON/CLI/MCP; `0` to opt out) so earlier prefix keys assert at `start_flip`, the final key asserts at `start_flip + lead_flips` and releases at `+ hold_flips`, and prefix keys release in reverse order `lead_flips` later (omitted for single-key or deduplicated-to-1 chords), advances `start_flip` past `last_polled_flip` so mid-frame `Live`-mode chords never collapse onto one flip (while preserving static poll-loop delivery for `lead_flips: 0` with `hold_ms`), arms `hold_ms` release windows at the final key's press flip (`Due::AfterFlipOrMs` with `min_poll` guard), and rejects negative/non-integer `lead_flips`.
Piped route flip addressing & queue clearing (#346) strictly outside guest state (`core/src/control/{timeline,bus,host,http,stdio,snapshot}.rs`):

- **Unambiguous `pipe` flip addressing (`+N` relative vs. absolute default):** `Timeline::pipe_route` now supports explicit `flip +N` offsets (always relative to `current_flip`, even in absolute mode) while `POST /api/pipe` and `stdio` `pipe_route` default bare `flip N` lines to absolute flip numbers matching `#lego-legacy-route 1` files and `export-route` (`?relative=1` / `"relative": true` preserves bare-number relative offsets). `escape_quits` guarding uses the same parser so `flip +N key Escape down` is guarded identically.
- **`POST /api/clear-queue` & held-input cleanup:** Adds `POST /api/clear-queue` (Bearer-protected) returning `"cleared": <N>` via `ResponseData::Cleared`, clears any runaway `Step`-mode `step_budget` and active waits (waking blocked `dispatch_sync` waiters) without pausing running `Live` emulation, and tracks held mouse buttons alongside `held_keys` so dropping queued `Key up` / `Button up` events stages clean immediate releases on the next `poll_input` (recorded into `machine.recording`) instead of leaving inputs stuck held.
Blank OS cursor (`SetCursor(0)`) overlay suppression (#328) strictly outside guest state (`core/src/host/mod.rs`, `core/src/control/{timeline,bus,host,snapshot,observe,json,ws,spectator,watch.html}`), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- **Track blank cursor shapes (`Cursor::is_blank`):** `ControlHost::pointer_shape` (and `sync_after_live_restore` / `reset_for_rewind`) records whether the active OS cursor shape is completely transparent (`Cursor::none()` from `SetCursor(NULL)`), exposing `cursor_blank` on `FrameSnapshot`, `ControlStatus`, `/api/status`, `/api/frame.json`, and byte 13 of the 14-byte `/api/stream` WS v2 header (`POINTER_FLAG_BLANK = 0x02`, emitted only when `pointer_shown == true` for backwards-compatible `POINTER_FLAG_SHOWN = 0x01`).
- **Suppress `--cursor` and `/watch` `#reticle` on blank OS cursors:** `observe::render_frame_png` and `watch.html` suppress the crosshair reticle when `pointer_shown && cursor_blank` (hiding overlays during blank-cursor intro/cutscenes like LEGO Island's `SetCursor(0)` while leaving software-cursor titles with `!pointer_shown` unaffected).

`/watch` spectator & `action_feed` polish (#350) strictly outside guest state (`core/src/control/{bus,host,http,observe,snapshot,spectator,timeline,watch.html}`), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- **Human Take Over `action_feed` source labeling:** `ControlBus` labels `Command::Act`/`Command::Batch` (now atomic on partial failure) feed entries as `"human"` whenever `owner == InputOwner::Human` (and `"agent"` when `owner == InputOwner::Agent`), adds an optional `"source": "human" | "agent"` override on `POST /api/act`, and updates `watch.html` Take Over actions to send `source: 'human'` with `.src-human` styling and anchored non-duplicating click ripples.
- **Clean `action_feed` summaries:** Replaces Rust `Debug` `{:?}` formatting across `Action::summary()` (`Click`, `ClickPoll`, `Drag`, `Move`, `MouseDelta`, `KeyHold`, `Chord`) and `Command::Wait` with human-readable strings (e.g. `click Left (508, 377) hold=2`, `click Right in-place hold=3`, `wait 10f`), eliminating all `Some(...)` / `None` leaks.
- **Suppress false `stuck_loop` on static menu idle & Take Over:** Tracks `last_agent_action_flip` and `agent_timeline_end_flip` on `BusState` (cleared on `ClearQueue`, `SetOwner(Human)`, rewind, and live restore) and evaluates `BusState::is_agent_driving()` under a single lock in `/api/frame.json` and `/api/events` (with `active_driving` + `uniform` in `SseFingerprint`) so `stuck_loop` fires only when the agent has queued agent work or recently acted at/after the unchanged streak began (never on static menu idle in `live 1.0x` or Human Take Over).
- **Collapsible Controls & Goals panel in `/watch`:** Adds a collapsible `Controls & Goals` drawer (`#refDrawer`) in `watch.html` backed by `GET /api/controls` and `GET /api/goals` (rendering mouse mode/skip/escape chips, keybindings, hazards, progression milestones, and save files via XSS-safe DOM text nodes with retry-on-partial-failure while preserving `#327`'s `.viewport` layout, `fitViewport()`, and Take Over behavior).

Control timeline click settle gap (#344) strictly outside guest state (`core/src/control/{timeline,json}.rs`):

- **Default 1-flip settle between `Move` and `Down` on `Action::Click`:** Adds `settle_flips` (default `1`; `0` to opt out) so coordinate clicks emit `Move` (or complete relative move/delta chunking) on flip `M`, `Down` on flip `M + settle_flips`, and `Up` on `M + settle_flips + hold_flips` (preventing same-flip hover/button-down races in menu hit-testing), while omitting the settle gap for in-place clicks without coordinates, arming `hold_ms` release windows at `down_flip` (preserving `#327`'s `min_flip` guard) so `Up` never fires before `Down`, rejecting non-integer `settle_flips` or `settle_flips` on `click_poll`, and keeping `click_poll` semantics unchanged.

DirectDraw surface reclamation on final `Release` (`core/src/win32/{mod,ddraw,d3d,amstream}.rs`, `core/src/com.rs`, `core/src/control/inspect.rs`, #338) fixes the guest-heap pixel-buffer and COM leak that hit the 512 MiB ceiling (`0x40000000`) during extended LEGO Chess matches:

- **Segregated surface & COM-face reuse across all faces:** `State::com_ref` reclaims a `ComState::Surface` once every interface face sharing its `identity` (`IDirectDrawSurface`, `IDirectDrawSurface2/3/4`, `IDirect3DTexture2`, `IUnknown`, and textures released by `IDirect3DDevice::SetTexture` or `Device3D::Release`) reaches refcount `0` and no live flipping-chain parent, `Device3D`, `RetainedDevice`, or `StreamSample` holds it — returning DirectDraw-owned pixel buffers to a size-keyed `State::surface_pool` and 8-byte COM face slots to `State::free_com` (segregated from `State::heap`'s general free list so small `malloc`/COM allocations never hijack 8 KiB/32 KiB surface buffers or leak old pixels into uninitialized guest structs), removing stale `com`, `faces`, `surface_dcs`, `immediate.handles`, `color_keys`, and `clipped` entries, and snapshotting both pools deterministically in `LLSNAP v2`.
- **`SetSurfaceDesc` client-memory ownership:** `ComState::Surface` tracks `owned_pixels: bool` (included in `LLSNAP v2` `ComState::snap`). `IDirectDrawSurface3::SetSurfaceDesc` returns DirectDraw's prior pixel allocation to `surface_pool` when replaced by a guest pointer and marks `owned_pixels = false` so client-owned memory is never pooled or freed on `Release`.
- **True-to-Windows internal surface holders:** `IDirectDraw::CreateSurface` (`back_count > 0`) and `IDirectDrawMediaStream::CreateSample(NULL)` initialize implicitly owned surfaces with `refs = 0`; `GetAttachedSurface` and `IDirect3DDevice::GetRenderTarget` add a COM reference; `SetRenderTarget` and final `Release` on `Device3D`, `RetainedDevice`, and `StreamSample` drain bound textures and retry reclamation of guarded surfaces.

Control timeline `type_text` first-character hold fix (#345) strictly outside guest state (`core/src/control/{timeline,host,bus,snapshot}.rs`):

- **Preserve non-zero first-character hold when `type_text` schedules mid-frame:** `Timeline` tracks the highest polled flip internally (updated in `Timeline::drain_due` and `ControlHost::poll_input`, cleared on rewind/live restore, and preserved across `Timeline::clear`). When `Action::TypeText` is scheduled after `start_flip` has already been polled (e.g. `Live` mode between `deliver_input(N)` and `observe_flip(N+1)`), it advances the first character's start flip to `polled + 1` so `key down` and `key up` never collapse onto the same flip (`up_flip > down_flip` for every character while preserving exact `flip_gap` spacing).

Host/agent/WebSocket pointer-to-cursor alignment across all mouse regimes (#327) strictly outside guest state (`core/src/title.rs`, `core/src/control/{timeline,bus,host,meta,json,http,snapshot,ws,spectator,watch.html}`), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- **`MouseMode::SoftwareRelative` & helpers:** Adds `MouseMode::SoftwareRelative { virtual_span, start_pos }` (keeping existing `AbsoluteKept`, `SoftwareAbsolute`, and `RelativeSteered` source-compatible), `MouseMode::is_relative()`, and `MouseMode::recenters_every_flip()`, extends `TitleGuide::relative_steered_scale()` to `SoftwareRelative`, and serializes `software_relative` / `recenter_every_flip` in `/api/meta` and `/api/controls` with zero title keys in `core/` (Golden Rule 4).
- **Split `hw_pointer_pos` vs. `visual_cursor_pos` & owed-delta chunking:** `Timeline` tracks Win32's `last_move` anchor (`hw_pointer`, initialized to `(w/2, h/2)` / `recenter`) separately from the rendered in-game cursor (`visual_cursor`, sub-pixel `f64`, initialized per `MouseMode` including clamped `start_corner` / `start_pos`). `ControlHost::put_pointer` updates only `hw_pointer` on relative modes; emitted `Input::Move`s (including `InputOwner::Human`/`Shared` and `PipeRoute`) integrate `(raw_dx / scale_x, raw_dy / scale_y)` into `visual_cursor`; `observe_flip` reconciles non-relative modes when `pointer_shown == true` (and `SoftwareAbsolute`); `Timeline::drain_due` carries clamped remainder in `owed_delta` across recenter flips for `RelativeSteered` (using `round_ties_even()` to prevent `±0.5` oscillation), discards edge overflow on non-recentering clamped modes, emits unclamped deltas for `SoftwareRelative`, and automatically translates `Action::Move`/`Click`/`ClickPoll`/`Drag` on relative modes. Exposes `visual_cursor_pos` as `ControlStatus::pointer_pos` alongside additive `hw_pointer_pos` and attaches frame-locked `(pointer_pos, pointer_shown)` to `FrameSnapshot`.
- **Frame-locked WS v2 frames & `/watch` fixes:** Extends `/api/stream` binary frames with `FRAME_FLAG_HAS_POINTER = 0x80` (`FRAME_HEADER_V2_LEN = 14`: `i16 LE ptr_x`, `i16 LE ptr_y`, `u8 ptr_shown`) and `decode_binary_frame` (backwards-compatible with v1). Updates `watch.html` to commit frame-locked WS pointer coordinates atomically with canvas pixels (with generation-guarded async PNG decode and clean `wsPointerLocked` reset on close/fallback), fixes `.viewport` `box-sizing: content-box` + `fitViewport()` `ResizeObserver` aspect-ratio sizing and `Math.floor` `stageCoords`, and loads `/api/meta` on boot so Human Take Over dispatches `mouse_delta` + in-place/`Timeline`-sequenced clicks on relative titles and `click_poll` on `requires_poll_release: true` titles.

Telemetry accuracy fixes (#349) strictly outside guest state (`core/src/host/{mod,pace,telemetry,mac,linux,windows}.rs`, `core/src/control/{host,snapshot}.rs`, `core/src/run.rs`):

- **Exclude intentional `Pace` sleep from `over_budget_flips` / `over_budget_pct`:** `Pace` tracks cumulative wall nanoseconds spent in intentional pacing sleeps (`Pace::slept_total_nanos`, exposed via `Host::pace_sleep_nanos()` across `MacWindow`, `LinuxWindow`, `WinWindow`, `TelemetryHost`, and `ControlHost`), and `Telemetry::metrics` compares per-flip compute time (`wall_delta_nanos.saturating_sub(pace_sleep_delta_nanos)`) against the declared frame budget so steady paced `Live` runs no longer report `44–49%` false over-budget flips from sleep timer jitter while true compute overruns remain counted.
- **Re-baseline `Telemetry` across snapshot restore and rewind:** Adds `Telemetry::rebaseline` (re-anchoring `last_wall_nanos`, `last_clock_nanos`, `last_executed`, and `last_pace_sleep_nanos`) and invokes it in `sync_after_live_restore` alongside `pace_epoch` and in `ControlHost::reset_for_rewind` (after `setup()` on Tier-1 rewind) so restoring a cold `LLSNAP v2` snapshot or rewinding never injects phantom `declared_nanos` or `executed_insns` deltas into cumulative `pace`, `guest_fps`, or `throughput_mips`.

Control & spectator polish (#325) strictly outside guest state (`core/src/host/{mod,mac,linux,windows,telemetry}.rs`, `core/src/control/{bus,host,snapshot,observe,http,spectator}.rs`):

- **Headless `Live` wall-clock pacing:** Adds `Host::is_paced()` (returning `true` on `MacWindow`/`LinuxWindow`/`WinWindow` and `false` on `Headless`) and embeds `host::pace::Pace` inside `ControlHost` so unpaced headless `Live` runs (`!paused && !route_active && pace > 0.0`) pace against `declared_nanos / pace` (`0.5× / 1× / 2× / 4×`) with clean wall-anchor resets across pause/resume, rewind/live restore, mode change, and pace change (zero catch-up bursts), while keeping windowed hosts double-sleep-free and `Step` (`step_uncapped: true`) plus `--route` replays 100% uncapped.
- **`stuck_loop` false-positive suppression:** Raises `STUCK_LOOP_THRESHOLD` from `30` to `180` flips (3+ seconds at 60 FPS) so normal idle loops do not trigger false alarms, and suppresses `stuck_loop` warnings whenever `paused` or `finished` is `true` across both `/api/frame.json` and `/api/events`.
- **`HEAD /watch` support:** `SpectatorRoutes` handles `HEAD` on `/`, `/watch`, and `/watch/`, returning `200 OK` with `Content-Type: text/html; charset=utf-8`, `Content-Length`, and an empty body.

Full-machine live snapshots and instant restore (`LLSNAP\x02\x00`, pure `std`, zero external crates, #313) replace the minimal v1 snapshot format across `core/src/{snap,machine,clock,sound,synth,shim,loader,route,spin,disc,iv50,msv1,install}.rs`, `core/src/cpu/{mod,x87,interp/mod}.rs`, `core/src/guest/{mod,heap}.rs`, `core/src/win32/*.rs`, and `core/src/control/{snapshot,mod,bus,host,http}.rs`, enabling sub-50 ms bit-exact save-state and rewind across all five retail titles without changing any run's cursor, flips, declared milliseconds, or `state_digest()`:

- **Exhaustive compile-time `Snap` serialization (`LLSNAP v2`):** Every `Snap` implementation across `Cpu`, `X87` (full `regs`, `top`, `used`, `control`, `status`), `Clock`, `Heap`, `Arena` metadata, `Mixer`, `Synth` (preserving immutable `GM16.DLS` bank), `Boundary`, `Modules`, `Route`, `Spins`, `RedBook`, `iv50::Decoder`, `msv1::Picture`, `Tree` wall state, `win32::State` (threads, handles, objects, windows, queues, timers, critical sections, COM objects, DirectDraw surfaces/palettes/clippers, Direct3D/D3DRM devices/scenes/frames/meshes/lights/materials/textures, DirectSound buffers, DirectInput queues, MCI/CD/MIDI, AVI streams), and `Machine` exhaustively destructures every field with zero `..` rest patterns so adding any future state field fails to compile until snapshotted.
- **Sparse 4 KiB arena pages + zero-copy read-only archive/AVI rehydration + profile overlay rollback:** `Machine::snapshot_v2` captures only non-zero 4 KiB pages across live guest-accessible regions (avoiding faulting untouched anonymous pages of the 2 GiB arena), defers large immutable read-only `Contents::Held` open handles (`>= 64 KiB`, non-profile), `CreateFileMappingA` disc archives (`State::mapped_files`), and `AVIFile` byte caches (`State::movie_bytes`) for zero-copy in-place swap on live restore and `Tree` rehydration on cold restore, and snapshots `<data-root>/profile/` (validated against path traversal) so on-disk save files roll back bit-for-bit. `Machine::restore_v2` / `restore_v2_live` zeroes post-snapshot dirty pages in a single linear merge pass, calls `engine.forget()` + `arena.clear_code()` + `bound.clear()`, re-syncs `next_mix_cycle`/`next_route_at`/`next_snapshot`, and enforces provenance (`build`, `title`, and strict `route` on `restore_v2`) with `SnapError::Mismatch`.
- **HTTP control plane (`POST /api/snapshot`, `GET /api/snapshot`, `POST /api/restore`) & instant `/api/rewind`:** Adds named in-memory snapshot slots and disk file save/restore (`Bearer`-authenticated on mutating `POST`s), upgrades `POST /api/bookmark` and `POST /api/rewind` to instant live `LLSNAP v2` in-place restore (`mode: "llsnap_v2"`, retaining Tier-1 route replay as fallback), and re-anchors `ControlHost::pace` on backward `declared_nanos` jumps.
- **Verified** via expanded `core/tests/snapshots.rs` (11/11: full x87 `top`/`status`/`-0.0`/NaN bits, dirty-page zeroing & decoded fast-block cache invalidation, profile overlay rollback & deferred read-only file live/cold rehydration, v1/truncation/route-mismatch/path-traversal refusal), `core/src/control/http.rs` end-to-end TCP/Unix tests (auth, slot + file snapshot/restore, profile rollback, instant `llsnap_v2` rewind), `scripts/check-targets.sh` (all 6 desktop targets), native `aarch64` & Rosetta 2 `x86_64-apple-darwin` core lib tests, full `CLAUDE.md` release PR gate (SingleStepTests 80386 corpus, `versions`, `zero_crates`, four released titles' non-disc suites), and bit-exact mid-gameplay live (`B == A`) + cold fresh-process (`C == A`) resume parity across all 5 retail titles (identical `cursor`, `flips`, `declared_ms`, `state_digest()`, `frame_hash`, PNG SHA-256, and DirectSound voice state):
  - `lego-chess` (flip 600 → 950): `38.37 MiB`, capture `16.32 ms`, live restore `4.72 ms`, cold restore `40.67 ms`, instant `/api/rewind` `10.10 ms`; plus `save-game.route` profile save rollback bit-exact (`18.15 ms`).
  - `hotwheels-stunt-track-driver` (flip 1100 → 1300): `11.88 MiB`, capture `14.45 ms`, live restore `1.79 ms`, cold restore `13.57 ms`.
  - `lego-island` (flip 8100 → 8300): `20.15 MiB`, capture `20.51 ms`, live restore `6.27 ms`, cold restore `22.14 ms`.
  - `lego-rock-raiders` (flip 5450 → 5600): `210.28 MiB`, capture `80.51 ms`, live restore `39.05 ms`, cold restore `52.22 ms`.
  - `lego-alpha-team` (flip 3100 → 3250): `33.11 MiB`, capture `18.74 ms`, live restore `8.00 ms`, cold restore `31.91 ms`.

First-class static per-title playability metadata (`TitleGuide`, `MouseMode`, `AgentCadence`, `ControlHint`, `Goal`, `Milestone`) is added strictly outside guest state (`core/src/{title,run}.rs`, `core/src/control/{mod,meta,bus,host,http}.rs`, ADR-0001, ADR-0003, #311), with zero title names hardcoded in `core/` (Golden Rule 4) and every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `title` defines `const`-constructible `TitleGuide` (with `TitleGuide::EMPTY` fallback and `relative_steered_scale()`), `MouseMode` (`AbsoluteKept`, `SoftwareAbsolute { requires_poll_release }`, `RelativeSteered { virtual_span, recenter, start_corner }`), `AgentCadence` (`PauseToThink`, `BurstThenPause`, `MicroStep`), `ControlHint`, `Goal`, and `Milestone`.
- `run` adds additive `main_with_guide`, `run_with_guide`, and `run_digested_with_guide` while keeping `run::main`, `run::run`, and `run::run_digested` 100% source-compatible (defaulting to `&TitleGuide::EMPTY`) so `core` stays `1.2.0` cleanly under ADR-0007.
- `control::meta` serializes `&TitleGuide` to zero-crate RFC 8259 JSON for `GET /api/meta`, `GET /api/controls`, and `GET /api/goals`. `ControlBus` / `ControlHost` / `HttpContext` carry `&'static TitleGuide`, automatically enforce the `escape_quits` safety guard across `Act`, atomic `Batch`, and `PipeRoute` (blocking DirectInput `Escape` / `0x01` unless `force: true` is passed via `dispatch_forced` / `POST /api/act` / `POST /api/pipe`), and automatically apply `MouseMode::RelativeSteered` scaling hints and `recenter` origin to `Action::MouseDelta` unless explicit `scale_x`/`scale_y` overrides are supplied.
- ADR-0001 and ADR-0003 add short clarifying notes covering composable `Host` decorators (`TelemetryHost<H>`, `ControlHost<H>`) and pause-compensated `Pace`/`Waiting` windows.

Zero-crate Twitch-Plays live spectator UI (`/watch`, `/`), RFC 6455 WebSocket binary/delta video stream (`/api/stream` + `multipart/x-mixed-replace` PNG fallback on `/api/stream` and `/api/stream.mjpeg`), and Server-Sent Events HUD stream (`/api/events`) are added strictly outside guest state (`core/src/control/{ws,spectator,watch.html,http,mod}.rs`, ADR-0001, #310), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `control::ws` implements pure-`std` integer-only RFC 3174 SHA-1 + Base64 `Sec-WebSocket-Accept` handshake (`258EAFA5-E914-47DA-95CA-C5AB0DC85B11`), RFC 6455 unmasked server framing and masked client Close/Ping/Pong decoding, a 9-byte binary video frame envelope (`[1B flags][4B LE flip][2B LE width][2B LE height][payload]`), and a single-pass zero-allocation XOR + PackBits-RLE BGRA delta codec.
- `control::spectator::SpectatorRoutes` attaches by default in `HttpContext::new` via the bidirectional `StreamIo` / `ExtraRoutes::handle_stream` seam (while preserving `.with_extra(...)` overrides and `?token=` / `Authorization: Bearer` auth): serves embedded `watch.html` at `GET /` and `GET /watch`, streams WebSocket keyframes/XOR+RLE deltas with automatic keyframe recovery (first frame, resize, scene cut, periodic interval, or oversized delta) and drop-if-busy backpressure (`latest_frame()` snapshots so slow clients never stall `ControlHost`) plus `multipart/x-mixed-replace` PNG fallback, and pushes compact JSON SSE updates on `GET /api/events` (capped at ~12.5 Hz for flip/telemetry updates with immediate push on pause/mode/owner/pace/agent-state/action-feed changes and 10 s keepalive heartbeats).
- Embedded single-file `core/src/control/watch.html` renders a zero-dependency dark-mode spectator console (4:3 `pixelated` `<canvas>` with serialized WebSocket binary decoder and automatic `/api/frame.png` polling fallback, toggleable 80×80 `A..H`/`1..6` coordinate grid, live pointer reticle + click ripples, translucent Live Input Strip, right HUD sidebar with mode pill, `#296` FPS/pace/MIPS telemetry, goal/sub-goal checklist, agent thought/commentary + stuck/uniform critic warnings, reverse-chronological action feed, Pause/Resume/Step/Pace controls, Human Takeover canvas pointer/keyboard forwarding to `POST /api/act`, Coach Hint `"Ask Gem"` input posting `user_hint` to `POST /api/agent-state`, and collapsible CPU/Win32/Saves/Disc/Route Inspect drawer with Bearer token support).
- Verified via new unit & live TCP integration tests (FIPS 180-4 SHA-1 + official RFC 6455 `Sec-WebSocket-Accept` vector, WebSocket framing & masked Ping/Close, XOR+PackBits-RLE bit-exact round-trip across 640×480 deltas and edge run lengths, keyframe selection heuristics, and live `--stub` `/watch` + `/api/events` SSE + `/api/stream` WS/Ping/Pong/XOR-delta + MJPEG fallback asserting exact `Finish` and `state_digest()` parity against unattached `Headless`); all six desktop targets (`scripts/check-targets.sh`); `cargo fmt --all -- --check` & `cargo clippy --workspace --all-targets -- -D warnings`; native `aarch64` and Rosetta 2 `x86_64-apple-darwin` core lib tests; and the full `CLAUDE.md` PR gate.

Zero-crate HTTP/1.1 & Unix domain socket control and read-only machine-boundary introspection endpoint (`--control [HOST:PORT]`, `--control-unix <path>`, `--token <secret>` / `LEGO_LEGACY_TOKEN`), host-side visual grounding overlays, Tier-1 route checkpoints, and RFC 1951 fixed-Huffman DEFLATE PNG compression are added strictly outside guest state (`core/src/control/{mod,bus,host,json,stdio,observe,inspect,http,unix}.rs`, `core/src/{png,cli,run,machine,install,sound}.rs`, `core/src/win32/{mod,d3d,dsound,winmm}.rs`, ADR-0001, ADR-0003, #309), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `png::encode` upgrades from stored DEFLATE blocks and bit-at-a-time CRC-32 to pure-`std` integer-only RFC 1951 fixed-Huffman LZ77/RLE DEFLATE with adaptive None/Sub scanline filtering (falling back to stored blocks on incompressible noise), a compile-time 256-entry CRC-32 table, and chunked Adler-32 — shrinking real 640×480 `lego-chess` screenshots from ~922 KB down to 136–285 KB (69%–85% savings; verified with Python `zlib` + IEEE CRC-32 and `crate::inflate`) while remaining 100% standard valid PNG and bit-for-bit pixel-identical.
- `control::observe` adds host-side BGRA visual grounding over `FrameSnapshot`: `GET /api/frame.png?grid=0|1&cursor=0|1&crop=x,y,w,h` (80×80 labeled `A..H`/`1..6` coordinate grid via embedded 5×7 bitmap font, high-contrast cursor crosshair, 2× nearest-neighbor crop zoom, and `X-Flip`/`X-Cursor`/`X-Frame-Hash`/`X-Diff-Pct` headers) and `GET /api/frame.json?grid=16x12` (downsampled integer RGB grid, diff metrics, and `uniform_frame`/`stuck_loop` warnings).
- `control::inspect` adds 100% read-only `&Machine` flip-boundary inspection (served deadlock-free while paused in `Step` mode via `Machine::set_flip_observer` and `service_flip_boundary`, without touching `Machine::step`/`step_fast` or mutating live ISO/guest state): `GET /api/mem` (`hex|raw|u8|u16|u32|i32|f32|cstr`, 1 MiB cap), `GET /api/mem/map`, `POST /api/mem/search` (`bytes_hex|u32|ascii`), `GET /api/cpu` (GPRs, EIP, EFLAGS, x87 ST(0..7)/top/used/control/status, cooperative threads, declared clock, `state_digest`), `GET /api/win32` (window/messages/timers, cursor/input, DirectDraw surfaces/palettes, Direct3D devices, DirectSound active voices, MIDI/CD audio, open handles/files), `GET /api/files` & `GET /api/files/read` (`root=disc|profile`, path-traversal-safe with SHA-256 on profile saves), and `GET /api/route`.
- `control::http` & `control::unix` (`#[cfg(unix)]`) share one zero-crate HTTP/1.1 router over `TcpListener` (default `127.0.0.1:0` or explicit `HOST:PORT`) and `UnixListener`, with optional constant-time Bearer token auth on mutating `POST` routes, `~/.local/state/lego-legacy/sessions/<pid>.json` session discovery (cleaned up on drop), baseline `/api/meta`/`/api/controls`/`/api/goals` JSON stubs, an `ExtraRoutes` streaming seam for `#310`/`#311`, unified control & piping (`/api/act` with `observe_after` base64 PNG, `/api/pipe` for raw routes or JSONL, `/api/step`, `/api/wait`, `/api/mode`, `/api/agent-state`, `/api/fps`, `/api/status`), and Tier-1 route checkpoints (`POST /api/bookmark`, `POST /api/rewind` rebuilding `Machine` cleanly at `run.rs`'s outer loop and fast-forward replaying to the exact flip while verifying `state_digest()`).
- Verified via new unit & integration tests (fixed-Huffman PNG round-trip & 640×480 compression, visual grounding overlays, read-only inspect & path-traversal refusal, live TCP + Unix socket end-to-end across all endpoints + auth + bookmark/rewind `digest_match`, `ExtraRoutes` & JSONL pipe, CLI flags, and bit-exact determinism comparing polled vs unpolled runs); all six desktop targets (`scripts/check-targets.sh`); native `aarch64` and Rosetta 2 `x86_64-apple-darwin` core lib tests (353 passed); and the full `CLAUDE.md` PR gate (including SingleStepTests 80386 corpus, `lego-chess` 33/33, `versions`, `zero_crates`, and the four released titles' non-disc suites).

Branchless effective-address calculation (`NO_REG = 8`), exact `[code_min, code_max)` code-page bounding-box store filtering, `x86_64` `pushfq`-free flag capture, and a conservative intra-block dead-flag `*Nf` pass are landed in `run_fast_block` across `aarch64` and `x86_64` (SysV + Win64, ADR-0009, #283), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- **Branchless EA with `NO_REG = 8` (`ZERO_REG = 8`) & base-only `Mov` specialization:** `insn::NO_REG` moves from `0xff` to `8` and `FLAG_FS` moves to `0x80` (so `(shift | FLAG_FS) & 31 == shift` needs zero masking on both `aarch64` `lsl` and `x86_64` `shl`, while `FS:` tests a single sign/bit check). `Interp::run_native` stages `Cpu::r` in a 9-element `fast_regs = [0u32; 9]` window with constant-zero `fast_regs[8] == 0` (asserted on exit, keeping `Cpu::r: [u32; 8]` and `Machine::state_digest()` untouched), turning flat32 EA across `aarch64.rs` and `x86_64.rs` into branchless `disp + r[base] + (r[index] << scale)` (+ cold `FS:` add). `compile_fast` specializes non-`FS:` `[base + disp]`/`[base]`/`[disp]` loads and stores to `MovRegMemBase`/`MovMemRegBase` (skipping index, scale, and `FS:` work completely).
- **Exact `[code_min, code_max)` code-page bounding box:** `guest::Arena` tracks `[code_min, code_max)` across `mark_code`, `unmark_code` (tightening on boundary unmark), and `clear_code` (reset to `(u32::MAX, 0)` on `written_all` / cache clear), short-circuiting `wrote_over_code` and publishing `code_bbox_start = code_min.saturating_sub(7)` / `code_bbox_len` in `FastCtx` (`size_of::<FastCtx>() == 184`). Inside `run_fast_block` (`x21` on `aarch64`, `r9d` + `[rbp + 180]` on `x86_64`), every store skips `code_bitmap` loads and page-crossing fallbacks with a 2-op range check when outside `[code_min.saturating_sub(7), code_max)` and routes any store overlapping `[code_min, code_max)` (including 2/4/8-byte stores straddling `code_min - 1`..`code_min - 7` or `code_max - 1`) to out-of-line exact page-cross + `code_bitmap` checks.
- **`x86_64` `pushfq`-free flag capture:** Eliminates all 105 microcoded `pushfq; pop r15` sequences in `core/src/cpu/interp/x86_64.rs` (SysV + Win64) by packing `r15d` directly via register-only `lahf` + `seto al` (`OF=0, CF=8, PF=10, AF=12, ZF=14, SF=15`, mask `0xd501`), with `and eax, 0xc400` for logic ops, `CF` preservation on `Inc`/`Dec`, and `bt r15d, 8` for `Adc`/`Sbb`/`Sbb8`.
- **Conservative intra-block dead-flag `*Nf` pass:** `Interp::run_at` promotes register-only 32-bit ALU/shift ops (`Add`/`Sub`/`Xor`/`And`/`Or` reg/imm, `Shl`/`Shr`/`Sar` imm) to `*Nf` (`FastOp` `133..=145`) only when an unconditional all-6-flag overwriter occurs later in the same run with zero intervening flag readers, partial-flag writers, branches, `Slow`/`X87` ops, or potentially faulting memory ops. `exec::execute_straight` still computes exact flags for `*Nf`, keeping `CpuEngine::step()`, `LEGO_LEGACY_ARCH=rust`, and mid-run `STATUS_BUDGET_SHORT`/`STATUS_OUTSIDE_OVERLAP` stops 100% flag-exact. `FastInsn` stays 16 B `#[repr(C)]` with zero `x18`/`w18`, zero `pushfq`/`popfq`, zero callee-saved FP spills, and zero outgoing calls.
- Verified via new differential unit and integration tests covering `NO_REG = 8` / SIB / `FS:` / `Mov*Base` / `r[8] == 0`, `[code_min, code_max)` fast-path + straddling 1/2/4/8-byte SMC boundaries, all 64 flag combinations + zero-`pushfq` static check, and conservative `*Nf` exactness across every `step()` and mid-run budget on BOTH native `aarch64-apple-darwin` (321/321) and Rosetta 2 `x86_64-apple-darwin` (321/321); `scripts/check-targets.sh` across all six desktop targets; `core/tests/arch_determinism.rs`; all ≥877 SingleStepTests 80386 conformance vectors; `core/tests/versions.rs`; `core/tests/interp.rs` (`bench_flat32_tight_loop`: `1507.8 MIPS` native `aarch64` vs `567.6 MIPS` `rust`, `593.6 MIPS` Rosetta 2 `x86_64` vs `332.7 MIPS` `rust`); full `CLAUDE.md` PR gate; and Layer-2 headless parity (`native` vs `LEGO_LEGACY_ARCH=rust` vs `b02e419`) across all five titles with exact `cursor`, `flips`, `declared_ms`, and `state_digest()`:
  - `hotwheels-stunt-track-driver` (`first-race.route`): `25.88 s` wall / `25.56 s` user (`520.2 MIPS`, `4.67×` pace; `-35.6%` vs `40.19 s` `rust`, `-1.0%` vs `b02e419`'s `26.15 s`).
  - `lego-chess` (`checkmate.route`): `25.71 s` wall / `27.68 s` user (`454.5 MIPS`, `4.24×` pace; `-33.1%` vs `38.44 s` `rust`, `-1.8%` vs #280's `26.18 s`).
  - `lego-island` (`pizza-delivery.route`): `27.41 s` wall / `28.67 s` user (`388.8 MIPS`, `22.09×` pace; `-30.1%` vs `39.20 s` `rust`).
  - `lego-rock-raiders` (`first-mission.route`, close @ 9050): `38.85 s` wall / `38.23 s` user (`366.9 MIPS`, `9.65×` pace; `-37.0%` vs `61.69 s` `rust`).
  - `lego-alpha-team` (`first-mission.route`, close @ 7990): `71.80 s` wall / `74.67 s` user (`323.5 MIPS`, `4.59×` pace; `-31.9%` vs `105.42 s` `rust`).

Composable `ControlHost<H: Host>` and `ControlBus` unify human play, live agent control, and piped TAS/route tasks over any `Host` (`Headless`, `MacWindow`, `LinuxWindow`, or `WinWindow`) strictly outside guest state (`core/src/control/{mod,bus,host,timeline,stdio}.rs`, `core/src/{lib,cli,run}.rs`, ADR-0001, ADR-0003, #308), leaving every non-control run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `control::host::ControlHost<H>` wraps any `Host`, embeds `#296`'s pause-compensated `Telemetry`, snapshots dense BGRA frames (`Arc<Vec<u8>>`) with single-pass `FrameDiff` (`diff_pixels`, `diff_pct`, `diff_bbox`, `unchanged_streak`, deterministic 64-bit FNV-1a `frame_hash`, and uniformity), tracks `FlipMeta`, pointer position/visibility, held keys, and `AgentState` (`goal`, `subgoals`, `thought`, `commentary`, `notepad`, `user_hint`, bounded `action_feed`).
- Dual execution modes (`ExecMode::Step` "Democracy" vs. `ExecMode::Live` "Anarchy") pause cleanly in `observe_flip` via `inner.set_paused(true)` + 16 ms `condvar.wait_timeout` `inner.pump()` ticks (keeping native windows 60 Hz-responsive with zero beachball while `Machine::clock` stays frozen), skip pausing automatically during `--route` pre-roll (`meta.route_active == true`) before handing off cleanly on the first post-route flip, and support configurable Live-mode pacing (`0.25×..4.0×`) plus uncapped Step bursts without phantom sleep jumps on mode switch. Equal-citizen `InputOwner` (`Agent` | `Human` | `Shared`) lets `Agent` ignore stray human mouse/key input (forwarding `Input::Close` and keeping windowed hosts non-hijacking `Capture::Free`), `Human` forward window input directly, and `Shared` merge both cleanly.
- `control::timeline::Timeline` compiles high-level actions (`move`, `mouse_delta` dynamically translated against live `put_pointer`/`Move` origin with `Mouse::Steered` scaling, `click` & dual flip+declared-ms `click_poll` releasing even inside static non-flipping poll loops, `drag`, `key_hold`, `chord`, `type_text`, `wait`, `wait_until_change`, `wait_until_settled`, and relative/absolute `#lego-legacy-route 1` `pipe_route`) into deterministic `Host::Input` events automatically recorded into `machine.recording`.
- `control::stdio::StdioBridge` (`--control-stdio`, `--control-mode <step|live>`, `--paused`) provides a zero-crate newline-delimited JSON transport over stdin/stdout (routing run reports and `--fps-report` lines to stderr while active) backed by extensible `ControlBus::dispatch` / `ControlBus::dispatch_sync` ready for `#309`.
- Verified via unit tests covering `mouse_delta` (`Steered` translation), dual flip+ms `click_poll` releases on static non-flipping poll loops, `chord`, `drag`, `type_text`, `owner` switching (`Agent`/`Human`/`Shared`), `pipe_route`, `FrameDiff`, `pace` scaling/resync, `Step` 60 Hz pump/release, CLI flags, `stdio` JSONL parsing, and bit-exact deterministic equivalence (driving `--stub` and a multi-flip stub via `ControlHost<Headless>`/`ControlBus`/`StdioBridge`, exporting `machine.recording`, and replaying on bare `Headless` with identical `cursor`, `flips`, `declared_ms`, and `state_digest()`); all six desktop targets (`scripts/check-targets.sh`); native `aarch64` and Rosetta 2 `x86_64-apple-darwin` core lib tests (343 passed); and the full `CLAUDE.md` PR gate (including SingleStepTests 80386 conformance corpus, `lego-chess`, `versions.rs`, `zero_crates.rs`, and the four released titles' non-disc suites).

Host flip/pause hooks, pause-safe pacing and audio playback, and dev-only `--fps-report` / `--fps-window` host telemetry are added strictly outside guest state (`core/src/host/{mod,pace,feed,mac,mac_audio,linux,linux_audio,windows,windows_audio,telemetry}.rs`, `core/src/{machine,cli,run}.rs`, ADR-0001, ADR-0003, #296), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `host::FlipMeta` and three default no-op methods on `host::Host` (`pump`, `set_paused`, `observe_flip`) plus a blanket `Host` impl for `&mut H` provide the shared seam for composable host wrappers and pause-safe live control. `Machine::present_guest` (and `Machine::present` for `--stub`) invokes `host.observe_flip` once per flip after audio mixing and immediately before input delivery, leaving `step_fast` untouched.
- `host::pace::Pace` tracks pause windows (`paused_at`, `paused_total_nanos`) with idempotent `set_paused(bool)`, signed alignment `resync(declared_nanos)`, and optional uncapped fast-burst mode (`set_uncapped(bool)`) so pausing for host think time or bursting uncapped never warps audio `hold_mixed()` on resume. `host::feed::Waiting` adds `set_paused(bool)` so `fill_frames` emits silence without consuming queued samples or counting phantom underruns while paused; `MacAudio` (`AudioQueuePause`/`AudioQueueStart`), `WinAudio` (`waveOutPause`/`waveOutRestart`), `LinuxAudio` (paused `Waiting` + `-EPIPE` guard), and `MacWindow`/`LinuxWindow`/`WinWindow` wire `Host::pump` and `Host::set_paused` across all six desktop targets.
- `host::telemetry::{Telemetry, TelemetryHost}` implements dev-only `--fps-report` and optional `--fps-window <start>..<end>` in `cli::Cli` and `run::run_digested`, measuring all five `CONTEXT.md` speed quantities (throughput MIPS excluding passed clock spins, pace ratio, host FPS, guest presentation rate, and declared intended FPS), pause-compensated per-flip wall-time percentiles (`p50`/`p95`/`p99`/`max` ms), over-budget frame count/percentage, `>= 100 ms` frame stalls, uniform frames, and host audio underruns for both the full run and optional active window, printed to stdout and written to `RunLog`.
- Verified via new unit tests covering `Pace` pause/resync/uncapped mode, `Waiting` pause without phantom underruns, percentile math, window slicing, `TelemetryHost` pause compensation and hook forwarding, `--fps-report`/`--fps-window` CLI parsing, and bit-exact `--stub` determinism; all six desktop targets (`scripts/check-targets.sh`); native `aarch64` and Rosetta 2 `x86_64-apple-darwin` core lib tests (325 passed); the full `CLAUDE.md` PR gate (including all ≥877 SingleStepTests 80386 conformance vectors, `lego-chess` 33/33, `versions.rs`, `zero_crates.rs`, and the four released titles' non-disc suites); and headless parity on `hotwheels-stunt-track-driver` `first-race.route` without vs. with `--fps-report --fps-window 500..2500` ending on identical `cursor 13462673313`, `flips 3903`, `declared-ms 120966`, and digest `25ad6546...` (`521.4 MIPS`, `4.68×` pace, `151.1` host FPS, `32.3` guest FPS, `p50=5.72ms`, `p95=16.79ms`, `p99=18.44ms`).

Hot integer `Slow` operations are promoted into `FastOp` and 8-bit register ALU ops are split with pre-decoded byte offsets inside `run_fast_block` across `aarch64` and `x86_64` (SysV + Win64, ADR-0009, #282), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- Empirical Step-1 `execute_slow` and `Alu8` histogram instrumentation on `lego-rock-raiders` (`first-mission`, close @ 9050: 583.4M `execute_slow` calls and 1.27B `Alu8` calls) and `lego-alpha-team` (`first-mission`, close @ 7990: 387.5M `execute_slow` calls) identified the dominant integer fallback hotspots: `Sbb B8` (280.7M RR / 1.7M AT), `Leave B32` (85.3M RR / 87.6M AT), `Adc`/`Sbb B32` (6.4M RR / 134.4M AT), `Xchg B8` (70.4M RR), `Shift B8` (51.5M RR / 8.3M AT), `Extend ZX8->B16` (40.8M RR / 7.9M AT), `Not B32` (22.8M RR / 2.2M AT), `Shift B16` (21.0M AT), `Inc B16 (M)` (8.4M AT), `Neg B32`/`B8` (6.8M RR / 1.3M AT), `Cdq` (5.9M RR / 2.2M AT), and `Div`/`Idiv`/`Mul`/`Imul B32` (5.0M RR / 6.6M AT) — eliminating 98.7% of Rock Raiders' `execute_slow` calls and 97.4% of Alpha Team's non-`rdtsc` `execute_slow` calls.
- `cpu::insn::r8_off` pre-encodes 8-bit register byte offsets (`0, 4, 8, 12, 1, 5, 9, 13` for `AL..BH`) in `compile_fast()` across all 8-bit register ops (`Mov8*`, `Alu8*`, `Sbb8*`, `SetccReg`, `ExtendReg`, `Extend16Reg`, `Xchg8RegReg`, `Neg8Reg`, `Not8Reg`, `IncDec8Reg`, `Shift8Reg`), and `Alu8RegReg`/`Alu8RegImm` are split into 14 dedicated branch-free ops (`Add8`/`Sub8`/`Cmp8`/`And8`/`Or8`/`Xor8`/`Test8` × `RegReg`/`RegImm`) plus `Sbb8RegReg`/`Sbb8RegImm`, removing the 6-branch `aux` cascade from 1.24B of Rock Raiders' 1.27B 8-bit ALU ops.
- Promoted flat32 ops (`AdcReg*`/`AdcMem*`, `SbbReg*`/`SbbMem*`, `Leave`, `Sahf`, `Lahf`, `Cdq`, `Cwde`, `Neg32`/`Neg8`, `Not32`/`Not8`, `Xchg32`/`Xchg8`, `Extend16Reg`, `IncDec8`/`IncDec16` reg & mem, `Shift8Reg`, `Shift16Reg`, `Rot32Reg`, `Mul32`/`Imul32`, `Div32`/`Idiv32`, and `Cmovcc32Reg`/`Cmovcc32Mem`) preserve exact 80386 flag and fault semantics across `aarch64`, `x86_64` (SysV + Win64, zero `popfq`, using `bt r15d, 0` for `adc`/`sbb`), and pure Rust (`LEGO_LEGACY_ARCH=rust`): `Div32`/`Idiv32` bail to `STATUS_FALLBACK` on div-by-0 or overflow (`EDX >= divisor`, `i32::MIN / -1`, `i64::MIN`) BEFORE mutating `EAX`/`EDX`, `Shift*`/`Rot32` preserve all flags on `count == 0` and compute exact 80386 `CF`/`OF` for `count >= bits`, `IncDec8`/`16` preserve `CF`, `Cmovcc32Mem` skips EA/reads when the condition is false, and `FastInsn` (16 B `#[repr(C)]`) and `FastCtx` (176 B) stay unchanged with zero `x18`/`w18`, zero callee-saved FP spills, and zero outgoing calls.
- Verified via exhaustive differential unit tests covering every new `FastOp` and edge case (zero-fallback loop assertion, shift-by-0, `#DE` pre-mutation bail, all 16 `Cmovcc` conditions across all 64 flag states + lazy flags + unmapped false-cond reads, all 8-bit `AL..BH` offsets, `Inc`/`Dec` `CF` preservation, `Sahf`/`Lahf`) on both native `aarch64-apple-darwin` and Rosetta 2 `x86_64-apple-darwin`; `scripts/check-targets.sh` across all six desktop targets; `core/tests/arch_determinism.rs`; all ≥877 SingleStepTests 80386 conformance vectors; full `CLAUDE.md` PR gate; and Layer-2 headless parity (`native` vs `LEGO_LEGACY_ARCH=rust` vs `c74b5d6`) with exact `cursor`, `flips`, `declared_ms`, and `state_digest()`:
  - `lego-rock-raiders` (`first-mission.route`, close @ 9050): `36.84 s` wall / `36.32 s` user (`387.0 MIPS`, `10.18×` pace; `-17.7%` wall vs `c74b5d6`'s `44.74 s`, `-38.6%` vs `60.04 s` `rust`).
  - `lego-alpha-team` (`first-mission.route`, close @ 7990): `67.66 s` wall / `70.22 s` user (`343.3 MIPS`, `4.88×` pace; `-2.7%` wall / `-3.4%` user vs `c74b5d6`'s `69.55 s` / `72.66 s`).
  - `hotwheels-stunt-track-driver` (`first-race.route`): `26.15 s` wall / `25.47 s` user (`514.8 MIPS`, `4.63×` pace; exact `cursor`/`flips`/`declared_ms`/`digest` parity vs `c74b5d6` and `rust`).

Software rasteriser bilinear texture sampling, texel gathers, and short-span / quad fast paths are tightened across `aarch64` NEON, `x86_64` baseline SSE2, and the canonical pure-Rust oracle (`core/src/raster/span/{rust,aarch64,x86_64}.rs`, ADR-0009, #281), leaving every frame, cursor, flip count, declared millisecond, and `state_digest()` unchanged:

- Separable 16-bit integer bilinear interpolation (`bilinear16_load_pairs` + `bilinear_channel_s16`) replaces the 32-bit four-corner weight multiply-sum across `BilinearRgb565`, `BilinearArgb1555`, and `BilinearArgb4444` using exact `top = (c00 << 8) + (c10 - c00) * fx`, `res = (top << 8) + (bot - top) * fy` via NEON `vmlaq_s16`/`vmlal_s16` and SSE2 `_mm_mullo_epi16`/`_mm_madd_epi16`, pairing adjacent horizontal texels into 8 unaligned `u32` loads deinterleaved via `vuzp1q_u16`/`vuzp2q_u16` (NEON) or 16-bit mask/shift + `_mm_packus_epi16` (SSE2) whenever a quad does not straddle a horizontal wrap seam (falling back cleanly to per-corner 16-bit loads on seams). Exact IEEE-754 float RGB565 packing is preserved (`pack_rgb565_4_unclamped` is used only in `span_textured_copy_opaque` when `TexSample4::UNIT_RANGE` guarantees finite `[0.0, 1.0]` inputs, never integer-only RGB565 packing).
- Eliminated store-forwarding stack spills across `resolve_xy4`, `gather_u16_4`, `BilinearPaletted8`, and `zv` construction by using direct SIMD↔GPR lane extraction (`vgetq_lane_*` / `_mm_cvtsi128_si32` + `_mm_srli_si128`) and lane insertion (`vset_lane_u16` / `vsetq_lane_*` / `_mm_insert_epi16`), verified in release AArch64 disassembly with zero stack spills and zero calls inside the hot quad loop.
- Added early `if n < 4` scalar exits outside `#[inline(never)]` SIMD loop bodies (`*_neon` / `*_sse2`) paired with `#[inline(always)]` `rust::span_*` `0..3` pixel tails so narrow spans bypass callee-saved SIMD register prologues, added a transparent-alpha quad skip (`!ALPHA_GT && !depth_write && all sa == 0.0` with finite-RGB guard matching exact scalar `NaN`/`±inf` clamping) across `span_untextured_alpha` and `span_textured_modulate_blend`, and added all-pass opaque Z direct stores (`vminvq_u32` / `z_bits == 0xf`) skipping framebuffer read-modify-write blends when all 4 lanes pass depth testing. All kernels honor `LEGO_LEGACY_ARCH=rust` with zero FMA, zero `algebraic_*`, and zero approximate `rcp`/`rsqrt`.
- Verified via new Layer-1 differential oracle unit tests on both native `aarch64` and Rosetta 2 `x86_64-apple-darwin` (exhaustive RGB565 round-trip, separable `s16` bilinear & unclamped pack parity across corner/weight grids, wrap seams, `1×1` textures, unaligned slices, `NaN`/`±inf` UV, transparent-alpha finite/non-finite RGB guards, all-pass/partial Z masks, widths `0..=65` and odd offsets `0..=7`), `core/tests/arch_determinism.rs`, all six desktop targets (`scripts/check-targets.sh`), the full `CLAUDE.md` PR gate (including all ≥877 SingleStepTests 80386 conformance vectors), and interleaved Layer-2 headless parity (`native` vs `LEGO_LEGACY_ARCH=rust` vs `3b1aed2` baseline, exact `cursor`, `flips`, `declared_ms`, and `state_digest()`): `lego-alpha-team` `first-mission.route` (close @ 7990) 79.78 s → 79.04 s wall (95.91 s `rust`), `lego-rock-raiders` `first-mission.route` (close @ 9050) 47.59 s → 47.22 s wall (58.91 s `rust`), and `lego-chess` `checkmate.route` 31.71 s → 31.80 s wall (34.77 s `rust`).

Hot scalar IEEE-754 x87 operations are inlined directly inside `run_fast_block` across `aarch64` and `x86_64` (SysV + Win64, ADR-0009 Rung 4 clarification, #280), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- ADR-0009 Rung 4 clarifies that `run_fast_block` may use strictly caller-saved volatile scalar IEEE-754 FP registers (`d0..d3` on `aarch64`, `xmm0..xmm3` on `x86_64`) with zero callee-saved FP spills (Win64 `xmm6..xmm15` and AAPCS64 `d8..d15` remain 100% untouched), zero FMA contraction, zero approximate `rcp`/`rsqrt`, zero `FPCR`/`FPSR`/`MXCSR` mutation, and zero outgoing calls.
- `FastCtx` carries `fpu: *mut X87` (`#[repr(C)]` 72-byte layout from #272, pinned via `offset_of!` assertions at offset 168, pinned in leaf-free `x30` on `aarch64` and loaded into `rdi` on `x86_64`). `FastOp::X87Reg` (`0..=58`) and `FastOp::X87Mem` (`59..=111`, computing flat32 `ea` once before secondary PC-relative jump-table dispatch) inline `Fnop`, `Fld` (`reg`/`m32`/`m64`/`0`/`1`), `Fst`/`Fstp` (`reg`/`m32`/`m64`), `Fild` (`m16`/`m32`), `Fadd`/`Fmul`/`Fsub`/`Fsubr`/`Fdiv`/`Fdivr` (`st0`/`sti`/`pop`/`m32`/`m64`), `Fsqrt`, `Fchs`/`Fabs`, `Fxch`, `Fcom*`/`Fucom*`/`Fcomi*`/`Fucomi*`/`Ftst`, `FnstswAx`, `Fldcw`/`Fnstcw` (keeping MSVC `_ftol` in-block), and `Fistp32`/`Fistp64` for `RC == 3` (truncate toward zero in `[-2^31, 2^31)` / `|x| < 2^52`) and `RC == 0` (round-to-nearest-even under invariant default host RNE in `|x| < 2^30` / `|x| < 2^52`, covering Hot Wheels' 41M `ctrl=0x127f` `FistpI32` ops).
- Strict Golden Rule 1 pre-mutation guards preserve bit-exactness across all hosts: every op checks `NaN` inputs and results (`fcmp`/`b.vs 954f`, `ucomisd`/`jp 954f`, routing all NaNs through Rust `x87::answer` to preserve #269 quieted sNaN payloads), `X87::used` occupancy/overflow, 2/4/8-byte guest bounds, single-page and `code_bitmap` write checks, and `RC == 1/2` / out-of-range `Fistp` bounds BEFORE mutating `X87` or guest memory. Empirical fallback profiling showed `lego-chess` (`0x003f`/`0x0c3f`, 157M ops) and `lego-alpha-team` (`0x0c3f`, 79M ops) execute in single-precision mode (`PC == 0`), so value-writing ops apply exact IEEE-754 `f64 -> f32 -> f64` rounding (`fcvt` / `cvtsd2ss`+`cvtss2sd`, matching `X87::round`) whenever `(control & 0x300) == 0`. `LEGO_LEGACY_ARCH=rust` continues to run the unchanged pure-Rust oracle.
- Verified via exhaustive differential tests covering every inlined x87 op across `NaN`/sNaN/`±0.0`/`±Inf`/subnormals/`PC=0..3`/`RC=0..3`/`_ftol` boundaries (`±2^30`, `-2^31`, `2^31-1`, `2^31`, `-2^31-1`, `±2^52`), stack wrap (`top=0..7`), empty/full stack faults, memory/page-crossing/`code_bitmap` faults, `FS:` overrides, lazy/materialized `AF` in `Fcomi`/`Fcomip`, and zero-fallback `run_fast_block` assertions on both native `aarch64-apple-darwin` and Rosetta 2 `x86_64-apple-darwin`; `scripts/check-targets.sh` across all six desktop targets; `core/tests/arch_determinism.rs`; `core/tests/x87.rs`; all ≥877 SingleStepTests 80386 conformance vectors; full `CLAUDE.md` PR gate; and Layer-2 headless parity (`native` vs `LEGO_LEGACY_ARCH=rust` vs `3b1aed2`) with exact `cursor`, `flips`, `declared_ms`, and `state_digest()` across all five titles:
  - `lego-chess` (`checkmate.route`): `26.18 s` wall / `28.05 s` user (`446.4 MIPS`, `4.16×` pace; `-28.5%` wall vs `36.60 s` `rust`, `-13.2%` vs #279's `30.17 s`).
  - `lego-alpha-team` (`first-mission.route`, close @ 7990): `69.55 s` wall / `72.66 s` user (`333.9 MIPS`, `4.74×` pace; `-28.2%` wall vs `96.83 s` `rust`, `-22.6%` vs #278's `89.81 s`).
  - `hotwheels-stunt-track-driver` (`first-race.route`): `25.39 s` wall / `24.97 s` user (`530.2 MIPS`, `4.76×` pace; `-32.1%` wall vs `37.40 s` `rust`, `-10.3%` vs #279's `28.30 s`).
  - `lego-rock-raiders` (`first-mission.route`, close @ 9050): `44.74 s` wall / `44.21 s` user (`318.6 MIPS`, `8.38×` pace; `-24.4%` wall vs `59.19 s` `rust`, `-2.2%` vs #279's `45.74 s`).
  - `lego-island` (`pizza-delivery.route`): `25.04 s` wall / `26.39 s` user (`425.6 MIPS`, `24.18×` pace; `-28.3%` wall vs `34.93 s` `rust`, `-14.5%` vs #278's `29.29 s`).

Software rasteriser `select_kernel` and `span_fast_rgb565` SIMD coverage gaps are closed across `aarch64` NEON, `x86_64` baseline SSE2, and the canonical pure-Rust oracle (`core/src/raster/span/{mod,rust,aarch64,x86_64}.rs`, ADR-0009, #278), leaving every frame, cursor, flip count, declared millisecond, and `state_digest()` unchanged:

- Empirical Step-1 pixel-tuple instrumentation across `lego-alpha-team` (`first-mission`, close @ 7990: 37.67% / 1.87B fallback pixels), `lego-island` (`pizza-delivery`: 4.09% / 119.6M fallback pixels), and `lego-rock-raiders` (`first-mission`, close @ 9050: 2.54% / 34.6M fallback pixels) showed 99.29% of fallback pixels came from seven tuples: `Argb4444` and `Rgb565` alpha-blended `alpha_test == Greater` draws (`z_le` and `Always && !depth_write`), untextured `alpha_test == Greater` `z_none` draws, untextured Gouraud opaque Z + specular (100% of LEGO Island's fallback), textured `(SourceAlpha, One)` additive blending (59.6% of Rock Raiders' fallback), and textured `(Zero, InverseSourceColour)` multiplicative shadows.
- Pure-Rust & `DrawPlan` wins: `Compare::Always && !depth_write` canonicalizes to `z_none` (tightening `need_z`), `alpha_test == Always` canonicalizes to none, and in-bounds opaque textures (`Rgb565`/`Rgb555`/`Xrgb8888`) with `alpha == Combine::Texture` simplify passing alpha tests and `(SourceAlpha, InverseSourceAlpha)` to opaque (moving 449M Alpha Team UI pixels onto `TexturedModulateOpaque` with zero framebuffer reads); `DrawPlan` precomputes a per-draw `[[f32; 4]; 256]` float palette LUT for `Paletted8` (direct 4×128-bit SIMD loads + transpose in `NearestPaletted8::sample4`, zero stack initialization for non-paletted draws via `MaybeUninit`); untextured `flat && specular` folds `(flat.0 + flat.1).min(1.0)` once per span; and `span_fast_rgb565` hoists `tex.is_some()`/`blend.is_some()` and replaces `[v; 4]` splats with scalar/`[f32; 3]` helpers, eliminating per-pixel `_memset_pattern16` calls (verified in release disassembly).
- `aarch64` NEON and `x86_64` baseline SSE2 parity (honoring `LEGO_LEGACY_ARCH=rust`): adds 4-pixel SIMD kernels for untextured opaque (`specular`, `z_none`, `z_le && !depth_write`), untextured alpha (`z_none`, `alpha_test == Greater`), textured opaque (`flat && specular`, `z_le && !depth_write`), and textured modulate blend (`z_none`, `alpha_test == Greater` with early 4-pixel transparent-quad skip before diffuse RGB/framebuffer loads, `flat && specular`, `(SourceAlpha, One)` additive, and `(Zero, InverseSourceColour)` multiplicative). Exact `f32::round()` semantics for `alpha_test == Greater` use `vrndaq_f32` (`FRINTA`) on NEON and `t + (frac >= 0.5)` on baseline SSE2, with zero FMA, zero `algebraic_*`, and zero approximate `rcp`/`rsqrt`.
- Verified via expanded Layer-1 differential oracle tests (widths `0..=65`, odd byte offsets `0..=7`, exhaustive `half.next_down() / 255.0` alpha-rounding boundaries, `NaN`/`±inf`/`-0.0`, and `span_general_in_bounds` checks) on both native `aarch64` and Rosetta 2 `x86_64-apple-darwin`, `core/tests/arch_determinism.rs`, all six desktop targets (`scripts/check-targets.sh`), the full `CLAUDE.md` PR gate, and Layer-2 headless parity (`native` vs `LEGO_LEGACY_ARCH=rust` vs `cd166e8` baseline) across all four Direct3D titles (exact `cursor`, `flips`, `declared_ms`, and `state_digest()`): `lego-alpha-team` `first-mission.route` (close @ 7990) 109.15 s → 89.81 s (-17.7%), `lego-island` `pizza-delivery.route` 48.81 s → 29.29 s (-40.0%), `lego-rock-raiders` `first-mission.route` (close @ 9050) 61.20 s → 54.49 s (-11.0%), and `lego-chess` `checkmate.route` 34.96 s → 33.10 s (-5.3%).

CPU fast block dispatch replaces the per-block 6-load 3-level page-table walk and 4 `FastCtx` `outside` reloads inside `run_fast_block` with a direct-mapped L1 block cache, direct static-transfer chaining, and a zero-load `outside` range check across `aarch64` and `x86_64` (SysV + Win64, ADR-0009, #279), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `cpu::interp::Interp` adds a 4,096-slot 16-byte `#[repr(C, align(16))]` direct-mapped L1 block cache (`L1Slot { eip: u32, run_idx: u32, run: Run }`, 64 KiB, indexed by `(eip ^ (eip >> 12)) & 0xfff`, cleared on `forget()` and `forget_page()` and populated in place on flat32 decode and L1 misses) while moving cold 3-level table pointers (`directory`, `region_ptrs`, `page_ptrs`) to miss-only loads from `FastCtx`.
- Static control transfers (`JmpRel`, `JccRel`, `CallRel`, `EndRun`) reuse unused 4-byte-aligned fields inside the unchanged 16-byte `#[repr(C)]` `FastInsn` (`imm` at offset 12 for target / `JccRel` taken, and `base..=aux` at offset 4 zeroed at decode for `JccRel` fallthrough) to cache `target_run_idx + 1` patched in place on first miss. Direct hits index `runs` directly (`x11 = runs - 8` on `aarch64`, preloading the chain word with `ldp` and falling through into dispatch) and trap `forget_page()` self-modifying-code invalidations automatically via `run.len == 0` with zero stale execution.
- `FastCtx` carries `outside_danger_start = outside.start.wrapping_sub(MAX_RUN_BYTES)` and `outside_danger_len = outside_len + MAX_RUN_BYTES` (or `0` when `outside.is_empty()`), pinned in `w8`/`x7` on `aarch64` so both EXE (`0x0040_0000`) and DLL (`0x1000_0000`) addresses pass `Boundary::RANGE` (and any custom or empty `outside` range) in two instructions with zero loads, branching to an exact containment and `[eip, eip + run.bytes)` overlap path only within `MAX_RUN_BYTES` of or inside `outside`. Zero FP/XMM/NEON registers, zero `x18`/`w18`, and zero outgoing calls are used; `LEGO_LEGACY_ARCH=rust` continues to run the unchanged pure-Rust oracle.
- Verified via new differential unit tests and integration tests covering `L1Slot`/`FastCtx` `offset_of!`/`size_of!` layouts, SMC overwrites of chained `JmpRel`/`JccRel` (taken & fallthrough)/`CallRel`/`EndRun` targets (including length-changing rewrites), same-slot L1 cache collisions/evictions (including slot 0), and custom `outside` ranges (below/above, wrapping `outside_danger_start`, `0..u32::MAX`, and re-running already-chained blocks under tighter `outside` bounds); native `aarch64` and Rosetta 2 `x86_64-apple-darwin` core lib tests; `scripts/check-targets.sh` across all six desktop targets; all ≥877 SingleStepTests 80386 conformance vectors; `arch_determinism`; full PR gate; and headless Layer-2 parity against `7d48827` and `LEGO_LEGACY_ARCH=rust` with exact `cursor`, `flips`, `declared_ms`, and `state_digest()`:
  - `hotwheels-stunt-track-driver` (`first-race.route`): `34.03 s` → `28.30 s` wall (`395.6` → `475.7 MIPS`, `4.27×` pace; `-16.8%` vs `7d48827`, `-28.6%` vs `39.61 s` `rust`).
  - `lego-rock-raiders` (`first-mission.route`, close @ 9050): `54.67 s` → `45.74 s` wall (`260.8` → `311.7 MIPS`, `8.20×` pace; `-16.3%` vs `7d48827`, `-23.6%` vs `59.89 s` `rust`).
  - `lego-chess` (`checkmate.route`): `33.60 s` → `30.17 s` wall (`347.8` → `387.3 MIPS`, `3.61×` pace; `-10.2%` vs `7d48827`, `-16.7%` vs `36.24 s` `rust`).

Declared clock conversions, audio frame/mix hot paths, and `Machine` boundary crossings are streamlined in pure Rust (#277), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `clock::Clock` caches `(cached_millis, next_millis_cycle)` and updates it only when `cycles >= next_millis_cycle` across `retire`, `retire_many`, `present`, `wait`, `at`, and `resumed`, making `Clock::millis()` a single cached `u64` read (so `Machine::ctx` copies one cached `u64` with zero per-crossing division and without changing `Ctx` outside this crate's owned files). `Clock::nanos()` reduces `1_000_000_000 / hz` by `gcd` at construction and computes 64-bit `(cycles * nanos_num) / nanos_den` with an exact `#[cold]` `u128` overflow fallback; `first_cycle_of` and `first_cycle_of_audio_frame` follow the same 64-bit-first + `u128`-fallback pattern.
- `sound::frame_at` computes GCD-reduced 64-bit `(nanos * 441) / 20_000_000` with an exact `#[cold]` `u128` overflow fallback. `sound::Mixer::catch_up` bulk zero-fills silent spans when `voices.is_empty() && synth.is_none()`, borrows each guest `Pcm` slice once per chunk via `mem.slice` (falling back per frame if out of bounds), and skips the unused `next` sample read and interpolation whenever `part == 0`.
- `machine::Machine` gates `mix()` behind `next_mix_cycle` (`Clock::first_cycle_of_audio_frame(mixer.next_frame() + 1)`, reset on `resume`), defers saving the 160-byte `Cpu` in `switch_thread` until `next != running`, reads `#[inline(always)] stack_args` into a caller `[u32; 16]` with a single `arena.slice` check (falling back per slot on fault to preserve exact fault address and order), adds a 2-compare `next_route_at` fast return in `poll`, embeds `BOUND_POLL`/`BOUND_CLOCK_READ` flags plus COM `(interface, slot)` and special entries (`Return`, `ThreadEnd`, `MidiPlayer`) directly in `self.bound`, inlines `threads.len() >= 2` and `trace` checks, and fuses the normal burst→crossing path in `step_fast` with a single `stopped(host)` check.
- Verified via unit tests proving `Clock::nanos`/`millis`/`first_cycle_of_audio_frame` and `sound::frame_at` match `u128` references bit-for-bit across `0..10^14` and overflow/saturation boundaries, `Mixer::catch_up` silence and guest-PCM/fallback parity, `stack_args` fault parity, `next_mix_cycle` gating, `next_route_at` poll delivery, and `switch_thread` copy skipping; the full `CLAUDE.md` PR gate (including all 877+ SingleStepTests 80386 conformance vectors and 33 `lego-chess` tests); and headless route verification against `7d48827` with identical `cursor`, `flips`, `declared_ms`, and `state_digest()`: `hotwheels-stunt-track-driver` `first-race.route` (`34.27 s` real / `33.89 s` user, `cursor 13462673313`, `flips 3903`, `declared-ms 120966`, digest `25ad6546...`) and `lego-island` `pizza-delivery.route` (`35.13 s` real / `36.09 s` user, `cursor 10657537990`, `flips 22041`, `declared-ms 605367`, digest `7d9a938f...`).

Software rasteriser banded dispatch now runs on a lazy persistent `BandPool` (`BANDS = 4`, ADR-0004) and `triangle_with_plan` computes and steps only active attribute planes (#275), leaving every frame, cursor, flip count, declared millisecond, and `state_digest()` unchanged:

- `raster::BandPool` (`core/src/raster/mod.rs`, pure `std::thread`/`std::sync`, zero external crates) keeps `BANDS - 1 = 3` persistent worker threads behind a `OnceLock` with 128-byte cache-line-padded spin-then-`thread::park`/`unpark` wake and a `JoinGuard` completion barrier that waits for all in-flight workers even on unwind (and captures/re-raises worker panics without poisoning the pool). The calling thread executes Band 0 directly while workers draw Bands 1..3, eliminating per-draw `pthread_create`/`mmap`/`mprotect` overhead. `raster::triangles` clamps the four-band split to the batch's actual `[min_y..max_y]` intersected with the clip rect and skips out-of-band triangles by Y bounds before entering `triangle_with_plan`. Empirical testing of `WORTH_BANDS = 16_000.0` vs `64_000.0` across all four Direct3D titles was mixed (slightly faster on Chess and Island, slower on Alpha Team and Rock Raiders with extra wake/park overhead), so `WORTH_BANDS` stays `64_000.0`.
- `DrawPlan` (`core/src/raster/span/mod.rs`) tracks exact active-plane flags (`need_z`, `need_rhw`, `need_uv`, `need_diffuse_rgb`, `need_diffuse_a`, `need_specular`, `need_fog`) and tightens `carried` (fixing `carried = 12` on `flat && specular`, tightening untextured flat Z to `1` and no-Z to `0`, textured copy to `4`, opaque RGB modulate to `7`, and Gouraud specular without fog to `11`). `triangle_with_plan` computes and per-scanline evaluates only active planes, hoists edge `(dx, dy, top_left)` per triangle and `ey = dx * (y - p.y)` per scanline, finds `(cov_from, cov_to)` before initializing `value` (skipping empty rows), and left-trims `value` with 4-lane `as_chunks_mut::<4>()` sequential `+= step` adds — preserving exact IEEE-754 `(ex * value) / area`, exact `from` bounds, exact `ey - dy * (x - p.x)` coverage, exact sequential stepping, and zero FMA.
- Verified via `raster::*` unit tests (including tightened `carried` / active-plane differential parity against `triangle_reference`, repeated concurrent clamped-band `BandPool` parity, and `JoinGuard` unwind barrier) on both `aarch64` (native, `LEGO_LEGACY_ARCH=rust`, and debug overflow checks) and `x86_64-apple-darwin` (Rosetta 2), `core/tests/arch_determinism.rs`, all six desktop targets (`scripts/check-targets.sh`), and the full `CLAUDE.md` PR gate.
- Measured headless on the Mac mini against `23f525d` (`WORTH_BANDS = 64_000.0`), all four Direct3D titles end on the exact same `cursor`, `flips`, `declared_ms`, and `state_digest()` while cutting kernel `sys` time by 37%–78%: `lego-island` `pizza-delivery.route` 61.92 s → 48.81 s (`sys` 2.16 s → 0.48 s), `lego-rock-raiders` `first-mission.route` (close @ 9050) 67.27 s → 61.20 s (`sys` 0.68 s → 0.43 s), `lego-alpha-team` `first-mission.route` (close @ 7990) 115.85 s → 109.15 s (`sys` 2.99 s → 0.97 s), and `lego-chess` `checkmate.route` 35.47 s → 34.96 s (`sys` 0.99 s → 0.32 s).

Win32, COM, and guest Arena/Heap hot paths drop per-call heap allocations, redundant `BTreeMap` traversals, `Vec::remove(0)` free-list shifts, and bit-by-bit multi-page code-bitmap scans in pure Rust (#276), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `guest::Arena::wrote_over_code_multi` scans the code-page bitmap word-wise in `u64` chunks with `head_mask` and `tail_mask` instead of bit-by-bit (3 word loads vs. ~150 iterations on a 640×480×16 framebuffer).
- `guest::heap::Heap` stores per-size free lists in `VecDeque<u32>` so standard 8-byte-aligned reuse pops from the front in O(1) while preserving exact FIFO guest pointer order (and wider-alignment scans preserve order via `VecDeque::remove`), and indexes `live` with a deterministic Fibonacci/xor-shift `HashMap` (zero host entropy; `live` is never iterated in order).
- `com::query_interface` matches compile-time `const [u8; 16]` binary GUID tables (`FACES`, `OWN`, `IUNKNOWN`, `RM_OBJECT`, `RM_VISUAL`) parsed via `const fn parse_iid`, formatting `iid_text` only on a DirectX 5 wall or refusal; `win32::interface_named` searches `INTERFACE_SETS` directly without allocating a `Vec`.
- `win32::State::com` and `State::com_mut` fast-path the primary identity (`o.identity == at`) in a single lookup; `State::handles` is stored as `Vec<Option<Object>>` indexed by `(handle - 0x100) >> 2` with alignment checking and exact ascending handle iteration; `State::sections` uses a compact `Vec<(u32, u32, u32)>` paired with a `section_waiters: u32` counter so uncontended `EnterCriticalSection`/`LeaveCriticalSection` skip unparking and waiter scans; and `win32::fill` writes in place via a single `Arena::slice_mut` with specialised 1/2/3/4-byte pixel loops.
- `win32::kernel32::wait_for_single_object` resolves and claims in a single handle lookup and skips `park()` when single-threaded or already unparked; `win32::dsound::Buffer` caches `(cached_millis, cached_at, cached_playing)` (invalidated on `settle`, `change`, `play`, `stop`, and `duplicate`) to avoid repeated 64-bit divisions within the same declared millisecond; `win32::dinput::device_data` short-circuits when input queues are empty while preserving exact `count_at` fault checks; and `win32::ddraw` short-circuits `dc_out`/`busy` when `surface_dcs` is empty, merges `surface_of`/`format_of`/`caps_of` into a single `SurfaceInfo` lookup, and copies disjoint `Flip` buffers in place via `Arena::read_and_write` with an overlapping fallback.
- Verified via new unit tests covering word-wise `wrote_over_code_multi` (same-word, adjacent-word, multi-word), strict FIFO `Heap` reuse, binary GUID `query_interface` hits/walls/refusals, `State::handles` lookup/close/iteration, `State::com`/`com_mut`, `interface_named`, `win32::fill`, `wait_for_single_object`/`sections`, `dsound::Buffer` cache invalidation, `dinput::device_data` empty fast path, and `ddraw::flip`/`dc_out`; full PR gate green; and headless route verification on `hotwheels-stunt-track-driver` (`first-race.route`), `lego-rock-raiders` (`first-mission.route`, close @ 9050), and `lego-island` (`pizza-delivery.route`) ending on identical `cursor`, `flips`, `declared_ms`, and `state_digest()`.

Direct3D frustum clipping and immediate-mode draw setup drop per-triangle stack zeroing, indirect plane calls, and redundant COM/device lookups (`core/src/win32/d3d.rs`, #274), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `d3d::Immediate::scratch` (`DrawScratch`) now owns persistent `clip_buf`, `clip_scratch`, and `poly_buf` arrays (initialised once per machine instead of zeroing 2,016 bytes per clipped triangle), and `emit_clipped` skips viewport projection whenever clipped `len < 3`.
- `d3d::rasterise` clips and culls into `scratch.raster` before texture lookup, 1 KB palette copy, `take_depth_at`, and `Arena::read_and_write` (returning early when empty), using an inline monomorphised six-plane clipper (`clip_plane<const PLANE: u8>` with cached `dp` across edges and `CLIP_BUFFER` overflow fallback to `pipeline::clip`) guarded by exact 6-bit frustum outcodes (`or == 0` trivial accept; `and != 0` with no prior straddling plane and finite `w > 0` trivial reject, bailing to the full clipper on `w <= 0`, `NaN`, or `±inf`).
- `Immediate::devices` switches from `BTreeMap<u32, Device>` to `Vec<(u32, Device)>` with single-lookup `device_index` in `draw`, `rasterise`, `clear_on`, and `clear_depth`; `ComState::clone()` calls in `draw`, `rasterise`, `clear_rects`, and `clear_on` are removed; `Device::view_proj` caches `VIEW * PROJECTION` across `WORLD` updates (invalidated on `SetTransform(VIEW|PROJECTION)`); `set_material` reuses its existing `Vec<u8>`; and `draw` decodes `D3DFVF_TLVERTEX` directly through an exact `U8_TO_F32` lookup table, skipping dead specular unpack when `!specular && fog.is_none()`.
- Unit tests pin exhaustive `U8_TO_F32` and `D3DFVF_TLVERTEX` decode bit-identity, exact outcode trivial-accept/reject and monomorphised 6-plane clipping against `pipeline::clip` (including prior-straddling-plane FP rounding, `w <= 0`, `NaN`, and `±inf`), `view_proj` cache invalidation, and `Vec` device-slot reuse. Headless verification on `lego-island` (`pizza-delivery.route`, 96.03 s → 77.23 s wall / 76.77 s → 69.71 s user under concurrent Wave A load) and `lego-chess` (`checkmate.route`) confirms identical `cursor`, `flips`, `declared_ms`, and `state_digest()`.

Direct3D Retained Mode lighting, mesh culling, and scene-graph traversal are optimised in pure Rust (#273), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `win32::pipeline` adds `PrecomputedLight`, `PreparedLights`, and `lit_prepared` to fold ambient lights in slice order, normalise directional `towards` and `halfway = normalise(towards + viewer)` vectors and spot axes once per pass, and track `has_positional`. Both `lit` and `lit_prepared` inline specular highlight evaluation, compute point/spot distance `dot(v, v).sqrt()` once before the range check (`scene::normalise_with_length`), skip spot highlights when `cone == 0.0` under finite non-negative power, and skip `x87::maths::log2`/`pow2` when the specular exponent is guaranteed finite `<= -151.0` (never calling host `powf`).
- `win32::d3drm::draw_mesh` projects clip coordinates and exact 6-bit frustum outcodes (`frustum_outcode`, guarding non-finite coordinates and matching `pipeline::clip_into`'s six half-space planes across `w <= 0`) before lighting, trivially rejecting off-screen groups and faces while preserving exact unset-vertex and indexed-texture walls; lazily lights only surviving vertices (and only `face[0]` on `SHADE_FLAT`); skips world-space `project(world, pos)` when `!has_positional`; and clears `states.specular = false` when all surviving vertices have zero specular and finite diffuse so the rasteriser takes its non-specular SIMD span kernels.
- `win32::d3drm` and `win32::scene` fuse `collect_visits` and `collect_lights` into a single `collect_scene` traversal with lazy bottom-up `Scene::world()` (skipping empty, user-visual-only, and ambient-only frames while preserving exact bottom-up matrix associativity and bitwise-identity shortcuts), hoist or cache filtered `PreparedLights` per `last_frame` in `draw_mesh_pass`, add row-3-only `relative_origin()` / `Scene::world_origin_row()` / `scene::multiply_row()` for `GetPosition`, and store `Scene::frames` in a flat slab (`FrameTable`) with a deterministic zero-entropy `u32` open-addressed lookup and cached `parent_slot`.
- Verified via bit-exact differential unit tests (`PrecomputedLight` / `lit_prepared` vs. reference `lit`, `frustum_outcode` vs. `pipeline::clip` across `w <= 0` and `NaN`/`Inf`, lazy flat/culled drawing and unset-vertex walls, `FrameTable` and `relative_origin()` including `-0.0` and signaling NaN payloads), the full `CLAUDE.md` PR gate, and headless route verification against `23f525d` with identical `cursor`, `flips`, `declared_ms`, and `state_digest()`: `lego-island` `pizza-delivery.route` drops from 83.93 s wall (75.75 s user) to 58.93 s wall (54.97 s user, -29.8% wall / -27.4% user) and `lego-rock-raiders` `first-mission.route` (close @ 9050) drops from 87.15 s user to 82.53 s user (-5.3% user).

Pure-Rust CPU, x87, flags, and string-op hot paths are tightened and `X87` is compacted to a fixed `#[repr(C)]` layout ready for Wave C assembly (#272), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `cpu::x87::X87` switches to a 72-byte `#[repr(C)]` layout (`regs: [f64; 8]` @ 0, `top: u8` @ 64, `used: u8` bitmask @ 65, `control: u16` @ 66, `status: u16` @ 68, pinned by `offset_of!` unit tests), drops the 32-byte `Stop` return from `x87::execute`/`read`/`write`/`read_extended`/`write_extended` in favor of `Result<_, crate::guest::Fault>`, marks `X87::compare` and `x87::answer` `#[inline(always)]` with NaN propagation split out to `#[cold] #[inline(never)] answer_nan`, adds provably bit-exact `let next = sum + term; if next == sum { break; }` early-exit to `x87::maths::{log2, pow2}`, and bit-constructs `x87::exp2` directly for `-1022..=1023` (`f64::from_bits(((power + 1023) as u64) << 52)`) with a `#[cold]` fallback outside the normal exponent range.
- `cpu::x87::X87Op` is `#[repr(u8)]` with `to_fast`/`from_fast` packing its 2-byte `(tag, index)` directly into `FastInsn::{aux, dst}` (keeping `size_of::<FastInsn>() == 16` and `FastCtx` untouched), removing `Interp::x87_ops` and its per-op indirection; `cpu::interp::rust::finish_run_from` is marked `#[cold] #[inline(never)]` so `run_native` stays lean.
- `cpu::flags::Flags` adds `#[inline(always)]` `add8`/`sub8`/`add16`/`sub16` routed from `exec` 8/16-bit ALU, `neg`, and string comparisons, and `cpu::exec::string_op` adds 8-byte SWAR/word-wise bulk forward `repne scasb` and `repe cmpsb` fast paths (including `ECX = u32::MAX` `strlen` scans), falling back element-by-element on faults, `DF=1`, `addr16`, and `Arena::is_watching()`.
- Verified via exhaustive unit tests (`X87` `offset_of!`, `used` bitmask wrap/free, all `X87Op` decode/fast round-trips, `exp2` over `-1200..=1200`, `log2`/`pow2` early-exit bit-identity, exhaustive 65,536-pair `add8`/`sub8` and edge+sweep `add16`/`sub16`, `repne scasb`/`repe cmpsb` differential checks across hits/misses/overlaps/`FS:`/`DF=1`/`addr16`/faults/watches), native and Rosetta 2 `x86_64-apple-darwin` core lib tests, `core/tests/x87.rs`, `core/tests/synth.rs` pinned SHA-256 hashes, all 877+ SingleStepTests 80386 conformance vectors, full PR gate, and headless route replays of `lego-chess` (`checkmate.route`, cursor `11685933862`, flips `4311`, declared-ms `109017`, digest `aa189299...`) and `lego-alpha-team` (`first-mission.route` close @ 7990, cursor `23226131304`, flips `7991`, declared-ms `329886`, digest `98f4c238...`).

Presentation surface conversion, headless frame capture, and run-log writes are optimised behind a co-located presentation seam (`core/src/present/{mod.rs, rust.rs, aarch64.rs, x86_64.rs}`, ADR-0009, #271), leaving every frame, cursor, flip count, declared millisecond, and `state_digest()` unchanged:

- `present::paletted8_to_bgra` builds a 256-entry pre-swizzled BGRA `[[u8; 4]; 256]` stack LUT once per flip from `arena.slice(palette, 1024)` (with per-entry `arena.read_u32` fallback on fault and opaque black `[0, 0, 0, 0xff]` when `palette == 0`) and converts packed or padded rows without per-pixel guest-memory lookups (~5.7×–6.0× faster on 640×480 frames).
- `present::rgb565_to_bgra` pairs a row-sliced pure-Rust oracle (`as_chunks::<2>()` -> `as_chunks_mut::<4>()`) with baseline `aarch64` NEON (`vld1q_u8`/`vsriq_n_u8`/`vst4q_u8`, 16 px/iter + 8-px pass; -26.8% vs pure Rust, ~16.2× vs scalar) and baseline `x86_64` SSE2 (8 px/iter; -58.1% vs pure Rust under Rosetta 2, ~15.9× vs scalar) adapters honoring `arch::use_rust()`, preserving exact top-bit channel replication `(b << 3) | (b >> 2)`, `(g << 2) | (g >> 4)`, `(r << 3) | (r >> 2)`, `0xff`. `present::copy_bgra32` copies dense 32-bit frames in one slice copy.
- `host::headless::Headless::present` recycles evicted `previous.pixels` buffers when `!keep.contains(&previous.flip)`, bulk-copies dense frames (`pitch == width * 4`), and scans uniformity over 16-byte/4-byte chunks instead of byte-slice comparisons.
- `runlog::RunLog` wraps its log `File` in a 64 KiB `BufWriter<File>`, adds `RunLog::line_fmt(fmt::Arguments)` (used by `Machine::present_guest` to eliminate per-flip `format!` `String` allocations), and flushes on `Drop`, `dir()`, `flush()`, and `end`/`wall`/`digest` lines.
- Verified by exhaustive differential unit tests over all 65,536 RGB565 values, odd byte offsets (`0..8`), and widths `0..=65` on both `aarch64` NEON and `x86_64-apple-darwin` SSE2 (Rosetta 2); `scripts/check-targets.sh` across all six desktop targets; `core/tests/arch_determinism.rs`; the full PR gate; and headless `native` vs `LEGO_LEGACY_ARCH=rust` route parity on `lego-island` (`pizza-delivery.route`: `real 99.41 s -> 60.91 s`, `user 79.02 s -> 58.91 s`, `sys 4.28 s -> 2.58 s`; `cursor 10657537990`, `flips 22041`, `declared-ms 605367`, identical digest) and `hotwheels-stunt-track-driver` (`first-race.route`: `real 42.70 s -> 39.35 s`, `user 40.98 s -> 37.82 s`, `sys 0.49 s -> 0.43 s`; `cursor 13462673313`, `flips 3903`, `declared-ms 120966`, identical digest).

x87 whole-number rounding (`round_half_to_even` and `X87::to_whole`) now routes NaN inputs through `x87::answer` (#269), quieting signaling NaNs with their sign and payload preserved identically across `aarch64` and `x86_64` (macOS, Linux, and Windows) where baseline `x86_64` `f64::round_ties_even` on Linux and `f64::round` on Windows otherwise leave signaling NaNs unquieted; `round_half_to_even_matches_reference_bit_for_bit` pins exact quieted NaN bits directly and tests `X87::to_whole` across all four rounding modes, with zero drift on any committed route or digest.

CPU interpreter flat32 execution is accelerated with static, direct-threaded `aarch64` and `x86_64` `#[unsafe(naked)]` + `naked_asm!` dispatch loops behind a co-located seam (`core/src/cpu/interp/{mod.rs, rust.rs, aarch64.rs, x86_64.rs}`, ADR-0009, #256), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `cpu::insn` and `cpu::exec` extend `FastOp` with flat32 8-bit and 16-bit `mov`/ALU, 32-bit `imul` (`Imul2`), `setcc`, `fwait` (`Nop`), and an `EndRun` sentinel appended after non-transfer runs (keeping `FastInsn` at 16 bytes `#[repr(C)]` and `Interp::cached()` unchanged). `Arena::raw_parts_mut` exposes the arena pointer, floor, length, and code-page bitmap to the crate-internal seam.
- `cpu::interp::aarch64` and `cpu::interp::x86_64` implement a single-pointer, integer-only `extern "C" fn run_fast_block(ctx: *mut FastCtx) -> u32` direct-threaded loop that pins arena, code-bitmap, decode-cache, flag, and dispatch-table invariants across basic blocks via `Interp`'s 3-level page table, uses a branchless 16-entry `u16` condition LUT for `Jcc`/`Setcc`, touches zero FP/XMM/NEON registers (leaving Win64 `xmm6..xmm15` untouched), never names `x18`/`w18`, makes zero outgoing calls, and checks bounds and `code_bitmap` before mutating guest state — returning cleanly to `cpu::interp::rust` on cache misses, budget tails, `outside` overlaps, `Slow`/`X87` ops, faults, page-crossing writes, or code-page writes. `Mode::REAL`, `LEGO_LEGACY_ARCH=rust`, active `Arena::watch()`, and non-`aarch64`/non-`x86_64` targets execute `cpu::interp::rust`.
- `.github/workflows/x86_64-core-tests.yml` runs `lego-legacy-core` unit, interpreter, and determinism tests on `ubuntu-latest` (SysV) and `windows-latest` (Win64) in both native and `LEGO_LEGACY_ARCH=rust` modes.
- Verified across all six desktop targets (`scripts/check-targets.sh`), locally on `x86_64-apple-darwin` under Rosetta 2, on all 877+ SingleStepTests 80386 conformance vectors, on differential `run_native` vs `rust::run` unit tests covering every supported `FastOp`, all 64 arithmetic flag combinations, faults, `code_bitmap` hits, and watches, and on `arch_determinism`.
- Measured headless on the Mac mini: tight flat32 loop throughput jumps from 496.1 to 1,751.4 MIPS (+253% / 3.53×); `hotwheels-stunt-track-driver` `first-race.route` drops from 47.31 s (`375c31a`, 42.29 s on `LEGO_LEGACY_ARCH=rust`) to 37.83 s (355.9 MIPS, 3.20× pace, -20.0% vs `375c31a` / -10.5% vs `rust`); `lego-rock-raiders` `first-mission.route` (close @ 9050) runs in 66.13 s vs 67.17 s `rust` (-11.0% vs #253's 74.34 s); `lego-alpha-team` `first-mission.route` (close @ 7990) in 114.49 s vs 117.60 s `rust`; `lego-chess` `checkmate.route` in 40.92 s vs 42.65 s `rust`; and `lego-island` `pizza-delivery.route` (close @ 15400) in 38.86 s vs 39.67 s `rust` — all five Roster titles producing identical `cursor`, `flips`, `declared_ms`, and `state_digest()`.

Software rasteriser interior spans are accelerated 4 pixels at a time with `aarch64` NEON and `x86_64` baseline SSE2 adapters behind a co-located seam (`core/src/raster/span/{mod.rs, rust.rs, aarch64.rs, x86_64.rs}`, ADR-0009, #255), leaving every frame, cursor, flip count, declared millisecond, and `state_digest()` unchanged:

- Step 1 Go/No-Go gate passed decisively on `375c31a`: post-Wave-1 `sample` profiling showed `raster::triangles` at 20.5% (`lego-chess` `checkmate`), 22.9% (`lego-island` `pizza-delivery`), 20.0% (`lego-rock-raiders` `first-mission` 3D phase), and 12.9% (`lego-alpha-team` `first-mission`), and the 4-pixel `aarch64` NEON span prototype beat pure Rust by 23.9%–52.1% (1.31×–2.09×) across the hot RGB565 span kernels.
- `core/src/raster/span/` splits the canonical pure-Rust oracle into `rust.rs` beside matching 4-pixel `aarch64` NEON (`aarch64.rs`) and `x86_64` baseline SSE2 (`x86_64.rs`, zero runtime feature dispatch) adapters covering all six specialized RGB565 span families (untextured flat/Gouraud opaque and alpha-blended, and textured `Modulate`/`Texture` nearest and `bilinear16` across RGB565, ARGB1555, ARGB4444, and 8-bit paletted, opaque and `SourceAlpha`/`InverseSourceAlpha`, plus specular and vertex fog). `DrawPlan::new` honors `LEGO_LEGACY_ARCH=rust` via `crate::arch::use_rust()` and delegates `0..3` tail pixels to `rust.rs`.
- Golden Rule 1 bit-exactness is preserved across all six desktop targets: sequential `+= step` plane stepping via 4-lane vector adds and 4×4 transpose, exact IEEE-754 `vdivq_f32`/`_mm_div_ps`, separate rounded multiply and add (zero FMA, verified in emitted assembly and `core/tests/arch_determinism.rs`), exact Q16 integer bilinear weights (`_mm_mul_epu32` on SSE2), clamped `[0.0, 1.0]` float-to-int RGB565 packing (matching saturating `as u32` including `NaN`), and unaligned 64-bit pixel slice I/O.
- Verified via Layer 1 differential unit tests (`aarch64` native and `x86_64-apple-darwin` under Rosetta 2) across span widths `0..=65`, unaligned byte offsets, and `NaN`/±inf/non-positive `rhw`/huge-UV edge inputs; `scripts/check-targets.sh` across all six desktop targets; and Layer 2 full-route `LEGO_LEGACY_ARCH=rust` vs. native parity on all four Direct3D titles (identical `cursor`, `flips`, `declared_ms`, and `state_digest()`): `lego-chess` `checkmate.route` 41.78 s → 38.55 s (-7.7%), `lego-rock-raiders` `first-mission.route` (close @ 9050) 79.79 s → 75.45 s (-5.4%), `lego-alpha-team` `first-mission.route` (close @ 7990) 126.40 s → 121.56 s (-3.8%), and `lego-island` `pizza-delivery.route` 64.91 s → 64.19 s (-1.1%).

CPU interpreter run-loop invariants are hoisted and hot flat32 runs execute from a compact 16-byte `#[repr(C)]` `FastInsn` IR (#253), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `cpu::insn` defines a compact 16-byte `#[repr(C)]` `FastInsn` (`size_of == 16`, `align_of == 4`, `#[repr(u8)]` `FastOp` with non-terminal ops `0..=47` and terminal transfers `48..=55`) ready for Wave 2 `naked_asm!` dispatch (#256). `Interp` replaces its parallel `Vec<Insn>` + `Vec<Fast>` hot arrays with `fast_insns: Vec<FastInsn>`, keeping full `Insn` records only in cold `slow_insns: Vec<Insn>` and 2-byte pre-decoded x87 ops in `x87_ops: Vec<X87Op>`.
- `cpu::exec` expands flat32 fast handling beyond the old 5 shapes (#70) to cover `nop`, 32-bit `mov` (reg-reg, reg-imm, reg-mem, mem-reg, mem-imm), `lea`, `movzx`/`movsx` (8/16 to 32), `push`/`pop` (reg, imm, mem), 32-bit ALU (`add`/`sub`/`cmp`/`and`/`or`/`xor`/`test`/`inc`/`dec`), `shl`/`shr`/`sar` (imm and `cl`), `x87` reg/mem dispatch, and terminal `jmp`/`jcc`/`call`/`ret` (direct and indirect) via `effective_flat32` (skipping `addr16`/`base_shift` and adding `seg_base[FS]` only when `FS:` is used), `Arena` scalar helpers, and `flags32`. `Mode::REAL` and `addr16`/`rep` instructions stay on `FastOp::Slow` -> `exec::execute`.
- `cpu::interp::Interp` stops decoding runs before `Boundary::RANGE`, clamps the run step budget and checks `outside` overlap once per run, and splits non-terminal straight-line instructions from the terminal control transfer — dropping per-instruction count, `outside`, `eip`, `CS`, and `tsc` updates in the inner loop while preserving exact `code_written` invalidation, `rdtsc` cycle counts, `Stop::Interrupt` (+1 retirement), `Stop::Halted` (0 retirement, `eip` after `hlt`), and fault rollback (`eip = at`).
- All 877+ SingleStepTests 80386 conformance vectors, unit tests (16-byte `#[repr(C)]` layout/offsets, differential `FastInsn` vs `exec::execute`, mid-run fault/interrupt/halt/`rdtsc`/self-modifying-code tests), and `arch_determinism` pass.
- Measured headless on the Mac mini against `3e288e6`, all four benchmark routes end on the exact same `cursor`, `flips`, `declared_ms`, and `state_digest()`: `hotwheels-stunt-track-driver` `first-race.route` drops from 72.66 s to 45.88 s (-36.9%, 185.3 → 293.4 MIPS, pace 1.66× → 2.64×), `lego-chess` `checkmate.route` from 58.00 s to 39.52 s (-31.9%, 155.4 → 228.1 MIPS, pace 1.88× → 2.76×), `lego-rock-raiders` `first-mission.route` (close @ 9050) from 98.24 s to 74.34 s (-24.3%, 145.1 → 191.8 MIPS, pace 3.82× → 5.04×), and `lego-alpha-team` `first-mission.route` (close @ 7990) from 170.40 s to 116.18 s (-31.8%, 136.3 → 199.9 MIPS, pace 1.94× → 2.84×).

32-bit CPU arithmetic flag updates are specialised and x87 instructions are pre-decoded at decode time with single 8-byte `Double` memory I/O (#258), leaving every run's cursor, flips, declared milliseconds, and `state_digest()` unchanged:

- `cpu::flags::Flags` replaces `u8::count_ones()` parity with a branchless 16-bit nibble lookup (`0x9669u32 >> ((b ^ (b >> 4)) & 0xf)`) fast on both `aarch64` and baseline `x86_64` SSE2, and adds `#[inline(always)]` 32-bit flag helpers (`add32`, `adc32`, `sub32`, `sbb32`, `logic32`, `logic_keep_carry32`, `inc32`, `dec32`) using native `overflowing_add`/`overflowing_sub` without dynamic `Width` masks or `u64` widening. `cpu::exec` routes hot `Width::B32` arithmetic, logic, `inc`/`dec`, `neg`, shift, and string-compare paths through them.
- `cpu::x87` defines a compact 2-byte `X87Op` enum (`Kind::X87(X87Op)`, preserving `Kind` and `Insn` sizes) and decodes `(escape, modrm)` once in `cpu::decode` via `x87::decode_reg`/`x87::decode_mem`, so `x87::execute` dispatches directly with no per-execution opcode decoding.
- `x87::read`/`x87::write` (`Format::Double`), 64-bit integer loads/stores (`fild`/`fistp` qword), and 10-byte extended significands use single 8-byte `Arena::read_u64`/`write_u64` calls (and 16-bit FPU memory ops use `read_u16`/`write_u16`), and `round_half_to_even` uses `f64::round_ties_even()` (preserving `+0.0` for `-0.5` for exact bit-identity with the legacy formula). `x87::maths` and `synth.rs`'s SHA-256 table pins are untouched.
- All 877+ SingleStepTests 80386 conformance vectors (1,513,954 vectors), unit tests (exhaustive 256-byte parity, differential 32-bit flag tests, exhaustive 2,048 `(escape, modrm)` pre-decode checks, 8-byte Double I/O fault/invalidation tests, `round_half_to_even` bit-identity tests), and `arch_determinism` pass.
- Measured headless on the Mac mini against `fdf4b55`, all three benchmark routes end on the exact same `cursor`, `flips`, `declared_ms`, and `state_digest()`: `lego-chess` `checkmate.route` drops from 63.72 s to 56.78 s (-10.9%), `lego-alpha-team` `first-mission.route` from 166.89 s to 156.77 s (-6.1%), and `hotwheels-stunt-track-driver` `first-race.route` from 75.34 s to 72.77 s (-3.4%).

The software rasteriser specialises its covered scanline spans behind a co-located interior-span
seam (`core/src/raster/{mod.rs, span.rs}`, #254), leaving every frame and every run's digest
unchanged:

- Each scanline trims its left and right edges to the exact covered interval `[cov_from..=cov_to]`
  (preserving top-left edge coverage and sequential plane stepping), verifies target depth and
  pixel row bounds once per row, and hands the whole covered span to `span::draw_interior` with no
  per-pixel edge tests or slice bounds checks.
- Power-of-two textures without clamping wrap with a bitmask (`c & (dim - 1)`) instead of
  `rem_euclid`, and eight-bit paletted texels map through an exact `c as f32 / 255.0` lookup table.
- `DrawPlan::new` classifies the target format, texture format and addressing, and render states
  once per draw (`triangles()` shares one plan across all triangles and all four `BANDS`),
  resolving a single monomorphised span function pointer for the hot RGB565 paths (untextured flat
  and Gouraud opaque/blended, and textured `Modulate`/`Texture` nearest and `bilinear16` across
  RGB565, ARGB1555, ARGB4444 and 8-bit paletted, opaque and `SourceAlpha`/`InverseSourceAlpha`,
  plus DirectX 5 specular and vertex fog) with compile-time channel shifts and fallbacks to a fast
  RGB565 loop and an arbitrary-format loop.
- Exact plane setup order, the constant-depth shortcut (#210), sequential `value[k] += step[k]`,
  IEEE-754 `1.0 / rhw`, `bilinear16` Q16 integer weights, non-FMA blending, RGB565 rounding, and
  four-band multithreading (`BANDS = 4`) are unchanged. Differential unit tests prove every
  specialised path bit-for-bit identical to the general reference loop across formats, wrap/clamp
  modes, alpha modes, NaN/non-positive `rhw`, and span widths `0..=65`, and verify 4-band vs 1-band
  equivalence.
- Measured headless on the Mac mini against the baseline profile, all four Direct3D titles end on
  the exact baseline `cursor`, `flips`, and `declared_ms`: `lego-chess` `checkmate.route` drops
  from 81.41 s to 71.25 s (-12.5%), `lego-rock-raiders` `first-mission.route` from 118.38 s to
  109.70 s (-7.3%), `lego-island` `pizza-delivery.route` from 126.45 s to 106.62 s (-15.7%), and
  `lego-alpha-team` `first-mission.route` from 197.15 s to 177.64 s (-9.9%).

Direct3D and Retained Mode drop per-triangle heap allocations, reuse draw scratch buffers, flat-index device state tables, and sink scene NaN scanning behind a cold path (#259):

- `pipeline::clip_into` clips polygons into a fixed 12-vertex stack buffer (a convex triangle clipped against six frustum planes yields at most 9 vertices), with `#[cold]` heap fallback for oversized or pathological inputs; `d3d::rasterise` and `d3drm::nearest_face` clip and project on the stack without per-triangle `Vec` allocations, and wholly-inside or already-transformed triangles skip the clip buffers altogether.
- `d3d::Immediate` and `scene::Scene` reuse scratch buffers (`indices`, `lights`, `prepared`, `triangles`, `raster`, and Retained Mode mesh-pass traversal buffers) across draw calls and frames, avoid `Vec` allocations in `set_device_transform`/`get_transform`, skip world/view/projection matrix multiplies for screen-space (`XYZRHW`) draws, and hoist per-pass quality and surface lookups out of `d3drm::draw_mesh`.
- `d3d::Device` stores render states (`0..160`), texture stage states (`8 × 32`), transforms (`0..4`), textures (`0..8`), and light states (`0..8`) in flat arrays with set-bitmasks and `BTreeMap` overflow spill for out-of-range indices, eliminating per-draw `BTreeMap` lookups while keeping DirectX 5/6/7 default and unset-state semantics unchanged.
- `scene::multiply` and `scene::settled` sink `x87::answer_single` operand scanning into `#[cold]` helpers when no output cell is NaN, preserving exact left-fold `-0.0` canonicalisation and cross-ISA NaN payloads.
- Pure optimisation with bit-exact output: `lego-island` (`pizza-delivery.route`) and `lego-chess` (`checkmate.route`) end on the exact same cursor, flips, declared milliseconds, and `state_digest()`, so no released run moves.

Per-call machine allocations are removed, guest arena scalar accesses and code-page invalidation
checks are tightened, and flat forward `rep movs`/`rep stos` take bulk copy/fill paths (#252):
- `Machine::stack_args` reads up to 16 arguments into a stack buffer (`StackArgs::Inline`) and
  falls back to the heap only past that; `Machine::deadlocked` short-circuits when fewer than two
  guest threads are live and allocates only when formatting an actual deadlock.
- `GuestMem` gains typed `read_u8`/`write_u8`, `read_u16`/`write_u16`, and `read_u64`/`write_u64`
  beside `read_u32`/`write_u32`; `Arena` implements all eight with a single non-zero bounds check,
  unaligned little-endian scalar access, a single-page code-bitmap fast path in `wrote_over_code`
  (out-of-line `wrote_over_code_multi` on page crossings), `#[cold] #[inline(never)]` on
  `note_written`, and clock-spin watch preservation via `hold_bytes`.
- `cpu::exec` routes scalar loads/stores through `Arena`'s typed helpers and executes 32-bit
  forward `rep movs` / `rep stos` (when `count > 1`, non-wrapping, in bounds, disjoint for `movs`,
  and not watching) in bulk via `read_and_write` / `slice_mut`, falling back element-by-element on
  overlaps, `DF = 1`, active watches, or mid-way faults.
- All 877+ 80386 conformance vectors pass, unit tests cover page-crossing writes, code
  invalidation, floor/end faults, `stack_args` overflow, and `rep` edge cases, and no route moves:
  Hot Wheels' `first-race.route` drops from 103.3 s to 79.8 s wall (97.6 s to 79.1 s user;
  90.4 s in #246) and LEGO Chess's `throne-room.route` from 6.32 s to 5.32 s wall (6.07 s to
  5.39 s user; 5.48 s in #246), both with identical cursor, flips, declared milliseconds, and
  `state_digest()`.

DirectDraw's surface blit transfer (`win32::ddraw::transfer`) now takes pure-Rust fast paths (#251),
replacing per-scanline heap copies, per-pixel 64-bit integer divisions, and dynamic-width per-pixel
`copy_from_slice` calls with bit-exact equivalents:
- Disjoint 1:1 unkeyed blits borrow source and destination once through `Arena::read_and_write` and
  copy contiguously (single `copy_from_slice` when pitches match row width) or row by row with zero
  allocations.
- Disjoint 1:1 colour-keyed blits specialize by pixel byte width (`u8`, `u16`, 24-bit, `u32`) with
  fixed-width chunk loops and zero per-scanline allocations.
- Stretched blits precompute the nearest-neighbour source-X index table once per blit (keeping the
  exact `sx0 + (x - dx0) * sw / dw` integer formula), reuse repeated unkeyed scanlines, and
  specialize pixel copies by byte width.
- Overlapping source/destination blits walk scanlines in ascending `y` order through a single
  reusable scratch row buffer, preserving exact reference behaviour across all four self-overlap
  directions, bounds faults, `wrote_over_code` invalidation, and `Arena::watch` cancellation.
- Unit tests in `core/src/win32/ddraw.rs` hold every path against the pre-#251 reference algorithm
  across 8/16/24/32-bit depths, clipping, self-overlap directions, stretches, faults, code-page
  writes, and watch notifications.
- Measured headless on the Mac mini, LEGO Island's `maintainer-first-play.route` drops from 53.37 s
  to 43.70 s (-18.1%) and `pizza-delivery.route` from 126.45 s to 105.91 s (-16.2%), and LEGO Rock
  Raiders' `first-mission.route` from 118.38 s to 116.89 s, each ending at the identical cursor,
  flips, declared milliseconds, and state digest. No released run moves.

Cross-platform static host assembly seam and determinism scaffolding (#249, ADR-0009):
- `arch` (`core/src/arch.rs`) exposes `Tier` (`Rust`, `Aarch64`, `X86_64`), `Tier::native()`,
  `Tier::active()`, and `arch::use_rust()`, backed by a single `OnceLock` reading the startup
  environment variable `LEGO_LEGACY_ARCH` (`rust` forces the canonical pure-Rust fallback,
  `native` or unset uses the compile-time host tier, and any unrecognized value fails loudly).
  Nothing in `arch` is guest-visible or hashed into machine digests.
- `core/tests/arch_determinism.rs` enforces ADR-0009's Golden Rule 1 bans across `core/src/`
  (FMA contraction, `algebraic_*` float operations, approximate reciprocal/rsqrt instructions,
  host FP control-register mutations, and explicit `aarch64` `x18`/`w18` operands), with unit
  tests for every forbidden pattern and end-to-end tests for `LEGO_LEGACY_ARCH`.
- `scripts/check-targets.sh` compiles `lego-legacy-core` as a release library across all six
  desktop targets (`aarch64` and `x86_64` × macOS, Linux GNU, Windows MSVC) with zero
  cross-linkers and zero external crates.
- No guest-visible behaviour or execution path changed, so no released title's run moves.

`GlobalLock` and `GlobalUnlock` answer fixed blocks allocated by `GlobalAlloc` (#158), where
`GMEM_MOVEABLE` remains a wall. As Microsoft's `GlobalLock` and `GlobalUnlock` pages document for
`GMEM_FIXED`, a live block hands back its own address from `GlobalLock` and `TRUE` (1) from
`GlobalUnlock`; a null, unknown, or freed handle fails with `ERROR_INVALID_HANDLE` (6) and returns
zero. LEGO Chess's chess engine wraps its working-table allocations in `GlobalAlloc(0x20, ...)` ->
`GlobalLock` (`0x0046e9fe`) and frees them with `GlobalUnlock` -> `GlobalFree` (`0x0046ea2e`).
`core/tests/shims.rs` tests live, null, unknown, and freed handles. No released title calls either
shim, so no released run moves.

The player's own bank is a declared input, and LEGO Chess's music is heard (#156, ADR-0008).

- `roster::GM16` names Roland's `GM16.DLS` where it lies loose, byte for byte the same with Roland's
  `GMREADME.TXT` beside it, on the four pressings of the roster that carry it: LEGO Racers'
  Masterpiece and original (`REDIST/DIRECTX6/DIRECTX/`) and Rock Raiders' (`DIRECTX6/DIRECTX6/DIRECTX/`),
  each an `EntryImage`, so it is read off the player's own rip as an entry image is, checked by the
  same sha256, `3229b09b…`, and never copied out.
- `roster::Entry::bank` says where a title's bank may be: `GM16` for LEGO Chess, nothing for the
  others. The roster holds it, as it holds a title's image and music pressing, since the launcher
  says where it is too, and `run::setup` looks it up there by the title's key so no title's own
  declaration changes. `roster::find_bank` takes the first place that holds it and otherwise says
  why none does: what each present rip held instead, or that none is ripped.
- A run's setup opens the synth on it after the install, and its log says which pressing it came
  from, or why it has no MIDI out. Without the bank #154's answers stand: no device, and no music.
- The README names the bank, where it lies, and Roland's licence note, which says it is licensed "for
  use with Microsoft operating system products only", with the fact that the copy played is the
  player's own; its reading is the player's.
- `tests/roster.rs` checks empty, missing, damaged and wrong-hash places without needing a disc, and
  finds the bank on any of the four pressings present (`testkit::any_rip`).
- No released title has a bank, so none opens a synth, and no released run moves.

LEGO Chess's MIDI stream runs as Windows runs it, on the declared Microsoft GS Wavetable SW Synth
(#154, ADR-0008). Each call answers as Windows does, from Microsoft's page for it and from Wine's
`dlls/winmm/winmm.c` and its conformance tests (`dlls/winmm/tests/midi.c`), which pass on Windows,
read as documentation. No title hears it yet: the machine has a MIDI out only with the player's bank
(#156), and without one `midiStreamOpen` and the mapper's caps answer `MMSYSERR_BADDEVICEID`, as
before.

- WINMM's `midiStreamOpen`, `midiStreamClose`, `midiStreamProperty`, `midiStreamOut`,
  `midiStreamRestart`, `midiStreamPause` and `midiStreamStop`, and `midiOutGetDevCapsA`,
  `midiOutSetVolume`, `midiOutPrepareHeader`, `midiOutUnprepareHeader` and `midiOutReset`, on the
  MIDI mapper (with device 0 also accepted by `midiStreamOpen`), with a `CALLBACK_FUNCTION`. A
  stream opens paused at Windows' 24 ticks
  and 500,000 µs a quarter note, and its callback hears `MOM_OPEN` there. It plays each `MIDIEVENT`
  at its delta in ticks at the tempo then in force, a tempo applying to the deltas after it; an event
  marked `MEVT_F_CALLBACK` calls back `MOM_POSITIONCB` with `dwOffset` on it, and a buffer played to
  its end `MOM_DONE`. Pausing keeps its place and does not count the pause. Stop turns off the notes
  it turned on, reset sends a note-off for every note and turns sustain off, and both return the
  queued buffers; close stops first, the stream still there for the callbacks of the buffers it
  returns, and ends with `MOM_CLOSE`. What no title asks for is a wall: another kind of callback,
  getting a property or setting the tempo, a SMPTE division, long and comment events, and a stopped
  stream played again.
- A short message goes to the synth at its event's own nanosecond, which the mix hears from there
  unless it has passed it. It usually has: the player wakes in the millisecond after the moment, at
  the next switch, where the machine mixes before the player plays, so a note is heard up to a
  millisecond and a quantum late, the same on every run.
- The mapper's caps are what Windows 11 answers for it, as Wine's tests print them there:
  "Microsoft MIDI Mapper", `MOD_MAPPER`, support 9 (`MIDICAPS_VOLUME | MIDICAPS_STREAM`).
- `midiOutSetVolume` sets the synth's own volume (`Synth::volume`, through `Mixer::midi_volume`):
  decibels straight from 0 dB at 0xFFFF to -96 dB at 0x0000, Microsoft's "interpreted
  logarithmically" with its silence at the bottom of sixteen-bit sound, the low word alone, since the
  mapper has no `MIDICAPS_LRVOLUME`. LEGO Chess sets 0xFA00 for a new player, 2.25 dB down. Unset,
  the synth's samples are as they were, so #155's pins hold.
- The callback runs on WINMM's player, a guest thread the machine makes with the first stream
  (`kernel32::spawn`, which `CreateThread` now goes through too), standing at a boundary entry of its
  own between callbacks. It is time-critical, as Wine makes its player: the scheduler gives it the
  processor ahead of every other thread whenever it can run, so it keeps it until it waits, as
  Windows runs such a thread on a machine of one processor, and a callback is never interleaved with
  the title's own code (ADR-0005). A call that hands it work makes its caller give way, so a stop's
  buffers are back before the caller goes on. A callback that waits for another thread, or ends the
  player, is a wall. Once a title has a stream it has two threads, so its waits park as ADR-0005's do
  rather than answering at once; LEGO Chess's only waits are on its own mutex, which it holds, and on
  the event its callback sets. No released title makes a stream, so none has such a thread, and no
  released run moves.
- `tests/midi.rs` plays two buffers written as LEGO Chess's converter writes them, on a bank of one
  ramp: every callback at its declared millisecond, a refill from inside the callback, the event it
  sets waking the main thread's wait, the notes heard from the frames they are due at, and the same
  every run; a pause of three tenths of a second holding every event back by that, its note sounding
  through it; stop, reset and close each returning both buffers before their caller goes on, the
  callback unpreparing each; `midiOutSetVolume` attenuation and silence; `midiOutReset` stopping
  playback, fractional tick remainders, and close-then-reopen on the reused player thread; the
  mapper's caps; and no device without a bank.

AVIFile decodes 16-bit Microsoft Video 1 (`CRAM`, `MSVC`, `WHAM`) streams (#152), written in
`msv1.rs` from the public 4×4 block format description (skip, solid 1-colour, 2-colour, and
8-colour 2×2-quadrant blocks in 15-bit RGB555, walked bottom-to-top and left-to-right).
`AVIStreamGetFrameOpen` accepts 16-bit Video 1 streams into plain bottom-up 16-bit or 24-bit DIBs
and refuses 8-bit Video 1 or non-multiple-of-four dimensions at a named wall. When expanding 15-bit
RGB555 pixels to a 24-bit BGR DIB, each 5-bit channel is shifted left three bits (`c << 3`),
matching Windows' `msvidc32` and turning LEGO Chess's six Advisors' `0x4212` RGB555 background into
the exact `0x808090` chroma key `Lego Chess.exe` checks at `0x00401242` and `0x00401373` (bit
replication would yield `0x848494` and leave an opaque box). `core/src/msv1.rs` tests synthetic
solid, skip, 2-colour, 8-colour, early-EOF, malformed-chunk and 16/24-bit DIB cases;
`core/tests/shims.rs` tests synthetic Video 1 streams through AVIFile (24-bit and 16-bit DIBs,
inter-frame skips, empty dropped frames, backward-seek clear, 8-bit/bad-dimension walls) and pins
first, middle and last 24-bit frames of all six Advisors (`admiral`, `baddude`, `blueking`,
`pirate`, `redking`, `sherriff`) from `MAIN.Z` against FFmpeg's `rgb555le` frames expanded with
`c << 3`. Indeo 5's decoder and DIB conversion are untouched, and no released title calls
`AVIStreamGetFrameOpen` or `AVIStreamGetFrame`, so no released run moves.

`TranslateMessage` makes `WM_CHAR` (#157), where it answered zero and posted nothing. A key going
down that types a character on the declared machine's keyboard has the character posted to the
title's window with the key-down's `lParam`, and the title's loop takes it next, ahead of the input
still waiting, as Microsoft's `TranslateMessage` page and its "Keyboard Input Overview" describe.
LEGO Chess's New Player form takes a name this way (0x0045C746).
- The keyboard is the US layout Microsoft publishes with its driver samples
  (`input/layout/kbdus/kbdus.c`, read as documentation): what each key types alone, with Shift,
  with Ctrl and with both, the keypad with Num Lock on. Ctrl with a letter types its place in the
  alphabet, which the layout leaves to Windows, as Wine's `test_ToUnicode` table finds on Windows.
  A key typed with Alt types nothing: Windows makes it a system keystroke and translates it into
  `WM_SYSCHAR`, which this machine does not make. Caps Lock is off, since no host sends it.
  `tests/shims.rs` tests the layout.
- The modifiers are the ones the title has read: the key messages it has taken off its queue, kept
  as `State::keys_read`, as Windows keeps a thread's own key state, and Wine changes it as each one
  is removed. So a Shift let go between a letter going down and the title taking it still types a
  capital. The digest hashes the keys read as held when there are any.
- The keypad's divide is `VK_DIVIDE`, which types a slash with Shift too, where it was the main
  keyboard's slash, which types a question mark.
- The answer is nonzero for every key message, as Microsoft documents. A `WM_SYSKEYDOWN`, which
  this machine never makes, is a wall. No title on the roster reads the answer: Rock Raiders' pump
  at 0x00478341, Hot Wheels' three at 0x00415461, 0x004154B0 and 0x00416001, Alpha Team's at
  0x004050F3, LEGO Island's at 0x00401B5B and LEGO Chess's at 0x0045C4F8 each drop it.

Every title on the roster translates its keys, so each now hears a `WM_CHAR` after a key that types
one. Alpha Team's window procedure asks a handler of its own (0x00404B10), which answers that it is
not its, and hands it to `DefWindowProcA`, as LEGO Island's does (0x00401E56); LEGO Chess's keeps it
for its text boxes; Hot Wheels' and Rock Raiders' were not read through. Every released run of a
route that presses Space, Escape or a letter moves, and no other: no flip, declared millisecond or
exit code moves, so the moves are the instructions the new messages cost, and, for the three runs
that end with a key read as held, Hot Wheels' two exits and Alpha Team's click at flip 600, the
digest's hash of it. The maintainer blessed the move on 2026-11-23. Each title's changelog lists its
own:
- Hot Wheels' two runs that exit, `first-race.route` at 13,462,673,313 where it ended at
  13,462,673,233, and the escape route at 583,424,210 where it ended at 583,424,170;
- LEGO Island's three that exit, each 288 instructions later: `first-save.route` past its end at
  13,071,130,021, `pizza-delivery.route` at 10,657,537,990, and the hand that closes its window in
  the Information Center at 4,998,299,127;
- and the digest alone of every other run of such a route: Alpha Team's five, Rock Raiders' two and
  LEGO Island's others, each ending at the same cursor, flips and declared time.

A clock spin is passed (#217), as ADR-0004 now allows, only when:
- two crossings in a row are the same clock read (`QueryPerformanceCounter`, `GetTickCount`,
  `timeGetTime` or `timeGetSystemTime`) from the same place, in the same declared millisecond;
- the processor is the same bit for bit at both, but for its time stamp counter, its floating-point
  unit compared by its bits;
- no byte of memory changed between them. The arena's new write watch tells, and `rdtsc` stops it,
  since the time stamp counter is a time the millisecond does not cover;
- no other thread can run, and the pass cost the clock its instructions and nothing more, in no
  more than the scheduler's quantum.

Then the pass between them is one the guest would make again, unchanged, until the clock reads the
next millisecond (`Clock::first_cycle_of`). So the machine retires those passes at once, its cycle
counter with them, and the schedule carries on from the read that sees the next millisecond as it
would have (`spin.rs`).
- It lands at exactly the cursor, declared clock and state that running every pass gives.
  `tests/spins.rs` holds each of the four reads to a traced run, which runs every pass, at a rate
  that is a whole number of kilohertz and at one that is not. So too a budget that runs out inside a
  spin, a stepped run, a run stepped on from its budget, a thread that wakes inside a passed spin, a
  spin after a NaN, a pass that writes a word twice, and a spin through a playing sound, whose mix
  must be the same.
- It never passes a spin that writes memory, changes a register, keeps or branches on the time stamp
  counter, polls for input, shares the processor with a thread that can run, runs longer than the
  quantum beside a sleeping thread, or costs more than its instructions, and a test holds each.
  Every check, taken out in turn, fails a test.
- A traced or stepped run runs every pass. A run that passed any says so at its end: `clock spins:`,
  and the instructions passed, a millisecond at a time.
- LEGO Chess's movies were 99.996 % such a spin. Headless on the Mac mini they now play at about 40
  times real time, where they played at 0.90, and the whole intro takes 7.1 s, where it took 67.6,
  ending on the same line, flips, digest and mix as before.
- No released title's committed route passes a spin, and no released run moves. The watch is armed
  only at a read that follows one like it and a pass that came back the same, so a title that never
  spins pays little. On the Mac mini, Hot Wheels' first race takes 0.67 % more of the host's
  instructions, 2,470.0 billion where it took 2,453.7, and Rock Raiders' first mission 0.34 % more,
  3,298.4 billion where it took 3,287.1.

A machine that runs ahead of real time between flips, as one passing clock spins can, is held to real
time at a window as it goes (#217), the same for every title:
- Each block of the mix is held to its own declared time before it is handed over
  (`Pace::hold_mixed`), so a wait passed between two flips hands its sound over as it plays, not all
  at once past what the device keeps waiting, where the oldest of it was dropped.
- A hold that has to wait has found the machine ahead, so the pace there is real time, worked out
  afresh from that hold. A slow stretch, such as LEGO Chess's silent holds that poll as they wait,
  no longer stretches the sound after it for up to half a second: at most the one block that
  straddles the stretch's end is stretched.
- A flip is held to its own time before its frame is shown, so a frame is shown no sooner than it is
  due.
- The feed's simulated runs hold all three: a movie after a slow hold, a wait passed between flips,
  and a slow run still stretched. A headless run holds nothing, so no run moves.

What a person does reaches a title between flips too, at its own input polls (#215), where it
reached it only at a flip. A poll is a call that reads the keyboard, the mouse or the message queue,
and every reader core has written is one: `PeekMessageA`, `GetMessageA`, `WaitMessage`,
`GetCursorPos`, `GetAsyncKeyState`, and DirectInput's `GetDeviceState` and `GetDeviceData`, on every
thread and whatever they filter, each counted from the flip before it; a test fails on any USER32 or
DirectInput call named neither a poll nor not one, so a reader written later is classed as it is
written. A route keys an event between
flips as `flip N poll K`, delivered as the guest makes its Kth poll after flip N, before the poll
answers, or at the first point after it if the guest makes fewer. An event with no `poll` is the
flip's own, as every route's are, so every route reads as it did, and an older build refuses a
`poll` line, naming the line. A run at a window asks the host at every flip, as before, and between
flips at the first poll once the frame has stood `ASK_BETWEEN_FLIPS_MILLIS` of declared time, its
waits included, since the flip or since the host was last asked; it holds the pace there, as at a
flip, and records what it heard at that poll. A run replaying a route drains the host there and
drops what it has, as at a flip. The released titles meet the threshold only where a frame waits or
loads: Rock Raiders in the frame that ends a load and in its boot splash, Hot Wheels in its boot and
its race's finish; Alpha Team and LEGO Island poll at the start of their frames, before they wait.
An ask is made before the host is told of a pointer the title put earlier in that frame, so a title
that puts its pointer back every frame and polls a threshold into one would lose the move heard
there: a ceiling marked in `Machine::poll`, which Alpha Team alone of the released titles could meet,
in a load that polls late, and which is to be lifted before any shorter threshold. The headless host
has nothing to hand over between flips, its events for a flip going at the flip, and holds no pace,
and the polls are counted outside the digest, so no committed route moves: the full gate's lines for
the released titles are the last full gate's. `route undelivered` counts the events after the last
flip or poll a run reached, so a run that presents nothing counts its first flip's too. The polls
cost Hot Wheels' first race, which in its menus polls every seventy-odd instructions, 0.18% more of
the host's instructions on the Mac mini, 2,453.46 billion where it took 2,449.16. LEGO Chess,
which after a movie Space skipped waits for Space to come up and presents nothing while it waits,
gets past the wait. ADR-0003 says how input is delivered and keyed, and CONTEXT.md has **Poll**.

DirectX 5's `D3DTBLEND_MODULATEALPHA` draws (#210): a texel's colour and alpha each multiplied by
the vertex's, as Wine's `d3d_device3_SetRenderState`, read as documentation, maps it onto the first
texture stage, d3dtypes.h numbering it 4; no public copy of Microsoft's own `D3DTEXTUREBLEND` page
was found. LEGO Chess draws its throne room with it. No released run reaches it: it walled before.
LEGO Island also drives a DirectX 5 device (#117), but its retained mode draws through
`d3d::rasterise` itself and never reaches `texture_map_blend`, and its disc tests on this change end
on the lines they ended on before. Rock Raiders draws through its `IDirect3DDevice3` face, and the
legacy blends are taken only on a DirectX 5 face, so it never reaches them either. The blends still
not written stay walls, each tested. The device's `dwTextureBlendCaps` still declares modulate
alone, less than it now draws, which #231 is to correct.

A triangle at one depth, its three depths equal bit for bit, has that depth at every pixel it covers
(#210): the steps of its depth are differences of equal floats, which are exactly zero, so a depth
interpolated linearly across it is that depth throughout. The edge planes it was interpolated
through left an ulp or so of noise across it, and two such triangles at one depth under
`D3DCMP_LESSEQUAL` were drawn as if one were a hair behind the other wherever the noise fell that
way: in a sweep of overlapping quads at a third, 215 of 270 placements lost pixels. LEGO Chess draws
its New Player form's letters as overlapping quads at a third, and a third of them did not draw.
Depths equal only to within rounding, and sloped triangles that share a plane, keep their noise, a
ceiling `raster::triangle` names; a triangle at an infinite depth, which no title draws, is
interpolated as before.
Rock Raiders, Alpha Team and LEGO Island draw through this rasteriser, and none of their runs moves:
their disc tests end on the lines they ended on before. Hot Wheels draws no triangle, locking its
surfaces and drawing into them itself, and its lines are unchanged too.

The Audio Compression Manager's streams decode Microsoft's ADPCM (#153): MSACM32's `acmStreamOpen`,
`acmStreamSize`, `acmStreamPrepareHeader`, `acmStreamUnprepareHeader` and `acmStreamConvert`, which
were walls, answer as Microsoft's pages and Wine's conformance test `test_prepareheader`, which
passes on Windows, have them, and otherwise as Wine's `msacm32` and `msadp32`, read as
documentation. A stream turns MS ADPCM into sixteen-bit PCM of the same rate and channels, a block
as many samples as its format's `wSamplesPerBlock` says (mmreg.h's `ADPCMWAVEFORMAT`). Another rate
or channels, or eight bits at a rate off `msadp32`'s own table, is `ACMERR_NOTPOSSIBLE`, no
conversion the codec makes. Its size counts a short last block whole, in a `DWORD` as `msadp32`
counts it. Its conversion decodes whole blocks, and a short last block at least its header long
after them, unless `ACM_STREAMCONVERTF_BLOCKALIGN` asks for whole blocks only, which Microsoft's
`acmStreamConvert` page says it alone does; it says how much of each side it used. A header with no
ADPCM is `MMSYSERR_INVALPARAM`, less than a block `ACMERR_NOTPOSSIBLE`, prepared with its reserved
argument set `MMSYSERR_INVALFLAG`, unprepared `ACMERR_UNPREPARED`, and an unknown stream
`MMSYSERR_INVALHANDLE`. These are walls: eight-bit PCM, since the one measurement of Windows' own
codec into eight bits, `test_prepareheader`'s, has a 512-byte block of 1,012 samples convert to
1,010 bytes, which is unmeasured on a real block; a destination shorter than one block's PCM, which
Windows there fills from the whole source; another format; a driver, a filter or a callback; asking
only whether it can, with `ACM_STREAMOPENF_QUERY`; no format, or no handle to answer; a block too
small for its own header, other predictors than Microsoft's seven, or a `wSamplesPerBlock` its size
does not hold; sizing from the destination; `ACM_STREAMCONVERTF_END`; and unprepare's reserved
argument set. `acmStreamClose` is not written, since LEGO Chess does not import it. The decoding is
`sound.rs`'s, Rock Raiders' movies' own, now open to the crate, its delta's steps now wrapping as
the codec's own integers do where corrupt data runs them past 32 bits, which only a debug build
answered otherwise; LEGO Chess's `sounds\chimp.wav`, its short block too, decodes through it to
FFmpeg's PCM byte for byte. Only LEGO Chess imports MSACM32, so no released run moves; a
`LoadLibraryA("MSACM32")` now loads it, which no released title asks for.

`SetCurrentDirectoryA` sets a directory by its full path, as `GetFullPathNameA` names it (#171). A
relative name is the current directory's child, where it was taken as the whole directory; one
rooted by either separator is on the current drive; a trailing separator goes; and `.` names where
it is: as Wine's conformance test `test_setdir` (checks 1, 6 to 9 and 11) finds Windows doing it.
Separators of either kind are one, however many, as Microsoft's "File path formats on Windows
systems" says `GetFullPathName` makes them. A directory the machine does not have fails with
`ERROR_FILE_NOT_FOUND`, or `ERROR_PATH_NOT_FOUND` when the one it would be in is missing too, as
Wine's `test_SetCurrentDirectoryA` finds Windows failing it, where it was taken as named: a
directory is there when the declared tree holds a file under it, the title wrote one there, or the
disc has it, which is how every title's installer leaves its folders. An empty name, or one of
nothing but white space, fails with `ERROR_INVALID_NAME`, where it was taken as an empty directory:
such a name has no full path (`test_GetFullPathNameA`), and Wine's `RtlSetCurrentDirectory_U`, read
as documentation, fails it so. That is Windows NT's answer; Windows 98's is unmeasured. Whenever it
fails the directory stays. These are walls, each named: a name through `..`, another machine's
(`\\server`, by separators of either kind), a drive's own current directory (`C:` alone or
`C:name`), a drive the machine does not have, a file's name, and a name with white space around it.
`CreateDirectoryA` still makes nothing, so a title that made a folder that way could not set it;
none on the roster does both. `Ctx::absolute`, through which every call that takes a path resolves
it, roots a name on `/` as on `\`, and joins a name to a drive's root with one separator, where it
made `C:\\x`: no released title's current directory is a root, and none names a path from `/`.
`GetVolumeInformationA` and `SetFileAttributesA` answer as before, the rest of #171. LEGO Chess
restores its saved directory at boot before it has saved it, an empty name, and sets its own
folders by name (#210), each of which its install holds. Hot Wheels sets its own install directory
by its full name, which resolves to itself and is there, and no other released title sets one: the
full `ci.sh` on this change moved none of their runs.

LEGO Chess's bumper draws through its DirectX plugin's Direct3D 2 device (#210). Four walls are
written. `IDirect3D2::CreateMaterial` makes a material, DirectX 5's `IDirect3DMaterial2` being
DirectX 6's slot for slot (d3d.h), and the material answers `IID_IDirect3DMaterial2` as the viewport
answers DirectX 5's. `IDirect3DTexture2::GetHandle` gives a texture one handle whichever device
asks, from the table materials' handles come from, counted from one, as Wine's
`d3d_texture2_GetHandle`, read as documentation, gives it. `IDirect3DTexture2::Load` fills a texture from one the same size,
as Wine's `d3d_texture2_Load` and its conformance test `test_texture_load_ckey` find Windows doing:
the pixels, copied as `memmove` copies so that two textures over the same memory load, and the
source's colour key where it has one, and the texture no longer claims `DDSCAPS_ALLOCONLOAD`;
another size is `D3DERR_TEXTURE_LOAD_FAILED`, and another format, a palette to copy or mipmaps are
walls. `IDirect3DDevice2::DrawIndexedPrimitive` draws `D3DVT_TLVERTEX` vertices, laid out as
`D3DFVF_TLVERTEX`, through DirectX 6's draw. A DirectX 5 device has no texture stages, so there the
draw takes the texture `D3DRENDERSTATE_TEXTUREHANDLE` names, and `D3DRENDERSTATE_TEXTUREMAPBLEND`'s
decal, copy and modulate, the last taking a texture's alpha when it has one and the vertex's when
not, as Wine's `d3d_device3_SetRenderState` maps them onto the first stage; on DirectX 6's and 7's
faces both stay walls. For vertices already on the screen, a DirectX 5 device's draw takes the
rectangle of a viewport the title described as where they are cut off, which LEGO Chess's plugin
needs and which was a wall; on a later device it still is. The other vertex types and blends, and a
handle that names no texture, are walls. A DirectX 5 device starts with specular highlights on, as
Wine's conformance test `test_lighting_interface_versions` finds Windows starting one. A device
before DirectX 7's starts `D3DRENDERSTATE_TEXTUREMAPBLEND` at `D3DTBLEND_MODULATE` (2), as Wine's
`d3d_device_init` starts it, where every device answered `D3DTBLEND_MODULATEALPHA` (4) under a
comment saying modulate; DirectX 7 has no such state, Wine's `d3d_device7_GetRenderState` refusing
it, so reading it unset there is a wall. Two answers that faked success now answer as DirectX does,
and a third does in one call of a family:
- `IDirectDrawSurface3::SetSurfaceDesc`, which changed nothing, points a surface in system memory at
  the memory the title hands it, with a width and pitch given together, its colour key and clipper
  going with it; it refuses what Wine's conformance test `test_set_surface_desc` finds Windows
  refusing, in the order Wine's `ddraw_surface7_SetSurfaceDesc` refuses it, and a height or a format
  is a wall;
- `GetColorKey`, which wrote nothing, answers the source key, or `DDERR_NOCOLORKEY` with the key it
  was handed left as it was;
- `GetSurfaceDesc`, which wrote no `ddsCaps` while saying it had, answers the caps a surface was made
  with and where DirectDraw put it: in video memory unless asked for system memory, the screen
  visible and its front buffer when it flips, the buffer behind it the back buffer, as Wine's
  `test_rt_caps`, `test_primary_caps` and `test_flip` find Windows answering. It and `Lock` answer a
  surface's own pitch. The rest of the family is #227's: every other description core writes, for
  `Lock`, `GetDisplayMode`, `EnumDisplayModes` and the movie stream's `GetFormat`, still claims
  `DDSD_CAPS` and writes none, and `GetCaps` still answers from whether a surface is the screen.

LEGO Chess's plugin makes each texture in system memory over the game's own pixels, loads it into a
copy in video memory that it drops unless the copy says it is there, keys the copy with what
`GetColorKey` answers, and draws it by its handle: its bumper presents. LEGO Island's runs move, in
memory alone: it keeps the descriptions `GetSurfaceDesc` fills, which now carry their caps, so each
of its runs ends at the same cursor, flips and declared time as before with another digest, a move
the maintainer blessed on 2026-10-22. Rock Raiders', Hot Wheels' and Alpha Team's runs do not move.
Alpha Team's `NiDirect3d.dll` branches on the caps at four sites (0x6F4569, 0x6F4714, 0x6F4A5F and
0x6F51AA, at its preferred base), and none of them turns another way on its path: the one its calls
reach tests `DDSCAPS_NONLOCALVIDMEM`, which no surface core makes claims, and the other three come
only from a renderer made over another, which it never makes. LEGO Island's changelog says how its
runs move; they are listed here:

- lego-island, `first-light.route` (held to 98,000,000): digest `1295656d` → `48366519`;
- lego-island, `first-ride.route` (held to 6,350,000,000): digest `d4014002` → `adbcf549`;
- lego-island, `first-save.route` (held to 6,030,000,000): digest `b299dcd3` → `99bf7904`;
- lego-island, `information-center.route` (held to 3,965,000,000): digest `d74f0616` → `91b5d02b`;
- lego-island, `maintainer-first-play.route` (held to 3,341,815,910): digest `6b5eca98` → `800b55b0`;
- lego-island, a run with no route (held to 5,571,000): digest `a39bb4f2` → `f25712dd`;
- lego-island, a run with no route (held to 320,000,000): digest `8d7906a2` → `bda2ef63`;
- lego-island, a run with no route (held to 4,490,000,000): digest `9b73e5c5` → `ff6fa6e1`;
- lego-island, a run with no route (held to 6,030,000,000): digest `b299dcd3` → `99bf7904`;
- lego-island, a run with no route (held to 6,067,000,000): digest `4fc2975c` → `c7157b94`;
- lego-island, a run with no route (held to 6,350,000,000): digest `d4014002` → `adbcf549`.

Its runs that close themselves, `first-save.route` and `pizza-delivery.route` past their ends and
the closing hand's, end on the same lines as before.

`AVIStreamGetFrameClose` lets a frame getter go, its DIB and its hold on its stream with it,
answering the references it has left, as Wine's `api.c` releases it, a getter holding its stream
from its open as Wine's `getframe.c` holds it; `AVIStreamRelease` counts a stream's reference out,
the file keeping the stream so that asking for it again hands out the same one, and one released
with none left answers -1 and is still there, as Wine's conformance test `test_default_data` finds
it on Windows; and `AVIFileExit` answers as `AVIFileInit` does (#151). All three were walls. LEGO
Chess lets go of each movie's streams and frame getter this way, its file staying open as the game
never releases one, and its intro's three movies play one after another. Rock Raiders imports
`AVIStreamRelease` and reaches a stream only through `AVIStreamOpenFromFileA`, still a wall; no
released title imports the other two; so no released run moves.

`AVIStreamStart` and `AVIStreamLength` answer a stream's start and length, and `AVIStreamRead` reads
a stream of fixed-size samples, a sound's, across its chunks, as Wine's `avifile.c`, read as
documentation, reads one, its samples its format's blocks as `AVIStreamInfoA` says; all three were
walls (#150). Both counts it writes are cleared first; a start before the stream or past its end is
`AVIERR_NODATA`; what is asked for stops at the stream's end; -1 samples is as many as fit, and with
no room is `AVIERR_BUFFERTOOSMALL`; no buffer is the counts alone; and what does not fit is
`AVIERR_BUFFERTOOSMALL`. A stream of whole chunks, a video's, is a wall, since no title reads one,
and so are a count below -1 and a stream whose chunks hold less than its header says, a damaged rip.
LEGO Chess fills its movies' DirectSound buffers through it, and with it reaches first light. No
released title imports any of the three, so no released run moves.

A DirectDraw surface's `GetDC` hands out a device context of its own, where it answered success and
wrote nothing, and `ReleaseDC` lets it go (#151): one at a time, whichever face of the surface is
asked, a second before the first is let go of answering `DDERR_DCALREADYCREATED` and leaving what it
was handed alone, as ddraw.h's error says and Wine's `surface.c`, read as documentation, answers; a
surface with none out answers `DDERR_NODC`, and one handed another DC than its own is a wall. With
its DC out a surface is locked, as Microsoft's GetDC page says: `Lock` answers `DDERR_SURFACEBUSY`,
as Wine's conformance test `test_getdc` finds Windows answering, and `Blt`, `BltFast` and `Flip`,
which nothing has observed, are walls. A screen with nothing behind it has its frame when its DC is
let go of, as letting go of a lock of the whole of it does. What draws on a DC is DrawDib:
`DrawDibOpen` hands out handles counted up from one, as Wine's `drawdib.c` counts them, so one let
go of is not handed out again; `DrawDibClose` answers TRUE, or FALSE for a handle it never gave; and
`DrawDibDraw` draws a plain bottom-up 16-bit DIB whole onto a surface's DC, each pixel repeated as
many times over as the destination is the DIB's size, from the bottom row up, and what falls off the
surface left out. Each channel is widened by repeating its top bits and narrowed to the surface's,
as Wine's DIB engine does, so a 555 DIB's green gets its sixth bit on a 565 surface from its own top
one. A top-down or empty DIB is refused with FALSE, as Microsoft's DrawDibDraw page says it "will
not draw inverted DIBs" and Wine's `test_DrawDib_sizeimage` finds an empty one refused; a flag, a
part of a DIB, a scale that is not whole, a codec, another depth or an indexed surface is a wall.
LEGO Chess's DirectX plugin draws each movie frame this way. No released title imports MSVFW32, and
no released title takes a surface's DC on a committed route: the full gate on this change moved none
of the released titles' runs.

LEGO Chess's movies give up their frames (#151). `AVIStreamGetFrameOpen` makes a getter of a video
stream's frames as a plain bottom-up DIB in the format the title asks for, 16 bits a pixel, which is
555, for its movies, or 24 for its Advisors (#152), at the movie's own size, the header asked for
followed by its bits; a stream that is not video gets none, as Wine's `getframe.c`, read as
documentation, refuses one, and the decompressor's own format, another depth or size, a colour
table, or a codec other than Indeo 5 is a wall, as is a picture too big for memory, which only a
damaged rip's header could say. `AVIStreamGetFrame` decodes the pictures in order up to the one
asked for, a null picture keeping the last, and answers the DIB with it in: the same frame again is
the DIB as it is, one before the last decoded starts again from the first, and one before the
stream's start or past its last is none. LEGO Chess's logo's frames 5 and 8 are the DIBs a reference
decoder's pictures give, FFmpeg 9.0.1's, turned into DIBs the same way, as the test beside them
records. Every open of a movie file shares one copy of its bytes, since the game opens each movie
anew every time it plays it and never lets go of a file. The conversion from YUV to RGB, BT.601 in
the integer form Microsoft documents and not FFmpeg's, moved from DirectShow's movies into `iv50.rs`
as `Picture::rgb`, unchanged, and both now call it; NOTICE says it is this project's own. Rock
Raiders imports both calls and reaches a stream only through `AVIStreamOpenFromFileA`, still a wall,
so no released run moves; its movies' tests and its first-light route end as they did.

LEGO Chess's movies open through AVIFile's C API (#151), which past `AVIFileInit` was a wall.
`AVIFileOpenA` opens a movie to read it: one that is not there, or on the empty floppy drive, gives
`AVIERR_FILEOPEN`, which LEGO Chess takes as a movie to skip; one that is there and is not a movie,
or holds fewer streams than its header says, which Wine fails to load, stops the run at its name, as
a damaged archive member does (#169); one that holds more is read, as Wine reads it, as the streams
its header counts; another mode, or a handler named, is a wall. `AVIFileInfoA` answers the main
header's flags and stream count and works the rest out from the streams as Wine's
`AVIFILE_UpdateInfo`, read as documentation, works them out: the first stream's time scale, the
longest stream's length in it, the streams' bytes a second added up, the biggest chunk as the
buffer, and the video's biggest frame as the size. `AVIFileGetStream` hands out a stream by number,
or by number of its type, the same object each time with a reference more, and `AVIERR_NODATA` past
the last. `AVIStreamInfoA` answers the stream's header as vfw.h's 56-byte `AVIStreamHeader` lays it
out but for three things it answers as Windows does: a video stream's empty frame is filled from its
picture's size, as Wine fills it, which twenty of LEGO Chess's twenty-two movies need, their
headers' 32-bit edges being empty read as 16-bit ones; the buffer is the stream's biggest chunk, and
none for none, as Wine's conformance test `test_default_data` finds Windows answering; and a sound's
sample size is its format's `nBlockAlign`, whatever its header says, as `test_ash1_corruption` finds
it. A sound's time scale is its header's, which Wine changes and no test observes Windows changing.
A stream is named by its `strn`, as its bytes, or as AVIFile names one with none, by the file's
name, its kind and its number of that kind, as two of LEGO Chess's movies kept the names their
authors' AVIFile gave them, `LMI.avi Video #1` and `WL1lose.avi Audio #1`. `AVIStreamAddRef` counts
a reference, and `AVIStreamReadFormat` answers how long the format is, or copies as much as fits and
says `AVIERR_BUFFERTOOSMALL` when that is not all of it, as Wine's does. The file and its streams
are COM objects whose own tables are walls. `avi.rs` reads the rest of each stream's header, its
name and the main header's flags and stream count, and what it read before is unchanged. Rock
Raiders imports `AVIStreamInfoA` and reaches a stream only through `AVIStreamOpenFromFileA`, still a
wall, so no released run moves.

A message sent to the title's own window is handed to its procedure, whose answer is
`SendMessageA`'s, as Microsoft documents a send to a window of the calling thread; it was a wall
(#147). `InvalidateRect` of the title's window, or of no window, which on this machine is the same
window, marks it with its background to erase, the erase forced with no window as Microsoft and Wine
say; a rectangle of its own is a wall. A marked window is handed `WM_PAINT` by `PeekMessageA` and
`GetMessageA` once nothing else waits, in `GetMessage`'s documented order, and sent it by
`UpdateWindow`, until `BeginPaint` unmarks it and says to draw the whole client area, its background
still to erase if that was asked for, or `DefWindowProcA` is handed the `WM_PAINT`, which validates
the region as Microsoft's WM_PAINT page says. `BeginPaint` with nothing due says nothing is to be
drawn, and `EndPaint` answers success. `UpdateWindow` of no window, or of one the machine never
made, fails, as Wine's conformance tests record Windows 9x doing; the title's window, the desktop and
a dialog and its controls are the machine's. A send from a thread other than the window's, or to a
window with no procedure here, is a wall. `GetMessageA` takes what `PeekMessageA` would and answers
zero for `WM_QUIT` and -1 with nowhere to put it; with nothing waiting it is a wall, since a wait for
a timer's hour or another thread's post is not written yet. A paint due and a screen saver said to
run are part of the machine's digest when there is one. LEGO Chess makes all of these calls. No
released title invalidates its window, sends its own window a message, or calls `BeginPaint`,
`EndPaint` or `GetMessageA`; Rock Raiders calls `UpdateWindow` and every title calls `PeekMessageA`,
and with no window marked both answer as before, so no released run moves. Windows also activates
and paints a window as it is first shown; #203 is to do that.

`IDirect3DDevice2::EnumTextureFormats` lists the declared device's texture formats, the ones
`IDirect3DDevice7`'s lists, each in a whole `DDSURFACEDESC` as d3dtypes.h has its callback take one
and Wine fills it, and `IDirect3D2::CreateViewport` makes a viewport, DirectX 5's face being DirectX
6's slot for slot as far as it goes, as d3d.h declares them; both were walls (#148). The viewport
answers to DirectX 5's and DirectX 3's names too. With no callback, either device's
`EnumTextureFormats` is refused with `DDERR_INVALIDPARAMS`, and a material, light or viewport made
inside another object with `CLASS_E_NOAGGREGATION`, as Wine refuses them. LEGO Chess's DirectX plugin
calls both as it makes its device; no released title passes either refusal's argument, so no
released run moves.

A title's window is told it is the active window and has the keyboard as Windows tells it (#196):
after `WM_ACTIVATEAPP` come `WM_ACTIVATE` with `WA_ACTIVE` and no window deactivated, and then
`WM_SETFOCUS` with no window that lost it, in Microsoft's order, posted with it as the window is made.
Windows sends them from `ShowWindow`, or a visible `CreateWindowExA`, with `WM_NCACTIVATE` between the
first two; core posts them at `CreateWindowExA`, and #203 is to send them. LEGO Chess's message pump
waits for `WM_SETFOCUS` (0x0045C626), and on a probe it spun for ever without it. Every
released title's window procedure now hears two more messages, so every run of a released title that
makes its window moves, a move the maintainer blessed on 2026-10-22; each title's changelog says how,
and the runs are listed here:

- hotwheels-stunt-track-driver, `first-race.route` (exits): cursor 13,462,673,372 → 13,462,673,233, digest `ca24a96c` → `4fc367fd`;
- hotwheels-stunt-track-driver, `hot-wheels-escape.route` (exits): cursor 583,424,086 → 583,424,170, digest `1448924f` → `929363be`;
- hotwheels-stunt-track-driver, a run with no route (held to 100,000): digest `c4eae7cb` → `bf20b397`;
- hotwheels-stunt-track-driver, a run with no route (held to 2,200,000,000): digest `cb56737b` → `638636af`;
- lego-alpha-team, `click-the-intro-300.route` (held to 600,000,000): digest `a71a75d7` → `f486059d`;
- lego-alpha-team, `click-the-intro-600.route` (held to 600,000,000): digest `00c2662d` → `2f818e98`;
- lego-alpha-team, `first-mission.route` (held to 23,400,000,000): flips 8,054 → 8,053, ms 331,967 → 331,938, digest `1b7d7349` → `dea8cb61`;
- lego-alpha-team, `menu.route` (held to 5,400,000,000): ms 139,471 → 139,469, digest `f34a24e8` → `ec35babc`;
- lego-alpha-team, a run with no route (held to 1,100,000,000): digest `1a08e772` → `da79732d`;
- lego-alpha-team, a run with no route (held to 2,000,000): digest `e37ef1e8` → `e8b61fb0`;
- lego-alpha-team, a run with no route (held to 600,000,000): digest `198706f8` → `80389ef1`;
- lego-alpha-team, `select-adventure.route` (held to 14,000,000,000): flips 4,454 → 4,455, ms 214,104 → 214,138, digest `87323b4d` → `9dbb0afd`;
- lego-island, `first-light.route` (held to 98,000,000): digest `e753e559` → `1295656d`;
- lego-island, `first-ride.route` (held to 6,350,000,000): digest `e08fb5c1` → `d4014002`;
- lego-island, `first-save.route` (held to 6,030,000,000): digest `cbc3eae1` → `b299dcd3`;
- lego-island, `first-save.route` (exits): cursor 13,071,129,606 → 13,071,129,733, digest `cce081c4` → `a8f05879`;
- lego-island, `information-center.route` (held to 3,965,000,000): digest `93ac795c` → `d74f0616`;
- lego-island, a run with no route (held to 320,000,000): digest `99ce5f30` → `8d7906a2`;
- lego-island, a run with no route (held to 4,490,000,000): digest `0851a9b9` → `9b73e5c5`;
- lego-island, a run with no route (held to 5,571,000): digest `da6e3642` → `a39bb4f2`;
- lego-island, a run with no route (held to 6,030,000,000): digest `cbc3eae1` → `b299dcd3`;
- lego-island, a run with no route (held to 6,067,000,000): digest `f108f2f8` → `4fc2975c`;
- lego-island, a run with no route (held to 6,350,000,000): digest `e08fb5c1` → `d4014002`;
- lego-island, a run with no route (exits): cursor 10,657,537,575 → 10,657,537,702, digest `3d447ba5` → `d65d776f`;
- lego-island, a run with no route (exits): cursor 4,998,298,712 → 4,998,298,839, digest `038ebb6b` → `895d407d`;
- lego-island, `pizza-delivery.route` (exits): cursor 10,657,537,575 → 10,657,537,702, digest `3d447ba5` → `d65d776f`;
- lego-island, `route` (exits): cursor 4,998,298,712 → 4,998,298,839, digest `038ebb6b` → `895d407d`;
- lego-rock-raiders, `click-the-intro.route` (held to 150,000,000): digest `b1c537af` → `0447048c`;
- lego-rock-raiders, `first-light.route` (held to 8,010,000,000): digest `322ab7a0` → `704487cb`;
- lego-rock-raiders, `first-mission.route` (held to 14,500,000,000): digest `4888818a` → `27d40ec6`;
- lego-rock-raiders, `level-select.route` (held to 8,035,000,000): digest `49e57d12` → `df1a6347`;
- lego-rock-raiders, a run with no route (held to 400,000,000): digest `6714ce40` → `e76c516d`;
- lego-rock-raiders, a run with no route (held to 61,000,000): digest `3e8b8a38` → `f53901b5`;
- lego-rock-raiders, `skip-the-intro.route` (held to 150,000,000): digest `315d3880` → `b0ab6356`
  (Rock Raiders' run held to 100,000 makes no window and is unchanged);
- lego-island, `maintainer-first-play.route`: replayed to 3,341,815,910 where it was held to 3,341,815,783, the same moment 127 instructions later, ending at flip 10,667 and 274,650 ms as before; digest `db7bd6b2` → `6b5eca98`.

LEGO Chess's run ends at the same wall, cursor and flips, its digest `52a34d88` → `7fcea288`: its
window's three messages wait in its queue, which it has not reached.

`GetSystemDefaultLangID` answers the declared locale, US English, as `GetUserDefaultLangID` does
(#147). `SystemParametersInfoA` answers two more actions, only as LEGO Chess asks them, anything
else being a wall: `SPI_GETSCREENSAVEACTIVE`, no, since the declared machine has no screen saver,
and fails with nowhere to say so, as Wine's `get_bool_entry` does; and `SPI_SETSCREENSAVERRUNNING`,
which Windows 9x keeps and programs of the time set to switch off Ctrl+Alt+Del and Alt+Tab
(Microsoft's Q161133), each setting handing back the one it replaced, which is [inferred]: Q161133
never reads it. `timeGetSystemTime`, which LEGO Chess was seen on a probe to wait on, hands
`timeGetTime`'s milliseconds back in an `MMTIME` that says `TIME_MS`; one smaller than twelve
bytes, or none, is a wall that names it. LEGO Chess calls all of them; no released title calls any,
so no released run moves.

DirectDraw's `IDirectDraw2` face, the one a DirectX 5 program holds and here the object's
`IDirectDraw4` face, hands out `IDirect3D2`, as any face of a COM object answers for any other and as
Wine's ddraw hands Direct3D 2 out from every DirectDraw face, where it refused it (#148). LEGO Chess's
DirectX plugin asks it for Direct3D 2 and calls `EnumDevices` through what it is given without
looking, so a refusal was an access violation in the plugin. LEGO Island asks the `IDirectDraw` face,
which already answered; no released title asks this one, so no released run moves.

The 3D listener's `SetAllParameters` sets every value at once from a `DS3DLISTENER`, as dsound.h
lays it out, dropping the velocity as `SetVelocity` does, where it was a wall (#149). Another size,
an apply that is neither immediate nor deferred, a distance factor not above zero, or a rolloff or
Doppler factor outside 0 to 10 is refused with `DSERR_INVALIDPARAM` and nothing kept, as DirectSound
documents. LEGO Chess sets its listener so at boot; Alpha Team's A3D (`a3dapi.dll` 0x1001FD34)
would, on a renderer none of its runs takes.

The `cdaudio` device opens for a title with no Red Book audio, its own data disc in the drive, as
Windows' mcicda opens it, where it was not ready: Wine's conformance tests, which run on Windows 9x
too, record the open succeeding with a data-only disc and with none (#149). Both faces of MCI share
the one open. `mciSendCommandA`, MCI's numeric face, opens the device by its type's number, the
first of its type only, sets its time format to tracks, minutes, seconds and frames, and stops and
closes it, each logged with its answer as the string face's are; any other command, device or
flag is a wall. Rock Raiders, the only released title that imports MCI, has Red Book audio, so its
open is the one it always was.

`midiStreamOpen` is refused with `MMSYSERR_BADDEVICEID`, the mapper included: the machine has no
MIDI port until #154 gives it one, and none after that whenever the player has no bank (ADR-0008).
`midiStreamOpen`'s own page lists that answer, and Wine's conformance tests record every Windows they
ran on answering it for the mapper with no port. A stream of other than one device, or with nowhere
to say which, is refused as invalid; each answer is logged. LEGO Island's `LEGO1.DLL` imports it,
behind a `midiOutGetNumDevs` that is still a wall, so no run reaches it.

No released run reaches any of the three, so none moves.

`GetClipCursor` answers where the pointer is confined, as Microsoft documents (#147): the
rectangle `ClipCursor` was last given, which it now keeps, cut to the screen, until it is given none,
or the declared screen, in the mode it is in, while it is confined nowhere; with nowhere to write it,
it fails. The host's pointer is not held to the rectangle; no title has been seen to need it. LEGO
Chess's framework asks as it starts. Only LEGO Chess imports `GetClipCursor`, and nothing reads what
`ClipCursor` keeps but it, so no released run moves.

`DestroyWindow` tells the window it destroys, as Microsoft documents, where it answered success and
told it nothing (#140). Its procedure is sent `WM_DESTROY` and then `WM_NCDESTROY`, whatever it
answers the first, through the program `call_each` writes, which no longer has to stop at a zero.
The window is then gone: the messages waiting for it go too, what a person does reaches no window,
`GetActiveWindow` answers none, and destroying it again, or destroying any other handle, fails with
`ERROR_INVALID_WINDOW_HANDLE`. `DefWindowProcA` destroys the window it is handed `WM_CLOSE` for, as
Microsoft documents; every other message is still answered zero. The messages Windows also sends a
window that is shown and in front as it is destroyed, hiding it and taking the front from it, are
not sent. Hot Wheels destroys its window as it exits, and its procedure hands both messages to
`DefWindowProcA`, so its two runs that exit move, each by 54 instructions, their flips, declared
milliseconds and exit code unchanged, a move the maintainer blessed on 2026-10-21:

- the escape route from 583,424,271 and `920d063e…` to 583,424,325 and `8bf567a9…`;
- `first-race.route` from 13,462,722,489 and `ca178965…` to 13,462,722,543 and `d48d8761…`.

LEGO Island's procedure does the same as it leaves after its credits, so each of its runs that
close themselves ends 58 instructions later, its flips unchanged: `first-save.route`, past its end,
at 13,071,129,606, and `pizza-delivery.route` at 10,657,537,575. No other title destroys its window
in a committed run.

Closing the window reaches the title as Windows delivers a close, and the run ends when the title
leaves (#140). It stopped the run at once, and the title was never told. This changes how every
title's run ends when a person closes it.

- Every way a person closes a run is counted, `Host::closes` where `Host::closed` said whether:
  the window's close button, Command-Q or -W, Control-Q, Alt-F4, and the run's terminal or
  console, by Ctrl-C, Ctrl-Break, `SIGTERM` or `SIGHUP`. A key held down repeating is one close.
- The first is delivered at the next flip, as everything a person does is, as `Input::Close`. A
  route records it as `flip N close`, so a replay closes where the person did. The title's window
  is told as Alt-F4 tells it: `WM_SYSCOMMAND` with `SC_CLOSE` is posted to it (Wine's `defwnd.c`,
  read as documentation). The close button sends the same from `DefWindowProc`, inside the
  title's own dispatch of the click.
- `DefWindowProcA` answers `SC_CLOSE` by sending the window `WM_CLOSE` and answering what the
  window answers, as Microsoft documents and `defwnd.c` does.
- `PostQuitMessage` posts `WM_QUIT` with its code, where it only said so in the log. The thread
  that asked is handed it after whatever was posted to it and before what a person did, whatever
  it filters for, as Wine's message server hands it out (`server/queue.c`). The digest hashes a
  quit asked for and not taken, only when there is one.
- The run ends with the title's own exit. A second close still stops it at once, wherever it is,
  as `stopped by the person`, as one close did before; so does a close before the title has a
  window to tell. A title told that has not left within `machine::CLOSE_GRACE_MILLIS`, five
  declared seconds, is stopped then, on the same instruction every time its route is replayed. A
  third Ctrl-C or signal is let be what it was, as the second was. A closing Windows console,
  which Windows gives five seconds, stops the run as a second close does.

What each title does when it is closed, by a route that replays a committed one and closes:

| Title, where | What it does | Instructions to its exit |
|---|---|---|
| Rock Raiders, in its first mission (flip 9000) | stops its CD audio, exits 0 | 1,500,815 |
| Rock Raiders, at its level select (flip 5000) | its window is destroyed and it runs on; stopped after the grace | never exits |
| Hot Wheels, in its first race (flip 1020) | exits 1, as it always exits | 13,835,553 |
| Alpha Team, at its menu (flip 2300) | posts its quit, writes `Game.txt`, shuts NetImmerse down, exits 0 | 1,062,668 |
| Alpha Team, in its first mission (flip 8000) | the same | 973,232 |
| LEGO Island, in its Information Center (flip 5200) | posts itself `WM_CLOSE`, exits 0 | 6,669,503 |
| LEGO Island, signed as LEGO with Pepper picked (flip 9000) | the same, saving `G0.GS` and `History.gsi` first | 8,858,439 |

None asks a question. Rock Raiders' front end runs loops of its own that never look at the flag its
window procedure sets on `WM_CLOSE` and `WM_DESTROY` (OpenLRR's `Main_WndProc` and its note on the
close button within menus, read as documentation), so on Windows too it would run on with its
window gone. No committed route closes, so no run moves: LEGO Island's
`maintainer-first-play.route` was recorded before a route kept a close, and ends on its budget. The
stub run `stopped.rs` stops with a signal, which has no window to tell, now logs the close as
`input flip 0 Close` and ends where and as it did. The Windows and X backends are compiled and
clippy-clean, not run.

Minimizing the window on macOS, or hiding the application, is not a close (#173). The macOS backend
took its window leaving the screen for its close button, and a window minimized into the Dock, or
one of an application hidden, is off the screen too (Apple's `isVisible`, `isMiniaturized` and
`hide(_:)`). So, read from the code (#6, `a8fc43a`) and Apple's documentation rather than seen at a
window, minimizing the window stopped the run in 1.0.0 and 1.1.0, ended the "close the window to
end" wait after a run, and with the close above would have told the title `SC_CLOSE`. Windows
answers a minimize with `SC_MINIMIZE` and an iconic window, never `SC_CLOSE`, and the Windows and X
backends never took one for a close. The decision was `host::closed`, with no `cfg`, until the
window's delegate took the close from the button itself (#183, below). On a minimize or a hide the
title is told nothing new: no `SC_MINIMIZE`, `WM_SIZE`, `WM_ACTIVATE` or `WM_ACTIVATEAPP` FALSE,
only the keys and buttons it was told are held let go, as for any window not in front (#141), and it
runs on, unseen but heard. Telling it as Windows does is #177. No route moves; a headless run has no
window.

The macOS window outlives its close button (#175). A window made in code is released when it closes,
by default (Apple's `isReleasedWhenClosed` and `close()`), and the backend went on asking it about
itself, setting its caption and drawing into its layer while the title left (#140), and closed it
again as the run ended: messages to an object it may have freed, read from the documentation and
never seen to crash. The window is now kept until the process ends, and since #183, below, its
button no longer takes it off the screen. No route moves; a headless run has no window.

A window on another Space is not in front on macOS, so switching Spaces frees a captured mouse, as
losing the front does, and the title hears no key there (#176). #141's front read whether the
window was on the screen, not minimized, and of the active application. A window on another Space
is still on the screen (Apple's `isVisible`), and, reasoned from the code and not seen at a window,
its application stays active when nothing else takes the front there: the mouse would stay
captured with the window out of sight, the pointer hidden and frozen, or LEGO Island's pulled back
into the window's place. The front now also asks `isOnActiveSpace`; `isVisible` and
`isMiniaturized` stay, because Apple says a window off the screen answers where it would go. The
decision is `host::in_front`, with no `cfg`, so every host's tests hold it. Coming back to the
Space, like coming back to the front, captures nothing until a click (#141). A key pressed while
the window is not in front is the title's no more, on every backend (`Capture::key`), as Windows
and X send such a window no keys and macOS sends them on to an application whose window is on
another Space, where Escape would have reached Hot Wheels and quit it; on macOS the keys the title
holds are let go when the window leaves the front as well as when it stops being the key window.
A known limit: Mission Control, App Exposé and Show Desktop change no Space, so a captured mouse
stays captured over them until Escape. No route moves; a headless run has no window.

The Dock's Quit, a logout and a restart reach the title on macOS as the window's close button does
(#178): one close, delivered at the next flip as `SC_CLOSE` and recorded in the route, and the run
ends when the title leaves, a second close or the five declared seconds still stopping one that
never does (#140). So does any other request to quit the application, Activity Monitor's Quit and
AppleScript's `quit` among them. They ask the application `terminate:`, which, with no delegate,
ended the process at once, inside `pump`'s `nextEventMatchingMask:`: the title was never told, the
run wrote no `end` line, and LEGO Island didn't save. The application now has a delegate, a class
made at run time through the Objective-C runtime's own `objc_allocateClassPair`, `class_addMethod`
and `objc_registerClassPair`, with no crate. Its `applicationShouldTerminate:` counts the close and
answers `NSTerminateCancel` (Apple, `terminate(_:)`), as SDL and GLFW do, read as documentation:
`NSTerminateLater` would have AppKit run its own loop until answered, with the title stopped inside
it. So a logout or a restart is interrupted, and the person asks for it again once the title has
left; that holds in the "close the window to end" wait too, which a quit ends. A logout also asks
the run's terminal to quit, and a terminal that hangs up the run without asking is a second close,
which stops the run before LEGO Island has saved. It also answers
`applicationSupportsSecureRestorableState:` yes, which spares the terminal AppKit's warning about
an application delegate without it. No route moves; a headless run has no window.

A Windows sign-out, shutdown or restart reaches the title as a close too (#184), where the window
handed `WM_QUERYENDSESSION` to `DefWindowProcW`, whose TRUE let the session end the process with the
title untold. The window holds the session while the title leaves: Windows wants the answer at once
and the title runs on the window's own thread, so it is held as Microsoft documents for a window
that learns only then that it must block ("Application Shutdown Changes in Windows Vista", usage
model 2): `ShutdownBlockReasonCreate` with "The game is closing.", then FALSE. Windows shows that on
its screen while the title leaves; whether the screen then goes on by itself or the person asks
again, Microsoft does not say. The process asks to be asked first, at the top of the "Application
reserved first shutdown range" (`SetProcessShutdownParameters(0x3FF, 0)`; every process starts at
0x280, and Windows asks from high to low), so its FALSE ends the round before Windows Terminal, the
default console on Windows 11, is asked: Terminal quits at any `WM_ENDSESSION`, and the title's
console closing would be a second close that stops it before it is done.

It is at most one close, unlike a quit on macOS, which counts each (#178): a sign-out after the
close button, Alt-F4 or Ctrl-C, or one asked again while the title leaves, adds none. Alt-F4 now
counts in `CLOSE_ASKED` with the rest, where the window kept its own count. The Restart Manager's
session end (`ENDSESSION_CLOSEAPP`) is answered TRUE and counts no close, as `DefWindowProcW`
answered it: its guidelines ask for TRUE and for shutting down only at `WM_ENDSESSION`, and the
`WM_CLOSE` it sends a window still there is the close. A forced one, Windows' "Sign out anyway",
"Shut down anyway" or "Restart anyway", or `ENDSESSION_CRITICAL`, cannot be refused: Windows ends
the process, the title untold and the run with no `end` line, as a forced sign-out (`EWX_FORCE`)
ends it without asking at all. So does a run that has answered none of its window's messages for
five seconds when the session ends, such as one loading without pumping: Windows ends it as not
responding, untold, as before. The reason is withdrawn as the window goes. `WM_ENDSESSION` is still
`DefWindowProcW`'s, which answers zero as documented.

X is unchanged. systemd-logind's `KillUserProcesses=` is set when systemd is built: upstream's
default is yes, but Debian, Ubuntu, Fedora and Arch build it no. There a logout sends no `SIGTERM`,
and the display going away ends the process through Xlib's I/O error handler, the title untold and
the run with no `end` line. Where it is yes, the session's `SIGTERM` is a close (#140), and so is a
desktop's `WM_DELETE_WINDOW`. A session manager asks only a client registered through libSM. No
route moves; a headless run has no window.

The macOS backend drains an autorelease pool every pump, so a long run no longer grows the
process's memory (#180). AppKit opens a pool for each turn of its own event loop, and this backend
runs its own loop, so nothing AppKit handed back autoreleased was ever let go: each event, the run
loop mode and the date asked for every pump, and each new caption (Apple, `NSAutoreleasePool`:
"If a pool is not available, autoreleased objects do not get released and you leak memory").
Measured by probes making the same calls with no window, a million times each: a pump with no
event leaked about 58 bytes, each event fetched about 0.9 KB, and the first probe's loop, which
also makes a caption and asks for the screens, about 110 bytes a pass, growing by 109 MB with no
pool and by 1.4 MB with one drained each pass. A pump runs twice a flip. The pool is an
`NSAutoreleasePool`, opened at every way into the backend that reaches AppKit, `open`, `pump`,
`present`, the title's pointer calls, opening and closing the audio, and `Drop`, and drained as
each ends, a panic's unwinding included. What the backend keeps across pumps is owned, never only
autoreleased: the window (#175), the delegate (#178), the title's pointer shape and the window's
layer, now retained by the backend; the view is the window's and the application AppKit's. No
route moves; a headless run has no window.

The macOS window's close button reaches the title through the window's delegate, and the window
stays on the screen until the title has left, as on Windows and X (#183). The button took the window
off the screen at once, and the backend read the close from the window leaving it (`host::closed`,
#173), so the title saved and left with nothing in view, and a close of a minimized window, AltTab's
for one, was never counted (reasoned, not seen at a window). #178's delegate now also answers
`windowShouldClose:`, which AppKit asks when the person clicks the button (Apple,
`windowShouldClose(_:)`): it counts one close and answers NO, so the window stays; it is closed when
the run ends, by `close`, which asks no one (Apple, `close()`). `host::closed` is gone, and nothing
reads a close from the window's visibility. Command-W and Command-Q are counted by the key as
before, so a key held down repeating is still one close, as Windows counts Alt-F4 by the key; sent
on as `performClose:`, each repeat would be another close, and the second would stop the run. No
route moves; a headless run has no window.

The macOS backend sends every message that answers a rectangle through `objc_msgSend_stret` on an
Intel Mac, so an x86_64 build no longer reads garbage or crashes the first time it asks a window,
its view or a screen for its `frame` (#186): at any press on the window, a move of the free mouse
over it, a steered move, or a pump with the mouse kept. On x86_64 a struct larger than two
eightbytes comes back through memory its caller provides (System V AMD64 psABI, 3.2.3), and the
runtime has its own entry point for that (`objc/message.h`); an `NSRect`, four doubles, is one, and
an `NSPoint` or `NSSize`, two, is not, nor is a `CGFloat`, as Apple's own compiler sends them. Arm64
has no such entry point and `objc_msgSend` answers any struct, so nothing changes on Apple silicon.
The decision is `host::stret`, with no `cfg`, so every host's tests hold it, and every struct-
returning send goes through `Runtime::send_struct`, which answers zeros for a message to nil, as
Apple's compiler does, since the stret entry point writes nothing then (the first screen is nil with
no display). Built and clippy-clean for `x86_64-apple-darwin`; core's `host` tests, the struct
path's included, pass as x86_64 under Rosetta on an Apple silicon Mac, and fail without the stret
entry point (a crash) or without the nil answer. Not played on an Intel Mac. No route moves; a
headless run has no window.

`WM_TIMER` is sent as Windows sends it (#138), where `SetTimer` and `KillTimer` answered success
and no timer ever came due. It moves Hot Wheels' runs, which are released and signed off, a move
the maintainer blessed on 2026-10-21; a person drives Hot Wheels' first race again before its next
release (golden rule 7).

- `SetTimer` keeps a window's timer: its number, its period, held between `USER_TIMER_MINIMUM`
  and `USER_TIMER_MAXIMUM`, and when it next comes due on the declared clock. Set again with the
  same number it is replaced, and comes due a whole period from then. It answers the number. A
  timer with no window, the thread's own, is a wall, and so is one for a window that is gone: no
  title sets either.
- `KillTimer` takes the timer away, and with it any message it had due. A timer that is not there
  is refused: zero, and `ERROR_INVALID_PARAMETER`. `DestroyWindow` takes the window's timers with
  it, as Microsoft documents, as it takes the messages waiting for it.
- `PeekMessageA` makes a timer's `WM_TIMER` only when the filter takes nothing else: after posted
  messages, `WM_QUIT`, input and `WM_PAINT` (which this machine never makes). A timer has one at
  most: its number in `wParam`, its callback in `lParam`, and the tick count it was taken at as
  its time. Looked at, it stays; taken, the timer comes due again on its own beat, the first whole
  period after it last came due that is still to come, so a title that was late hears it once. As
  with posted messages (#139), only the thread that made the window takes it.
- `DispatchMessageA` calls a timer's callback in place of the window procedure, with the window,
  `WM_TIMER`, the number and the tick count, as a `TimerProc` is called. No title sets one.
- Sources: Microsoft's documentation of `SetTimer`, `KillTimer`, `WM_TIMER`, `DestroyWindow` and
  `PeekMessage`'s order (sent, posted, input, `WM_PAINT`, `WM_TIMER`); Wine's `server/queue.c`
  (`set_win_timer`, `kill_win_timer`, `get_message`, `find_expired_timer`, `restart_timer`) and
  `dlls/win32u/message.c` (`NtUserSetTimer`, `NtUserDispatchMessage`), read as documentation.

LEGO Island sets a one-second timer at a press of the button with nothing under the pointer to
take it (`LEGO1.DLL` 0x1005cfca) and kills it at the release. Its `WM_TIMER`, with the button still
down and the pointer not moved past the drag threshold, starts a drag: a button held still on the
island walks, from a declared second after the press. Before, the island stood still under it.

Hot Wheels sets a one-second timer as it starts (0x004152bb) and kills it only as it quits, before
it destroys its window, so its window procedure now hears a `WM_TIMER` once a declared second, 117
times over `first-race.route`. Its handler (0x41a31c) copies the count of frames drawn since the
last one (counted at 0x41a5d9) into a word nothing in the program reads, zeroes the count, and
passes the message to `DefWindowProcA`: a frame rate that is never shown, 52 instructions a
second. Nothing in the game reads what it writes, but the game reads its tick count at 120 places,
and every read after the first message lands that much later. Its first race finishes as before:
the same race, the same `WHEEL.DAT` and `TIMES.DAT`, written at the same flips, 870 and 3728, and
the quit at flip 3903. Every 25th frame of it compared, the menus, the track, the camera, the race
clock and the meters are the same throughout; in 46 of the 157 the cars themselves, and the sparks
they throw, are drawn a pixel or two apart, at most 13,368 pixels a frame, from the drop onto the
starting line (flip 875) to 1100 and from 2475 to the finish.

The runs that move, against main at `5a775ac`, every other end line and digest unchanged, Rock
Raiders', Alpha Team's and every one of LEGO Island's committed routes among them:

- Hot Wheels' `first-race.route`: end cursor 13,462,722,543 to 13,462,673,372, the same flip and
  declared millisecond; digest `d48d8761…` to `ca24a96c…`.
- Hot Wheels' escape route, written by its test: end cursor 583,424,325 to 583,424,086; digest
  `8bf567a9…` to `1448924f…`.
- Hot Wheels' boot to 2,200,000,000: digest `62d6d90b…` to `cb56737b…`.
- LEGO Island's restless hand: digest `c029337f…` to `99ce5f30…`. Two of its releases, at flips
  488 and 682, kill a timer that is not there and are refused with `ERROR_INVALID_PARAMETER`,
  which `LEGO1.DLL`'s C runtime saves and puts back around its thread data (`GetLastError` at
  0x1008d5f2). Nothing else of the run moves.

`LoadLibraryA` answers as Windows does for a library the declared machine does not have, and stops
at a wall for one it cannot say (#146), where it handed back a module for any name.

- A library is found when the title ships it, in its install directory whatever case it is asked
  for in, or when core writes an entry point of it. A name with no extension is looked for with
  `.DLL` added, as Microsoft's `LoadLibraryA` page documents.
- `Declared` gains `absent_libraries`: the libraries a title's machine does not have, each with
  why. One of those is answered NULL, with the declared Windows' error: `ERROR_DLL_NOT_FOUND`
  (1157) on Windows 9x, the answer Wine's conformance test records for 9x (wine-1.0,
  `dlls/kernel32/tests/module.c`, `testLoadLibraryA_Wrong`: "Expected ERROR_MOD_NOT_FOUND or
  ERROR_DLL_NOT_FOUND (win9x)"), and `ERROR_MOD_NOT_FOUND` (126) on NT, as Microsoft documents. Every
  declared Windows today is 9x. The run logs `library missing <name>: <why>`.
- Anything else stops at a wall that names the library as neither the title's own, nor written,
  nor declared absent: a library of Microsoft's that core writes nothing of (SHELL32, say), which
  every Windows 98 has, and a library the title ships only somewhere other than its install
  directory, the one place a title's library is looked for (a path asked for is cut to its file
  name). Answering either "not found" would send a title down a fallback
  no player met (golden rule 2).
- A library's `.dll` is now matched whatever its case, where only `.dll` and `.DLL` were. This is
  `find`'s match too, so an import naming `DDraw.Dll` binds to its shims, and `GetProcAddress` on a
  handle `LoadLibraryA` gave for it finds them, where both missed before. No binary on the roster
  spells an extension so.
- A handle for a library core writes is known by the file it names, not the path it was asked
  for by, so `GetProcAddress` on one asked for by a path finds its shims, where it missed before.
  No run asks so.
- `GetProcAddress`'s and `FreeLibrary`'s own code is unchanged: a title given NULL does not call
  them.

Every released title declares no absent library: in every gate run it asks only for libraries it
ships and ones core writes. Only Alpha Team calls `LoadLibraryA` in a committed run, for thirteen
libraries it ships or core writes, so no run moves. LEGO Chess, which is not on `main` yet, will
declare two when #143 lands: `glide2x.dll`, since its declared machine has no 3dfx card, and
`KLIB_OpenGL_PlugIn.dll`, a renderer plugin it walks but never shipped. Each is then told it is not
there, where the game would have called a null hook and its Glide plugin gone on into null `gr*`
pointers.

`absent_libraries` is a required field every title had to add to build, so it makes core's next
release a major one (ADR-0007, as #163 settled). Core stays at 1.2.0 until that release.

The mouse is the person's until they click on the picture, the same in every title, by the
maintainer's rule of 2026-10-21 (#141). A title that draws its own pointer took the mouse as soon as
the person's pointer was over the picture with the window in front, and only Cmd-Tab gave it back;
LEGO Island never took it, and was handed every move over its picture.

- Until a click on the picture the title sees none of the mouse, no move and no button. That
  click captures it and is the host's, and so is its release: it would otherwise press whatever
  the title's own pointer was over. A click on a window behind others captures too, once the
  window has come to the front, which it must do within two pumps after the one the click came
  in or half a second, whichever is longer, so that neither a fast host nor a stall cuts it
  short; a window that comes to the front any later captures nothing. A press on the window's
  frame never captures (#49).
- Captured, every move and button is the title's. A title that draws its own pointer (Rock
  Raiders, Hot Wheels, Alpha Team) has the person's hidden and frozen and its own steered by how
  far the mouse goes, as when it was taken. One that shows the host's (LEGO Island, #139) keeps
  it on the picture, in the title's shape, where until then it was the host's arrow: by
  `ClipCursor` to the client area on Windows, by a grab confined to the window on X, and on
  macOS, which has neither, by putting it back at the edge it left by once a pump, after the
  window's place in front is settled, with the mouse joined to the pointer again straight after
  so that the warp holds no event back. So on macOS the pointer can stray past the edge for a
  frame, or for as long as the title takes to flip, before it is put back, and a pointer that
  left for a window now in front, or a window closed by its button or minimized, is not put back. Which of
  the two a captured mouse is follows the title's `ShowCursor`, whenever it changes, and a title
  the mouse comes to be kept for, by a click or by its `ShowCursor`, is told where the pointer is
  at the end of the pump, which no move has told it since. "The picture" here is the window's
  content, bars included, as for a click (#49).
- Escape frees it and is the host's, repeating and let go, so the Escape that frees the mouse
  never quits Hot Wheels; the next one, with the mouse free, reaches the title as any key does,
  and stays the title's, repeating and let go, if a click captures the mouse while it is held.
  The window leaving the front frees it too; only a click captures it again. Either way the title
  lets go of every button it was told is down, whose release may go to another window, or on X
  come to nobody once the grab is gone. The end of the run frees it for good, through
  `Host::free_mouse`, which the run calls where it called `show_pointer(true)`, so no click
  captures it while the window waits to be closed (#111).
- A button let go reaches the title only if its press did, as a key does, and only once, and a
  press on the window's frame let go on the picture is no longer a release the title never saw
  pressed.
- The window's caption says which it is, `<name> — click to play` or `<name> — Escape frees the
  mouse`, with a plain dash on X, whose `XStoreName` takes Latin-1, and the name alone once the run
  is over. The run log's `pointer flip` line says a title that draws its own pointer has a
  captured mouse steered.

The rule is one state machine, `host::pointer::Capture`, to which each backend hands its clicks,
its keys, the title's `ShowCursor` and, once a pump, whether it is in front, and which answers
what the host does with the mouse (`host::pointer::Mouse`: free, steered or kept on the picture);
nothing in it is a title's. No route records the click that captures, its release or the Escape
that frees, and a headless run has no window to capture, so no committed route moves and no run
does: LEGO Island's hands, which drive the machine through a headless host, record the routes
they recorded before. The macOS backend is compiled and its unit tests run, not played; the
Windows and X backends are compiled and clippy-clean, not run.

A General MIDI synth in core plays a DLS Level 1 bank into the mix on the declared clock (#155,
ADR-0008 1(a)). Nothing calls it yet: LEGO Chess's MIDI stream (#154) and the player's bank (#156)
will, and no title's run moves.

- `dls::Bank::parse` reads a bank, written from the MIDI Manufacturers Association's DLS Level 1
  specification (v1.1b, read with its errata): instruments, their regions of the keyboard, each
  region's wave through the pool table, its sample values (the region's own `wsmp`, else its wave's,
  else a one-shot at middle C) and its articulation (the region's own `art1`, else its instrument's,
  else the specification's defaults). It walks RIFF with `winmm`'s chunk finder.
- Each wave is read once, however many entries of the pool table point at it, and they share it. A
  bank that records more than it holds is refused: a chunk past what holds it, a count past its
  chunk, a `colh` or `insh` counting other than the instruments or regions that follow, or waves
  whose data, added up, is more than the bank's own bytes. So the synth holds at most twice a bank's
  size, an eight-bit wave widened to sixteen. Refused as well, as Level 1 does not have them or no
  bank played yet uses them: a wave that is not mono PCM at eight or sixteen bits, a `wsmp` of more
  than one loop or a loop of another type, a loop past its wave's end, and a key set not to move its
  pitch.
- `synth::Synth` plays it: note on and off, program change and bank select, modulation, volume, pan,
  expression, sustain, the pitch wheel and its range, fine and coarse tuning, reset all controllers,
  all notes off, all sound off and the mode messages that end every note, and drums on channel 10
  from the bank's own kits, their keys past General MIDI's 35 to 81 included. Volume and pitch
  envelopes, one LFO, velocity and key scaling and loops come from each region's articulation. Bank
  select and data entry follow MIDI 1.0: a high part clears its low part, and data entry goes to
  whichever of a registered or a non-registered parameter was chosen last. A system common message
  ends running status.
- What the specification leaves to the device is declared in `synth.rs` with its reasons: 32 voices,
  Windows 98 SE's, a finished voice freeing its place before a note looks for one, stolen by DLS
  Level 1's static channel priority, a released voice before a held one and the oldest first; a key
  struck again cutting in 15 ms its notes that have had no note off, unless their own region lets
  them overlap, a note the pedal keeps ringing on; linear interpolation; envelopes and the LFO moved
  every frame; the LFO swinging gain both ways, the specification's worked example taken over its
  prose; gain added up in centibels and stopped at 0 dB; a key group cut in 15 ms; a variation the
  bank lacks played as its capital tone; reset all controllers keeping bank select unless asked for
  every power-on value; no reverb or chorus.
- The mix plays the synth: `Mixer::open_synth` gives it one, and `Mixer::midi` hands it a short
  message, packed as `midiOutShortMsg` takes one, heard from the frame its declared time falls in.
  It says whether a synth heard it: without the player's bank there is no MIDI device to send
  through, and the device that sent a message none heard stops the run at a wall (#154).
- Every sample is integer arithmetic, but for a note let go in its attack, which finds its decibels
  with the same maths as the tables. Cents to a ratio, centibels to an amplitude and the LFO's sine
  read tables built once with `cpu::x87::maths`, whose IEEE operations round alike on every host,
  and a test pins the tables' hash and what a tiny bank written in the tests renders.
- The player's `GM16.DLS` (sha256 `3229b09b…`) is loose on both pressings of Rock Raiders and both
  of LEGO Racers, and the bank can come from either. Read off the Rock Raiders Masterpiece rip the
  gate already uses, a C major scale on the piano and the GS Shaker, key 82, sound and render to the
  same samples every time; without the rip those tests skip, and `LEGO_LEGACY_REQUIRE_DISC=1` fails
  them. LEGO Chess's four tunes, played through it with that bank, reach 40 to 55 voices at once
  with every release tail heard. At 32 they steal 620 times, 611 of them a release tail, and 66
  notes are never played, 62 of them on channel 16, the lowest priority.

The declared CD can answer long names from its Joliet tree, as Windows 95's CDFS does (#144).
LEGO Chess opens `D:\intro\Chess Bumper01.bmp` after its intro, a name only its disc's Joliet tree
holds; the primary tree calls the same file `INTRO/CHESSB~1.BMP`.

- `Iso::read_joliet` finds the supplementary volume descriptor whose escape sequences are `%/@`,
  `%/C` or `%/E` (ECMA-119 8.5.6, the Joliet specification) and answers names from its tree from
  then on: UCS-2 big-endian, `;1` stripped. `Iso::open` still reads the primary descriptor alone.
- The primary tree's 8.3 names are not answered on a disc read this way. Nothing public says
  whether Windows 95's CDFS answers them or 8.3 names of its own making, as NT's does, and the two
  can differ on the roster's own rips: Chess's `ereg` records `LEGO Chess_EZ.exe` as
  `LEGOCH~3.EXE` and `LEGO Chess_eReg.exe` as `LEGOCH~2.EXE`, where numbering them in the long
  names' order swaps the two. A name only the primary tree records is told it is not
  there, and the machine stops at a wall naming it at the boundary, or as the title loads (golden
  rule 2); a name neither tree records is not found. No title asks for one. A path that mixes a
  long directory name and an 8.3 name is in neither tree, so it is not found rather than a wall.
- A path through a file is no path, in either tree, and is not found: every name on the way but
  the last must be a directory's, and `Iso::list` lists only a directory. Before, a file's bytes
  were read as a directory's records.
- `Declared::joliet` says whether a title's machine reads its disc this way. `load_title` reads
  the title's image by the primary tree's names, as the launcher does, and then the disc as the
  title declares; `testkit::tree` reads it the same way. It is a field every title had to add to
  keep building, so core's next release is a major (ADR-0007, settled on #163); core stays at
  1.2.0, past its last tag, until then.
- `FindFirstFileA` on the CD is a wall, for every title, where it looked only at the declared
  install tree and found nothing, `ERROR_FILE_NOT_FOUND`. No title searches its CD, and what
  Windows answers for a disc read through its Joliet tree, the 8.3 names included, is not known;
  it is written when a title's wall names it (golden rule 4). A pattern relative to a directory on
  the CD walls too. No committed run reaches it: only Alpha Team searches, with patterns relative
  to its install directory on C:, and its only `_chdir`s sit behind `PackDir` and `LocaleDir`,
  which its declared `Config.txt` never sets.
- The gate needs LEGO Chess's rip whenever `LEGO_LEGACY_REQUIRE_DISC=1`: its bumper's test reads
  the Masterpiece pressing, `game-data/chess/masterpiece-chess`.

Every roster rip has a Joliet tree. Read through it, with `joliet` forced on, Rock Raiders', Hot
Wheels' and LEGO Island's runs do not move (all 29 of their run lines, measured at `7509b49`):
their discs answer the same names but for case. Alpha Team's do. Its original pressing's Joliet tree has
`Strings_en.txt`, the primary tree's `STR_0012.TXT`, the autorun launcher's string table, and
the title's `MSVCRT.dll` opens `D:\Strings_EN.txt` at boot, which is otherwise missing. Every
route moves, and every boot run past 2,000,000 instructions; the boots to 100,000, 200,000 and
20,000,000 instructions and the debugger's quit do not. Measured at `55a3105` (#138 moves none of
Alpha Team's runs, and it asks for neither of its two 8.3 names that differ):

| Alpha Team run | Off | On |
|---|---|---|
| `menu.route` | 2,341 flips, 139,471 ms, `f34a24e8…` | 2,341 flips, 139,470 ms, `101c533b…` |
| `click-the-intro-300.route` | 1,634 flips, 110,593 ms, `a71a75d7…` | 1,634 flips, 110,591 ms, `5070d423…` |
| `click-the-intro-600.route` | 605 flips, 41,938 ms, `00c2662d…` | 605 flips, 41,937 ms, `b2d82eff…` |
| `select-adventure.route` | 4,454 flips, 214,104 ms, `87323b4d…` | 4,454 flips, 214,102 ms, `254746d4…` |
| `first-mission.route` | 8,054 flips, 331,967 ms, `1b7d7349…` | 8,053 flips, 331,937 ms, `48dfc4dc…` |
| boot to 1,100,000,000 | 1,634 flips, 111,704 ms, `1a08e772…` | 1,634 flips, 111,703 ms, `624bd32f…` |
| boot to 600,000,000 | 1,634 flips, 110,593 ms, `198706f8…` | 1,634 flips, 110,592 ms, `17169231…` |
| boot to 2,000,000 | 0 flips, 4 ms, `e37ef1e8…` | 0 flips, 4 ms, `2d5f0520…` |

So, by #142's rule, `joliet` is declared per title: the four released titles declare `false`,
their runs unchanged, until the maintainer decides, and LEGO Chess declares `true` with its crate
(#143).

LEGO Chess joined the roster as its fifth entry, `roster::LEGO_CHESS`, with its own crate and in
progress (#143): `Lego Chess.exe` out of the Masterpiece pressing's `MAIN.Z`, no Red Book audio.
Nothing of that game is loose on its disc, so core reads the archives its setup expands:

- `is3`, InstallShield 3's `.Z` archive: its header, its table of directories and then files, and
  each file in the directory its own record names by index, not where the table lists it. A table
  whose record is too short for its own name is refused as damaged (`Is3Error::Damaged`). The
  original pressing's `MAIN.Z` lists its executable, a root file, last, and a reader that deals the
  records out to directories in turn misplaces 48 of its files; a unit test holds the rule over an
  archive listed that way.
- `explode`, PKWARE's Data Compression Library implode undone, written from Ben Rudiak-Gould's
  description of the format (comp.compression, 2001, placed by him in the public domain), its
  three tables his, code word by code word: plain or coded literals, and a dictionary of one, two
  or four kilobytes. `explode(bytes, expanded)` takes the size its table gives it and stops a
  stream as soon as it would pass it (`ExplodeError::TooLong`). His
  example, with the one offset Mark Adler's `blast` notes as wrong put right, explodes to
  `AIAIAIAIAIAIA`; all 881 members of `MAIN.Z` and the 7 of `_SETUP.LIB` expand to the sizes their
  tables give, and were checked once, off the suite, against the research's own expansion of
  them.
- `title::Installed::Expanded`: every member of an archive on the disc under the install directory,
  expanded when it is read, as a cabinet's entry is unpacked (`install::Tree::set_expanded`), and
  at the root members of the setup's other archives it copies there on their own: Chess's
  `Text.txt` out of `_SETUP.LIB`.
- `EntryImage::in_cabinet` is `EntryImage::kept`, a `title::Kept`: loose on the disc, in the
  pressing's cabinet, or imploded in an archive it names. `roster::look_for` and the machine read it
  alike.

LEGO Chess's machine reads its disc through its Joliet tree from its install on (#144), and is the
first to declare absent libraries (#146), Glide's runtime and its OpenGL plugin; its own changelog
says why. Its bumper and its plugin table were not reached then: the run stopped at
`GetClipCursor`, which #147 answers above; #149's run reaches the plugin table and #148's walks it
through.
`install::Tree::install` is now the one way a title's tree is built, by the machine and by
`testkit::tree` alike, where each had its own copy of the step and Chess's test a third.

No other title's route moves.

A title can declare a floppy drive, A:, with no disk in it (#145). `Declared` gains the required
field `floppy`, which every title crate had to add to build, so by ADR-0007 this makes core's next
release a major: this heading releases as 2.0.0 (#163). The drives are listed in one place,
`Ctx::drives`. `GetLogicalDrives` sets bit 0 for the floppy, which Microsoft documents as A:;
`GetLogicalDriveStringsA` names `A:\` first; and `GetDriveTypeA("A:\")` answers `DRIVE_REMOVABLE`
(2). `GetDriveTypeA` with no root still answers C:'s type whatever the current directory, since
Alpha Team saves its game only on that answer. Every call that names a place on A: is refused as on
an empty drive, through `Ctx::refuse_on_empty_floppy`:

- KERNEL32's `GetVolumeInformationA`, `CreateFileA`, `OpenFile`, `GetFileAttributesA`,
  `FindFirstFileA`, `DeleteFileA`, `CreateDirectoryA`, `MoveFileA` from or to A:,
  `SetCurrentDirectoryA`, `SetFileAttributesA` and `LoadLibraryA`;
- VERSION's `GetFileVersionInfoA`, WINMM's `mmioOpenA`, and the movie stream's
  `IAMMultiMediaStream::OpenFile`.

For a title that has set `SEM_FAILCRITICALERRORS` each fails with `ERROR_NOT_READY` (21); the movie
stream hands it back as `HRESULT_FROM_WIN32(ERROR_NOT_READY)`. For one that has not, Windows would show its box asking
for a disk (`GetVolumeInformationA`'s remarks), and this machine has no such box, so the run stops
at a wall that names it. Other drives the machine lacks are not refused yet (#171). Only LEGO Chess
declares the drive. Every released title and the stub declare none and answer as before, so no
route moves.

A member of the pressing's archive that will not come out stops the run at a wall naming the
archive, the image, the member and why (#169), where the guest was told the file was missing, a
file `GetFileAttributesA` and `FindFirstFileA` had just said was there. A damaged import was taken
at boot for one of Microsoft's, and `LoadLibraryA` of a damaged library stopped at #146's wall,
which named the title's own library as neither its own nor written. The tree answers both kinds of
archive in one place, `Tree::member`: a cabinet entry (Rock Raiders, Alpha Team) and an
InstallShield 3 member (LEGO Chess) whose span will not read or runs past its archive's end, whose
data will not decode, that is stored a way the reader was not taught, or that comes out another
size than its table gives. The executable the launcher hashes (`EntryImage::read`) and LEGO Chess's
`Text.txt` are read elsewhere, and still refuse with their readers' own errors. `Cabinet::unpack`
now refuses the wrong size as `Corrupt`, as `Member::expand` already did, an entry whose header
gives bytes it holds none of among them; it handed back whatever its chunks inflated to.
`Iso::read_range` refuses a span past the image's end as the image damaged, so a rip cut short says
it has to be ripped again. `Tree::wall` hands the machine this wall as it hands #144's, after the
call that met it. A good rip never comes here: measured once off the suite, every one of Rock
Raiders' 444 packed entries and Alpha Team's 57 unpacks to its header's size, and Alpha Team's
seven placeholders declare none; the suite expands all 881 of `MAIN.Z`'s members to their table's.
No route moves.

## 1.1.0 — 2026-10-21

What LEGO Island 1.0.0 runs on, from its roster entry to its saves: counted semaphores, the old
file API and files read where they lie on the image, waits on several handles, DirectX 5's
Direct3D and retained mode with meshes built, drawn, picked and projected with uniform scaling,
posted messages and a title's own pointer shapes, and `MoveFileA`. Beside it, the Windows (#107)
and Linux (#108) backends, run by machine and not yet played by a person; a run's wrap-up however
it is stopped; and answers truer to Windows for every title: x87's own NaNs, `GetVersion` as
Windows 9x answers it, a null pointer faulting, `RaiseException`'s context, and DirectX 6's
Direct3D for Rock Raiders. Each change below lists the released titles' runs it moved.

Two backends joined, each written and compiled for its operating system and untested there:

- Windows (#107): a window through user32 and gdi32, the pointer by macOS's rules, and sound
  through winmm's `waveOut`, loaded from System32 at run time, with kernel32's `VirtualAlloc` and
  `LoadLibraryExW` beside `mmap` and `dlopen`. Compiles for x86-64 and Arm64 Windows.
- Linux (#108): an X11 window with the Mac's pointer rules and ALSA for sound, both reached by
  `dlopen`. Compiles for x86-64 and aarch64 Linux.

The pointer, pace and audio feed the backends share moved out of the Mac's into `host::pointer`,
`host::pace` and `host::feed`.

Fixed once the Linux backend was first run, on x86-64 and aarch64 under Xvfb (#108):

- A resized window's bars kept pieces of the frame drawn just before the backend was told the new
  size, since the X server only paints the background where a window is uncovered. The bars are
  now cleared at every frame.
- A window destroyed by another client (`xdotool windowclose`) ended the run, and then Xlib's own
  error handler ended the process over the window that was gone. Errors that only say the window
  is gone are now let pass; every other X error still ends the process as Xlib's handler does.
- An x87 operation with no answer, such as `0/0`, kept the NaN the host's own arithmetic made: the
  unit's negative indefinite on an x86-64 host, a positive NaN on an Arm one. Rock Raiders stores
  one at instruction 356,515,652 of its boot, so its longer runs ended with different digests on
  macOS and on x86-64 Linux, breaking golden rule 1. Every x87 result now follows the unit's own
  rule (`x87::answer`): a NaN operand comes through quieted, and a NaN made from numbers is the
  indefinite. Seven of Rock Raiders' macOS digests move to the ones x86-64 Linux already gave.
- Control-Q ended the run without the guest seeing the Q pressed, but the Q let go still reached
  the guest and a recorded route as `Q up`. The Mac's Command-Q and Command-W and Windows' Alt-F4
  did the same with their Q, W and F4. Every backend's keys now pass through one rule
  (`host::key_change`): a key let go that the guest was never told was pressed goes no further.
  No route moves: a route replays what the guest was told, and none holds such a release.

The same rule now holds for the floats the shims make on the host (#109). Retained mode's maths
(`win32::scene`: matrices, vectors, rotations, inverses, the projection, and a viewport's
`Transform` and `InverseTransform`) and `IDirect3DRMViewport2::GetField` kept whatever NaN the host
made when a title passed an infinity, or numbers large enough to overflow, and the guest reads
those results back. Each now comes out by the unit's rule (`x87::answer_single`, beside `answer`),
from the operands it was made from. No route reaches one, so no digest moves. The rasteriser, the
sound mix and every other float a shim only compares, or turns into a whole number, were audited
and left alone: no NaN's bits reach the guest from them.

Fixed once the Windows backend was first run, on x86-64 in a VM (#107):

- A title is a console program, so that what it says reaches the terminal it was started from,
  and closing its console, or Ctrl-C or Ctrl-Break in it, ended the process with no wrap-up: no
  `end` line and no digest in the log. Each now ends the run as the window's close button does. A
  closing console gives a process five seconds, and the handler holds it open while the run writes
  up; a second Ctrl-C, with the run already ending, ends the process as before.
- `close the window to end` was printed after a run the person had ended by closing the window.
  It is printed only when there is a window left to close, on every host.
- A run that ended by itself, its route finished or its budget spent, kept the pointer while its
  window waited to be closed, if the title had taken it: on Windows clipped to one pixel and
  hidden, so a person had to Alt-Tab away to get it back. Every host now gives it back as soon as
  the run ends, before `close the window to end`, the way losing focus does, and does not take it
  again while the window waits, clicked or in front (#111).

A run stopped from its terminal got no wrap-up either, on any other host (#110). On macOS, Ctrl-C,
`SIGTERM` and the terminal closing (`SIGHUP`) put the pointer back and ended the process; on Linux
nothing heard them; and a headless run ended on Ctrl-C, on Windows too, with no `end` line and no
digest. A title's `main` now hears each of them, headless or in a window (`host::listen`), and
each ends the run as the window's close button does, with its `end` line and digest. The signal
handler only asks for the close, which every host's `closed` answers; a second signal, with the
run already ending, ends the process as before, putting the Mac's pointer back first. A signal
the run was started ignoring, as `nohup` starts it ignoring `SIGHUP`, stays ignored. The
debugger does not listen, so Ctrl-C there still ends it. No route moves. Such a run's `end`
line says `stopped by the person`, where it said `window closed`, since a run stopped from its
terminal may have no window.

LEGO Island joined the roster as its fourth entry, `roster::LEGO_ISLAND`, with its own crate and
in progress (#113): `DATA/disk/ISLE.EXE`, one pressing, no Red Book audio. `title::WINDOWS_95`
is the Windows it declares, packed as `WINDOWS_98_SE` is. No other title's route moves.

Counted semaphores and `Sleep(0)`, for LEGO Island's streamer threads (#115):

- `CreateSemaphoreA` and `ReleaseSemaphore` are answered as Win32 documents them. A wait on one
  takes one of its count. A release past its maximum fails with `ERROR_TOO_MANY_POSTS` and
  changes nothing, and the count before is written back only when a release succeeds and it was
  asked for. Counts out of range are `ERROR_INVALID_PARAMETER`, and a named semaphore is a wall
  until a title asks for one.
- A wait on a semaphore parks as one on an event or a mutex does (ADR-0005), and a release wakes
  whoever is parked on it, the first in the machine's order taking the count.
- A wait with no timeout that was woken and found another waiter had taken what woke it goes back
  down. It answered `WAIT_TIMEOUT`, which an endless wait never does.
- `Sleep(0)` with another thread ready gives that thread the rest of the turn, where the caller
  ran on to its next call. The order is the scheduler's, never the host's, so every run
  interleaves the same way.

`core/tests/threads.rs` proves each of these on hand-assembled guests, without a title. No route
moves.

The old file API LEGO Island reads its scripts and world through, for its `MxIOInfo` (#116):
`OpenFile`, `_hread`, `_hwrite`, `_llseek` and `_lclose`, with `GetFileSize` and `SetEndOfFile`.
They are faces over the handles `CreateFileA` makes, which `ReadFile`, `SetFilePointer` and
`CloseHandle` take as well.

- `OpenFile` reads, writes, makes (`OF_CREATE`, read and write as MS-DOS's create opens it) and
  asks (`OF_EXIST`, answered TRUE with nothing left open), with each `OF_SHARE_*` mode as the
  documentation maps it onto `CreateFileA`'s. It fills the `OFSTRUCT`: its length, whether the
  file is on a fixed disk, the MS-DOS error, and the path. A bare name is looked for where the
  title started, then where it is. A style that parses, deletes, verifies, prompts or reopens is a
  wall.
- `_hread` short-counts at the end of a file; `_hwrite` of nothing sets where the file ends, as
  MS-DOS's write does; each fails with `HFILE_ERROR` and `GetLastError` as Windows does.
- A second open of a file another handle does not share as it asks, through either door, is
  refused with `ERROR_SHARING_VIOLATION`.
- `SetFilePointer` past the end of a file opened only to read lands there, as Windows says, where
  it stopped at the end; a read there finds nothing. A move method that is not one is
  `ERROR_INVALID_PARAMETER`, where it was the end. A handle that is not a file is
  `ERROR_INVALID_HANDLE`, where `GetLastError` kept whatever it last said, which is a failed seek
  that reads as a success at `0xffffffff`. Hot Wheels' C runtime seeks its three standard streams
  as it exits and keeps the error it reads, so the digests of its two routes that exit move, their
  end lines unchanged: `first-race.route` from `189da6d5…` to `1f7255c4…`, and the escape route
  from `c8f1a71e…` to `5e33fc05…`. The maintainer accepted the move.
- A file on the pressing opened only to read is read where it lies on the image, a piece at a
  time, rather than held whole when it is opened (`win32::Contents`, `Tree::lies`, `Iso::lies`):
  Island's scripts are 534 MB, and `Isle\ISLE.SI` alone is 157 MB. What a title writes is held
  and reaches its profile as before.

Waits on several handles and on threads, as Win32 documents them, for LEGO Island's engine (#130):

- `WaitForMultipleObjects` waiting for any of its handles parks on all of them, and is woken by
  whichever is signalled, answering `WAIT_OBJECT_0` plus its place. It was parked on the first
  only, so a signal to any other never woke it.
- Waiting for all of them, which was a wall, takes none until it can take every one together, and
  then answers `WAIT_OBJECT_0`. A handle named twice is `ERROR_INVALID_PARAMETER`.
- A wait with a timeout that was woken and could not take what it waited for goes back down until
  its hour, as an endless one already did. It answered `WAIT_TIMEOUT` early.
- A thread's handle is signalled when the thread ends, by returning or by `ExitThread`, so a wait
  on it is over. `GetExitCodeThread` is written: `STILL_ACTIVE` until then, and what it ended with
  after.

`core/tests/threads.rs` proves each on hand-assembled guests, without a title. No route moves.

What LEGO Island asks for before Direct3D (#114), each written when its run stopped there:

- `SystemParametersInfoA` turns mouse trails off (`SPI_SETMOUSETRAILS` with 0 or 1), which this
  machine never draws. Any other action is a wall that names it.
- `AdjustWindowRectEx` leaves the rectangle as it is, as `AdjustWindowRect` already did: a
  frame is as wide as `GetSystemMetrics` says its border, caption and menu are, and the declared
  machine's are all zero.
- `GetProfileStringA` answers the caller's default, cut to fit and with trailing blanks left
  off: the declared machine has no `WIN.INI`, so no section holds the key. A null section or key,
  which lists names, is a wall.
- `GetWindowLongA` answers `GWL_STYLE` with the style `CreateWindowExA` was given. Any other
  index is a wall.

`core/tests/shims.rs` calls each one as a title would, without a title, and holds `VirtualAlloc`
to Windows' 64 KiB granularity, which SmartHeap finds its blocks' headers by. No route moves.

A traced or stepped run of a threaded title switches threads where its route's run does (#131),
as golden rule 1 asks. Under `--trace insns` threads switched at every instruction, and in the
debugger they never switched at the quantum, so what either showed was not what the route did.
Every run now keeps one schedule, `Machine::step`: a burst to the 65,536-instruction quantum or
the next call across the boundary, the switch, the call or instruction the next thread stands
on, and the next burst. A traced run takes its bursts an instruction at a time to write each one
down, and the debugger steps by the same schedule an instruction at a time, standing past any
switch, so a break at a thread's next instruction is met. The stub still runs an instruction a
turn: it has one thread and no boundary, so nothing is switched, and its `int`s are heard and a
snapshot can fall between any two instructions. `Machine::tick` is one instruction or call with no
schedule. `core/tests/threads.rs` runs two threads past the quantum plainly, traced and stepped,
and finds the same switches, cursor for cursor, and the same finish and digest. No route moves.

DirectX 5's Direct3D and retained mode, which LEGO Island builds the other way round from Rock
Raiders: immediate mode first, by itself, then retained mode over it (#117). Each was written
when its run stopped there, and every identifier is from Wine's `d3d.h` and `d3drm.h`:

- `IDirect3D2` comes off the DirectDraw object (`QueryInterface` for `{6aae1ec1-…}`).
  `EnumDevices` offers the declared machine's one device, the same one DirectX 7's offers, in
  DirectX 5's 0xcc-byte `D3DDEVICEDESC`: a hardware device by its colour model, with nothing
  emulated. `CreateDevice` makes an `IDirect3DDevice2` over the back buffer for that device; any
  other is a wall that names it.
- `IDirectDrawClipper::SetHWnd` clips to the part of the screen the window shows, which is all of
  it, as `GetClientRect` already said, so the engine's primary surface takes its clipper.
- `IDirect3DRM2` comes off the retained-mode object (`{4516ecc8-…}`). `CreateFrame` makes an
  `IDirect3DRMFrame2`; `CreateDeviceFromD3D` an `IDirect3DRMDevice2` over the device the title
  made, which it holds; `CreateViewport` an `IDirect3DRMViewport`; `CreateLightRGB` a light as
  DirectX 6's does. `SetDefaultTextureColors` and `SetDefaultTextureShades` are kept.
- `IDirect3DRMDevice2` is DirectX 6's device's first 39 slots, and the frame array's
  `IDirect3DRMFrame` is DirectX 5's frame's first 69, each sharing its table. The v1 viewport is
  DirectX 6's but for `Clear`, which takes no flags, and is a wall.
- `IDirect3DRMFrame2` is a table of its own, mapped by name: DirectX 6's frame drops
  `GetTextureTopology`, `SetTextureTopology` and `AddMoveCallback2`, so from slot 33 up the two
  sit one, two and then three places apart, where the old tree had measured only the first.
- A face DirectX 5's own faces are asked for that is not here is a wall naming the face and the
  identifier, rather than `E_NOINTERFACE`. Only LEGO Island holds them.
- `D3DRMCreateColorRGBA` is written.

`core/tests/shims.rs` places a frame through DirectX 5's slots and reads it back, and makes
colours. No other title's route moves.

`GetVersion` told every title it was on Windows NT (#129). The declared Windows was packed with
its top bit clear, which Microsoft documents as NT, and with the build in the high word, where
Windows 9x puts none: no real machine ever answered that, breaking golden rule 4.
`title::Windows` now holds what each Windows answered to `GetVersion` and `GetVersionExA`, with
the sources beside it: Windows 98 SE `0xC0000A04`, build `0x040A08AE` and `" A "`; Windows 95
`0xC0000004`, build `0x040003B6` and no CSD string.

No title takes a different turn, and every route's end line is unchanged. Each C runtime only
stores a copy of the word: Rock Raiders', Hot Wheels' Watcom one, Island's two, and the
`MSVCRT.DLL` and A3D that Alpha Team runs. The `GetVersionExA` callers, that `MSVCRT.DLL`, A3D
and NetImmerse's `LoadComp.dll`, read only the platform and the version, which did not change.
The one reader that branches on the top bit, SmartHeap's test for NT in `ISLE.EXE` and
`LEGO1.DLL`, which picks where shared memory may lie, is not reached by Island's boot. The copies
are guest memory, so the digest of every title's run moves, and no stub's:

- Rock Raiders: `first-light.route` `856f27b3…` to `d0504186…`, `first-mission.route`
  `e9edcf14…` to `5c74c51c…`, `level-select.route` `4fe12832…` to `82a0c841…`,
  `click-the-intro.route` `0c754229…` to `e71381ad…`, `skip-the-intro.route` `b4fa40cc…` to
  `ed8f2de4…`, and the runs with no route to 100,000 (`5bd29ec1…` to `6c54a6b2…`), 61,000,000
  (`2222ff17…` to `f820fc34…`) and 400,000,000 (`179ce83e…` to `f0cc0561…`).
- Hot Wheels: `first-race.route` `1f7255c4…` to `ca178965…`, the escape route `5e33fc05…` to
  `920d063e…`, and the runs to 100,000 (`b388bc29…` to `c4eae7cb…`) and 2,200,000,000
  (`7f753ad4…` to `62d6d90b…`).
- Alpha Team: `first-mission.route` `5fe9a0f0…` to `8ca987d2…`, `menu.route` `5a9a6b3c…` to
  `d237d472…`, `select-adventure.route` `3a6da532…` to `27585089…`, `click-the-intro-300.route`
  `02fd93af…` to `2ca490be…`, `click-the-intro-600.route` `405fa26d…` to `d67fb670…`, and the runs
  to 100,000 (`b37a2a14…` to `6bebf32f…`), 200,000 (`c0749233…` to `4f6172f6…`), 2,000,000
  (`a799b066…` to `e37ef1e8…`), 600,000,000 (`da5f48ed…` to `7b40ea29…`) and 1,100,000,000
  (`ff247d41…` to `920a2827…`).
- LEGO Island: its boot (measured to 435,000 `92babebb…` to `dfd1ac5e…`, before #117 moved its
  tests to 478,000), every call it logs the same, and the run to 1,000 `66dd6743…` to `20202003…`.

Accepted by the maintainer on 2026-10-20, with Windows 95's word as Wine's `0xC0000004`.

What LEGO Island asks between its retained mode and its world's first mesh (#118), each written
when its run stopped there:

- `GetWindowLongA(GWL_HINSTANCE)` answers the instance `CreateWindowExA` was given.
- `LoadAcceleratorsA` looks for a table of keys where Windows does, among the resources the
  module's own headers point to. `ISLE.EXE`'s are cursors, icons and its version, so it answers
  none, with `ERROR_RESOURCE_TYPE_NOT_FOUND`. A module that holds a table stops the run.
- `VirtualQuery` answers from what `VirtualAlloc` and `VirtualFree` now write down: each region's
  start, size and protection, and which of its pages are committed and how. It gives the run of
  like pages from an address, as SmartHeap asks before it gives pages back; an address outside
  every region stops the run. Nothing a title reads changes from the writing down.
- `IDirect3DRM2::CreateTextureFromSurface` makes DirectX 5's `IDirect3DRMTexture2`: DirectX 6's
  texture as far as `GenerateMIPMap`, but for `Changed`, which takes two flags.
- `IDirect3DRM2::CreateMaterial` makes DirectX 5's `IDirect3DRMMaterial`: DirectX 6's material
  without its ambient colour.

`core/tests/shims.rs` asks all five. No other title's route moves.

Rock Raiders is handed DirectX 6's Direct3D, as Windows 98 with DirectX 6 hands it (#132). Its
mode dialog asks its DirectDraw for `IDirect3D3` to list the devices it can draw with. This
machine refused, so the list said the title's own "Error: DirectX6 not installed." and the title
went on with no device, reading its flags through a null pointer at `0x004784be`. That read gave 0
until the bottom of the address space faulted, below.

- `IDirectDraw4::QueryInterface` answers `IID_IDirect3D3` (Wine's `d3d.h`) with the same object's
  `IDirect3D3`.
- `IDirect3D3::EnumDevices` offers the declared machine's one device, the `Direct3D HAL` that
  DirectX 5 and 7 already enumerate, in DirectX 6's `D3DDEVICEDESC`. That is 0xfc bytes, as Wine's
  `d3dcaps.h` has it: DirectX 5's fields, then the texture repeat, anisotropy and blend stages that
  DirectX 7's description gives. One enumeration serves DirectX 5 and 6.

The title now lists the device and hands its identifier to `IDirect3DRM3::CreateDeviceFromSurface`.
It asks `SetSceneFogMethod` for table fog rather than vertex fog. This machine's retained mode
takes neither further, and every hundredth frame of `first-mission.route` to flip 9,100 is the
same picture as before. Every end line is unchanged. The digest of each run past the dialog,
about 39.7 million instructions in, moves:

- `first-light.route` `d0504186…` to `322ab7a0…`, `first-mission.route` `5c74c51c…` to
  `74b1031b…`, `level-select.route` `82a0c841…` to `49e57d12…`, `click-the-intro.route`
  `e71381ad…` to `b1c537af…`, `skip-the-intro.route` `ed8f2de4…` to `315d3880…`.
- The runs with no route to 61,000,000 (`f820fc34…` to `3e8b8a38…`) and 400,000,000
  (`f0cc0561…` to `6714ce40…`).

The run to 100,000 does not reach the dialog and does not move. No other title's route moves.
Accepted by the maintainer on 2026-10-20.

The bottom of a title's address space answers as Windows 9x's does, rather than reading as zero
(#132). LEGO Island read through a null pointer at address 8 and carried on with a 0 (#114).

- A read or a write in the first 4 KiB is an access violation (`0xC0000005`): Windows 9x's
  Win32 data selector refuses those addresses so that a null pointer faults (Pietrek, *Windows 95
  System Programming Secrets*). The record says whether it was a write and which address, as
  `EXCEPTION_RECORD`'s documentation has it. It goes to the guest's own handler chain through the
  dispatch `RaiseException` already used (`kernel32::dispatch`).
- A handler that carries on (`ExceptionContinueExecution`) gets the processor back as the context
  now says, every register and the flags, at its `Eip`. If nothing on the chain takes it, the run
  stops at a wall naming the access, the address and where it faulted.
- The rest of the first megabyte is MS-DOS's memory, which Windows 9x lets a Win32 process read
  and write, and code run from the bottom is not refused there either. This machine has neither,
  so each is a wall that names the address.
- A shim handed a pointer down there stops at a wall that says so, where it read or wrote
  whatever was there. A borrow of no bytes is always allowed, since Alpha Team's rasteriser
  borrows none at a null texture. The arena's bounds check is one subtraction longer
  (`Arena::with_floor`). The conformance corpus's arena, a 386 in real mode, has no floor.

`core/tests/access_violation.rs` proves it on hand-assembled guests, handled and unhandled.

Retained mode keeps the meshes LEGO Island builds its world from (#119), about four hundred of
them out of `WORLD.WDB`, each written when its run stopped there:

- `IDirect3DRM2::CreateMesh` makes the same `IDirect3DRMMesh` as DirectX 6's. The scene keeps
  each mesh's groups (`scene::Mesh`, `scene::Group`).
- `AddGroup` copies a group's faces and numbers it in turn. `SetVertices` copies its
  `D3DRMVERTEX`es, as the guest's bits, out of the guest's array, which Island reuses for the
  next group. Faces of differing sizes, a vertex past the group's end, or a group the mesh lacks
  are a wall or `D3DRMERR_BADVALUE`; every group at once (`D3DRMGROUP_ALLGROUPS`) is a wall.
- `SetGroupColor`, `SetGroupColorRGB`, `SetGroupMapping` and `SetGroupQuality` keep what they are
  given. `SetGroupMaterial` and `SetGroupTexture` keep a reference to what the group is drawn
  with and let go of what it was drawn with before.
- `GetBox` answers the least and greatest corner of every vertex in the mesh, picked by
  comparing. A vertex never set, or a mesh with none, is a wall.
- `Clone`, as `IID_IDirect3DRMMesh` and nothing else, makes a mesh of its own with the same
  groups.
- Nothing reads a group back yet: Island never calls `GetGroup`, `GetVertices`, `GetGroupCount`
  or a group's getters, and they stay walls.

Past the meshes, the same run stopped at two more:

- `IDirectSound3DBuffer::SetConeOutsideVolume` answers as done. A cone starts all the way round
  and nothing narrows one (`SetConeAngles` is a wall), so the volume outside it is never heard.
- `IDirect3DRMViewport::ForceUpdate` has nothing to copy, since a viewport here draws straight
  into its device's surface. Part of a viewport outside it is a wall.

Island now runs to 3,770,493,606, flip 3200, where its viewport's `Clear` is #120's.
`core/tests/shims.rs` builds meshes and forces an update. No other title's route moves.

Retained mode draws its own meshes, lit and textured (#120), which is how LEGO Island draws its
Information Center's actors. Each was written when Island's run stopped there:

- DirectX 5's `IDirect3DRMViewport::Clear` takes no flags and clears everything, as Wine's d3drm
  forwards it (`D3DRMCLEAR_ALL`): the viewport's part of the surface to its camera's scene's
  background, as DirectX 6's already did for Rock Raiders, and now the depth buffer under it too
  (`d3d::clear_depth`).
- `Render` draws a mesh hung on a frame itself, group by group, through the immediate-mode
  device underneath, with the projection and the rasteriser Rock Raiders' user visuals already
  draw through (`d3d::rasterise`, taken out of `DrawPrimitive` unchanged). The device's quality
  is the most a group's is drawn with: flat or Gouraud, lit or not; points, wireframe and Phong
  are walls. A face is in front when its corners go round clockwise, and one turned away is not
  drawn. A lit vertex takes the group's colour from the scene's lights and the material's
  emissive colour, and a material's highlight (Direct3D's specular term, with the viewer as far
  away as can be) is added over the texture. A group's colour with alpha is blended over what is
  behind it. A scene of both meshes and user visuals is a wall.
- A frame can be a visual of another, which draws its tree again wherever that frame is: Island
  hangs every model on a frame of its own and adds that frame to the one that places it.
- `IDirect3DRM2::CreateTextureFromSurface` keeps the surface, with a reference to it, and a
  texture's texels are read from the surface each time it is drawn, never copied. So
  `IDirect3DRMTexture2::Changed` has nothing to do.
- `CreateMaterial` keeps its power, and `SetSpecular` and `SetEmissive` their colours. A
  material starts with a white highlight, as Wine's d3drm makes one (`scene::Material`).
- `IDirect3DRMMesh::GetGroupTexture` hands back DirectX 3's `IDirect3DRMTexture`, as the header
  has it, which Island asks for `{120f30c0-1629-11d0-941c-0080c80cfa7b}`: that is
  `IID_IDirect3DRMTexture2` in Wine's `d3drmobj.h`, the same texture's DirectX 5 face.
- A null reference frame is the world, whatever the frame hangs from. It was a wall for a frame
  with a parent.

`SetField` is the half-width of the view at the front plane, which the projection already was:
Island's front plane is a tenth of a unit away, where Rock Raiders' a unit away hid it.

The projection still stretches the field to the viewport's own edges, fitted to Rock Raiders
(#29). Retained mode's documented default is uniform scaling, and Wine's tests hold a native
viewport to one scale across and down; Island's minifigures look a third too wide without it.
Rock Raiders never calls `SetUniformScaling`, so the two titles cannot be told apart, and uniform
scaling would move every Rock Raiders route. That is left to the maintainer.

Island goes on into its Information Center and its idle loop, past 19 billion instructions with no
wall. `core/tests/shims.rs` clears and draws a lit mesh, and a textured one whose surface changes,
and puts a frame in the world through a null reference. No other title's route moves.

`RaiseException`'s `CONTEXT` had `EFlags` and `Esp` one field late, at 0xc4 and 0xc8 (#133).
`winnt.h`'s i386 `CONTEXT` (Wine's `include/winnt.h`) has `EFlags` at 0xc0, `Esp` at 0xc4 and
`SegSs` at 0xc8, so a handler reading the stack pointer out of it read the flags, breaking golden
rule 4. A fault's context already had them there. The one writer in `kernel32::dispatch` now puts
them there for both.

Only Alpha Team raises through `RaiseException`: its C++ throws, `0xe06d7363`, eight of them
before its 20 millionth instruction. No handler on its chain reads or writes any byte of the
context: watched on `menu.route` to 45.7 billion instructions, no guest instruction touches one.
So no title takes a different turn and every end line is unchanged. The contexts are on the heap
the digest hashes, so the digest of every Alpha Team run that throws moves:

- `first-mission.route` `8ca987d2…` to `1b7d7349…`, `menu.route` `d237d472…` to `f34a24e8…`,
  `select-adventure.route` `27585089…` to `87323b4d…`, `click-the-intro-300.route` `2ca490be…` to
  `a71a75d7…`, `click-the-intro-600.route` `d67fb670…` to `00c2662d…`.
- The runs with no route to 600,000,000 (`7b40ea29…` to `198706f8…`) and 1,100,000,000
  (`920a2827…` to `1a08e772…`).

The runs to 100,000, 200,000 and 2,000,000 throw nothing and do not move, and no other title's
run or stub's moves. `core/tests/access_violation.rs` reads both fields back from a handler
`RaiseException` called. Accepted by the maintainer on 2026-10-20.

Retained mode projects with uniform scaling, as d3drm does by default (#134). A viewport's front
plane, its field or `SetPlane`'s, reaches the edges of the viewport's longer side, and the shorter
side shows as much of it as the same scale fits, so a square stays square: a field of 0.5 on 640
by 480 is half a unit across and 0.375 up. Since #29 the plane was stretched to all four edges.

- Retained mode's documentation: uniform scaling scales "the viewing volume into the larger
  window dimension" (`GetUniformScaling`), and "the viewing angle is calculated using half the
  height or half the width of the front clipping plane, whichever is larger" (Viewing Frustum).
  Wine's tests hold native retained mode's own viewport underneath to one scale, the longer
  side's half.
- Frames of both titles on Windows agree. Rock Raiders' Chief in its retail Mission Brief is as
  tall as this draws him, where the stretch drew him a quarter shorter, his head wider than tall. LEGO
  Island's two plants stand on the same pixels as in the game on Windows 10 with its own d3drm,
  in front of the stage's lip, where the stretch stood them eighteen pixels higher.
- #29 fitted the stretch to an earlier emulator's frames, which stretched too. The dark walls
  and black edges it saw were its spot lights' halved cones, fixed in the same change.
- `SetUniformScaling(FALSE)`, which scales the view into the window across and up apart, is a
  wall: no title asks for it. `core/tests/shims.rs` holds it to that.

Every end line is unchanged. The digests of the runs that draw a viewport wider than it is tall
move:

- Rock Raiders' `first-mission.route` `74b1031b…` to `4888818a…`.
- LEGO Island's `information-center.route`, `2f2d5411…` to `69bd9a2e…`.

Rock Raiders' other runs, its menus and level select included, and every other title's, do not
move. Accepted by the maintainer on 2026-10-20; Rock Raiders' first mission is played again by a
person before its next release (golden rule 7).

What LEGO Island asks for once a hand is on it (#124), each written when a run driven by a
synthetic hand through the host stopped there:

- `IDirect3DRMViewport::Pick`: every mesh the viewport draws over a place on the device, nearest
  first, each found where `Render` would draw it, cut to the view volume and culled as it is, and
  picked once at its nearest face. `IDirect3DRMPickedArray` hands each one out with its visual,
  an `IDirect3DRMFrameArray` of the frames from its scene's root down to the one it hangs on (a
  frame hung as a visual among them), and its `D3DRMPICKDESC`: face, group, and where in the scene
  the place picked is. The game asks at every press of the button and takes the first frame below
  the root that carries its own data as what was pressed. This was the old tree's live crash: it
  had no `Pick`, and the first press ended the run, even in the black seconds before first light,
  which a person saw as the game crashing right away and no route, which never pressed a button,
  could see. This tree stopped at the same wall, at instruction 942,097, flip 102.
- `IDirect3DRMFrame2::GetLights` and `IDirect3DRMLightArray`: the lights hung on a frame as they
  are now. Signing the registration book recolours a light through it.
- `IDirectDrawSurface::Blt` with `DDBLT_ROP` and the raster operation `SRCCOPY`, which is a copy;
  any other operation is a wall. The registration book draws its blinking cursor this way.
- `GetAsyncKeyState`, as Windows 95 answers it: the top bit while a key is down, the bottom bit if
  it went down since it was last asked about, which asking clears; no left key told from a right
  one; the two buttons as keys too. The game asks about the arrows before it believes
  DirectInput's word that they are held.

`ComState::FrameArray` is now `ComState::Array`, a frame array's frames or a light array's
lights. Still not sent: `WM_TIMER` (#138). LEGO Island sets a timer at every press outside and
kills it at the release, and a button held still for a second starts a drag on Windows with the
timer's message; Hot Wheels sets a one-second timer as it starts, so sending the message moves
every Hot Wheels run, which is the maintainer's to decide. Nor is LEGO Island's own
`PostMessageA` to itself, which sets its pointer's shape (#139). The joystick's `joyGetPosEx` and
`joyGetDevCapsA` are not reached: `UseJoystick` is not declared and the game's default is off.

No other title's run moves.

`PostMessageA` posts, where it answered success and posted nothing, which golden rule 4 forbids
(#139). The message waits in the queue of the thread that made the window, stamped with the
declared time and the pointer as it is posted, and `PeekMessage` takes it as Windows documents:
after whatever was sent or posted before it, before every key and mouse message, however long those
have waited. Only the thread that made the window takes its messages. A message below `WM_USER`,
or to any window but the title's own, is a wall: which of the system's carry pointers, and so
cannot be posted, is not written. No title calls `PostThreadMessageA`.

And a title's pointer shapes reach the host (#139). `LoadCursorA` loads a cursor out of a module's
own resources, its `RT_GROUP_CURSOR` and the `RT_CURSOR` the group names, once, as `LoadCursor`
shares what it loads; a group of more than one cursor, or a cursor that is not paletted, is a
wall. Windows' own arrow is answered with the handle every title had before. `SetCursor` sets the
shape and answers the one before; a handle that is not a cursor is a wall. At the flip the host is
told, through a new `Host::pointer_shape` whose default does nothing: macOS sets an `NSCursor` of
it over the picture while the window is in front, Windows makes it the window class's cursor
while the window is in front, and X defines it on the window while the window has the keyboard.
Each gives the arrow back on focus loss, and every host is given it back when the run ends (#111).
The headless host records what it was told. The Windows and X backends are compiled and their
bitmaps tested on every host, not run.

LEGO Island posts itself `WM_ISLE_SETCURSOR`, 21 times on its ride, and its window procedure answers
with `SetCursor`: the hourglass as it starts, the arrow, the hourglass again, no pointer for the
intro, and the arrow and the hourglass as it loads and is clicked. Its runs move, and only its:

- `first-light.route` `47ad2868…` to `e753e559…`; first light is still flip 460, now at
  instruction 97,000,702 where it was 97,000,222.
- `information-center.route` `69bd9a2e…` to `93ac795c…`; its idle loop still starts at flip
  5099, now at 3,952,209,667 where it was 3,952,208,885.
- `first-ride.route`, and the hand's run it was recorded from, `cf079381…` to `e08fb5c1…`. The
  route is unchanged: the hand's run records the same one.
- The restless hand `b172d1ec…` to `c029337f…`, and the boot `31cce8ba…` to `da6e3642…`.

Every run ends on the instruction, flip and declared millisecond it ended on before. Rock Raiders,
Hot Wheels and Alpha Team never post a message, set a cursor or load one but Windows' arrow, and
none of their runs moves.

`MoveFileA`, which LEGO Island reaches when a second name signs its registration book (#125): it
moves every player's saved game down a slot, `G0.GS` to `G1.GS` and on, before it saves the new
player's as the first. A file the guest wrote moves within the profile, where the next run finds
it. A move from nothing fails with `ERROR_FILE_NOT_FOUND`, and one onto a path already there, the
guest's or the pressing's, with `ERROR_ALREADY_EXISTS`, as on Windows; the game carries on past
either. A move the host refuses is told to the person as a refused save is, and the guest is told
it moved. Moving a file a handle holds open, one of the pressing's own, or anything onto the disc
in D: is a wall. `core/tests/old_file_api.rs` holds each. The game writes its saves through its C
runtime, `CreateFileA`, `WriteFile` and `CloseHandle`, and reads them back through `CreateFileA`,
all answered already; its `_hwrite` and `SetEndOfFile` are its scripts' (#116), and a save does not
reach them. No title's run moves.

`PostMessageA` posts `WM_CLOSE`, the first of the system's own messages it posts (#140). It carries
no pointer, so Windows posts it as it posts a title's own. LEGO Island closes its own window this
way when its credits are over (`LegoOmni::CloseMainWindow`), which stopped its runs at a wall at
flip 17338. Its window procedure now takes it: the game is deleted, its `Close` saves the game and
the scores once more, the same bytes the quit door saved, and it leaves its loop, destroys its
window and exits 0, at instruction 13,071,129,548, flip 17341. Every other message below `WM_USER`
is still a wall. No other title posts one, and no committed route reaches Island's, so no run
moves.

## 1.0.0 — 2026-10-18

The engine the three v1 titles run on: an x86 and x87 interpreter for the guest's code, the Win32
and DirectX calls those titles make answered in Rust, a declared machine per title that the title
cannot see past, and a macOS host reached at run time with no external crates (ADR-0001). A run is
replayed from a route, logs what it did, and stops at a named wall when a title asks for something
the machine does not have.
