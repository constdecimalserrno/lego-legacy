//! The engine, held to somebody else's measurements.
//!
//! The corpus is SingleStepTests' 80386 suite: for each instruction encoding,
//! thousands of recorded (initial state, final state) pairs taken from real
//! silicon. The vectors are real mode, which is why the engine has a mode at
//! all: the arithmetic and the flags are the same machine either way, and only
//! how an address is formed differs.
//!
//! Fetch it with `scripts/fetch-corpus.sh`. Without it these tests print a
//! skip line and pass, which is a gate that did not run rather than one that
//! held: `LEGO_LEGACY_REQUIRE_CORPUS=1` turns the skip into a failure, and
//! `LEGO_LEGACY_CORPUS` reads the vectors from somewhere else.
//!
//! Three more, for working on the engine rather than for the gate:
//! - `CONFORMANCE_FILE=<prefix>` runs only the files whose names start with
//!   it, and lifts the claimed floor, which part of the corpus cannot reach.
//!   The nine saturating byte divisions are still counted, so a part without
//!   the byte divisions fails on that count and not on its own vectors.
//! - `CONFORMANCE_UNCLAIMED` (any value) prints every encoding not decoded
//!   yet, by name.
//! - `CONFORMANCE_DUMP` (any value) prints each vector, its bytes, registers
//!   and memory before and after, as it is compared: with `CONFORMANCE_FILE`,
//!   to read the vectors one encoding fails on.

use lego_legacy_core::cpu::{interp::Interp, Cpu, CpuEngine, Mode, Stop};
use lego_legacy_core::guest::{Arena, GuestMem};
use lego_legacy_core::host::headless::Headless;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the vectors live under the fetched corpus.
const CORPUS: &str = "vendor/singlesteptests/80386/v1_ex_real_mode";

/// How many of the corpus's 941 encodings this engine claims today.
///
/// It only ever goes up. A change that quietly stops decoding an encoding
/// fails here rather than in a title six months later.
const CLAIMED_FLOOR: usize = 877;

/// The encodings this engine does not decode, and why each is left out.
///
/// Nothing is here for lack of effort: each entry is a decision about what the
/// declared machine is. A file whose name is not covered here fails the test
/// rather than joining the list quietly, so the set can only shrink on purpose.
const LEFT_OUT: [(&str, &str); 8] = [
    (
        "6C|6D|6E|6F|E4|E5|E6|E7|EC|ED|EE|EF",
        "port input and output: the declared machine has no I/O ports, and a Win32 title that \
         reaches one has left the machine we are emulating",
    ),
    (
        "9A|CA|CB|EA|FF.3|FF.5",
        "far calls, far returns and far jumps: the roster runs flat, where every one of these \
         is a segment change the declared machine does not have",
    ),
    (
        "CC|CE|CF",
        "int3, into and iret: this machine has no interrupt table to vector through, and a \
         guest that reaches one stops as a wall rather than being dispatched",
    ),
    (
        "C4|C5|0FB2|0FB4|0FB5",
        "les, lds, lss, lfs and lgs: loading a segment register together with an offset, which \
         a flat title has no use for",
    ),
    (
        "62",
        "bound: it exists to raise an exception, and this machine has no gate to raise it \
         through",
    ),
    (
        "0F06",
        "clts: a control-register instruction on a machine that declares no control registers",
    ),
    (
        "D6",
        "salc: undocumented, and no compiler that built the roster emits it",
    ),
    (
        "6662",
        "bound with a wide operand, for the same reason as bound itself",
    ),
];

/// Whether an encoding's absence has been accounted for.
fn accounted_for(name: &str) -> bool {
    // A file name carries its prefixes, and the reasons are written against the
    // instruction rather than against every prefixed spelling of it.
    let mut bare = name;
    loop {
        let stripped = bare.strip_prefix("66").or_else(|| bare.strip_prefix("67"));
        match stripped {
            Some(rest) if !rest.is_empty() => bare = rest,
            _ => break,
        }
    }
    LEFT_OUT
        .iter()
        .any(|(names, _)| names.split('|').any(|n| n == bare || n == name))
}

fn corpus_dir() -> Option<PathBuf> {
    let p = std::env::var_os("LEGO_LEGACY_CORPUS").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(CORPUS)
        },
        PathBuf::from,
    );
    p.is_dir().then_some(p)
}

