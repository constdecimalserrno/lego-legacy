//! The interpreter's own corners, where a run through the seam cannot reach.

use lego_legacy_core::cpu::{interp::Interp, Cpu, CpuEngine, Mode, Stop, EAX};
use lego_legacy_core::guest::{Arena, GuestMem};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::ARENA_BYTES;

#[test]
fn an_instruction_at_the_very_end_of_the_arena_is_still_bounded() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let last = u32::try_from(ARENA_BYTES).unwrap() - 1;

    // `inc eax` as the arena's final byte: it fits, so it retires.
    mem.write(last, &[0x40]).unwrap();
    let mut regs = Cpu {
        eip: last,
        ..Cpu::default()
    };
    let mut cpu = Interp::new(Mode::FLAT);
    cpu.step(&mut regs, &mut mem)
        .expect("the last byte of the arena is executable");
    assert_eq!(regs.r[EAX], 1);

    // The next fetch is off the end, and is refused rather than read.
    let stop = cpu.step(&mut regs, &mut mem).unwrap_err();
    assert!(
        matches!(stop, Stop::Fault(_)),
        "stopped {stop} instead of faulting"
    );
}

#[test]
fn an_opcode_the_engine_does_not_decode_stops_the_run_where_it_stands() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    // `ud2`: the two-byte opcode that exists to be undefined.
    mem.write(0x0040_0000, &[0x0f, 0x0b]).unwrap();

    let mut regs = Cpu {
        eip: 0x0040_0000,
        ..Cpu::default()
    };
    let stop = Interp::new(Mode::FLAT)
        .step(&mut regs, &mut mem)
        .unwrap_err();

    assert!(
        matches!(
            stop,
            Stop::Undecoded {
                eip: 0x0040_0000,
                byte: 0x0b,
                ..
            }
        ),
        "stopped {stop} instead of refusing the opcode"
    );
    assert_eq!(
        regs.eip, 0x0040_0000,
        "a wall leaves the cursor where the guest stood"
    );
    assert!(stop.to_string().contains("undecoded opcode 0x0b"));
    assert!(
        stop.to_string().contains("two-byte opcode"),
        "a wall says what it could not read"
    );
}

#[test]
fn code_is_decoded_once_however_often_it_runs() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    mem.write(
        lego_legacy_core::stub::ENTRY,
        lego_legacy_core::stub::program(),
    )
    .unwrap();
    let start = Cpu {
        eip: lego_legacy_core::stub::ENTRY,
        ..Cpu::default()
    };

    let mut engine = Interp::new(Mode::FLAT);
    let mut cpu = start;
    let (retired, stopped) = engine.run(&mut cpu, &mut mem, 100, 0..0);
    // The run stops at the stub's first software interrupt, which retires:
    // two setup instructions, its loop's three instructions three times over,
    // the write, and the interrupt.
    assert_eq!(
        retired, 13,
        "thirteen instructions retired to the first stop"
    );
    assert!(matches!(stopped, Err(Stop::Interrupt(_))));
    // Decoded as it went, in runs to each transfer of control: the setup
    // into the loop, the loop from its top, and what follows it.
    let decoded = engine.cached();
    assert_eq!(decoded, 10);

    // Run again from the start: nothing more is decoded.
    let mut cpu = start;
    let _ = engine.run(&mut cpu, &mut mem, 100, 0..0);
    assert_eq!(engine.cached(), decoded, "the second time decodes nothing");

    engine.forget();
    assert_eq!(engine.cached(), 0);
}

