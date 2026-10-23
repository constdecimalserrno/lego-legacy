//! A clock spin passed in one step, exactly (ADR-0004, #217).
//!
//! A title that waits by asking the clock again and again makes the same pass
//! thousands of times a declared millisecond. The machine may retire those
//! passes at once, but only where running every one of them would leave it
//! exactly as it is: the same cursor, declared clock, registers, memory, flips
//! and mix. Each guest here runs twice: as a route's run runs it, and traced,
//! which runs every pass (a traced run shows every call, so it never skips
//! one). The two must end as one machine.
//!
//! Each guest is a few hand-assembled instructions calling KERNEL32, WINMM,
//! USER32 and DirectSound through the boundary the way a title's imports do,
//! as `threads.rs` assembles its own.

use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::x87::X87;
use lego_legacy_core::cpu::Cpu;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::{Finish, Machine, Outcome, STACK_TOP};
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::Title;

const STUB: Title = Title::STUB;

/// Where the main thread's code goes, and a second thread's.
const MAIN: u32 = 0x0040_0000;
const SECOND: u32 = 0x0040_1000;
/// Where the guest keeps what it saw, a word apiece.
const DATA: u32 = 0x0050_0000;

/// A word of the guest's data.
const fn word(n: u32) -> u32 {
    DATA + n * 4
}

/// The word the guest keeps the hour it waits for in.
const UNTIL: u32 = word(0);

#[derive(Clone, Copy)]
enum Arg {
    Imm(u32),
    /// The word at this address, as `push dword [at]`.
    At(u32),
}
use Arg::{At, Imm};

/// The clock reads whose answer is the declared millisecond.
#[derive(Clone, Copy, Debug)]
enum Read {
    QueryPerformanceCounter,
    GetTickCount,
    TimeGetTime,
    TimeGetSystemTime,
}

const READS: [Read; 4] = [
    Read::QueryPerformanceCounter,
    Read::GetTickCount,
    Read::TimeGetTime,
    Read::TimeGetSystemTime,
];

/// A guest, assembled by hand into the machine it will run on.
struct Guest {
    machine: Machine,
    host: Headless,
    code: Vec<(u32, Vec<u8>)>,
    finish: Option<Finish>,
}

impl Guest {
    fn new() -> Self {
        std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
        let mut host = Headless::new();
        let machine = Machine::new(&STUB, &mut host).expect("a machine");
        Self {
            machine,
            host,
            code: Vec::new(),
            finish: None,
        }
    }

    /// Code from here on is the thread that starts at `at`.
    fn thread(&mut self, at: u32) -> &mut Self {
        self.code.push((at, Vec::new()));
        self
    }

    fn emit(&mut self, bytes: &[u8]) -> &mut Self {
        self.code.last_mut().expect("a thread").1.extend(bytes);
        self
    }

    fn emit32(&mut self, op: &[u8], value: u32) -> &mut Self {
        self.emit(op).emit(&value.to_le_bytes())
    }

    /// Where the next instruction goes.
    fn here(&self) -> u32 {
        let (at, bytes) = self.code.last().expect("a thread");
        at + u32::try_from(bytes.len()).expect("small")
    }

    fn push(&mut self, args: &[Arg]) -> &mut Self {
        for arg in args.iter().rev() {
            match *arg {
                Imm(value) => self.emit32(&[0x68], value),
                At(at) => self.emit32(&[0xff, 0x35], at),
            };
        }
        self
    }

    /// A stdcall through the boundary: arguments pushed last first, and the
    /// answer left in `eax`.
    fn call(&mut self, dll: &str, name: &str, args: &[Arg]) -> &mut Self {
        self.push(args);
        let to = self.machine.boundary.bind_named(dll, name);
        self.emit32(&[0xb8], to).emit(&[0xff, 0xd0]) // mov eax, to; call eax
    }

    /// A COM method on the object whose pointer is the word at `object`:
    /// the object first, then its arguments, through its vtable's `slot`.
    fn method(&mut self, object: u32, slot: u8, args: &[Arg]) -> &mut Self {
        self.push(args);
        self.emit32(&[0xff, 0x35], object) // push dword [object]
            .emit32(&[0xa1], object) // mov eax, [object]
            .emit(&[0x8b, 0x00]) // mov eax, [eax]
            .emit(&[0xff, 0x50, slot * 4]) // call [eax + slot * 4]
    }

