# Core Wall-Time & CPU Sample Profiles Across Committed Roster Routes

Empirical baseline for `#246` (child of `#243`, unblocking `#248`). All five title crates in the workspace (`lego-rock-raiders`, `hotwheels-stunt-track-driver`, `lego-alpha-team`, `lego-island`, `lego-chess`) were profiled sequentially on uncontended CPU with full symbolication.

## Environment & Methodology

- **Host**: Apple M4 (10-core: 4 performance + 6 efficiency, 16 GB RAM), macOS 26.6.2 (25G83), `aarch64-apple-darwin`
- **Toolchain**: `rustc 1.98.0 (88d9e12ae 2026-08-18)`, `cargo 1.98.0`
- **Build profile**: workspace `[profile.release]` (`opt-level = 3`, `debug = true`, `codegen-units = 1`, `lego-legacy-core` v1.2.0)
- **Execution**: each route run sequentially (`--headless`, isolated `--data-root` and `LEGO_LEGACY_RUNS`, `LEGO_LEGACY_GAME_DATA` pointed at local rips). Routes that do not self-exit (`maintainer-first-play`, `level-select`, `first-mission` for Rock Raiders and Alpha Team, `menu`) had `flip <N> close` appended to a temporary copy matching their integration-test endpoint flip so each run terminates cleanly with full run-log metrics.
- **Sampling**: `/usr/bin/sample <pid> <secs> 1 -file <out>` (1 ms interval, covering the entire run from 50 ms after spawn to process exit). Call graphs were demangled with `/usr/bin/c++filt` and analyzed strictly over the `lego_legacy_core::run::main` main-thread subtree (excluding pre-`main` `dyld` launch samples).
- **Metric definitions** (per `CONTEXT.md`):
  - **Cursor**: total retired guest instructions (includes passed clock spins).
  - **Executed instructions**: `cursor - instructions_passed` (only instructions actually executed by the engine).
  - **Throughput**: `executed_instructions / wall_seconds` (MIPS).
  - **Pace**: `declared_ms / wall_ms` (`1.00×` = real time).
  - **Frame rate**: `flips / wall_seconds` (host FPS).
  - **Presentation rate**: `flips / declared_seconds` (guest FPS).
- **Subsystem attribution**: leaf-to-root walk over the main-thread call stack, attributing leaf libc/allocator helpers (`_platform_memmove`, `malloc`/`free`, `u128_div_rem`) and `guest::Arena` calls to the Core subsystem that invoked them. Main-thread samples represent true serial wall time (1 sample ≈ 1 ms on the critical path).

### Note on `semaphore_wait_trap` in `raster::triangles`

`raster::triangles` (`core/src/raster.rs:741–799`) splits draws exceeding `64_000` pixels across `BANDS = 4` bands using `std::thread::scope`, spawning 4 worker threads while the main thread blocks in `std::thread::Thread::park` → `semaphore_wait_trap`. As documented in `core/src/raster.rs:722–733` (`#72`), a persistent thread pool moved `semaphore_wait_trap` off the main thread without changing wall clock time because the wait is the main thread waiting for the slowest of the 4 raster bands to finish `raster::triangle`. Accordingly, `semaphore_wait_trap` under `raster::triangles` is attributed to **Software rasteriser** (broken out explicitly below) and represents multi-threaded rasterisation wall time, not wasted sync overhead.

---

## 1. Roster Wall-Time, Throughput, and Pace Summary

Primary gameplay routes are marked with **\***.