#[test]
fn a_run_retires_exactly_what_as_many_steps_would() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    mem.write(
        lego_legacy_core::stub::ENTRY,
        lego_legacy_core::stub::program(),
    )
    .unwrap();
    let start = Cpu {
        eip: lego_legacy_core::stub::ENTRY,
        ..Cpu::default()
    };
    // Every budget from nothing to past the stub's first stop, which is an
    // interrupt and retires: the run ends where that many steps end, mid-run
    // or not.
    for budget in 0..16 {
        let (mut stepped, mut ran) = (start, start);
        let mut by_step = Interp::new(Mode::FLAT);
        let (mut steps, mut step_stop) = (0, Ok(()));
        while steps < budget {
            match by_step.step(&mut stepped, &mut mem) {
                Ok(()) => steps += 1,
                Err(stop @ Stop::Interrupt(_)) => {
                    steps += 1;
                    step_stop = Err(stop);
                    break;
                }
                Err(stop) => {
                    step_stop = Err(stop);
                    break;
                }
            }
        }
        let (retired, run_stop) = Interp::new(Mode::FLAT).run(&mut ran, &mut mem, budget, 0..0);
        assert_eq!(
            (retired, run_stop, ran.eip, ran.r),
            (steps, step_stop, stepped.eip, stepped.r),
            "a budget of {budget}"
        );
    }
    // A run stops before an address it is told is outside, straight-line code
    // or not: here the third instruction's.
    let mut third = start;
    let mut engine = Interp::new(Mode::FLAT);
    for _ in 0..2 {
        engine.step(&mut third, &mut mem).unwrap();
    }
    let mut ran = start;
    let (retired, stopped) =
        Interp::new(Mode::FLAT).run(&mut ran, &mut mem, 100, third.eip..third.eip + 1);
    assert_eq!((retired, stopped, ran.eip), (2, Ok(()), third.eip));
}

#[test]
fn a_repeated_scan_stops_on_the_byte_it_was_looking_for() {
    // The string-replacing idiom a title uses to swap a file's extension:
    // scan for the terminator with `repne scasb`, step back onto it, and write
    // from there. Getting `edi` wrong by one lands the new extension in the
    // middle of the name.
    use lego_legacy_core::cpu::{ECX, EDI};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let at = 0x0040_0000;
    mem.write(at, &[0xf2, 0xae]).unwrap(); // repne scasb
    let text = b"data\\car02.tid\0";
    let buffer = 0x0050_0000;
    mem.write(buffer, text).unwrap();

    let mut regs = Cpu {
        eip: at,
        ..Cpu::default()
    };
    regs.r[EDI] = buffer;
    regs.r[ECX] = u32::MAX;
    regs.r[EAX] = 0;
    let mut cpu = Interp::new(Mode::FLAT);
    cpu.step(&mut regs, &mut mem).expect("the scan runs");

    let nul = buffer + u32::try_from(text.len()).unwrap() - 1;
    assert_eq!(
        regs.r[EDI],
        nul + 1,
        "a scan stops one past the byte it matched"
    );
    assert_eq!(
        regs.r[ECX],
        u32::MAX - u32::try_from(text.len()).unwrap(),
        "and counts down once for every byte it looked at"
    );
}

/// What each declared processor says it is: the part's own answer, with a
/// feature word naming only what that part has and this emulator can actually
/// run (#56, #104).
#[test]
fn cpuid_answers_as_the_declared_processor_and_nothing_it_cannot_run() {
    use lego_legacy_core::cpu::{Processor, EBX, ECX, EDX};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    mem.write(0x0040_0000, &[0x0f, 0xa2]).unwrap();
    let mut ask = |processor, leaf| {
        let mut cpu = Interp::new(Mode {
            processor,
            ..Mode::FLAT
        });
        let mut regs = Cpu {
            eip: 0x0040_0000,
            r: [leaf, 0, 0, 0, 0, 0, 0, 0],
            ..Cpu::default()
        };
        cpu.step(&mut regs, &mut mem).expect("cpuid is decoded");
        [regs.r[EAX], regs.r[EBX], regs.r[ECX], regs.r[EDX]]
    };
    let vendor = |[_, b, c, d]: [u32; 4]| -> Vec<u8> {
        [b, d, c].iter().flat_map(|w| w.to_le_bytes()).collect()
    };
    let (fpu, tsc, cmov, mmx) = (1, 1 << 4, 1 << 15, 1 << 23);

    // A Pentium 166, the P54C: one leaf past the first, family 5, and no
    // conditional moves.
    let first = ask(Processor::Pentium, 0);
    assert_eq!(first[0], 1, "one leaf past the first");
    assert_eq!(vendor(first), b"GenuineIntel");
    assert_eq!(
        ask(Processor::Pentium, 1),
        [0x052c, 0, 0, fpu | tsc],
        "family 5, model 2, stepping C, with floating point and a cycle counter"
    );
    assert_eq!(ask(Processor::Pentium, 2), [0; 4], "and nothing past it");

    // A Pentium II 450, Deschutes: two leaves past the first, the second its
    // caches, and conditional moves but no MMX, which this emulator does not
    // decode.
    let first = ask(Processor::PentiumII, 0);
    assert_eq!(first[0], 2, "two leaves past the first");
    assert_eq!(vendor(first), b"GenuineIntel");
    let [signature, _, _, features] = ask(Processor::PentiumII, 1);
    assert_eq!(signature, 0x0652, "family 6, model 5, stepping 2");
    assert_eq!(features, fpu | tsc | cmov);
    assert_eq!(features & mmx, 0);
    assert_eq!(
        ask(Processor::PentiumII, 2),
        [0x0302_0101, 0, 0, 0x0c04_0843],
        "its TLBs, 16 KB caches and 512 KB second level"
    );
    assert_eq!(
        ask(Processor::PentiumII, 0x8000_0000),
        [0; 4],
        "and no extended leaves"
    );
}

