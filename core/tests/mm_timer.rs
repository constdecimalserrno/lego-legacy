//! Deterministic `WINMM!timeSetEvent` and `WINMM!timeKillEvent` callbacks on
//! the declared clock (#371).
//!
//! Each test runs hand-assembled 32-bit x86 guest code on a real [`Machine`]
//! with [`Headless`] so stdcall argument passing, stack discipline, thread
//! isolation (`WaitForSingleObject(mutex, 0)`), declared-clock cadence,
//! self-cancellation, arming-from-callback, thread reuse, walls, and
//! `LLSNAP v2` round-trips are exercised end-to-end.

use lego_legacy_core::cli::Trace;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::{Machine, Outcome, STACK_TOP};
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::Title;

const STUB: Title = Title::STUB;

const MAIN: u32 = 0x0040_0000;
const CALLBACK: u32 = 0x0040_1000;
const DATA: u32 = 0x0050_0000;

const TIMER_A: u32 = DATA;
const TIMER_B: u32 = DATA + 4;
const MUTEX: u32 = DATA + 8;
const COUNT: u32 = DATA + 12;
const SELF_KILL_AT: u32 = DATA + 16;
const ANSWERS: u32 = DATA + 0x40;
/// Each callback log entry is 32 bytes:
/// `[uTimerID, uMsg, dwUser, dw1, dw2, timeGetTime(), thread_id, wait_result]`.
const LOG: u32 = DATA + 0x100;
const LOG_STRIDE: u32 = 32;

const TIME_ONESHOT: u32 = 0x0000;
const TIME_PERIODIC: u32 = 0x0001;
const TIME_KILL_SYNCHRONOUS: u32 = 0x0100;
const TIMERR_NOERROR: u32 = 0;
const MMSYSERR_INVALPARAM: u32 = 11;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x0102;

#[derive(Clone, Copy)]
enum Arg {
    Imm(u32),
    At(u32),
}
use Arg::{At, Imm};

struct Code {
    at: u32,
    bytes: Vec<u8>,
    next: u32,
}

impl Code {
    fn new(at: u32) -> Self {
        Self {
            at,
            bytes: Vec::new(),
            next: ANSWERS,
        }
    }

