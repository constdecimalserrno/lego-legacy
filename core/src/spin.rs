//! A clock spin passed exactly, a millisecond at a time (ADR-0004, #217).
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
struct Read {
    /// Which crossing of the run it was.
    crossing: u64,
    cursor: u64,
    cycles: u64,
    millis: u64,
    cpu: Cpu,
}

/// Which crossing of the run was the last clock read, and at which entry: a
/// spin is two of them in a row at the same one.
#[derive(Default, PartialEq)]
struct Entry {
    crossing: u64,
    eip: u32,
}

/// The clock reads the guest has crossed for, and how much passing has
/// saved.
#[derive(Default)]
pub(crate) struct Spins {
    /// Crossings so far.
    crossings: u64,
    read: Entry,
    /// The second of two such reads, as the guest stood at it: what the
    /// next is held to.
    last: Option<Read>,
    /// The crossing whose burst came back to the last read with the
    /// processor as it was there: only the read straight after it is worth
    /// watching memory from, since a pass that changes a register is never
    /// passed anyway.
    steady: Option<u64>,
    /// How far a pass may take the cursor, and whether one may be taken at
    /// all: a traced run logs every call and a stepped one stops at every
    /// instruction, so neither is ever passed for.
    budget: u64,
    allowed: bool,
    instructions: u64,
}

impl Spins {
    /// What the schedule allows from here: never past `budget`, and nothing
    /// unless `allowed`.
    pub(crate) fn allow(&mut self, budget: u64, allowed: bool) {
        self.budget = budget;
        self.allowed = allowed;
    }

    /// Instructions retired by passing, in all.
    #[must_use]
    pub(crate) fn instructions(&self) -> u64 {
        self.instructions
    }

    /// The guest crossed the boundary: any crossing ends a spin.
    pub(crate) fn crossed(&mut self, mem: &mut Arena) {
        self.crossings += 1;
        mem.unwatch();
    }

    /// The guest is about to be answered by `shim`. A clock read straight
    /// after another at the same entry is remembered as it stands, and memory
    /// is watched from here if the pass before it came back the same; any
    /// other read is only noted.
    pub(crate) fn crossing(&mut self, shim: &Shim, cpu: &Cpu, clock: &Clock, mem: &mut Arena) {
        if !reads_clock(shim) {
            return;
        }
        let here = Entry {
            crossing: self.crossings,
            eip: cpu.eip,
        };
        let again = self.read
            == Entry {
                crossing: self.crossings - 1,
                ..here
            };
        self.read = here;
        if again && self.allowed {
            self.last = Some(Read {
                crossing: self.crossings,
                cursor: clock.cursor(),
                cycles: clock.cycles(),
                millis: clock.millis(),
                cpu: *cpu,
            });
            if self.steady.take() == Some(self.crossings - 1) {
                mem.watch();
            }
        }
    }