/// A title finds out whether it may ask `cpuid` by toggling the ID flag, and a
/// Pentium lets it, and alignment check with it. An Intel386 lets neither, and
/// has no `cpuid` to ask (#104).
#[test]
fn the_id_flag_toggles_on_a_processor_that_has_cpuid() {
    use lego_legacy_core::cpu::{Processor, ECX, ESP};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    // pushfd; pop eax; mov ecx, eax; xor eax, 0x240000; push eax; popfd;
    // pushfd; pop eax: AP-485's own test, for both bits at once.
    mem.write(
        0x0040_0000,
        &[
            0x9c, 0x58, 0x89, 0xc1, 0x35, 0x00, 0x00, 0x24, 0x00, 0x50, 0x9d, 0x9c, 0x58,
        ],
    )
    .unwrap();
    mem.write(0x0041_0000, &[0x0f, 0xa2]).unwrap();
    for (processor, toggled) in [
        (Processor::I386, 0),
        (Processor::Pentium, 0x24_0000),
        (Processor::PentiumII, 0x24_0000),
    ] {
        let mut cpu = Interp::new(Mode {
            processor,
            ..Mode::FLAT
        });
        let mut regs = Cpu {
            eip: 0x0040_0000,
            ..Cpu::default()
        };
        regs.r[ESP] = 0x0050_0000;
        let (retired, result) = cpu.run(&mut regs, &mut mem, 8, 0..1);
        result.expect("the flags round trip");
        assert_eq!(retired, 8);
        assert_eq!(
            (regs.r[EAX] ^ regs.r[ECX]) & 0x24_0000,
            toggled,
            "{processor:?}"
        );

        regs.eip = 0x0041_0000;
        let asked = cpu.step(&mut regs, &mut mem);
        assert_eq!(
            asked.is_ok(),
            toggled != 0,
            "{processor:?} has cpuid exactly when it has the ID flag"
        );
    }
}

/// The cycle counter a title times itself against is the declared clock's,
/// carried on one to the instruction (#56).
#[test]
fn rdtsc_reads_the_declared_cycle_counter() {
    use lego_legacy_core::cpu::EDX;
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    // Two of them, a nop apart.
    mem.write(0x0040_0000, &[0x0f, 0x31, 0x90, 0x0f, 0x31])
        .unwrap();
    let mut cpu = Interp::new(Mode::FLAT);
    let mut regs = Cpu {
        eip: 0x0040_0000,
        tsc: 0x1_0000_0000,
        ..Cpu::default()
    };
    let (retired, result) = cpu.run(&mut regs, &mut mem, 3, 0..1);
    result.expect("three instructions retire");
    assert_eq!(retired, 3);
    // Started past what one register holds, so the count is in both: the
    // second reading is two instructions after the first.
    assert_eq!(regs.r[EDX], 1, "the top half of the count");
    assert_eq!(regs.r[EAX], 2, "and the bottom, two instructions on");
}