| Title | Route | Wall (s) | Cursor | Passed Spins | Executed Insns | Flips | Declared (ms) | Throughput (MIPS) | Pace | Frame rate (FPS) | Pres. rate (FPS) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| `lego-rock-raiders` | `level-select` (`close` @ 4790) | 56.13 | 8,036,708,351 | 0 | 8,036,708,351 | 5,091 | 308,716 | 143.17 | 5.50× | 90.7 | 16.5 |
| `lego-rock-raiders` | **`first-mission`*** (`close` @ 9050) | 118.38 | 14,255,879,422 | 0 | 14,255,879,422 | 9,052 | 374,983 | 120.43 | 3.17× | 76.5 | 24.1 |
| `hotwheels-stunt-track-driver` | **`first-race`*** (self-exit) | 90.43 | 13,462,673,313 | 0 | 13,462,673,313 | 3,903 | 120,966 | 148.87 | 1.34× | 43.2 | 32.3 |
| `lego-alpha-team` | `menu` (`close` @ 2350) | 46.35 | 5,430,529,066 | 0 | 5,430,529,066 | 2,351 | 139,821 | 117.17 | 3.02× | 50.7 | 16.8 |
| `lego-alpha-team` | **`first-mission`*** (`close` @ 7990) | 197.15 | 23,226,131,304 | 0 | 23,226,131,304 | 7,991 | 329,886 | 117.81 | 1.67× | 40.5 | 24.2 |
| `lego-island` | `maintainer-first-play` (`close` @ 10667) | 53.37 | 3,348,707,447 | 0 | 3,348,707,447 | 10,671 | 274,718 | 62.74 | 5.15× | 199.9 | 38.8 |
| `lego-island` | **`pizza-delivery`*** (self-exit) | 126.45 | 10,657,537,990 | 0 | 10,657,537,990 | 22,041 | 605,367 | 84.29 | 4.79× | 174.3 | 36.4 |
| `lego-chess` | `throne-room` (self-exit) | 5.48 | 3,246,057,181 | 2,670,184,342 | 575,872,839 | 471 | 24,867 | 105.01 | 4.54× | 85.9 | 18.9 |
| `lego-chess` | **`checkmate`*** (self-exit) | 81.41 | 11,685,933,862 | 2,670,184,342 | 9,015,749,520 | 4,311 | 109,017 | 110.75 | 1.34× | 53.0 | 39.5 |

**Key observations:**
- **Tightest Pace headroom**: `hotwheels-stunt-track-driver` (`first-race`, **1.34×**), `lego-chess` (`checkmate`, **1.34×**), and `lego-alpha-team` (`first-mission`, **1.67×** whole-route average; subtracting the 46.35 s / 139.8 s-declared menu prefix yields **1.26×** Pace during the 3D training mission itself: 190.1 s declared in 150.8 s wall!).
- **Passed clock spins**: only `lego-chess` triggers clock-spin passing (`2.67B` instructions passed during intro/boot waits — 82.3% of `throne-room`'s cursor and 22.8% of `checkmate`'s). Hot Wheels spins inside its game loop without matching the current spin detector, retiring `13.46B` real instructions.
- **Throughput spread (62.7–148.9 MIPS)**: `hotwheels-first-race` reaches **148.9 MIPS** because 97.5% of wall time is pure CPU interpreter (no D3D rasteriser). `lego-island` drops to **62.7–84.3 MIPS** because **42.8%–47.4%** of wall time is spent outside the interpreter in the software rasteriser and `ddraw::transfer` blits.

---

## 2. Subsystem Wall-Time Breakdown (% of `run::main`)