/// Say loudly what did not run, or refuse to pretend when the corpus is
/// required.
fn skip() {
    let required = std::env::var("LEGO_LEGACY_REQUIRE_CORPUS").is_ok_and(|v| v != "0");
    assert!(
        !required,
        "LEGO_LEGACY_REQUIRE_CORPUS is set but {CORPUS} is absent"
    );
    eprintln!("skipping: {CORPUS} is absent (run scripts/fetch-corpus.sh)");
}

/// The corpus's own container format, read with a cursor.
mod moo {
    /// One recorded vector.
    pub struct Test {
        pub name: String,
        pub bytes: Vec<u8>,
        pub init: State,
        pub fina: State,
        /// Which bits of which registers the corpus itself calls undefined.
        pub undefined: [Option<u32>; 20],
        /// The interrupt the processor took, when it took one.
        pub exception: Option<u8>,
    }

    #[derive(Default, Clone)]
    pub struct State {
        /// Registers, in the corpus's own order, present only when recorded.
        pub regs: [Option<u32>; 20],
        pub ram: Vec<(u32, u8)>,
    }

    pub struct File {
        pub cpu_id: [u8; 4],
        pub tests: Vec<Test>,
    }

    struct Cur<'a> {
        b: &'a [u8],
        at: usize,
    }

    impl<'a> Cur<'a> {
        fn u8(&mut self) -> u8 {
            let v = self.b.get(self.at).copied().unwrap_or(0);
            self.at += 1;
            v
        }
        fn u16(&mut self) -> u16 {
            u16::from(self.u8()) | (u16::from(self.u8()) << 8)
        }
        fn u32(&mut self) -> u32 {
            u32::from(self.u16()) | (u32::from(self.u16()) << 16)
        }
        fn take(&mut self, n: usize) -> &'a [u8] {
            let end = (self.at + n).min(self.b.len());
            let s = &self.b[self.at.min(self.b.len())..end];
            self.at = end;
            s
        }
        fn done(&self) -> bool {
            self.at + 8 > self.b.len()
        }
        /// One chunk: a four-byte identifier and a counted payload.
        fn chunk(&mut self) -> ([u8; 4], &'a [u8]) {
            let mut id = [0u8; 4];
            id.copy_from_slice(self.take(4));
            let len = self.u32() as usize;
            (id, self.take(len))
        }
    }

    fn regs32(payload: &[u8], into: &mut [Option<u32>; 20]) {
        let mut c = Cur { b: payload, at: 0 };
        let mask = c.u32();
        for (i, slot) in into.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *slot = Some(c.u32());
            }
        }
    }

    fn ram(payload: &[u8]) -> Vec<(u32, u8)> {
        let mut c = Cur { b: payload, at: 0 };
        let count = c.u32() as usize;
        (0..count).map(|_| (c.u32(), c.u8())).collect()
    }

    fn state(payload: &[u8], undefined: &mut [Option<u32>; 20]) -> State {
        let mut s = State::default();
        let mut c = Cur { b: payload, at: 0 };
        while !c.done() {
            let (id, body) = c.chunk();
            match &id {
                b"RG32" => regs32(body, &mut s.regs),
                b"RM32" => regs32(body, undefined),
                b"RAM " => s.ram = ram(body),
                // REGS, RMSK, QUEU and EA32 describe a machine narrower than
                // this one, or repeat what is already here.
                _ => {}
            }
        }
        s
    }

    /// Read one decompressed corpus file.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> File {
        let mut c = Cur { b: bytes, at: 0 };
        let (_id, header) = c.chunk();
        let mut h = Cur { b: header, at: 0 };
        let (_major, _minor) = (h.u8(), h.u8());
        h.take(2);
        let _count = h.u32();
        let mut cpu_id = [0u8; 4];
        cpu_id.copy_from_slice(h.take(4));

        let mut file_undefined = [None; 20];
        let mut tests = Vec::new();
        while !c.done() {
            let (id, body) = c.chunk();
            match &id {
                b"RM32" => regs32(body, &mut file_undefined),
                b"TEST" => {
                    let mut t = Cur { b: body, at: 0 };
                    let _index = t.u32();
                    let mut test = Test {
                        name: String::new(),
                        bytes: Vec::new(),
                        init: State::default(),
                        fina: State::default(),
                        undefined: file_undefined,
                        exception: None,
                    };
                    while !t.done() {
                        let (sid, sbody) = t.chunk();
                        let mut s = Cur { b: sbody, at: 0 };
                        match &sid {
                            b"NAME" => {
                                let n = s.u32() as usize;
                                test.name = String::from_utf8_lossy(s.take(n)).into_owned();
                            }
                            b"BYTS" => {
                                let n = s.u32() as usize;
                                test.bytes = s.take(n).to_vec();
                            }
                            b"INIT" => {
                                test.init = state(sbody, &mut test.undefined);
                            }
                            b"FINA" => {
                                test.fina = state(sbody, &mut test.undefined);
                            }
                            b"EXCP" => test.exception = Some(s.u8()),
                            _ => {}
                        }
                    }
                    tests.push(test);
                }
                _ => {}
            }
        }
        File { cpu_id, tests }
    }
}