    fn emit(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    fn emit32(&mut self, op: &[u8], value: u32) -> &mut Self {
        self.emit(op).emit(&value.to_le_bytes())
    }

    fn call(&mut self, m: &mut Machine, dll: &str, name: &str, args: &[Arg]) -> &mut Self {
        for arg in args.iter().rev() {
            match *arg {
                Imm(v) => self.emit32(&[0x68], v),
                At(a) => self.emit32(&[0xff, 0x35], a),
            };
        }
        let to = m.boundary.bind_named(dll, name);
        self.emit32(&[0xb8], to).emit(&[0xff, 0xd0])
    }

    fn store(&mut self, at: u32) -> &mut Self {
        self.emit32(&[0xa3], at)
    }

    fn answer(&mut self) -> &mut Self {
        self.next += 4;
        self.store(self.next - 4)
    }

    fn jump(&mut self, op: u8) -> usize {
        self.emit(&[op, 0]);
        self.bytes.len() - 1
    }

    fn land(&mut self, hole: usize) -> &mut Self {
        self.bytes[hole] = u8::try_from(self.bytes.len() - (hole + 1)).expect("short jump");
        self
    }
}

/// Standard `LPTIMECALLBACK(uTimerID, uMsg, dwUser, dw1, dw2)` (`ret 0x14`):
/// records all five stdcall arguments, `timeGetTime()`, `GetCurrentThreadId()`,
/// and (when `[MUTEX] != 0`) the non-blocking `WaitForSingleObject([MUTEX], 0)`
/// result (immediately releasing the mutex if acquired). When `[SELF_KILL_AT]`
/// matches the new 1-based call count, kills `uTimerID` from inside itself.
fn build_default_callback(m: &mut Machine) -> Code {
    let mut c = Code::new(CALLBACK);
    c.emit(&[0x53, 0x56]) // push ebx; push esi (esp shifted by +8)
        .emit32(&[0x8b, 0x1d], COUNT) // mov ebx, [COUNT]
        .emit(&[0xc1, 0xe3, 0x05]) // shl ebx, 5 (* 32)
        .emit32(&[0x81, 0xc3], LOG); // add ebx, LOG
    for i in 0u8..5 {
        let stack_off = 12 + i * 4;
        c.emit(&[0x8b, 0x44, 0x24, stack_off])
            .emit(&[0x89, 0x43, i * 4]);
    }
    c.call(m, "WINMM.dll", "timeGetTime", &[])
        .emit(&[0x89, 0x43, 20])
        .call(m, "KERNEL32.dll", "GetCurrentThreadId", &[])
        .emit(&[0x89, 0x43, 24]);

    // Optional non-blocking mutex probe (mirrors LEGORacers.exe 0x004bd5e0).
    c.emit32(&[0x8b, 0x35], MUTEX).emit(&[0x85, 0xf6]);
    let no_mutex = c.jump(0x74);
    c.emit(&[0x6a, 0x00, 0x56])
        .call(m, "KERNEL32.dll", "WaitForSingleObject", &[])
        .emit(&[0x89, 0x43, 28])
        .emit(&[0x85, 0xc0]);
    let not_acquired = c.jump(0x75);
    c.emit(&[0x56])
        .call(m, "KERNEL32.dll", "ReleaseMutex", &[])
        .land(not_acquired)
        .land(no_mutex);

    c.emit32(&[0xff, 0x05], COUNT)
        .emit32(&[0xa1], SELF_KILL_AT)
        .emit(&[0x85, 0xc0]);
    let no_kill = c.jump(0x74);
    c.emit32(&[0x3b, 0x05], COUNT);
    let skip_kill = c.jump(0x75);
    c.emit(&[0xff, 0x74, 0x24, 0x0c])
        .call(m, "WINMM.dll", "timeKillEvent", &[])
        .land(skip_kill)
        .land(no_kill)
        .emit(&[0x5e, 0x5b, 0xc2, 0x14, 0x00]);
    c
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    id: u32,
    msg: u32,
    user: u32,
    dw1: u32,
    dw2: u32,
    time_ms: u32,
    thread_id: u32,
    wait_res: u32,
}

fn read_entries(m: &Machine) -> Vec<Entry> {
    let word = |at: u32| m.arena.read_u32(at).expect("in arena");
    let count = word(COUNT);
    (0..count)
        .map(|i| {
            let base = LOG + i * LOG_STRIDE;
            Entry {
                id: word(base),
                msg: word(base + 4),
                user: word(base + 8),
                dw1: word(base + 12),
                dw2: word(base + 16),
                time_ms: word(base + 20),
                thread_id: word(base + 24),
                wait_res: word(base + 28),
            }
        })
        .collect()
}

fn build_guest(
    setup_main: impl FnOnce(&mut Machine, &mut Code),
    setup_cb: impl FnOnce(&mut Machine) -> Code,
) -> (Machine, Headless) {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    let mut host = Headless::new();
    let mut m = Machine::new(&STUB, &mut host).expect("machine");
    m.state.claimed.push((MAIN, DATA + 0x1000));
    let mut main = Code::new(MAIN);
    setup_main(&mut m, &mut main);
    let cb = setup_cb(&mut m);
    m.arena.write(main.at, &main.bytes).expect("write main");
    m.arena.write(cb.at, &cb.bytes).expect("write callback");
    m.cpu.eip = MAIN;
    m.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
    (m, host)
}

fn run_guest_with(
    setup_main: impl FnOnce(&mut Machine, &mut Code),
    setup_cb: impl FnOnce(&mut Machine) -> Code,
) -> (Machine, Outcome) {
    let (mut m, mut host) = build_guest(setup_main, setup_cb);
    let mut log = RunLog::create("mm-timer-test", Trace::Off).expect("log");
    let dir = log.dir().to_path_buf();
    let finish = m.run(&mut host, &mut log, 50_000_000);
    drop(log);
    let _ = std::fs::remove_dir_all(dir);
    (m, finish.outcome)
}

fn run_guest(setup: impl FnOnce(&mut Machine, &mut Code)) -> (Machine, Outcome) {
    run_guest_with(setup, build_default_callback)
}

#[test]
fn time_oneshot_fires_once_with_exact_stdcall_args_and_auto_removes() {
    let (m, outcome) = run_guest(|m, main| {
        main.call(m, "KERNEL32.dll", "GetCurrentThreadId", &[])
            .answer()
            .call(m, "WINMM.dll", "timeGetTime", &[])
            .answer()
            .call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[
                    Imm(75),
                    Imm(10),
                    Imm(CALLBACK),
                    Imm(0x1234_5678),
                    Imm(TIME_ONESHOT),
                ],
            )
            .store(TIMER_A)
            .answer()
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(250)])
            .call(m, "WINMM.dll", "timeKillEvent", &[At(TIMER_A)])
            .answer()
            .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
    });

    assert_eq!(outcome, Outcome::Exited { code: 0 });
    let word = |at: u32| m.arena.read_u32(at).expect("word");
    let main_tid = word(ANSWERS);
    let t0 = word(ANSWERS + 4);
    let id = word(ANSWERS + 8);
    let late_kill = word(ANSWERS + 12);

    assert_ne!(id, 0);
    assert_eq!(
        late_kill, MMSYSERR_INVALPARAM,
        "fired one-shot timer is auto-removed"
    );

    let entries = read_entries(&m);
    assert_eq!(entries.len(), 1, "TIME_ONESHOT fires exactly once");
    assert_eq!(
        entries[0],
        Entry {
            id,
            msg: 0,
            user: 0x1234_5678,
            dw1: 0,
            dw2: 0,
            time_ms: t0 + 75,
            thread_id: entries[0].thread_id,
            wait_res: 0,
        }
    );
    assert_ne!(
        entries[0].thread_id, main_tid,
        "callback runs on WINMM's dedicated timer thread"
    );
}

