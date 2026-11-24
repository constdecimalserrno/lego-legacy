# Static host assembly behind per-subsystem seams

**Status: Accepted** (#247, #249, #257).

## Context

Core runs every title on the roster inside a deterministic, zero-crate Rust emulator (ADR-0001,
ADR-0004). Profiling across the roster's committed routes shows that three subsystems account for
97.1% of wall time — the CPU interpreter (74.9%), the software rasteriser (18.1%), and DirectDraw
blits (up to 19.7% on LEGO Island) — leaving tight pace headroom on heavy scenes (Hot Wheels
`first-race` at 1.34×, LEGO Chess `checkmate` at 1.34×, LEGO Alpha Team's 3D training mission at
1.26× on Apple Silicon).

Dynamic JIT and native overrides of guest code remain forbidden (Golden Rule 2, ADR-0004), and
external crates, external assemblers, or build-script code generators remain forbidden (Golden
Rule 5, ADR-0001). Static host SIMD and assembly written with Rust 1.98's `core::arch` and
`std::arch` can recover headroom without breaking either rule, provided every accelerated path
preserves Golden Rule 1 bit-for-bit across all six desktop targets (`aarch64` and `x86_64` ×
macOS, Linux GNU, Windows MSVC) and falls back cleanly to pure Rust everywhere else.

## Decision

### 1. Pure Rust first, benchmark-gated assembly second

Pure-Rust algorithmic, allocation, and specialization work always precedes assembly. A static
SIMD or assembly kernel may land only when:

- **Profiling bar:** the path accounts for at least 5% of a committed route's wall time (or
  threatens 1.0× live pace); for whole-loop CPU dispatch, at least 10% wall time or tight pace
  headroom.
- **Benchmark bar:** after pure-Rust cleanup, the accelerated kernel beats idiomatic `rustc 1.98`
  by at least 15% on that kernel (or 15% interpreter throughput for CPU dispatch).

Subsystems below the bar (audio synthesis and mixing, Indeo 5 and MSV1 video codecs, host
presentation scaling, SHA-256 digests, and cabinet decompression) stay pure Rust.

### 2. Co-located per-subsystem seams and one minimal `arch` module

Accelerated subsystems do not dump unrelated kernels into one global assembly module. Instead,
each accelerated subsystem keeps its canonical pure-Rust implementation beside its `aarch64` and
`x86_64` adapters and local invariants — the same locality `core/src/host/` uses for operating
system backends (ADR-0001).

`core/src/arch.rs` carries only shared host-side dispatch plumbing:

- `arch::Tier` (`Rust`, `Aarch64`, `X86_64`) and `Tier::native()` for the compile-time target.
- `Tier::active()` and `arch::use_rust()`, backed by a single `OnceLock` reading the startup
  environment variable `LEGO_LEGACY_ARCH` (`rust` forces `Tier::Rust`, `native` or unset uses
  `Tier::native()`, any other value panics at startup rather than guessing).

Nothing in `core/src/arch.rs` is ever guest-visible, read by a shim, or hashed into a machine
state digest (Golden Rule 1). `CONTEXT.md` stays free of implementation mechanics.

### 3. The four-rung assembly primitive ladder

Every optimization climbs this ladder in order and stops at the lowest rung that meets its bar:

1. **Idiomatic pure Rust.** Always written first; stays forever as the canonical fallback for any
   architecture outside `aarch64` and `x86_64` (`riscv64`, `wasm32`, future hosts) and as the
   bit-exact test oracle on every host.
2. **`core::arch` SIMD intrinsics on common desktop floors.** Baseline `neon` on `aarch64` and
   baseline `sse2` on `x86_64` are enabled on all three desktop operating systems, cover 100% of
   desktop hosts with zero runtime dispatch, and inline with zero overhead when called from
   baseline code. Note that `rustc 1.98` still requires caller `#[target_feature]` or `unsafe`
   even for default-enabled `neon` and `sse2` (`E0133`). Optional higher tiers (`sse4.1`, `avx2`)
   are deferred until benchmarks justify maintaining a second SIMD tier; if ever added,
   `#[target_feature]` must wrap the entire span or buffer loop (never a per-pixel or
   per-instruction helper, because non-baseline `#[target_feature]` is an inlining barrier) and
   `std::arch::is_x86_feature_detected!` must be hoisted outside the loop (preventing per-call
   checks and cold unwind landing pads in the hot body).
