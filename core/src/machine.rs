//! One emulator instance: an arena, a processor, a clock, and the loop that
//! spends them.

use crate::cli::Trace;
use crate::clock::{Clock, FlipCost};
use crate::cpu::{interp::Interp, Cpu, CpuEngine, Mode, Stop, EAX, REG_NAMES};
use crate::guest::{Arena, GuestMem};
use crate::host::{Frame, Host};
use crate::runlog::RunLog;
use crate::shim::Boundary;
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
    /// The person closed the window.
    Closed,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited { code } => write!(f, "exited {code}"),
            Self::Wall { what } => write!(f, "wall: {what}"),
            Self::BudgetExhausted => write!(f, "budget exhausted"),
            Self::Closed => write!(f, "window closed"),
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
    pub cpu: Cpu,
    pub clock: Clock,
    engine: Box<dyn CpuEngine>,
    flip_cost: FlipCost,
    surface: Option<Surface>,
    stub: bool,
    flips: u64,
    /// What the title's imports were bound to.
    pub boundary: Boundary,
    /// The image this machine is running, once one is loaded.
    pub image: Option<crate::pe::Image>,
}

impl Machine {
    /// Build a machine for a title, with the interpreter and an empty arena.
    ///
    /// # Errors
    /// When the host cannot map the arena.
    pub fn new(title: &Title, host: &mut dyn Host) -> Result<Self, crate::host::HostError> {
        Ok(Self {
            arena: Arena::new(host, ARENA_BYTES)?,
            cpu: Cpu::default(),
            clock: Clock::new(),
            engine: Box::new(Interp::new(Mode::FLAT)),
            flip_cost: title.flip_cost,
            surface: None,
            stub: false,
            flips: 0,
            boundary: Boundary::new(),
            image: None,
        })
    }

    /// Place the stub guest and point the processor at it.
    ///
    /// # Errors
    /// When the arena will not take the program.
    pub fn load_stub(&mut self) -> Result<(), crate::guest::Fault> {
        self.arena.write(stub::ENTRY, stub::program())?;
        self.cpu = Cpu {
            eip: stub::ENTRY,
            ..Cpu::default()
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
            // A person closing the window ends the run there and then, with
            // the log written up to the instruction it stopped on.
            if host.closed() {
                break Outcome::Closed;
            }
            // A call that lands on the boundary is a call into something
            // Microsoft wrote. Either it is implemented here, or the run stops
            // and names what was asked for, who asked, and where.
            if let Some(import) = self.boundary.at(self.cpu.eip) {
                let caller = self
                    .arena
                    .read_u32(self.cpu.r[crate::cpu::ESP])
                    .map_or_else(|_| "unknown".to_owned(), |a| format!("{a:#010x}"));
                let what = format!("{}!{}", import.dll, import.name);
                log.at(
                    Trace::Shims,
                    &format!("shim {what} from {caller} cursor {}", self.clock.cursor()),
                );
                break Outcome::Wall {
                    what: format!("{what} is imported and not written yet, called from {caller}"),
                };
            }
            if log.trace() >= Trace::Insns {
                log.line(&format!(
                    "insn cursor {} eip {:#010x} eax {:#010x}",
                    self.clock.cursor(),
                    self.cpu.eip,
                    self.cpu.r[EAX]
                ));
            }
            match self.engine.step(&mut self.cpu, &mut self.arena) {
                Ok(()) => self.clock.retire(),
                Err(Stop::Interrupt(n)) => {
                    self.clock.retire();
                    match (self.stub, n) {
                        (true, stub::INT_PRESENT) => self.present(host, log),
                        (true, stub::INT_EXIT) => {
                            break Outcome::Exited {
                                code: self.cpu.r[EAX],
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
                .zip(self.cpu.r)
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

/// Where the guest's stack lives, and how much of it there is.
///
/// A declared value: the address is chosen, not measured, and it sits below
/// every image the roster's titles ask to be loaded at.
pub const STACK_TOP: u32 = 0x0020_0000;
pub const STACK_BYTES: u32 = 0x0010_0000;

impl Machine {
    /// Load a title's entry image out of its pressing and point the processor
    /// at it.
    ///
    /// # Errors
    /// When the rip is absent, the image is not the one the title declares,
    /// the image is a copy-protection wrapper, or it will not fit.
    pub fn load_title(&mut self, title: &Title) -> Result<(), Box<dyn std::error::Error>> {
        let mut iso = crate::disc::open_pressing(title.image.disc_dir, title.image.pressing)?;
        let bytes = iso.read(title.image.path)?;

        // A title is the bytes it is made of. A pressing that has been patched
        // or is simply the other one is refused by name rather than run.
        let digest = crate::hash::sha256_hex(&bytes);
        if digest != title.image.sha256 {
            return Err(format!(
                "{} in {} hashes {digest}, and this title is declared as {}. \
                 A different pressing, a patched executable, or a bad rip",
                title.image.path,
                iso.path().display(),
                title.image.sha256
            )
            .into());
        }

        let image = crate::pe::load(&bytes, &mut self.arena)?;
        self.boundary.bind(&image.imports, &mut self.arena)?;
        self.cpu = Cpu {
            eip: image.base.wrapping_add(image.entry),
            r: {
                let mut r = [0u32; 8];
                r[crate::cpu::ESP] = STACK_TOP - 16;
                r
            },
            ..Cpu::default()
        };
        self.image = Some(image);
        Ok(())
    }
}