/// The corpus's register order, and where each lands on this engine.
///
/// `cr0`, `cr3`, `dr6` and `dr7` are recorded and not modelled: this machine
/// declares no control or debug registers, and no test in the corpus's real
/// mode changes one.
const CR0: usize = 0;
const EIP: usize = 16;
const EFLAGS: usize = 17;
const GPR: [(usize, usize); 8] = [
    (2, lego_legacy_core::cpu::EAX),
    (3, lego_legacy_core::cpu::EBX),
    (4, lego_legacy_core::cpu::ECX),
    (5, lego_legacy_core::cpu::EDX),
    (6, lego_legacy_core::cpu::ESI),
    (7, lego_legacy_core::cpu::EDI),
    (8, lego_legacy_core::cpu::EBP),
    (9, lego_legacy_core::cpu::ESP),
];
const SEGS: [(usize, usize); 6] = [
    (10, lego_legacy_core::cpu::CS),
    (11, lego_legacy_core::cpu::DS),
    (12, lego_legacy_core::cpu::ES),
    (13, lego_legacy_core::cpu::FS),
    (14, lego_legacy_core::cpu::GS),
    (15, lego_legacy_core::cpu::SS),
];

/// Which `EFLAGS` bits the architecture itself leaves undefined for an
/// instruction.
///
/// The corpus records what one particular 386 left in those bits. Holding an
/// engine to that would be measuring the silicon's own arbitrary choice rather
/// than the instruction, so these bits are excluded from the comparison. The
/// list is Intel's: each entry is a documented "undefined" in the manual's
/// flags column.
fn undefined_flags(bytes: &[u8], init_ecx: u32) -> u32 {
    use lego_legacy_core::cpu::flags::{AF, CF, OF, PF, SF, ZF};

    let mut i = 0;
    let mut wide_operand = false;
    for b in bytes {
        if *b == 0x66 {
            wide_operand = true;
        }
    }
    while i < bytes.len()
        && matches!(
            bytes[i],
            0x66 | 0x67 | 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 | 0xf0 | 0xf2 | 0xf3
        )
    {
        i += 1;
    }
    let Some(&op) = bytes.get(i) else { return 0 };
    let modrm_reg = |at: usize| bytes.get(at).map_or(0, |b| (b >> 3) & 7);

    match op {
        // The decimal adjustments
        0x27 | 0x2f => OF,
        0x37 | 0x3f => OF | SF | ZF | PF,
        0xd4 | 0xd5 => CF | OF | AF,
        // Shifts and rotates: the count decides whether overflow is defined.
        0xc0 | 0xc1 | 0xd0 | 0xd1 | 0xd2 | 0xd3 => {
            // An immediate count is the encoding's last byte, wherever the
            // addressing bytes left it.
            let count = match op {
                0xc0 | 0xc1 => u32::from(bytes.last().copied().unwrap_or(0)),
                0xd0 | 0xd1 => 1,
                _ => init_ecx & 0x1f,
            } & 0x1f;
            let rotate = modrm_reg(i + 1) < 4;
            // How wide the thing being shifted is, so that a count longer than
            // it can be recognised.
            let bits = if op & 1 == 0 {
                8
            } else if wide_operand {
                32
            } else {
                16
            };
            if count == 0 {
                0
            } else if !rotate && count > bits {
                // Measured, on 433 byte shifts with a count past the operand's
                // width: the carry left behind is zero in 414 of them and its
                // own previous value in 203, so it follows neither rule and is
                // undefined in fact as well as in the manual.
                CF | AF | OF
            } else if rotate {
                if count == 1 {
                    0
                } else {
                    OF
                }
            } else if count == 1 {
                AF
            } else {
                AF | OF
            }
        }
        // Multiply and divide leave everything but carry and overflow adrift,
        // and divide leaves those too.
        0xf6 | 0xf7 => match modrm_reg(i + 1) {
            4 | 5 => SF | ZF | AF | PF,
            6 | 7 => CF | OF | SF | ZF | AF | PF,
            _ => 0,
        },
        0x69 | 0x6b => SF | ZF | AF | PF,
        0x0f => match bytes.get(i + 1).copied().unwrap_or(0) {
            // The bit tests
            0xa3 | 0xab | 0xb3 | 0xbb | 0xba => OF | SF | AF | PF,
            // The bit scans
            0xbc | 0xbd => CF | OF | SF | AF | PF,
            0xaf => SF | ZF | AF | PF,
            // The double shifts
            0xa4 | 0xac => {
                let count = u32::from(bytes.last().copied().unwrap_or(0)) & 0x1f;
                if count == 0 {
                    0
                } else if count == 1 {
                    AF
                } else {
                    AF | OF
                }
            }
            0xa5 | 0xad => {
                let count = init_ecx & 0x1f;
                if count == 0 {
                    0
                } else if count == 1 {
                    AF
                } else {
                    AF | OF
                }
            }
            _ => 0,
        },
        _ => 0,
    }
}

