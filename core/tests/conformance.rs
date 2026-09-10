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
//! held: `LEGO_LEGACY_REQUIRE_CORPUS=1` turns the skip into a failure.

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
const CLAIMED_FLOOR: usize = 515;

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
        let mask = !undef(EFLAGS);
        let got = cpu.flags.to_eflags();
        if got & mask != want & mask {
            return Err(format!(
                "eflags = {got:#06x}, corpus says {want:#06x} (undefined {:#06x})",
                undef(EFLAGS)
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
        // An instruction the corpus took an exception on exercises a machine
        // this engine does not have yet: no interrupt table, no fault gate.
        if test.exception.is_some() {
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
        if let Err(why) = result {
            if score.failures.len() < 20 {
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
    let tests_run: usize = scores.iter().map(|s| s.tests_run).sum();
    let mut unclaimed: Vec<&String> = scores.iter().flat_map(|s| &s.unclaimed).collect();
    unclaimed.sort();
    let failures: Vec<&String> = scores.iter().flat_map(|s| &s.failures).collect();

    eprintln!(
        "corpus: {claimed} of {} encodings claimed, {tests_run} vectors run",
        files.len()
    );
    eprintln!("corpus: not decoded yet: {}", unclaimed.len());
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
    assert!(
        claimed >= CLAIMED_FLOOR || std::env::var("CONFORMANCE_FILE").is_ok(),
        "this engine claims {claimed} encodings, fewer than the {CLAIMED_FLOOR} it did before"
    );
}
