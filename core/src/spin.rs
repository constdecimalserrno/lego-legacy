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
use crate::cpu::Cpu;
use crate::guest::Arena;
use crate::win32::{Shim, State};

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

/// A clock read as the guest stood at it, before it was answered.
#[derive(Clone, Copy)]
struct Read {
    cursor: u64,
    millis: u64,
    cpu: Cpu,
}

/// The last clock read, while nothing else has crossed since, and how much
/// passing in one step has saved.
#[derive(Default)]
pub struct Spins {
    last: Option<Read>,
    /// How far a pass may take the cursor, and whether one may be taken at
    /// all: a traced run logs every call and a stepped one stops at every
    /// instruction, so neither is ever passed for.
    budget: u64,
    allowed: bool,
    passed: u64,
}

impl Spins {
    /// What the schedule allows from here: never past `budget`, and nothing
    /// unless `allowed`.
    pub fn allow(&mut self, budget: u64, allowed: bool) {
        self.budget = budget;
        self.allowed = allowed;
    }

    /// Instructions retired in one step, in all.
    #[must_use]
    pub fn passed(&self) -> u64 {
        self.passed
    }

    /// The guest crossed the boundary: any crossing ends a spin.
    pub fn forget(&mut self, mem: &mut Arena) {
        self.last = None;
        mem.unwatch();
    }

    /// The guest is about to be answered by `shim`. A clock read is
    /// remembered as it stands, and memory is watched from here.
    pub fn crossing(&mut self, shim: &Shim, cpu: &Cpu, clock: &Clock, mem: &mut Arena) {
        if CLOCK_READS.contains(&(shim.dll, shim.name)) {
            self.last = Some(Read {
                cursor: clock.cursor(),
                millis: clock.millis(),
                cpu: *cpu,
            });
            mem.watch();
        }
    }

    /// A burst of the guest's own code has ended. If it ended back at the
    /// clock read the last one was answered at, in the same millisecond, with
    /// the processor as it was there and no byte of memory changed, and no
    /// other thread can run, the pass between them will be made again,
    /// unchanged, until the clock reads the next millisecond: so the cursor
    /// goes to the first read that does, or as far toward it as the budget
    /// lets it. The schedule then carries on there as it would have.
    pub fn pass(&mut self, cpu: &mut Cpu, clock: &mut Clock, mem: &Arena, state: &State) {
        let Some(last) = self.last.filter(|_| self.allowed) else {
            return;
        };
        let millis = clock.millis();
        let len = clock.cursor() - last.cursor;
        if millis != last.millis
            || len == 0
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
/// from the clock at every burst, and `==` on a float calls 0.0 and -0.0 one.
fn same(a: &Cpu, b: &Cpu) -> bool {
    Cpu { tsc: 0, ..*a } == Cpu { tsc: 0, ..*b } && a.fpu.same_bits(&b.fpu)
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