#[test]
fn mid_run_fault_and_terminal_fault_roll_back_eip_and_counts() {
    use lego_legacy_core::cpu::{EBX, ESP};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;

    // Straight-line run: `inc eax; inc eax; mov ebx, [0xffff_fff0]; inc eax`
    // Faults on the 3rd instruction (offset +2) after retiring 2 instructions.
    mem.write(
        code,
        &[0x40, 0x40, 0x8b, 0x1d, 0xf0, 0xff, 0xff, 0xff, 0x40],
    )
    .unwrap();
    let mut regs = Cpu {
        eip: code,
        tsc: 100,
        ..Cpu::default()
    };
    let mut engine = Interp::new(Mode::FLAT);
    let (retired, res) = engine.run(&mut regs, &mut mem, 10, 0..0);
    assert_eq!(retired, 2);
    assert!(matches!(res, Err(Stop::Fault(_))));
    assert_eq!(regs.eip, code + 2, "eip rolls back to faulting mov");
    assert_eq!(regs.tsc, 102);
    assert_eq!(regs.r[EAX], 2);
    assert_eq!(regs.r[EBX], 0);

    // Terminal fault: `inc eax; ret` with unmapped ESP rolls back to `ret`.
    let code2 = 0x0040_0100u32;
    mem.write(code2, &[0x40, 0xc3]).unwrap();
    let mut regs = Cpu {
        eip: code2,
        tsc: 50,
        ..Cpu::default()
    };
    regs.r[ESP] = 0xffff_fff0;
    let (retired, res) = engine.run(&mut regs, &mut mem, 10, 0..0);
    assert_eq!(retired, 1);
    assert!(matches!(res, Err(Stop::Fault(_))));
    assert_eq!(regs.eip, code2 + 1, "eip rolls back to faulting ret");
    assert_eq!(regs.tsc, 51);
    assert_eq!(regs.r[EAX], 1);
}

#[test]
fn mid_run_interrupt_and_halt_preserve_exact_retirement_and_eip() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;

    // `inc eax; inc eax; int 0x2e` -> retires 3, eip after int, tsc += 2.
    mem.write(code, &[0x40, 0x40, 0xcd, 0x2e]).unwrap();
    let mut regs = Cpu {
        eip: code,
        tsc: 10,
        ..Cpu::default()
    };
    let mut engine = Interp::new(Mode::FLAT);
    let (retired, res) = engine.run(&mut regs, &mut mem, 10, 0..0);
    assert_eq!(retired, 3);
    assert_eq!(res, Err(Stop::Interrupt(0x2e)));
    assert_eq!(regs.eip, code + 4);
    assert_eq!(regs.tsc, 12);
    assert_eq!(regs.r[EAX], 2);

    // `inc eax; hlt` -> retires 1, eip after hlt, tsc += 1.
    let code2 = 0x0040_0100u32;
    mem.write(code2, &[0x40, 0xf4]).unwrap();
    let mut regs = Cpu {
        eip: code2,
        tsc: 20,
        ..Cpu::default()
    };
    let (retired, res) = engine.run(&mut regs, &mut mem, 10, 0..0);
    assert_eq!(retired, 1);
    assert_eq!(res, Err(Stop::Halted));
    assert_eq!(regs.eip, code2 + 2);
    assert_eq!(regs.tsc, 21);
    assert_eq!(regs.r[EAX], 1);
}

#[test]
fn rdtsc_mid_run_after_fast_ops_sees_exact_tsc() {
    use lego_legacy_core::cpu::EDX;
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;

    // `inc ecx; inc ecx; inc ecx; rdtsc; inc ecx; hlt`
    mem.write(code, &[0x41, 0x41, 0x41, 0x0f, 0x31, 0x41, 0xf4])
        .unwrap();
    let mut regs = Cpu {
        eip: code,
        tsc: 0x2_0000_0010,
        ..Cpu::default()
    };
    let (retired, res) = Interp::new(Mode::FLAT).run(&mut regs, &mut mem, 10, 0..0);
    assert_eq!(retired, 5);
    assert_eq!(res, Err(Stop::Halted));
    assert_eq!(regs.r[EDX], 2);
    assert_eq!(
        regs.r[EAX], 0x13,
        "rdtsc observes the three preceding fast ops"
    );
    assert_eq!(regs.tsc, 0x2_0000_0015);
}