#[test]
fn time_periodic_fires_on_declared_clock_cadence_stops_on_kill_and_reuses_thread() {
    let (m, outcome) = run_guest(|m, main| {
        main.call(m, "WINMM.dll", "timeGetTime", &[])
            .answer()
            .call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[
                    Imm(100),
                    Imm(100),
                    Imm(CALLBACK),
                    Imm(0xabcd_0001),
                    Imm(TIME_PERIODIC | TIME_KILL_SYNCHRONOUS),
                ],
            )
            .store(TIMER_A)
            .answer()
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(350)])
            .call(m, "WINMM.dll", "timeKillEvent", &[At(TIMER_A)])
            .answer()
            .call(m, "WINMM.dll", "timeKillEvent", &[At(TIMER_A)])
            .answer()
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(150)])
            // Recreate a second timer after idle to prove thread reuse.
            .call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[
                    Imm(50),
                    Imm(10),
                    Imm(CALLBACK),
                    Imm(0xabcd_0002),
                    Imm(TIME_ONESHOT),
                ],
            )
            .store(TIMER_B)
            .answer()
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(100)])
            .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
    });

    assert_eq!(outcome, Outcome::Exited { code: 0 });
    assert_eq!(
        m.state.threads.len(),
        2,
        "timer thread is reused, never re-spawned"
    );
    let word = |at: u32| m.arena.read_u32(at).expect("word");
    let t0 = word(ANSWERS);
    let id1 = word(ANSWERS + 4);
    let first_kill = word(ANSWERS + 8);
    let second_kill = word(ANSWERS + 12);
    let id2 = word(ANSWERS + 16);

    assert_ne!(id1, 0);
    assert_ne!(id2, 0);
    assert_ne!(id1, id2);
    assert_eq!(first_kill, TIMERR_NOERROR);
    assert_eq!(second_kill, MMSYSERR_INVALPARAM);

    let entries = read_entries(&m);
    assert_eq!(entries.len(), 4, "3 periodic ticks + 1 recreated one-shot");
    let times: Vec<u32> = entries.iter().map(|e| e.time_ms).collect();
    assert_eq!(times, vec![t0 + 100, t0 + 200, t0 + 300, t0 + 550]);
    for e in &entries[..3] {
        assert_eq!(
            (e.id, e.msg, e.user, e.dw1, e.dw2),
            (id1, 0, 0xabcd_0001, 0, 0)
        );
    }
    assert_eq!(
        (entries[3].id, entries[3].user, entries[3].thread_id),
        (id2, 0xabcd_0002, entries[0].thread_id)
    );
}

