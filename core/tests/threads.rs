//! Guest threads, without a title: counted semaphores, waits that park and
//! are woken, and `Sleep(0)` giving the turn away (ADR-0005, #115).
//!
//! Each guest is a few hand-assembled instructions a thread, as the stub's
//! are, calling KERNEL32 through the boundary the way a title's imports do,
//! and run by the loop and the scheduler every title runs on.

use lego_legacy_core::cli::Trace;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::{Finish, Machine, Outcome, STACK_TOP};
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::Title;

const STUB: Title = Title::STUB;

/// Where the main thread's code goes, and each other thread's after it.
const MAIN: u32 = 0x0040_0000;
const FIRST: u32 = 0x0040_1000;
const SECOND: u32 = 0x0040_2000;
/// Where the threads keep what they saw, a word apiece.
const DATA: u32 = 0x0050_0000;

const INFINITE: u32 = 0xffff_ffff;
const WAIT_TIMEOUT: u32 = 0x102;
const CREATE_SUSPENDED: u32 = 4;

/// A word of the guest's data.
const fn word(n: u32) -> u32 {
    DATA + n * 4
}

#[derive(Clone, Copy)]
enum Arg {
    Imm(u32),
    /// The word at this address, as `push dword [at]`.
    At(u32),
}
use Arg::{At, Imm};

/// A guest, assembled by hand into the machine it will run on.
struct Guest {
    machine: Machine,
    host: Headless,
    /// Each thread's code, from where it starts.
    code: Vec<(u32, Vec<u8>)>,
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

    /// A stdcall into KERNEL32, through the boundary: arguments pushed last
    /// first, and the answer left in `eax`.
    fn call(&mut self, name: &str, args: &[Arg]) -> &mut Self {
        for arg in args.iter().rev() {
            match *arg {
                Imm(value) => self.emit32(&[0x68], value),
                At(at) => self.emit32(&[0xff, 0x35], at),
            };
        }
        let to = self.machine.boundary.bind_named("KERNEL32.dll", name);
        self.emit32(&[0xb8], to).emit(&[0xff, 0xd0]) // mov eax, to; call eax
    }

    /// `CreateThread`, of a thread that starts at `start`.
    fn spawn(&mut self, start: u32, flags: u32) -> &mut Self {
        self.call(
            "CreateThread",
            &[Imm(0), Imm(0), Imm(start), Imm(0), Imm(flags), Imm(0)],
        )
    }

    /// `mov [at], eax`.
    fn store(&mut self, at: u32) -> &mut Self {
        self.emit32(&[0xa3], at)
    }

    /// `mov eax, [at]`.
    fn load(&mut self, at: u32) -> &mut Self {
        self.emit32(&[0xa1], at)
    }

    /// `mov dword [at], value`.
    fn set(&mut self, at: u32, value: u32) -> &mut Self {
        self.emit32(&[0xc7, 0x05], at).emit(&value.to_le_bytes())
    }

    /// `inc dword [at]`.
    fn inc(&mut self, at: u32) -> &mut Self {
        self.emit32(&[0xff, 0x05], at)
    }

    /// `dec dword [at]` and back to `to` until it is zero.
    fn dec_and_loop(&mut self, at: u32, to: u32) -> &mut Self {
        self.emit32(&[0xff, 0x0d], at);
        let back = to.wrapping_sub(self.here() + 2) as u8;
        self.emit(&[0x75, back])
    }

    /// `jmp to`, a short way back.
    fn jump(&mut self, to: u32) -> &mut Self {
        let back = to.wrapping_sub(self.here() + 2) as u8;
        self.emit(&[0xeb, back])
    }

    /// Write the code in and run it from the main thread's first instruction.
    fn run(&mut self) -> Finish {
        for (at, bytes) in &self.code {
            self.machine.arena.write(*at, bytes).expect("the code fits");
        }
        self.machine.cpu.eip = self.code[0].0;
        self.machine.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
        let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
        self.machine.run(&mut self.host, &mut log, 50_000_000)
    }

    fn read(&self, at: u32) -> u32 {
        self.machine.arena.read_u32(at).expect("in the arena")
    }
}

