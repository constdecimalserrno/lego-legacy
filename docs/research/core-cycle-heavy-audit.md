# Core Cycle-Heavy Subsystem Audit & Determinism Invariants

Audit of `core/src/` at commit `ca46366` under workspace `[profile.release]` (`rustc 1.98.0`, `opt-level = 3`, `codegen-units = 1`, `Cargo.toml:21-29`). Answers wayfinder research ticket #245 for map #243 and feeds seam design (#247) and profiled triage (#248).

---

## Executive Summary

| # | Subsystem & Loop | Primary Source | Dominant Cost / LLVM Blocker | Rust-Only First? | Suitability Rating |
|---|---|---|---|---|---|
| 1 | Interpreter run loop & dispatch | `core/src/cpu/interp.rs:321-358`, `exec.rs:288-338` | Centralized `match` BTB mispredicts; ~56 B `Insn` + parallel `Fast`; per-insn `count`/`outside`/`eip` checks | **Yes** (compact IR + hoist loop invariants) | **Static `global_asm!` candidate** (threaded dispatch, register-pinned flat32 core) |
| 2 | Guest memory writes & code-page check | `core/src/guest/mod.rs:204-271, 356-388`, `exec.rs:92-112, 266-281` | Per-write page-range loop in `wrote_over_code`; `#[inline(always)]` `note_written`; byte-loop writes | **Yes** (single-page fast path + `write_unaligned`) | **Rust-only refactor** (inline into asm only if #1 goes to asm) |
| 3 | `rep movs` / `rep stos` string ops | `core/src/cpu/exec.rs:1217-1313` | Element-by-element checked reads/writes block `memcpy`/`memset` lowering | **Yes** (bulk span check + `copy_from_slice`/`fill`) | **Rust-only refactor** |
| 4 | x86 arithmetic flag materialisation | `core/src/cpu/flags.rs:103-169` | Dynamic `Width` shifts/masks + `u64` widening; ~5% total ceiling (#69) | **Yes** (`add32`/`sub32`/`logic32` via `overflowing_*`) | **Rust-only refactor** / Skip standalone asm |
| 5 | x87 FPU & pure-Rust `maths` | `core/src/cpu/x87.rs:375-405, 485-683, 693-1105` | Runtime escape/modrm decode; split 2×`u32` `f64` I/O; serial FP divisions in Taylor series | **Yes** (single 8 B I/O, pre-decoded ops) | **Skip asm** (serial, ULP-locked to `synth.rs` table hash) |
| 6 | Software rasteriser span inner loop | `core/src/raster.rs:574-710, 137-256` | Un-specialized uber-loop; early `continue`s; `rem_euclid` `sdiv`; dynamic `Channels`; per-pixel `1.0/rhw` | **Yes** (monomorphized states/formats + POT mask + 3-phase span) | **Top `core::arch` SIMD candidate** (NEON/SSE2 4-lane RGBA + 4-px span; **no FMA**) |
| 7 | 3D transform, lighting, clip & draw prep | `core/src/win32/d3d.rs:1865-2398`, `d3drm.rs:2114-2293`, `pipeline.rs:201-398`, `scene.rs:329-470` | Per-triangle `Vec` allocs in `clip()`/`rasterise()`; ~25 `BTreeMap` lookups/draw; `settled` NaN iterator | **Yes** (stack clip buffer, scratch `Vec`s, flat state arrays) | **Rust-only refactor** (Skip asm unless >5% post-cleanup) |
| 8 | 32-voice DLS wavetable synth | `core/src/synth.rs:138-163, 434-484, 551-569` | Frame-outer loop; 2× 64-bit `div_euclid`/`rem_euclid` + 3× `OnceLock` per voice/sample; per-frame `retain` | **Yes** (block-render between MIDI events + steady cache + div-free `pow2`) | **Rust-only first**, then **`core::arch` SIMD candidate** (steady-voice 4-sample mix) |
| 9 | PCM mixer & resampler | `core/src/sound.rs:236-247, 290-333, 462-490` | 2× `&dyn GuestMem` virtual calls/sample; per-sample `/ RATE` & `% frames`; `f64::round()` | **Yes** (block-render + hoist `mem.slice` + fast `rate == RATE`) | **Rust-only refactor** (Skip asm unless >5%) |
| 10 | IV50 decoder (bitstream, slant, motion) | `core/src/iv50.rs:98-112, 196-215, 1189-1318` | 8 B copy per bit; 1-bit unary loop; `kind`/`add` branch inside `motion` inner loop; 2× tile clones/frame | **Yes** (64-bit bitbuf + VLC LUT; hoist `motion` branches; `split_at_mut`) | **`core::arch` SIMD candidate** (`slant8`/`slant4` 8×8 `i32x4` + `motion` `i16x8`, if ≥5%) |
| 11 | IV50 YUV 4:1:0 → RGB565 / 555 / BGR24 | `core/src/iv50.rs:60-84`, `amstream.rs:510-550`, `avifile.rs:973-995` | Recomputes chroma math + `x/4` indexing per pixel instead of per 4×4 block | **Yes** (4-pixel horizontal chroma hoist + row slices) | **`core::arch` SIMD candidate** (8-wide integer BT.601 + pack, if ≥5%) |
| 12 | DirectDraw blits (`transfer`) & `DrawDibDraw` | `core/src/win32/ddraw.rs:1383-1429`, `avifile.rs:1100-1143` | `to_vec()` alloc per scanline; 64-bit div per pixel even at 1:1; byte-by-byte key compare | **Yes** (1:1 `memcpy` fast path; typed key compare; hoist `sx` LUT) | **Rust-only first**; optional **`core::arch` SIMD** for 1:1 keyed blits |
| 13 | Presentation conversion & host scaling | `core/src/machine.rs:1964-2043`, `host/headless.rs:107-135`, `host/x11.rs:129-147` | 8-bit calls `arena.read_u32` 307k×/flip; 16-bit indexed bounds; `Headless` allocs/scans 1.2 MB/flip | **Yes** (hoist 256-entry palette; row slices; `Headless` zero-alloc scan) | **Rust-only first**; trivial **SIMD** for RGB565→BGRA32 if ≥5% |
| 14 | MSV1, MS-ADPCM, `Stretch`, `decode.rs`, `spin.rs`, `Sha256` | `msv1.rs`, `sound.rs:115-175`, `host/stretch.rs`, `decode.rs`, `spin.rs`, `hash.rs` | Cold, sub-ms, or strictly serial IIR/variable-length loops | Minor cleanup (`msv1` hoist depth) | **Skip** (revisit `Sha256` only if CI profiling flags `state_digest`) |

---

## Global Build & Golden Rule 1 Determinism Contract

Every acceleration candidate in `core/` is bound by these hard invariants verified in source:

1. **Compiler baseline (`Cargo.toml:21-29`):** `rustc 1.98.0`, `edition = "2021"`, `[profile.release]` uses `opt-level = 3`, `debug = true`, `codegen-units = 1` (full crate-wide inlining inside `core`). Zero external crates (`core/tests/zero_crates.rs:10-70`, ADR-0001).
2. **No dynamic codegen or guest overrides (ADR-0004, Golden Rule 2):** Static compiled host assembly/SIMD only; guest bytes execute unchanged (except exact clock-spin passing in `core/src/spin.rs`).
3. **No host `libm` transcendentals (`core/tests/x87.rs:310-347`):** Source-scanning test forbids `.sin()`, `.cos()`, `.tan()`, `.ln()`, `.log2()`, `.exp()`, `.powf(`, `.atan(`, `.atan2(`, `.exp2(` anywhere under `core/src/`. Only IEEE-754 `.sqrt()` is permitted.
4. **No FMA contraction:** Rust never fuses `a * b + c` into FMA unless `mul_add` is explicit (`core/src/synth.rs:75-77`). Any `aarch64` NEON or `x86_64` AVX2/FMA kernel **must not** emit `fmla`/`vfmadd` for `a * b + c`; it must execute separate rounded `mul` then `add` to stay bit-exact with the Rust oracle.
5. **Exact IEEE-754 division & NaN propagation:**
   - Never substitute approximate reciprocals (`frecpe`/`rcpss`) for `1.0 / x` without proving exact bit-equivalence (e.g., `raster.rs:634` `1.0 / here[1]`).
   - Guest-visible float results use `x87::answer` (`core/src/cpu/x87.rs:325-340`, indefinite `0xfff8_0000_0000_0000`) and `x87::answer_single` (`x87.rs:349-360`, indefinite `0xffc0_0000`) so `aarch64` and `x86_64` produce identical NaN payloads (`core/src/win32/scene.rs:477-491`).
6. **Observable state & pinned oracles:**
   - `Machine::state_digest()` (`core/src/machine.rs:1267-1337`) hashes GPRs, `eip`, `EFLAGS`, segments, `cursor`, `flips`, input/queue state, and all writable guest memory (images, modules, `VirtualAlloc` regions, heap, 1 MiB stack, TIB/PEB, surface).
   - Audio determinism is pinned by SHA-256 of `synth.rs` tables (`c42f41d9...`, `synth.rs:1145-1148`), synthetic render (`5185c0f5...`, `synth.rs:1149-1152`), and `GM16.DLS` render (`68a20ae6...`, `core/tests/synth.rs:99-102`).
   - Video decoders are pinned by DIB SHA-256 hashes in `core/tests/shims.rs:3211, 3776, 4168, 4187`.
   - CPU conformance is gated by `>= 877` SingleStepTests 80386 encodings (`core/tests/conformance.rs:37`).

---

## Subsystem 1: CPU Interpreter

### 1.1 Interpreter Run Loop & Dispatch
- **Files & lines:** `core/src/cpu/interp.rs:30-62, 155-272, 321-358`; `core/src/cpu/insn.rs:73-99, 273-287`; `core/src/cpu/exec.rs:49-89, 236-338, 347-1011`.
- **Loop structure & data layout:**
  - `Interp` keeps straight-line basic-block runs (`runs: Vec<(u32, u32)>`, capped at `MOST_IN_A_RUN = 64`, `interp.rs:67`) indexed by a 3-level page table (`directory -> regions -> pages`, `interp.rs:55-57`). Previously measured at ~3.3% total lookup cost (#71, `interp.rs:14-29`).
  - Decoded instructions live in two parallel `Vec`s: `insns: Vec<Insn>` and `shapes: Vec<exec::Fast>` (`interp.rs:41-44`). `Insn` embeds three `Option<Op>` operands where `Op::Mem(MemRef)` carries 6 fields (`seg`, `base`, `index`, `base_shift`, `disp`, `addr16`), making `Insn` ~48–56 bytes (almost a full cache line per instruction).
  - Outer loop (`interp.rs:329-356`) calls `run_at()` per run; inner loop (`interp.rs:334-355`) steps `index in first..first + length`, re-checking `retired >= count || outside.contains(&cpu.eip)` (`335`), reading `self.insns[index].len` (`340`), calling `execute_one` (`343`) which re-reads `self.insns[index]` (`250`) and `self.shapes[index]` (`258`), updating `retired` and `cpu.tsc` (`345-346`), and checking `cpu.eip != next || cpu.seg_base[CS] != base` (`352`) on **every** instruction.
  - `execute_fast` (`exec.rs:288-338`) covers 5 shapes (~46% of dynamic instructions on Alpha Team's menu, #70), but still re-destructures `Option<Op>` from `Insn` and calls `effective()` (`exec.rs:49-89`), which branches on `m.addr16`, `m.base`, `m.index`, `m.base_shift`, and indexes `cpu.seg_base[m.seg.index()]`.
- **LLVM codegen / microarchitectural blockers:**
  - **Single indirect-branch dispatch bottleneck:** `execute_fast` + `execute` compile to centralized jump tables inside one loop body; every instruction shares the same indirect branch site, causing high BTB/TAGE mispredictions (direct-threaded / per-handler indirect tail jumps give each opcode shape its own branch history).
  - **Cache footprint & redundant checks:** Reading ~56 B `Insn` + separate `Fast` byte + redundant per-instruction `retired >= count`, `outside.contains`, `eip != next`, and `seg_base[CS]` checks even though `transfers()` (`interp.rs:277-291`) guarantees only the final instruction in a run can change `eip` or `CS`.
  - **Register spilling:** `Cpu` fields (`r[0..8]`, `eip`, `tsc`) are mutated through `&mut Cpu` across non-inlined calls to `execute()`, preventing LLVM from keeping guest GPRs live in host registers across straight-line instructions.
- **Rust-only refactor before assembly:**
  1. Clamp `let steps = (length as u64).min(count - retired)` once per run (and stop decoding a run before `Boundary::RANGE`), eliminating per-instruction `retired >= count` and `outside.contains` checks.
  2. Split non-terminal instructions (guaranteed fallthrough) from the final instruction in a run, eliminating per-instruction `cpu.eip != next || cpu.seg_base[CS] != base` checks.
  3. Replace parallel `Vec<Insn>` + `Vec<Fast>` with a compact 16-byte tagged `FastInsn` (or compact operand struct) carrying decoded `dst_reg`, `src_reg`, `base_reg`, `idx_reg`, `scale`, `disp`, `len`, and `flat32` flag, falling back to full `Insn` only for `Slow` ops.
  4. Specialize `effective_flat32` (no `addr16`, no `base_shift`, skip `seg_base` unless `Seg::Fs`).
- **Determinism invariants:**
  - Exact retirement count (`cursor` and `cpu.tsc` +1 per retired insn; `Stop::Interrupt` retires +1; all other `Stop`s retire 0 and restore `cpu.eip = at`, `interp.rs:263-270, 348-349`).
  - `mem.code_written` invalidation before executing any run (`interp.rs:158-167`, Alpha Team self-modifying/dynamically unpacked code #58/#65).
  - Real mode (`Mode::REAL`) conformance (`core/tests/conformance.rs`) must remain untouched (flat32 fast paths can be gated on `!self.mode.real`).
- **Suitability:** **Rust-only refactor first**, then **High-value static `global_asm!` candidate** (direct-threaded dispatch keeping `r[0..8]`, `eip`, `arena_base`, `code_bitmap`, and `pc` pinned in callee-saved registers for the top ~15–25 flat32 instructions, calling back into Rust `execute()` on slow ops/faults/end-of-run).

### 1.2 Guest Memory Access & Code-Page Tracking
- **Files & lines:** `core/src/guest/mod.rs:147-152, 204-271, 356-388`; `core/src/cpu/exec.rs:92-112, 266-281`.
- **Loop structure & data layout:**
  - Every guest store (`write_mem`, `write32`) calls `Arena::slice_mut(addr, len)` (`guest/mod.rs:248-271`), which runs:
    1. `in_bounds(addr, len)` (`147-152`).
    2. `wrote_over_code(addr, len)` (`380-388`): computes `(first..=last)` 4 KiB page range and runs an `.any(...)` loop with bounds-checked `self.code.get(page / 64)`.
    3. `note_written` (`356-376`), marked `#[inline(always)]` despite being cold.
    4. `if self.watching { self.hold(addr, len); }` (`259-261`).
    5. Back in `exec.rs:108-110, 277-279`, writes bytes via `for (to, byte) in s.iter_mut().zip(value.to_le_bytes())`.
- **Codegen blockers:**
  - Loop in `wrote_over_code` + `Vec::get` bounds check on every single `push`/`mov [mem], r32` even when a 4-byte write stays inside one 4 KiB page (99.9%+ of stores).
  - `#[inline(always)]` on `note_written` inflates hot store paths (unlike `compare`/`hold` which are `#[cold]`).
  - `slice_mut` + byte-zip loop in `write_mem` prevents clean single-instruction unaligned store emission across variable widths.
- **Rust-only refactor:**
  - Dedicated `Arena::read_u32_unchecked_len` / `write_u32_fast` (and `u8`/`u16`): single `in_bounds` check (dropping the `len == 0` branch), single-page bitmap test when `(addr & 0xfff) <= 4092` (unchecked bitmap index since `code` is sized to `arena_len / 4096 / 64 + 1`), mark `note_written` `#[cold] #[inline(never)]`, and use `ptr::read_unaligned` / `ptr::write_unaligned` directly on `mapping.as_ptr().add(addr as usize)`.
- **Determinism invariants:** Faults below `floor` (`DOS_MEMORY = 0x10_0000`, `NULL_PAGE = 0x1000` for SEH access violations #132) and above `mapping.len()` must report exact `(addr, len, write)`. `code_written` and `watching` must trigger identically on any overlapping write.
- **Suitability:** **Rust-only refactor** (no standalone asm needed; directly reusable inside threaded asm if 1.1 is built).

### 1.3 `rep` String Instructions (`movs`, `stos`, `lods`, `cmps`, `scas`)
- **Files & lines:** `core/src/cpu/exec.rs:1217-1313`.
- **Loop structure & blockers:** Executes 1 element per iteration inside `loop {}`, re-extracting `src_seg`, checking `insn.addr32`, calling checked `read_mem`/`write_mem` (with full `wrote_over_code` + `watching` + byte-loop write), and updating `ESI`/`EDI`/`ECX` per iteration. LLVM cannot hoist bounds checks or lower `rep movs`/`rep stos` to `memcpy`/`memset`.
- **Rust-only refactor:** For `rep movs`/`rep stos` with `addr32 && !cpu.flags.df && !mem.watching`, verify total span `count * width` fits in bounds without wrapping `u32` (and for `movs`, non-overlapping or forward-safe), check `wrote_over_code` once over the destination span, then execute `copy_from_slice` / `fill` (which LLVM lowers to SIMD `memcpy`/`memset`) and advance `ESI`/`EDI`/`ECX` in one shot. Fall back to element-by-element on any edge case (fault, wrap, overlap, `watching`).
- **Determinism invariants:** Exact partial register/memory state if a `rep` instruction faults mid-way; exact single instruction retirement (`cursor += 1`, `exec.rs:1215-1216`).
- **Suitability:** **Rust-only refactor**.

### 1.4 Flag Materialisation
- **Files & lines:** `core/src/cpu/flags.rs:50-169`; `core/src/cpu/exec.rs:187-207`.
- **Data layout & blockers:** 7 separate `bool` fields + `rest: u32`. Previously measured (#69, `flags.rs:8-32`): all 6 eager arithmetic flags cost ~5% of Alpha Team's menu route (`PF`+`AF` ~2.3%), making lazy flags low ROI. However, `add`/`sub`/`logic` take dynamic `Width`, widen to `u64`, and compute `w.mask()`/`w.bits()`/`w.sign_bit()` dynamically; `(result as u8).count_ones()` lacks `popcnt` on baseline `x86_64` SSE2.
- **Rust-only refactor:** Add `add32`/`sub32`/`logic32` using `u32::overflowing_add`/`overflowing_sub` (directly emitting `adds`/`subs` + `cset` on `aarch64`, `add`/`sub` + `setc`/`seto` on `x86_64`) and a nibble/LUT parity helper (`0x6996u16 >> ((b ^ (b >> 4)) & 0xf)` — 3 ALU ops on both ISAs).
- **Determinism invariants:** Exact 80386 flag behavior across all 6 arithmetic flags + untouched `rest` bits (`core/tests/conformance.rs`).
- **Suitability:** **Rust-only refactor**; **Skip** standalone asm (only consider native host flag capture if fused into a threaded `global_asm!` CPU loop).

### 1.5 x87 FPU & Pure-Rust Transcendentals
- **Files & lines:** `core/src/cpu/x87.rs:34-46, 117-132, 313-405, 464-683, 693-1105`.
- **Loop structure & blockers:**
  - Registers are `f64` (not 80-bit extended, `x87.rs:28-32`). `execute()` re-decodes `(escape, modrm)` on every x87 instruction (`693-1093`).
  - `x87::read` / `x87::write` for `Format::Double` (`381-385, 398-403`) issue two separate `mem.read_u32`/`write_u32` calls (double bounds/code-page checks).
  - `round_half_to_even` (`1098-1105`) calls `x.round()`, `x.trunc()`, and `rounded % 2.0` (`fmod` libcall!) instead of single-instruction IEEE-754 `x.round_ties_even()` (verify NaN/edge-case parity before switching).
  - `maths::{sin, cos, tan, atan, atan2, log2, pow2}` (`485-683`) are scalar Taylor-series loops (8–29 iterations) with loop-carried FP multiplication and per-term division (`term *= -x2 / k` or `sum += term / denom`). Strictly serial; LLVM cannot vectorize or replace `/ k` with `* (1.0 / k)` without risking 1-ULP drift.
- **Hard determinism invariants:**
  - `core/tests/x87.rs:310-347` forbids host `libm`.
  - `core/src/synth.rs:1145-1148` pins the SHA-256 of `Tables` built by `maths::pow2`, `maths::sin`, `maths::log2`. **Any 1-ULP change in `maths` breaks `synth.rs` and every title's `state_digest()`.**
  - `answer()` (`325-340`) enforces identical NaN bits across `aarch64` and `x86_64`.
- **Suitability:** **Rust-only refactor** for 8-byte `Double` I/O and pre-decoded x87 ops if x87 appears in profiles; **Skip asm** for `maths` (serial, ULP-locked).

### 1.6 Decoder, Machine Burst Loop & Spin Passing
- **Files & lines:** `core/src/cpu/decode.rs:82-850`; `core/src/machine.rs:511-600, 717-798, 1090-1255, 1679-1818`; `core/src/spin.rs:113-199`.
- **Findings:**
  - `decode.rs` runs only on cache miss (~300k decoded insns over an entire Alpha Team route, `interp.rs:177-179`) — cold.
  - `machine.rs`: burst quantum is `BURST = 65_536` (`839`). Boundary crossings are cached in `self.bound` (`1106-1160`, ~30 ns/crossing, `521-527`), though `stack_args()` (`1090-1094`) heap-allocates a `Vec<u32>` on every Win32/COM crossing, and `deadlocked()` (`1791-1818`) allocates a `Vec` on every burst end even when single-threaded (`threads.len() < 2`). Both are trivial Rust-only cleanups.
  - `spin.rs`: `pass()` early-outs on `self.last.is_none()` or `eip` mismatch (`152-157`); cold outside active clock spins.
- **Suitability:** **Skip** asm (trivial Rust-only allocation cleanup in `stack_args` / `deadlocked`).

---

## Subsystem 2: Software Rasteriser & 3D Pipeline

### 2.1 Triangle Span Inner Loop & Texture Sampling
- **Files & lines:** `core/src/raster.rs:134-257, 467-711` (hot inner loop `615-709`).
- **Loop structure & data layout:**
  - Per triangle (`triangle()`, `467-711`): builds 12 `f32` plane equations `(dx, dy, k)` (`508-547`), sets `carried` (`2`, `4`, `8`, or `12`).
  - Per scanline `py in top..bottom` (`574-710`): clips `[from, to]` against 3 edges, initializes `value: [f32; 12] = planes.map(|(px, py_, k)| px * start + py_ * y + k)` (`610`).
  - Per pixel `px in from..=to` (`615-709`):
    1. Steps `carried` planes: `for (v, s) in value.iter_mut().zip(step).take(carried) { *v += s; }` (`617-619`).
    2. Edge-end check `if px < inside_from || px > inside_to` (`620-626`).
    3. Depth test (`target.depth.get(at)` + dynamic `Compare::passes`, `629-633`).
    4. Perspective divide `1.0 / here[1]` (`634`) + diffuse/specular multiplies (`635-647`).
    5. Texture sample (`sample()` `225-256`, `bilinear16()` `169-207`, `texel_at()` `210-222`): calls `across()`/`down()` (`143-164`) with `x.rem_euclid(w)` (signed integer division when wrapping!), dynamic `Channels` shifts/masks/steps, and slice `.get()` bounds checks.
    6. Colour key (`654-656`), stage `Combine` (`657-662`), optional specular (`663-667`), fog (`670-675`), alpha test (`676-681`), target pixel bounds check (`683`), alpha blend (`686-695`, 2× 10-way `Blend::factor` matches), `pixel_channels.value()` pack (`696-703`), depth write (`704-708`).
  - Previously measured (`raster.rs:460-466, 718-732`): `triangle` + `texel_at` + bands = **~16.9%** of Alpha Team's menu; Rock Raiders' rockfall was 26 ms/frame on 1 thread (5.5 ms on 4 bands).
- **Concrete LLVM codegen & auto-vectorisation blockers:**
  1. **Uber-loop state explosion:** One monolithic loop tests ~15 loop-invariant `States` / `PixelFormat` / `Texture` branches per pixel.
  2. **Scalar early `continue`s:** Edge coverage, depth fail, colour key, alpha test, and slice bounds checks prevent LLVM from vectorizing across `px`.
  3. **Integer division in texture wrap:** `x.rem_euclid(w)` (`150, 162`) emits `sdiv`/`idiv` per coordinate, even though D3D textures are almost universally power-of-two (`x & (w - 1)`).
  4. **Dynamic `Channels` bitfield loops (`80-114, 191-205`):** Runtime shift/mask arrays defeat compile-time constant folding for RGB565, ARGB1555, ARGB4444, XRGB8888/ARGB8888.
  5. **Serial FP stepping recurrence (`617-619`):** `value[k] += step[k]` accumulates rounding sequentially pixel-by-pixel. Jumping ahead by `v0 + [0,1,2,3] * step` or `+= 4 * step` would diverge by 1+ ULP from scalar stepping!
     - **Key insight for bit-exact SIMD:**
       - *Intra-pixel 4-lane SIMD* (stepping `[z, rhw, u, v]` and `[r, g, b, a]` together) is **100% bit-exact** with scalar `+= step`.
       - *Horizontal 4-pixel SIMD* (vectorizing `1.0 / rhw`, `u*near`, `v*near`, `bilinear16` integer weights, `combine`, `fog`, `blend`, and `RGB565` pack across 4 consecutive interior pixels) is **100% bit-exact** if the 4 pixels' `here` inputs are produced by sequential scalar/intra-pixel `+= step` (or stepped across a 4-wide transpose after 4 cheap dependent `fadd`s) and uses exact `vdivq_f32`/`_mm_div_ps` + separate `mul`/`add` (NO FMA, NO `frecpe`/`rcpss`).
- **Rust-only refactor before assembly:**
  1. Hoist row slices `&mut target.depth[row_depth..]` and `&mut target.pixels[row_pixels..]` out of the `px` loop.
  2. Split `[from..=to]` into 3 loops: left edge (`covers`), interior (`inside_from..=inside_to`, zero edge-test branches), right edge (`covers`).
  3. Fast-path power-of-two texture wrap (`x & (w - 1)`) when `w.is_power_of_two() && !clamp`.
  4. Monomorphize the hottest `States` × `PixelFormat` combinations (e.g., RGB565 target, `LessEqual` Z, `Modulate`, nearest/bilinear16, opaque vs `SrcAlpha/InvSrcAlpha`, no fog/specular) with compile-time RGB565/ARGB1555/ARGB4444 bit shifts.
- **Hard determinism invariants:**
  - Exact plane setup order (`out.0 += ex * value / area`, `511-513`, no FMA) and bit-equal constant-depth shortcut (`527-531`, #210).
  - Exact top-left edge coverage (`edge`/`covers`, `804-818`).
  - Exact sequential `value[k] += step[k]`, exact IEEE-754 `1.0 / here[1]`, exact `bilinear16` `u32` Q16 weights (`172-206`), exact non-FMA blend `colour[i] * s[i] + under[i] * d[i]` (`693`), exact RGB565 rounding `(c.clamp(0.0, 1.0) * 255.0 + 0.5) as u32` (`109`).
- **Suitability:** **High-priority Rust-only refactor first**, then **Top `core::arch` SIMD candidate** (NEON + SSE2 4-pixel interior span kernels for the top specialized RGB565 textured/blended paths).

### 2.2 Banded Multithreading (`raster::BANDS`)
- **Files & lines:** `core/src/raster.rs:733-799` (test `1105-1158`).
- **Invariants:** `BANDS = 4` is a declared constant (ADR-0004), triggered when `area >= 64_000.0` (`WORTH_BANDS`). Splits `[top, bottom)` into 4 disjoint row slices; every band draws the full `list` in the exact same order. Determinism holds because each row's output depends solely on `(py, px)` and triangle order, never on thread scheduling. Previously measured (#72, `722-732`): worker pool and 8 bands showed zero wall-clock gain on Rock Raiders / Alpha Team.
- **Suitability:** **Keep as-is** (SIMD span kernels compose orthogonally inside each band).

### 2.3 3D Vertex Pipeline, Lighting, Clipping & Draw Prep
- **Files & lines:** `core/src/win32/d3d.rs:351-400, 1675-1695, 1865-2398`; `core/src/win32/d3drm.rs:1741-1907, 2114-2293`; `core/src/win32/pipeline.rs:57-439`; `core/src/win32/scene.rs:329-470`.
- **Loop structures & blockers:**
  - **Heavy per-draw / per-triangle heap allocations:** Every `draw()` / `draw_mesh()` allocates `indices`, `inputs`, `lights`, `prepared`, two `triangles` `Vec`s, and `list` (`d3d.rs:378-388, 1905, 2149, 2153, 2193, 2207, 2358`), PLUS `rasterise()` allocates a `Vec<Clipped>` inside `pipeline::clip()` (`pipeline.rs:369, 380`) and a `Vec<raster::Vertex>` (`d3d.rs:2368-2375`) **for every single triangle**!
  - **Per-draw state map lookups:** `d3d::draw` performs ~25 `BTreeMap` lookups on `device.render` and `device.stages` (`d3d.rs:1935-2080`).
  - **`scene.rs` NaN canonicalization (`settled` / `answer_single`, `scene.rs:333-336, 465-470`):** Constructing the operand iterator `(0..4).flat_map(...)` on every cell of `scene::multiply` / `transform` / `direct` can inhibit LLVM vectorization if not sunk behind a `#[cold]` NaN branch.
  - **`pipeline::lit` (`pipeline.rs:201-317`):** Specular highlights invoke `x87::maths::pow2` + `log2` per vertex per light (`224-226`).
- **Determinism invariants:** `settled()`/`answer_single()` exact NaN bits (`scene.rs:477-491`); exact FP add/mul order (no FMA); `0.0 + nz` `-0.0` canonicalisation (`pipeline.rs:431`); `x87::maths` for specular power.
- **Suitability:** **Rust-only refactor** (stack `[Clipped; 12]` / `[Vertex; 12]` clip buffer, reusable scratch buffers on `Immediate`, flat arrays for render/stage states, `#[cold]` NaN path in `scene.rs`). **Skip** asm unless profiling shows vertex T&L ≥ 5% after removing allocations.

---

## Subsystem 3: Audio

### 3.1 32-Voice DLS Wavetable Synthesiser
- **Files & lines:** `core/src/synth.rs:105-169, 238-298, 349-360, 433-485, 551-569`.
- **Loop structure & data layout:**
  - `Synth::frame()` (`551-569`) runs **one sample at a time** (22,050×/game-sec), iterating `self.voices` (AoS `Vec<Voice>`, up to `VOICES = 32`) and calling `self.voices.retain(|v| !v.done())` on every single sample frame (`567`).
  - Inside `Voice::frame()` (`434-484`, up to 705,600 calls/game-sec):
    - Re-computes `channel.pitch()` (`349-353`) and `channel.attenuation()` (`356-359`) per voice per sample.
    - Calls `pow2(cents)` **twice** per voice per sample (`444`, `454 -> 162`). Each `pow2()` loads `tables()` via `OnceLock` (atomic acquire) and executes **two 64-bit signed Euclidean divisions** (`cents.div_euclid(OCTAVE)` and `cents.rem_euclid(OCTAVE)` where `OCTAVE = 1200 << 16`, `140`). That is up to **1.4 M 64-bit divisions/game-sec**!
    - Calls `Lfo::value()` (`283-289`) which also touches `OnceLock` per sample.
    - Performs `u128` pitch step multiply (`444`), gathers `wave[whole]` and `wave.get(next)`, computes linear interpolation `a + (((b - a) * part) >> 32)` (`467`), stereo gain/pan `(sample * (gain * side) as i64) >> 32` (`468`), and loop wrap `% length` (`474`).
- **Codegen / auto-vectorisation blockers:**
  - Frame-outer / voice-inner structure forces pointer chasing (`instruments -> regions -> waves -> samples`) and prevents vectorizing consecutive samples of a voice.
  - 64-bit `div_euclid`/`rem_euclid` and `OnceLock` checks inside the innermost sample function.
- **Rust-only refactor before assembly (very high leverage):**
  1. **Block-render between queued MIDI events:** Render chunks of frames up to `self.queue.front().at` (or block end), outer-looping over `voice` and inner-looping over `frames`. Look up `region`, `wave`, `tables()`, `channel.pitch()`, and `channel.attenuation()` **once per voice per chunk**.
  2. **Steady-state fast path:** When pitch modulation is zero/steady (`lfo_pitch == 0`, `wheel_pitch == 0 || wheel == 0`, `pitch_depth == 0 || pitch_eg.stage == Sustain/Done`), compute `step` once for the entire chunk; likewise cache `gain` when volume is in `Sustain` with no LFO gain swing.
  3. **Division-free `pow2`:** `cents` spans only `[-9600<<16, 4800<<16]` (12 octaves) and attenuation `[0, 960<<16]`; replace 64-bit `div_euclid`/`rem_euclid` with exact fixed-point reciprocal multiplication + 1 correction step (bit-identical to Euclidean div/rem).
  4. Only run `self.voices.retain(...)` when `any_done` is true.
- **Hard determinism invariants:**
  - Pure integer `i64` accumulation across voices is commutative/associative (`i64` cannot overflow for 32 voices of 16-bit samples), so voice-outer block rendering is **mathematically bit-identical** to frame-outer rendering between MIDI message timestamps.
  - Must match all three pinned SHA-256 hashes: tables (`synth.rs:1145-1148`), synthetic render (`synth.rs:1149-1152`), and `GM16.DLS` render (`core/tests/synth.rs:99-102`).
- **Suitability:** **Major Rust-only refactor first** (expected 5×–10× reduction in synth CPU); then **`core::arch` SIMD candidate** for the steady-voice inner sample loop (4-sample linear interp + stereo mix) only if profiling still shows synth ≥ 5%.

### 3.2 PCM Mixer, Resampler & MS-ADPCM
- **Files & lines:** `core/src/sound.rs:115-175, 236-247, 290-333, 462-490`; `core/src/win32/dsound.rs:777-820`.
- **Loop structure & blockers:**
  - `Mixer::catch_up()` (`sound.rs:462-490`) runs frame-by-frame. Each `Source::Guest(pcm)` voice makes **two `&dyn GuestMem::slice` virtual calls per sample** (`238-240, 317`), computes `along()` (`u64` div/mod by `RATE = 22_050`, `290-293`), two `% frames` modulos if looping (`304`), and `f64` linear interpolation + `.round()` (`318-323` — on baseline `x86_64` SSE2, `f64::round()` calls `libm::round`!). Even `Source::Decoded` (already at `RATE`) runs `along()` div/mod every sample.
  - `Format::ms_adpcm()` (`115-175`) is a strictly serial IIR decoder called only at load/ACM-convert time (cold).
  - `dsound::weights()` (`dsound.rs:786-820`) runs only on `Play` / parameter change (cold).
- **Rust-only refactor:** Pass concrete `&Arena` (or hoist `mem.slice(pcm.store, pcm.bytes as usize)` once per voice per block); block-render up to `BLOCK_FRAMES` / voice end; fast-path `rate == RATE` (integer copy/scale with no `f64::round()`).
- **Hard determinism invariants:** Exact `f64` interpolation + `.round()` (half-away-from-zero, no FMA) for resampled `Source::Guest` voices; exact `catch_up` before `IDirectSoundBuffer::Lock` (`dsound.rs:725-727`); exact saturating `i32` add + `i16` clamp.
- **Suitability:** **Rust-only refactor**; **Skip** asm unless profiling shows PCM mixing ≥ 5%.

### 3.3 Host Time-Stretch & Audio Feed
- **Files & lines:** `core/src/host/stretch.rs:67-136`; `core/src/host/feed.rs:55-74`.
- **Findings:** `Stretch::block()` runs only when `pace < 1.0` (~360k float ops/sec in `best_offset`, <0.2 ms/sec); allocates `next: Vec<f32>` per 256-frame hop (`88`). `feed::fill_frames` drains 2 samples/iter from `VecDeque` (`60`). Host-only (does not affect guest state or `state_digest()`).
- **Suitability:** **Skip** (negligible cost).

---

## Subsystem 4: Video Codecs, Blits & Frame Presentation

### 4.1 Indeo 5 (`IV50`) Decoder
- **Files & lines:** `core/src/iv50.rs:98-112, 196-215, 455-497, 1118-1330`.
- **Loop structures & blockers:**
  - **Bitstream & Huffman (`Bits::bits` `98-112`, `Codebook::read` `196-215`):** Every `bit()` / `bits()` call copies up to 8 bytes into a stack `[0u8; 8]` array; `Codebook::read` loops one bit at a time for unary prefix then calls `reverse()`. Max code length is ≤ 13 bits (`164`), so a 64-bit bit-buffer + 2-level (or 8/13-bit) VLC lookup table eliminates 5–13 calls per symbol in pure Rust.
  - **Per-frame tile clones (`456-462`):** Clones `planes[0].tiles` macroblock arrays twice per frame just to borrow plane 0 alongside planes 1 & 2 (`split_at_mut(1)` removes both clones).
  - **Inverse 2D Slant Transform (`slant8` `1189-1205`, `slant4` `1207-1214`, `inverse` `1216-1265`):** Pure `i32` butterfly/reflect integer ops (`(a + b*2 + 2) >> 2`, `(a*4 - b + 4) >> 3`, `(t + 1) >> 1`). Scalar column gather/scatter (`c[i + 8*k]`) with conditional `columns[i]` skip and row-by-row `from_fn` blocks LLVM from executing 8 columns in 2× `i32x4` SIMD + in-register 8×8 transpose.
  - **Motion compensation (`motion` `1279-1318`):** `match kind { 0..=3 }` and `if add` sit **inside** the per-pixel `for j in 0..size` loop (`1301-1315`), and `size` (`4` or `8`) is dynamic, preventing LLVM from vectorizing 8-pixel `i16` rows. (Note: `(a + b) >> 1` is floor-average, **not** rounding `vrhadd`/`pavgw`, and 4-tap sums in `i32` before `>> 2`.)
  - **`copy_plane` (`1322-1330`):** Clean slice loop; LLVM auto-vectorizes `(i32::from(*v) + 128).clamp(0, 255) as u8`.
- **Hard determinism invariants:** 100% bit-exact integer output, pinned by DIB SHA-256 tests (`core/tests/shims.rs:3211, 3776`). Note LGPL-2.1-or-later header (`iv50.rs:1-3`).
- **Suitability:** **Rust-only refactor first** (64-bit bit-buffer + VLC LUT, hoist `kind`/`add`/`size` in `motion`, eliminate tile clones); then **`core::arch` SIMD candidate** for `slant8`/`slant4` (`i32x4` 2D transform) and `motion` (`i16x8`/`i32x4`) only if profiling shows IV50 ≥ 5%.

### 4.2 IV50 YUV 4:1:0 → RGB565 / RGB555 / BGR24 Conversion
- **Files & lines:** `core/src/iv50.rs:60-84`; `core/src/win32/amstream.rs:510-550`; `core/src/win32/avifile.rs:973-995`.
- **Loop structure & blockers:** `put_picture` (`amstream.rs:539-548`) and `dib_bits` (`avifile.rs:981-993`) call `picture.rgb(x, y)` per pixel, which recomputes `chroma_size()`, `(y/4)*cw + x/4`, and all BT.601 chroma multiplies (`409*e`, `-100*d - 208*e`, `516*d`) 16× per 4×4 chroma block, defeating auto-vectorisation.
- **Rust-only refactor:** Process 4 pixels per chroma sample horizontally (precomputing `r_off`, `g_off`, `b_off` once per 4 pixels and indexing row slices directly).
- **Hard determinism invariants:** Exact Microsoft integer BT.601 (`c = (y - 16) * 298`, `clip = ((x + 128) >> 8).clamp(0, 255)`, `iv50.rs:75-84`) and exact RGB565 / RGB555 / BGR24 packing (`core/tests/shims.rs:3211, 3776`).
- **Suitability:** **Rust-only refactor first**; clean **`core::arch` SIMD candidate** (8-wide `i16x8`/`i32x4` BT.601 + pack) if movie routes spend ≥ 5% here.

### 4.3 Microsoft Video 1 (`MSV1`) Decoder & DIB Export
- **Files & lines:** `core/src/msv1.rs:67-174`.
- **Findings:** Variable-length 4×4 block decode (`67-140`) is branchy and already sub-millisecond (`avifile.rs:889-890`). `to_dib_bits` (`150-174`) branches on `if bytes == 2` inside the inner pixel loop (`163`) — hoisting that check makes 16-bit DIB export a straight row `copy_from_slice` on little-endian hosts. Pinned against FFmpeg digests (`core/tests/shims.rs:3990, 4168, 4187`).
- **Suitability:** **Rust-only cleanup** (hoist depth check); **Skip** asm.

### 4.4 DirectDraw Blits (`transfer`), Fills & `DrawDibDraw`
- **Files & lines:** `core/src/win32/ddraw.rs:1383-1429`; `core/src/win32/mod.rs:1244-1261`; `core/src/win32/avifile.rs:1017-1143`.
- **Loop structures & blockers:**
  - **`ddraw::transfer` (`ddraw.rs:1403-1427`):**
    1. Calls `mem.slice(...).to_vec()` on **every scanline** (`1406-1409` — 480 heap allocs per 640×480 blit!).
    2. Executes a **64-bit signed division** `(x - dx0) * sw / dw` per pixel (`1416`) even when unstretched (`sw == dw`).
    3. Runs byte-by-byte key compare (`1420`) and dynamic-width `copy_from_slice` (`1424`) per pixel even when `key.is_none()`.
    4. Reads full destination row and writes it back via `mem.write` (triggering `wrote_over_code` per row).
  - **`avifile::draw_dib_draw` (`avifile.rs:1102-1141`):** Copies whole DIB into a `Vec` (`1106`), allocates another `Vec<u32>` per unique source row (`1127`), calls dynamic `format.encode()` per pixel (`1131`), and divides `(dx - x) / scale_x` per destination pixel (`1134`).
  - **`win32::fill` (`win32/mod.rs:1244-1261`):** Allocates 1 row and calls `ctx.mem.write` per scanline (minor win: single `slice_mut` + row fill).
- **Rust-only refactor:**
  - `ddraw::transfer`: (a) 1:1 unkeyed fast path using `mem.read_and_write` (when non-overlapping) + row `copy_from_slice` (zero allocs, zero divs, SIMD `memcpy`); (b) 1:1 keyed fast path specialized for `u8`/`u16`/`u32` (typed compare allows LLVM SIMD mask-blend); (c) stretched path with reusable row scratch + precomputed `sx` LUT.
  - `draw_dib_draw`: direct RGB555→RGB565 bit-twiddle (`((v & 0x7fe0) << 1) | ((v & 0x0200) >> 4) | (v & 0x001f)`), reusable row buffer, division-free `scale_x == 2` unroll.
- **Hard determinism invariants:** Exact nearest-neighbour formula `sx0 + (x - dx0) * sw / dw` (`ddraw.rs:1607-1656`), self-overlap safety when `source.at == destination.at`, exact 555→565 top-bit replication (`avifile.rs:1109-1115`).
- **Suitability:** **Pure Rust-only refactor first** (likely 10×–30× win on `ddraw::transfer`); optional **`core::arch` SIMD** for 1:1 8/16-bit chroma-key blit (`cmpeq` + `vbsl`/`blendv`) only if profiling still shows ≥ 5%.

### 4.5 Frame Presentation, Surface Conversion & Host Scaling
- **Files & lines:** `core/src/machine.rs:1964-2043`; `core/src/host/headless.rs:107-135`; `core/src/host/x11.rs:129-147`; `core/src/host/mac.rs:1304-1352`; `core/src/host/windows.rs:863-913, 1079-1101`.
- **Loop structures & blockers:**
  - **`Machine::present_guest` 8-bit paletted (`machine.rs:2012-2036`):** Calls `self.arena.read_u32(palette + index * 4)` **307,200 times per 640×480 flip** (`2024-2026`) instead of copying the 1 KiB (256-entry) palette to a stack `[u32; 256]` once before the loop!
  - **`Machine::present_guest` 16-bit RGB565→BGRA32 (`machine.rs:1993-2010`):** Dynamic 2D indexing (`source[at]`, `self.present_buffer[out..out+3]`) adds per-pixel bounds checks and impedes clean vectorisation. Slicing per row with `chunks_exact(2)` + `chunks_exact_mut(4)` unlocks LLVM auto-vectorisation.
  - **`Headless::present` (`host/headless.rs:107-135`):** On **every** flip of every headless CI route (e.g., 2,247 flips in Alpha Team's menu), allocates a 1.2 MB `Vec<u8>`, copies all rows, scans every pixel for `uniform`, and frees the previous frame's 1.2 MB buffer (after `KEPT = 16`). Computing `uniform` directly on `frame.pixels` and reusing/skipping the allocation for discarded frames is a clean Rust-only CI throughput win.
  - **`x11::scale` (`host/x11.rs:129-147`):** Precomputes `columns: Vec<usize>` per frame, zeroes `out` (`clear` + `resize`), copies 4 bytes per pixel. Reusing `columns` and skipping zero-fill when all rows are valid removes per-frame overhead.
  - **`mac.rs:1321` / `windows.rs:1096`:** Single frame copy; scaling is handled by CoreGraphics / `StretchDIBits`.
- **Determinism invariants:** Host-only output (never feeds guest memory or `state_digest()`); screenshot PNGs depend on exact RGB565 top-bit replication (`(b << 3) | (b >> 2)`, `(g << 2) | (g >> 4)`, `(r << 3) | (r >> 2)`, `0xff`, `machine.rs:2005-2008`).
- **Suitability:** **Rust-only refactor first** (hoist 8-bit palette, row-slice 16-bit loop, `Headless` zero-alloc scan); trivial **`core::arch` SIMD candidate** for RGB565→BGRA32 (8 pixels/iter) if profiling still shows ≥ 5%.

### 4.6 Cross-Cutting: `Sha256::block`
- **Files & lines:** `core/src/hash.rs:123-160`, called by `Machine::state_digest()` (`core/src/machine.rs:1267-1337`).
- **Findings:** Pure-Rust scalar SHA-256 hashing megabytes of guest memory at route end (once per CI run, never during gameplay Pace). Hardware SHA2 extensions exist on Apple Silicon `aarch64` and newer `x86_64`, but ROI depends purely on whether #246 profiling shows `state_digest()` ≥ 5% of CI wall time.
- **Suitability:** **Skip** unless #246 CI profile flags it.

---

## Implications for #247 (Seam) & #248 (Triage)

1. **Two distinct assembly shapes exist in Core (feed into #247 seam design):**
   - **Data-parallel SIMD kernels (`core::arch` NEON + SSE2, optional AVX2):** `raster.rs` interior span loops, `synth.rs` steady-voice mixer, `iv50.rs` 8×8 slant / motion / BT.601, `ddraw.rs` 1:1 keyed blit, `machine.rs` RGB565→BGRA32. All are pure slice-in/slice-out functions with straightforward Rust fallbacks and exact bit-for-bit oracle tests.
   - **Control-flow / dispatch assembly (`global_asm!`):** Only **one** subsystem warrants hand-written `global_asm!`: the CPU interpreter's direct-threaded dispatch loop (`core/src/cpu/interp.rs` + `exec.rs`), because LLVM cannot emit per-opcode indirect-jump threading or pin guest GPRs across arbitrary opcode dispatch in stable Rust.
2. **Enormous Rust-only headroom must be cleared before judging assembly (#248):**
   - Before writing any assembly, profiling (#246) is likely to surface severe pure-Rust inefficiencies already visible in static audit: `ddraw::transfer` (per-row `to_vec()` + per-pixel 64-bit div), `synth.rs` (1.4M 64-bit `div_euclid`/sec + frame-outer loop), `machine.rs:2024` (307k `arena.read_u32`/flip on 8-bit surfaces), `d3d.rs` (per-triangle `Vec` allocs in `clip`/`rasterise`), `guest/mod.rs` (page-loop `wrote_over_code` per store), `exec.rs:string_op` (element-by-element `rep movs`/`stos`), and `raster.rs` (un-specialized uber-loop + `rem_euclid` `sdiv`).
   - Only subsystems that remain ≥ 5% of wall time **after** clean Rust-only fixes should graduate to throwaway `aarch64` + `x86_64` assembly prototypes.