#[test]
fn multiple_timers_self_kill_and_mutex_isolation_work_deterministically() {
    let (m, outcome) = run_guest(|m, main| {
        main.call(m, "KERNEL32.dll", "CreateMutexA", &[Imm(0), Imm(1), Imm(0)])
            .store(MUTEX)
            .call(m, "WINMM.dll", "timeGetTime", &[])
            .answer()
            .call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[
                    Imm(40),
                    Imm(10),
                    Imm(CALLBACK),
                    Imm(0xa),
                    Imm(TIME_PERIODIC),
                ],
            )
            .store(TIMER_A)
            .answer()
            .call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[Imm(60), Imm(10), Imm(CALLBACK), Imm(0xb), Imm(TIME_ONESHOT)],
            )
            .store(TIMER_B)
            .answer()
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(70)])
            .call(m, "KERNEL32.dll", "ReleaseMutex", &[At(MUTEX)])
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(120)])
            .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
        m.arena.write_u32(SELF_KILL_AT, 3).expect("self kill at");
    });

    assert_eq!(outcome, Outcome::Exited { code: 0 });
    let word = |at: u32| m.arena.read_u32(at).expect("word");
    let t0 = word(ANSWERS);
    let id_a = word(ANSWERS + 4);
    let id_b = word(ANSWERS + 8);

    let entries = read_entries(&m);
    assert_eq!(entries.len(), 3, "A@40, B@60, A@80 (self-kills)");
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.id, e.user, e.time_ms, e.wait_res))
            .collect::<Vec<_>>(),
        vec![
            (id_a, 0xa, t0 + 40, WAIT_TIMEOUT),
            (id_b, 0xb, t0 + 60, WAIT_TIMEOUT),
            (id_a, 0xa, t0 + 80, WAIT_OBJECT_0),
        ]
    );
}

#[test]
fn timer_callback_can_arm_a_chained_oneshot_timer() {
    let (m, outcome) = run_guest_with(
        |m, main| {
            main.call(m, "WINMM.dll", "timeGetTime", &[])
                .answer()
                .call(
                    m,
                    "WINMM.dll",
                    "timeSetEvent",
                    &[Imm(30), Imm(10), Imm(CALLBACK), Imm(1), Imm(TIME_ONESHOT)],
                )
                .answer()
                .call(m, "KERNEL32.dll", "Sleep", &[Imm(100)])
                .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
        },
        |m| {
            // Records `[user, timeGetTime()]` into LOG and, on the first fire
            // (dwUser == 1), registers a follow-up 40 ms one-shot with dwUser == 2.
            let mut c = Code::new(CALLBACK);
            c.emit(&[0x53]) // push ebx (esp + 4)
                .emit32(&[0x8b, 0x1d], COUNT)
                .emit(&[0xc1, 0xe3, 0x05])
                .emit32(&[0x81, 0xc3], LOG)
                .emit(&[0x8b, 0x44, 0x24, 16]) // dwUser
                .emit(&[0x89, 0x43, 8])
                .call(m, "WINMM.dll", "timeGetTime", &[])
                .emit(&[0x89, 0x43, 20])
                .emit32(&[0xff, 0x05], COUNT)
                .emit(&[0x83, 0x7c, 0x24, 16, 1]); // cmp dword [esp+16], 1
            let skip = c.jump(0x75);
            c.call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[Imm(40), Imm(10), Imm(CALLBACK), Imm(2), Imm(TIME_ONESHOT)],
            )
            .store(TIMER_B)
            .land(skip)
            .emit(&[0x5b, 0xc2, 0x14, 0x00]);
            c
        },
    );

    assert_eq!(outcome, Outcome::Exited { code: 0 });
    let t0 = m.arena.read_u32(ANSWERS).unwrap();
    let entries = read_entries(&m);
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.user, e.time_ms))
            .collect::<Vec<_>>(),
        vec![(1, t0 + 30), (2, t0 + 70)]
    );
}

