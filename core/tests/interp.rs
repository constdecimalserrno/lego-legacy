//! The interpreter's own corners, where a run through the seam cannot reach.

use lego_legacy_core::cpu::{interp::Interp, CpuEngine, Regs, Stop, EAX};
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
    let mut regs = Regs {
        eip: last,
        ..Regs::default()
    };
    let mut cpu = Interp::new();
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
    mem.write(0x0040_0000, &[0x0f, 0x0b]).unwrap();

    let mut regs = Regs {
        eip: 0x0040_0000,
        ..Regs::default()
    };
    let stop = Interp::new().step(&mut regs, &mut mem).unwrap_err();

    assert_eq!(
        stop,
        Stop::Undecoded {
            eip: 0x0040_0000,
            byte: 0x0f
        }
    );
    assert_eq!(
        regs.eip, 0x0040_0000,
        "a wall leaves the cursor where the guest stood"
    );
    assert!(stop.to_string().contains("undecoded opcode 0x0f"));
}
