//! Direct-threaded `aarch64` `#[unsafe(naked)]` + `naked_asm!` flat32 CPU
//! interpreter loop (ADR-0009, #256).
//!
//! Single-pointer, integer-only C ABI:
//!   `extern "C" fn run_fast_block(ctx: *mut FastCtx) -> u32`
//!
//! ABI and determinism invariants:
//! - Uses strictly caller-saved volatile scalar IEEE-754 FP registers (`d0..d3` /
//!   `s0..s3`, ADR-0009 Rung 4 clarification #280); leaves AAPCS64 callee-saved
//!   `d8..d15` 100% untouched with zero FP spills, zero FMA, zero `rcp`/`rsqrt`,
//!   and zero `FPCR`/`FPSR` writes.
//! - Never names or modifies `x18` / `w18` (Darwin and Windows platform reg).
//! - Makes zero outgoing calls; returns a status code to Rust for cache misses,
//!   budget exhaustion, `outside` overlaps, `Slow`/cold x87 ops, `NaN`, `PC == 0`,
//!   x87 stack faults, memory faults, page-crossing writes, and `code_bitmap` hits.
//! - Checks all bounds, `code_bitmap`, `NaN`, precision/rounding, and x87 stack
//!   invariants BEFORE mutating guest state so any fallback re-executes the exact
//!   instruction cleanly in Rust.

use super::{
    FastCtx, STATUS_BUDGET_SHORT, STATUS_DONE, STATUS_FALLBACK, STATUS_MISS, STATUS_OUTSIDE_OVERLAP,
};
use core::arch::naked_asm;

/// Pack `Flags` arithmetic bits into the `aarch64` `FastCtx::flags` word:
/// - bit 31 = N (`SF`)
/// - bit 30 = Z (`ZF`)
/// - bit 29 = C (`CF`)
/// - bit 28 = V (`OF`)
/// - bit  4 = `AF`
/// - bit  2 = `PF`
/// - bit  0 = `1` (`PF` and `AF` are materialized and valid)
#[inline(always)]
pub(crate) fn pack_flags(f: &crate::cpu::flags::Flags) -> u32 {
    (u32::from(f.sf) << 31)
        | (u32::from(f.zf) << 30)
        | (u32::from(f.cf) << 29)
        | (u32::from(f.of) << 28)
        | (u32::from(f.af) << 4)
        | (u32::from(f.pf) << 2)
        | 1
}

/// Unpack the `aarch64` `FastCtx::flags` word back into `Flags`, preserving
/// `df` and all unmodeled `EFLAGS` bits untouched.
#[inline(always)]
pub(crate) fn unpack_flags(raw: u32, f: &mut crate::cpu::flags::Flags) {
    f.sf = (raw & (1 << 31)) != 0;
    f.zf = (raw & (1 << 30)) != 0;
    f.cf = (raw & (1 << 29)) != 0;
    f.of = (raw & (1 << 28)) != 0;
    f.af = (raw & (1 << 4)) != 0;
    f.pf = (raw & (1 << 2)) != 0;
}

