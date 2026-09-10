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
/// A gigabyte: the heap alone is half of it, and an anonymous mapping costs
/// only the pages something writes to, so the rest is free until it is used.
pub const ARENA_BYTES: usize = 1024 * 1024 * 1024;

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
    /// Where a guest frame is turned into something the host can show.
    present_buffer: Vec<u8>,
    /// Calls the machine made into guest code, waiting to come back.
    continuations: Vec<(u32, u32)>,
    /// Which parked calls are a modal dialog's turns, by their depth.
    dialog_turns: Vec<usize>,
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
            present_buffer: Vec::new(),
            continuations: Vec::new(),
            dialog_turns: Vec::new(),
            render_turns: Vec::new(),
            input: Vec::new(),
            route: None,
            recording: None,
            screenshots: crate::cli::Screenshots::default(),
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

        self.deliver_input(host, log);
        self.flips += 1;
    }

    /// Hand the guest what a person did while the frame was on screen, at the
    /// flip boundary (#10).
    ///
    /// A replaying run takes its input from the route rather than from the
    /// person, and the person's own input is not mixed in: a route replays
    /// what it says and nothing else.
    fn deliver_input(&mut self, host: &mut dyn Host, log: &mut RunLog) {
        let events = match &self.route {
            Some(route) => {
                let _ = host.poll_input(self.flips);
                route.at(self.flips)
            }
            None => host.poll_input(self.flips),
        };
        let millis = u32::try_from(self.clock.millis() & 0xffff_ffff).unwrap_or(0);
        for event in events {
            log.line(&format!("input flip {} {event:?}", self.flips));
            if let Some(recording) = &mut self.recording {
                recording.push(self.flips, event);
            }
            self.input.push((self.flips, event));
            crate::win32::user32::deliver(&mut self.state, event, millis);
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
        // A frame the guest finished is presented before the next instruction
        // runs, so what the host shows is what the guest had when it flipped.
        if let Some(frame) = self.state.present.take() {
            self.present_guest(&frame, host, log);
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
            if let Some(outcome) = self.run_guest(host, log, budget) {
                break outcome;
            }
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
        let entry = self.boundary.at(addr).cloned();
        if let Some(crate::shim::Entry::Return) = entry {
            return self.guest_returned(log);
        }
        if let Some(crate::shim::Entry::Method { interface, slot }) = entry {
            return self.cross_com(interface, slot, log);
        }
        let Some(crate::shim::Entry::Import(import)) = entry else {
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
            boundary: &mut self.boundary,
            call_guest: None,
            float_answer: None,
        };
        let answer = match (shim.run)(&mut ctx, &args) {
            // A shim that stops the run cannot see who called it.
            Err(Stop::NotWritten { what, caller: 0 }) => {
                return Err(Stop::NotWritten { what, caller });
            }
            other => other?,
        };
        let call_guest = ctx.call_guest.take();

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
            self.call_guest(function, &args, caller, answer)?;
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
            i32::from(input.mouse_down),
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

impl Machine {
    /// Answer a call the guest made through a COM object's own table.
    fn cross_com(
        &mut self,
        interface: &'static str,
        slot: usize,
        log: &mut RunLog,
    ) -> Result<(), Stop> {
        let esp = self.cpu.r[crate::cpu::ESP];
        let caller = self.arena.read_u32(esp).map_err(Stop::Fault)?;
        // Every COM method is called with the object first.
        let this = self.arena.read_u32(esp + 4).map_err(Stop::Fault)?;

        let Some(method) = crate::win32::com_method(interface, slot) else {
            return Err(Stop::NotWritten {
                what: format!("{interface} slot {slot}"),
                caller,
            });
        };

        let mut args = Vec::with_capacity(method.args as usize);
        for i in 0..method.args {
            args.push(self.arena.read_u32(esp + 8 + i * 4).map_err(Stop::Fault)?);
        }
        log.at(
            Trace::Shims,
            &format!(
                "call {interface}::{}({}) on {this:#010x} from {caller:#010x} cursor {}",
                method.name,
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
            boundary: &mut self.boundary,
            call_guest: None,
            float_answer: None,
        };
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
        if let Some(value) = ctx.float_answer.take() {
            self.cpu.fpu.push(f64::from(value));
        }

        // Every COM method is stdcall, and the object counts as an argument.
        self.cpu.r[crate::cpu::ESP] = esp.wrapping_add(4 + (method.args + 1) * 4);
        self.cpu.r[crate::cpu::EAX] = answer;
        self.cpu.eip = caller;
        if let Some((function, args)) = call_guest {
            let stack = self.cpu.r[crate::cpu::ESP];
            self.call_guest(function, &args, caller, answer)?;
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
                let Some((resume_at, answer)) = self.continuations.pop() else {
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
            let Some((resume_at, _)) = self.continuations.pop() else {
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
        let end = budget.min(self.clock.cursor() + BURST);
        while self.clock.cursor() < end && !Boundary::in_range(self.cpu.eip) {
            match self.engine.step(&mut self.cpu, &mut self.arena) {
                Ok(()) => self.clock.retire(),
                Err(Stop::Interrupt(n)) => {
                    self.clock.retire();
                    return Some(Outcome::Wall {
                        what: format!("int {n:#04x}"),
                    });
                }
                Err(stop) => {
                    return Some(Outcome::Wall {
                        what: stop.to_string(),
                    })
                }
            }
        }
        host.closed().then_some(Outcome::Closed)
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
        self.push_call(function, args)?;
        self.continuations.push((resume_at, answer));
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
        let Some((resume_at, answer)) = self.continuations.pop() else {
            return Err(Stop::NotWritten {
                what: "a return into the machine with nothing waiting for it".to_owned(),
                caller: self.cpu.eip,
            });
        };
        log.at(
            Trace::Shims,
            &format!(
                "guest returned {:#x} cursor {}",
                self.cpu.r[EAX],
                self.clock.cursor()
            ),
        );
        // The callback's own answer is discarded: what the guest is waiting
        // for is the answer to the call it made.
        self.cpu.r[EAX] = answer;
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
                // Through the palette the guest set.
                let palette = self.state.palette;
                for y in 0..frame.height as usize {
                    for x in 0..frame.width as usize {
                        let index = source[y * frame.pitch as usize + x];
                        let entry = self
                            .arena
                            .read_u32(palette + u32::from(index) * 4)
                            .unwrap_or(0);
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
        host.pace(self.clock.nanos());
        self.mix(host);
        self.deliver_input(host, log);
        self.flips += 1;
    }
}

impl Machine {
    /// Whether the processor is running somewhere that is not the guest's own
    /// code.
    ///
    /// The image it loaded, and the boundary it calls across, are the only two
    /// places a healthy run ever executes. Anything else is a return address
    /// that was not one.
    #[must_use]
    pub fn is_wild(&self) -> bool {
        if crate::shim::Boundary::in_range(self.cpu.eip) {
            return false;
        }
        match &self.image {
            Some(image) => {
                let eip = self.cpu.eip;
                eip < image.base || eip >= image.base.wrapping_add(image.size)
            }
            // The stub guest is its own small thing and is never wild.
            None => false,
        }
    }
}
