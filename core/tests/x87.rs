//! The floating-point unit, one instruction at a time.
//!
//! Every case here is hand-written: known bytes in, known bits out. There is
//! no corpus for the 387, so the vectors are the arithmetic itself and the
//! identities it has to satisfy.

use lego_legacy_core::cpu::{interp::Interp, Cpu, CpuEngine, Mode, Stop, EAX};
use lego_legacy_core::guest::{Arena, GuestMem};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::ARENA_BYTES;

const CODE: u32 = 0x0040_0000;
const DATA: u32 = 0x0050_0000;

/// Run a hand-assembled sequence and hand back the machine it left.
fn run(bytes: &[u8], setup: impl FnOnce(&mut Arena)) -> (Cpu, Arena) {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).expect("an arena");
    mem.write(CODE, bytes).expect("the code fits");
    setup(&mut mem);

    let mut cpu = Cpu {
        eip: CODE,
        ..Cpu::default()
    };
    let mut engine = Interp::new(Mode::FLAT);
    let end = CODE + bytes.len() as u32;
    let mut steps = 0;
    while cpu.eip < end {
        engine
            .step(&mut cpu, &mut mem)
            .unwrap_or_else(|stop| panic!("stopped at {:#010x}: {stop}", cpu.eip));
        steps += 1;
        assert!(steps < 100, "the sequence should end, not run on");
    }
    (cpu, mem)
}

fn put_single(mem: &mut Arena, at: u32, value: f32) {
    mem.write_u32(at, value.to_bits()).expect("in the arena");
}

fn put_double(mem: &mut Arena, at: u32, value: f64) {
    let bits = value.to_bits();
    mem.write_u32(at, bits as u32).expect("in the arena");
    mem.write_u32(at + 4, (bits >> 32) as u32)
        .expect("in the arena");
}

fn get_single(mem: &Arena, at: u32) -> f32 {
    f32::from_bits(mem.read_u32(at).expect("in the arena"))
}

fn get_double(mem: &Arena, at: u32) -> f64 {
    let lo = mem.read_u32(at).expect("in the arena");
    let hi = mem.read_u32(at + 4).expect("in the arena");
    f64::from_bits((u64::from(hi) << 32) | u64::from(lo))
}

/// `fld dword [DATA]` then `fstp dword [DATA+16]`.
#[test]
fn a_single_loaded_and_stored_comes_back_unchanged() {
    let code = [
        0xd9, 0x05, 0x00, 0x00, 0x50, 0x00, // fld dword [0x500000]
        0xd9, 0x1d, 0x10, 0x00, 0x50, 0x00, // fstp dword [0x500010]
    ];
    let (_cpu, mem) = run(&code, |mem| put_single(mem, DATA, 1.5));
    assert_eq!(get_single(&mem, DATA + 16), 1.5);
}

#[test]
fn a_double_loaded_and_stored_keeps_every_bit() {
    let code = [
        0xdd, 0x05, 0x00, 0x00, 0x50, 0x00, // fld qword [0x500000]
        0xdd, 0x1d, 0x10, 0x00, 0x50, 0x00, // fstp qword [0x500010]
    ];
    let value = 0.1_f64;
    let (_cpu, mem) = run(&code, |mem| put_double(mem, DATA, value));
    assert_eq!(get_double(&mem, DATA + 16).to_bits(), value.to_bits());
}

#[test]
fn the_four_operations_give_the_answers_they_should() {
    // fld a; fadd b; fmul b; fsub a; fdiv b; fstp
    for (a, b, expected) in [
        (3.0_f64, 2.0_f64, ((3.0 + 2.0) * 2.0 - 3.0) / 2.0),
        (0.5, 0.25, ((0.5 + 0.25) * 0.25 - 0.5) / 0.25),
        (-7.5, 1.5, ((-7.5 + 1.5) * 1.5 + 7.5) / 1.5),
    ] {
        let code = [
            0xdd, 0x05, 0x00, 0x00, 0x50, 0x00, // fld qword [DATA]      a
            0xdc, 0x05, 0x08, 0x00, 0x50, 0x00, // fadd qword [DATA+8]   b
            0xdc, 0x0d, 0x08, 0x00, 0x50, 0x00, // fmul qword [DATA+8]   b
            0xdc, 0x25, 0x00, 0x00, 0x50, 0x00, // fsub qword [DATA]     a
            0xdc, 0x35, 0x08, 0x00, 0x50, 0x00, // fdiv qword [DATA+8]   b
            0xdd, 0x1d, 0x10, 0x00, 0x50, 0x00, // fstp qword [DATA+16]
        ];
        let (_cpu, mem) = run(&code, |mem| {
            put_double(mem, DATA, a);
            put_double(mem, DATA + 8, b);
        });
        let got = get_double(&mem, DATA + 16);
        assert!(
            (got - expected).abs() < 1e-12,
            "({a} + {b}) * {b} - {a} then / {b} gave {got}, not {expected}"
        );
    }
}