3. **Leaf inline `asm!`.** Used only when LLVM will not emit a required instruction sequence or
   control-flow pattern. Blocks stay leaf-only: no outgoing `call`/`bl`, no `clobber_abi` (which
   conservatively spills all vector registers), explicit `options(...)`, Intel syntax on `x86_64`,
   and the x86 direction flag (`DF`) clear on exit.
4. **`#[unsafe(naked)]` + `naked_asm!`.** Reserved for whole-loop register-pinned dispatch (such
   as flat32 CPU direct-threaded dispatch). `rustc 1.98` handles Darwin's leading `_` symbol
   prefix, ELF `.type`/`.size`, and MSVC `.def`/COMDAT sections automatically and forbids
   duplicating the body. Whole-loop naked kernels use a single-pointer, integer-only C ABI
   (`extern "C"`, or explicit `sysv64` / `win64` adapters) that touches zero FP/XMM/NEON
   registers (leaving Win64 callee-saved `xmm6..xmm15` untouched), makes zero outgoing calls, and
   returns a status code to Rust for slow ops, x87, faults, code writes, watches, or budget end.
   `global_asm!` is reserved strictly for read-only tables or assembler directives that cannot
   live in `naked_asm!`.

### 4. Cross-platform parity across six desktop targets

Every assembly or SIMD kernel ships `aarch64` and `x86_64` implementations together across all
six desktop targets:

- `aarch64-apple-darwin`, `x86_64-apple-darwin`
- `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-gnu`
- `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc`

Every other target compiles and runs on the canonical pure-Rust fallback automatically.

Cross-ABI rules that every `naked_asm!` or `asm!` block must respect:

- **SysV (`x86_64` Darwin/Linux) vs Win64 (`x86_64` Windows MSVC):** SysV passes integer arguments
  in `rdi, rsi, rdx, rcx, r8, r9`, preserves `rbx, rbp, r12..r15`, has no callee-saved XMM
  registers, and provides a 128-byte red zone. Win64 passes integer arguments in
  `rcx, rdx, r8, r9`, preserves `rbx, rbp, rdi, rsi, r12..r15` plus `xmm6..xmm15`, has no red
  zone, and requires 32 bytes of caller shadow space before any call.
- **`aarch64` platform register `x18`:** reserved by the operating system on Darwin and Windows
  MSVC; never named or modified in any `aarch64` assembly.

### 5. Golden Rule 1 determinism bans

Every accelerated path must produce bit-for-bit identical guest memory, registers, pixels, audio
samples, cursors, flips, declared milliseconds, and `Machine::state_digest()` values to its
pure-Rust oracle across all hosts. The following primitives are banned in Core:

- **FMA contraction:** `f32::mul_add`, `f64::mul_add`, x86 `_mm*_fmadd_*` / `_mm*_fmsub_*` /
  `_mm*_fnmadd_*` / `_mm*_fnmsub_*` intrinsics, `aarch64` `vfma*` / `vfms*` intrinsics, and
  assembly mnemonics `fmadd`, `fmsub`, `fnmadd`, `fnmsub`, `fmla`, `fmls`, `vfmadd*`, `vfmsub*`,
  `vfnmadd*`, `vfnmsub*`. Pure Rust `a * b + c` never contracts to FMA; single-rounded FMA drifts
  by 1 ULP from separate multiply and add.
- **`algebraic_*` float methods:** `algebraic_add`, `algebraic_sub`, `algebraic_mul`,
  `algebraic_div`, `algebraic_rem`, which permit non-deterministic reassociation and reciprocal
  substitution.
- **Approximate reciprocal and rsqrt instructions:** `_mm*_rcp_*`, `_mm*_rsqrt_*`, `rcpss`,
  `rcpps`, `vrcpss`, `vrcpps`, `rsqrtss`, `rsqrtps`, `vrsqrtss`, `vrsqrtps`, `vrecpe*`,
  `vrecps*`, `vrsqrte*`, `vrsqrts*`, `frecpe`, `frecps`, `frsqrte`, `frsqrts`. Their lookup
  tables diverge across Intel, AMD, and ARM even after Newton-Raphson refinement.