#[test]
fn a_semaphore_answers_as_many_waits_as_its_count_and_no_more_than_its_maximum() {
    let (sem, before) = (word(0), word(1));
    let mut guest = Guest::new();
    guest
        .thread(MAIN)
        // A second thread, so that a wait is a wait (ADR-0005). It never runs.
        .spawn(FIRST, CREATE_SUSPENDED)
        .call("CreateSemaphoreA", &[Imm(0), Imm(1), Imm(2), Imm(0)])
        .store(sem)
        .call("WaitForSingleObject", &[At(sem), Imm(0)])
        .store(word(10))
        .call("WaitForSingleObject", &[At(sem), Imm(0)])
        .store(word(11))
        .call("ReleaseSemaphore", &[At(sem), Imm(2), Imm(before)])
        .store(word(12))
        .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(word(2))])
        .store(word(13))
        .call("GetLastError", &[])
        .store(word(14))
        .call("ReleaseSemaphore", &[At(sem), Imm(0), Imm(0)])
        .store(word(15))
        .call("GetLastError", &[])
        .store(word(16))
        .call("CreateSemaphoreA", &[Imm(0), Imm(3), Imm(2), Imm(0)])
        .store(word(17))
        .call("GetLastError", &[])
        .store(word(18))
        .call("WaitForSingleObject", &[At(sem), Imm(0)])
        .store(word(19))
        .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(word(3))])
        .call("ExitProcess", &[Imm(0)])
        .thread(FIRST)
        .emit(&[0xc3]);
    guest
        .machine
        .arena
        .write_u32(word(2), 0xaaaa_aaaa)
        .expect("in the arena");
    assert_eq!(guest.run().outcome, Outcome::Exited { code: 0 });

    assert_ne!(guest.read(sem), 0, "made with one of two");
    assert_eq!(guest.read(word(10)), 0, "the first wait takes the one");
    assert_eq!(guest.read(word(11)), WAIT_TIMEOUT, "the second finds none");
    assert_eq!(guest.read(word(12)), 1, "two released, to its maximum");
    assert_eq!(guest.read(before), 0, "and it had none before");
    assert_eq!(guest.read(word(13)), 0, "one more is past its maximum");
    assert_eq!(guest.read(word(14)), 298, "ERROR_TOO_MANY_POSTS");
    assert_eq!(
        guest.read(word(2)),
        0xaaaa_aaaa,
        "a release that fails writes no count back"
    );
    assert_eq!(guest.read(word(15)), 0, "a release of none fails");
    assert_eq!(guest.read(word(16)), 87, "ERROR_INVALID_PARAMETER");
    assert_eq!(
        guest.read(word(17)),
        0,
        "more to start with than its maximum"
    );
    assert_eq!(guest.read(word(18)), 87, "ERROR_INVALID_PARAMETER");
    assert_eq!(
        guest.read(word(19)),
        0,
        "the refused release changed nothing"
    );
    assert_eq!(guest.read(word(3)), 1, "and the wait took one of the two");
}

#[test]
fn a_wait_on_a_semaphore_parks_until_another_thread_releases_it() {
    let (sem, mark) = (word(0), word(1));
    let mut guest = Guest::new();
    guest
        .thread(MAIN)
        .call("CreateSemaphoreA", &[Imm(0), Imm(0), Imm(1), Imm(0)])
        .store(sem)
        .spawn(FIRST, 0)
        .call("GetTickCount", &[])
        .store(word(10))
        .call("WaitForSingleObject", &[At(sem), Imm(INFINITE)])
        .store(word(11))
        .load(mark)
        .store(word(12))
        .call("GetTickCount", &[])
        .store(word(13))
        .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(word(14))])
        .call("ExitProcess", &[Imm(0)])
        // The releaser sleeps first, so the main thread is at its wait long
        // before anything is there to take.
        .thread(FIRST)
        .call("Sleep", &[Imm(5)])
        .set(mark, 1)
        .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(0)])
        .store(word(15))
        .emit(&[0xc3]);
    assert_eq!(guest.run().outcome, Outcome::Exited { code: 0 });

    assert_eq!(guest.read(word(15)), 1, "the release");
    assert_eq!(guest.read(word(11)), 0, "the wait was answered by it");
    assert_eq!(guest.read(word(12)), 1, "after the releaser's mark");
    assert!(
        guest.read(word(13)) - guest.read(word(10)) >= 5,
        "the waiter stayed down until the releaser's hour"
    );
    assert_eq!(guest.read(word(14)), 0, "and the wait took the count");
}