/// Execute flat32 cached runs in a direct-threaded `aarch64` register-pinned
/// loop until budget exhaustion, `outside` entry, cache miss, or Rust fallback.
///
/// # Safety
/// `ctx` must point to a validly initialized [`FastCtx`] whose arena, code
/// bitmap, and decode-cache pointers remain valid for the duration of the call
/// and satisfy `arena_len >= arena_floor + 4` and `!mem.is_watching()`.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn run_fast_block(ctx: *mut FastCtx) -> u32 {
    naked_asm!(
        // Save callee-saved registers x19..x28, x29, x30 (96 bytes, 16-byte aligned).
        // Never touch x18/w18 or callee-saved FP d8..d15 (only volatile d0..d3).
        "stp x29, x30, [sp, #-96]!",
        "stp x19, x20, [sp, #16]",
        "stp x21, x22, [sp, #32]",
        "stp x23, x24, [sp, #48]",
        "stp x25, x26, [sp, #64]",
        "stp x27, x28, [sp, #80]",

        // Load pinned invariants from FastCtx (x0 -> x29; x19 = ctx.regs; x30 = ctx.fpu).
        "mov x29, x0",
        "ldr x19, [x29, #0]",       // x19 = regs (*mut u32)
        "ldr x30, [x29, #168]",     // x30 = fpu (*mut X87, 72B repr(C); leaf fn so LR is free)
        "ldr x20, [x29, #8]",       // x20 = arena_base
        "ldr x21, [x29, #16]",      // x21 = code_bitmap
        "ldr x23, [x29, #24]",      // x23 = arena_limit4 (= span - 4)
        "ldr x26, [x29, #32]",      // x26 = remaining
        "mov x27, #0",              // x27 = retired in this call
        "ldr x10, [x29, #48]",      // x10 = fast_insns (*mut FastInsn)
        "ldr x11, [x29, #56]",      // runs (*const Run)
        "sub x11, x11, #8",         // x11 = runs - 8 (indexed by 1-based run_idx!)
        "ldr x9,  [x29, #144]",     // x9  = l1_cache (*mut L1Slot, 4096 x 16B)
        "ldr x7,  [x29, #152]",     // x7  = outside_danger_len (u64)
        "ldr w8,  [x29, #160]",     // w8  = outside_danger_start (u32)
        "ldr w14, [x29, #104]",     // w14 = eip
        "ldr w28, [x29, #108]",     // w28 = flags (NZCV + PF + AF + valid bit 0)
        "ldr w6,  [x29, #112]",     // w6  = fs_base
        "ldr w22, [x29, #116]",     // w22 = arena_floor
        "mov w12, #0",              // w12 = lazy PF result
        "mov w13, #0",              // w13 = lazy AF xor
        "adr x24, 900f",            // x24 = dispatch offset table base

        // =====================================================================
        // next_block (label 10): indirect transfer / entry via L1 block cache.
        // =====================================================================
        "10:",
        "cbz x26, 950f",            // remaining == 0 -> STATUS_DONE
        // Zero-load fast `outside` check (covers Boundary::RANGE, empty, & custom ranges)
        "sub w1, w14, w8",          // w1 = eip.wrapping_sub(outside_danger_start) (zero-ext x1)
        "cmp x1, x7",               // x1 < outside_danger_len?
        "b.lo 18f",                 // cold exact outside check

        // Direct-mapped L1 block cache probe (4096 x 16B = 64 KiB, pinned in x9)
        "eor w0, w14, w14, lsr #12",
        "and w0, w0, #0xfff",
        "add x3, x9, x0, lsl #4",   // x3 = &l1[slot]
        "ldp x0, x2, [x3]",         // x0 = (run_idx+1 << 32) | eip; x2 = Run
        "cmp w0, w14",              // slot.eip == eip?
        "b.ne 11f",                 // L1 miss (no chain slot)
        "ubfx x16, x2, #48, #8",    // x16 = run.len
        "cbz x16, 11f",             // empty slot (guards eip == 0)
        "b 20f",                    // L1 hit -> dispatch!

        // =====================================================================
        // chain_dispatch (label 12): static transfer with w14 = eip, w0 = run_idx+1, x5 = &slot.
        // =====================================================================
        "12:",
        "cbz x26, 950f",            // remaining == 0 -> STATUS_DONE
        "sub w1, w14, w8",
        "cmp x1, x7",
        "b.lo 18f",                 // cold exact outside check
        "cbz w0, 13f",              // unchained -> L1 / 3-level lookup & patch [x5]
        "ldr x2, [x11, x0, lsl #3]",// Run (1-based index into x11 = runs - 8)
        "ubfx x16, x2, #48, #8",    // x16 = run.len
        "cbz x16, 13f",             // invalidated by forget_page() -> re-resolve & re-patch

        // =====================================================================
        // dispatch (label 20): enter Run in x2 with x16 = run.len (chain-hit fallthrough!).
        // =====================================================================
        "20:",
        "cmp x26, x16",
        "b.lo 952f",                // remaining < run.len -> STATUS_BUDGET_SHORT
        "ubfx x15, x2, #32, #16",   // x15 = run.bytes
        "mov w0, w2",               // zero-extend run.first into x0
        "add x17, x10, x0, lsl #4", // x17 = run_first_pc
        "mov x25, x17",             // x25 = pc
        "ldrb w0, [x25]",           // op = pc->op
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // chain_resolve (label 13): probe L1 & patch [x5] on hit, else 3-level walk.
        // =====================================================================
        "13:",
        "eor w0, w14, w14, lsr #12",
        "and w0, w0, #0xfff",
        "add x3, x9, x0, lsl #4",   // x3 = &l1[slot]
        "ldp x0, x2, [x3]",
        "cmp w0, w14",
        "b.ne 16f",                 // L1 miss (keep x5)
        "ubfx x16, x2, #48, #8",
        "cbz x16, 16f",             // empty slot
        "lsr x0, x0, #32",          // w0 = run_idx + 1
        "str w0, [x5]",             // patch chain slot in place
        "b 20b",

        // =====================================================================
        // l1_miss (labels 11 / 16): cold 3-level walk, fill [x3], optional patch [x5].
        // =====================================================================
        "11:",
        "mov x5, #0",               // indirect transfer: no chain slot to patch
        "16:",
        "ldr x4, [x29, #64]",       // directory (miss-only)
        "lsr w0, w14, #22",
        "ldr w0, [x4, x0, lsl #2]",
        "cbz w0, 951f",             // STATUS_MISS
        "sub w0, w0, #1",
        "ldr x4, [x29, #72]",       // region_ptrs (miss-only)
        "ldr x4, [x4, x0, lsl #3]", // region_ptr
        "ubfx w1, w14, #12, #10",
        "ldr w0, [x4, x1, lsl #2]",
        "cbz w0, 951f",             // STATUS_MISS
        "sub w0, w0, #1",
        "ldr x4, [x29, #80]",       // page_ptrs (miss-only)
        "ldr x4, [x4, x0, lsl #3]", // page_ptr
        "and w1, w14, #0xfff",
        "ldr w0, [x4, x1, lsl #2]", // w0 = run_idx + 1 (zero-ext x0)
        "cbz w0, 951f",             // STATUS_MISS
        "ldr x2, [x11, x0, lsl #3]",// Run (1-based index into runs - 8)
        "ubfx x16, x2, #48, #8",    // x16 = run.len
        "cbz x16, 951f",            // invalidated run -> STATUS_MISS
        "orr x1, x14, x0, lsl #32", // { eip: w14, run_idx: w0 }
        "stp x1, x2, [x3]",         // populate L1 slot in place
        "cbz x5, 20b",
        "str w0, [x5]",             // patch chain slot in place
        "b 20b",

        // =====================================================================
        // cold_outside (labels 18 / 19 / 21): exact `outside` containment & overlap.
        // =====================================================================
        "18:",
        "ldr w0, [x29, #120]",      // outside_start
        "ldr w1, [x29, #128]",      // outside_len
        "sub w2, w14, w0",          // eip.wrapping_sub(outside_start)
        "cmp w2, w1",
        "b.lo 950f",                // eip in outside -> STATUS_DONE

        "eor w0, w14, w14, lsr #12",
        "and w0, w0, #0xfff",
        "add x3, x9, x0, lsl #4",
        "ldp x0, x2, [x3]",
        "cmp w0, w14",
        "b.ne 19f",
        "ubfx x16, x2, #48, #8",
        "cbnz x16, 21f",

        "19:",
        "ldr x4, [x29, #64]",
        "lsr w0, w14, #22",
        "ldr w0, [x4, x0, lsl #2]",
        "cbz w0, 951f",
        "sub w0, w0, #1",
        "ldr x4, [x29, #72]",
        "ldr x4, [x4, x0, lsl #3]",
        "ubfx w1, w14, #12, #10",
        "ldr w0, [x4, x1, lsl #2]",
        "cbz w0, 951f",
        "sub w0, w0, #1",
        "ldr x4, [x29, #80]",
        "ldr x4, [x4, x0, lsl #3]",
        "and w1, w14, #0xfff",
        "ldr w0, [x4, x1, lsl #2]",
        "cbz w0, 951f",
        "ldr x2, [x11, x0, lsl #3]",
        "ubfx x16, x2, #48, #8",
        "cbz x16, 951f",
        "orr x1, x14, x0, lsl #32",
        "stp x1, x2, [x3]",

        "21:",
        "ubfx x15, x2, #32, #16",   // x15 = run.bytes
        "adds w3, w14, w15",        // w3 = eip + run.bytes, C set on u32 wrap
        "b.cs 953f",                // wraps u32 -> STATUS_OUTSIDE_OVERLAP
        "ldr w0, [x29, #120]",      // outside_start
        "cmp w3, w0",
        "b.ls 20b",                 // end <= outside_start -> safe
        "ldr w0, [x29, #124]",      // outside_end
        "cmp w14, w0",
        "b.lo 953f",                // eip < outside_end && end > outside_start -> overlap
        "b 20b",

        // =====================================================================
        // Op 0 (Slow) -> STATUS_FALLBACK (label 954)
        // =====================================================================
        "100:",
        "b 954f",

        // =====================================================================
        // Op 46 (X87Reg): dispatch via PC-relative table 920: (tags 0..=58)
        // =====================================================================
        "146:",
        "ldrb w0, [x25, #7]",       // X87Op tag (0..=58)
        "adr x1, 920f",
        "ldrsw x2, [x1, x0, lsl #2]",
        "add x1, x1, x2",
        "br x1",

        // =====================================================================
        // Op 47 (X87Mem): compute flat32 ea -> w4, dispatch via 930: (tags 59..=111)
        // =====================================================================
        "147:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",        // [base | index<<8 | scale<<16 | aux<<24]
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",     // index | (scale << 8)
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "lsr w0, w0, #24",          // X87Op tag (59..=111)
        "sub w0, w0, #59",
        "adr x1, 930f",
        "ldrsw x2, [x1, x0, lsl #2]",
        "add x1, x1, x2",
        "br x1",

        // =====================================================================
        // Op 1: Nop
        // =====================================================================
        "101:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 2: MovRegReg (r[dst] = r[src])
        // =====================================================================
        "102:",
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x1, lsl #2]",
        "str w2, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 3: MovRegImm (r[dst] = imm)
        // =====================================================================
        "103:",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x25, #12]",
        "str w2, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 4: MovRegMem (r[dst] = read32(ea))
        // =====================================================================
        "104:",
        // effective_flat32 -> w4
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",        // [base | index<<8 | scale<<16 | aux<<24]
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",     // index | (scale << 8)
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        // bounds check 4B
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w2, [x20, x4]",
        "ldrb w1, [x25, #2]",
        "str w2, [x19, x1, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 5: MovMemReg (write32(ea, r[src]))
        // =====================================================================
        "105:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        // bounds + single-page + code_bitmap check
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldrb w2, [x25, #3]",
        "ldr w2, [x19, x2, lsl #2]",
        "str w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 6: MovMemImm (write32(ea, imm))
        // =====================================================================
        "106:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x25, #12]",
        "str w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 7: Lea (r[dst] = effective_flat32; FLAG_FS already cleared)
        // =====================================================================
        "107:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "sbfx w1, w0, #8, #8",
        "tbnz w1, #31, 2f",
        "ubfx w3, w0, #16, #2",
        "ldr w2, [x19, x1, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "ldrb w1, [x25, #2]",
        "str w4, [x19, x1, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 8: ExtendReg (movzx/movsx r32, r8/r16; src pre-decoded r8_off when 8-bit)
        // =====================================================================
        "108:",
        "ldrb w0, [x25, #3]",       // src (r8_off when ZX8/SX8, reg 0..7 when ZX16/SX16)
        "ldrb w1, [x25, #7]",       // aux: 0=ZX8, 1=ZX16, 2=SX8, 3=SX16
        "ldrb w3, [x25, #2]",       // dst
        "tbnz w1, #0, 2f",          // bit 0 set -> 16-bit (1 or 3)
        "tbnz w1, #1, 1f",
        "ldrb w4, [x19, x0]",       // ZX8
        "b 4f",
        "1:",
        "ldrsb w4, [x19, x0]",      // SX8
        "b 4f",
        "2:",
        "ldr w4, [x19, x0, lsl #2]",
        "tbnz w1, #1, 3f",
        "uxth w4, w4",              // ZX16
        "b 4f",
        "3:",
        "sxth w4, w4",              // SX16
        "4:",
        "str w4, [x19, x3, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 9: ExtendMem (movzx/movsx r32, m8/m16)
        // =====================================================================
        "109:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "lsr w1, w0, #24",          // aux
        "sub w2, w4, w22",          // off = ea - floor (zero-extended in x2)
        "tbnz w1, #0, 5f",          // 16-bit
        // 8-bit bounds: off <= limit4 + 3
        "add x3, x23, #3",
        "cmp x2, x3",
        "b.hi 954f",
        "tbnz w1, #1, 4f",
        "ldrb w2, [x20, x4]",       // ZX8
        "b 7f",
        "4:",
        "ldrsb w2, [x20, x4]",      // SX8
        "b 7f",
        "5:",
        // 16-bit bounds: off <= limit4 + 2
        "add x3, x23, #2",
        "cmp x2, x3",
        "b.hi 954f",
        "tbnz w1, #1, 6f",
        "ldrh w2, [x20, x4]",       // ZX16
        "b 7f",
        "6:",
        "ldrsh w2, [x20, x4]",      // SX16
        "7:",
        "ldrb w3, [x25, #2]",
        "str w2, [x19, x3, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 10: PushReg
        // =====================================================================
        "110:",
        "ldrb w2, [x25, #3]",
        "ldr w2, [x19, x2, lsl #2]",// read value BEFORE modifying ESP!
        "ldr w4, [x19, #16]",       // ESP
        "sub w4, w4, #4",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "str w4, [x19, #16]",
        "str w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 11: PushImm
        // =====================================================================
        "111:",
        "ldr w4, [x19, #16]",
        "sub w4, w4, #4",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x25, #12]",
        "str w4, [x19, #16]",
        "str w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 12: PushMem
        // =====================================================================
        "112:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w5, [x20, x4]",        // value = read32(ea)
        "ldr w4, [x19, #16]",
        "sub w4, w4, #4",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "str w4, [x19, #16]",
        "str w5, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 13: PopReg
        // =====================================================================
        "113:",
        "ldr w4, [x19, #16]",       // sp
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w2, [x20, x4]",        // value = read32(sp)
        "add w4, w4, #4",
        "str w4, [x19, #16]",       // ESP = sp + 4 (overwritten if dst == ESP!)
        "ldrb w1, [x25, #2]",
        "str w2, [x19, x1, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 14: PopMem (ea sees post-pop ESP; rollback ESP on fault/code-hit)
        // =====================================================================
        "114:",
        "ldr w5, [x19, #16]",       // old sp
        "sub w0, w5, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x5]",        // value
        "add w2, w5, #4",
        "str w2, [x19, #16]",       // tentatively ESP = sp + 4 for ea calc
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ldr w2, [x19, x2, lsl #2]",
        "ubfx w0, w1, #8, #2",
        "lsl w2, w2, w0",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 4f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 4f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 4f",
        "str w3, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "4:",
        "str w5, [x19, #16]",       // rollback ESP before fallback
        "b 954f",

        // =====================================================================
        // Ops 15..17: AddRegReg / AddRegImm / AddRegMem
        // =====================================================================
        "115:", // AddRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "adds w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "116:", // AddRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "adds w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "117:", // AddRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "adds w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 18..20: SubRegReg / SubRegImm / SubRegMem
        // =====================================================================
        "118:", // SubRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "119:", // SubRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "120:", // SubRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 21..23: CmpRegReg / CmpRegImm / CmpRegMem
        // =====================================================================
        "121:", // CmpRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "122:", // CmpRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "123:", // CmpRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 24..26: AndRegReg / AndRegImm / AndRegMem
        // =====================================================================
        "124:", // AndRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "125:", // AndRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "126:", // AndRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 27..29: OrRegReg / OrRegImm / OrRegMem
        // =====================================================================
        "127:", // OrRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "orr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "128:", // OrRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "orr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "129:", // OrRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "orr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 30..32: XorRegReg / XorRegImm / XorRegMem
        // =====================================================================
        "130:", // XorRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "eor w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "131:", // XorRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "eor w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "132:", // XorRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "eor w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 33..34: TestRegReg / TestRegImm
        // =====================================================================
        "133:", // TestRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "134:", // TestRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 35..36: AluMemReg / AluMemImm (aux: 0=ADD,1=SUB,2=CMP,3=AND,4=OR,5=XOR,6=TEST)
        // =====================================================================
        "135:", // AluMemReg -> w3 = r[src]
        "ldrb w3, [x25, #3]",
        "ldr w3, [x19, x3, lsl #2]",
        "b 235f",

        "136:", // AluMemImm -> w3 = imm
        "ldr w3, [x25, #12]",

        "235:", // shared AluMem body: w3 = b
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w5, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w5",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        // Read bounds check first
        "sub w1, w4, w22",
        "cmp x1, x23",
        "b.hi 954f",
        "lsr w5, w0, #24",          // w5 = aux
        "ldr w2, [x20, x4]",        // a = read32(ea)
        // Read-only ALU ops (CMP=2, TEST=6) skip code_bitmap check!
        "cmp w5, #2",
        "b.eq 5f",
        "cmp w5, #6",
        "b.eq 6f",
        // Write check: single-page + code_bitmap BEFORE mutating flags or memory
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "cbz w5, 4f",               // 0 = ADD
        "cmp w5, #1",
        "b.eq 7f",                  // 1 = SUB
        "cmp w5, #3",
        "b.eq 8f",                  // 3 = AND
        "cmp w5, #4",
        "b.eq 9f",                  // 4 = OR
        // 5 = XOR
        "eor w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "4:", // ADD
        "adds w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "5:", // CMP (read-only)
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "6:", // TEST (read-only)
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "7:", // SUB
        "subs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "8:", // AND
        "ands w12, w2, w3",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "9:", // OR
        "orr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 37..40: IncReg / DecReg / IncMem / DecMem (preserve CF bit 29!)
        // =====================================================================
        "137:", // IncReg
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "and w1, w28, #0x20000000", // save old CF
        "adds w12, w2, #1",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",         // restore CF
        "eor w13, w2, w12",         // AF = ((a ^ res) & 0x10) != 0
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "138:", // DecReg
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "and w1, w28, #0x20000000",
        "subs w12, w2, #1",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "eor w13, w2, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "139:", // IncMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x20, x4]",
        "and w1, w28, #0x20000000",
        "adds w12, w2, #1",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "eor w13, w2, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "140:", // DecMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x20, x4]",
        "and w1, w28, #0x20000000",
        "subs w12, w2, #1",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "eor w13, w2, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 41..43: ShlRegImm / ShrRegImm / SarRegImm (count in src, 1..=31)
        // =====================================================================
        "141:", // ShlRegImm
        "ldrb w0, [x25, #2]",
        "ldrb w3, [x25, #3]",       // count (1..=31)
        "ldr w2, [x19, x0, lsl #2]",
        "lsl w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "mov w1, #32",
        "sub w1, w1, w3",
        "lsr w1, w2, w1",           // bit 0 = cf
        "bfi w28, w1, #29, #1",
        "eor w1, w1, w12, lsr #31", // bit 0 = of
        "bfi w28, w1, #28, #1",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "142:", // ShrRegImm
        "ldrb w0, [x25, #2]",
        "ldrb w3, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "lsr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "lsr w1, w2, w1",           // bit 0 = cf
        "bfi w28, w1, #29, #1",
        "lsr w1, w2, #31",          // bit 0 = of = (a as i32) < 0
        "bfi w28, w1, #28, #1",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "143:", // SarRegImm
        "ldrb w0, [x25, #2]",
        "ldrb w3, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "asr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "asr w1, w2, w1",           // bit 0 = cf
        "bfi w28, w1, #29, #1",     // of stays 0
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 44: ShiftRegCl (aux: 0=SHL, 1=SHR, 2=SAR; count = ECX & 31)
        // =====================================================================
        "144:",
        "ldr w3, [x19, #4]",        // ECX
        "and w3, w3, #31",
        "cbz w3, 4f",               // count == 0: preserve ALL flags!
        "ldrb w0, [x25, #2]",
        "ldrb w4, [x25, #7]",       // aux
        "ldr w2, [x19, x0, lsl #2]",
        "cbz w4, 1f",
        "cmp w4, #1",
        "b.eq 2f",
        // SAR
        "asr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "asr w1, w2, w1",
        "bfi w28, w1, #29, #1",
        "b 3f",
        "1:", // SHL
        "lsl w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "mov w1, #32",
        "sub w1, w1, w3",
        "lsr w1, w2, w1",
        "bfi w28, w1, #29, #1",
        "eor w1, w1, w12, lsr #31",
        "bfi w28, w1, #28, #1",
        "b 3f",
        "2:", // SHR
        "lsr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "lsr w1, w2, w1",
        "bfi w28, w1, #29, #1",
        "lsr w1, w2, #31",
        "bfi w28, w1, #28, #1",
        "3:",
        "str w12, [x19, x0, lsl #2]",
        "4:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 45: ShiftMem (aux: kind | SHIFT_CL; check count == 0 BEFORE read!)
        // =====================================================================
        "145:",
        "ldrb w5, [x25, #7]",       // aux
        "tbnz w5, #7, 1f",
        "ldrb w3, [x25, #3]",       // imm count
        "b 2f",
        "1:",
        "ldr w3, [x19, #4]",        // ECX
        "and w3, w3, #31",
        "2:",
        "cbz w3, 9f",               // count == 0: no memory read, no flag change!
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 3f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "3:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 5f",
        "tbnz w1, #7, 4f",
        "and w2, w1, #7",
        "ubfx w0, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w0",
        "add w4, w4, w2",
        "4:",
        "tbz w1, #10, 5f",
        "add w4, w4, w6",
        "5:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x20, x4]",        // a = read32(ea)
        "and w5, w5, #3",
        "cbz w5, 6f",
        "cmp w5, #1",
        "b.eq 7f",
        // SAR
        "asr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "asr w1, w2, w1",
        "bfi w28, w1, #29, #1",
        "b 8f",
        "6:", // SHL
        "lsl w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "mov w1, #32",
        "sub w1, w1, w3",
        "lsr w1, w2, w1",
        "bfi w28, w1, #29, #1",
        "eor w1, w1, w12, lsr #31",
        "bfi w28, w1, #28, #1",
        "b 8f",
        "7:", // SHR
        "lsr w12, w2, w3",
        "tst w12, w12",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "lsr w1, w2, w1",
        "bfi w28, w1, #29, #1",
        "lsr w1, w2, #31",
        "bfi w28, w1, #28, #1",
        "8:",
        "str w12, [x20, x4]",
        "9:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Terminal Op 48: JmpRel
        // =====================================================================
        "148:",
        "ldp w3, w0, [x25, #8]",    // w3 = disp, w0 = cached run_idx + 1 (zero-ext x0)
        "add w14, w14, w15",        // fallthrough = start_eip + run_bytes
        "add w14, w14, w3",         // target
        "add x5, x25, #12",         // &chain_slot (&pc->imm)
        "sub x26, x26, x16",        // remaining -= run_len
        "add x27, x27, x16",        // retired += run_len
        "b 12b",

        // =====================================================================
        // Terminal Op 49: JccRel (branchless 16x u16 NZCV LUT + csel + dual chain)
        // =====================================================================
        "149:",
        "ldrb w0, [x25, #2]",       // cc (0..15)
        "and w1, w0, #0xe",
        "cmp w1, #0xa",             // cold JP/JNP parity check
        "b.eq 210f",
        "adr x1, 910f",
        "ldrh w1, [x1, x0, lsl #1]",
        "lsr w2, w28, #28",         // nzcv = 0..15
        "lsr w1, w1, w2",
        "ldp w3, w4, [x25, #8]",    // w3 = disp, w4 = taken_chain (imm)
        "ldr w5, [x25, #4]",        // w5 = fallthrough_chain (base..=aux)
        "add w2, w14, w15",         // fallthrough
        "add w3, w2, w3",           // taken target
        "tst w1, #1",
        "csel w14, w3, w2, ne",     // w14 = target eip
        "csel w0, w4, w5, ne",      // w0 = selected chain (zero-ext x0)
        "add x4, x25, #12",         // &taken_chain (&pc->imm)
        "add x5, x25, #4",          // &fallthrough_chain (&pc->base)
        "csel x5, x4, x5, ne",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 12b",

        "210:", // JP (0xa) / JNP (0xb)
        "tbnz w28, #0, 1f",
        "eor w1, w12, w12, lsr #4",
        "and w1, w1, #0x0f",
        "mov w2, #0x9669",
        "lsr w1, w2, w1",
        "b 2f",
        "1:",
        "lsr w1, w28, #2",
        "2:",
        "eor w1, w1, w0",           // bit 0: PF ^ (cc & 1) == 1 iff taken
        "ldp w3, w4, [x25, #8]",
        "ldr w5, [x25, #4]",
        "add w2, w14, w15",
        "add w3, w2, w3",
        "tst w1, #1",
        "csel w14, w3, w2, ne",
        "csel w0, w4, w5, ne",
        "add x4, x25, #12",
        "add x5, x25, #4",
        "csel x5, x4, x5, ne",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 12b",

        // =====================================================================
        // Terminal Op 50: CallRel
        // =====================================================================
        "150:",
        "ldr w4, [x19, #16]",       // ESP
        "sub w4, w4, #4",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "add w2, w14, w15",         // ret_addr = start_eip + run_bytes
        "ldp w3, w0, [x25, #8]",    // w3 = disp, w0 = cached run_idx + 1 (zero-ext x0)
        "str w4, [x19, #16]",
        "str w2, [x20, x4]",
        "add w14, w2, w3",          // target = ret_addr + disp
        "add x5, x25, #12",         // &chain_slot (&pc->imm)
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 12b",

        // =====================================================================
        // Terminal Op 51: Ret (pop eip; esp += 4 + imm)
        // =====================================================================
        "151:",
        "ldr w4, [x19, #16]",       // sp
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w14, [x20, x4]",       // target = read32(sp)
        "ldr w0, [x25, #12]",       // imm
        "add w4, w4, #4",
        "add w4, w4, w0",
        "str w4, [x19, #16]",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 10b",

        // =====================================================================
        // Terminal Op 52: JmpReg
        // =====================================================================
        "152:",
        "ldrb w0, [x25, #3]",
        "ldr w14, [x19, x0, lsl #2]",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 10b",

        // =====================================================================
        // Terminal Op 53: JmpMem
        // =====================================================================
        "153:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w14, [x20, x4]",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 10b",

        // =====================================================================
        // Terminal Op 54: CallReg
        // =====================================================================
        "154:",
        "ldrb w3, [x25, #3]",
        "ldr w3, [x19, x3, lsl #2]",// target = r[src] BEFORE modifying ESP!
        "ldr w4, [x19, #16]",
        "sub w4, w4, #4",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "add w2, w14, w15",         // ret_addr
        "str w4, [x19, #16]",
        "str w2, [x20, x4]",
        "mov w14, w3",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 10b",

        // =====================================================================
        // Terminal Op 55: CallMem
        // =====================================================================
        "155:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",        // target = read32(ea)
        "ldr w4, [x19, #16]",
        "sub w4, w4, #4",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "add w2, w14, w15",         // ret_addr
        "str w4, [x19, #16]",
        "str w2, [x20, x4]",
        "mov w14, w3",
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 10b",

        // =====================================================================
        // Ops 56..60: Mov8RegReg / Mov8RegImm / Mov8RegMem / Mov8MemReg / Mov8MemImm
        // All 8-bit register operands carry pre-decoded byte offsets (`r8_off`).
        // =====================================================================
        "156:", // Mov8RegReg
        "ldrb w1, [x25, #3]",
        "ldrb w0, [x25, #2]",
        "ldrb w2, [x19, x1]",
        "strb w2, [x19, x0]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "157:", // Mov8RegImm
        "ldrb w1, [x25, #2]",
        "ldrb w2, [x25, #12]",
        "strb w2, [x19, x1]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "158:", // Mov8RegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #3",
        "cmp x0, x1",
        "b.hi 954f",
        "ldrb w2, [x20, x4]",
        "ldrb w1, [x25, #2]",
        "strb w2, [x19, x1]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "159:", // Mov8MemReg
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #3",
        "cmp x0, x1",
        "b.hi 954f",
        // 1-byte write never crosses a page boundary
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x1]",
        "strb w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "160:", // Mov8MemImm
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #3",
        "cmp x0, x1",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldrb w2, [x25, #12]",
        "strb w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 61..65: Mov16RegReg / Mov16RegImm / Mov16RegMem / Mov16MemReg / Mov16MemImm
        // =====================================================================
        "161:", // Mov16RegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "add x1, x19, x1, lsl #2",
        "ldrh w2, [x1]",
        "add x0, x19, x0, lsl #2",
        "strh w2, [x0]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "162:", // Mov16RegImm
        "ldrb w0, [x25, #2]",
        "ldrh w2, [x25, #12]",
        "add x0, x19, x0, lsl #2",
        "strh w2, [x0]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "163:", // Mov16RegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "ldrh w2, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "add x0, x19, x0, lsl #2",
        "strh w2, [x0]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "164:", // Mov16MemReg
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffe",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldrb w1, [x25, #3]",
        "add x1, x19, x1, lsl #2",
        "ldrh w2, [x1]",
        "strh w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "165:", // Mov16MemImm
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffe",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldrh w2, [x25, #12]",
        "strh w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 66..68: Add8RegReg / Add8RegImm / Alu8RegMem (pre-decoded r8_off)
        // =====================================================================
        "166:", // Add8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "adds w12, w0, w1",
        "mrs x28, nzcv",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "167:", // Add8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "adds w12, w0, w1",
        "mrs x28, nzcv",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "168:", // Alu8RegMem -> w3 = read_u8(ea)
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w1, w4, w22",
        "add x2, x23, #3",
        "cmp x1, x2",
        "b.hi 954f",
        "ldrb w3, [x20, x4]",
        "ldrb w4, [x25, #2]",       // pre-decoded dst r8 byte offset
        "ldrb w2, [x19, x4]",       // w2 = a (u8)
        "lsr w5, w0, #24",          // aux
        "cmp w5, #2",
        "b.eq 5f",                  // CMP
        "cmp w5, #6",
        "b.eq 6f",                  // TEST
        "cbz w5, 4f",               // ADD
        "cmp w5, #1",
        "b.eq 7f",                  // SUB
        "cmp w5, #3",
        "b.eq 8f",                  // AND
        "cmp w5, #4",
        "b.eq 9f",                  // OR
        // XOR
        "eor w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "4:", // ADD8
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "adds w12, w0, w1",
        "mrs x28, nzcv",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "5:", // CMP8
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "6:", // TEST8
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "7:", // SUB8
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "8:", // AND8
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "9:", // OR8
        "orr w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 69..70: Alu8MemImm / Alu8MemReg
        // =====================================================================
        "169:", // Alu8MemImm -> w3 = imm8
        "ldrb w3, [x25, #12]",
        "b 269f",

        "170:", // Alu8MemReg -> w3 = r8[src] (pre-decoded r8_off)
        "ldrb w1, [x25, #3]",
        "ldrb w3, [x19, x1]",

        "269:", // shared Alu8Mem body: w3 = b (u8)
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w5, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w5",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w1, w4, w22",
        "add x2, x23, #3",
        "cmp x1, x2",
        "b.hi 954f",
        "lsr w5, w0, #24",          // w5 = aux
        "ldrb w2, [x20, x4]",       // a = read_u8(ea)
        "cmp w5, #2",
        "b.eq 5f",                  // CMP8 (read-only)
        "cmp w5, #6",
        "b.eq 6f",                  // TEST8 (read-only)
        // Write check: 1-byte code_bitmap BEFORE mutating flags or memory
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "cbz w5, 4f",               // ADD8
        "cmp w5, #1",
        "b.eq 7f",                  // SUB8
        "cmp w5, #3",
        "b.eq 8f",                  // AND8
        "cmp w5, #4",
        "b.eq 9f",                  // OR8
        // XOR8
        "eor w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "4:", // ADD8 mem
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "adds w12, w0, w1",
        "mrs x28, nzcv",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "5:", // CMP8 mem
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "6:", // TEST8 mem
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "7:", // SUB8 mem
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "8:", // AND8 mem
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "9:", // OR8 mem
        "orr w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 71..73: Alu16RegImm / Alu16RegReg / Alu16RegMem
        // =====================================================================
        "171:", // Alu16RegImm -> w3 = imm16
        "ldrh w3, [x25, #12]",
        "b 271f",

        "172:", // Alu16RegReg -> w3 = r16[src]
        "ldrb w0, [x25, #3]",
        "add x0, x19, x0, lsl #2",
        "ldrh w3, [x0]",
        "b 271f",

        "173:", // Alu16RegMem -> w3 = read_u16(ea)
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "ldrh w3, [x20, x4]",

        "271:", // shared Alu16Reg body: w3 = b (u16)
        "ldrb w0, [x25, #2]",
        "add x4, x19, x0, lsl #2",  // x4 = &ctx.r[dst]
        "ldrh w2, [x4]",            // w2 = a (u16)
        "ldrb w5, [x25, #7]",       // aux
        "cmp w5, #2",
        "b.eq 5f",                  // CMP16
        "cmp w5, #6",
        "b.eq 6f",                  // TEST16
        "cbz w5, 4f",               // ADD16
        "cmp w5, #1",
        "b.eq 7f",                  // SUB16
        "cmp w5, #3",
        "b.eq 8f",                  // AND16
        "cmp w5, #4",
        "b.eq 9f",                  // OR16
        // XOR16
        "eor w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strh w12, [x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "4:", // ADD16
        "lsl w0, w2, #16",
        "lsl w1, w3, #16",
        "adds w12, w0, w1",
        "mrs x28, nzcv",
        "lsr w12, w12, #16",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strh w12, [x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "5:", // CMP16
        "lsl w0, w2, #16",
        "lsl w1, w3, #16",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #16",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "6:", // TEST16
        "and w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "7:", // SUB16
        "lsl w0, w2, #16",
        "lsl w1, w3, #16",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #16",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strh w12, [x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "8:", // AND16
        "and w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strh w12, [x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "9:", // OR16
        "orr w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strh w12, [x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 74..75: Alu16MemImm / Alu16MemReg
        // =====================================================================
        "174:", // Alu16MemImm -> w3 = imm16
        "ldrh w3, [x25, #12]",
        "b 274f",

        "175:", // Alu16MemReg -> w3 = r16[src]
        "ldrb w0, [x25, #3]",
        "add x0, x19, x0, lsl #2",
        "ldrh w3, [x0]",

        "274:", // shared Alu16Mem body: w3 = b (u16)
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w5, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w5",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w1, w4, w22",
        "add x2, x23, #2",
        "cmp x1, x2",
        "b.hi 954f",
        "lsr w5, w0, #24",          // w5 = aux
        "ldrh w2, [x20, x4]",       // a = read_u16(ea)
        "cmp w5, #2",
        "b.eq 5f",                  // CMP16 (read-only)
        "cmp w5, #6",
        "b.eq 6f",                  // TEST16 (read-only)
        // Write check: single-page + code_bitmap BEFORE mutating flags or memory
        "and w0, w4, #0xfff",
        "cmp w0, #0xffe",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "cbz w5, 4f",               // ADD16
        "cmp w5, #1",
        "b.eq 7f",                  // SUB16
        "cmp w5, #3",
        "b.eq 8f",                  // AND16
        "cmp w5, #4",
        "b.eq 9f",                  // OR16
        // XOR16
        "eor w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strh w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "4:", // ADD16 mem
        "lsl w0, w2, #16",
        "lsl w1, w3, #16",
        "adds w12, w0, w1",
        "mrs x28, nzcv",
        "lsr w12, w12, #16",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strh w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "5:", // CMP16 mem
        "lsl w0, w2, #16",
        "lsl w1, w3, #16",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #16",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "6:", // TEST16 mem
        "and w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "7:", // SUB16 mem
        "lsl w0, w2, #16",
        "lsl w1, w3, #16",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #16",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strh w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "8:", // AND16 mem
        "and w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strh w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",
        "9:", // OR16 mem
        "orr w12, w2, w3",
        "lsl w0, w12, #16",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strh w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 76..77: Imul2Reg / Imul2Mem (32-bit signed mul, sets CF=OF only)
        // =====================================================================
        "176:", // Imul2Reg
        "ldrb w0, [x25, #2]",       // dst
        "ldrb w1, [x25, #3]",       // src
        "ldrb w4, [x25, #7]",       // aux (0=r[dst], 1=imm)
        "ldr w2, [x19, x1, lsl #2]",
        "cbz w4, 1f",
        "ldr w3, [x25, #12]",
        "b 2f",
        "1:",
        "ldr w3, [x19, x0, lsl #2]",
        "2:",
        "smull x1, w2, w3",
        "cmp x1, w1, sxtw",
        "cset w2, ne",
        "bfi w28, w2, #29, #1",
        "bfi w28, w2, #28, #1",
        "str w1, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "177:", // Imul2Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w1, w4, w22",
        "cmp x1, x23",
        "b.hi 954f",
        "ldr w2, [x20, x4]",        // a = read32(ea)
        "lsr w4, w0, #24",          // aux
        "ldrb w0, [x25, #2]",       // dst
        "cbz w4, 4f",
        "ldr w3, [x25, #12]",
        "b 5f",
        "4:",
        "ldr w3, [x19, x0, lsl #2]",
        "5:",
        "smull x1, w2, w3",
        "cmp x1, w1, sxtw",
        "cset w2, ne",
        "bfi w28, w2, #29, #1",
        "bfi w28, w2, #28, #1",
        "str w1, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 78: SetccReg (r8[dst] = cond(aux); flags untouched)
        // =====================================================================
        "178:",
        "ldrb w0, [x25, #7]",       // cc
        "and w1, w0, #0xe",
        "cmp w1, #0xa",
        "b.eq 1f",
        "adr x1, 910f",
        "ldrh w1, [x1, x0, lsl #1]",
        "lsr w2, w28, #28",
        "lsr w1, w1, w2",
        "and w2, w1, #1",
        "b 3f",
        "1:",
        "tbnz w28, #0, 2f",
        "eor w1, w12, w12, lsr #4",
        "and w1, w1, #0x0f",
        "mov w2, #0x9669",
        "lsr w1, w2, w1",
        "eor w2, w1, w0",
        "and w2, w2, #1",
        "b 3f",
        "2:",
        "lsr w1, w28, #2",
        "eor w2, w1, w0",
        "and w2, w2, #1",
        "3:",
        "ldrb w1, [x25, #2]",       // pre-decoded r8_off
        "strb w2, [x19, x1]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Op 79: EndRun (sentinel after non-transfer straight-line run)
        // =====================================================================
        "179:",
        "ldr w0, [x25, #12]",       // w0 = cached run_idx + 1 (zero-ext x0)
        "add w14, w14, w15",
        "add x5, x25, #12",         // &chain_slot (&pc->imm)
        "sub x26, x26, x16",
        "add x27, x27, x16",
        "b 12b",

        // =====================================================================
        // Ops 80..93: Split 8-bit register ALU & SBB8 (pre-decoded r8_off, #282)
        // =====================================================================
        "680:", // Sub8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "681:", // Sub8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "682:", // Cmp8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "683:", // Cmp8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "lsl w0, w2, #24",
        "lsl w1, w3, #24",
        "subs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "684:", // And8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "685:", // And8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "686:", // Or8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "orr w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "687:", // Or8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "orr w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "688:", // Xor8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "eor w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "689:", // Xor8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "eor w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "690:", // Test8RegReg
        "ldrb w4, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x19, x1]",
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "691:", // Test8RegImm
        "ldrb w4, [x25, #2]",
        "ldrb w2, [x19, x4]",
        "ldrb w3, [x25, #12]",
        "and w12, w2, w3",
        "lsl w0, w12, #24",
        "tst w0, w0",
        "mrs x28, nzcv",
        "mov w13, #0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "692:", // Sbb8RegReg
        "ldrb w5, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x5]",
        "ldrb w3, [x19, x1]",
        "sbfx w4, w28, #29, #1",
        "lsr w4, w4, #8",           // 0x00000000 if CF==0, 0x00ffffff if CF==1
        "orr w1, w4, w3, lsl #24",
        "lsl w0, w2, #24",
        "eor w4, w28, #0x20000000", // ARM C = !x86 CF
        "msr nzcv, x4",
        "sbcs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x5]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "693:", // Sbb8RegImm
        "ldrb w5, [x25, #2]",
        "ldrb w2, [x19, x5]",
        "ldrb w3, [x25, #12]",
        "sbfx w4, w28, #29, #1",
        "lsr w4, w4, #8",
        "orr w1, w4, w3, lsl #24",
        "lsl w0, w2, #24",
        "eor w4, w28, #0x20000000",
        "msr nzcv, x4",
        "sbcs w12, w0, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "strb w12, [x19, x5]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 94..98: AdcRegReg / AdcRegImm / AdcRegMem / AdcMemReg / AdcMemImm
        // =====================================================================
        "694:", // AdcRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "msr nzcv, x28",
        "adcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "695:", // AdcRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "msr nzcv, x28",
        "adcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "696:", // AdcRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "msr nzcv, x28",
        "adcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "697:", // AdcMemReg
        "ldrb w1, [x25, #3]",
        "ldr w3, [x19, x1, lsl #2]",
        "b 1f",
        "698:", // AdcMemImm
        "ldr w3, [x25, #12]",
        "1:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 2f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "2:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 4f",
        "tbnz w1, #7, 3f",
        "and w2, w1, #7",
        "ubfx w5, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w5",
        "add w4, w4, w2",
        "3:",
        "tbz w1, #10, 4f",
        "add w4, w4, w6",
        "4:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x20, x4]",
        "msr nzcv, x28",
        "adcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 99..103: SbbRegReg / SbbRegImm / SbbRegMem / SbbMemReg / SbbMemImm
        // =====================================================================
        "699:", // SbbRegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "eor w4, w28, #0x20000000",
        "msr nzcv, x4",
        "sbcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "700:", // SbbRegImm
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x25, #12]",
        "eor w4, w28, #0x20000000",
        "msr nzcv, x4",
        "sbcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "701:", // SbbRegMem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "eor w4, w28, #0x20000000",
        "msr nzcv, x4",
        "sbcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "702:", // SbbMemReg
        "ldrb w1, [x25, #3]",
        "ldr w3, [x19, x1, lsl #2]",
        "b 1f",
        "703:", // SbbMemImm
        "ldr w3, [x25, #12]",
        "1:",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 2f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "2:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 4f",
        "tbnz w1, #7, 3f",
        "and w2, w1, #7",
        "ubfx w5, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w5",
        "add w4, w4, w2",
        "3:",
        "tbz w1, #10, 4f",
        "add w4, w4, w6",
        "4:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x20, x4]",
        "eor w5, w28, #0x20000000",
        "msr nzcv, x5",
        "sbcs w12, w2, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w2, w3",
        "eor w13, w13, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 104..117: Leave / Sahf / Lahf / Cdq / Cwde / Neg / Not / Xchg / Extend16
        // =====================================================================
        "704:", // Leave (check 4B read at EBP BEFORE mutating ESP/EBP)
        "ldr w4, [x19, #20]",       // frame = EBP
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w2, [x20, x4]",        // popped EBP
        "add w4, w4, #4",           // new ESP = frame + 4
        "str w4, [x19, #16]",
        "str w2, [x19, #20]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "705:", // Sahf (load SF, ZF, AF, PF, CF from AH; preserve OF)
        "ldrb w0, [x19, #1]",
        "and w28, w28, #0x10000000",// preserve OF (bit 28)
        "bfi w28, w0, #29, #1",     // CF -> bit 29
        "lsr w1, w0, #6",
        "bfi w28, w1, #30, #2",     // ZF -> bit 30, SF -> bit 31
        "mov w2, #0x14",            // AF (bit 4) | PF (bit 2)
        "and w0, w0, w2",
        "orr w28, w28, w0",
        "orr w28, w28, #1",         // mark materialized
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "706:", // Lahf (store SF|ZF|AF|PF|CF|0x02 into AH; materialize PF/AF if lazy)
        "tbnz w28, #0, 1f",
        "eor w1, w12, w12, lsr #4",
        "and w1, w1, #0x0f",
        "mov w2, #0x9669",
        "lsr w1, w2, w1",
        "bfi w28, w1, #2, #1",
        "lsr w1, w13, #4",
        "bfi w28, w1, #4, #1",
        "orr w28, w28, #1",
        "1:",
        "mov w2, #0x14",
        "and w0, w28, w2",          // AF | PF
        "orr w0, w0, #0x02",        // bit 1 always 1
        "ubfx w1, w28, #29, #1",    // CF -> bit 0
        "orr w0, w0, w1",
        "lsr w1, w28, #30",         // ZF -> bit 0, SF -> bit 1
        "bfi w0, w1, #6, #2",       // ZF -> bit 6, SF -> bit 7
        "strb w0, [x19, #1]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "707:", // Cdq (EDX = (EAX as i32) >> 31)
        "ldr w0, [x19]",
        "asr w0, w0, #31",
        "str w0, [x19, #8]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "708:", // Cwde (EAX = sign_extend_16(AX))
        "ldrsh w0, [x19]",
        "str w0, [x19]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "709:", // Neg32Reg
        "ldrb w0, [x25, #2]",
        "ldr w3, [x19, x0, lsl #2]",
        "subs w12, wzr, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w3, w12",
        "str w12, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "710:", // Neg32Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w3, [x20, x4]",
        "subs w12, wzr, w3",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "eor w13, w3, w12",
        "str w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "711:", // Neg8Reg (pre-decoded r8_off)
        "ldrb w0, [x25, #2]",
        "ldrb w3, [x19, x0]",
        "lsl w1, w3, #24",
        "subs w12, wzr, w1",
        "mrs x28, nzcv",
        "eor w28, w28, #0x20000000",
        "lsr w12, w12, #24",
        "eor w13, w3, w12",
        "strb w12, [x19, x0]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "712:", // Not32Reg
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "mvn w2, w2",
        "str w2, [x19, x0, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "713:", // Not32Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldr w2, [x20, x4]",
        "mvn w2, w2",
        "str w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "714:", // Not8Reg (pre-decoded r8_off)
        "ldrb w0, [x25, #2]",
        "ldrb w2, [x19, x0]",
        "mvn w2, w2",
        "strb w2, [x19, x0]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "715:", // Xchg32RegReg
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldr w2, [x19, x0, lsl #2]",
        "ldr w3, [x19, x1, lsl #2]",
        "str w3, [x19, x0, lsl #2]",
        "str w2, [x19, x1, lsl #2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "716:", // Xchg8RegReg (pre-decoded r8_off)
        "ldrb w0, [x25, #2]",
        "ldrb w1, [x25, #3]",
        "ldrb w2, [x19, x0]",
        "ldrb w3, [x19, x1]",
        "strb w3, [x19, x0]",
        "strb w2, [x19, x1]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "717:", // Extend16Reg (movzx/movsx r16, r8; src=r8_off, aux: 0=ZX, 1=SX)
        "ldrb w0, [x25, #3]",
        "ldrb w1, [x25, #7]",
        "ldrb w2, [x25, #2]",
        "cbnz w1, 1f",
        "ldrb w3, [x19, x0]",
        "b 2f",
        "1:",
        "ldrsb w3, [x19, x0]",
        "2:",
        "add x2, x19, x2, lsl #2",
        "strh w3, [x2]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 118..121: IncDec8Reg / IncDec8Mem / IncDec16Reg / IncDec16Mem
        // Preserve CF (bit 29); aux: 0=Inc, 1=Dec.
        // =====================================================================
        "718:", // IncDec8Reg
        "ldrb w4, [x25, #2]",       // pre-decoded r8_off
        "ldrb w2, [x19, x4]",
        "ldrb w5, [x25, #7]",
        "lsl w0, w2, #24",
        "and w1, w28, #0x20000000", // save old CF
        "mov w3, #0x01000000",
        "cbnz w5, 1f",
        "adds w12, w0, w3",
        "b 2f",
        "1:",
        "subs w12, w0, w3",
        "2:",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "lsr w12, w12, #24",
        "eor w13, w2, w12",
        "strb w12, [x19, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "719:", // IncDec8Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w1, w4, w22",
        "add x2, x23, #3",
        "cmp x1, x2",
        "b.hi 954f",
        "lsr w1, w4, #12",
        "lsr w2, w1, #6",
        "ldr x2, [x21, x2, lsl #3]",
        "lsr x2, x2, x1",
        "tbnz x2, #0, 954f",
        "lsr w5, w0, #24",
        "ldrb w2, [x20, x4]",
        "lsl w0, w2, #24",
        "and w1, w28, #0x20000000",
        "mov w3, #0x01000000",
        "cbnz w5, 4f",
        "adds w12, w0, w3",
        "b 5f",
        "4:",
        "subs w12, w0, w3",
        "5:",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "lsr w12, w12, #24",
        "eor w13, w2, w12",
        "strb w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "720:", // IncDec16Reg
        "ldrb w0, [x25, #2]",
        "add x4, x19, x0, lsl #2",
        "ldrh w2, [x4]",
        "ldrb w5, [x25, #7]",
        "lsl w0, w2, #16",
        "and w1, w28, #0x20000000",
        "mov w3, #0x00010000",
        "cbnz w5, 1f",
        "adds w12, w0, w3",
        "b 2f",
        "1:",
        "subs w12, w0, w3",
        "2:",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "lsr w12, w12, #16",
        "eor w13, w2, w12",
        "strh w12, [x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "721:", // IncDec16Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 1f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "1:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 3f",
        "tbnz w1, #7, 2f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "2:",
        "tbz w1, #10, 3f",
        "add w4, w4, w6",
        "3:",
        "sub w1, w4, w22",
        "add x2, x23, #2",
        "cmp x1, x2",
        "b.hi 954f",
        "and w1, w4, #0xfff",
        "cmp w1, #0xffe",
        "b.hi 954f",
        "lsr w1, w4, #12",
        "lsr w2, w1, #6",
        "ldr x2, [x21, x2, lsl #3]",
        "lsr x2, x2, x1",
        "tbnz x2, #0, 954f",
        "lsr w5, w0, #24",
        "ldrh w2, [x20, x4]",
        "lsl w0, w2, #16",
        "and w1, w28, #0x20000000",
        "mov w3, #0x00010000",
        "cbnz w5, 4f",
        "adds w12, w0, w3",
        "b 5f",
        "4:",
        "subs w12, w0, w3",
        "5:",
        "mrs x28, nzcv",
        "bic w28, w28, #0x20000000",
        "orr w28, w28, w1",
        "lsr w12, w12, #16",
        "eor w13, w2, w12",
        "strh w12, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 122..124: Shift8Reg / Shift16Reg / Rot32Reg (exact 80386 flags!)
        // =====================================================================
        "722:", // Shift8Reg (dst=r8_off, aux=(SHL=0..ROR=4)|SHIFT_CL)
        "ldrb w5, [x25, #7]",
        "tbnz w5, #7, 1f",
        "ldrb w3, [x25, #3]",
        "b 2f",
        "1:",
        "ldr w3, [x19, #4]",
        "and w3, w3, #31",
        "cbz w3, 9f",               // count == 0: preserve ALL flags!
        "2:",
        "ldrb w0, [x25, #2]",
        "ldrb w2, [x19, x0]",
        "and w5, w5, #7",
        "cmp w5, #1",
        "b.eq 4f",                  // SHR8 (hottest on Rock Raiders!)
        "cbz w5, 3f",               // SHL8
        "cmp w5, #2",
        "b.eq 5f",                  // SAR8
        "cmp w5, #3",
        "b.eq 6f",                  // ROL8
        // ROR8 (preserves SF, ZF, PF, AF, w12, w13!)
        "and w3, w3, #7",
        "lsr w1, w2, w3",
        "mov w4, #8",
        "sub w4, w4, w3",
        "lsl w4, w2, w4",
        "orr w1, w1, w4",
        "and w1, w1, #0xff",
        "lsr w4, w1, #7",
        "bfi w28, w4, #29, #1",     // CF = r bit 7
        "eor w4, w4, w1, lsr #6",
        "bfi w28, w4, #28, #1",     // OF = bit 7 ^ bit 6
        "strb w1, [x19, x0]",
        "b 9f",
        "3:", // SHL8
        "lsl w1, w2, w3",
        "and w12, w1, #0xff",
        "lsl w4, w12, #24",
        "tst w4, w4",
        "mrs x28, nzcv",
        "mov w13, #0",
        "lsr w4, w1, #8",           // CF exact for count 1..=31
        "bfi w28, w4, #29, #1",
        "eor w4, w4, w12, lsr #7",
        "bfi w28, w4, #28, #1",
        "strb w12, [x19, x0]",
        "b 9f",
        "4:", // SHR8
        "lsr w12, w2, w3",
        "lsl w4, w12, #24",
        "tst w4, w4",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "lsr w4, w2, w1",           // CF exact for count 1..=31
        "bfi w28, w4, #29, #1",
        "lsr w4, w2, #7",
        "bfi w28, w4, #28, #1",
        "strb w12, [x19, x0]",
        "b 9f",
        "5:", // SAR8
        "sxtb w2, w2",
        "asr w12, w2, w3",
        "and w12, w12, #0xff",
        "lsl w4, w12, #24",
        "tst w4, w4",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "asr w4, w2, w1",
        "bfi w28, w4, #29, #1",
        "strb w12, [x19, x0]",
        "b 9f",
        "6:", // ROL8 (preserves SF, ZF, PF, AF, w12, w13!)
        "and w3, w3, #7",
        "lsl w1, w2, w3",
        "mov w4, #8",
        "sub w4, w4, w3",
        "lsr w4, w2, w4",
        "orr w1, w1, w4",
        "and w1, w1, #0xff",
        "bfi w28, w1, #29, #1",     // CF = r bit 0
        "eor w4, w1, w1, lsr #7",
        "bfi w28, w4, #28, #1",     // OF = bit 7 ^ CF
        "strb w1, [x19, x0]",
        "9:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "723:", // Shift16Reg (aux=(SHL=0..SAR=2)|SHIFT_CL)
        "ldrb w5, [x25, #7]",
        "tbnz w5, #7, 1f",
        "ldrb w3, [x25, #3]",
        "b 2f",
        "1:",
        "ldr w3, [x19, #4]",
        "and w3, w3, #31",
        "cbz w3, 9f",
        "2:",
        "ldrb w0, [x25, #2]",
        "add x0, x19, x0, lsl #2",
        "ldrh w2, [x0]",
        "and w5, w5, #7",
        "cmp w5, #2",
        "b.eq 5f",                  // SAR16 (hottest on Alpha Team!)
        "cbz w5, 3f",               // SHL16
        // SHR16
        "lsr w12, w2, w3",
        "lsl w4, w12, #16",
        "tst w4, w4",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "lsr w4, w2, w1",
        "bfi w28, w4, #29, #1",
        "lsr w4, w2, #15",
        "bfi w28, w4, #28, #1",
        "strh w12, [x0]",
        "b 9f",
        "3:", // SHL16
        "lsl w1, w2, w3",
        "and w12, w1, #0xffff",
        "lsl w4, w12, #16",
        "tst w4, w4",
        "mrs x28, nzcv",
        "mov w13, #0",
        "lsr w4, w1, #16",
        "bfi w28, w4, #29, #1",
        "eor w4, w4, w12, lsr #15",
        "bfi w28, w4, #28, #1",
        "strh w12, [x0]",
        "b 9f",
        "5:", // SAR16
        "sxth w2, w2",
        "asr w12, w2, w3",
        "and w12, w12, #0xffff",
        "lsl w4, w12, #16",
        "tst w4, w4",
        "mrs x28, nzcv",
        "mov w13, #0",
        "sub w1, w3, #1",
        "asr w4, w2, w1",
        "bfi w28, w4, #29, #1",
        "strh w12, [x0]",
        "9:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "724:", // Rot32Reg (aux=(0=ROL,1=ROR)|SHIFT_CL; preserves SF,ZF,PF,AF!)
        "ldrb w5, [x25, #7]",
        "tbnz w5, #7, 1f",
        "ldrb w3, [x25, #3]",
        "b 2f",
        "1:",
        "ldr w3, [x19, #4]",
        "and w3, w3, #31",
        "cbz w3, 9f",
        "2:",
        "ldrb w0, [x25, #2]",
        "ldr w2, [x19, x0, lsl #2]",
        "tbnz w5, #0, 3f",
        // ROL32
        "neg w4, w3",
        "ror w1, w2, w4",
        "bfi w28, w1, #29, #1",
        "eor w4, w1, w1, lsr #31",
        "bfi w28, w4, #28, #1",
        "str w1, [x19, x0, lsl #2]",
        "b 9f",
        "3:", // ROR32
        "ror w1, w2, w3",
        "lsr w4, w1, #31",
        "bfi w28, w4, #29, #1",
        "eor w4, w4, w1, lsr #30",
        "bfi w28, w4, #28, #1",
        "str w1, [x19, x0, lsl #2]",
        "9:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 125..128: Mul32Reg / Mul32Mem / Div32Reg / Div32Mem (aux: 0=u, 1=s)
        // =====================================================================
        "725:", // Mul32Reg
        "ldrb w0, [x25, #3]",
        "ldr w3, [x19, x0, lsl #2]",
        "ldrb w5, [x25, #7]",
        "b 1f",

        "726:", // Mul32Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 4f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "4:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 6f",
        "tbnz w1, #7, 5f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "5:",
        "tbz w1, #10, 6f",
        "add w4, w4, w6",
        "6:",
        "sub w1, w4, w22",
        "cmp x1, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "lsr w5, w0, #24",

        "1:", // shared Mul32 body: w3 = src, w5 = signed
        "ldr w2, [x19]",            // EAX
        "cbnz w5, 2f",
        "umull x1, w2, w3",
        "lsr x2, x1, #32",
        "cmp w2, #0",
        "cset w0, ne",
        "b 3f",
        "2:",
        "smull x1, w2, w3",
        "lsr x2, x1, #32",
        "cmp x1, w1, sxtw",
        "cset w0, ne",
        "3:",
        "bfi w28, w0, #29, #1",
        "bfi w28, w0, #28, #1",
        "str w1, [x19]",            // EAX = low
        "str w2, [x19, #8]",        // EDX = high
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "727:", // Div32Reg (bail to 954: on #DE BEFORE mutating EAX/EDX!)
        "ldrb w0, [x25, #3]",
        "ldr w3, [x19, x0, lsl #2]",
        "ldrb w5, [x25, #7]",
        "b 1f",

        "728:", // Div32Mem
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 4f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "4:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 6f",
        "tbnz w1, #7, 5f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "5:",
        "tbz w1, #10, 6f",
        "add w4, w4, w6",
        "6:",
        "sub w1, w4, w22",
        "cmp x1, x23",
        "b.hi 954f",
        "ldr w3, [x20, x4]",
        "lsr w5, w0, #24",

        "1:", // shared Div32 body: w3 = divisor (zero-ext x3), w5 = signed
        "cbz w3, 954f",
        "ldr w0, [x19]",            // EAX (zero-ext x0)
        "ldr w1, [x19, #8]",        // EDX (zero-ext x1)
        "cbnz w5, 2f",
        // Unsigned DIV: overflow iff EDX >= divisor
        "cmp w1, w3",
        "b.hs 954f",
        "orr x0, x0, x1, lsl #32",
        "udiv x2, x0, x3",
        "msub x1, x2, x3, x0",
        "str w2, [x19]",
        "str w1, [x19, #8]",
        "b 3f",
        "2:", // Signed IDIV: check i32 range BEFORE mutating EAX/EDX
        "orr x0, x0, x1, lsl #32",
        "sxtw x3, w3",
        "sdiv x2, x0, x3",
        "cmp x2, w2, sxtw",
        "b.ne 954f",
        "msub x1, x2, x3, x0",
        "str w2, [x19]",
        "str w1, [x19, #8]",
        "3:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Ops 129..130: Cmovcc32Reg / Cmovcc32Mem (aux = cc 0..=15)
        // =====================================================================
        "729:", // Cmovcc32Reg
        "ldrb w0, [x25, #7]",       // cc
        "and w1, w0, #0xe",
        "cmp w1, #0xa",
        "b.eq 1f",
        "adr x1, 910f",
        "ldrh w1, [x1, x0, lsl #1]",
        "lsr w2, w28, #28",
        "lsr w1, w1, w2",
        "b 3f",
        "1:",
        "tbnz w28, #0, 2f",
        "eor w1, w12, w12, lsr #4",
        "and w1, w1, #0x0f",
        "mov w2, #0x9669",
        "lsr w1, w2, w1",
        "eor w1, w1, w0",
        "b 3f",
        "2:",
        "lsr w1, w28, #2",
        "eor w1, w1, w0",
        "3:",
        "tbz w1, #0, 4f",
        "ldrb w0, [x25, #3]",
        "ldrb w2, [x25, #2]",
        "ldr w3, [x19, x0, lsl #2]",
        "str w3, [x19, x2, lsl #2]",
        "4:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "730:", // Cmovcc32Mem (evaluate cc FIRST; skip EA/read when false!)
        "ldrb w0, [x25, #7]",
        "and w1, w0, #0xe",
        "cmp w1, #0xa",
        "b.eq 1f",
        "adr x1, 910f",
        "ldrh w1, [x1, x0, lsl #1]",
        "lsr w2, w28, #28",
        "lsr w1, w1, w2",
        "b 3f",
        "1:",
        "tbnz w28, #0, 2f",
        "eor w1, w12, w12, lsr #4",
        "and w1, w1, #0x0f",
        "mov w2, #0x9669",
        "lsr w1, w2, w1",
        "eor w1, w1, w0",
        "b 3f",
        "2:",
        "lsr w1, w28, #2",
        "eor w1, w1, w0",
        "3:",
        "tbz w1, #0, 7f",
        "ldr w4, [x25, #8]",
        "ldr w0, [x25, #4]",
        "sbfx w1, w0, #0, #8",
        "tbnz w1, #31, 4f",
        "ldr w2, [x19, x1, lsl #2]",
        "add w4, w4, w2",
        "4:",
        "ubfx w1, w0, #8, #16",
        "cmp w1, #0xff",
        "b.eq 6f",
        "tbnz w1, #7, 5f",
        "and w2, w1, #7",
        "ubfx w3, w1, #8, #2",
        "ldr w2, [x19, x2, lsl #2]",
        "lsl w2, w2, w3",
        "add w4, w4, w2",
        "5:",
        "tbz w1, #10, 6f",
        "add w4, w4, w6",
        "6:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w2, [x20, x4]",
        "ldrb w1, [x25, #2]",
        "str w2, [x19, x1, lsl #2]",
        "7:",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Inline hot scalar IEEE-754 x87 register ops (ADR-0009 Rung 4, #280)
        // Strictly volatile d0..d2 / s0..s1; pre-mutation bailout to 954:.
        // =====================================================================
        "300:", // Fnop
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "301:", // Fchs
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "fneg d0, d0",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "1:",
        "str d0, [x30, x1, lsl #3]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "302:", // Fabs
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "fabs d0, d0",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "1:",
        "str d0, [x30, x1, lsl #3]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "303:", // Ftst
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fmov d1, xzr",
        "b 501f",

        "305:", // Fld1
        "fmov d0, #1.0",
        "b 500f",

        "311:", // Fldz
        "fmov d0, xzr",
        "b 500f",

        "322:", // Fsqrt
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fsqrt d2, d0",
        "b 510f",

        "330:", // Fcompp / Fucompp
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "add w3, w1, #1",
        "and w3, w3, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fcmp d0, d1",
        "b.vs 954f",
        "mov w0, #0x4000",
        "mov w4, #0x0100",
        "csel w0, w0, wzr, eq",
        "csel w0, w4, w0, lt",
        "ldrh w4, [x30, #68]",
        "mov w5, #0x4500",
        "bic w4, w4, w5",
        "orr w4, w4, w0",
        "strh w4, [x30, #68]",
        "mov w0, #1",
        "lsl w4, w0, w1",
        "lsl w5, w0, w3",
        "orr w4, w4, w5",
        "bic w2, w2, w4",
        "add w1, w1, #2",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "331:", // FnstswAx
        "ldrb w0, [x30, #64]",
        "ldrh w1, [x30, #68]",
        "bic w1, w1, #0x3800",
        "and w0, w0, #7",
        "orr w1, w1, w0, lsl #11",
        "strh w1, [x19]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "332:", // FldSt(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",           // src = (top + i) & 7
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",         // st(i) empty?
        "sub w1, w1, #1",
        "and w1, w1, #7",           // new_top = (top - 1) & 7
        "lsr w4, w2, w1",
        "tbnz w4, #0, 954f",        // stack overflow?
        "ldr d0, [x30, x3, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "1:",
        "str d0, [x30, x1, lsl #3]",
        "mov w4, #1",
        "lsl w4, w4, w1",
        "orr w2, w2, w4",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "333:", // Fxch(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fcmp d0, d1",
        "b.vs 954f",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "fcvt s1, d1",
        "fcvt d1, s1",
        "1:",
        "str d1, [x30, x1, lsl #3]",
        "str d0, [x30, x3, lsl #3]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "335:", // FstSt(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "1:",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "str d0, [x30, x3, lsl #3]",
        "mov w4, #1",
        "lsl w4, w4, w3",
        "orr w2, w2, w4",
        "strb w2, [x30, #65]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "336:", // FstpSt(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "1:",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "str d0, [x30, x3, lsl #3]",
        "mov w4, #1",
        "lsl w5, w4, w3",
        "orr w2, w2, w5",           // set_st(i) marks dst occupied
        "lsl w5, w4, w1",
        "bic w2, w2, w5",           // pop() clears old top (handles i == 0!)
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "337:", // FcomSt(i) / FucomSt(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "b 501f",

        "338:", // FcompSt(i) / FucompSt(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "b 502f",

        "339:", // Fcomi(i) / Fucomi(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fcmp d0, d1",
        "b.vs 954f",
        "tbnz w28, #0, 1f",
        "lsr w0, w13, #4",
        "bfi w28, w0, #4, #1",      // materialize lazy AF before overwriting w28
        "1:",
        "and w28, w28, #0x10",      // preserve AF; SF=OF=PF=0
        "orr w28, w28, #1",         // mark materialized
        "mov w0, #0x40000000",      // ZF
        "mov w3, #0x20000000",      // CF
        "csel w0, w0, wzr, eq",
        "csel w0, w3, w0, lt",
        "orr w28, w28, w0",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "340:", // Fcomip(i) / Fucomip(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fcmp d0, d1",
        "b.vs 954f",
        "tbnz w28, #0, 1f",
        "lsr w0, w13, #4",
        "bfi w28, w0, #4, #1",
        "1:",
        "and w28, w28, #0x10",
        "orr w28, w28, #1",
        "mov w0, #0x40000000",
        "mov w3, #0x20000000",
        "csel w0, w0, wzr, eq",
        "csel w0, w3, w0, lt",
        "orr w28, w28, w0",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // Register binary arithmetic: St0 (341..=346)
        "341:", // FaddSt0(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fadd d2, d0, d1",
        "b 510f",

        "342:", // FmulSt0(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fmul d2, d0, d1",
        "b 510f",

        "343:", // FsubSt0(i): st(0) = st(0) - st(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fsub d2, d0, d1",
        "b 510f",

        "344:", // FsubrSt0(i): st(0) = st(i) - st(0)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fsub d2, d1, d0",
        "b 510f",

        "345:", // FdivSt0(i): st(0) = st(0) / st(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fdiv d2, d0, d1",
        "b 510f",

        "346:", // FdivrSt0(i): st(0) = st(i) / st(0)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fdiv d2, d1, d0",
        "b 510f",

        // Register binary arithmetic: Sti (347..=352, write to x1 = other)
        "347:", // FaddSti(i)
        "ldr w0, [x30, #64]",
        "and w3, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w1, [x25, #2]",
        "add w1, w3, w1",
        "and w1, w1, #7",           // x1 = other (dst for 510:)
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x3, lsl #3]",
        "ldr d1, [x30, x1, lsl #3]",
        "fadd d2, d0, d1",
        "b 510f",

        "348:", // FmulSti(i)
        "ldr w0, [x30, #64]",
        "and w3, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w1, [x25, #2]",
        "add w1, w3, w1",
        "and w1, w1, #7",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x3, lsl #3]",
        "ldr d1, [x30, x1, lsl #3]",
        "fmul d2, d0, d1",
        "b 510f",

        "349:", // FsubSti(i): st(i) = st(0) - st(i)
        "ldr w0, [x30, #64]",
        "and w3, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w1, [x25, #2]",
        "add w1, w3, w1",
        "and w1, w1, #7",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x3, lsl #3]",
        "ldr d1, [x30, x1, lsl #3]",
        "fsub d2, d0, d1",
        "b 510f",

        "350:", // FsubrSti(i): st(i) = st(i) - st(0)
        "ldr w0, [x30, #64]",
        "and w3, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w1, [x25, #2]",
        "add w1, w3, w1",
        "and w1, w1, #7",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x3, lsl #3]",
        "ldr d1, [x30, x1, lsl #3]",
        "fsub d2, d1, d0",
        "b 510f",

        "351:", // FdivSti(i): st(i) = st(0) / st(i)
        "ldr w0, [x30, #64]",
        "and w3, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w1, [x25, #2]",
        "add w1, w3, w1",
        "and w1, w1, #7",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x3, lsl #3]",
        "ldr d1, [x30, x1, lsl #3]",
        "fdiv d2, d0, d1",
        "b 510f",

        "352:", // FdivrSti(i): st(i) = st(i) / st(0)
        "ldr w0, [x30, #64]",
        "and w3, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w1, [x25, #2]",
        "add w1, w3, w1",
        "and w1, w1, #7",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x3, lsl #3]",
        "ldr d1, [x30, x1, lsl #3]",
        "fdiv d2, d1, d0",
        "b 510f",

        // Register binary arithmetic: Pop (353..=358, write to x3 = other then pop w1 = top)
        "353:", // Faddp(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fadd d2, d0, d1",
        "b 511f",

        "354:", // Fmulp(i)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fmul d2, d0, d1",
        "b 511f",

        "355:", // Fsubp(i): st(i) = st(0) - st(i); pop
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fsub d2, d0, d1",
        "b 511f",

        "356:", // Fsubrp(i): st(i) = st(i) - st(0); pop
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fsub d2, d1, d0",
        "b 511f",

        "357:", // Fdivp(i): st(i) = st(0) / st(i); pop
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fdiv d2, d0, d1",
        "b 511f",

        "358:", // Fdivrp(i): st(i) = st(i) / st(0); pop
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "ldrb w3, [x25, #2]",
        "add w3, w1, w3",
        "and w3, w3, #7",
        "lsr w4, w2, w1",
        "tbz w4, #0, 954f",
        "lsr w4, w2, w3",
        "tbz w4, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "ldr d1, [x30, x3, lsl #3]",
        "fdiv d2, d1, d0",
        "b 511f",

        // =====================================================================
        // Inline hot scalar IEEE-754 x87 memory ops (w4 = flat32 ea)
        // =====================================================================
        // f32 memory arithmetic & comparisons (359..=366)
        "359:", // FaddF32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "fadd d2, d0, d1",
        "b 510f",

        "360:", // FmulF32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "fmul d2, d0, d1",
        "b 510f",

        "361:", // FcomF32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "b 501f",

        "362:", // FcompF32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "b 502f",

        "363:", // FsubF32: st(0) = st(0) - m32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "fsub d2, d0, d1",
        "b 510f",

        "364:", // FsubrF32: st(0) = m32 - st(0)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "fsub d2, d1, d0",
        "b 510f",

        "365:", // FdivF32: st(0) = st(0) / m32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "fdiv d2, d0, d1",
        "b 510f",

        "366:", // FdivrF32: st(0) = m32 / st(0)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s1, [x20, x4]",
        "fcvt d1, s1",
        "ldr d0, [x30, x1, lsl #3]",
        "fdiv d2, d1, d0",
        "b 510f",

        // f64 memory arithmetic & comparisons (367..=374)
        "367:", // FaddF64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "fadd d2, d0, d1",
        "b 510f",

        "368:", // FmulF64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "fmul d2, d0, d1",
        "b 510f",

        "369:", // FcomF64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "b 501f",

        "370:", // FcompF64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "b 502f",

        "371:", // FsubF64: st(0) = st(0) - m64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "fsub d2, d0, d1",
        "b 510f",

        "372:", // FsubrF64: st(0) = m64 - st(0)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "fsub d2, d1, d0",
        "b 510f",

        "373:", // FdivF64: st(0) = st(0) / m64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "fdiv d2, d0, d1",
        "b 510f",

        "374:", // FdivrF64: st(0) = m64 / st(0)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d1, [x20, x4]",
        "ldr d0, [x30, x1, lsl #3]",
        "fdiv d2, d1, d0",
        "b 510f",

        // Memory loads (391..=394)
        "391:", // FldF32
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr s0, [x20, x4]",
        "fcvt d0, s0",
        "fcmp d0, d0",
        "b.vs 954f",
        "b 500f",

        "392:", // FldF64
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr d0, [x20, x4]",
        "fcmp d0, d0",
        "b.vs 954f",
        "b 500f",

        "393:", // FildI32
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "ldr w2, [x20, x4]",
        "scvtf d0, w2",
        "b 500f",

        "394:", // FildI16
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "ldrsh w2, [x20, x4]",
        "scvtf d0, w2",
        "b 500f",

        // Memory stores (397..=400, 403, 406, 407, 408)
        "397:", // FstF32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w3, w0, #6",
        "ldr x3, [x21, x3, lsl #3]",
        "lsr x3, x3, x0",
        "tbnz x3, #0, 954f",
        "fcvt s0, d0",
        "str s0, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "398:", // FstpF32
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w3, w0, #6",
        "ldr x3, [x21, x3, lsl #3]",
        "lsr x3, x3, x0",
        "tbnz x3, #0, 954f",
        "fcvt s0, d0",
        "str s0, [x20, x4]",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "399:", // FstF64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xff8",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w3, w0, #6",
        "ldr x3, [x21, x3, lsl #3]",
        "lsr x3, x3, x0",
        "tbnz x3, #0, 954f",
        "str d0, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "400:", // FstpF64
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr d0, [x30, x1, lsl #3]",
        "fcmp d0, d0",
        "b.vs 954f",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xff8",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w3, w0, #6",
        "ldr x3, [x21, x3, lsl #3]",
        "lsr x3, x3, x0",
        "tbnz x3, #0, 954f",
        "str d0, [x20, x4]",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "403:", // FistpI32 (RC == 3 trunc in [-2^31, 2^31), or RC == 0 RNE in |x| < 2^30)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr x5, [x30, x1, lsl #3]",
        "ubfx w0, w0, #26, #2",     // RC (0=RNE, 3=trunc)
        "cmp w0, #3",
        "b.eq 2f",
        "cbnz w0, 954f",
        "ubfx x3, x5, #52, #11",
        "cmp x3, #0x41d",           // RC == 0: |x| < 2^30
        "b.hs 954f",
        "fmov d0, x5",
        "fcvtns w5, d0",
        "b 3f",
        "2:",
        "ubfx x3, x5, #52, #11",
        "cmp x3, #0x41e",
        "b.lo 1f",
        "ror x3, x5, #52",
        "cmp x3, #0xc1e",           // exact -2^31?
        "b.ne 954f",
        "1:",
        "fmov d0, x5",
        "fcvtzs w5, d0",
        "3:",
        "sub w0, w4, w22",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffc",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w3, w0, #6",
        "ldr x3, [x21, x3, lsl #3]",
        "lsr x3, x3, x0",
        "tbnz x3, #0, 954f",
        "str w5, [x20, x4]",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "406:", // FistpI64 (RC == 3 trunc or RC == 0 RNE, finite |x| < 2^52)
        "ldr w0, [x30, #64]",
        "and w1, w0, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbz w3, #0, 954f",
        "ldr x5, [x30, x1, lsl #3]",
        "ubfx x3, x5, #52, #11",
        "cmp x3, #0x433",
        "b.hs 954f",
        "ubfx w0, w0, #26, #2",
        "cmp w0, #3",
        "b.eq 2f",
        "cbnz w0, 954f",
        "fmov d0, x5",
        "fcvtns x5, d0",
        "b 3f",
        "2:",
        "fmov d0, x5",
        "fcvtzs x5, d0",
        "3:",
        "sub w0, w4, w22",
        "add x0, x0, #4",
        "cmp x0, x23",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xff8",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w3, w0, #6",
        "ldr x3, [x21, x3, lsl #3]",
        "lsr x3, x3, x0",
        "tbnz x3, #0, 954f",
        "str x5, [x20, x4]",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "407:", // Fldcw (MSVC _ftol pair)
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "ldrh w2, [x20, x4]",
        "strh w2, [x30, #66]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "408:", // Fnstcw (MSVC _ftol pair)
        "sub w0, w4, w22",
        "add x1, x23, #2",
        "cmp x0, x1",
        "b.hi 954f",
        "and w0, w4, #0xfff",
        "cmp w0, #0xffe",
        "b.hi 954f",
        "lsr w0, w4, #12",
        "lsr w1, w0, #6",
        "ldr x1, [x21, x1, lsl #3]",
        "lsr x1, x1, x0",
        "tbnz x1, #0, 954f",
        "ldrh w2, [x30, #66]",
        "strh w2, [x20, x4]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Shared x87 commit tails (500..511)
        // =====================================================================
        "500:", // x87_push_d0 (non-NaN d0 -> check overflow, round if PC==0, push, next)
        "ldr w0, [x30, #64]",
        "sub w1, w0, #1",
        "and w1, w1, #7",
        "ubfx w2, w0, #8, #8",
        "lsr w3, w2, w1",
        "tbnz w3, #0, 954f",
        "tst w0, #0x03000000",
        "b.ne 1f",
        "fcvt s0, d0",
        "fcvt d0, s0",
        "1:",
        "str d0, [x30, x1, lsl #3]",
        "mov w3, #1",
        "lsl w3, w3, w1",
        "orr w2, w2, w3",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "501:", // x87_cmp_nopop (d0 vs d1 -> bail NaN, update C3/C2/C0, next)
        "fcmp d0, d1",
        "b.vs 954f",
        "mov w0, #0x4000",
        "mov w3, #0x0100",
        "csel w0, w0, wzr, eq",
        "csel w0, w3, w0, lt",
        "ldrh w3, [x30, #68]",
        "mov w4, #0x4500",
        "bic w3, w3, w4",
        "orr w3, w3, w0",
        "strh w3, [x30, #68]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "502:", // x87_cmp_pop1 (d0 vs d1, w1=top, w2=used -> bail NaN, status, pop, next)
        "fcmp d0, d1",
        "b.vs 954f",
        "mov w0, #0x4000",
        "mov w3, #0x0100",
        "csel w0, w0, wzr, eq",
        "csel w0, w3, w0, lt",
        "ldrh w3, [x30, #68]",
        "mov w4, #0x4500",
        "bic w3, w3, w4",
        "orr w3, w3, w0",
        "strh w3, [x30, #68]",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "510:", // x87_store_d2_x1 (bail NaN, round if PC==0, regs[x1] = d2, next)
        "fcmp d2, d2",
        "b.vs 954f",
        "ldrh w0, [x30, #66]",
        "tst w0, #0x0300",
        "b.ne 1f",
        "fcvt s2, d2",
        "fcvt d2, s2",
        "1:",
        "str d2, [x30, x1, lsl #3]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        "511:", // x87_store_d2_x3_pop (bail NaN, round if PC==0, regs[x3] = d2, pop w1/w2, next)
        "fcmp d2, d2",
        "b.vs 954f",
        "ldrh w0, [x30, #66]",
        "tst w0, #0x0300",
        "b.ne 1f",
        "fcvt s2, d2",
        "fcvt d2, s2",
        "1:",
        "str d2, [x30, x3, lsl #3]",
        "mov w0, #1",
        "lsl w0, w0, w1",
        "bic w2, w2, w0",
        "add w1, w1, #1",
        "and w1, w1, #7",
        "orr w1, w1, w2, lsl #8",
        "strh w1, [x30, #64]",
        "ldrb w0, [x25, #16]!",
        "ldrsw x1, [x24, x0, lsl #2]",
        "add x1, x24, x1",
        "br x1",

        // =====================================================================
        // Exit trampolines: materialize lazy PF/AF, write back FastCtx, return.
        // =====================================================================
        "950:",
        "mov w0, #{STATUS_DONE}",
        "b 960f",

        "951:",
        "mov w0, #{STATUS_MISS}",
        "b 960f",

        "952:",
        "mov w0, #{STATUS_BUDGET_SHORT}",
        "b 960f",

        "953:",
        "mov w0, #{STATUS_OUTSIDE_OVERLAP}",
        "b 960f",

        "954:",
        "str x25, [x29, #88]",      // fault_pc
        "str x17, [x29, #96]",      // run_first_pc
        "str w14, [x29, #132]",     // run_start_eip
        "str w15, [x29, #136]",     // run_bytes
        "str w16, [x29, #140]",     // run_len
        "mov w0, #{STATUS_FALLBACK}",

        "960:",
        // Materialize PF (bit 2) and AF (bit 4) if bit 0 == 0
        "tbnz w28, #0, 961f",
        "eor w1, w12, w12, lsr #4",
        "and w1, w1, #0x0f",
        "mov w2, #0x9669",
        "lsr w1, w2, w1",
        "bfi w28, w1, #2, #1",      // PF -> bit 2
        "lsr w1, w13, #4",
        "bfi w28, w1, #4, #1",      // AF -> bit 4
        "orr w28, w28, #1",         // mark materialized
        "961:",
        "str w14, [x29, #104]",     // eip
        "str w28, [x29, #108]",     // flags
        "str x26, [x29, #32]",      // remaining
        "str x27, [x29, #40]",      // retired

        "ldp x27, x28, [sp, #80]",
        "ldp x25, x26, [sp, #64]",
        "ldp x23, x24, [sp, #48]",
        "ldp x21, x22, [sp, #32]",
        "ldp x19, x20, [sp, #16]",
        "ldp x29, x30, [sp], #96",
        "ret",

        // =====================================================================
        // Read-only PC-relative dispatch tables (zero relocations on Mach-O/ELF/COFF)
        // =====================================================================
        ".p2align 2",
        "900:",
        ".long 100b - 900b", // 0: Slow
        ".long 101b - 900b", // 1: Nop
        ".long 102b - 900b", // 2: MovRegReg
        ".long 103b - 900b", // 3: MovRegImm
        ".long 104b - 900b", // 4: MovRegMem
        ".long 105b - 900b", // 5: MovMemReg
        ".long 106b - 900b", // 6: MovMemImm
        ".long 107b - 900b", // 7: Lea
        ".long 108b - 900b", // 8: ExtendReg
        ".long 109b - 900b", // 9: ExtendMem
        ".long 110b - 900b", // 10: PushReg
        ".long 111b - 900b", // 11: PushImm
        ".long 112b - 900b", // 12: PushMem
        ".long 113b - 900b", // 13: PopReg
        ".long 114b - 900b", // 14: PopMem
        ".long 115b - 900b", // 15: AddRegReg
        ".long 116b - 900b", // 16: AddRegImm
        ".long 117b - 900b", // 17: AddRegMem
        ".long 118b - 900b", // 18: SubRegReg
        ".long 119b - 900b", // 19: SubRegImm
        ".long 120b - 900b", // 20: SubRegMem
        ".long 121b - 900b", // 21: CmpRegReg
        ".long 122b - 900b", // 22: CmpRegImm
        ".long 123b - 900b", // 23: CmpRegMem
        ".long 124b - 900b", // 24: AndRegReg
        ".long 125b - 900b", // 25: AndRegImm
        ".long 126b - 900b", // 26: AndRegMem
        ".long 127b - 900b", // 27: OrRegReg
        ".long 128b - 900b", // 28: OrRegImm
        ".long 129b - 900b", // 29: OrRegMem
        ".long 130b - 900b", // 30: XorRegReg
        ".long 131b - 900b", // 31: XorRegImm
        ".long 132b - 900b", // 32: XorRegMem
        ".long 133b - 900b", // 33: TestRegReg
        ".long 134b - 900b", // 34: TestRegImm
        ".long 135b - 900b", // 35: AluMemReg
        ".long 136b - 900b", // 36: AluMemImm
        ".long 137b - 900b", // 37: IncReg
        ".long 138b - 900b", // 38: DecReg
        ".long 139b - 900b", // 39: IncMem
        ".long 140b - 900b", // 40: DecMem
        ".long 141b - 900b", // 41: ShlRegImm
        ".long 142b - 900b", // 42: ShrRegImm
        ".long 143b - 900b", // 43: SarRegImm
        ".long 144b - 900b", // 44: ShiftRegCl
        ".long 145b - 900b", // 45: ShiftMem
        ".long 146b - 900b", // 46: X87Reg
        ".long 147b - 900b", // 47: X87Mem
        ".long 148b - 900b", // 48: JmpRel
        ".long 149b - 900b", // 49: JccRel
        ".long 150b - 900b", // 50: CallRel
        ".long 151b - 900b", // 51: Ret
        ".long 152b - 900b", // 52: JmpReg
        ".long 153b - 900b", // 53: JmpMem
        ".long 154b - 900b", // 54: CallReg
        ".long 155b - 900b", // 55: CallMem
        ".long 156b - 900b", // 56: Mov8RegReg
        ".long 157b - 900b", // 57: Mov8RegImm
        ".long 158b - 900b", // 58: Mov8RegMem
        ".long 159b - 900b", // 59: Mov8MemReg
        ".long 160b - 900b", // 60: Mov8MemImm
        ".long 161b - 900b", // 61: Mov16RegReg
        ".long 162b - 900b", // 62: Mov16RegImm
        ".long 163b - 900b", // 63: Mov16RegMem
        ".long 164b - 900b", // 64: Mov16MemReg
        ".long 165b - 900b", // 65: Mov16MemImm
        ".long 166b - 900b", // 66: Add8RegReg
        ".long 167b - 900b", // 67: Add8RegImm
        ".long 168b - 900b", // 68: Alu8RegMem
        ".long 169b - 900b", // 69: Alu8MemImm
        ".long 170b - 900b", // 70: Alu8MemReg
        ".long 171b - 900b", // 71: Alu16RegImm
        ".long 172b - 900b", // 72: Alu16RegReg
        ".long 173b - 900b", // 73: Alu16RegMem
        ".long 174b - 900b", // 74: Alu16MemImm
        ".long 175b - 900b", // 75: Alu16MemReg
        ".long 176b - 900b", // 76: Imul2Reg
        ".long 177b - 900b", // 77: Imul2Mem
        ".long 178b - 900b", // 78: SetccReg
        ".long 179b - 900b", // 79: EndRun
        ".long 680b - 900b", // 80: Sub8RegReg
        ".long 681b - 900b", // 81: Sub8RegImm
        ".long 682b - 900b", // 82: Cmp8RegReg
        ".long 683b - 900b", // 83: Cmp8RegImm
        ".long 684b - 900b", // 84: And8RegReg
        ".long 685b - 900b", // 85: And8RegImm
        ".long 686b - 900b", // 86: Or8RegReg
        ".long 687b - 900b", // 87: Or8RegImm
        ".long 688b - 900b", // 88: Xor8RegReg
        ".long 689b - 900b", // 89: Xor8RegImm
        ".long 690b - 900b", // 90: Test8RegReg
        ".long 691b - 900b", // 91: Test8RegImm
        ".long 692b - 900b", // 92: Sbb8RegReg
        ".long 693b - 900b", // 93: Sbb8RegImm
        ".long 694b - 900b", // 94: AdcRegReg
        ".long 695b - 900b", // 95: AdcRegImm
        ".long 696b - 900b", // 96: AdcRegMem
        ".long 697b - 900b", // 97: AdcMemReg
        ".long 698b - 900b", // 98: AdcMemImm
        ".long 699b - 900b", // 99: SbbRegReg
        ".long 700b - 900b", // 100: SbbRegImm
        ".long 701b - 900b", // 101: SbbRegMem
        ".long 702b - 900b", // 102: SbbMemReg
        ".long 703b - 900b", // 103: SbbMemImm
        ".long 704b - 900b", // 104: Leave
        ".long 705b - 900b", // 105: Sahf
        ".long 706b - 900b", // 106: Lahf
        ".long 707b - 900b", // 107: Cdq
        ".long 708b - 900b", // 108: Cwde
        ".long 709b - 900b", // 109: Neg32Reg
        ".long 710b - 900b", // 110: Neg32Mem
        ".long 711b - 900b", // 111: Neg8Reg
        ".long 712b - 900b", // 112: Not32Reg
        ".long 713b - 900b", // 113: Not32Mem
        ".long 714b - 900b", // 114: Not8Reg
        ".long 715b - 900b", // 115: Xchg32RegReg
        ".long 716b - 900b", // 116: Xchg8RegReg
        ".long 717b - 900b", // 117: Extend16Reg
        ".long 718b - 900b", // 118: IncDec8Reg
        ".long 719b - 900b", // 119: IncDec8Mem
        ".long 720b - 900b", // 120: IncDec16Reg
        ".long 721b - 900b", // 121: IncDec16Mem
        ".long 722b - 900b", // 122: Shift8Reg
        ".long 723b - 900b", // 123: Shift16Reg
        ".long 724b - 900b", // 124: Rot32Reg
        ".long 725b - 900b", // 125: Mul32Reg
        ".long 726b - 900b", // 126: Mul32Mem
        ".long 727b - 900b", // 127: Div32Reg
        ".long 728b - 900b", // 128: Div32Mem
        ".long 729b - 900b", // 129: Cmovcc32Reg
        ".long 730b - 900b", // 130: Cmovcc32Mem

        // 16 x u16 NZCV condition masks (indexed by cc, bit = (N<<3)|(Z<<2)|(C<<1)|V)
        ".p2align 1",
        "910:",
        ".short 0xAAAA", // 0x0: JO  (V=1)
        ".short 0x5555", // 0x1: JNO (V=0)
        ".short 0xCCCC", // 0x2: JB  (C=1)
        ".short 0x3333", // 0x3: JAE (C=0)
        ".short 0xF0F0", // 0x4: JE  (Z=1)
        ".short 0x0F0F", // 0x5: JNE (Z=0)
        ".short 0xFCFC", // 0x6: JBE (C|Z)
        ".short 0x0303", // 0x7: JA  (!C&!Z)
        ".short 0xFF00", // 0x8: JS  (N=1)
        ".short 0x00FF", // 0x9: JNS (N=0)
        ".short 0x0000", // 0xa: JP  (handled via parity path)
        ".short 0x0000", // 0xb: JNP (handled via parity path)
        ".short 0x55AA", // 0xc: JL  (N!=V)
        ".short 0xAA55", // 0xd: JGE (N==V)
        ".short 0xF5FA", // 0xe: JLE (Z|(N!=V))
        ".short 0x0A05", // 0xf: JG  (!Z&(N==V))

        // X87Reg PC-relative jump table (tags 0..=58)
        ".p2align 2",
        "920:",
        ".long 300b - 920b", // 0: Fnop
        ".long 301b - 920b", // 1: Fchs
        ".long 302b - 920b", // 2: Fabs
        ".long 303b - 920b", // 3: Ftst
        ".long 954b - 920b", // 4: Fxam
        ".long 305b - 920b", // 5: Fld1
        ".long 954b - 920b", // 6: Fldl2t
        ".long 954b - 920b", // 7: Fldl2e
        ".long 954b - 920b", // 8: Fldpi
        ".long 954b - 920b", // 9: Fldlg2
        ".long 954b - 920b", // 10: Fldln2
        ".long 311b - 920b", // 11: Fldz
        ".long 954b - 920b", // 12: F2xm1
        ".long 954b - 920b", // 13: Fyl2x
        ".long 954b - 920b", // 14: Fptan
        ".long 954b - 920b", // 15: Fpatan
        ".long 954b - 920b", // 16: Fxtract
        ".long 954b - 920b", // 17: Fprem1
        ".long 954b - 920b", // 18: Fdecstp
        ".long 954b - 920b", // 19: Fincstp
        ".long 954b - 920b", // 20: Fprem
        ".long 954b - 920b", // 21: Fyl2xp1
        ".long 322b - 920b", // 22: Fsqrt
        ".long 954b - 920b", // 23: Fsincos
        ".long 954b - 920b", // 24: Frndint
        ".long 954b - 920b", // 25: Fscale
        ".long 954b - 920b", // 26: Fsin
        ".long 954b - 920b", // 27: Fcos
        ".long 954b - 920b", // 28: Fnclex
        ".long 954b - 920b", // 29: Fninit
        ".long 330b - 920b", // 30: Fcompp / Fucompp
        ".long 331b - 920b", // 31: FnstswAx
        ".long 332b - 920b", // 32: FldSt
        ".long 333b - 920b", // 33: Fxch
        ".long 954b - 920b", // 34: Ffree
        ".long 335b - 920b", // 35: FstSt
        ".long 336b - 920b", // 36: FstpSt
        ".long 337b - 920b", // 37: FcomSt / FucomSt
        ".long 338b - 920b", // 38: FcompSt / FucompSt
        ".long 339b - 920b", // 39: Fcomi / Fucomi
        ".long 340b - 920b", // 40: Fcomip / Fucomip
        ".long 341b - 920b", // 41: FaddSt0
        ".long 342b - 920b", // 42: FmulSt0
        ".long 343b - 920b", // 43: FsubSt0
        ".long 344b - 920b", // 44: FsubrSt0
        ".long 345b - 920b", // 45: FdivSt0
        ".long 346b - 920b", // 46: FdivrSt0
        ".long 347b - 920b", // 47: FaddSti
        ".long 348b - 920b", // 48: FmulSti
        ".long 349b - 920b", // 49: FsubSti
        ".long 350b - 920b", // 50: FsubrSti
        ".long 351b - 920b", // 51: FdivSti
        ".long 352b - 920b", // 52: FdivrSti
        ".long 353b - 920b", // 53: Faddp
        ".long 354b - 920b", // 54: Fmulp
        ".long 355b - 920b", // 55: Fsubp
        ".long 356b - 920b", // 56: Fsubrp
        ".long 357b - 920b", // 57: Fdivp
        ".long 358b - 920b", // 58: Fdivrp

        // X87Mem PC-relative jump table (tags 59..=111)
        ".p2align 2",
        "930:",
        ".long 359b - 930b", // 59: FaddF32
        ".long 360b - 930b", // 60: FmulF32
        ".long 361b - 930b", // 61: FcomF32
        ".long 362b - 930b", // 62: FcompF32
        ".long 363b - 930b", // 63: FsubF32
        ".long 364b - 930b", // 64: FsubrF32
        ".long 365b - 930b", // 65: FdivF32
        ".long 366b - 930b", // 66: FdivrF32
        ".long 367b - 930b", // 67: FaddF64
        ".long 368b - 930b", // 68: FmulF64
        ".long 369b - 930b", // 69: FcomF64
        ".long 370b - 930b", // 70: FcompF64
        ".long 371b - 930b", // 71: FsubF64
        ".long 372b - 930b", // 72: FsubrF64
        ".long 373b - 930b", // 73: FdivF64
        ".long 374b - 930b", // 74: FdivrF64
        ".long 954b - 930b", // 75: FiaddI32
        ".long 954b - 930b", // 76: FimulI32
        ".long 954b - 930b", // 77: FicomI32
        ".long 954b - 930b", // 78: FicompI32
        ".long 954b - 930b", // 79: FisubI32
        ".long 954b - 930b", // 80: FisubrI32
        ".long 954b - 930b", // 81: FidivI32
        ".long 954b - 930b", // 82: FidivrI32
        ".long 954b - 930b", // 83: FiaddI16
        ".long 954b - 930b", // 84: FimulI16
        ".long 954b - 930b", // 85: FicomI16
        ".long 954b - 930b", // 86: FicompI16
        ".long 954b - 930b", // 87: FisubI16
        ".long 954b - 930b", // 88: FisubrI16
        ".long 954b - 930b", // 89: FidivI16
        ".long 954b - 930b", // 90: FidivrI16
        ".long 391b - 930b", // 91: FldF32
        ".long 392b - 930b", // 92: FldF64
        ".long 393b - 930b", // 93: FildI32
        ".long 394b - 930b", // 94: FildI16
        ".long 954b - 930b", // 95: FldF80
        ".long 954b - 930b", // 96: FildI64
        ".long 397b - 930b", // 97: FstF32
        ".long 398b - 930b", // 98: FstpF32
        ".long 399b - 930b", // 99: FstF64
        ".long 400b - 930b", // 100: FstpF64
        ".long 954b - 930b", // 101: FstpF80
        ".long 954b - 930b", // 102: FistI32
        ".long 403b - 930b", // 103: FistpI32
        ".long 954b - 930b", // 104: FistI16
        ".long 954b - 930b", // 105: FistpI16
        ".long 406b - 930b", // 106: FistpI64
        ".long 407b - 930b", // 107: Fldcw
        ".long 408b - 930b", // 108: Fnstcw
        ".long 954b - 930b", // 109: FnstswMem
        ".long 954b - 930b", // 110: Fnstenv
        ".long 954b - 930b", // 111: Fldenv

        STATUS_DONE = const STATUS_DONE,
        STATUS_MISS = const STATUS_MISS,
        STATUS_BUDGET_SHORT = const STATUS_BUDGET_SHORT,
        STATUS_OUTSIDE_OVERLAP = const STATUS_OUTSIDE_OVERLAP,
        STATUS_FALLBACK = const STATUS_FALLBACK,
    );
}