#[test]
fn a_square_root_and_a_sine_come_off_the_stack_where_they_were_put() {
    let code = [
        0xdd, 0x05, 0x00, 0x00, 0x50, 0x00, // fld qword [DATA]
        0xd9, 0xfa, // fsqrt
        0xd9, 0xfe, // fsin
        0xdd, 0x1d, 0x10, 0x00, 0x50, 0x00, // fstp qword [DATA+16]
    ];
    let (_cpu, mem) = run(&code, |mem| put_double(mem, DATA, 4.0));
    let got = get_double(&mem, DATA + 16);
    // sin(sqrt(4)) = sin(2)
    assert!(
        (got - 0.909_297_426_825_681_7).abs() < 1e-12,
        "sin(sqrt(4)) was {got}"
    );
}

#[test]
fn an_integer_loaded_and_stored_rounds_to_nearest_with_ties_to_even() {
    for (value, expected) in [(2.5_f64, 2), (3.5, 4), (-2.5, -2), (1.4, 1), (1.6, 2)] {
        let code = [
            0xdd, 0x05, 0x00, 0x00, 0x50, 0x00, // fld qword [DATA]
            0xdb, 0x1d, 0x10, 0x00, 0x50, 0x00, // fistp dword [DATA+16]
        ];
        let (_cpu, mem) = run(&code, |mem| put_double(mem, DATA, value));
        let got = mem.read_u32(DATA + 16).expect("in the arena") as i32;
        assert_eq!(got, expected, "storing {value} as an integer");
    }
}

#[test]
fn a_comparison_reaches_the_processor_through_the_status_word() {
    // fld a; fcomp b; fnstsw ax
    for (a, b, c3, c0) in [
        (2.0_f64, 1.0_f64, false, false),
        (1.0, 2.0, false, true),
        (1.0, 1.0, true, false),
    ] {
        let code = [
            0xdd, 0x05, 0x00, 0x00, 0x50, 0x00, // fld qword [DATA]
            0xdc, 0x1d, 0x08, 0x00, 0x50, 0x00, // fcomp qword [DATA+8]
            0xdf, 0xe0, // fnstsw ax
        ];
        let (cpu, _mem) = run(&code, |mem| {
            put_double(mem, DATA, a);
            put_double(mem, DATA + 8, b);
        });
        let status = cpu.r[EAX] as u16;
        assert_eq!(status & 0x4000 != 0, c3, "C3 comparing {a} with {b}");
        assert_eq!(status & 0x0100 != 0, c0, "C0 comparing {a} with {b}");
    }
}

#[test]
fn the_control_word_is_written_and_read_back() {
    let code = [
        0xd9, 0x2d, 0x00, 0x00, 0x50, 0x00, // fldcw [DATA]
        0xd9, 0x3d, 0x10, 0x00, 0x50, 0x00, // fnstcw [DATA+16]
    ];
    let (_cpu, mem) = run(&code, |mem| {
        mem.write_u32(DATA, 0x0272).expect("in the arena");
    });
    assert_eq!(
        mem.read_u32(DATA + 16).expect("in the arena") & 0xffff,
        0x0272
    );
}