    /// The clock, as each read gives it, into `eax`: milliseconds, or the
    /// performance counter's microseconds.
    fn read(&mut self, read: Read) -> &mut Self {
        match read {
            Read::QueryPerformanceCounter => self
                .call("KERNEL32.dll", "QueryPerformanceCounter", &[Imm(word(2))])
                .emit32(&[0xa1], word(2)),
            Read::GetTickCount => self.call("KERNEL32.dll", "GetTickCount", &[]),
            Read::TimeGetTime => self.call("WINMM.dll", "timeGetTime", &[]),
            Read::TimeGetSystemTime => self
                .call("WINMM.dll", "timeGetSystemTime", &[Imm(word(4)), Imm(12)])
                .emit32(&[0xa1], word(5)),
        }
    }

    /// Wait `millis` declared milliseconds by asking the clock again and
    /// again, with `also` in every pass, as a title waits for its next movie
    /// frame.
    fn wait(&mut self, read: Read, millis: u32, also: &dyn Fn(&mut Self)) -> &mut Self {
        let units = match read {
            Read::QueryPerformanceCounter => millis * 1000,
            _ => millis,
        };
        self.read(read)
            .emit32(&[0x05], units) // add eax, units
            .emit32(&[0xa3], UNTIL); // mov [UNTIL], eax
        let top = self.here();
        also(self);
        self.read(read).emit32(&[0x3b, 0x05], UNTIL); // cmp eax, [UNTIL]
        let back = top.wrapping_sub(self.here() + 2);
        assert!(back >= 0xffff_ff80, "a short way back");
        self.emit(&[0x72, back as u8]) // jb top
    }

    fn exit(&mut self) -> &mut Self {
        self.call("KERNEL32.dll", "ExitProcess", &[Imm(0)])
    }

    /// Write the code in and run it from the main thread's first instruction,
    /// within `budget`, as a route's run runs it or traced.
    fn go(&mut self, trace: Trace, budget: u64) -> &mut Self {
        for (at, bytes) in &self.code {
            self.machine.arena.write(*at, bytes).expect("the code fits");
        }
        self.machine.cpu.eip = self.code[0].0;
        self.machine.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
        let mut log = RunLog::create("stub", trace).expect("a run directory");
        self.finish = Some(self.machine.run(&mut self.host, &mut log, budget));
        self
    }

    /// Write the code in and step it one instruction or call at a time, as
    /// the debugger steps, to its end.
    fn step_every_instruction(&mut self) -> &mut Self {
        for (at, bytes) in &self.code {
            self.machine.arena.write(*at, bytes).expect("the code fits");
        }
        self.machine.cpu.eip = self.code[0].0;
        self.machine.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
        let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
        let outcome = loop {
            if let Some(outcome) = self.machine.step(&mut self.host, &mut log, u64::MAX, true) {
                break outcome;
            }
        };
        self.finish = Some(self.machine.finish(outcome));
        self
    }

    /// Instructions the machine retired in one step.
    fn passed(&self) -> u64 {
        self.machine.spins.passed()
    }
}

/// The same guest twice, and how each ended: as a route's run runs it, and
/// traced, which runs every pass.
fn twice(budget: u64, guest: &dyn Fn(&mut Guest)) -> (Guest, Guest) {
    let mut run = Guest::new();
    guest(&mut run);
    run.go(Trace::Off, budget);
    let mut every = Guest::new();
    guest(&mut every);
    every.go(Trace::Shims, budget);
    assert_eq!(every.passed(), 0, "a traced run runs every pass");
    (run, every)
}

