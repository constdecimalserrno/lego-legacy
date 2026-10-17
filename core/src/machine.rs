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

/// How much address space the declared machine has.
///
/// Two gigabytes: the heap is a quarter of it, and an anonymous mapping costs
/// only the pages something writes to, so the rest is free until it is used.
pub const ARENA_BYTES: usize = 2 * 1024 * 1024 * 1024;

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
    /// The pace the host last said it was holding, so a change is said once
    /// rather than every flip (#75).
    paced: f64,
    /// What the title's imports were bound to.
    pub boundary: Boundary,
    /// The image this machine is running, once one is loaded.
    pub image: Option<crate::pe::Image>,
    /// The binaries the title ships beside it, loaded as guest code (#55).
    pub modules: crate::loader::Modules,
    /// How many names were bound across every image, which is what the run log
    /// says about the loader's work.
    pub bound_names: usize,
    /// Everything the guest holds that is not memory.
    pub state: State,
    /// What the declared machine has on its disc.
    pub tree: crate::install::Tree,
    /// The title this machine was built for.
    title: &'static Title,
    /// Everything delivered to the guest, in order.
    pub input: Vec<(u64, crate::host::Input)>,
    /// Where a guest frame is turned into something the host can show.
    present_buffer: Vec<u8>,
    /// How many times the title put the pointer, and how many of those the
    /// host was told about. The two differ by every frame the title moved the
    /// pointer while the person was still looking at their own — a title
    /// steering by a measurement the host never restarted (#40, #64).
    pointer_puts: u64,
    pointer_puts_told: u64,
    /// Whether the host is drawing the pointer, so that it is told only when
    /// that changes: hiding is counted, and telling it twice would take two
    /// showings to undo.
    pointer_shown: bool,
    /// Calls the machine made into guest code, waiting to come back.
    /// Calls the machine made into guest code, waiting to come back: where to
    /// resume, what to answer with, and where the guest's stack stood when the
    /// call was set up. The last of those is how an abandoned call is
    /// recognised — an exception handler that goes to its catch never returns
    /// (#54).
    continuations: Vec<(u32, u32, u32)>,
    /// Which parked calls are a modal dialog's turns, by their depth.
    dialog_turns: Vec<usize>,
    /// Which parked calls answer for themselves, by their depth (#60).
    pass_through: Vec<usize>,
    /// What each boundary slot was found to stand for the first time it was
    /// called, by the slot: looking a shim or a method up by its name on every
    /// call is most of what a call costs.
    bound: Vec<Option<Bound>>,
    /// Which parked calls are a render pass's turns, by their depth, with
    /// where the stack stood before the pass's first call: the title's
    /// callbacks leave their arguments for the caller to take.
    render_turns: Vec<(usize, u32)>,
    /// The route this run replays, if it is replaying one.
    pub route: Option<crate::route::Route>,
    /// What this run did, if it is recording.
    pub recording: Option<crate::route::Route>,
    /// Which flips to write into the run directory as pictures.
    pub screenshots: crate::cli::Screenshots,
    /// How many flips apart to write a snapshot into the run directory, if
    /// anyone asked for them.
    pub snapshot_every: Option<u64>,
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
            clock: Clock::at(title.declared.hz),
            engine: Box::new(Interp::new(Mode::FLAT)),
            flip_cost: title.flip_cost,
            surface: None,
            stub: false,
            flips: 0,
            paced: 1.0,
            boundary: Boundary::new(),
            image: None,
            modules: crate::loader::Modules::new(),
            bound_names: 0,
            state: State::new(),
            tree: crate::install::Tree::new(),
            title,
            present_buffer: Vec::new(),
            pointer_puts: 0,
            pointer_puts_told: 0,
            pointer_shown: true,
            continuations: Vec::new(),
            dialog_turns: Vec::new(),
            pass_through: Vec::new(),
            bound: Vec::new(),
            render_turns: Vec::new(),
            input: Vec::new(),
            route: None,
            recording: None,
            screenshots: crate::cli::Screenshots::default(),
            snapshot_every: None,
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

    /// Give `fs` a base, and put a thread information block under it.
    ///
    /// Every MSVC-built image starts by pushing an exception frame onto
    /// `fs:[0]`, and a flat `fs` makes that guest address zero: the chain head
    /// would be written over whatever is at the bottom of the arena and read
    /// back from there. Each field below is a declared value, and the ones
    /// this machine has no honest answer for are left zero rather than
    /// invented (#54).
    ///
    /// # Errors
    /// When the arena will not take it.
    fn declare_tib(&mut self) -> Result<(), crate::guest::Fault> {
        write_tib(
            &mut self.arena,
            TIB,
            STACK_TOP,
            STACK_TOP - STACK_BYTES,
            THREAD_ID,
        )?;
        // The process block, of which one field is read often enough to be
        // worth declaring: where the image was loaded.
        self.arena
            .write_u32(PEB + 0x08, self.image.as_ref().map_or(0, |i| i.base))?;
        self.cpu.seg_base[crate::cpu::FS] = TIB;
        Ok(())
    }

    #[must_use]
    pub fn flips(&self) -> u64 {
        self.flips
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
        self.pace(host, log);

        self.deliver_input(host, log);
        self.flips += 1;
    }

    /// Hold the host to the declared clock, and say what pace it settled on
    /// when that changes: a run in slow motion says so rather than looking
    /// broken (#75). Every flip comes here, the stub's and a title's (#94).
    fn pace(&mut self, host: &mut dyn Host, log: &mut RunLog) {
        let pace = host.pace(self.clock.nanos());
        if (pace - self.paced).abs() >= 0.05 {
            self.paced = pace;
            log.line(&format!("pace {pace:.2}x"));
        }
    }

    /// Hand the guest what a person did while the frame was on screen, at the
    /// flip boundary (#10).
    ///
    /// A replaying run takes its input from the route rather than from the
    /// person, and the person's own input is not mixed in: a route replays
    /// what it says and nothing else.
    fn deliver_input(&mut self, host: &mut dyn Host, log: &mut RunLog) {
        let events = match &self.route {
            // A route that has run out hands the machine to the person: what
            // it was for was getting here, and from here on the mouse and the
            // keyboard are theirs. Headless, the host has nothing to say and
            // the run reads as it always did (#86).
            Some(route)
                if !route.at(self.flips).is_empty() || !route.after(self.flips).is_empty() =>
            {
                let _ = host.poll_input(self.flips);
                route.at(self.flips)
            }
            _ => host.poll_input(self.flips),
        };
        let millis = u32::try_from(self.clock.millis() & 0xffff_ffff).unwrap_or(0);
        for event in events {
            log.line(&format!("input flip {} {event:?}", self.flips));
            if let Some(recording) = &mut self.recording {
                recording.push(self.flips, event);
            }
            self.input.push((self.flips, event));
            crate::win32::user32::deliver(&mut self.state, event, millis);
            // A title that listens for a key to get past its own intro, and
            // not for the mouse, is handed that key when a person clicks
            // through the movie: the click is what the person did, and this
            // is its translation, not the machine deciding the movie is over
            // (#66). What the title does with the key stays the title's rule.
            // Either button: a person hitting the mouse to get past a movie
            // has not chosen which one (#61).
            if let (Some(code), Some(down)) = (
                self.title.click_in_a_movie,
                match event {
                    crate::host::Input::Down(_) => Some(true),
                    crate::host::Input::Up(_) => Some(false),
                    _ => None,
                },
            ) {
                if self.state.movie_running() {
                    log.line(&format!(
                        "input flip {} click as key {code:#04x}",
                        self.flips
                    ));
                    crate::win32::user32::deliver(
                        &mut self.state,
                        crate::host::Input::Key { code, down },
                        millis,
                    );
                }
            }
        }
    }

    /// Take the machine one instruction further, or say why it stopped.
    ///
    /// Returns `None` while there is more to do. The debugger steps with this,
    /// and the run loop comes back to it for every call across the boundary,
    /// so what a person steps through is what a route replays.
    pub fn tick(&mut self, host: &mut dyn Host, log: &mut RunLog) -> Option<Outcome> {
        // A person closing the window ends the run there and then, with the
        // log written up to the instruction it stopped on.
        if host.closed() {
            return Some(Outcome::Closed);
        }
        // A call that lands on the boundary is a call into something Microsoft
        // wrote. Either it is implemented here, or the run stops and names
        // what was asked for, who asked, and where.
        // A crossing costs about thirty nanoseconds of machinery, measured
        // (#74): at Alpha Team's menu the guest crosses 3,951 times a flip
        // and the whole of it is 6.9 ms, of which 6.6 ms is one method —
        // `IDirect3DDevice7::DrawIndexedPrimitiveVB` rasterising, 171 calls a
        // flip at 38.7 us each. The 3,182 `EnterCriticalSection` and
        // `LeaveCriticalSection` calls a flip that ticket was written about
        // come to 0.10 ms a flip against a 36 ms frame, under a third of one
        // percent, and that figure still carries the two `Instant::now()`
        // calls that measured it. There is nothing here to make faster.
        if self.boundary.holds(self.cpu.eip) {
            return match self.cross(log) {
                Ok(()) => None,
                Err(Stop::Exited { code }) => Some(Outcome::Exited { code }),
                // A thread ending is not the run ending: the machine leaves it
                // where it is and runs somebody else (ADR-0005).
                Err(Stop::ThreadEnded) => {
                    self.switch_thread(log);
                    None
                }
                // Waiting for another thread: the machine leaves the call
                // where it is, and this thread makes it again when it comes
                // back round (ADR-0005).
                Err(Stop::Blocked) => {
                    self.switch_thread(log);
                    None
                }
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
        // A frame the guest finished is presented before the next instruction
        // runs, so what the host shows is what the guest had when it flipped.
        if let Some(frame) = self.state.present.take() {
            self.present_guest(&frame, host, log);
        }
        // The cycle counter a title reads with `rdtsc` is the declared
        // clock's own, and the engine carries it on from here (#56).
        self.cpu.tsc = self.clock.cycles();
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

    /// How often the title put the pointer, and how often the host was told.
    #[must_use]
    pub fn pointer_puts(&self) -> (u64, u64) {
        (self.pointer_puts, self.pointer_puts_told)
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
        // Snapshots are taken here, in the loop every run goes through, so a
        // run that keeps them is the same machine as one that does not
        // (ADR-0005, #94). They are off unless asked for: scrubbability costs
        // nothing until a person wants it.
        let every = self.snapshot_every.map(|n| n.max(1));
        let mut next_snapshot = every;
        let outcome = loop {
            if self.clock.cursor() >= budget {
                break Outcome::BudgetExhausted;
            }
            if let Some(at) = next_snapshot.filter(|at| self.flips >= *at) {
                match self.snapshot() {
                    Ok(bytes) => {
                        let name = format!("flip-{}.snapshot", self.flips);
                        let _ = std::fs::write(log.dir().join(&name), bytes);
                        log.line(&format!("snapshot {name} cursor {}", self.clock.cursor()));
                    }
                    Err(fault) => log.line(&format!("snapshot refused: {fault}")),
                }
                next_snapshot = every.map(|every| at + every);
            }
            if let Some(outcome) = self.run_guest(host, log, budget) {
                break outcome;
            }
            if let Some(what) = self.deadlocked() {
                break Outcome::Wall { what };
            }
            // A burst is the declared quantum: whoever ran it hands over to
            // whoever is next (ADR-0005).
            self.switch_thread(log);
            // The mix reaches the host between flips as well as at them: a
            // title that goes a long way without flipping — loading a level,
            // or thinking hard about one — would otherwise hand over nothing
            // for as long as that took, and the device would run dry and
            // drop what came late.
            self.mix(host);
            if self.clock.cursor() >= budget {
                break Outcome::BudgetExhausted;
            }
            if let Some(outcome) = self.tick(host, log) {
                break outcome;
            }
        };
        self.mix(host);
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
            "heap {} handed out, {} live, {} freed that it never handed out",
            self.state.heap.extent(),
            self.state.heap.live_bytes(),
            self.state.foreign_frees
        ));
        log.line(&format!(
            "end {} cursor {} flips {} declared-ms {}",
            finish.outcome, finish.cursor, finish.flips, finish.declared_millis
        ));
        finish
    }
}

