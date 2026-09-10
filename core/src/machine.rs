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
use crate::win32::{self, Convention, Ctx, State};
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
    /// Everything the guest holds that is not memory.
    pub state: State,
    /// What the declared machine has on its disc.
    pub tree: crate::install::Tree,
    /// The title this machine was built for.
    title: &'static Title,
    /// Everything delivered to the guest, in order.
    pub input: Vec<(u64, crate::host::Input)>,
    /// The route this run replays, if it is replaying one.
    pub route: Option<crate::route::Route>,
    /// What this run did, if it is recording.
    pub recording: Option<crate::route::Route>,
}

impl Machine {
    /// Build a machine for a title, with the interpreter and an empty arena.
    ///
    /// # Errors
    /// When the host cannot map the arena.
    pub fn new(title: &'static Title, host: &mut dyn Host) -> Result<Self, crate::host::HostError> {
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
            state: State::new(),
            tree: crate::install::Tree::new(),
            title,
            input: Vec::new(),
            route: None,
            recording: None,
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

        // A replaying run takes its input from the route rather than from the
        // person, and the person's own input is not mixed in: a route replays
        // what it says and nothing else.
        let events = match &self.route {
            Some(route) => {
                let _ = host.poll_input(self.flips);
                route.at(self.flips)
            }
            None => host.poll_input(self.flips),
        };
        for event in events {
            log.line(&format!("input flip {} {event:?}", self.flips));
            if let Some(recording) = &mut self.recording {
                recording.push(self.flips, event);
            }
            self.input.push((self.flips, event));
        }
        self.flips += 1;
    }