/// The two ended as one machine: the same cursor, declared clock and flips,
/// the same registers in every thread, the same memory, and the same mix.
fn assert_one_machine(run: &Guest, every: &Guest) {
    assert_eq!(run.finish, every.finish, "the same end");
    let threads = |g: &Guest| -> Vec<Cpu> {
        std::iter::once(g.machine.cpu)
            .chain(g.machine.state.threads.iter().map(|t| t.cpu))
            .collect()
    };
    let (ours, theirs) = (threads(run), threads(every));
    assert_eq!(ours.len(), theirs.len(), "as many threads");
    for (a, b) in ours.iter().zip(&theirs) {
        // Every register, the floating-point unit's by its bits: `==` on a
        // float calls no NaN equal to itself.
        let rest = |c: &Cpu| Cpu {
            fpu: X87::new(),
            ..*c
        };
        assert_eq!(rest(a), rest(b), "the same registers");
        assert!(a.fpu.same_bits(&b.fpu), "the same floating-point unit");
    }
    let data = |g: &Guest| -> Vec<u32> {
        (0..64)
            .map(|n| g.machine.arena.read_u32(word(n)).expect("in the arena"))
            .collect()
    };
    assert_eq!(data(run), data(every), "the guest's own words");
    assert_eq!(
        run.machine.state_digest(),
        every.machine.state_digest(),
        "every page the guest can write"
    );
    assert_eq!(run.host.audio_blocks, every.host.audio_blocks, "the mix");
    assert_eq!(
        run.host.audio_flips, every.host.audio_flips,
        "handed over at the same flips"
    );
}

#[test]
fn a_clock_spin_is_passed_in_one_step_and_lands_where_running_every_pass_lands() {
    for read in READS {
        let (run, every) = twice(50_000_000, &|g| {
            g.thread(MAIN).wait(read, 2, &|_| {}).exit();
        });
        assert_eq!(
            run.finish.as_ref().map(|f| &f.outcome),
            Some(&Outcome::Exited { code: 0 }),
            "{read:?}: the wait ends"
        );
        assert_one_machine(&run, &every);
        assert!(run.passed() > 0, "{read:?}: the spin is passed in one step");
    }
}

/// What changes from one pass to the next makes each pass a different one,
/// so a spin with any other effect in it is never passed, and still lands
/// where running every pass lands.
fn never_passed_with(also: &dyn Fn(&mut Guest)) {
    let (run, every) = twice(50_000_000, &|g| {
        g.thread(MAIN)
            .wait(Read::QueryPerformanceCounter, 2, also)
            .exit();
    });
    assert_eq!(run.passed(), 0, "never passed");
    assert_one_machine(&run, &every);
}

#[test]
fn a_spin_that_writes_memory_every_pass_is_never_passed() {
    never_passed_with(&|g| {
        g.emit32(&[0xff, 0x05], word(1)); // inc dword [word 1]
    });
}

#[test]
fn a_spin_that_changes_a_register_every_pass_is_never_passed() {
    never_passed_with(&|g| {
        g.emit(&[0x43]); // inc ebx
    });
}

#[test]
fn a_spin_that_keeps_the_time_stamp_counter_is_never_passed() {
    never_passed_with(&|g| {
        g.emit(&[0x0f, 0x31, 0x89, 0xc6]); // rdtsc; mov esi, eax
    });
}

/// An input poll in the pass is another crossing, and a crossing ends a spin:
/// what a poll answers is not the clock's.
#[test]
fn a_spin_that_polls_for_input_every_pass_is_never_passed() {
    never_passed_with(&|g| {
        g.call(
            "USER32.dll",
            "PeekMessageA",
            &[Imm(word(8)), Imm(0), Imm(0), Imm(0), Imm(0)],
        );
    });
}

/// With another thread that can run, the passes are not the spinning
/// thread's alone: running each gives the other a turn.
#[test]
fn a_spin_beside_a_thread_that_can_run_is_never_passed() {
    let (run, every) = twice(50_000_000, &|g| {
        g.thread(MAIN)
            .call(
                "KERNEL32.dll",
                "CreateThread",
                &[Imm(0), Imm(0), Imm(SECOND), Imm(0), Imm(0), Imm(0)],
            )
            .wait(Read::QueryPerformanceCounter, 2, &|_| {})
            .exit()
            // It counts in a register and never crosses: only its turns move it.
            .thread(SECOND)
            .emit(&[0x41, 0xeb, 0xfd]); // inc ecx; jmp back to the inc
    });
    assert_eq!(run.passed(), 0, "never passed");
    assert_one_machine(&run, &every);
}