/// Whether a thread can be given the processor at this moment (ADR-0005).
fn runnable_at(thread: &crate::win32::GuestThread, now: u64) -> bool {
    !thread.ended && !thread.suspended && thread.wake_at <= now
}

/// What a boundary slot stands for, once it has been called: the shim or the
/// COM method behind it, or nothing written.
#[derive(Clone, Copy)]
enum Bound {
    Shim(Option<&'static win32::Shim>),
    Method(Option<&'static crate::com::Method>),
}

/// Where the guest's stack lives, and how much of it there is.
///
/// A declared value: the address is chosen, not measured, and it sits below
/// every image the roster's titles ask to be loaded at.
pub const STACK_TOP: u32 = 0x0020_0000;
pub const STACK_BYTES: u32 = 0x0010_0000;

/// Where the declared machine keeps the thread information block, which `fs`
/// is based at.
///
/// A declared value: a page of its own, above the stack and below every image
/// the roster's titles ask to be loaded at. It has to be somewhere the guest
/// can reach and nothing else claims, because the first thing an MSVC runtime
/// does is read and write `fs:[0]` (#54).
pub const TIB: u32 = 0x0020_0000;

/// The process block the thread block points at, on the page after it.
pub const PEB: u32 = 0x0020_1000;

/// What a title's declared process and thread are called, matching what
/// `GetCurrentProcessId` and `GetCurrentThreadId` answer with: a run must not
/// depend on which process happened to host it.
pub const PROCESS_ID: u32 = 0x0000_1234;
pub const THREAD_ID: u32 = 0x0000_1000;

/// Write a thread information block, wherever a thread's own one lives.
///
/// Each field is a declared value, and the ones this machine has no honest
/// answer for are left zero rather than invented (#54).
///
/// # Errors
/// When the arena will not take it.
pub fn write_tib(
    mem: &mut crate::guest::Arena,
    at: u32,
    stack_top: u32,
    stack_limit: u32,
    thread_id: u32,
) -> Result<(), crate::guest::Fault> {
    mem.write(at, &[0u8; 0x1000])?;
    // No handler is registered yet: the end of the chain is all ones, and the
    // guest's own prologue pushes the first frame in front of it.
    mem.write_u32(at, 0xffff_ffff)?;
    // The stack this thread was given, which a runtime checks itself against
    // when it grows one.
    mem.write_u32(at + 0x04, stack_top)?;
    mem.write_u32(at + 0x08, stack_limit)?;
    // A thread block points at itself, which is how code that has `fs` gets an
    // ordinary pointer to it.
    mem.write_u32(at + 0x18, at)?;
    mem.write_u32(at + 0x20, PROCESS_ID)?;
    mem.write_u32(at + 0x24, thread_id)?;
    // The slots `TlsAlloc` hands out sit in the block itself, where Windows
    // keeps them, and the pointer at 0x2c names them.
    mem.write_u32(at + 0x2c, at + 0x0e10)?;
    mem.write_u32(at + 0x30, PEB)?;
    Ok(())
}

impl Machine {
    /// Load a title's entry image out of its pressing and point the processor
    /// at it.
    ///
    /// # Errors
    /// When the rip is absent, the image is not the one the title declares,
    /// the image is a copy-protection wrapper, or it will not fit.
    pub fn load_title(&mut self, title: &Title) -> Result<(), Box<dyn std::error::Error>> {
        let mut iso = crate::disc::open_pressing(title.image.disc_dir, title.image.pressing)?;
        let bytes = title.image.read(&mut iso)?;

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
        // declared from the pressing itself rather than by running the
        // installer (#16).
        let mut iso = iso;
        self.state.installed_files = match title.installed {
            crate::title::Installed::Cabinet => {
                let header = iso.read("DATA1.HDR")?;
                self.tree.set_cabinet(&header, "DATA", title.install_dir)?
            }
            crate::title::Installed::Copied(from) => {
                self.tree.set_copied(&mut iso, from, title.install_dir)?
            }
        };
        self.tree.set_disc(iso);

        // What this title's installer would have left in the registry, which
        // is where a title looks to find itself and its disc.
        for (key, name, value) in title.registry {
            let mut bytes = value.as_bytes().to_vec();
            bytes.push(0);
            self.state
                .declare_registry(key, name, crate::win32::advapi32::REG_SZ, bytes);
        }

        let image = crate::pe::load(&bytes, &mut self.arena)?;
        // The guest starts where it was installed, as a player double-clicking
        // it would.
        self.state.current_directory = title.install_dir.to_owned();

        // Every binary the title ships beside its executable, loaded as guest
        // code (ADR-0002, #55). What the declared tree does not hold is
        // Microsoft's, and the boundary stands where its exports would be.
        for import in &image.imports {
            self.modules.load(
                &import.dll,
                title.install_dir,
                &mut self.tree,
                &mut self.arena,
            )?;
        }
        let mut bound = 0;
        for module in 0..self.modules.modules().len() {
            let (name, base) = {
                let m = &self.modules.modules()[module];
                (m.name.clone(), m.base)
            };
            let bytes = self
                .tree
                .read(&format!("{}\\{name}", title.install_dir))
                .unwrap_or_default();
            let imports = crate::pe::imports_of(&bytes, base)?;
            bound += imports.len();
            self.modules
                .bind(&imports, &mut self.boundary, &mut self.arena)?;
        }
        self.modules
            .bind(&image.imports, &mut self.boundary, &mut self.arena)?;
        let attach = self
            .modules
            .attach_program(image.base.wrapping_add(image.entry), &mut self.boundary);
        self.arena
            .write(crate::loader::ATTACH_PROGRAM, &attach)
            .map_err(crate::pe::PeError::Fault)?;
        self.bound_names = bound + image.imports.len();

        self.cpu = Cpu {
            // The guest starts in the loader's own program, which attaches
            // every module and jumps to the title's entry point.
            eip: crate::loader::ATTACH_PROGRAM,
            r: {
                let mut r = [0u32; 8];
                r[crate::cpu::ESP] = STACK_TOP - 16;
                r
            },
            ..Cpu::default()
        };
        self.image = Some(image);
        self.declare_tib()?;
        Ok(())
    }
}

/// A call's arguments, as a trace line writes them.
fn listed(args: &[u32]) -> String {
    args.iter()
        .map(|a| format!("{a:#x}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl Machine {
    /// What a shim or a method is handed: the machine, for the one call.
    fn ctx<'a>(&'a mut self, log: &'a mut RunLog) -> Ctx<'a> {
        Ctx {
            cpu: &mut self.cpu,
            mem: &mut self.arena,
            state: &mut self.state,
            title: self.title,
            log,
            tree: &mut self.tree,
            modules: &mut self.modules,
            clock_millis: self.clock.millis(),
            boundary: &mut self.boundary,
            call_guest: None,
            float_answer: None,
            answer_is_the_call_s: false,
        }
    }

    /// A call's four-byte arguments, from where on the stack they start.
    fn stack_args(&self, from: u32, count: u32) -> Result<Vec<u32>, Stop> {
        (0..count)
            .map(|i| self.arena.read_u32(from + i * 4).map_err(Stop::Fault))
            .collect()
    }

    /// Answer a call the guest made across the boundary.
    ///
    /// The arguments are where the calling convention says they are, the
    /// answer goes back in `eax`, and the stack is left as the real library
    /// would have left it.
    fn cross(&mut self, log: &mut RunLog) -> Result<(), Stop> {
        use crate::shim::{Entry, THUNK_BASE, THUNK_STRIDE};
        let addr = self.cpu.eip;
        let index = (addr.wrapping_sub(THUNK_BASE) / THUNK_STRIDE) as usize;
        let bound = match self.bound.get(index).copied().flatten() {
            Some(bound) => bound,
            None => {
                let bound = match self.boundary.at(addr) {
                    Some(Entry::Return) => return self.guest_returned(log),
                    // A thread's own function returned (ADR-0005). Its exit
                    // code is what it answered with.
                    Some(Entry::ThreadEnd) => {
                        let code = self.cpu.r[EAX];
                        let running = self.state.thread;
                        if let Some(thread) = self.state.threads.get_mut(running) {
                            thread.ended = true;
                            thread.code = code;
                        }
                        return Err(Stop::ThreadEnded);
                    }
                    // The exception walk ran out of handlers (#54).
                    Some(Entry::Unhandled { code }) => {
                        return Err(Stop::NotWritten {
                            what: format!(
                                "an exception of {code:#010x} was raised and nothing on the \
                                 guest's own handler chain took it"
                            ),
                            caller: self.cpu.eip,
                        })
                    }
                    // The loader's own program jumped here, which a module
                    // only makes it do by refusing to attach (#55).
                    Some(Entry::Attach { module }) => {
                        return Err(Stop::NotWritten {
                            what: format!(
                                "{module} did not attach: its DllMain answered zero, and a \
                                 module that says it is not there is not called into"
                            ),
                            caller: self.cpu.eip,
                        })
                    }
                    Some(Entry::Method { interface, slot }) => {
                        Bound::Method(win32::com_method(interface, *slot))
                    }
                    Some(Entry::Import(import)) => {
                        Bound::Shim(win32::find(&import.dll, &import.name))
                    }
                    None => {
                        return Err(Stop::Undecoded {
                            eip: addr,
                            byte: 0,
                            group: None,
                            why: "an address inside the boundary that no import was bound to",
                        })
                    }
                };
                if self.bound.len() <= index {
                    self.bound.resize(index + 1, None);
                }
                self.bound[index] = Some(bound);
                bound
            }
        };
        let found = match bound {
            Bound::Method(method) => {
                let Some(Entry::Method { interface, slot }) = self.boundary.at(addr) else {
                    unreachable!("a slot bound to a method is one");
                };
                return self.cross_com(interface, *slot, method, log);
            }
            Bound::Shim(shim) => shim,
        };
        let boundary = &self.boundary;
        let what = || match boundary.at(addr) {
            Some(Entry::Import(import)) => format!("{}!{}", import.dll, import.name),
            _ => String::new(),
        };
        let esp = self.cpu.r[crate::cpu::ESP];
        let caller = self.arena.read_u32(esp).map_err(Stop::Fault)?;

        let Some(shim) = found else {
            let cursor = self.clock.cursor();
            log.at(Trace::Shims, || {
                format!("wall {} from {caller:#010x} cursor {cursor}", what())
            });
            return Err(Stop::NotWritten {
                what: what(),
                caller,
            });
        };

        let args = self.stack_args(esp + 4, shim.args)?;
        let cursor = self.clock.cursor();
        log.at(Trace::Shims, || {
            format!(
                "call {}({}) from {caller:#010x} cursor {cursor}",
                what(),
                listed(&args),
            )
        });

        let mut ctx = self.ctx(log);
        let answer = match (shim.run)(&mut ctx, &args) {
            // A shim that stops the run cannot see who called it.
            Err(Stop::NotWritten { what, caller: 0 }) => {
                return Err(Stop::NotWritten { what, caller });
            }
            other => other?,
        };
        let call_guest = ctx.call_guest.take();
        let pass_through = ctx.answer_is_the_call_s;

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
        if let Some((function, args)) = call_guest {
            if pass_through {
                self.call_guest_for_its_answer(function, &args, caller)?;
            } else {
                self.call_guest(function, &args, caller, answer)?;
            }
            // A dialog that has just come up parks its first turn here; the
            // guest's own callbacks inside it are ordinary calls.
            if let Some(dialog) = self.state.dialog.as_mut().filter(|d| !d.parked) {
                dialog.parked = true;
                self.dialog_turns.push(self.continuations.len());
            }
        }
        // A call across the boundary costs the declared clock what an
        // instruction costs, so a title that spends its time in shims still
        // advances its own clock.
        self.clock.retire();
        self.charge_waiting();
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

        // What a person did that the guest has not taken yet: the devices'
        // state and the messages still waiting.
        let input = &self.state;
        for value in [
            input.pointer.0,
            input.pointer.1,
            input.last_move.0,
            input.last_move.1,
            input.mouse_delta.0,
            input.mouse_delta.1,
            // Both buttons in one number, the left one where it has always
            // been: a run that never touches the right button hashes to what
            // it hashed to before this machine had one (#61).
            i32::from(input.mouse_down) | i32::from(input.right_down) << 1,
        ] {
            hash.update(&value.to_le_bytes());
        }
        for key in &input.keys_down {
            hash.update(&key.to_le_bytes());
        }
        for m in &input.messages {
            for value in [m.window, m.message, m.wparam, m.lparam, m.time] {
                hash.update(&value.to_le_bytes());
            }
        }

        // The regions anything writes to, which are the ones a snapshot
        // carries.
        for (start, len) in self.snapshot_regions() {
            if let Ok(bytes) = self.arena.slice(start, len as usize) {
                hash.update(bytes);
            }
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

    /// The regions of guest memory a snapshot carries, and the digest hashes:
    /// where the image and every binary the title ships were placed — whose
    /// `.data` is where Alpha Team keeps most of itself (#55, #94) — how far
    /// the heap has grown, the stack down to where it stands, and the surface.
    ///
    // ponytail: the declared regions rather than every page. A guest that
    // wrote outside its own images, heap, stack and surface would resume with
    // that writing lost. Upgrade path: a dirty-page map in the arena, which
    // the engine's unchecked path would have to go through.
    fn snapshot_regions(&self) -> Vec<(u32, u32)> {
        let mut regions = Vec::new();
        if let Some(image) = &self.image {
            regions.push((image.base, image.size));
        }
        for module in self.modules.modules() {
            regions.push((module.base, module.size));
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

impl Machine {
    /// Answer a call the guest made through a COM object's own table.
    fn cross_com(
        &mut self,
        interface: &'static str,
        slot: usize,
        method: Option<&'static crate::com::Method>,
        log: &mut RunLog,
    ) -> Result<(), Stop> {
        let esp = self.cpu.r[crate::cpu::ESP];
        let caller = self.arena.read_u32(esp).map_err(Stop::Fault)?;
        // Every COM method is called with the object first.
        let this = self.arena.read_u32(esp + 4).map_err(Stop::Fault)?;

        let Some(method) = method else {
            return Err(Stop::NotWritten {
                what: format!("{interface} slot {slot}"),
                caller,
            });
        };

        let args = self.stack_args(esp + 8, method.args)?;
        let cursor = self.clock.cursor();
        log.at(Trace::Shims, || {
            format!(
                "call {interface}::{}({}) on {this:#010x} from {caller:#010x} cursor {cursor}",
                method.name,
                listed(&args),
            )
        });

        let mut ctx = self.ctx(log);
        let answer = match (method.run)(&mut ctx, this, &args) {
            // A slot the header declares and nothing has written: the wall
            // names it the way the interface does.
            Err(Stop::NotWritten { what, .. }) if what.is_empty() => {
                return Err(Stop::NotWritten {
                    what: format!("{interface}::{}", method.name),
                    caller,
                });
            }
            Err(Stop::NotWritten { what, caller: 0 }) => {
                return Err(Stop::NotWritten { what, caller });
            }
            other => other?,
        };
        let call_guest = ctx.call_guest.take();
        let pass_through = ctx.answer_is_the_call_s;
        if let Some(value) = ctx.float_answer.take() {
            self.cpu.fpu.push(f64::from(value));
        }

        // Every COM method is stdcall, and the object counts as an argument.
        self.cpu.r[crate::cpu::ESP] = esp.wrapping_add(4 + (method.args + 1) * 4);
        self.cpu.r[crate::cpu::EAX] = answer;
        self.cpu.eip = caller;
        if let Some((function, args)) = call_guest {
            let stack = self.cpu.r[crate::cpu::ESP];
            if pass_through {
                self.call_guest_for_its_answer(function, &args, caller)?;
            } else {
                self.call_guest(function, &args, caller, answer)?;
            }
            // A render pass that has just begun parks its first turn here.
            if let Some(pass) = self.state.render.as_mut().filter(|p| !p.parked) {
                pass.parked = true;
                self.render_turns.push((self.continuations.len(), stack));
            }
        }
        self.clock.retire();
        self.charge_waiting();
        Ok(())
    }

    /// One of a render pass's calls into the title has returned: the next
    /// one, or the pass is over and `Render` returns.
    fn render_turn(&mut self, stack: u32) -> Result<(), Stop> {
        self.cpu.r[crate::cpu::ESP] = stack;
        match crate::win32::d3drm::render_turn(&mut self.state, self.cpu.r[EAX]) {
            Some((function, args)) => {
                self.clock.retire();
                self.push_call(function, &args)
            }
            None => {
                self.render_turns.pop();
                // A call the machine only arranged answers for itself.
                let pass_through = self.pass_through.last() == Some(&self.continuations.len());
                if pass_through {
                    self.pass_through.pop();
                }
                let Some((resume_at, answer, _)) = self.continuations.pop() else {
                    return Err(Stop::NotWritten {
                        what: "a render pass ended with no call waiting for it".to_owned(),
                        caller: self.cpu.eip,
                    });
                };
                self.cpu.r[EAX] = answer;
                self.cpu.eip = resume_at;
                self.clock.retire();
                Ok(())
            }
        }
    }

    /// A modal dialog's procedure has returned from a message.
    ///
    /// If it ended the dialog, `DialogBoxParamA` returns what it ended with.
    /// If not, the dialog is still up, and the next thing that happens to it
    /// is what a person pressing Enter does: the default button, `IDOK`.
    fn dialog_turn(&mut self, log: &mut RunLog) -> Result<(), Stop> {
        let Some(dialog) = self.state.dialog.as_mut() else {
            self.dialog_turns.pop();
            return self.guest_returned(log);
        };
        dialog.turns += 1;
        if let Some(result) = dialog.ended {
            log.line(&format!("dialog ended with {result}"));
            self.state.dialog = None;
            self.dialog_turns.pop();
            let Some((resume_at, _, _)) = self.continuations.pop() else {
                return Err(Stop::NotWritten {
                    what: "a dialog ended with no call waiting for it".to_owned(),
                    caller: self.cpu.eip,
                });
            };
            self.cpu.r[EAX] = result;
            self.cpu.eip = resume_at;
            self.clock.retire();
            return Ok(());
        }
        if dialog.turns > 1 {
            return Err(Stop::NotWritten {
                what: "USER32.dll!DialogBoxParamA: the dialog did not end when its default \
                       button was pressed"
                    .to_owned(),
                caller: self.cpu.eip,
            });
        }
        if dialog.disabled.contains(&1) {
            return Err(Stop::NotWritten {
                what: "USER32.dll!DialogBoxParamA: the dialog's default button is disabled, \
                       so nobody can press it"
                    .to_owned(),
                caller: self.cpu.eip,
            });
        }
        log.line("dialog default button pressed");
        let (window, procedure) = (dialog.window, dialog.procedure);
        // WM_COMMAND, IDOK clicked, from the button itself.
        // The dialog's parked call stays where it is: this turn comes back to
        // it too.
        self.push_call(
            procedure,
            &[window, 0x0111, 1, crate::win32::user32::control(1)],
        )?;
        self.clock.retire();
        Ok(())
    }

    /// Run the guest's own code until it calls across the boundary, the budget
    /// is spent, or a burst ends, without the checks `tick` makes before each
    /// instruction.
    ///
    /// Nothing but the engine has anything to do between calls out: a frame to
    /// present only appears when a shim runs, and a window closed mid-burst is
    /// noticed at its end. What the guest does is the same either way; only
    /// the host's time is saved.
    fn run_guest(&mut self, host: &mut dyn Host, log: &mut RunLog, budget: u64) -> Option<Outcome> {
        const BURST: u64 = 1 << 16;
        if self.stub || log.trace() >= Trace::Insns || self.state.present.is_some() {
            return None;
        }
        let count = budget.min(self.clock.cursor() + BURST) - self.clock.cursor().min(budget);
        self.cpu.tsc = self.clock.cycles();
        let (retired, result) =
            self.engine
                .run(&mut self.cpu, &mut self.arena, count, Boundary::RANGE);
        self.clock.retire_many(retired);
        match result {
            Ok(()) => host.closed().then_some(Outcome::Closed),
            Err(Stop::Interrupt(n)) => Some(Outcome::Wall {
                what: format!("int {n:#04x}"),
            }),
            Err(stop) => Some(Outcome::Wall {
                what: stop.to_string(),
            }),
        }
    }

    /// Give the next runnable guest thread the processor.
    ///
    /// One host thread runs them all, so switching is putting the registers
    /// back where the parked thread left them. It happens at the end of a
    /// burst — the declared quantum — and when a thread ends, and nowhere
    /// else: a schedule that depended on the host's own threads would make
    /// every run of this title a different one (ADR-0005).
    fn switch_thread(&mut self, log: &mut RunLog) {
        if self.state.threads.len() < 2 {
            return;
        }
        let running = self.state.thread;
        self.state.threads[running].cpu = self.cpu;
        let count = self.state.threads.len();
        // A thread that asked to be woken later is not run until then, and if
        // every thread is asleep the clock goes to the earliest of their hours
        // rather than the machine spinning between them.
        let now = self.clock.millis();
        if !self.state.threads.iter().any(|t| runnable_at(t, now)) {
            let due = self
                .state
                .threads
                .iter()
                .filter(|t| !t.ended && !t.suspended)
                .map(|t| t.wake_at)
                .min();
            // A wait with no timeout has no hour: only a signal lifts it, so
            // there is nothing for the clock to go to (#60).
            if let Some(due) = due.filter(|d| *d > now && *d != u64::MAX) {
                self.clock.wait(due - now);
            }
        }
        let now = self.clock.millis();
        let next = (1..=count)
            .map(|step| (running + step) % count)
            .find(|i| runnable_at(&self.state.threads[*i], now));
        let Some(next) = next else {
            // Nothing else can run; whoever was running carries on, and if it
            // has ended too the run is over by its own exit.
            return;
        };
        if next == running {
            return;
        }
        self.state.thread = next;
        self.cpu = self.state.threads[next].cpu;
        log.at(Trace::Shims, || {
            format!(
                "thread {} takes over at {:#010x} cursor {}",
                self.state.threads[next].id,
                self.cpu.eip,
                self.clock.cursor()
            )
        });
    }

    /// Whether every thread left is waiting for another with no hour to be
    /// woken at, which nothing can move on (ADR-0005, #60).
    ///
    /// Without this a deadlock is a machine that looks busy for ever: each
    /// thread takes its turn, makes its wait again, hands the turn back, and
    /// the cursor never moves.
    fn deadlocked(&self) -> Option<String> {
        let live: Vec<_> = self.state.threads.iter().filter(|t| !t.ended).collect();
        if live.len() < 2 || !live.iter().all(|t| t.suspended || t.wake_at == u64::MAX) {
            return None;
        }
        Some(format!(
            "every thread is waiting for another: {}",
            live.iter()
                .map(|t| format!(
                    "thread {} {}",
                    t.id,
                    if t.suspended {
                        "suspended".to_owned()
                    } else {
                        format!("on handle {:#x}", t.waiting_on)
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// Hand the host every block of the mix the declared clock has passed.
    /// Mixing runs whether or not a device takes the samples.
    fn mix(&mut self, host: &mut dyn Host) {
        for block in self.state.mixer.due(self.clock.nanos(), &self.arena) {
            host.audio(&block);
        }
    }

    /// What a call spent waiting becomes declared time.
    fn charge_waiting(&mut self) {
        let millis = std::mem::take(&mut self.state.waited_millis);
        if millis > 0 {
            self.clock.wait(millis);
        }
    }
}

impl Machine {
    /// Run the guest's own code from inside a call it made.
    ///
    /// The machine cannot hold a Rust frame across guest execution, so what
    /// happens instead is a parked continuation: the call is set up on the
    /// guest's stack with a return address inside the boundary, and when the
    /// guest returns to it the machine puts back what the original call was
    /// going to answer with (ADR-0002).
    fn call_guest(
        &mut self,
        function: u32,
        args: &[u32],
        resume_at: u32,
        answer: u32,
    ) -> Result<(), Stop> {
        let before = self.cpu.r[crate::cpu::ESP];
        self.push_call(function, args)?;
        self.continuations.push((resume_at, answer, before));
        Ok(())
    }

    /// Run guest code and give the guest what it answers with, rather than
    /// what the shim answered (#60).
    fn call_guest_for_its_answer(
        &mut self,
        function: u32,
        args: &[u32],
        resume_at: u32,
    ) -> Result<(), Stop> {
        let before = self.cpu.r[crate::cpu::ESP];
        self.push_call(function, args)?;
        self.pass_through.push(self.continuations.len());
        self.continuations.push((resume_at, 0, before));
        Ok(())
    }

    /// Set a call up on the guest's own stack, arguments last first, returning
    /// into the machine.
    fn push_call(&mut self, function: u32, args: &[u32]) -> Result<(), Stop> {
        let mut esp = self.cpu.r[crate::cpu::ESP];
        for value in args.iter().rev() {
            esp = esp.wrapping_sub(4);
            self.arena.write_u32(esp, *value).map_err(Stop::Fault)?;
        }
        esp = esp.wrapping_sub(4);
        self.arena
            .write_u32(esp, crate::shim::RETURN_TO_MACHINE)
            .map_err(Stop::Fault)?;
        self.cpu.r[crate::cpu::ESP] = esp;
        self.cpu.eip = function;
        Ok(())
    }

    /// The guest has returned from something the machine called.
    fn guest_returned(&mut self, log: &mut RunLog) -> Result<(), Stop> {
        if self.dialog_turns.last() == Some(&self.continuations.len()) {
            return self.dialog_turn(log);
        }
        if let Some(&(_, stack)) = self
            .render_turns
            .last()
            .filter(|(depth, _)| *depth == self.continuations.len())
        {
            return self.render_turn(stack);
        }
        // Calls the guest never came back from: an exception handler that goes
        // to its catch leaves the stack above the frame the call was made
        // from, and what was waiting for it is never going to happen. A call
        // that returned the ordinary way leaves the stack exactly where it
        // was, whoever cleaned the arguments (#54).
        let esp = self.cpu.r[crate::cpu::ESP];
        while self
            .continuations
            .last()
            .is_some_and(|(_, _, at)| *at < esp)
        {
            self.continuations.pop();
            self.pass_through.retain(|d| *d < self.continuations.len());
            log.line("a parked call was abandoned by an unwind");
        }
        // A call the machine only arranged answers for itself.
        let pass_through = self.pass_through.last() == Some(&self.continuations.len());
        if pass_through {
            self.pass_through.pop();
        }
        let Some((resume_at, answer, _)) = self.continuations.pop() else {
            return Err(Stop::NotWritten {
                what: "a return into the machine with nothing waiting for it".to_owned(),
                caller: self.cpu.eip,
            });
        };
        let (eax, cursor) = (self.cpu.r[EAX], self.clock.cursor());
        log.at(Trace::Shims, || {
            format!("guest returned {eax:#x} cursor {cursor}")
        });
        // The callback's own answer is discarded: what the guest is waiting
        // for is the answer to the call it made — unless the call was the
        // answer, which is what a COM server the title ships hands back.
        if !pass_through {
            self.cpu.r[EAX] = answer;
        }
        self.cpu.eip = resume_at;
        self.clock.retire();
        Ok(())
    }
}

impl Machine {
    /// Show the guest a frame it has finished with.
    ///
    /// The surface is the guest's own memory, in whatever format it declared,
    /// and the host takes four bytes a pixel: the conversion is this machine's
    /// last act before the frame leaves it.
    fn present_guest(
        &mut self,
        frame: &crate::win32::Present,
        host: &mut dyn Host,
        log: &mut RunLog,
    ) {
        let want = (frame.width * frame.height * 4) as usize;
        self.present_buffer.resize(want, 0);
        let source = match self
            .arena
            .slice(frame.pixels, (frame.pitch * frame.height) as usize)
        {
            Ok(bytes) => bytes,
            Err(fault) => {
                log.line(&format!("flip refused: {fault}"));
                return;
            }
        };

        match frame.depth {
            32 => {
                for y in 0..frame.height as usize {
                    let from = y * frame.pitch as usize;
                    let to = y * frame.width as usize * 4;
                    let width = frame.width as usize * 4;
                    self.present_buffer[to..to + width]
                        .copy_from_slice(&source[from..from + width]);
                }
            }
            16 => {
                // Five-six-five, which is what a title of this era declares.
                for y in 0..frame.height as usize {
                    for x in 0..frame.width as usize {
                        let at = y * frame.pitch as usize + x * 2;
                        let pixel = u16::from_le_bytes([source[at], source[at + 1]]);
                        let r = ((pixel >> 11) & 0x1f) as u8;
                        let g = ((pixel >> 5) & 0x3f) as u8;
                        let b = (pixel & 0x1f) as u8;
                        let out = (y * frame.width as usize + x) * 4;
                        // Widened by repeating the top bits, so white stays
                        // white rather than landing just short of it.
                        self.present_buffer[out] = (b << 3) | (b >> 2);
                        self.present_buffer[out + 1] = (g << 2) | (g >> 4);
                        self.present_buffer[out + 2] = (r << 3) | (r >> 2);
                        self.present_buffer[out + 3] = 0xff;
                    }
                }
            }
            8 => {
                // Through the palette the guest set. A screen it has not given
                // a palette to is black: there are no colours to read yet, and
                // reading them from wherever address zero lands would paint
                // the screen one nobody chose.
                let palette = self.state.palette;
                for y in 0..frame.height as usize {
                    for x in 0..frame.width as usize {
                        let index = source[y * frame.pitch as usize + x];
                        let entry = if palette == 0 {
                            0
                        } else {
                            self.arena
                                .read_u32(palette + u32::from(index) * 4)
                                .unwrap_or(0)
                        };
                        let out = (y * frame.width as usize + x) * 4;
                        let [r, g, b, _] = entry.to_le_bytes();
                        self.present_buffer[out] = b;
                        self.present_buffer[out + 1] = g;
                        self.present_buffer[out + 2] = r;
                        self.present_buffer[out + 3] = 0xff;
                    }
                }
            }
            other => {
                log.line(&format!(
                    "flip refused: {other} bits a pixel is not a declared format"
                ));
                return;
            }
        }

        let out = Frame {
            width: frame.width,
            height: frame.height,
            pitch: frame.width * 4,
            pixels: &self.present_buffer,
        };
        host.present(&out);
        if self.screenshots.wants(self.flips) {
            // A picture of exactly what the host was handed, in the run
            // directory beside the log, named with ten digits so a listing
            // sorts by flip.
            let dir = log.dir().join("shots");
            let png = crate::png::encode(frame.width, frame.height, &self.present_buffer);
            let written = std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(dir.join(format!("{:010}.png", self.flips)), png));
            if let Err(e) = written {
                log.line(&format!("screenshot {} refused: {e}", self.flips));
            }
        }
        self.clock.present(self.flip_cost);
        log.line(&format!(
            "flip {} cursor {} {}x{}x{}",
            self.flips,
            self.clock.cursor(),
            frame.width,
            frame.height,
            frame.depth
        ));
        self.pace(host, log);
        // One pointer on the screen: the host stops drawing its own once the
        // title has said it draws one itself (ShowCursor's count).
        let shown = self.state.cursor_shown >= 0;
        if shown != self.pointer_shown {
            self.pointer_shown = shown;
            host.show_pointer(shown);
            // Which pointer a person is looking at, and so which of the two
            // ways of moving one is in force: where the title draws its own,
            // the host freezes the person's and steers by how far the mouse
            // went; where it does not, the pointer is the person's and its
            // position is its own. A run that feels wrong under the hand is
            // first of all a question of which of those it was in (#40, #64).
            log.line(&format!(
                "pointer flip {} {}",
                self.flips,
                if shown {
                    "the person's own"
                } else {
                    "the title draws it, so the host steers by the mouse"
                }
            ));
        }
        // And where the title put it, while the title is the one drawing a
        // pointer: the host measures the person's next move from there, and
        // the machine has to measure from there with it or the person's
        // movement is the difference between the two (#40, #64). Everywhere
        // else the pointer is the person's and the host was not told, so
        // neither of them restarts (#46, #59).
        if let Some((x, y)) = self.state.pointer_put.take() {
            self.pointer_puts += 1;
            if !shown {
                self.pointer_puts_told += 1;
                host.put_pointer(x, y);
                crate::win32::user32::set_pointer_back(&mut self.state, (x, y));
            }
        }
        self.mix(host);
        self.deliver_input(host, log);
        self.flips += 1;
    }
}

impl Machine {
    /// Whether the processor is running somewhere that is not the guest's own
    /// code.
    ///
    /// The image it loaded, every binary the title ships beside it, the
    /// loader's own program that attaches them and is where every run starts
    /// (#55), and the boundary it calls across, are the only places a healthy
    /// run ever executes. Anything else is a return address that was not one.
    #[must_use]
    pub fn is_wild(&self) -> bool {
        // The stub guest is its own small thing and is never wild.
        let Some(image) = &self.image else {
            return false;
        };
        let eip = self.cpu.eip;
        let inside = |base: u32, size: u32| eip.wrapping_sub(base) < size;
        let modules = self.modules.modules();
        !(crate::shim::Boundary::in_range(eip)
            || inside(image.base, image.size)
            || inside(crate::loader::ATTACH_PROGRAM, 0x1000)
            || modules.iter().any(|m| inside(m.base, m.size)))
    }
}