fn load(test: &moo::Test, mem: &mut Arena) -> Cpu {
    let mut cpu = Cpu::default();
    for (moo_i, ours) in GPR {
        if let Some(v) = test.init.regs[moo_i] {
            cpu.r[ours] = v;
        }
    }
    for (moo_i, ours) in SEGS {
        if let Some(v) = test.init.regs[moo_i] {
            cpu.set_seg(ours, v as u16, Mode::REAL);
        }
    }
    cpu.eip = test.init.regs[EIP].unwrap_or(0);
    cpu.flags =
        lego_legacy_core::cpu::flags::Flags::from_eflags(test.init.regs[EFLAGS].unwrap_or(2));
    for &(addr, value) in &test.init.ram {
        mem.write(addr, &[value])
            .expect("corpus ram is inside the arena");
    }
    cpu
}

/// What the corpus expected, against what the engine did.
fn compare(test: &moo::Test, cpu: &Cpu, mem: &Arena) -> Result<(), String> {
    let expect = |i: usize| test.fina.regs[i].or(test.init.regs[i]);
    let undef = |i: usize| test.undefined[i].unwrap_or(0);

    if std::env::var_os("CONFORMANCE_DUMP").is_some() {
        let mut init = test.init.ram.clone();
        init.sort_unstable();
        eprintln!(
            "DUMP name={} bytes={:02x?} regs={:?} fina={:?} init_ram={:x?} fina_ram={:x?}",
            test.name, test.bytes, test.init.regs, test.fina.regs, init, test.fina.ram
        );
    }
    for (moo_i, ours) in GPR {
        if let Some(want) = expect(moo_i) {
            let got = cpu.r[ours];
            let mask = !undef(moo_i);
            if got & mask != want & mask {
                return Err(format!(
                    "{} = {got:#010x}, corpus says {want:#010x}",
                    lego_legacy_core::cpu::REG_NAMES[ours]
                ));
            }
        }
    }
    for (moo_i, ours) in SEGS {
        if let Some(want) = expect(moo_i) {
            if u32::from(cpu.seg[ours]) != want & 0xffff {
                return Err(format!(
                    "{} = {:#06x}, corpus says {:#06x}",
                    lego_legacy_core::cpu::SEG_NAMES[ours],
                    cpu.seg[ours],
                    want & 0xffff
                ));
            }
        }
    }
    if let Some(want) = expect(EIP) {
        if cpu.eip != want {
            return Err(format!("eip = {:#010x}, corpus says {want:#010x}", cpu.eip));
        }
    }
    if let Some(want) = expect(EFLAGS) {
        let architectural = undefined_flags(&test.bytes, test.init.regs[4].unwrap_or(0));
        let mask = !(undef(EFLAGS) | architectural);
        let got = cpu.flags.to_eflags();
        if got & mask != want & mask {
            return Err(format!(
                "eflags = {got:#06x}, corpus says {want:#06x} (undefined {:#06x})",
                undef(EFLAGS) | architectural
            ));
        }
    }
    for &(addr, want) in &test.fina.ram {
        let got = mem.slice(addr, 1).map_err(|f| f.to_string())?[0];
        if got != want {
            return Err(format!(
                "[{addr:#010x}] = {got:#04x}, corpus says {want:#04x}"
            ));
        }
    }
    Ok(())
}