    /// A burst of the guest's own code has ended. If it ended back at the
    /// clock read the last one was answered at, with nothing crossed since,
    /// in the same millisecond, with the processor as it was there, no byte
    /// of memory changed and no other thread able to run, and no quantum
    /// ended a burst inside the pass, the pass will be made again, unchanged,
    /// until the clock reads the next millisecond. So the cursor goes to the
    /// first read that does, or as far toward it as the budget lets it, and
    /// the schedule carries on there as it would have.
    pub(crate) fn pass(&mut self, cpu: &mut Cpu, clock: &mut Clock, mem: &Arena, state: &State) {
        // Most bursts end anywhere but back at the read: that is told first,
        // and cheaply, since every burst comes here.
        let Some(last) = &self.last else {
            return;
        };
        if !self.allowed || last.crossing != self.crossings || cpu.eip != last.cpu.eip {
            return;
        }
        let millis = clock.millis();
        let len = clock.cursor().saturating_sub(last.cursor);
        if millis != last.millis
            || len == 0
            // A pass costs the declared clock its instructions and nothing
            // more, or its passes would land somewhere running them does not.
            || clock.cycles() - last.cycles != len
            // The pass is its own instructions and the read's crossing; more
            // than a quantum of them and a burst ended inside it.
            || len - 1 > crate::machine::BURST
            || !same(cpu, &last.cpu)
        {
            return;
        }
        self.steady = Some(self.crossings);
        if !mem.unchanged() || another_runs(state, millis) {
            return;
        }
        let passes = clock
            .first_cycle_of(millis + 1)
            .saturating_sub(clock.cycles())
            .div_ceil(len)
            .min(self.budget.saturating_sub(clock.cursor()) / len);
        // The cycle counter `rdtsc` reads goes on with the clock, as it does
        // one to an instruction through every pass.
        clock.retire_many(passes * len);
        cpu.tsc += passes * len;
        self.instructions += passes * len;
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

#[cfg(test)]
mod tests {
    use super::{Spins, CLOCK_READS};
    use crate::clock::Clock;
    use crate::cpu::Cpu;
    use crate::guest::Arena;
    use crate::host::headless::Headless;
    use crate::win32::{Shim, State};

    const HZ: u64 = 450_000_000;

    /// One of the clock reads, as the boundary binds it.
    fn read() -> &'static Shim {
        let (dll, name) = CLOCK_READS[1];
        crate::win32::find(dll, name).expect("a clock read")
    }

    /// The machine at a moment of the run: the cursor, and the declared
    /// cycles, which are the cursor's and whatever was charged on top.
    fn at(cursor: u64, charged: u64) -> Clock {
        Clock::resumed(cursor, cursor + charged, HZ)
    }

    /// Three reads in a row at the same entry, a pass of `len` apart, each
    /// burst ending back at the next: by the third, a spin passed. Answers
    /// the cursor the next read stands at.
    fn spinning(spins: &mut Spins, mem: &mut Arena, cpu: &Cpu, len: u64) -> u64 {
        let state = State::new();
        let mut cursor = 1_000;
        for _ in 0..3 {
            spins.crossed(mem);
            spins.crossing(read(), cpu, &at(cursor, 0), mem);
            cursor += len;
            spins.pass(&mut cpu.clone(), &mut at(cursor, 0), mem, &state);
        }
        cursor
    }

    /// A pass is passed only if it cost the declared clock its instructions
    /// and nothing more. One charged more, as a declared cost per clock read
    /// would charge it, takes fewer of them to reach the next millisecond
    /// than its length says, so passing it by its length would land beyond
    /// the read that sees it.
    #[test]
    fn a_pass_charged_more_than_its_instructions_is_never_passed() {
        let cpu = Cpu {
            eip: 0x0f00_0010,
            ..Cpu::default()
        };
        let state = State::new();
        for (charge, passed) in [(0, true), (7, false)] {
            let mut mem = Arena::new(&mut Headless::new(), 1 << 20).expect("an arena");
            let mut spins = Spins::default();
            spins.allow(u64::MAX, true);
            let cursor = spinning(&mut spins, &mut mem, &cpu, 5);
            // The fourth read, then its pass, charged on top or not.
            let before = spins.instructions();
            spins.crossed(&mut mem);
            spins.crossing(read(), &cpu, &at(cursor, 0), &mut mem);
            spins.pass(&mut cpu.clone(), &mut at(cursor + 5, charge), &mem, &state);
            assert_eq!(
                spins.instructions() > before,
                passed,
                "a pass charged {charge} cycles more than its instructions"
            );
        }
    }

    /// The mark a pass that came back the same leaves is for the read
    /// straight after it, and no later one: memory is watched only where a
    /// pass might be passed.
    #[test]
    fn a_steady_pass_arms_the_watch_at_the_read_after_it_and_no_later() {
        let cpu = Cpu {
            eip: 0x0f00_0010,
            ..Cpu::default()
        };
        let state = State::new();
        let mut mem = Arena::new(&mut Headless::new(), 1 << 20).expect("an arena");
        let mut spins = Spins::default();
        spins.allow(u64::MAX, true);
        // Two reads in a row and a pass that comes back the same, refused
        // there since nothing watched it: the mark is left.
        let mut cursor = 1_000;
        for _ in 0..2 {
            spins.crossed(&mut mem);
            spins.crossing(read(), &cpu, &at(cursor, 0), &mut mem);
            cursor += 5;
        }
        spins.pass(&mut cpu.clone(), &mut at(cursor, 0), &mem, &state);
        // Something else is crossed for, then two reads in a row again.
        spins.crossed(&mut mem);
        for _ in 0..2 {
            spins.crossed(&mut mem);
            spins.crossing(read(), &cpu, &at(cursor, 0), &mut mem);
            cursor += 5;
        }
        assert!(
            !mem.unchanged(),
            "no watch: no pass came back the same since"
        );
    }
}