/// Two waiters parked on one semaphore. One release wakes both, and the one
/// next in the machine's order takes it; the other goes back down until the
/// second release, rather than being told its endless wait ran out.
fn two_waiters() -> Guest {
    let (sem, woken) = (word(0), word(1));
    let mut guest = Guest::new();
    guest
        .thread(MAIN)
        .call("CreateSemaphoreA", &[Imm(0), Imm(0), Imm(2), Imm(0)])
        .store(sem)
        .spawn(FIRST, 0)
        .spawn(SECOND, 0)
        .call("Sleep", &[Imm(1)])
        .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(0)])
        .call("Sleep", &[Imm(1)])
        .load(woken)
        .store(word(10))
        .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(0)])
        .call("Sleep", &[Imm(1)])
        .load(woken)
        .store(word(11))
        .call("ExitProcess", &[Imm(0)]);
    for (n, at) in [(0, FIRST), (1, SECOND)] {
        guest
            .thread(at)
            .call("WaitForSingleObject", &[At(sem), Imm(INFINITE)])
            .store(word(20 + n))
            .inc(woken)
            .load(woken)
            .store(word(22 + n))
            .emit(&[0xc3]);
    }
    guest
}

#[test]
fn one_release_wakes_one_waiter_in_the_declared_order_and_the_other_waits_on() {
    let mut guest = two_waiters();
    assert_eq!(guest.run().outcome, Outcome::Exited { code: 0 });

    assert_eq!(guest.read(word(10)), 1, "one release, one waiter through");
    assert_eq!(guest.read(word(11)), 2, "the second release, the second");
    assert_eq!(
        (guest.read(word(22)), guest.read(word(23))),
        (1, 2),
        "the first thread made is the first after the releaser"
    );
    assert_eq!(
        (guest.read(word(20)), guest.read(word(21))),
        (0, 0),
        "both waits were answered, neither ran out"
    );
}

#[test]
fn sleep_0_hands_the_rest_of_the_turn_to_the_next_thread_that_can_run() {
    let count = word(0);
    let mut guest = Guest::new();
    guest
        .thread(MAIN)
        .spawn(FIRST, 0)
        .load(count)
        .store(word(10))
        .call("Sleep", &[Imm(0)])
        .load(count)
        .store(word(11))
        .call("ExitProcess", &[Imm(0)]);
    // The other thread counts its turns: each is a call, and one past it.
    let top = FIRST;
    guest
        .thread(FIRST)
        .call("GetCurrentThreadId", &[])
        .inc(count)
        .jump(top);
    let finish = guest.run();
    assert_eq!(finish.outcome, Outcome::Exited { code: 0 });

    assert_eq!(
        guest.read(word(11)) - guest.read(word(10)),
        1,
        "the other thread had a whole turn before Sleep(0) came back"
    );
    assert_eq!(finish.declared_millis, 0, "and no declared time went by");
}

/// Two threads taking turns by `Sleep(0)`, each writing its letter, and a
/// semaphore that says the second is done.
fn taking_turns() -> Guest {
    let (sem, at, left) = (word(0), word(1), word(2));
    let letters = word(16);
    let mut guest = Guest::new();
    guest
        .thread(MAIN)
        .call("CreateSemaphoreA", &[Imm(0), Imm(0), Imm(1), Imm(0)])
        .store(sem)
        .spawn(FIRST, 0);
    for (thread, letter, left) in [(MAIN, b'M', left), (FIRST, b'T', left + 4)] {
        if thread != MAIN {
            guest.thread(thread);
        }
        guest.set(left, 4);
        let top = guest.here();
        // mov eax, [at]; mov byte [eax + letters], letter; inc dword [at]
        guest
            .load(at)
            .emit32(&[0xc6, 0x80], letters)
            .emit(&[letter])
            .inc(at)
            .call("Sleep", &[Imm(0)])
            .dec_and_loop(left, top);
        if thread == MAIN {
            guest
                .call("WaitForSingleObject", &[At(sem), Imm(INFINITE)])
                .call("ExitProcess", &[Imm(0)]);
        } else {
            guest
                .call("ReleaseSemaphore", &[At(sem), Imm(1), Imm(0)])
                .emit(&[0xc3]);
        }
    }
    guest
}

#[test]
fn threads_take_their_turns_in_the_same_order_every_run() {
    let mut runs = [taking_turns(), taking_turns()];
    let finishes = runs.each_mut().map(Guest::run);
    let letters = runs.each_ref().map(|guest| {
        let mut out = [0; 8];
        guest
            .machine
            .arena
            .read(word(16), &mut out)
            .expect("in the arena");
        out
    });

    assert_eq!(finishes[0].outcome, Outcome::Exited { code: 0 });
    assert_eq!(
        &letters[0], b"MTMTMTMT",
        "one turn each, in the declared order"
    );
    assert_eq!(letters[0], letters[1]);
    assert_eq!(finishes[0], finishes[1]);
    assert_eq!(
        runs[0].machine.state_digest(),
        runs[1].machine.state_digest()
    );
}