| Route | Main Samples | CPU Interp | Software Rasteriser | Blits / DDraw | Video Codecs | Audio | Win32 / COM / I/O / Other | Host |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `rockraiders-level-select` | 46,937 | **85.8%** | 0.4% | 5.7% | 4.3% | 0.1% | 3.5% | 0.2% |
| **`rockraiders-first-mission`*** | 97,471 | **75.6%** | **15.3%** | 4.5% | 2.1% | 0.2% | 2.1% | 0.2% |
| **`hotwheels-first-race`*** | 74,268 | **97.5%** | 0.0% | 0.0% | 0.0% | 1.2% | 1.0% | 0.3% |
| `alphateam-menu` | 34,649 | **74.5%** | 9.9% | 2.2% | 4.4% | 0.1% | 8.8% | 0.2% |
| **`alphateam-first-mission`*** | 163,288 | **78.3%** | **17.7%** | 0.5% | 0.9% | 0.1% | 2.3% | 0.1% |
| `island-maintainer-play` | 44,142 | **50.3%** | **27.7%** | **19.7%** | 0.0% | 0.3% | 1.3% | 0.8% |
| **`island-pizza-delivery`*** | 105,687 | **55.8%** | **27.8%** | **15.0%** | 0.0% | 0.3% | 0.6% | 0.6% |
| `chess-throne-room` | 4,053 | **76.2%** | 12.9% | 0.2% | 4.0% | 1.8% | 4.7% | 0.3% |
| **`chess-checkmate`*** | 63,969 | **69.9%** | **28.5%** | 0.1% | 0.3% | 0.4% | 0.7% | 0.2% |
| **5-Primary Wall-Weighted Avg** | *(613.8 s)* | **74.9%** | **18.1%** | **4.1%** | **0.7%** | **0.4%** | **1.5%** | **0.3%** |

*(Across all 9 routes, `Machine::run` accounts for **98.3%–99.9%** of `run::main` wall time; `setup` is `<0.2%` and `wrap_up` / SHA-256 `state_digest` is `0.1%–1.7%`.)*

### 2a. CPU Interpreter Internal Split (% of `run::main` wall time)

| Route | `cpu::interp` | `cpu::exec` | `cpu::x87` | `cpu::flags` | `guest::Arena` (CPU) | `Machine` loop/dispatch | Total CPU |
|---|---:|---:|---:|---:|---:|---:|---:|
| `rockraiders-level-select` | 35.8% | 35.8% | 0.2% | 7.2% | 3.2% | 3.6% | **85.8%** |
| **`rockraiders-first-mission`*** | 32.7% | 28.4% | 1.6% | 5.3% | 3.3% | 4.2% | **75.6%** |
| **`hotwheels-first-race`*** | 38.6% | 36.0% | 3.9% | 5.3% | 4.3% | 9.1% | **97.5%** |
| `alphateam-menu` | 34.3% | 24.6% | 5.0% | 3.3% | 4.8% | 2.4% | **74.5%** |
| **`alphateam-first-mission`*** | 34.7% | 25.7% | **8.7%** | 2.9% | 4.2% | 2.2% | **78.3%** |
| `island-maintainer-play` | 16.5% | 17.5% | 1.3% | 2.8% | 1.3% | **10.8%** | **50.3%** |
| **`island-pizza-delivery`*** | 19.5% | 20.5% | 1.5% | 3.8% | 1.3% | **9.1%** | **55.8%** |
| `chess-throne-room` | 28.4% | 23.9% | 3.6% | 2.6% | 2.2% | **15.3%** | **76.2%** |
| **`chess-checkmate`*** | 26.6% | 22.8% | **11.4%** | 3.3% | 2.3% | 3.4% | **69.9%** |

*(`cpu::decode` is `<0.02%` on every route because the basic-block run cache in `Interp::run_at` amortizes decoding; `clock / u128_div` inside CPU is `0.0%–0.2%`.)*

### 2b. Software Rasteriser Internal Split (% of `run::main` wall time)