struct Score {
    claimed: usize,
    /// Vectors whose repeat runs past the segment they are in.
    unbounded: usize,
    /// Vectors where the recorded processor differs from the declared one.
    quirks: usize,
    /// Vectors the recorded processor took an exception on.
    exceptions: usize,
    unclaimed: Vec<String>,
    failures: Vec<String>,
    tests_run: usize,
}

fn run_file(path: &Path, score: &mut Score) {
    let out = Command::new("gunzip")
        .arg("-c")
        .arg(path)
        .output()
        .expect("the host has gunzip");
    assert!(out.status.success(), "gunzip refused {}", path.display());
    let file = moo::parse(&out.stdout);
    assert_eq!(
        &file.cpu_id,
        b"386E",
        "{} is not a 386 file",
        path.display()
    );

    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .replace(".MOO.gz", "");

    // An encoding is claimed when this engine decodes it. Nothing is listed by
    // hand: the decoder itself says what it knows.
    let Some(first) = file.tests.first() else {
        return;
    };
    if lego_legacy_core::cpu::interp::decode_one(&first.bytes, Mode::REAL).is_err() {
        score.unclaimed.push(name);
        return;
    }
    score.claimed += 1;

    let mut host = Headless::new();
    // Real mode reaches a megabyte plus the wrap the corpus exercises.
    let mut mem = Arena::new(&mut host, 0x0011_0000).expect("an arena for real mode");

    for test in &file.tests {
        // A vector the processor took an exception on. This machine has no
        // interrupt table to vector through, so what is checked is that the
        // engine refused in the same place and for the same reason: a divide
        // error is the one the roster's compilers actually emit code around.
        if let Some(vector) = test.exception {
            // A software interrupt vectors through the same table as a fault,
            // so `int 0` is recorded exactly like a divide error. What is
            // being checked here is the fault, not the instruction that asks
            // for one by name.
            let asked_for_it = test
                .bytes
                .iter()
                .find(|b| {
                    !matches!(
                        b,
                        0x66 | 0x67 | 0x2e | 0x36 | 0x3e | 0x26 | 0x64 | 0x65 | 0xf0 | 0xf2 | 0xf3
                    )
                })
                .is_some_and(|op| matches!(op, 0xcd | 0xcc | 0xce | 0xf1));
            if vector == 0 && !asked_for_it {
                let mut cpu = load(test, &mut mem);
                let mut engine = Interp::new(Mode::REAL);
                let stopped = loop {
                    match engine.step(&mut cpu, &mut mem) {
                        Ok(()) => continue,
                        Err(stop) => break stop,
                    }
                };
                score.exceptions += 1;
                if stopped != Stop::DivideError && score.failures.len() < 400 {
                    score.failures.push(format!(
                        "{name}: `{}`: corpus took a divide error, engine {stopped}",
                        test.name
                    ));
                }
            }
            continue;
        }
        // Control registers this machine does not declare.
        if test.init.regs[CR0].is_some_and(|v| v & 1 != 0) {
            continue;
        }

        for &(addr, _) in &test.init.ram {
            let _ = mem.write(addr, &[0]);
        }
        let mut cpu = load(test, &mut mem);
        let mut engine = Interp::new(Mode::REAL);
        score.tests_run += 1;
        // Each vector's bytes are followed by a halt, and the state the corpus
        // recorded is the one after it: a repeating instruction is one vector,
        // however many times it goes round.
        let mut steps = 0;
        let outcome = loop {
            match engine.step(&mut cpu, &mut mem) {
                Ok(()) => {
                    steps += 1;
                    if steps > 200_000 {
                        break Err("never halted".to_owned());
                    }
                }
                Err(Stop::Halted) => break Ok(()),
                Err(stop) => break Err(format!("stopped: {stop}")),
            }
        };
        let result = outcome.and_then(|()| compare(test, &cpu, &mem));
        // A repeat counted in thirty-two bits inside a sixteen-bit segment
        // runs past the segment it is in and over its own halt, and ends
        // somewhere neither the corpus nor an engine can be held to. Counted
        // rather than failed, and only when it actually ran away.
        let ran_away = result.as_ref().is_err_and(|why| {
            (why.contains("never halted") || why.contains("undecoded"))
                && test.bytes.contains(&0x67)
                && (test.bytes.contains(&0xf3) || test.bytes.contains(&0xf2))
        });
        if ran_away {
            score.unbounded += 1;
            continue;
        }
        // The 386EX does not fault on a byte division whose quotient will not
        // fit: it saturates and carries on. The declared machine is a Pentium
        // II, which faults, so this engine faults and these vectors are
        // counted rather than followed.
        let saturating_divide = result
            .as_ref()
            .is_err_and(|why| why.contains("divide error"))
            && test.exception.is_none();
        if saturating_divide {
            score.quirks += 1;
            continue;
        }
        if let Err(why) = result {
            if score.failures.len() < 400 {
                score
                    .failures
                    .push(format!("{name}: `{}`: {why}", test.name));
            }
        }
    }
}

