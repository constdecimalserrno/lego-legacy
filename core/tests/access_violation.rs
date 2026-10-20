//! The bottom of the address space, as on Windows 9x (#132): a read or a
//! write in the first 4 KiB is an access violation, offered to the guest's
//! own handler chain, and a wall if nothing on it takes it. Code run from
//! there, and the MS-DOS memory above it, are walls: Windows 9x lets a Win32
//! process reach both, and this machine does not have what is there.
//! `RaiseException` offers its exceptions through the same dispatch, with
//! the same context (#133).
//!
//! Each guest is a few hand-assembled instructions, as the stub's are, given
//! the thread block a title gets, and run by the loop every title runs on.

use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::{ESP, FS};
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::{
    write_tib, Machine, Outcome, STACK_BYTES, STACK_TOP, THREAD_ID, TIB,
};
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::Title;

const STUB: Title = Title::STUB;

/// Where the code goes, the handler after it, and where both keep what they
/// saw, a word apiece.
const CODE: u32 = 0x0040_0000;
const HANDLER: u32 = 0x0040_1000;
const DATA: u32 = 0x0050_0000;

const EXCEPTION_ACCESS_VIOLATION: u32 = 0xc000_0005;

const fn word(n: u32) -> u32 {
    DATA + n * 4
}