#[test]
fn callback_blocking_or_exiting_timer_thread_hits_a_named_wall() {
    let (_, block_outcome) = run_guest_with(
        |m, main| {
            main.call(
                m,
                "KERNEL32.dll",
                "CreateEventA",
                &[Imm(0), Imm(1), Imm(0), Imm(0)],
            )
            .store(MUTEX)
            .call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[Imm(20), Imm(10), Imm(CALLBACK), Imm(0), Imm(TIME_ONESHOT)],
            )
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(100)])
            .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
        },
        |m| {
            let mut cb = Code::new(CALLBACK);
            cb.call(
                m,
                "KERNEL32.dll",
                "WaitForSingleObject",
                &[At(MUTEX), Imm(u32::MAX)],
            )
            .emit(&[0xc2, 0x14, 0x00]);
            cb
        },
    );
    assert!(
        matches!(block_outcome, Outcome::Wall { ref what } if what.contains("WINMM's timer thread")),
        "expected timer-thread blocking wall, got {block_outcome:?}"
    );

    let (_, exit_outcome) = run_guest_with(
        |m, main| {
            main.call(
                m,
                "WINMM.dll",
                "timeSetEvent",
                &[Imm(20), Imm(10), Imm(CALLBACK), Imm(0), Imm(TIME_ONESHOT)],
            )
            .call(m, "KERNEL32.dll", "Sleep", &[Imm(100)])
            .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
        },
        |m| {
            let mut cb = Code::new(CALLBACK);
            cb.call(m, "KERNEL32.dll", "ExitThread", &[Imm(0)])
                .emit(&[0xc2, 0x14, 0x00]);
            cb
        },
    );
    assert!(
        matches!(exit_outcome, Outcome::Wall { ref what } if what.contains("ExitThread") && what.contains("WINMM's timer thread")),
        "expected timer-thread ExitThread wall, got {exit_outcome:?}"
    );
}

#[test]
fn active_mm_timer_survives_llsnap_v2_round_trip_bit_exact() {
    let build_machine = || {
        build_guest(
            |m, main| {
                main.call(m, "WINMM.dll", "timeGetTime", &[])
                    .answer()
                    .call(
                        m,
                        "WINMM.dll",
                        "timeSetEvent",
                        &[
                            Imm(50),
                            Imm(10),
                            Imm(CALLBACK),
                            Imm(0x55aa),
                            Imm(TIME_PERIODIC),
                        ],
                    )
                    .store(TIMER_A)
                    .answer()
                    .call(m, "KERNEL32.dll", "Sleep", &[Imm(120)])
                    .call(m, "KERNEL32.dll", "Sleep", &[Imm(120)])
                    .call(m, "WINMM.dll", "timeKillEvent", &[At(TIMER_A)])
                    .answer()
                    .call(m, "KERNEL32.dll", "ExitProcess", &[Imm(0)]);
            },
            build_default_callback,
        )
    };

    let (mut base, mut host_base) = build_machine();
    let mut log_base = RunLog::create("mm-timer-snap-base", Trace::Off).expect("log");
    let dir_base = log_base.dir().to_path_buf();
    let finish_base = base.run(&mut host_base, &mut log_base, 50_000_000);
    assert_eq!(finish_base.outcome, Outcome::Exited { code: 0 });
    assert_eq!(read_entries(&base).len(), 4);

    let (mut early, mut host_early) = build_machine();
    let mut log_early = RunLog::create("mm-timer-snap-early", Trace::Off).expect("log");
    let dir_early = log_early.dir().to_path_buf();
    while early.arena.read_u32(COUNT).unwrap() < 2 {
        assert!(early
            .step(&mut host_early, &mut log_early, 50_000_000, true)
            .is_none());
    }
    let snap = early.snapshot_v2().expect("snapshot v2");

    let (mut resumed, mut host_resumed) = build_machine();
    resumed.restore_v2(&snap).expect("restore v2");
    let mut log_resumed = RunLog::create("mm-timer-snap-resumed", Trace::Off).expect("log");
    let dir_resumed = log_resumed.dir().to_path_buf();
    let finish_resumed = resumed.run(&mut host_resumed, &mut log_resumed, 50_000_000);

    assert_eq!(finish_resumed, finish_base);
    assert_eq!(read_entries(&resumed), read_entries(&base));
    assert_eq!(resumed.state_digest(), base.state_digest());

    drop(log_base);
    drop(log_early);
    drop(log_resumed);
    let _ = std::fs::remove_dir_all(dir_base);
    let _ = std::fs::remove_dir_all(dir_early);
    let _ = std::fs::remove_dir_all(dir_resumed);
}