#[test]
fn the_register_forms_subtract_and_divide_the_way_round_intel_documents() {
    // st(1) = 10 and st(0) = 4. The escape picks where the answer goes (d8:
    // st(0); dc: st(1); de: st(1), then pop); the operation reads the same
    // either way: /4 is st(0) - st(i), /5 st(i) - st(0), /6 st(0) / st(i) and
    // /7 st(i) / st(0). Rock Raiders divides its keyframe interval with
    // `de f9`, fdivp st(1): (t - lo) / (hi - lo), not its reciprocal.
    const FXCH: [u8; 2] = [0xd9, 0xc9];
    for (op, expected) in [
        ([0xd8, 0xe1], -6.0),
        ([0xd8, 0xe9], 6.0),
        ([0xd8, 0xf1], 0.4),
        ([0xd8, 0xf9], 2.5),
        ([0xdc, 0xe1], -6.0),
        ([0xdc, 0xe9], 6.0),
        ([0xdc, 0xf1], 0.4),
        ([0xdc, 0xf9], 2.5),
        ([0xde, 0xe1], -6.0),
        ([0xde, 0xe9], 6.0),
        ([0xde, 0xf1], 0.4),
        ([0xde, 0xf9], 2.5),
    ] {
        let mut code = vec![
            0xdd, 0x05, 0x00, 0x00, 0x50, 0x00, // fld qword [DATA]    10
            0xdd, 0x05, 0x08, 0x00, 0x50, 0x00, // fld qword [DATA+8]  4
        ];
        code.extend(op);
        // The answer to st(0), wherever it went.
        if op[0] == 0xdc {
            code.extend(FXCH);
        }
        code.extend([0xdd, 0x1d, 0x10, 0x00, 0x50, 0x00]); // fstp qword [DATA+16]
        let (_cpu, mem) = run(&code, |mem| {
            put_double(mem, DATA, 10.0);
            put_double(mem, DATA + 8, 4.0);
        });
        assert_eq!(get_double(&mem, DATA + 16), expected, "{op:02x?}");
    }
}

#[test]
fn single_precision_control_rounds_what_the_unit_computes() {
    // With the precision field set to single, a third computed in the unit
    // must equal a third computed in a float.
    let code = [
        0xd9, 0x2d, 0x00, 0x00, 0x50, 0x00, // fldcw [DATA]  (single precision)
        0xdd, 0x05, 0x08, 0x00, 0x50, 0x00, // fld qword [DATA+8]   1.0
        0xdc, 0x35, 0x10, 0x00, 0x50, 0x00, // fdiv qword [DATA+16] 3.0
        0xdd, 0x1d, 0x20, 0x00, 0x50, 0x00, // fstp qword [DATA+32]
    ];
    let (_cpu, mem) = run(&code, |mem| {
        // Precision control 0 is single; everything else as fninit leaves it.
        mem.write_u32(DATA, 0x007f).expect("in the arena");
        put_double(mem, DATA + 8, 1.0);
        put_double(mem, DATA + 16, 3.0);
    });
    let got = get_double(&mem, DATA + 32);
    assert_eq!(
        got,
        f64::from(1.0f32 / 3.0f32),
        "the unit rounded to single as it was told"
    );
}

#[test]
fn an_escape_the_unit_does_not_have_stops_the_run_and_names_it() {
    let mut host = Headless::new();
    let mut mem = Arena::new(&mut host, ARENA_BYTES).expect("an arena");
    // db /4: one of the escapes nothing on the roster has asked for.
    mem.write(CODE, &[0xdb, 0xe4]).expect("the code fits");
    let mut cpu = Cpu {
        eip: CODE,
        ..Cpu::default()
    };
    let stop = Interp::new(Mode::FLAT)
        .step(&mut cpu, &mut mem)
        .expect_err("this form is not implemented");
    match stop {
        Stop::Undecoded { byte, why, .. } => {
            assert_eq!(byte, 0xdb);
            assert!(
                why.contains("floating-point"),
                "the wall says what it was: {why}"
            );
        }
        other => panic!("stopped {other} instead of naming the form"),
    }
}

#[test]
fn nothing_in_the_engine_reaches_the_hosts_maths_library() {
    // A run has to give the same answer on every machine and in twenty years,
    // and a library that is free to improve its accuracy is a run that is free
    // to change. Square root is the exception: it is one instruction on every
    // processor this runs on, and IEEE-754 fixes its answer exactly.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the source tree reads") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file reads");
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or(line);
                for call in [
                    ".sin()", ".cos()", ".tan()", ".ln()", ".log2()", ".exp()", ".powf(", ".atan(",
                    ".atan2(", ".exp2(",
                ] {
                    if code.contains(call) {
                        offenders.push(format!("{}:{}: {}", path.display(), n + 1, code.trim()));
                    }
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these reach the host's maths library instead of the engine's own:\n{}",
        offenders.join("\n")
    );
}