    /// Take the machine one instruction further, or say why it stopped.
    ///
    /// Returns `None` while there is more to do. The debugger and the run loop
    /// share this, so what a person steps through is what a route replays.
    pub fn tick(&mut self, host: &mut dyn Host, log: &mut RunLog) -> Option<Outcome> {
        // A person closing the window ends the run there and then, with the
        // log written up to the instruction it stopped on.
        if host.closed() {
            return Some(Outcome::Closed);
        }
        // A call that lands on the boundary is a call into something Microsoft
        // wrote. Either it is implemented here, or the run stops and names
        // what was asked for, who asked, and where.
        if self.boundary.holds(self.cpu.eip) {
            return match self.cross(log) {
                Ok(()) => None,
                Err(Stop::Exited { code }) => Some(Outcome::Exited { code }),
                Err(stop) => Some(Outcome::Wall {
                    what: stop.to_string(),
                }),
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
            Ok(()) => {
                self.clock.retire();
                None
            }
            Err(Stop::Interrupt(n)) => {
                self.clock.retire();
                match (self.stub, n) {
                    (true, stub::INT_PRESENT) => {
                        self.present(host, log);
                        None
                    }
                    (true, stub::INT_EXIT) => Some(Outcome::Exited {
                        code: self.cpu.r[EAX],
                    }),
                    _ => Some(Outcome::Wall {
                        what: format!("int {n:#04x}"),
                    }),
                }
            }
            Err(stop) => Some(Outcome::Wall {
                what: stop.to_string(),
            }),
        }
    }

    /// What the machine has spent so far.
    #[must_use]
    pub fn finish(&self, outcome: Outcome) -> Finish {
        Finish {
            outcome,
            cursor: self.clock.cursor(),
            flips: self.flips,
            declared_millis: self.clock.millis(),
        }
    }

    /// Run until the guest ends the run, the machine hits a wall, or the
    /// budget runs out.
    pub fn run(&mut self, host: &mut dyn Host, log: &mut RunLog, budget: u64) -> Finish {
        log.line(&format!("engine {}", self.engine.name()));
        let outcome = loop {
            if self.clock.cursor() >= budget {
                break Outcome::BudgetExhausted;
            }
            if let Some(outcome) = self.tick(host, log) {
                break outcome;
            }
        };
        let finish = self.finish(outcome);
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

        // The pressing stays in the declared machine's drive for the whole
        // run: a title reads its own disc, and the label it checks is the
        // label the image carries.
        self.state.volume_label = iso.label.clone();

        // What this title's installer would have put on the machine's disc,
        // declared from the pressing's own cabinet rather than by running the
        // installer (#16).
        let mut iso = iso;
        if let Ok(header) = iso.read("DATA1.HDR") {
            self.state.installed_files =
                self.tree
                    .set_cabinet(&header, "DATA1.CAB", title.install_dir)?;
        }
        self.tree.set_disc(iso);

        let image = crate::pe::load(&bytes, &mut self.arena)?;
        // The guest starts where it was installed, as a player double-clicking
        // it would.
        self.state.current_directory = title.install_dir.to_owned();
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

impl Machine {
    /// Answer a call the guest made across the boundary.
    ///
    /// The arguments are where the calling convention says they are, the
    /// answer goes back in `eax`, and the stack is left as the real library
    /// would have left it.
    fn cross(&mut self, log: &mut RunLog) -> Result<(), Stop> {
        let addr = self.cpu.eip;
        let Some(import) = self.boundary.at(addr).cloned() else {
            return Err(Stop::Undecoded {
                eip: addr,
                byte: 0,
                group: None,
                why: "an address inside the boundary that no import was bound to",
            });
        };
        let what = format!("{}!{}", import.dll, import.name);
        let esp = self.cpu.r[crate::cpu::ESP];
        let caller = self.arena.read_u32(esp).map_err(Stop::Fault)?;

        let Some(shim) = win32::find(&import.dll, &import.name) else {
            log.at(
                Trace::Shims,
                &format!(
                    "wall {what} from {caller:#010x} cursor {}",
                    self.clock.cursor()
                ),
            );
            return Err(Stop::NotWritten { what, caller });
        };

        let mut args = Vec::with_capacity(shim.args as usize);
        for i in 0..shim.args {
            args.push(self.arena.read_u32(esp + 4 + i * 4).map_err(Stop::Fault)?);
        }
        log.at(
            Trace::Shims,
            &format!(
                "call {what}({}) from {caller:#010x} cursor {}",
                args.iter()
                    .map(|a| format!("{a:#x}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                self.clock.cursor()
            ),
        );

        let mut ctx = Ctx {
            cpu: &mut self.cpu,
            mem: &mut self.arena,
            state: &mut self.state,
            title: self.title,
            log,
            tree: &mut self.tree,
            clock_millis: self.clock.millis(),
        };
        let answer = (shim.run)(&mut ctx, &args)?;

        // The call returns: the return address comes off the stack, the
        // arguments come off with it when the callee owns them, and the answer
        // is in eax.
        let popped = match shim.convention {
            Convention::Stdcall => 4 + shim.args * 4,
            Convention::Cdecl => 4,
        };
        self.cpu.r[crate::cpu::ESP] = esp.wrapping_add(popped);
        self.cpu.r[crate::cpu::EAX] = answer;
        self.cpu.eip = caller;
        // A call across the boundary costs the declared clock what an
        // instruction costs, so a title that spends its time in shims still
        // advances its own clock.
        self.clock.retire();
        Ok(())
    }
}

impl Machine {
    /// A digest of everything a run can be judged by.
    ///
    /// Two runs of the same route on the same build must produce the same
    /// value: the processor, the clock, and the parts of memory anything has
    /// written to. The whole arena is not hashed, because most of it has never
    /// been touched and hashing 256 MB to prove that is a slow way to learn
    /// nothing.
    #[must_use]
    pub fn state_digest(&self) -> String {
        let mut hash = crate::hash::Sha256::new();
        for value in self.cpu.r {
            hash.update(&value.to_le_bytes());
        }
        hash.update(&self.cpu.eip.to_le_bytes());
        hash.update(&self.cpu.flags.to_eflags().to_le_bytes());
        for seg in self.cpu.seg {
            hash.update(&seg.to_le_bytes());
        }
        hash.update(&self.clock.cursor().to_le_bytes());
        hash.update(&self.flips.to_le_bytes());

        // The regions anything writes to: where the image was placed, how far
        // the heap has grown, and the stack down to where it stands.
        let mut region = |start: u32, len: u32| {
            if len > 0 {
                if let Ok(bytes) = self.arena.slice(start, len as usize) {
                    hash.update(bytes);
                }
            }
        };
        if let Some(image) = &self.image {
            region(image.base, image.size);
        }
        region(crate::guest::heap::HEAP_BASE, self.state.heap.extent());
        let esp = self.cpu.r[crate::cpu::ESP];
        if esp < STACK_TOP {
            region(esp, STACK_TOP - esp);
        }
        if let Some(surface) = self.surface {
            region(surface.addr, surface.pitch * surface.height);
        }
        crate::hash::hex(&hash.finish())
    }
}

impl Machine {
    /// What this run is, for a snapshot to be keyed by.
    #[must_use]
    pub fn provenance(&self) -> crate::snap::Provenance {
        let route = match &self.route {
            Some(route) => crate::hash::sha256_hex(route.to_text().as_bytes()),
            None => crate::hash::sha256_hex(b""),
        };
        crate::snap::Provenance {
            build: crate::snap::build_digest(),
            title: self.title.key.to_owned(),
            route,
        }
    }

    /// The regions of guest memory a snapshot carries.
    ///
    // ponytail: the declared regions rather than every page. A guest that
    // wrote outside its own image, heap, stack and surface would resume with
    // that writing lost. Upgrade path: a dirty-page map in the arena, which
    // the engine's unchecked path would have to go through.
    fn snapshot_regions(&self) -> Vec<(u32, u32)> {
        let mut regions = Vec::new();
        if let Some(image) = &self.image {
            regions.push((image.base, image.size));
        }
        regions.push((crate::guest::heap::HEAP_BASE, self.state.heap.extent()));
        let esp = self.cpu.r[crate::cpu::ESP];
        if esp < STACK_TOP {
            regions.push((esp, STACK_TOP - esp));
        }
        if let Some(surface) = self.surface {
            regions.push((surface.addr, surface.pitch * surface.height));
        }
        regions.retain(|(_, len)| *len > 0);
        regions
    }

    /// Write the machine's state, keyed by what produced it.
    ///
    /// # Errors
    /// When guest memory cannot be read.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, crate::guest::Fault> {
        use crate::snap::{Snap, Visitor, Writer, MAGIC};

        let mut out = Vec::from(*MAGIC);
        let provenance = self.provenance();
        for part in [&provenance.build, &provenance.title, &provenance.route] {
            out.extend_from_slice(&u32::try_from(part.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(part.as_bytes());
        }

        let mut writer = Writer::new();
        self.cpu.snap(&mut writer);
        self.clock.snap(&mut writer);
        let mut flips = self.flips;
        writer.u64(&mut flips);

        let regions = self.snapshot_regions();
        let mut count = u32::try_from(regions.len()).unwrap_or(0);
        writer.u32(&mut count);
        for (start, len) in regions {
            let (mut start, mut len) = (start, len);
            writer.u32(&mut start);
            writer.u32(&mut len);
            let bytes = self.arena.slice(start, len as usize)?.to_vec();
            writer.out.extend_from_slice(&bytes);
        }
        out.extend_from_slice(&writer.out);
        Ok(out)
    }

    /// Put the machine back where a snapshot left it.
    ///
    /// # Errors
    /// When the snapshot came from another build, title or route, or when it
    /// is not a snapshot at all.
    pub fn resume(&mut self, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        use crate::snap::{Reader, Snap, SnapError, Visitor, MAGIC};

        if bytes.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
            return Err(SnapError::NotASnapshot.into());
        }
        let mut at = MAGIC.len();
        let mut part = || -> Result<String, SnapError> {
            let len = bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                .ok_or(SnapError::Truncated("provenance"))?;
            at += 4;
            let text = bytes
                .get(at..at + len)
                .ok_or(SnapError::Truncated("provenance"))?;
            at += len;
            Ok(String::from_utf8_lossy(text).into_owned())
        };
        let found = crate::snap::Provenance {
            build: part()?,
            title: part()?,
            route: part()?,
        };
        self.provenance().accepts(&found)?;

        let mut reader = Reader::new(&bytes[at..]);
        self.cpu.snap(&mut reader);
        self.clock.snap(&mut reader);
        let mut flips = 0u64;
        reader.u64(&mut flips);
        self.flips = flips;

        let mut count = 0u32;
        reader.u32(&mut count);
        for _ in 0..count {
            let (mut start, mut len) = (0u32, 0u32);
            reader.u32(&mut start);
            reader.u32(&mut len);
            let mut region = vec![0u8; len as usize];
            reader.bytes(&mut region);
            self.arena.write(start, &region)?;
        }
        if reader.short {
            return Err(SnapError::Truncated("state").into());
        }
        // Nothing decoded before the resume describes the memory that is here
        // now.
        self.forget_decoded();
        Ok(())
    }

    /// Throw away every decoded instruction, after memory has changed under
    /// the engine.
    pub fn forget_decoded(&mut self) {
        self.engine = Box::new(Interp::new(Mode::FLAT));
    }
}
