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
fn an_address_is_decoded_once_however_often_it_runs() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).unwrap();
    mem.write(
        lego_legacy_core::stub::ENTRY,
        lego_legacy_core::stub::program(),
    )
    .unwrap();

    let mut cpu = Cpu {
        eip: lego_legacy_core::stub::ENTRY,
        ..Cpu::default()
    };
    let mut engine = Interp::new(Mode::FLAT);
    let mut retired = 0;
    while engine.step(&mut cpu, &mut mem).is_ok() {
        retired += 1;
        assert!(retired < 100, "the stub guest does not run forever");
    }

    // The run stops at the stub's first software interrupt. Its loop ran its
    // three instructions three times over, and every address on the way was
    // decoded exactly once: two setup instructions, three in the loop, the
    // write, and the interrupt itself.
    assert_eq!(
        retired, 12,
        "twelve instructions retired before the first stop"
    );
    assert_eq!(engine.cached(), 7, "seven distinct addresses decoded");

    engine.forget();
    assert_eq!(engine.cached(), 0);
}