#[test]
fn self_modifying_code_invalidates_cached_fast_runs() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;
    let target = 0x0040_2000u32;

    // Target initially: `mov eax, 1; ret`
    mem.write(target, &[0xb8, 1, 0, 0, 0, 0xc3]).unwrap();
    // Caller: `call target; mov dword ptr [target+1], 42; call target; hlt`
    let rel1 = target.wrapping_sub(code + 5).to_le_bytes();
    let imm_addr = (target + 1).to_le_bytes();
    let val42 = 42u32.to_le_bytes();
    let rel2 = target.wrapping_sub(code + 20).to_le_bytes();
    let mut prog = Vec::new();
    prog.push(0xe8);
    prog.extend_from_slice(&rel1);
    prog.extend_from_slice(&[0xc7, 0x05]);
    prog.extend_from_slice(&imm_addr);
    prog.extend_from_slice(&val42);
    prog.push(0xe8);
    prog.extend_from_slice(&rel2);
    prog.push(0xf4);
    mem.write(code, &prog).unwrap();

    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[lego_legacy_core::cpu::ESP] = 0x0050_0000;
    let mut engine = Interp::new(Mode::FLAT);
    let (_, res) = engine.run(&mut regs, &mut mem, 100, 0..0);
    assert_eq!(res, Err(Stop::Halted));
    assert_eq!(
        regs.r[EAX], 42,
        "rewritten target page was invalidated and re-decoded"
    );
}

#[test]
fn smc_replaces_chained_jump_and_jcc_targets_with_different_instruction_lengths() {
    use lego_legacy_core::cpu::{ECX, ESP};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;
    let target = 0x0040_2000u32;

    // Target initially: `add eax, 1; ret` (4 bytes)
    mem.write(target, &[0x83, 0xc0, 0x01, 0xc3]).unwrap();

    // Caller loops 4 times (ecx = 4..1): calls `target`, on ecx == 2 rewrites
    // `target` to a 6-byte `mov eax, 0x100; ret` (changing both length & bytes).
    let mut prog = Vec::new();
    let rel_call = target.wrapping_sub(code + 5).to_le_bytes();
    prog.push(0xe8);
    prog.extend_from_slice(&rel_call); // call target
    prog.extend_from_slice(&[0x83, 0xf9, 0x02]); // cmp ecx, 2
    prog.extend_from_slice(&[0x75, 20]); // jnz skip (2 x 10-byte mov dword [imm32], imm32)
    prog.extend_from_slice(&[0xc7, 0x05]);
    prog.extend_from_slice(&target.to_le_bytes());
    prog.extend_from_slice(&[0xb8, 0x00, 0x01, 0x00]);
    prog.extend_from_slice(&[0xc7, 0x05]);
    prog.extend_from_slice(&(target + 4).to_le_bytes());
    prog.extend_from_slice(&[0x00, 0xc3, 0x90, 0x90]);
    // skip: dec ecx; jnz loop_top; hlt
    prog.push(0x49);
    let back = -(prog.len() as i8 + 2);
    prog.extend_from_slice(&[0x75, back as u8]);
    prog.push(0xf4);
    mem.write(code, &prog).unwrap();

    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ECX] = 4;
    regs.r[ESP] = 0x0050_0000;
    let mut engine = Interp::new(Mode::FLAT);
    let (_, res) = engine.run(&mut regs, &mut mem, 1_000, 0..0);
    assert_eq!(res, Err(Stop::Halted));
    assert_eq!(
        regs.r[EAX], 0x100,
        "chained target was re-decoded to the new longer instruction sequence"
    );
}

#[test]
fn custom_outside_range_clamps_both_before_and_after_block_chaining() {
    use lego_legacy_core::cpu::ESP;
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;
    let target = 0x0041_0000u32;

    // `call target; hlt` -> target: `inc eax; inc eax; inc eax; ret`
    let mut prog = vec![0xe8];
    prog.extend_from_slice(&target.wrapping_sub(code + 5).to_le_bytes());
    prog.push(0xf4);
    mem.write(code, &prog).unwrap();
    mem.write(target, &[0x40, 0x40, 0x40, 0xc3]).unwrap();

    let mut engine = Interp::new(Mode::FLAT);
    // Warm up with empty `outside` so `code -> target` chains directly.
    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ESP] = 0x0050_0000;
    let (retired, res) = engine.run(&mut regs, &mut mem, 100, 0..0);
    assert_eq!((retired, res, regs.r[EAX]), (5, Err(Stop::Halted), 3));

    // Re-run with custom `outside` mid-block inside `target` (after 2nd `inc eax`).
    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ESP] = 0x0050_0000;
    let (retired, res) = engine.run(&mut regs, &mut mem, 100, (target + 2)..(target + 3));
    assert_eq!(
        (retired, res, regs.eip, regs.r[EAX]),
        (3, Ok(()), target + 2, 2)
    );
}