- **Host FP control-register mutation:** `ldmxcsr`, `vldmxcsr`, `_mm_setcsr`, `fesetround`, and
  `msr` writes to `fpcr` or `fpsr`. Core relies on IEEE-754 round-to-nearest-even with no FTZ/DAZ.
- **Explicit `x18` / `w18` operands on `aarch64`.**

Additional invariants enforced by review and oracle tests:

- **Float-to-int truncation:** `aarch64` `fcvtzs` saturates like Rust `as i32`, whereas raw
  `x86_64` `cvtt*` returns `i32::MIN` on overflow or `NaN`. Prefer integer/fixed-point SIMD;
  otherwise clamp finite inputs into in-range bounds before conversion or apply an explicit
  saturation and `NaN` mask on `x86_64`.
- **`min`/`max` on `NaN` and signed zero:** x86 `minps`/`maxps` return the second operand on
  `NaN` or `-0.0` vs `+0.0`, while NEON `vminq_f32` propagates `NaN` and orders `-0.0 < +0.0`.
  Avoid raw FP `min`/`max` when `NaN` or signed zero can reach guest state, and canonicalize
  `NaN` payloads at guest-visible boundaries.
- **Subsystem invariants:** preserve exact sequential floating-point stepping in rasteriser spans,
  exact IEEE-754 division, pinned pure-Rust transcendental tables in `x87` and `synth`
  (`core/tests/x87.rs`), and the declared `raster::BANDS` frame decomposition (ADR-0004).

### 6. Mechanical enforcement and two-layer verification

1. **Source-scanning determinism test (`core/tests/arch_determinism.rs`):** zero-crate scan over
   `core/src/` (mirroring `core/tests/x87.rs`) rejecting every banned FMA, `algebraic_*`,
   approximate `rcp`/`rsqrt`, host FP control-register mutation, and `x18`/`w18` pattern.
2. **Layer 1 — co-located differential oracle tests:** every accelerated subsystem runs each
   compiled ISA adapter side-by-side against its co-located pure-Rust oracle across edge values,
   unaligned offsets, and tail lengths `0..=65`, asserting exact bit/byte equality.
3. **Layer 2 — full-route replay parity:** any committed route can be replayed with
   `LEGO_LEGACY_ARCH=rust` and compared against default execution, proving identical `cursor`,
   `flips`, `declared_ms`, and `Machine::state_digest()`.
4. **Six-target release-library build gate (`scripts/check-targets.sh`):** runs
   `cargo build -p lego-legacy-core --lib --release --target <T>` across all six desktop targets
   with zero cross-linkers. Because `cargo check` emits metadata only and silently exits 0 on
   invalid mnemonics inside `asm!`, release library compilation is the mandatory cross-target
   assembly check. Runtime `x86_64` oracle execution runs locally on Apple Silicon via Rosetta 2
   (up through AVX2), paired with Linux and Windows `x86_64` CI if `naked_asm!` lands.

## Considered options

- **One centralized `core/src/arch/` kernel library:** separates kernels from each subsystem's
  invariants and tests, encouraging drift between the pure-Rust oracle and its ISA adapters;
  rejected in favour of co-located seams plus a minimal `core/src/arch.rs` toggle.
- **External crates or assemblers (`cc`, `nasm`, `wide`, `bytemuck`):** breaks the zero-crate
  twenty-year build guarantee; rejected by ADR-0001 and Golden Rule 5.
- **Dynamic JIT compilation:** rejected by ADR-0004.
- **`cargo check --target <T>` as the cross-target gate:** empirically verified to exit 0 on
  invalid assembly mnemonics because `--emit=metadata` never invokes LLVM codegen or the
  integrated assembler; rejected in favour of `cargo build --lib --release --target <T>`.
- **Single-architecture (`aarch64`-only) assembly:** breaks desktop parity and leaves `x86_64`
  unverified; rejected.