| Route | `raster::triangle` (main) | `raster::triangles` 4-band wait | `raster::Texture::texel_at` | `win32::d3d` | `win32::d3drm` | `win32::pipeline` | Worker Samples (off-main) | Total Raster (wall) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **`rockraiders-first-mission`*** | 12.8% | 0.0% | 0.0% | 1.0% | 1.0% | 0.4% | 60 | **15.3%** |
| **`hotwheels-first-race`*** | — | — | — | — | — | — | 0 | **0.0%** |
| `alphateam-menu` | 6.4% | 2.0% | 1.2% | 0.3% | — | 0.1% | 1,629 | **9.9%** |
| **`alphateam-first-mission`*** | 10.2% | 4.8% | 2.0% | 0.5% | — | 0.1% | 17,454 | **17.7%** |
| `island-maintainer-play` | 9.3% | 5.6% | 0.4% | 0.9% | **6.1%** | **5.3%** | 4,574 | **27.7%** |
| **`island-pizza-delivery`*** | 9.0% | 6.8% | 0.3% | 0.8% | **5.5%** | **5.4%** | 12,699 | **27.8%** |
| `chess-throne-room` | 6.1% | 5.3% | 1.1% | 0.2% | 0.0% | 0.0% | 462 | **12.9%** |
| **`chess-checkmate`*** | 15.8% | 8.1% | 2.8% | 1.2% | 0.0% | 0.4% | 14,032 | **28.5%** |

*Notice why `rockraiders-first-mission` has `0.0%` band wait (only 60 worker samples): its mission draws submit small triangles below `WORTH_BANDS = 64_000` pixels, so virtually all rasterisation executes single-threaded on the main thread (`12.8%` in `raster::triangle`). By contrast, `chess-checkmate`, `alphateam-first-mission`, and `lego-island` hit the 4-band path heavily (`12k–17k` worker samples).*

---

## 3. Top 20 Hot Functions Per Title (Self-Time & Inclusive-Time)

All percentages are `% of run::main` main-thread samples on each title's primary gameplay route.

### 3.1 `lego-rock-raiders` — `first-mission.route` (97,471 samples, 118.38 s)

| # | Self % | Incl % | Function |
|---:|---:|---:|---|
| 1 | 27.7% | 35.5% | `cpu::exec::execute` |
| 2 | 27.2% | 65.6% | `cpu::interp::Interp::run` |
| 3 | 12.8% | 12.9% | `raster::triangle` |
| 4 | 5.5% | 5.5% | `cpu::interp::Interp::run_at` |
| 5 | 3.2% | 27.6% | `machine::Machine::tick` |
| 6 | 3.1% | 3.1% | `guest::Arena::slice_mut` |
| 7 | 2.8% | 2.8% | `_platform_memmove` *(ddraw::transfer 2.2%, Arena::write 0.5%, iv50 0.3%)* |
| 8 | 2.2% | 2.2% | `cpu::flags::Flags::sub` |
| 9 | 1.5% | 4.2% | `win32::ddraw::transfer` |
| 10 | 1.3% | 1.3% | `cpu::flags::Flags::add` |
| 11 | 1.2% | 1.2% | `cpu::flags::Flags::logic` |
| 12 | 1.1% | 1.6% | `cpu::x87::execute` |
| 13 | 1.0% | 1.0% | `inflate::block` *(InstallShield cabinet unpack during boot/load)* |
| 14 | 0.9% | 1.2% | `iv50::Codebook::read` |
| 15 | 0.6% | 0.6% | `cpu::flags::Flags::inc_dec` |
| 16 | 0.5% | 0.5% | `DYLD-STUB$$memcpy` *(from `win32::ddraw::transfer`)* |
| 17 | 0.4% | 1.6% | `iv50::blocks` |
| 18 | 0.4% | 0.9% | `guest::Arena::write` |
| 19 | 0.4% | 0.7% | `_xzm_free` |
| 20 | 0.4% | 0.4% | `cpu::exec::shift` |

*(Secondary route delta — `level-select.route`, 56.13 s: pre-mission boot/movies/rockfall/level-select is 85.8% CPU interpreter, 5.7% `ddraw::transfer`, 4.3% `iv50`/`amstream` video, 2.1% `inflate::block`, only 0.4% rasteriser. Subtracting `level-select` implies the 3D mission itself spends ~28–30% of wall time in `raster::triangle` + D3D/D3DRM.)*

### 3.2 `hotwheels-stunt-track-driver` — `first-race.route` (74,268 samples, 90.43 s)