#[test]
fn inline_x87_dot_product_and_ftol_match_exact_ieee754_and_preserve_cold_ops() {
    use lego_legacy_core::cpu::{ECX, EDX, ESI};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;
    let data = 0x0050_0000u32;

    // Loop computing sum += a[i] * b[i] (f32 -> f64), sqrt, compare with threshold,
    // convert via MSVC _ftol (fnstcw / fldcw RC=3 / fistp qword / fldcw), then cold `fsin` + `hlt`.
    let mut prog = Vec::new();
    prog.extend_from_slice(&[0xd9, 0xee]); // fldz (accumulator)
    let loop_top = prog.len();
    prog.extend_from_slice(&[0xd9, 0x06]); // fld dword [esi]
    prog.extend_from_slice(&[0xd8, 0x4e, 0x04]); // fmul dword [esi+4]
    prog.extend_from_slice(&[0xde, 0xc1]); // faddp st(1)
    prog.extend_from_slice(&[0x83, 0xc6, 0x08]); // add esi, 8
    prog.push(0x49); // dec ecx
    let rel = (loop_top as i32) - (prog.len() as i32 + 2);
    prog.extend_from_slice(&[0x75, rel as i8 as u8]); // jnz loop_top
    prog.extend_from_slice(&[0xd9, 0xfa]); // fsqrt
    prog.extend_from_slice(&[0xd9, 0x16]); // fst dword [esi]
    prog.extend_from_slice(&[0xd9, 0x7e, 0x04]); // fnstcw [esi+4]
    prog.extend_from_slice(&[0x66, 0x8b, 0x46, 0x04]); // mov ax, [esi+4]
    prog.extend_from_slice(&[0x80, 0xcc, 0x0c]); // or ah, 0x0c
    prog.extend_from_slice(&[0x66, 0x89, 0x46, 0x06]); // mov [esi+6], ax
    prog.extend_from_slice(&[0xd9, 0x6e, 0x06]); // fldcw [esi+6]
    prog.extend_from_slice(&[0xd9, 0xc0]); // fld st(0)
    prog.extend_from_slice(&[0xdf, 0x7e, 0x08]); // fistp qword [esi+8]
    prog.extend_from_slice(&[0xd9, 0x6e, 0x04]); // fldcw [esi+4]
    prog.extend_from_slice(&[0x8b, 0x46, 0x08]); // mov eax, [esi+8]
    prog.extend_from_slice(&[0x8b, 0x56, 0x0c]); // mov edx, [esi+12]
    prog.extend_from_slice(&[0xd9, 0xfe]); // fsin (cold op -> STATUS_FALLBACK!)
    prog.push(0xf4); // hlt

    mem.write(code, &prog).unwrap();
    // 4 pairs: (3,3) + (4,4) + (12,12) + (0,0) = 9 + 16 + 144 = 169 -> sqrt = 13.0
    let pairs: [(f32, f32); 4] = [(3.0, 3.0), (4.0, 4.0), (12.0, 12.0), (0.0, 0.0)];
    for (k, (a, b)) in pairs.iter().enumerate() {
        let off = data + (k as u32) * 8;
        mem.write(off, &a.to_le_bytes()).unwrap();
        mem.write(off + 4, &b.to_le_bytes()).unwrap();
    }

    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ESI] = data;
    regs.r[ECX] = 4;

    let mut engine = Interp::new(Mode::FLAT);
    let (_, res) = engine.run(&mut regs, &mut mem, 10_000, 0..0);
    assert_eq!(res, Err(Stop::Halted));
    let out_at = data + 32;
    assert_eq!(mem.read_u32(out_at).unwrap(), 13.0f32.to_bits());
    assert_eq!(regs.r[EAX], 13);
    assert_eq!(regs.r[EDX], 0);
    assert_eq!(
        regs.fpu.st(0).to_bits(),
        lego_legacy_core::cpu::x87::maths::sin(13.0).to_bits()
    );
}

