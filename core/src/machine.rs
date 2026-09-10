//! One emulator instance: an arena, a processor, a clock, and the loop that
//! spends them.

use crate::cli::Trace;
use crate::clock::{Clock, FlipCost};
use crate::cpu::{interp::Interp, CpuEngine, Regs, Stop, EAX, REG_NAMES};
use crate::guest::{Arena, GuestMem};
use crate::host::{Frame, Host};
use crate::runlog::RunLog;
use crate::stub;
use crate::Title;
use std::fmt;

/// How much address space the declared machine has. 256 MB covers every
/// address the roster's titles form, and an anonymous mapping costs only what
/// is touched.
pub const ARENA_BYTES: usize = 256 * 1024 * 1024;

/// Where the guest's presentable pixels are, and how they are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub addr: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

/// Why a run ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The guest ended the run itself.
    Exited { code: u32 },
    /// The guest asked the machine for something it does not have. A run stops
    /// at a wall and never continues past one.
    Wall { what: String },
    /// The instruction budget ran out first.
    BudgetExhausted,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited { code } => write!(f, "exited {code}"),
            Self::Wall { what } => write!(f, "wall: {what}"),
            Self::BudgetExhausted => write!(f, "budget exhausted"),
        }
    }
}

/// A run that has ended, and what it spent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    pub outcome: Outcome,
    pub cursor: u64,
    pub flips: u64,
    pub declared_millis: u64,
}

/// The whole machine.
pub struct Machine {
    pub arena: Arena,
    pub regs: Regs,
    pub clock: Clock,
    engine: Box<dyn CpuEngine>,
    flip_cost: FlipCost,
    surface: Option<Surface>,
    stub: bool,
    flips: u64,
}

impl Machine {
    /// Build a machine for a title, with the interpreter and an empty arena.
    ///
    /// # Errors
    /// When the host cannot map the arena.
    pub fn new(title: &Title, host: &mut dyn Host) -> Result<Self, crate::host::HostError> {
        Ok(Self {
            arena: Arena::new(host, ARENA_BYTES)?,
            regs: Regs::default(),
            clock: Clock::new(),
            engine: Box::new(Interp::new()),
            flip_cost: title.flip_cost,
            surface: None,
            stub: false,
            flips: 0,
        })
    }

    /// Place the stub guest and point the processor at it.
    ///
    /// # Errors
    /// When the arena will not take the program.
    pub fn load_stub(&mut self) -> Result<(), crate::guest::Fault> {
        self.arena.write(stub::ENTRY, stub::program())?;
        self.regs = Regs {
            eip: stub::ENTRY,
            ..Regs::default()
        };
        self.surface = Some(stub::surface());
        self.stub = true;
        Ok(())
    }

    #[must_use]
    pub fn flips(&self) -> u64 {
        self.flips
    }

    #[must_use]
    pub fn engine_name(&self) -> &'static str {
        self.engine.name()
    }

    /// Present the declared surface, charge the clock for it, and take
    /// whatever a person did while it was on screen.
    fn present(&mut self, host: &mut dyn Host, log: &mut RunLog) {
        let Some(s) = self.surface else {
            log.line("flip refused: no surface declared");
            return;
        };
        match self.arena.slice(s.addr, (s.pitch * s.height) as usize) {
            Ok(pixels) => {
                let frame = Frame {
                    width: s.width,
                    height: s.height,
                    pitch: s.pitch,
                    pixels,
                };
                host.present(&frame);
            }
            Err(fault) => {
                log.line(&format!("flip refused: {fault}"));
                return;
            }
        }
        self.clock.present(self.flip_cost);
        log.line(&format!(
            "flip {} cursor {}",
            self.flips,
            self.clock.cursor()
        ));
        host.pace(self.clock.nanos());
        for event in host.poll_input(self.flips) {
            log.line(&format!("input flip {} {event:?}", self.flips));
        }
        self.flips += 1;
    }

    /// Run until the guest ends the run, the machine hits a wall, or the
    /// budget runs out.
    pub fn run(&mut self, host: &mut dyn Host, log: &mut RunLog, budget: u64) -> Finish {
        log.line(&format!("engine {}", self.engine.name()));
        let outcome = loop {
            if self.clock.cursor() >= budget {
                break Outcome::BudgetExhausted;
            }
            if log.trace() >= Trace::Insns {
                log.line(&format!(
                    "insn cursor {} eip {:#010x} eax {:#010x}",
                    self.clock.cursor(),
                    self.regs.eip,
                    self.regs.r[EAX]
                ));
            }
            match self.engine.step(&mut self.regs, &mut self.arena) {
                Ok(()) => self.clock.retire(),
                Err(Stop::Interrupt(n)) => {
                    self.clock.retire();
                    match (self.stub, n) {
                        (true, stub::INT_PRESENT) => self.present(host, log),
                        (true, stub::INT_EXIT) => {
                            break Outcome::Exited {
                                code: self.regs.r[EAX],
                            }
                        }
                        _ => {
                            break Outcome::Wall {
                                what: format!("int {n:#04x}"),
                            }
                        }
                    }
                }
                Err(stop) => {
                    break Outcome::Wall {
                        what: stop.to_string(),
                    }
                }
            }
        };
        let finish = Finish {
            outcome,
            cursor: self.clock.cursor(),
            flips: self.flips,
            declared_millis: self.clock.millis(),
        };
        log.line(&format!(
            "regs {}",
            REG_NAMES
                .iter()
                .zip(self.regs.r)
                .map(|(n, v)| format!("{n}={v:#010x}"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        log.line(&format!(
            "end {} cursor {} flips {} declared-ms {}",
            finish.outcome, finish.cursor, finish.flips, finish.declared_millis
        ));
        finish
    }
}