| # | Self % | Incl % | Function |
|---:|---:|---:|---|
| 1 | 33.6% | 44.1% | `cpu::exec::execute` |
| 2 | 31.8% | 80.9% | `cpu::interp::Interp::run` |
| 3 | 6.8% | 6.8% | `cpu::interp::Interp::run_at` |
| 4 | 4.2% | 4.2% | `guest::Arena::slice_mut` *(Interp::run 2.7%, exec::execute 1.0%, string_op 0.6%)* |
| 5 | 3.7% | 7.7% | `machine::Machine::tick` |
| 6 | 3.1% | 3.8% | `cpu::x87::execute` |
| 7 | 2.1% | 2.1% | `cpu::flags::Flags::inc_dec` |
| 8 | 1.5% | 2.1% | `cpu::exec::string_op` *(guest software renderer `rep movsd`/`stosd`)* |
| 9 | 1.2% | 1.2% | `cpu::flags::Flags::sub` |
| 10 | 1.2% | 1.2% | `cpu::flags::Flags::logic` |
| 11 | 0.9% | 1.7% | `_xzm_free` *(malloc/free total 2.3%: Machine::step 1.1%, stack_args 0.5%, tick 0.5%)* |
| 12 | 0.9% | 0.9% | `cpu::flags::Flags::add` |
| 13 | 0.8% | 99.5% | `machine::Machine::step` |
| 14 | 0.6% | 0.6% | `cpu::exec::shift` |
| 15 | 0.5% | 0.5% | `__bzero` |
| 16 | 0.5% | 0.8% | `guest::Arena::write` |
| 17 | 0.5% | 0.5% | `compiler_builtins::int::specialized_div_rem::u128_div_rem` *(Audio mix/clock 0.6%)* |
| 18 | 0.4% | 0.4% | `_platform_memmove` |
| 19 | 0.4% | 0.4% | `_free` |
| 20 | 0.4% | 0.4% | `_malloc_zone_malloc` |

*Pure CPU-interpreter workload (**97.5%**): Hot Wheels runs its own 3D software renderer inside guest x86 code (including `cpu::exec::string_op` at 2.1% incl). Also notable: `Machine::step`/`tick`/`stack_args` + heap allocator overhead accounts for **~9.1%** of wall time due to high shim-call frequency (timer polling + DirectInput/DirectSound).*

### 3.3 `lego-alpha-team` — `first-mission.route` (163,288 samples, 197.15 s)

| # | Self % | Incl % | Function |
|---:|---:|---:|---|
| 1 | 30.1% | 71.3% | `cpu::interp::Interp::run` |
| 2 | 24.8% | 36.6% | `cpu::exec::execute` |
| 3 | 10.1% | 12.2% | `raster::triangle` |
| 4 | 7.0% | 8.5% | `cpu::x87::execute` |
| 5 | 4.6% | 4.6% | `cpu::interp::Interp::run_at` |
| 6 | 4.4% | 4.4% | `semaphore_wait_trap` *(4-band `raster::triangles` worker wait)* |
| 7 | 4.1% | 4.1% | `guest::Arena::slice_mut` *(Interp::run 3.2%, exec::execute 0.8%)* |
| 8 | 2.0% | 2.0% | `raster::Texture::texel_at` |
| 9 | 1.7% | 23.5% | `machine::Machine::tick` |
| 10 | 1.6% | 1.6% | `inflate::block` *(cabinet unpack during boot/mission load)* |
| 11 | 1.2% | 1.2% | `cpu::flags::Flags::sub` |
| 12 | 1.1% | 1.1% | `_platform_memmove` |
| 13 | 0.7% | 0.7% | `cpu::flags::Flags::logic` |
| 14 | 0.7% | 0.7% | `cpu::flags::Flags::add` |
| 15 | 0.6% | 1.1% | `guest::Arena::write` |
| 16 | 0.4% | 0.4% | `cpu::x87::memory_form::{closure#0}` |
| 17 | 0.4% | 0.4% | `cpu::exec::shift` |
| 18 | 0.3% | 0.3% | `cpu::flags::Flags::inc_dec` |
| 19 | 0.3% | 0.3% | `win32::amstream::put_picture` |
| 20 | 0.2% | 0.2% | `Option::map_or` inside `cpu::exec::execute` |