#[test]
fn promoted_integer_ops_leave_div_idiv_fault_and_cmovcc_preserve_exact_state() {
    use lego_legacy_core::cpu::{EBP, EBX, EDX, ESP};
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;
    let callee = 0x0040_0100u32;
    let stack = 0x0050_0000u32;

    // Caller: `call callee; cmp eax, 10; cmovge ebx, eax; mov edx, 0xffffffff; mov eax, 0x80000000; mov ecx, 0xffffffff; idiv ecx; hlt`
    let mut caller = Vec::new();
    caller.push(0xe8);
    caller.extend_from_slice(&callee.wrapping_sub(code + 5).to_le_bytes()); // call callee
    caller.extend_from_slice(&[0x83, 0xf8, 0x0a]); // cmp eax, 10
    caller.extend_from_slice(&[0x0f, 0x4d, 0xd8]); // cmovge ebx, eax
    caller.extend_from_slice(&[0xba, 0xff, 0xff, 0xff, 0xff]); // mov edx, -1
    caller.extend_from_slice(&[0xb8, 0x00, 0x00, 0x00, 0x80]); // mov eax, i32::MIN
    caller.extend_from_slice(&[0xb9, 0xff, 0xff, 0xff, 0xff]); // mov ecx, -1
    let idiv_off = caller.len() as u32;
    caller.extend_from_slice(&[0xf7, 0xf9]); // idiv ecx -> #DE (must NOT mutate eax/edx!)
    caller.push(0xf4); // hlt

    // Callee: `push ebp; mov ebp, esp; mov eax, 0xffff8005; cwde; cdq; neg eax; mov eax, 42; leave; ret`
    let sub: &[u8] = &[
        0x55, // push ebp
        0x89, 0xe5, // mov ebp, esp
        0xb8, 0x05, 0x80, 0xff, 0xff, // mov eax, 0xffff8005
        0x98, // cwde -> 0xffff8005
        0x99, // cdq -> edx = -1
        0xf7, 0xd8, // neg eax -> 0x00007ffb
        0xb8, 0x2a, 0x00, 0x00, 0x00, // mov eax, 42
        0xc9, // leave
        0xc3, // ret
    ];

    mem.write(code, &caller).unwrap();
    mem.write(callee, sub).unwrap();

    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ESP] = stack;
    regs.r[EBP] = 0x1234_5678;
    regs.r[EBX] = 99;

    let mut engine = Interp::new(Mode::FLAT);
    let (_, res) = engine.run(&mut regs, &mut mem, 1_000, 0..0);
    assert_eq!(res, Err(Stop::DivideError));
    assert_eq!(regs.eip, code + idiv_off, "eip rolls back to faulting idiv");
    assert_eq!(regs.r[EBP], 0x1234_5678, "leave restored caller ebp");
    assert_eq!(regs.r[ESP], stack, "leave+ret restored caller esp");
    assert_eq!(regs.r[EBX], 42, "cmovge copied callee result 42");
    assert_eq!(regs.r[EAX], 0x8000_0000, "idiv #DE did not mutate eax");
    assert_eq!(regs.r[EDX], 0xffff_ffff, "idiv #DE did not mutate edx");
}

#[test]
fn smc_straddling_code_bbox_lower_edge_invalidates_cached_run() {
    use lego_legacy_core::cpu::ESP;
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    // Place `target` at a lower page than `caller` so `target == code_min`.
    let target = 0x0040_0000u32;
    let caller = 0x0041_0000u32;

    // Target initially: `nop; ret` (0x90, 0xc3).
    mem.write(target, &[0x90, 0xc3]).unwrap();
    // Caller: `call target; mov dword [target - 3], 0x40000000; call target; hlt`
    // The 4-byte store starts 3 bytes BELOW `code_min` and overwrites `target[0]`
    // from `0x90` (`nop`) to `0x40` (`inc eax`).
    let mut prog = Vec::new();
    prog.push(0xe8);
    prog.extend_from_slice(&target.wrapping_sub(caller + 5).to_le_bytes());
    prog.extend_from_slice(&[0xc7, 0x05]);
    prog.extend_from_slice(&(target - 3).to_le_bytes());
    prog.extend_from_slice(&0x4000_0000u32.to_le_bytes());
    prog.push(0xe8);
    prog.extend_from_slice(&target.wrapping_sub(caller + 20).to_le_bytes());
    prog.push(0xf4);
    mem.write(caller, &prog).unwrap();

    let mut regs = Cpu {
        eip: caller,
        ..Cpu::default()
    };
    regs.r[ESP] = 0x0050_0000;
    let mut engine = Interp::new(Mode::FLAT);
    let (_, res) = engine.run(&mut regs, &mut mem, 100, 0..0);
    assert_eq!(res, Err(Stop::Halted));
    assert_eq!(
        regs.r[EAX], 1,
        "4-byte store straddling code_min - 3 invalidated and re-decoded target"
    );
}

