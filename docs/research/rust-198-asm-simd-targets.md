# Rust 1.98 static assembly and SIMD across desktop targets

- **Ticket:** [#244](https://github.com/constdecimalserrno/lego-legacy/issues/244) (child of map [#243](https://github.com/constdecimalserrno/lego-legacy/issues/243))
- **Toolchain measured:** `rustc 1.98.0 (88d9e12ae 2026-08-18)`, LLVM `22.1.8`, host `aarch64-apple-darwin` (`macOS 26.6.2`, `Darwin 25.6.0`)
- **Constraints:** Zero external crates ([ADR-0001](../adr/0001-zero-crates-and-one-host-seam.md)), bit-exact determinism across every host ([Golden Rule 1](../../GOLDEN-RULES.md)), no JIT ([ADR-0004](../adr/0004-interpreter-first-and-the-machine-stays-honest.md)).

All six desktop targets (`aarch64` and `x86_64` × `apple-darwin`, `unknown-linux-gnu`, `pc-windows-msvc`) are supported with zero external crates via `core::arch` (intrinsics, `asm!`, `naked_asm!`, `global_asm!`) and `std::arch` (runtime CPU feature detection). `lego-legacy-core` currently has zero `core::arch`/`asm!`/`#[target_feature]` uses (`grep` on `core/src/`).

---

## 1. `core::arch` SIMD intrinsics vs `asm!` vs `naked_asm!` vs `global_asm!`

All four live in `core::arch` (zero external crates). `#[unsafe(naked)]` + `naked_asm!` stabilized in Rust 1.88 ([PR #134213](https://github.com/rust-lang/rust/pull/134213)), `asm!` `label` (`asm_goto`) in Rust 1.87 ([PR #133870](https://github.com/rust-lang/rust/pull/133870)), `#[cfg]` on asm templates/operands (`asm_cfg`) in Rust 1.93 ([PR #147736](https://github.com/rust-lang/rust/pull/147736)), and default v0 symbol mangling in Rust 1.97 ([PR #151994](https://github.com/rust-lang/rust/pull/151994)).

| Property | `core::arch` SIMD intrinsics | `asm!` (function-scope) | `#[unsafe(naked)]` + `naked_asm!` | `global_asm!` (module-scope) |
| --- | --- | --- | --- | --- |
| **Calling convention** | Compiler-managed (`Rust` ABI) | Compiler-managed; `clobber_abi(...)` if calling out | Manual (`extern "C"` / `"sysv64"` / `"win64"`; `"Rust"` ABI forbidden) | Manual (all args, shadow space, callee-saved regs) |
| **Symbol mangling & `_` prefix** | Automatic | Automatic via `sym <path>` operand | Automatic (`_name` on Darwin, ELF `.type`/`.size`, COFF `.def`/COMDAT) | Manual: leading `_` on Darwin, unprefixed on Linux/Windows; manual `.type`/`.size`/`.def` |
| **Unwinding & debug info** | Full compiler DWARF CFI / Win64 SEH | Inherits caller CFI/SEH; **UB to unwind out of `asm!`** | Unwinding allowed (`extern "*-unwind"`), but requires manual `.cfi_*` (Darwin/Linux) or `.seh_*` (MSVC) | Manual `.cfi_*` (Darwin/Linux) or `.seh_*` (MSVC); no Rust debug info |
| **`#[target_feature]` & inlining** | Inlines freely within matching features | Inlines freely; LLVM allocates `reg`/`xmm_reg`/`vreg` | Cannot use `#[target_feature]` or `#[inline]` | Not a Rust function; never inlined |
| **Operand types** | Typed SIMD vectors & pointers | `in`/`out`/`lateout`/`inout`, `const`, `sym`, `label` | `sym` and `const` only | `sym` and `const` only |

*Primary sources: Rust 1.98 Reference (`[asm]`, `[asm.scope]`, `[asm.abi-clobbers]`, `[asm.rules]`, `[asm.naked-rules]`, `[attributes.codegen.naked]`); verified by compiling all four forms with `rustc 1.98.0 -C opt-level=3 --emit=asm,obj` across all 6 targets.*

### Cross-ABI constraints to respect across Darwin, SysV, and Win64

1. **`x86_64` calling conventions diverge between Windows and Unix:**
   - **Darwin / Linux (`sysv64`):** integer args in `rdi, rsi, rdx, rcx, r8, r9`; float/vector args in `xmm0..xmm7`; callee-saved `rbx, rbp, r12..r15`; **no callee-saved XMM registers**; 128-byte red zone below `rsp`.
   - **Windows MSVC (`win64`):** integer args in `rcx, rdx, r8, r9`; float/vector args in `xmm0..xmm3`; callee-saved `rbx, rbp, rdi, rsi, r12..r15` **plus `xmm6..xmm15`**; caller must reserve **32 bytes of shadow space** above the return address before any `call`; no red zone.
   - `clobber_abi("C")` handles the target's C ABI automatically, **but** conservatively clobbers all `xmm0..xmm31` on `x86_64` and `v0..v31` on `aarch64` (`[asm.abi-clobbers.supported-abis]`). Empirically on `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, and `aarch64-pc-windows-msvc`, a single `asm!("bl {f}", clobber_abi("C"))` forces an 80-byte spill/reload of `d8..d15` plus `x29`/`x30`. Avoid calling functions from inside hot `asm!` blocks.
2. **`aarch64` platform register `x18`:**
   - Reserved by the OS on `aarch64-apple-darwin` and `aarch64-pc-windows-msvc` (not reserved on Linux). `asm!` forbids `x18` as an explicit operand on all `aarch64` targets (`[asm.register-names.not-for-io]`), and `naked_asm!`/`global_asm!` must never touch `x18`.
3. **Reserved registers in `asm!`:**
   - `sp`, frame pointer (`rbp` on `x86_64`, `x29` on `aarch64`), and LLVM's base pointer (`rbx` on `x86_64`, `x19` on `aarch64`) cannot be named as explicit operands (`[asm.register-names.not-for-io]`).
   - On `x86_64`, `asm!` uses Intel syntax (`.intel_syntax noprefix`) by default (`[asm.ts-args.llvm-syntax]`), `DF` must remain clear (`[asm.rules.x86-df]`), and `options(pure, nomem, nostack, preserves_flags)` should be set whenever valid so LLVM keeps flags/memory live across the block.
4. **Recommendation for Core:**
   - **Default to `core::arch` SIMD intrinsics** — zero ABI/mangling/unwind boilerplate, full register allocation, inlining, and type safety.
   - **Use `asm!`** only when LLVM fails to emit a specific instruction sequence or control-flow pattern (e.g., `label` dispatch, carry/flag sequences), keeping the block leaf-only (no `call`/`bl`, no `clobber_abi`).
   - **Prefer `#[unsafe(naked)]` + `naked_asm!` over `global_asm!`** if a whole-function assembly kernel is ever needed: `rustc` automatically emits Mach-O `_` prefixes, ELF `.type`/`.size`, and MSVC `.def`/COMDAT sections, and forbids duplicating the body (`[attributes.codegen.naked.no-duplication]`). Reserve `global_asm!` only for custom read-only tables or directives that cannot live in a naked function.

---

## 2. ISA feature floor, safe `#[target_feature]`, and runtime dispatch

### Baseline `target_feature` floor per desktop target

Empirically queried via `rustc 1.98.0 --target <T> --print cfg`:

| Target | Rust tier (1.98) | Default enabled `target_feature` set |
| --- | --- | --- |
| `aarch64-apple-darwin` | Tier 1 | `neon` + 27 Armv8.5-A features (`aes`, `crc`, `dit`, `dotprod`, `dpb`, `dpb2`, `fcma`, `fhm`, `flagm`, `fp16`, `frintts`, `jsconv`, `lor`, `lse`, `paca`, `pacg`, `pan`, `pmuv3`, `ras`, `rcpc`, `rcpc2`, `rdm`, `sb`, `sha2`, `sha3`, `ssbs`, `vh`) |
| `aarch64-unknown-linux-gnu` | Tier 1 | `neon` |
| `aarch64-pc-windows-msvc` | Tier 1 (since 1.91, [PR #145682](https://github.com/rust-lang/rust/pull/145682)) | `neon` |
| `x86_64-apple-darwin` | Tier 1 | `cmpxchg16b`, `fxsr`, `sse`, `sse2`, `sse3`, `ssse3`, `sse4.1` |
| `x86_64-pc-windows-msvc` | Tier 1 | `cmpxchg16b`, `fxsr`, `sse`, `sse2`, `sse3` |
| `x86_64-unknown-linux-gnu` | Tier 1 | `fxsr`, `sse`, `sse2` |

- **Common `aarch64` floor (all 3 OSes):** `neon` (`FEAT_AdvSimd` + `FEAT_FP`).
- **Common `x86_64` floor (all 3 OSes):** `sse2` (`sse`, `sse2`, `fxsr`).
- **Feature implication chains (`[attributes.codegen.target_feature.x86]`):** `avx2` → `avx` → `sse4.2` → `sse4.1` → `ssse3` → `sse3` → `sse2` → `sse`. Note that **`avx2` does NOT imply `fma`**, and `fma` implies `avx` (not `avx2`).

### Safe `#[target_feature]` and `core::arch` safety rules in Rust 1.98

- **Safe `#[target_feature]` functions** stabilized in Rust 1.86 ([PR #134090](https://github.com/rust-lang/rust/pull/134090)).
- **Safe value-only SIMD intrinsics** stabilized in Rust 1.87 ([stdarch #1714](https://github.com/rust-lang/stdarch/pull/1714), [#1716](https://github.com/rust-lang/stdarch/pull/1716), [#1717](https://github.com/rust-lang/stdarch/pull/1717)): arithmetic/shuffle/bitwise intrinsics without pointer arguments (e.g., `vaddq_s32`, `_mm_add_epi32`, `_mm256_add_epi32`) are `pub fn` (not `unsafe fn`). Pointer load/store intrinsics (`vld1q_s32`, `_mm_loadu_si128`) remain `pub unsafe fn`.
- **Mandatory caller annotation gotcha (empirically verified):** Even though `neon` is enabled by default on all 3 `aarch64` targets and `sse2` on all 3 `x86_64` targets, calling a `#[target_feature]` function (including safe `vaddq_s32` or `_mm_add_epi32`) from an unannotated function **still fails with `E0133`** unless wrapped in `unsafe` or called from a function with matching `#[target_feature]`:
  > `note: the neon target feature being enabled in the build configuration does not remove the requirement to list it in #[target_feature]`
  > (*Rust 1.98 Reference `[attributes.codegen.target_feature.safety-restrictions]` and `rustc 1.98.0` diagnostic.*)
- **Additional `#[target_feature]` constraints (`[attributes.codegen.target_feature]`):**
  - Disallowed on `main`, `#[panic_handler]`, safe trait methods, safe trait default methods, and `#[unsafe(naked)]` functions.
  - `#[inline(always)]` is forbidden on `#[target_feature]` functions (`[attributes.codegen.target_feature.inline]`).
  - Named `#[target_feature]` functions do not implement `Fn`/`FnMut`/`FnOnce`, though closures defined inside them inherit the features and do implement `Fn*` (`[attributes.codegen.target_feature.fn-traits]`).

### Inlining barriers and dispatch overhead

Empirically verified via `rustc 1.98.0 -C opt-level=3 --emit=asm` across all 6 targets:

1. **Baseline-feature helpers inline with zero overhead:**
   - Calling `#[target_feature(enable = "neon")] #[inline]` from an unannotated function on any `aarch64` target inlined completely into a single `add.4s v0, v1, v0`.
   - Calling `#[target_feature(enable = "sse2")] #[inline]` from an unannotated function on any `x86_64` target inlined completely into a single `paddd %xmm1, %xmm0`.
   - Reason: LLVM allows inlining whenever the caller's target feature set is a superset of the callee's (`[attributes.codegen.target_feature.inline]`).
2. **Runtime-dispatched tiers (`sse4.1`, `avx2`) are hard inlining barriers:**
   - Calling `#[target_feature(enable = "avx2")] #[inline]` from a baseline `x86_64` function emitted a real `callq` AND spilled `__m128i` arguments through stack pointers (`movaps` + `leaq`), because passing SIMD vectors across a non-inlined `#[target_feature]` boundary cannot assume vector-register ABI compatibility.
   - **Rule:** Never place `#[target_feature(enable = "sse4.1" | "avx2")]` on a per-pixel or per-instruction helper; wrap the **entire scanline/span/block/buffer loop** (taking plain slices/pointers/scalars) inside the `#[target_feature]` function, and let inner `#[target_feature]` helpers inline into it.
3. **`is_x86_feature_detected!` / `is_aarch64_feature_detected!` mechanics and cost:**
   - Source-verified in `std_detect/detect/cache.rs`: backed by `static CACHE: [Cache; 3]` (`AtomicUsize`, `Ordering::Relaxed`).
   - When a feature is enabled at compile time (e.g., `is_x86_feature_detected!("sse4.1")` on `x86_64-apple-darwin`), the macro expands to literal `true` (zero runtime cost).
   - Otherwise, the hot path compiles to 1 relaxed load + 2 test/branch instructions (`movq CACHE(%rip), %rax; testq %rax, %rax; je init; testw %ax, %ax; js fast`). However, empirically in our `rustc 1.98.0 --emit=asm` probe, the cold `detect_and_initialize` call pulls an EH personality/unwind handler (`DW.ref.rust_eh_personality` on ELF, `__CxxFrameHandler3` on MSVC) into the enclosing function.
   - **Dispatch pattern:** Hoist `is_x86_feature_detected!` outside hot loops (or resolve a `fn(...)` pointer / tier enum once at startup/surface creation). A cached function pointer or hoisted branch keeps the hot loop free of both detection branches and EH landing pads.
   - **AArch64 runtime detection caveat:** `std::arch::is_aarch64_feature_detected!` docs explicitly note that most optional feature checks work on Linux (and Darwin via `sysctlbyname` in `std_detect/detect/os/darwin/aarch64.rs`), whereas other targets (including Windows ARM64 for many features) return `false`. Sticking to mandatory `neon` on `aarch64` avoids OS-divergent detection altogether.

---

## 3. Bit-exact determinism hazards (Golden Rule 1)

| Hazard | Pure Rust 1.98 behavior | `aarch64` vs `x86_64` SIMD/asm divergence | Prevention by construction |
| --- | --- | --- | --- |
| **FMA contraction** | `a * b + c` is **never** contracted to FMA (empirically verified: emits `fmul`+`fadd` on `aarch64` and `vmulss`+`vaddss` on `x86_64 +avx2,+fma`). `f32::mul_add` is IEEE 754 `fusedMultiplyAdd` (single rounding), emitting `fmadd` on `aarch64`, `vfmadd213ss` on `x86_64 +fma`, and a slow `libm` `fmaf` call on baseline `x86_64` SSE2. | Using NEON `vfmaq_f32`/`vfmaq_f64` or x86 `_mm256_fmadd_ps` against an `a * b + c` oracle causes 1-ULP drift. | **Forbid FMA instructions** unless the pure-Rust oracle also uses `mul_add`; **forbid** `f32::algebraic_*` / `f64::algebraic_*` (docs allow non-deterministic reassociation and reciprocal substitution). |
| **Float-to-int truncation (`as i32`)** | Saturating truncation toward zero: `NaN -> 0`, `>= i32::MAX -> i32::MAX`, `<= i32::MIN -> i32::MIN` (`[expr.as.numeric.float-as-int]`). | On `aarch64`, `fcvtzs` / `vcvtq_s32_f32` matches Rust in **1 instruction**. On `x86_64`, raw `cvttss2si` / `_mm_cvttps_epi32` returns `0x8000_0000` (`i32::MIN`) for `NaN`, `+Inf`, and any input `>= 2^31` (LLVM emits 7 instructions with `ucomiss`+`cmov` for `x as i32`). | Prefer integer/fixed-point SIMD; when converting floats, either clamp inputs into finite in-range bounds `[-2^31, 2^31 - 128]` before conversion or apply an explicit `x86_64` saturation/NaN mask matching `fcvtzs`. |
| **NaN payloads, sign, and `min`/`max`** | `{float}::NAN` is guaranteed quiet since Rust 1.88 ([PR #139483](https://github.com/rust-lang/rust/pull/139483)); both `aarch64` and `x86_64` have **no** target-specific extra NaN payloads (`core::primitive::f32` docs). However, arithmetic NaN sign and operand choice remain non-deterministic; `f32::min`/`max` also leave `-0.0` vs `+0.0` unspecified. | x86 `_mm_min_ps(a, b)` returns `b` when either operand is NaN or when comparing `-0.0` and `+0.0` (`a < b ? a : b`); NEON `vminq_f32` propagates NaN and orders `-0.0 < +0.0`, while `vminnmq_f32` ignores NaN. | Keep Core SIMD kernels in integer/fixed-point whenever possible. For any FP kernel whose bits reach guest state, avoid raw `minps`/`maxps` on NaN/signed-zero inputs and canonicalize NaN outputs at guest-visible boundaries. |
| **Approximate `rcp` / `rsqrt`** | Pure Rust has exact IEEE 754 `1.0 / x` and `sqrt`. | `_mm_rcp_ps`/`_mm_rsqrt_ps` (relative error `<= 1.5 * 2^-12`, **differs between Intel and AMD**) and NEON `vrecpeq_f32`/`vrsqrteq_f32` (8-bit table) produce different bit patterns even after Newton-Raphson refinement. | **Ban approximate reciprocal/rsqrt instructions** in deterministic Core paths. |
| **Vector integer shifts** | Rust shifts panic in debug / wrap-mask count in release (`count & (W-1)`). | x86 `_mm_sll_epi32`/`_mm_srl_epi32`/`_mm_sra_epi32` take an unsigned 64-bit count and saturate to `0`/sign-bit for `count >= W`; NEON `vshlq_s32`/`vshlq_u32` take a **signed 8-bit per-lane count** where negative shifts right and `|count| >= W` saturates to `0`/sign-bit. | Prefer compile-time immediate shifts in `0..W`, or mask/clamp variable shift counts identically before shifting on both ISAs. |
| **Saturating arithmetic & narrowing** | Explicit `saturating_*` / `.clamp(...)` in pure Rust. | `_mm_adds_epi16`/`_mm_packs_epi32`/`_mm_packus_epi16` match NEON `vqaddq_s16`/`vqmovn_s32`/`vqmovun_s16` bit-for-bit, as does `pavgb` ↔ `vrhaddq_u8` (`(a + b + 1) >> 1`). Watch out: `_mm_packus_epi32` requires **SSE4.1** (not baseline SSE2), and NEON `vhaddq_u8` omits the `+ 1` rounding bit. | Document the exact mathematical identity in each kernel's contract and test every boundary (`0`, `±1`, `MIN`, `MAX`, `MIN+1`, `MAX-1`) against the Rust oracle. |
| **Host FP control state (`MXCSR` / `FPCR`)** | Rust assumes IEEE 754 round-to-nearest-even, no FTZ/DAZ. | Changing `MXCSR` or `FPCR` (or calling host code that mutates them) changes every FP instruction silently. | Never mutate `MXCSR`/`FPCR` in Core kernels; treat IEEE 754 default rounding as an invariant. |

### Enforcing determinism by construction

1. **Co-located pure-Rust oracle:** Every accelerated kernel lives beside its canonical pure-Rust implementation, which serves both as the fallback for non-`aarch64`/non-`x86_64` hosts and as the test oracle on every host.
2. **Bit-exact differential tests:** Unit tests run every compiled tier (`aarch64` NEON; `x86_64` SSE2 plus any runtime-dispatched SSE4.1/AVX2 tier) against the Rust oracle and assert byte/bit equality (`to_bits()`, slice equality) across edge values, unaligned offsets, and tail lengths `0..=65`.
3. **Route replay gate:** Existing `scripts/ci.sh` route replays verify zero cursor, pixel, audio, or snapshot hash drift across hosts.

---

## 4. Empirical cross-target verification on this Mac mini

Tested on this Mac mini (`aarch64-apple-darwin`, `macOS 26.6.2`, `rustc 1.98.0`, all 6 `rust-std` targets installed, `CARGO_TARGET_DIR="$PWD/target"`):

| Command | Result across targets | Key finding |
| --- | --- | --- |
| `cargo check -p lego-legacy-core --target <T>` | **PASS on all 6 targets** (~1.5–2.0 s each) | Zero crates, no cross-linkers needed — **but insufficient for assembly verification**. |
| `cargo clippy -p lego-legacy-core --lib --target <T> -- -D warnings` | **PASS on all 6 targets** | Clean cross-target lint coverage without cross-linkers. |
| `cargo build -p lego-legacy-core --lib --release --target <T>` | **PASS on all 6 targets** (~18.7–21.7 s each) | Runs full LLVM codegen + integrated assembler for Mach-O, ELF, and COFF/PE with **zero cross-linkers**. |
| `cargo test -p lego-legacy-core --lib --target x86_64-apple-darwin --no-run` | **PASS (`exit 0`, 4.1 s)** | Compiles and links a Mach-O `x86_64` test executable using Xcode's universal `/usr/bin/clang` and `MacOSX.sdk`. |
| `cargo test -p lego-legacy-core --lib --target x86_64-apple-darwin` (run) | **FAIL (`Bad CPU type in executable`, os error 86)** | **Rosetta 2 is not installed on this Mac mini** (`com.apple.pkg.RosettaUpdateAuto` missing; `/Library/Apple/usr/share/rosetta` absent). |

### Two verification takeaways for #247 and implementation tickets

1. **Always gate cross-target assembly with `cargo build --lib --release --target <T>`, not `cargo check` alone:**
   Empirically verified with a probe containing `asm!("totally_not_a_real_instruction")`: `--emit=metadata` (`cargo check`) exited **`0`** (silent pass), whereas `--emit=obj` (`cargo build --lib`) failed with `error: invalid instruction mnemonic` (`exit 1`). Because `lego-legacy-core` is a pure-Rust `rlib` with zero dependencies, `cargo build -p lego-legacy-core --lib --release --target <T>` validates LLVM codegen, register allocation, symbol mangling, and integrated assembly across all 6 desktop targets locally on macOS without installing any Linux or Windows cross-linker.
2. **Local `x86_64` runtime oracle testing currently requires installing Rosetta 2 (or running CI/native `x86_64`):**
   - On this Mac mini (`macOS 26.6.2`), running `softwareupdate --install-rosetta` would immediately allow `cargo test -p lego-legacy-core --lib --target x86_64-apple-darwin` to execute locally. Per Apple's *About the Rosetta Translation Environment* documentation, Rosetta 2 on macOS 15+ translates standard `x86_64` instructions including **AVX and AVX2** (but **not AVX-512**), and macOS 27 is the final release slated to include general-purpose Rosetta 2.
   - Because `.github/workflows/` does not yet exist in this repository and Rosetta 2 does not test Win64/Linux SysV ABIs or native `x86_64` timings, hand-off verification for `x86_64` kernels should pair local cross-`cargo build --lib` (all 6 targets) with either Rosetta 2 (if installed) or an `x86_64` CI/hardware pass for runtime bit-exactness and timing.

---

## Primary sources cited

- Local `rustc 1.98.0 (88d9e12ae 2026-08-18)` / LLVM `22.1.8` binaries and bundled docs (`$(rustc --print sysroot)/share/doc/rust/html/`):
  - *The Rust Reference*: `reference/inline-assembly.html` (`[asm]`), `reference/attributes/codegen.html` (`[attributes.codegen.naked]`, `[attributes.codegen.target_feature]`), `reference/abi.html`, `reference/expressions/operator-expr.html` (`[expr.as.numeric.float-as-int]`).
  - *Rust Standard Library*: `core/primitive.f32.html` (`NaN bit patterns`, `Algebraic operators`), `std/primitive.f32.html` (`mul_add`, `min`, `minimum`), `std/arch/macro.is_x86_feature_detected.html`, `std/arch/macro.is_aarch64_feature_detected.html`, `src/std_detect/detect/cache.rs.html`, `src/std_detect/detect/os/darwin/aarch64.rs.html`.
  - *Rust Release Notes*: `releases.md` (1.86 PR #134090, 1.87 PR #133870 & stdarch #1714/#1716/#1717, 1.88 PR #134213 & #139483, 1.91 PR #145682, 1.93 PR #147736, 1.97 PR #151994).
- Apple Developer Documentation: *About the Rosetta Translation Environment* (macOS 15+ AVX/AVX2 support, no AVX-512; macOS 27 transition milestone).
- Empirical checks run on `lego-legacy-core` v1.2.0 (`rustc --print cfg`, `cargo check`, `cargo clippy`, `cargo build --lib --release`, `cargo test --target x86_64-apple-darwin`, and isolated `rustc -C opt-level=3 --emit=asm,obj` probes across all 6 desktop targets).