*(Secondary route delta — `menu.route`, 46.35 s: `inflate::block` is **7.7%** of wall time unpacking cabinet files at boot, `iv50` + `amstream::put_picture` is **4.4%** for the 1,632-flip intro movie, and `Software rasteriser` is **9.9%**. During the 3D training mission itself, x87 climbs to ~9.8% and Software rasteriser to ~20.1%.)*

### 3.4 `lego-island` — `pizza-delivery.route` (105,687 samples, 126.45 s)

| # | Self % | Incl % | Function |
|---:|---:|---:|---|
| 1 | 20.2% | 25.5% | `cpu::exec::execute` |
| 2 | 17.2% | 42.6% | `cpu::interp::Interp::run` |
| 3 | 10.0% | 10.0% | `_platform_memmove` *(10.0% from `win32::ddraw::transfer` alone!)* |
| 4 | 8.3% | 9.3% | `raster::triangle` |
| 5 | 7.7% | 52.7% | `machine::Machine::tick` |
| 6 | 6.5% | 6.5% | `semaphore_wait_trap` *(4-band `raster::triangles` worker wait)* |
| 7 | 4.3% | 14.9% | `win32::ddraw::transfer` |
| 8 | 2.3% | 2.3% | `cpu::interp::Interp::run_at` |
| 9 | 2.1% | 2.1% | `DYLD-STUB$$memcpy` *(per-pixel `copy_from_slice` in `ddraw::transfer`)* |
| 10 | 1.9% | 28.0% | `win32::d3drm::draw_mesh` |
| 11 | 1.4% | 2.6% | `win32::pipeline::lit` |
| 12 | 1.3% | 3.4% | `win32::pipeline::clip` *(includes 0.9% `malloc/free`)* |
| 13 | 1.3% | 1.3% | `guest::Arena::slice_mut` |
| 14 | 1.1% | 1.1% | `cpu::flags::Flags::inc_dec` |
| 15 | 1.1% | 1.1% | `cpu::flags::Flags::sub` |
| 16 | 1.0% | 1.8% | `_xzm_free` *(total malloc/free 2.6%: `pipeline::clip` 0.9%, `d3d::rasterise` 0.5%)* |
| 17 | 1.0% | 1.0% | `cpu::flags::Flags::add` |
| 18 | 1.0% | 21.6% | `win32::d3d::rasterise` |
| 19 | 0.8% | 0.8% | `_platform_memset_pattern16` |
| 20 | 0.6% | 1.3% | `win32::pipeline::lit::{closure#0}` |

*(Secondary route delta — `maintainer-first-play.route`, 53.37 s: even more non-CPU-heavy — `Blits / DDraw` hits **19.7%** (`ddraw::transfer` 19.6% incl, `_platform_memmove` + `DYLD-STUB$$memcpy` = 15.1% self!), `Software rasteriser` is **27.7%** (`d3drm::draw_mesh` 27.6% incl), and `CPU interpreter` is only **50.3%**.)*

### 3.5 `lego-chess` — `checkmate.route` (63,969 samples, 81.41 s)