#[test]
fn step_and_mid_run_stops_observe_exact_flags_on_nf_promoted_runs() {
    use lego_legacy_core::cpu::EBX;
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;

    // Straight-line run whose leading ALU/shift ops are promoted to `*Nf`:
    //   add eax, ebx; sub eax, 1; xor eax, 0xff; shl eax, 1; cmp eax, ebx; hlt
    let prog: &[u8] = &[
        0x01, 0xd8, // add eax, ebx
        0x83, 0xe8, 0x01, // sub eax, 1
        0x83, 0xf0, 0x7f, // xor eax, 0x7f
        0xc1, 0xe0, 0x01, // shl eax, 1
        0x39, 0xd8, // cmp eax, ebx
        0xf4, // hlt
    ];
    mem.write(code, prog).unwrap();

    let mut start = Cpu {
        eip: code,
        ..Cpu::default()
    };
    start.r[EAX] = 0x7fff_ffff;
    start.r[EBX] = 3;

    // Warm the decode cache once so `*Nf` promotion is already in place for `step()`.
    let mut stepped_engine = Interp::new(Mode::FLAT);
    for budget in 0..=6 {
        let mut stepped = start;
        let mut steps = 0u64;
        let mut step_res = Ok(());
        while steps < budget {
            match stepped_engine.step(&mut stepped, &mut mem) {
                Ok(()) => steps += 1,
                Err(stop) => {
                    step_res = Err(stop);
                    break;
                }
            }
        }
        let mut ran = start;
        let mut run_engine = Interp::new(Mode::FLAT);
        let (retired, run_res) = run_engine.run(&mut ran, &mut mem, budget, 0..0);
        assert_eq!(
            (retired, run_res, ran.eip, ran.r, ran.flags.to_eflags()),
            (
                steps,
                step_res,
                stepped.eip,
                stepped.r,
                stepped.flags.to_eflags()
            ),
            "budget={budget} must match step() bit-for-bit including intermediate flags"
        );
    }
}

#[test]
#[ignore]
fn bench_flat32_tight_loop() {
    use lego_legacy_core::cpu::{EBX, ECX, ESI, ESP};
    use std::time::Instant;
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    let code = 0x0040_0000u32;
    let data = 0x0050_0000u32;
    // Tight flat32 loop (8 insns per iteration):
    //   mov edx, [esi]          ; 8b 16
    //   add eax, edx            ; 01 d0
    //   xor ebx, eax            ; 31 c3
    //   add edx, 1              ; 83 c2 01
    //   mov [esi], edx          ; 89 16
    //   inc eax                 ; 40
    //   dec ecx                 ; 49
    //   jne -13 (back to start) ; 75 f3
    //   hlt                     ; f4
    let prog: &[u8] = &[
        0x8b, 0x16, 0x01, 0xd0, 0x31, 0xc3, 0x83, 0xc2, 0x01, 0x89, 0x16, 0x40, 0x49, 0x75, 0xf1,
        0xf4,
    ];
    mem.write(code, prog).unwrap();
    let mut engine = Interp::new(Mode::FLAT);
    // Warm up decode cache
    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ECX] = 100;
    regs.r[ESI] = data;
    regs.r[ESP] = 0x0060_0000;
    let _ = engine.run(&mut regs, &mut mem, 1_000_000, 0..0);

    // Timed run: 25M iterations = 200M guest instructions
    let mut regs = Cpu {
        eip: code,
        ..Cpu::default()
    };
    regs.r[ECX] = 25_000_000;
    regs.r[ESI] = data;
    regs.r[ESP] = 0x0060_0000;
    let t0 = Instant::now();
    let (retired, res) = engine.run(&mut regs, &mut mem, 300_000_000, 0..0);
    let dt = t0.elapsed().as_secs_f64();
    assert_eq!(res, Err(Stop::Halted));
    assert_eq!(retired, 200_000_000);
    eprintln!(
        "tier={:?} retired={retired} wall={dt:.4}s mips={:.1} eax={:#x} ebx={:#x}",
        lego_legacy_core::arch::Tier::active(),
        retired as f64 / dt / 1e6,
        regs.r[EAX],
        regs.r[EBX],
    );
}