/// A budget that runs out inside a spin ends on the instruction running every
/// pass ends on, whether that falls on a read or inside a pass.
#[test]
fn a_budget_that_ends_inside_a_spin_ends_on_the_same_instruction() {
    for budget in [1_500_000, 1_234_567] {
        let (run, every) = twice(budget, &|g| {
            g.thread(MAIN).wait(Read::GetTickCount, 5, &|_| {}).exit();
        });
        let finish = run.finish.as_ref().expect("it ran");
        assert_eq!(finish.outcome, Outcome::BudgetExhausted);
        assert_eq!(finish.cursor, budget, "it stops on the budget");
        assert!(run.passed() > 0, "it passed what it could before it");
        assert_one_machine(&run, &every);
    }
}

/// A stepped run, as the debugger steps one, stops at every instruction, so
/// it passes nothing, and it lands where the route's run lands.
#[test]
fn a_stepped_run_runs_every_pass() {
    let guest = |g: &mut Guest| {
        g.thread(MAIN)
            .wait(Read::QueryPerformanceCounter, 2, &|_| {})
            .exit();
    };
    let mut run = Guest::new();
    guest(&mut run);
    run.go(Trace::Off, 50_000_000);
    let mut stepped = Guest::new();
    guest(&mut stepped);
    stepped.step_every_instruction();
    assert_eq!(stepped.passed(), 0, "the debugger sees every pass");
    assert!(run.passed() > 0);
    assert_one_machine(&run, &stepped);
}

/// Where a pass is passed, the mix is what running every pass mixes: a
/// looping sound plays through the spin, and the machine hands the host the
/// same blocks at the same flips.
#[test]
fn a_spin_while_a_sound_plays_mixes_what_running_every_pass_mixes() {
    // IDirectSound and IDirectSoundBuffer, in slot order.
    const CREATE_SOUND_BUFFER: u8 = 3;
    const LOCK: u8 = 11;
    const PLAY: u8 = 12;
    const UNLOCK: u8 = 19;
    // 1,024 frames of 16-bit stereo, about 46 ms at the mix's rate: a ramp,
    // played round and round through the 120 ms the guest waits.
    const BYTES: u32 = 4096;
    const RAMP: u32 = 0x0050_1000;
    let (desc, format) = (word(32), word(40));
    let (sound, buffer, at, len) = (word(10), word(11), word(12), word(13));
    let (run, every) = twice(200_000_000, &|g| {
        // DSBUFFERDESC: its size, no flags, its bytes, and its format, a
        // WAVEFORMATEX of PCM, two channels, 22,050 Hz, sixteen bits.
        for (n, value) in [20, 0, BYTES, 0, format].into_iter().enumerate() {
            g.machine
                .arena
                .write_u32(desc + n as u32 * 4, value)
                .expect("in the arena");
        }
        let wave: [u8; 18] = [
            1, 0, 2, 0, 0x22, 0x56, 0, 0, 0x88, 0x58, 1, 0, 4, 0, 16, 0, 0, 0,
        ];
        g.machine.arena.write(format, &wave).expect("in the arena");
        let ramp: Vec<u8> = (0..BYTES / 2)
            .flat_map(|i| ((i as i16).wrapping_mul(37)).to_le_bytes())
            .collect();
        g.machine.arena.write(RAMP, &ramp).expect("in the arena");
        g.thread(MAIN)
            .call(
                "DSOUND.dll",
                "DirectSoundCreate",
                &[Imm(0), Imm(sound), Imm(0)],
            )
            .method(
                sound,
                CREATE_SOUND_BUFFER,
                &[Imm(desc), Imm(buffer), Imm(0)],
            )
            .method(
                buffer,
                LOCK,
                &[
                    Imm(0),
                    Imm(BYTES),
                    Imm(at),
                    Imm(len),
                    Imm(0),
                    Imm(0),
                    Imm(0),
                ],
            )
            .emit32(&[0xbe], RAMP) // mov esi, RAMP
            .emit32(&[0x8b, 0x3d], at) // mov edi, [at]
            .emit32(&[0xb9], BYTES / 4) // mov ecx, BYTES / 4
            .emit(&[0xf3, 0xa5]) // rep movsd
            .method(buffer, UNLOCK, &[At(at), At(len), Imm(0), Imm(0)])
            .method(buffer, PLAY, &[Imm(0), Imm(0), Imm(1)])
            // A pass of about two thousand instructions, so that the traced
            // run logs a few hundred reads a millisecond rather than tens of
            // thousands.
            .wait(Read::QueryPerformanceCounter, 120, &|g| {
                g.emit32(&[0xb9], 1000).emit(&[0x49, 0x75, 0xfd]); // mov ecx, 1000; dec ecx; jnz
            })
            .exit();
    });
    assert!(
        run.host.audio_blocks.len() >= 2 && run.host.audio_blocks.iter().flatten().any(|s| *s != 0),
        "the sound is heard through the spin"
    );
    assert!(run.passed() > 0, "the spin is passed");
    assert_one_machine(&run, &every);
}

