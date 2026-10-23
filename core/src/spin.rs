//! A clock spin passed in one step, exactly (ADR-0004, #217).
//!
//! A title waiting for a moment to come asks the clock again and again. LEGO
//! Chess's movie player reads `QueryPerformanceCounter` between frames, 38
//! instructions a time and about 4,400 times a declared millisecond, and the
//! answer moves once a millisecond. Two reads in a row from the same place,
//! with the processor the same bit for bit and no byte of memory changed
//! between them, are a pass the guest will make again, unchanged, for as long
//! as the clock reads the same millisecond. So the machine retires those
//! passes at once and lands on the read that sees the next millisecond, with
//! exactly the cursor, declared clock and state that running every pass gives.
//! The guest's own code still runs the first passes of every millisecond, and
//! the one that sees it change.

use crate::clock::Clock;
use crate::cpu::x87::X87;
use crate::cpu::Cpu;
use crate::guest::Arena;
use crate::win32::{Shim, State};
use std::sync::OnceLock;

/// The clock reads whose answer is the declared millisecond and nothing else.
///
/// ponytail: a read answered more finely than a millisecond would want a
/// horizon of its own in [`Spins::pass`], not the next millisecond's.
const CLOCK_READS: [(&str, &str); 4] = [
    ("KERNEL32", "QueryPerformanceCounter"),
    ("KERNEL32", "GetTickCount"),
    ("WINMM", "timeGetTime"),
    ("WINMM", "timeGetSystemTime"),
];

/// Whether a shim is one of the clock reads: the very entries the boundary
/// binds, told apart by where they are, which every crossing can afford.
fn reads_clock(shim: &Shim) -> bool {
    static READS: OnceLock<Vec<&'static Shim>> = OnceLock::new();
    READS
        .get_or_init(|| {
            CLOCK_READS
                .iter()
                .filter_map(|(dll, name)| crate::win32::find(dll, name))
                .collect()
        })
        .iter()
        .any(|read| std::ptr::eq(*read, shim))
}

/// A clock read as the guest stood at it, before it was answered.
#[derive(Clone, Copy)]
struct Read {
    /// Which crossing of the run it was.
    crossing: u64,
    cursor: u64,
    millis: u64,
    cpu: Cpu,
}

/// The clock reads the guest has crossed for, and how much passing in one
/// step has saved.
#[derive(Default)]
pub struct Spins {
    /// Crossings so far, and which of them was the last clock read and at
    /// which entry: a spin is two of them in a row at the same one.
    crossings: u64,
    read: (u64, u32),
    /// The second of two such reads, as the guest stood at it: what the
    /// next is held to.
    last: Option<Read>,
    /// How far a pass may take the cursor, the longest pass a burst runs to
    /// its end, and whether a pass may be taken at all: a traced run logs
    /// every call and a stepped one stops at every instruction, so neither
    /// is ever passed for.
    budget: u64,
    quantum: u64,
    allowed: bool,
    passed: u64,
}

impl Spins {
    /// What the schedule allows from here: never past `budget`, no pass
    /// longer than `quantum`, and nothing unless `allowed`.
    pub(crate) fn allow(&mut self, budget: u64, quantum: u64, allowed: bool) {
        self.budget = budget;
        self.quantum = quantum;
        self.allowed = allowed;
    }

    /// Instructions retired in one step, in all.
    #[must_use]
    pub fn passed(&self) -> u64 {
        self.passed
    }

    /// The guest crossed the boundary: any crossing ends a spin.
    pub(crate) fn forget(&mut self, mem: &mut Arena) {
        self.crossings += 1;
        mem.unwatch();
    }

    /// The guest is about to be answered by `shim`. A clock read straight
    /// after another at the same entry is remembered as it stands, and memory
    /// is watched from here; any other is only noted.
    pub(crate) fn crossing(&mut self, shim: &Shim, cpu: &Cpu, clock: &Clock, mem: &mut Arena) {
        if !reads_clock(shim) {
            return;
        }
        let again = self.read == (self.crossings - 1, cpu.eip);
        self.read = (self.crossings, cpu.eip);
        if again && self.allowed {
            self.last = Some(Read {
                crossing: self.crossings,
                cursor: clock.cursor(),
                millis: clock.millis(),
                cpu: *cpu,
            });
            mem.watch();
        }
    }

    /// A burst of the guest's own code has ended. If it ended back at the
    /// clock read the last one was answered at, with nothing crossed since,
    /// in the same millisecond, with the processor as it was there, no byte
    /// of memory changed and no other thread able to run, and the pass was no
    /// longer than a burst runs, the pass will be made again, unchanged,
    /// until the clock reads the next millisecond. So the cursor goes to the
    /// first read that does, or as far toward it as the budget lets it, and
    /// the schedule carries on there as it would have.
    pub(crate) fn pass(&mut self, cpu: &mut Cpu, clock: &mut Clock, mem: &Arena, state: &State) {
        // Most bursts end anywhere but back at the read: that is told first,
        // and cheaply, since every burst comes here.
        let Some(last) = &self.last else {
            return;
        };
        if last.crossing != self.crossings || cpu.eip != last.cpu.eip {
            return;
        }
        let millis = clock.millis();
        let len = clock.cursor().saturating_sub(last.cursor);
        if millis != last.millis
            || len == 0
            || len > self.quantum
            || !same(cpu, &last.cpu)
            || !mem.unchanged()
            || another_runs(state, millis)
        {
            return;
        }
        // The first declared cycle the clock reads the next millisecond at:
        // a millisecond is `cycles × 1000 ÷ hz`, rounded down.
        let next = ((u128::from(millis) + 1) * u128::from(clock.hz())).div_ceil(1000);
        let next = u64::try_from(next).unwrap_or(u64::MAX);
        let passes = next
            .saturating_sub(clock.cycles())
            .div_ceil(len)
            .min(self.budget.saturating_sub(clock.cursor()) / len);
        // The cycle counter `rdtsc` reads goes on with the clock, as it does
        // one to an instruction through every pass.
        clock.retire_many(passes * len);
        cpu.tsc += passes * len;
        self.passed += passes * len;
    }
}

/// The same processor, bit for bit. Its cycle counter is the machine's to set
/// from the clock at every burst, and its floating-point unit is held to its
/// bits, since `==` on a float calls 0.0 and -0.0 one and a NaN no number.
fn same(a: &Cpu, b: &Cpu) -> bool {
    let rest = |c: &Cpu| Cpu {
        tsc: 0,
        fpu: X87::new(),
        ..*c
    };
    rest(a) == rest(b) && a.fpu.same_bits(&b.fpu)
}

/// Whether a thread other than the running one could be given a turn: then
/// the passes are not the guest's alone, and running them gives it one.
fn another_runs(state: &State, millis: u64) -> bool {
    state
        .threads
        .iter()
        .enumerate()
        .any(|(i, t)| i != state.thread && crate::machine::runnable_at(t, millis))
}