| # | Self % | Incl % | Function |
|---:|---:|---:|---|
| 1 | 23.7% | 61.9% | `cpu::interp::Interp::run` |
| 2 | 22.0% | 36.1% | `cpu::exec::execute` |
| 3 | 15.8% | 18.6% | `raster::triangle` |
| 4 | 9.2% | 11.3% | `cpu::x87::execute` *(highest x87 share on the Roster: 11.4% total!)* |
| 5 | 7.7% | 7.7% | `semaphore_wait_trap` *(4-band `raster::triangles` worker wait)* |
| 6 | 2.9% | 2.9% | `cpu::interp::Interp::run_at` |
| 7 | 2.8% | 2.8% | `raster::Texture::texel_at` |
| 8 | 2.3% | 32.4% | `machine::Machine::tick` |
| 9 | 2.2% | 2.2% | `guest::Arena::slice_mut` |
| 10 | 1.2% | 1.2% | `cpu::flags::Flags::add` |
| 11 | 1.1% | 1.1% | `_platform_memmove` |
| 12 | 1.0% | 1.0% | `cpu::flags::Flags::sub` |
| 13 | 0.9% | 1.7% | `guest::Arena::write` |
| 14 | 0.9% | 0.9% | `cpu::flags::Flags::logic` |
| 15 | 0.5% | 0.5% | `cpu::exec::shift` |
| 16 | 0.4% | 0.4% | `cpu::x87::memory_form::{closure#0}` |
| 17 | 0.4% | 0.4% | `cpu::flags::Flags::inc_dec` |
| 18 | 0.3% | 0.6% | `_xzm_free` |
| 19 | 0.3% | 27.4% | `win32::d3d::rasterise` |
| 20 | 0.3% | 1.2% | `cpu::x87::write` |

*(Secondary route delta — `throne-room.route`, 5.48 s: `machine::Machine::tick`/`step`/`stack_args` + `malloc/free` is **15.3%** (`malloc/free` alone is 5.2%), `iv50` + `avifile` video is **4.0%**, `Audio` is **1.8%**, and `state_digest` is **1.7%** because the run is short.)*

---

## 4. `ci.sh` Wall-Time Share Across the Five Titles

`scripts/ci.sh` runs `cargo fmt`, `cargo clippy`, `cargo test --workspace --release`, and `cargo test -p lego-legacy-core --lib`. `cargo test --workspace --release` executes each crate's integration test binary **sequentially**, while tests inside a single binary run in parallel across the host's 10 cores (with determinism gates executing each major route twice via `OnceLock` pairs).

Using the measured single-route wall times and each title crate's committed test roster (`titles/*/tests/*.rs`):

| Title Crate | Primary Route Wall (s) | Share of 5-Primary Sum | Heavy Test Runs in `ci.sh` (incl. determinism pairs) | Est. Crate Critical-Path Wall in `ci.sh` |
|---|---:|---:|---|---:|
| `lego-alpha-team` | 197.15 | **32.1%** | `first-mission` (23.4B ×2), `select-adventure` (14.0B), `menu` (5.4B ×2), `first-light` (0.6B ×3) | ~210–250 s *(longest single crate)* |
| `lego-island` | 126.45 | **20.6%** | `pizza-delivery` (10.7B ×2), `closed` (13.1B), `first-ride` (6.35B ×2), `first-save` (6.03B ×2), `held` (6.07B), `closed-by-hand` (5.0B ×2), `again` (4.5B), `info-center` (3.97B ×2), `maintainer-play` (3.34B ×2) | ~160–210 s *(highest aggregate CPU-s across 16+ long runs)* |
| `lego-rock-raiders` | 118.38 | **19.3%** | `first-mission` (14.5B ×2), `level-select` (8.04B ×2), `first-light` (8.01B ×2) | ~130–160 s |
| `hotwheels-stunt-track-driver` | 90.43 | **14.7%** | `first-race` (13.6B ×2), `first-light` (2.2B ×3) | ~95–115 s |
| `lego-chess` | 81.41 | **13.3%** | `checkmate` (12.0B ×2), `vs-computer` (9.5B), `save-game` (6.4B ×2), `load-game` (5.4B ×2), `intro` (3.7B ×2), `throne-room` (3.3B ×2), `skip` (3.0B ×2) | ~95–125 s |
| **Total (5 titles)** | **613.81** | **100.0%** | — | **~11–14 min** *(matches `ci.sh` comment: "~10 min optimised")* |