fn le(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

/// A machine with the stub's declared values and a title's thread block, so
/// that `fs:[0]` is a handler chain.
fn machine(host: &mut Headless) -> Machine {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    let mut machine = Machine::new(&STUB, host).expect("a machine");
    write_tib(
        &mut machine.arena,
        TIB,
        STACK_TOP,
        STACK_TOP - STACK_BYTES,
        THREAD_ID,
    )
    .expect("in the arena");
    machine.cpu.seg_base[FS] = TIB;
    machine
}

/// `push handler; push fs:[0]; mov fs:[0], esp`: an exception frame, as
/// every MSVC function that has one makes it.
fn frame(handler: u32) -> Vec<u8> {
    let mut code = vec![0x68];
    code.extend(le(handler));
    code.extend([0x64, 0xff, 0x35, 0, 0, 0, 0, 0x64, 0x89, 0x25, 0, 0, 0, 0]);
    code
}

/// `push 0; call ExitProcess`.
fn exit(machine: &mut Machine) -> Vec<u8> {
    let to = machine.boundary.bind_named("KERNEL32.dll", "ExitProcess");
    let mut code = vec![0x6a, 0x00, 0xb8];
    code.extend(le(to));
    code.extend([0xff, 0xd0]);
    code
}

fn run(
    machine: &mut Machine,
    host: &mut Headless,
    code: &[(u32, Vec<u8>)],
    trace: Trace,
) -> Outcome {
    for (at, bytes) in code {
        machine.arena.write(*at, bytes).expect("the code fits");
    }
    machine.cpu.eip = CODE;
    machine.cpu.r[ESP] = STACK_TOP - 16;
    let mut log = RunLog::create("stub", trace).expect("a run directory");
    machine.run(host, &mut log, 1_000_000).outcome
}

/// A handler that writes down what it was handed, mends the context and
/// carries on: `eax` and `eip` changed, everything else as it was.
#[test]
fn a_read_and_a_write_below_4_kib_are_access_violations_a_handler_can_take() {
    // Each faulting instruction, and what the record should say: a write or
    // not, and which address.
    let cases: [(&[u8], u32, u32); 2] = [
        (&[0xa1, 0x08, 0, 0, 0], 0, 0x0008), // mov eax, [8]: a null pointer's field
        (&[0x89, 0x1d, 0xfc, 0x0f, 0, 0], 1, 0x0ffc), // mov [0xffc], ebx: the last word
    ];
    for (faulting, write, addr) in cases {
        // Both ways the loop runs the guest: in bursts, and one at a time.
        for trace in [Trace::Off, Trace::Insns] {
            let mut host = Headless::new();
            let mut machine = machine(&mut host);
            let mut code = frame(HANDLER);
            code.push(0xbb); // mov ebx, 0x1234: given back from the context
            code.extend(le(0x1234));
            let at = CODE + u32::try_from(code.len()).expect("small");
            code.extend(faulting);
            let resume = CODE + u32::try_from(code.len()).expect("small");
            code.push(0xa3); // mov [word 0], eax
            code.extend(le(word(0)));
            code.extend([0x89, 0x1d]); // mov [word 1], ebx
            code.extend(le(word(1)));
            code.extend([0x89, 0x25]); // mov [word 2], esp
            code.extend(le(word(2)));
            code.extend(exit(&mut machine));

            let mut handler = Vec::new();
            for (field, to) in [(0x00, 3), (0x0c, 4), (0x10, 5), (0x14, 6), (0x18, 7)] {
                handler.extend([0x8b, 0x44, 0x24, 0x04]); // mov eax, [esp+4]: the record
                handler.extend([0x8b, 0x48, field]); // mov ecx, [eax+field]
                handler.extend([0x89, 0x0d]); // mov [word to], ecx
                handler.extend(le(word(to)));
            }
            handler.extend([0x8b, 0x44, 0x24, 0x0c]); // mov eax, [esp+12]: the context
            handler.extend([0xc7, 0x80, 0xb8, 0, 0, 0]); // mov [eax+Eip], resume
            handler.extend(le(resume));
            handler.extend([0xc7, 0x80, 0xb0, 0, 0, 0]); // mov [eax+Eax], 0x77
            handler.extend(le(0x77));
            handler.extend([0x31, 0xc0, 0xc3]); // xor eax, eax: ExceptionContinueExecution

            let outcome = run(
                &mut machine,
                &mut host,
                &[(CODE, code), (HANDLER, handler)],
                trace,
            );
            assert_eq!(
                outcome,
                Outcome::Exited { code: 0 },
                "{faulting:02x?} {trace:?}"
            );
            let read = |n| machine.arena.read_u32(word(n)).expect("in the arena");
            assert_eq!(read(3), EXCEPTION_ACCESS_VIOLATION, "{faulting:02x?}");
            assert_eq!(read(4), at, "the record says where it faulted");
            assert_eq!(read(5), 2, "with two parameters");
            assert_eq!(read(6), write, "a write or not");
            assert_eq!(read(7), addr, "and which address");
            assert_eq!(read(0), 0x77, "eax as the handler left it in the context");
            assert_eq!(read(1), 0x1234, "ebx as it was when it faulted");
            assert_eq!(
                read(2),
                STACK_TOP - 24,
                "the stack as it was, the frame on it"
            );
        }
    }
}

/// `RaiseException`'s context has the flags and the stack where `winnt.h`'s
/// i386 `CONTEXT` has them: `EFlags` at 0xc0, `Esp` at 0xc4 (#133). The
/// handler writes both down and carries on, and the call returns.
#[test]
fn raise_exception_s_context_has_eflags_and_esp_where_winnt_h_puts_them() {
    let mut host = Headless::new();
    let mut machine = machine(&mut host);
    let raise = machine
        .boundary
        .bind_named("KERNEL32.dll", "RaiseException");
    let mut code = frame(HANDLER);
    code.push(0xf9); // stc: a flag that shows
    code.extend([0x9c, 0x8f, 0x05]); // pushfd; pop [word 0]: the flags as raised
    code.extend(le(word(0)));
    // RaiseException(0xe0000001, 0, 0, null): nothing below changes a flag.
    code.extend([0x6a, 0x00, 0x6a, 0x00, 0x6a, 0x00, 0x68]);
    code.extend(le(0xe000_0001));
    code.push(0xb8);
    code.extend(le(raise));
    code.extend([0xff, 0xd0]); // call eax
    code.extend(exit(&mut machine));

    let mut handler = vec![0x8b, 0x44, 0x24, 0x0c]; // mov eax, [esp+12]: the context
    for (field, to) in [(0xc0, 1), (0xc4, 2)] {
        handler.extend([0x8b, 0x88]); // mov ecx, [eax+field]
        handler.extend(le(field));
        handler.extend([0x89, 0x0d]); // mov [word to], ecx
        handler.extend(le(word(to)));
    }
    handler.extend([0x31, 0xc0, 0xc3]); // xor eax, eax: ExceptionContinueExecution

    let outcome = run(
        &mut machine,
        &mut host,
        &[(CODE, code), (HANDLER, handler)],
        Trace::Off,
    );
    assert_eq!(outcome, Outcome::Exited { code: 0 });
    let read = |n| machine.arena.read_u32(word(n)).expect("in the arena");
    assert_eq!(read(0) & 1, 1, "the carry is set");
    assert_eq!(read(1), read(0), "EFlags as they were when it was raised");
    assert_eq!(
        read(2),
        STACK_TOP - 44,
        "Esp at the call's return address, its four arguments and the frame above it"
    );
}

/// A frame whose handler says to keep looking, and nothing after it: the run
/// stops at a wall that says what faulted, where and on what.
#[test]
fn an_access_violation_nothing_takes_is_a_wall_naming_where_and_what() {
    let mut host = Headless::new();
    let mut machine = machine(&mut host);
    let mut code = frame(HANDLER);
    let at = CODE + u32::try_from(code.len()).expect("small");
    code.extend([0xa1, 0x08, 0, 0, 0]); // mov eax, [8]
    code.extend(exit(&mut machine));
    let handler = vec![0xb8, 1, 0, 0, 0, 0xc3]; // ExceptionContinueSearch

    let outcome = run(
        &mut machine,
        &mut host,
        &[(CODE, code), (HANDLER, handler)],
        Trace::Off,
    );
    let Outcome::Wall { what } = outcome else {
        panic!("ran on past a fault nothing took: {outcome}");
    };
    assert!(
        what.starts_with(&format!(
            "an access violation reading 0x00000008 at {at:#010x}: nothing on the guest's own \
             handler chain took it"
        )),
        "{what}"
    );
}

/// A call through a null pointer, and a read of the MS-DOS memory above the
/// first 4 KiB: Windows 9x runs and reads what is there, which this machine
/// does not have, so each is a wall that says so, and no handler is asked.
#[test]
fn code_run_from_the_bottom_and_ms_dos_memory_are_walls() {
    let cases: [(&[u8], &str); 2] = [
        (
            &[0xbe, 0x20, 0, 0, 0, 0xff, 0xe6], // mov esi, 0x20; jmp esi
            "the guest ran into 0x00000020, which on Windows 9x is MS-DOS's own memory",
        ),
        (
            &[0xa1, 0x00, 0x10, 0, 0], // mov eax, [0x1000]
            "guest address 0x00001000+4 is in the MS-DOS memory under the first megabyte, \
             which this machine does not have, at 0x00400013",
        ),
    ];
    for (faulting, wall) in cases {
        let mut host = Headless::new();
        let mut machine = machine(&mut host);
        let mut code = frame(HANDLER);
        code.extend(faulting);
        code.extend(exit(&mut machine));
        let handler = vec![0xcc]; // int3: no handler is run
        let outcome = run(
            &mut machine,
            &mut host,
            &[(CODE, code), (HANDLER, handler)],
            Trace::Off,
        );
        assert_eq!(
            outcome,
            Outcome::Wall {
                what: wall.to_owned()
            }
        );
    }
}