/// A pass that reads the time stamp counter reads a time the clock's
/// millisecond does not cover: two passes may go one way and a later one,
/// in the same millisecond, the other. So it is never passed, even when what
/// it read is not kept.
#[test]
fn a_spin_that_branches_on_the_time_stamp_counter_is_never_passed() {
    never_passed_with(&|g| {
        // rdtsc; test eax, 0x100; jz past; inc dword [word 1]; past:
        g.emit(&[0x0f, 0x31, 0xa9])
            .emit(&0x100u32.to_le_bytes())
            .emit(&[0x74, 0x06])
            .emit32(&[0xff, 0x05], word(1));
    });
}

/// A spin whose pass is longer than the scheduler's quantum has bursts that
/// end inside it, where a thread woken by the clock takes its turn; passed,
/// they would be gone, so it is never passed.
#[test]
fn a_spin_longer_than_the_quantum_beside_a_sleeping_thread_is_never_passed() {
    let (run, every) = twice(50_000_000, &|g| {
        g.thread(MAIN)
            .call(
                "KERNEL32.dll",
                "CreateThread",
                &[Imm(0), Imm(0), Imm(SECOND), Imm(0), Imm(0), Imm(0)],
            )
            .wait(Read::GetTickCount, 5, &|g| {
                g.emit32(&[0xb9], 35_000).emit(&[0x49, 0x75, 0xfd]); // mov ecx, 35000; dec ecx; jnz
            })
            .exit();
        sleeper(g);
    });
    assert_eq!(run.passed(), 0, "never passed");
    assert_one_machine(&run, &every);
}

/// A thread asleep while the spin is passed wakes on the same instruction it
/// wakes on when every pass is run: the read that sees its millisecond.
#[test]
fn a_thread_that_wakes_in_a_passed_spin_wakes_on_the_same_instruction() {
    let (run, every) = twice(50_000_000, &|g| {
        g.thread(MAIN)
            .call(
                "KERNEL32.dll",
                "CreateThread",
                &[Imm(0), Imm(0), Imm(SECOND), Imm(0), Imm(0), Imm(0)],
            )
            .wait(Read::GetTickCount, 5, &|_| {})
            .exit();
        sleeper(g);
    });
    assert!(run.passed() > 0, "passed while the other sleeps");
    assert!(
        run.machine.arena.read_u32(word(1)).expect("in the arena") >= 3,
        "and it woke"
    );
    assert_one_machine(&run, &every);
}

/// A NaN anywhere in the floating-point unit is a register like any other,
/// the same bit for bit from one pass to the next.
#[test]
fn a_spin_after_a_nan_is_passed() {
    let (run, every) = twice(50_000_000, &|g| {
        g.thread(MAIN)
            .emit(&[0xd9, 0xee, 0xd9, 0xee, 0xde, 0xf9]) // fldz; fldz; fdivp: 0 / 0
            .wait(Read::QueryPerformanceCounter, 2, &|_| {})
            .exit();
    });
    assert!(run.passed() > 0, "passed");
    assert_one_machine(&run, &every);
}

/// The second thread: it sleeps a millisecond and counts that it woke, for ever.
fn sleeper(g: &mut Guest) {
    g.thread(SECOND);
    let top = g.here();
    g.call("KERNEL32.dll", "Sleep", &[Imm(1)])
        .emit32(&[0xff, 0x05], word(1)); // inc dword [word 1]
    let back = top.wrapping_sub(g.here() + 2);
    g.emit(&[0xeb, back as u8]); // jmp top
}