*Speeding up the CPU interpreter + software rasteriser directly cuts `ci.sh` wall time across all five crates (`lego-alpha-team` and `lego-island` account for >50% of both single-route and CI wall time).*

---

## 5. High-Signal Findings for Hotspot Triage (`#248`)

1. **CPU interpreter is the undisputed #1 bottleneck (55.8%–97.5% of wall time; 74.9% wall-weighted across the 5 primary routes):**
   - `Interp::run` + `exec::execute` alone consume **37.4%–65.4%** self-time (55%–81% inclusive).
   - `Interp::run_at` (decode-cache table lookup) adds **2.3%–6.8%** self-time.
   - `cpu::x87::execute` is a major secondary CPU hot spot on 3D-heavy guest math titles: **11.4%** total on `chess-checkmate`, **8.7%** on `alphateam-first-mission`, **3.9%** on `hotwheels-first-race`.
   - Eager `cpu::flags` (`add`, `sub`, `logic`, `inc_dec`) takes **2.9%–7.2%** across every title.
   - `guest::Arena::slice_mut` (bounds + self-modifying-code + watch checks per guest memory access) takes **1.3%–4.7%** self-time from the CPU loop.
2. **Software rasteriser is #2 overall (15.3%–28.5% on the four D3D titles; 18.1% wall-weighted):**
   - `raster::triangle` + 4-band `raster::triangles` wait (`semaphore_wait_trap`) + `Texture::texel_at` is **26.7%** of `chess-checkmate`, **17.0%** of `alphateam-first-mission`, **16.1%** of `island-pizza-delivery`, and **12.8%** of `rockraiders-first-mission`.
   - On `lego-island`, the D3DRM/transform/lighting/clipping front end (`d3drm::draw_mesh`, `pipeline::lit`, `pipeline::clip`, `d3d::rasterise`) adds another **10.9%–11.4%** (including `1.4%` `malloc/free` inside `pipeline::clip` and `d3d::rasterise`).
3. **`win32::ddraw::transfer` is a huge, localized Rust-only win on `lego-island` (15.0%–19.7%) and `lego-rock-raiders` (4.5%–5.7%):**
   - Inspecting `core/src/win32/ddraw.rs:1404–1427` explains why `_platform_memmove` + `DYLD-STUB$$memcpy` alone hit **12.1%–15.1%** of `lego-island`: `transfer` allocates a `Vec` per scanline (`.to_vec()`), performs per-pixel integer division `(x - dx0) * sw / dw` even for 1:1 unscaled blits, and calls `row[out..out + bytes].copy_from_slice(pixel)` with a runtime `bytes` length — emitting an external `DYLD-STUB$$memcpy` call **for every single pixel**.
4. **Shim-dispatch allocations (`Machine::step`/`tick`/`stack_args`) are visible on high-shim-rate routes:**
   - `machine::Machine::stack_args` (`core/src/machine.rs:1090`) allocates a `Vec<u32>` per call, driving `malloc/free` to **5.2%** on `chess-throne-room` and **2.3%** on `hotwheels-first-race`, while `Machine::tick` self-time reaches **7.7%–9.1%** on `lego-island`.
5. **Video codecs, Audio, Host, and `inflate` are NOT primary assembly targets:**
   - `iv50` / `avifile` / `amstream` peak at **4.4%** (`alphateam-menu`) and drop to **0.0%–2.1%** on gameplay routes.
   - Audio (`sound::Mixer`, `synth`, `u128_div_rem`) is **0.1%–1.8%** across all 9 routes.
   - `inflate::block` is a one-time boot/cabinet-unpack cost (**7.7%** on `alphateam-menu`, **1.0%–1.6%** on full missions).
   - `Host` (`headless::present`) is **0.1%–0.8%**.