#[test]
fn the_engine_matches_the_corpus_on_every_encoding_it_claims() {
    let Some(dir) = corpus_dir() else {
        skip();
        return;
    };

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("the corpus directory can be read")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "gz"))
        .collect();
    files.sort();
    if let Ok(only) = std::env::var("CONFORMANCE_FILE") {
        files.retain(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with(&only)
        });
    }
    assert!(!files.is_empty(), "the corpus directory holds no vectors");

    let threads = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    let chunk = files.len().div_ceil(threads);
    let scores: Vec<Score> = std::thread::scope(|s| {
        let handles: Vec<_> = files
            .chunks(chunk)
            .map(|group| {
                s.spawn(move || {
                    let mut score = Score {
                        claimed: 0,
                        unbounded: 0,
                        quirks: 0,
                        exceptions: 0,
                        unclaimed: Vec::new(),
                        failures: Vec::new(),
                        tests_run: 0,
                    };
                    for p in group {
                        run_file(p, &mut score);
                    }
                    score
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a shard finished"))
            .collect()
    });

    let claimed: usize = scores.iter().map(|s| s.claimed).sum();
    let unbounded: usize = scores.iter().map(|s| s.unbounded).sum();
    let quirks: usize = scores.iter().map(|s| s.quirks).sum();
    let exceptions: usize = scores.iter().map(|s| s.exceptions).sum();
    let tests_run: usize = scores.iter().map(|s| s.tests_run).sum();
    let mut unclaimed: Vec<&String> = scores.iter().flat_map(|s| &s.unclaimed).collect();
    unclaimed.sort();
    let failures: Vec<&String> = scores.iter().flat_map(|s| &s.failures).collect();

    eprintln!(
        "corpus: {claimed} of {} encodings claimed, {tests_run} vectors run",
        files.len()
    );
    eprintln!("corpus: not decoded yet: {}", unclaimed.len());
    eprintln!("corpus: repeats past their segment, not run: {unbounded}");
    eprintln!("corpus: divide faults the engine had to agree with: {exceptions}");
    eprintln!("corpus: 386EX saturating byte divisions, not followed: {quirks}");
    if std::env::var_os("CONFORMANCE_UNCLAIMED").is_some() {
        eprintln!(
            "corpus: {}",
            unclaimed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    assert!(
        failures.is_empty(),
        "{} vectors disagree with the corpus; first few:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .map(|f| format!("  {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let unaccounted: Vec<&&String> = unclaimed.iter().filter(|n| !accounted_for(n)).collect();
    assert!(
        unaccounted.is_empty(),
        "these encodings are neither decoded nor accounted for: {unaccounted:?}. \
         Decode them, or say in LEFT_OUT why the declared machine does not have them."
    );
    assert_eq!(
        quirks, 9,
        "the 386EX's saturating byte division was measured at nine vectors; a different \
         number means something else is now failing to divide"
    );
    assert!(
        claimed >= CLAIMED_FLOOR || std::env::var("CONFORMANCE_FILE").is_ok(),
        "this engine claims {claimed} encodings, fewer than the {CLAIMED_FLOOR} it did before"
    );
}
