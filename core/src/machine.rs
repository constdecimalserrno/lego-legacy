//! One emulator instance: an arena, a processor, a clock, and the loop that
//! spends them.

use crate::cli::Trace;
use crate::clock::{Clock, FlipCost};
use crate::cpu::{interp::Interp, Cpu, CpuEngine, Mode, Stop, EAX, REG_NAMES};
use crate::guest::{Arena, Fault, GuestMem, DOS_MEMORY, NULL_PAGE};
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
    /// The person stopped the run (#110, #140): they closed the window, or
    /// ended the run from its terminal or console, as [`crate::host::listen`]
    /// hears, twice, or once with no window to tell, or once and the title
    /// was told and had not left within [`CLOSE_GRACE_MILLIS`].
    Closed,
    /// The person quit the debugger before the guest stopped.
    Quit,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited { code } => write!(f, "exited {code}"),
            Self::Wall { what } => write!(f, "wall: {what}"),
            Self::BudgetExhausted => write!(f, "budget exhausted"),
            Self::Closed => write!(f, "stopped by the person"),
            Self::Quit => write!(f, "quit from the debugger"),
        }
    }
}

/// How long a title is given to leave once it has been told the person
/// closed its window, in declared milliseconds, before the run is stopped as
/// the person stopping it (#140). Declared, so that a route that closes ends
/// on the same instruction every time it is replayed.
pub const CLOSE_GRACE_MILLIS: u64 = 5_000;

/// How long a frame stands, in declared milliseconds, before the host is
/// asked at the title's next input poll for what a person did, and again
/// each time as long again goes by before the next flip (#215, ADR-0003).
/// Declared time, so the waits a frame makes count, its `Sleep`s among them.
///
/// A tenth of a second, the maintainer's choice of 2026-10-22: a title waiting
/// on a key without flipping, as LEGO Chess waits after a movie it skipped,
/// hears a person within it. In the released titles' gate runs, measured in
/// declared time with their waits, it is met only where a frame waits or
/// loads: in Rock Raiders' frame that ends a load, and in each of the 35
/// frames of its boot splash, which `Sleep(100)` before they poll; and in Hot
/// Wheels' boot and its race's finish. Alpha Team and LEGO Island poll at the
/// start of each frame, before they wait, so their ordinary frames are never
/// asked. Once a refresh would hand Hot Wheels a person's input mid-frame in
/// most of its race frames, which a person would have to play first, and it
/// is no one constant: each title declares its own refresh. It moves only
/// where a person's input lands, never what a route replays.
pub const ASK_BETWEEN_FLIPS_MILLIS: u64 = 100;

/// A run that has ended, and what it spent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finish {
    pub outcome: Outcome,
    pub cursor: u64,
    pub flips: u64,
    pub declared_millis: u64,
}

/// Flip-boundary observer callback (#309, #313): serves read-only introspection
/// as well as instant live `LLSNAP v2` snapshot/restore operations.
pub(crate) type FlipObserver = Box<dyn Fn(&mut Machine, &mut dyn Host)>;

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
    /// Everything delivered to the guest, in order, each at its flip and
    /// poll (#215).
    pub input: Vec<((u64, u64), crate::host::Input)>,
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
    /// The cursor the host was last told the title set, so that it is told
    /// each one once (#139).
    cursor_told: Option<u32>,
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
    /// called, by the slot, with precomputed flags and COM `(interface, slot)`
    /// embedded so hot crossings avoid secondary boundary and table lookups.
    bound: Vec<Bound>,
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
    /// The flip the next snapshot is due at, once a run has started.
    next_snapshot: Option<u64>,
    /// Where the machine is in its schedule (ADR-0005, #131).
    turn: Turn,
    /// The declared millisecond the run stops at as the person stopping it,
    /// once a close has been delivered (#140).
    close_by: Option<u64>,
    /// How many input polls the guest has made since the last flip (#215).
    polls: u64,
    /// The declared cycle from which the host is asked again at a poll: the
    /// frame has stood [`ASK_BETWEEN_FLIPS_MILLIS`] since the flip, or since
    /// the host was last asked (#215).
    ask_at: u64,
    /// How many of the route's events have been delivered (#215).
    delivered: usize,
    /// Point `(flip, poll)` of the next undelivered route event, `(0, 0)`
    /// before initial seeding, or `(u64::MAX, u64::MAX)` once none remain.
    next_route_at: (u64, u64),
    /// Earliest declared cycle at which the audio mixer has another frame due.
    next_mix_cycle: u64,
    /// The guest's recent clock reads, from which a clock spin is passed (#217).
    spins: crate::spin::Spins,
    /// Optional read-only host-side flip boundary observer (e.g. HTTP/UDS
    /// introspection and pause servicing, #309). Never mutates guest state.
    flip_observer: Option<FlipObserver>,
}

/// Where the machine is in the one schedule every run keeps (#131).
#[derive(Clone, Copy)]
enum Turn {
    /// A burst is about to begin.
    Top,
    /// A burst of the running thread's own code is under way, and it ends at
    /// this cursor if no call across the boundary ends it first.
    Burst(u64),
    /// The burst is over and the switch made: the thread now running makes
    /// the call, or retires the instruction, it stands on.
    Switched,
}

impl Machine {
    /// Build a machine for a title, with the interpreter and an empty arena.
    ///
    /// # Errors
    /// When the host cannot map the arena.
    pub fn new(title: &'static Title, host: &mut dyn Host) -> Result<Self, crate::host::HostError> {
        Ok(Self {
            arena: Arena::new(host, ARENA_BYTES)?.with_floor(DOS_MEMORY),
            cpu: Cpu::default(),
            clock: Clock::at(title.declared.hz),
            engine: Box::new(Interp::new(Mode {
                processor: title.declared.processor,
                ..Mode::FLAT
            })),
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
            cursor_told: None,
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
            next_snapshot: None,
            turn: Turn::Top,
            close_by: None,
            polls: 0,
            ask_at: 0,
            delivered: 0,
            next_route_at: (0, 0),
            next_mix_cycle: 0,
            spins: crate::spin::Spins::default(),
            flip_observer: None,
        })
    }

    /// Install a read-only flip-boundary observer (strictly outside guest state, #309).
    pub(crate) fn set_flip_observer(&mut self, observer: FlipObserver) {
        self.flip_observer = Some(observer);
    }

    /// The declared stub/direct surface, if any.
    #[must_use]
    pub fn surface(&self) -> Option<Surface> {
        self.surface
    }

    /// The title this machine was built for.
    #[must_use]
    pub fn title(&self) -> &'static Title {
        self.title
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

    /// Instructions the machine retired by passing clock spins (#217): ones
    /// the cursor counts and no engine ran.
    #[must_use]
    pub fn instructions_passed(&self) -> u64 {
        self.spins.instructions()
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

        self.observe_flip(host);
        self.deliver_input(host, log);
        self.flips += 1;
    }

    /// Notify the host of flip metadata after audio mixing and immediately
    /// before input delivery, strictly outside guest state (#296).
    fn observe_flip(&mut self, host: &mut dyn Host) {
        let route_remaining = self.undelivered();
        host.observe_flip(&crate::host::FlipMeta {
            flip: self.flips,
            clock_nanos: self.clock.nanos(),
            cursor: self.clock.cursor(),
            instructions_passed: self.instructions_passed(),
            pointer_shown: self.pointer_shown,
            pointer_pos: self.state.pointer,
            route_active: route_remaining > 0,
            route_remaining,
        });
        if let Some(observer) = self.flip_observer.take() {
            observer(self, host);
            if self.flip_observer.is_none() {
                self.flip_observer = Some(observer);
            }
        }
    }

    /// Synchronize host-side pointer visibility/cursor shape after an in-place
    /// live snapshot restore (#313).
    pub(crate) fn sync_host_after_restore(&self, host: &mut dyn Host) {
        host.show_pointer(self.pointer_shown);
        if let Some(cursor) = self.cursor_told {
            host.pointer_shape(crate::win32::user32::shape(&self.state, cursor).as_ref());
        } else {
            host.pointer_shape(None);
        }
    }

    /// Latest converted BGRA framebuffer held by this machine (if any guest flip
    /// has presented), paired with its `(width, height)`.
    pub(crate) fn last_presented_bgra(&self) -> Option<(u32, u32, &[u8])> {
        let (w, h, _) = self.state.display_mode;
        let expected = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        if expected > 0 && self.present_buffer.len() >= expected {
            Some((w, h, &self.present_buffer[..expected]))
        } else {
            None
        }
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
    /// flip boundary (#10), and count the next frame's polls from here (#215).
    fn deliver_input(&mut self, host: &mut dyn Host, log: &mut RunLog) {
        let at = (self.flips, 0);
        let events = self.input_at(at, true, host);
        self.deliver(at, events, log);
        self.polls = 0;
        self.ask_again();
    }

    /// The guest is making an input poll: a call that reads the keyboard, the
    /// mouse or the message queue, about to answer (#215, ADR-0003). It is
    /// counted from the flip before it, and what is due by it is delivered
    /// first: the route's events, and what the host has once the frame has
    /// stood [`ASK_BETWEEN_FLIPS_MILLIS`] since the flip or since the host was
    /// last asked, the host holding the pace there as it does at a flip.
    // ponytail: from the first flip on. A poll before it is neither counted
    // nor handed anything, so a title that waited on a key before its first
    // frame would wait for ever; none of the five with a crate does.
    #[inline(always)]
    fn poll(&mut self, host: &mut dyn Host, log: &mut RunLog) {
        let Some(flip) = self.flips.checked_sub(1) else {
            return;
        };
        self.polls += 1;
        let at = (flip, self.polls);
        if self.clock.cycles() < self.ask_at && at < self.next_route_at {
            return;
        }
        self.poll_slow(at, host, log);
    }

    #[cold]
    #[inline(never)]
    fn poll_slow(&mut self, at: (u64, u64), host: &mut dyn Host, log: &mut RunLog) {
        let ask = self.clock.cycles() >= self.ask_at;
        if ask {
            self.ask_again();
            // A wait that sleeps runs ahead of the wall clock by a threshold
            // at most, rather than by however long the person held the key,
            // which the flip after it would sleep off with its window frozen.
            // ponytail: the host is asked before it is told of a put the
            // title made earlier in the frame, which it hears only at the
            // flip: `Steer` is drained against the place the title has left.
            // So a title that puts its pointer back every frame and polls a
            // threshold into one (Alpha Team, if a load of its polls late)
            // loses the move heard at the ask, or reads the frame's first
            // move twice, and its hand is held at the screen's edge until
            // the flip. Upgrade path, needed before any shorter threshold:
            // tell the host of a pending put before an ask, as
            // `present_guest` does, and keep `puts_back` across that drain.
            self.pace(host, log);
        }
        let events = self.input_at(at, ask, host);
        self.deliver(at, events, log);
    }

    /// Point of the next undelivered route event, or `(u64::MAX, u64::MAX)` if
    /// no route event remains.
    fn peek_next_route_at(&self) -> (u64, u64) {
        match &self.route {
            Some(route) if self.delivered < route.len() => route
                .due(self.delivered, (u64::MAX, u64::MAX))
                .first()
                .map_or((u64::MAX, u64::MAX), |(at, _)| *at),
            _ => (u64::MAX, u64::MAX),
        }
    }

    /// The host is next asked between flips once the frame has stood
    /// [`ASK_BETWEEN_FLIPS_MILLIS`] from now (#215).
    // ponytail: only at a poll. A frame that stands without polling pumps no
    // window and holds no pace until its flip, as before #215. Upgrade path:
    // ask at the burst ends too, where the mix already reaches the host.
    fn ask_again(&mut self) {
        self.ask_at = self.clock.cycles() + ASK_BETWEEN_FLIPS_MILLIS * self.clock.hz() / 1000;
    }

    /// What reaches the guest at a flip or a poll, with the host asked or not.
    ///
    /// A replaying run takes its input from the route rather than from the
    /// person, and the person's own input is not mixed in: a route replays
    /// what it says and nothing else. A route that has run out hands the
    /// machine to the person: what it was for was getting here, and from here
    /// on the mouse and the keyboard are theirs. Headless, the host has
    /// nothing to say and the run reads as it always did (#86).
    fn input_at(
        &mut self,
        at: (u64, u64),
        ask: bool,
        host: &mut dyn Host,
    ) -> Vec<crate::host::Input> {
        let heard = if ask {
            host.poll_input(at.0)
        } else {
            Vec::new()
        };
        let prev_delivered = self.delivered;
        let mut events = match &self.route {
            Some(route) if self.delivered < route.len() => {
                let due = route.due(self.delivered, at);
                self.delivered += due.len();
                due.iter().map(|(_, event)| *event).collect()
            }
            _ => heard,
        };
        if self.delivered != prev_delivered || self.next_route_at == (0, 0) {
            self.next_route_at = self.peek_next_route_at();
        }
        // The person's close is theirs whether or not a route is replaying,
        // and is delivered once, when the host is asked, as everything a
        // person does is, so a route that records it closes where they did
        // (#140).
        if ask && host.closes() > 0 && self.close_by.is_none() {
            events.push(crate::host::Input::Close);
        }
        events
    }

    /// Deliver what came at a flip or a poll: logged, recorded there, kept for
    /// the wall report, and set down where the guest reads it.
    fn deliver(&mut self, at: (u64, u64), events: Vec<crate::host::Input>, log: &mut RunLog) {
        if events.is_empty() {
            return;
        }
        let point = crate::route::point(at);
        let millis = u32::try_from(self.clock.millis() & 0xffff_ffff).unwrap_or(0);
        for event in events {
            // A title told it was closed is given its grace to leave; one with
            // no window has nothing to be told, and stops here.
            if event == crate::host::Input::Close && self.close_by.is_none() {
                let grace = if self.state.window_made {
                    CLOSE_GRACE_MILLIS
                } else {
                    0
                };
                self.close_by = Some(self.clock.millis() + grace);
            }
            log.line(&format!("input {point} {event:?}"));
            if let Some(recording) = &mut self.recording {
                recording.push(at, event);
            }
            self.input.push((at, event));
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
                    log.line(&format!("input {point} click as key {code:#04x}"));
                    crate::win32::user32::deliver(
                        &mut self.state,
                        crate::host::Input::Key { code, down },
                        millis,
                    );
                }
            }
        }
    }

    /// How many of the route's events the run has not reached.
    #[must_use]
    pub fn undelivered(&self) -> usize {
        self.route
            .as_ref()
            .map_or(0, |route| route.len() - self.delivered)
    }

    /// Take the machine one instruction or one call further, or say why it
    /// stopped, with no thread switched but by a call that waits or ends one.
    ///
    /// Returns `None` while there is more to do. The schedule comes back to it
    /// after every switch ([`Machine::step`]); stepping a threaded title by
    /// this alone is not the run its route makes.
    pub fn tick(&mut self, host: &mut dyn Host, log: &mut RunLog) -> Option<Outcome> {
        // A person stopping the run ends it there and then, with the log
        // written up to the instruction it stopped on.
        if self.stopped(host) {
            return Some(Outcome::Closed);
        }
        if self.boundary.holds(self.cpu.eip) {
            return self.cross_outcome(host, log);
        }
        self.trace_insn(log);
        self.tick_guest(host, log)
    }

    /// Execute one boundary crossing and map its stop reason to an optional
    /// run outcome.
    #[inline(always)]
    fn cross_outcome(&mut self, host: &mut dyn Host, log: &mut RunLog) -> Option<Outcome> {
        // A call that lands on the boundary is a call into something Microsoft
        // wrote. Either it is implemented here, or the run stops and names
        // what was asked for, who asked, and where.
        match self.cross(host, log) {
            Ok(()) => None,
            Err(stop) => self.cross_err(stop, log),
        }
    }

    #[cold]
    #[inline(never)]
    fn cross_err(&mut self, stop: Stop, log: &mut RunLog) -> Option<Outcome> {
        match stop {
            Stop::Exited { code } => Some(Outcome::Exited { code }),
            // A thread ending is not the run ending: the machine leaves it
            // where it is and runs somebody else (ADR-0005).
            Stop::ThreadEnded => {
                self.switch_thread(log);
                None
            }
            // A time-critical thread waits only at its own turn. Inside
            // a callback the machine runs on it, the title's threads
            // would run in the middle of the callback, and the calls the
            // machine has parked are one stack for every thread (#154, #371).
            // ponytail: a wall; a title whose callback waits wants each
            // thread's parked calls kept apart.
            Stop::Blocked
                if self.state.threads[self.state.thread].time_critical
                    && self.boundary.at(self.cpu.eip)
                        != Some(if self.state.mm_timer_thread == Some(self.state.thread) {
                            &crate::shim::Entry::MmTimer
                        } else {
                            &crate::shim::Entry::MidiPlayer
                        }) =>
            {
                let who = if self.state.mm_timer_thread == Some(self.state.thread) {
                    "WINMM's timer thread"
                } else {
                    "WINMM's MIDI player"
                };
                Some(Outcome::Wall {
                    what: format!("a callback {who} made waits for another thread"),
                })
            }
            // Waiting for another thread: the machine leaves the call
            // where it is, and this thread makes it again when it comes
            // back round (ADR-0005).
            Stop::Blocked => {
                self.switch_thread(log);
                None
            }
            other => Some(Outcome::Wall {
                what: other.to_string(),
            }),
        }
    }

    /// Present any pending guest frame and retire one guest instruction.
    fn tick_guest(&mut self, host: &mut dyn Host, log: &mut RunLog) -> Option<Outcome> {
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
            Err(Stop::Fault(fault)) if fault.addr < DOS_MEMORY => self.access_violation(fault, log),
            Err(stop) => Some(Outcome::Wall {
                what: stop.to_string(),
            }),
        }
    }

    /// The guest touched the bottom megabyte (#132). A read or a write in the
    /// first 4 KiB is an access violation, offered to the guest's own
    /// handlers as Windows offers one. The record says whether it was a write
    /// and which address, as `EXCEPTION_RECORD`'s documentation has it. The
    /// instruction that faulted has not happened, so a handler that carries
    /// on runs it again unless it moved the context past it.
    ///
    /// Anything else there is a wall. The rest of the megabyte is MS-DOS's
    /// memory, which Windows 9x lets a Win32 process read and write and this
    /// machine does not have. Code fetched from the bottom is not refused
    /// either: the code selector is flat, so "memory at address 0 is
    /// accessible, but it's certainly not code", and a call through a null
    /// pointer runs the real-mode interrupt table until it faults a few bytes
    /// on (Pietrek, *Microsoft Systems Journal*, June 1998).
    fn access_violation(&mut self, fault: Fault, log: &mut RunLog) -> Option<Outcome> {
        const EXCEPTION_ACCESS_VIOLATION: u32 = 0xc000_0005;
        let at = self.cpu.eip;
        if fault.addr >= NULL_PAGE || at < DOS_MEMORY {
            let what = if at < DOS_MEMORY {
                format!("the guest ran into {at:#010x}, which on Windows 9x is MS-DOS's own memory")
            } else {
                format!("{fault}, at {at:#010x}")
            };
            return Some(Outcome::Wall { what });
        }
        let what = format!(
            "an access violation {} {:#010x} at {at:#010x}",
            if fault.write { "writing" } else { "reading" },
            fault.addr
        );
        log.line(&what);
        let params = [u32::from(fault.write), fault.addr];
        let mut ctx = self.ctx(log);
        match win32::kernel32::dispatch(
            &mut ctx,
            EXCEPTION_ACCESS_VIOLATION,
            0,
            &params,
            at,
            what,
            true,
        ) {
            Ok(program) => {
                self.cpu.eip = program;
                None
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
        self.next_snapshot = self.snapshot_every.map(|n| n.max(1));
        let outcome = loop {
            if let Some(outcome) = self.step(host, log, budget, false) {
                break outcome;
            }
        };
        self.mix_slow(host);
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

    /// Take the machine on by the one schedule every run keeps, whoever is
    /// driving it (ADR-0005, #131): a burst of the running thread's own code,
    /// up to the declared quantum or its next call across the boundary; the
    /// switch to whoever is next; the call or the instruction that thread
    /// stands on; and the next burst.
    ///
    /// Without `one`, it goes on until the run is over. With it, it stops
    /// after one instruction or one call, standing where the next is about to
    /// be retired and past any switch before it, which is how the debugger
    /// steps. A traced run takes its bursts an instruction at a time, to write
    /// each one down. Either way a burst ends at the same instruction, so a
    /// stepped or traced run switches threads where its route's run does.
    pub fn step(
        &mut self,
        host: &mut dyn Host,
        log: &mut RunLog,
        budget: u64,
        one: bool,
    ) -> Option<Outcome> {
        let trace = log.trace();
        self.spins.allow(budget, !one && trace < Trace::Shims);
        if self.next_route_at == (0, 0) && self.flips > 0 {
            self.next_route_at = self.peek_next_route_at();
        }
        let fast = !one && trace < Trace::Insns && !self.stub;
        let mut moved = false;
        loop {
            if fast && matches!(self.turn, Turn::Top) {
                return Some(self.step_fast(host, log, budget));
            }
            match self.turn {
                Turn::Top => {
                    if self.clock.cursor() >= budget {
                        return Some(Outcome::BudgetExhausted);
                    }
                    if self.next_snapshot.is_some() {
                        self.snapshot_if_due(log);
                    }
                    // A thread that gave its turn away runs none of the rest
                    // of it: the switch after this is where the next one takes
                    // over (#115). A frame the guest finished is presented
                    // before anything else runs. And the stub runs alone, an
                    // instruction a turn: it has one thread and no boundary,
                    // so it is never switched, and what it gets from that is
                    // its `int`s heard by `tick` and a snapshot between any
                    // two of its instructions.
                    let now = self.clock.cursor();
                    let end = if std::mem::take(&mut self.state.yielded)
                        || self.stub
                        || self.state.present.is_some()
                    {
                        now
                    } else {
                        budget.min(now + BURST)
                    };
                    self.turn = Turn::Burst(end);
                }
                Turn::Burst(end)
                    if self.clock.cursor() < end && !Boundary::in_range(self.cpu.eip) =>
                {
                    if one && moved {
                        return None;
                    }
                    let count = if one || trace >= Trace::Insns {
                        1
                    } else {
                        end - self.clock.cursor()
                    };
                    if let Some(outcome) = self.run_guest(host, log, count) {
                        return Some(outcome);
                    }
                    moved = true;
                }
                Turn::Burst(_) => {
                    if self.state.threads.len() >= 2 {
                        if let Some(what) = self.deadlocked() {
                            return Some(Outcome::Wall { what });
                        }
                        // A burst is the declared quantum: whoever ran it hands
                        // over to whoever is next (ADR-0005).
                        self.switch_thread(log);
                    }
                    // The mix reaches the host between flips as well as at
                    // them: a title that goes a long way without flipping —
                    // loading a level, or thinking hard about one — would
                    // otherwise hand over nothing for as long as that took,
                    // and the device would run dry and drop what came late.
                    self.mix(host);
                    self.turn = Turn::Switched;
                }
                Turn::Switched => {
                    if one && moved {
                        return None;
                    }
                    if self.clock.cursor() >= budget {
                        return Some(Outcome::BudgetExhausted);
                    }
                    self.turn = Turn::Top;
                    if let Some(outcome) = self.tick(host, log) {
                        return Some(outcome);
                    }
                    moved = true;
                }
            }
        }
    }

    /// Fused burst→crossing loop for normal (non-stepped, non-insn-traced,
    /// non-stub) execution: keeps the exact same schedule, thread switches,
    /// mixing, and `self.turn` exit state while dropping per-turn state
    /// dispatch, redundant `stopped(host)` checks, and `trace_insn` checks.
    fn step_fast(&mut self, host: &mut dyn Host, log: &mut RunLog, budget: u64) -> Outcome {
        loop {
            let now = self.clock.cursor();
            if now >= budget {
                self.turn = Turn::Top;
                return Outcome::BudgetExhausted;
            }
            if self.next_snapshot.is_some() {
                self.snapshot_if_due(log);
            }
            let skip = std::mem::take(&mut self.state.yielded) || self.state.present.is_some();
            let end = if skip { now } else { budget.min(now + BURST) };
            while self.clock.cursor() < end && !Boundary::in_range(self.cpu.eip) {
                let count = end - self.clock.cursor();
                self.cpu.tsc = self.clock.cycles();
                let (retired, result) =
                    self.engine
                        .run(&mut self.cpu, &mut self.arena, count, Boundary::RANGE);
                self.clock.retire_many(retired);
                match result {
                    Ok(()) => {
                        self.spins
                            .pass(&mut self.cpu, &mut self.clock, &self.arena, &self.state);
                    }
                    Err(Stop::Interrupt(n)) => {
                        self.turn = Turn::Burst(end);
                        return Outcome::Wall {
                            what: format!("int {n:#04x}"),
                        };
                    }
                    Err(Stop::Fault(fault)) if fault.addr < DOS_MEMORY => {
                        if let Some(outcome) = self.access_violation(fault, log) {
                            self.turn = Turn::Burst(end);
                            return outcome;
                        }
                    }
                    Err(stop) => {
                        self.turn = Turn::Burst(end);
                        return Outcome::Wall {
                            what: stop.to_string(),
                        };
                    }
                }
            }
            if self.stopped(host) {
                self.turn = Turn::Burst(end);
                return Outcome::Closed;
            }
            if self.state.threads.len() >= 2 {
                if let Some(what) = self.deadlocked() {
                    self.turn = Turn::Burst(end);
                    return Outcome::Wall { what };
                }
                self.switch_thread(log);
                if self.close_by.is_some_and(|by| self.clock.millis() >= by) {
                    self.turn = Turn::Switched;
                    return Outcome::Closed;
                }
            }
            self.mix(host);
            if self.clock.cursor() >= budget {
                self.turn = Turn::Switched;
                return Outcome::BudgetExhausted;
            }
            self.turn = Turn::Top;
            let outcome = if self.boundary.holds(self.cpu.eip) {
                self.cross_outcome(host, log)
            } else {
                self.tick_guest(host, log)
            };
            if let Some(outcome) = outcome {
                return outcome;
            }
        }
    }

    /// Write a snapshot into the run directory, if one is due.
    ///
    /// Snapshots are taken at the top of a burst, in the schedule every run
    /// keeps, so a run that keeps them is the same machine as one that does
    /// not: the same bursts, thread switches and mixing (#94). They are off
    /// unless asked for, and they are for inspecting: only the stub resumes
    /// from one, because the Win32 state is not in it (`snap`).
    fn snapshot_if_due(&mut self, log: &mut RunLog) {
        let Some(at) = self.next_snapshot.filter(|at| self.flips >= *at) else {
            return;
        };
        match self.snapshot() {
            Ok(bytes) => {
                let name = format!("flip-{}.snapshot", self.flips);
                match std::fs::write(log.dir().join(&name), bytes) {
                    Ok(()) => log.line(&format!("snapshot {name} cursor {}", self.clock.cursor())),
                    Err(e) => log.line(&format!("snapshot {name} not written: {e}")),
                }
            }
            Err(fault) => log.line(&format!("snapshot refused: {fault}")),
        }
        self.next_snapshot = self.snapshot_every.map(|every| at + every.max(1));
    }

    /// The instruction about to be retired, for a run traced that far.
    fn trace_insn(&self, log: &mut RunLog) {
        if log.trace() >= Trace::Insns {
            log.line(&format!(
                "insn cursor {} eip {:#010x} eax {:#010x}",
                self.clock.cursor(),
                self.cpu.eip,
                self.cpu.r[EAX]
            ));
        }
    }
}

/// The declared quantum: the most of a thread's own instructions a burst
/// runs before it hands over (ADR-0005).
pub(crate) const BURST: u64 = 1 << 16;

/// Whether a thread can be given the processor at this moment (ADR-0005).
pub(crate) fn runnable_at(thread: &crate::win32::GuestThread, now: u64) -> bool {
    !thread.ended && !thread.suspended && thread.wake_at <= now
}

/// Boundary slot flag: this call reads user input (#215).
const BOUND_POLL: u8 = 1 << 0;
/// Boundary slot flag: this shim reads the declared clock (`spin::CLOCK_READS`).
const BOUND_CLOCK_READ: u8 = 1 << 1;

/// What a boundary slot stands for, once it has been called: the shim or the
/// COM method behind it, a special machine entry, or nothing resolved yet.
#[derive(Clone, Copy)]
enum Bound {
    Unresolved,
    Return,
    ThreadEnd,
    MidiPlayer,
    MmTimer,
    Shim {
        shim: Option<&'static win32::Shim>,
        flags: u8,
    },
    Method {
        interface: &'static str,
        slot: usize,
        method: Option<&'static crate::com::Method>,
        flags: u8,
    },
}

/// The calls that read what a person did: the keyboard, the mouse or the
/// message queue. They are a title's input polls, where what a person did
/// between flips reaches it (#215), and they are every input reader core has
/// written, on every thread and whatever they filter, by name, so another
/// DirectInput device's `GetDeviceState` is one too. A reader written later
/// joins them as it is written, and that renumbers no committed route's
/// polls: until then a call to it was a wall, so no route got past one. A
/// test fails on any USER32 or DirectInput call that is named neither here
/// nor as not a poll.
const POLLS: &[&str] = &[
    "PeekMessageA",
    "GetMessageA",
    "WaitMessage",
    "MsgWaitForMultipleObjects",
    "GetCursorPos",
    "GetAsyncKeyState",
    "GetKeyState",
    // `IDirectInputDeviceA`'s, the keyboard's and the mouse's.
    "GetDeviceState",
    "GetDeviceData",
];

/// Whether a shim is one of the clock reads tracked by `crate::spin::Spins`.
fn is_clock_read(shim: &win32::Shim) -> bool {
    matches!(
        (shim.dll, shim.name),
        ("KERNEL32", "QueryPerformanceCounter" | "GetTickCount")
            | ("WINMM", "timeGetTime" | "timeGetSystemTime")
    )
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
        // or is simply the other one is refused by name rather than run, and a
        // wrapped one is named as wrapped, as the launcher names it.
        match crate::roster::judge(&title.image, &bytes) {
            crate::roster::Found::Wrapped { scheme, .. } => {
                return Err(format!(
                    "{} in {}: {}",
                    title.image.path,
                    iso.path().display(),
                    crate::pe::PeError::Wrapped { scheme }
                )
                .into());
            }
            crate::roster::Found::WrongBytes { found, .. } => {
                return Err(format!(
                    "{} in {} hashes {found}, and this title is declared as {}. \
                     A different pressing, a patched executable, or a bad rip",
                    title.image.path,
                    iso.path().display(),
                    title.image.sha256
                )
                .into());
            }
            _ => {}
        }

        // The pressing stays in the declared machine's drive for the whole
        // run: a title reads its own disc, and the label it checks is the
        // label the image carries.
        self.state.volume_label = iso.label.clone();

        // What this title's installer would have put on the machine's disc,
        // and the pressing in its drive.
        self.state.installed_files = self.tree.install(iso, title)?;

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
        // A wall the declared tree met as the title loaded: an archive member
        // that would not come out (#169), or a name the title declares that
        // its disc, read through its Joliet tree, answers only by its primary
        // tree's 8.3 name (#144).
        if let Some(wall) = self.tree.wall() {
            return Err(wall.into());
        }
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

/// How many four-byte stack arguments fit inline without a heap allocation.
/// Covers every Win32 and COM call on the roster (the widest is 14).
const INLINE_STACK_ARGS: usize = 16;

impl Machine {
    /// What a shim or a method is handed: the machine, for the one call.
    #[inline(always)]
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

    /// A call's four-byte arguments, read into caller-owned `buf` via a single
    /// bulk `arena.slice` check (falling back per-slot on fault to preserve the
    /// exact faulting address and order), or into `heap` when `count > 16`.
    #[inline(always)]
    fn stack_args<'a>(
        &self,
        from: u32,
        count: u32,
        buf: &'a mut [u32; INLINE_STACK_ARGS],
        heap: &'a mut Vec<u32>,
    ) -> Result<&'a [u32], Stop> {
        let len = count as usize;
        if len == 0 {
            return Ok(&[]);
        }
        if len <= INLINE_STACK_ARGS {
            if let Ok(bytes) = self.arena.slice(from, len * 4) {
                for (slot, chunk) in buf[..len].iter_mut().zip(bytes.as_chunks::<4>().0) {
                    *slot = u32::from_le_bytes(*chunk);
                }
                return Ok(&buf[..len]);
            }
            return self.stack_args_slow(from, len, buf);
        }
        self.stack_args_heap(from, count, heap)
    }

    #[cold]
    #[inline(never)]
    fn stack_args_slow<'a>(
        &self,
        from: u32,
        len: usize,
        buf: &'a mut [u32; INLINE_STACK_ARGS],
    ) -> Result<&'a [u32], Stop> {
        for (i, slot) in buf[..len].iter_mut().enumerate() {
            *slot = self
                .arena
                .read_u32(from.wrapping_add((i as u32) * 4))
                .map_err(Stop::Fault)?;
        }
        Ok(&buf[..len])
    }

    #[cold]
    #[inline(never)]
    fn stack_args_heap<'a>(
        &self,
        from: u32,
        count: u32,
        heap: &'a mut Vec<u32>,
    ) -> Result<&'a [u32], Stop> {
        heap.clear();
        heap.reserve(count as usize);
        for i in 0..count {
            heap.push(
                self.arena
                    .read_u32(from.wrapping_add(i.wrapping_mul(4)))
                    .map_err(Stop::Fault)?,
            );
        }
        Ok(heap.as_slice())
    }

    #[cold]
    #[inline(never)]
    fn resolve_bound(&mut self, addr: u32, index: usize) -> Result<Bound, Stop> {
        use crate::shim::Entry;
        let bound = match self.boundary.at(addr) {
            Some(Entry::Return) => Bound::Return,
            Some(Entry::ThreadEnd) => Bound::ThreadEnd,
            Some(Entry::MidiPlayer) => Bound::MidiPlayer,
            Some(Entry::MmTimer) => Bound::MmTimer,
            // The exception walk ran out of handlers (#54).
            Some(Entry::Unhandled { what }) => {
                return Err(Stop::NotWritten {
                    what: format!("{what}: nothing on the guest's own handler chain took it"),
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
                let method = win32::com_method(interface, *slot);
                let flags = if method.is_some_and(|m| POLLS.contains(&m.name)) {
                    BOUND_POLL
                } else {
                    0
                };
                Bound::Method {
                    interface,
                    slot: *slot,
                    method,
                    flags,
                }
            }
            Some(Entry::Import(import)) => {
                let shim = win32::find(&import.dll, &import.name);
                let mut flags = 0;
                if let Some(s) = shim {
                    if POLLS.contains(&s.name) {
                        flags |= BOUND_POLL;
                    }
                    if is_clock_read(s) {
                        flags |= BOUND_CLOCK_READ;
                    }
                }
                Bound::Shim { shim, flags }
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
            self.bound.resize(index + 1, Bound::Unresolved);
        }
        self.bound[index] = bound;
        Ok(bound)
    }

    #[cold]
    #[inline(never)]
    fn trace_shim_call(&self, addr: u32, caller: u32, args: &[u32], log: &mut RunLog) {
        let what = match self.boundary.at(addr) {
            Some(crate::shim::Entry::Import(import)) => format!("{}!{}", import.dll, import.name),
            _ => String::new(),
        };
        log.line(&format!(
            "call {what}({}) from {caller:#010x} cursor {}",
            listed(args),
            self.clock.cursor()
        ));
    }

    #[cold]
    #[inline(never)]
    fn missing_shim(&self, addr: u32, caller: u32, log: &mut RunLog) -> Stop {
        let what = match self.boundary.at(addr) {
            Some(crate::shim::Entry::Import(import)) => format!("{}!{}", import.dll, import.name),
            _ => String::new(),
        };
        let cursor = self.clock.cursor();
        log.at(Trace::Shims, || {
            format!("wall {what} from {caller:#010x} cursor {cursor}")
        });
        Stop::NotWritten { what, caller }
    }

    /// Answer a call the guest made across the boundary.
    ///
    /// The arguments are where the calling convention says they are, the
    /// answer goes back in `eax`, and the stack is left as the real library
    /// would have left it.
    fn cross(&mut self, host: &mut dyn Host, log: &mut RunLog) -> Result<(), Stop> {
        use crate::shim::{THUNK_BASE, THUNK_STRIDE};
        self.spins.crossed(&mut self.arena);
        let addr = self.cpu.eip;
        let index = (addr.wrapping_sub(THUNK_BASE) / THUNK_STRIDE) as usize;
        let bound = match self.bound.get(index).copied() {
            Some(Bound::Unresolved) | None => self.resolve_bound(addr, index)?,
            Some(bound) => bound,
        };
        let (found, flags) = match bound {
            Bound::Return => return self.guest_returned(log),
            // A thread's own function returned (ADR-0005). Its exit code is
            // what it answered with.
            Bound::ThreadEnd => {
                win32::kernel32::end_thread(&mut self.state, self.cpu.r[EAX]);
                return Err(Stop::ThreadEnded);
            }
            Bound::MidiPlayer => return self.midi_player(log),
            Bound::MmTimer => return self.mm_timer(log),
            Bound::Method {
                interface,
                slot,
                method,
                flags,
            } => {
                if flags & BOUND_POLL != 0 {
                    self.poll(host, log);
                }
                return self.cross_com(interface, slot, method, log);
            }
            Bound::Shim { shim, flags } => (shim, flags),
            Bound::Unresolved => unreachable!("resolve_bound never returns Unresolved"),
        };
        // What a person did since the flip reaches a title as it asks (#215).
        if flags & BOUND_POLL != 0 {
            self.poll(host, log);
        }
        let esp = self.cpu.r[crate::cpu::ESP];
        let caller = self.arena.read_u32(esp).map_err(Stop::Fault)?;

        let Some(shim) = found else {
            return Err(self.missing_shim(addr, caller, log));
        };
        if flags & BOUND_CLOCK_READ != 0 {
            self.spins
                .crossing(shim, &self.cpu, &self.clock, &mut self.arena);
        }

        let mut buf = [0u32; INLINE_STACK_ARGS];
        let mut heap = Vec::new();
        let args = self.stack_args(esp + 4, shim.args, &mut buf, &mut heap)?;
        if log.trace() >= Trace::Shims {
            self.trace_shim_call(addr, caller, args, log);
        }

        let mut ctx = self.ctx(log);
        let answered = (shim.run)(&mut ctx, args);
        // A wall the declared tree met in the call, whatever it did next: an
        // archive member that would not come out (#169), or a name the disc
        // answers only by its primary tree's 8.3 name (#144), each of which
        // the call was told is not there.
        if let Some(what) = ctx.tree.wall() {
            return Err(Stop::NotWritten { what, caller });
        }
        let answer = match answered {
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
    /// been touched and hashing two gigabytes to prove that is a slow way to
    /// learn nothing.
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

        // What a person did, and what the title set, that the guest has not
        // taken yet: the devices' state, the messages still waiting, and the
        // paint and screen saver the title left due.
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
        // A quit asked for and not yet taken, hashed only when there is one,
        // so that a run that never asks hashes as it did (#140).
        if let Some((thread, code)) = input.quit {
            hash.update(&thread.to_le_bytes());
            hash.update(&code.to_le_bytes());
        }
        // A paint due and a screen saver said to run, likewise hashed only
        // when there is one (#147).
        if let Some(erase) = input.paint_erase {
            hash.update(&[1, u8::from(erase)]);
        }
        if input.screen_saver_running {
            hash.update(&[2]);
        }
        // And the keys the title has read as held, which its next translated
        // key types with, likewise only when there are some (#157).
        if !input.keys_read.is_empty() {
            hash.update(&[3]);
            for key in &input.keys_read {
                hash.update(&key.to_le_bytes());
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
    /// every page the guest can write. That is where the image and every
    /// binary the title ships were placed — whose `.data` is where Alpha Team
    /// keeps most of itself (#55, #94) — every page the guest committed, the
    /// main thread's whole stack and its thread block, the process block, and
    /// the surface. What the guest committed is what it claimed from
    /// `VirtualAlloc` by address and how far the heap has grown, and Alpha
    /// Team's own loader puts its level images in both: the first at
    /// `0x1000_0000`, and every one after it wherever the heap puts it. Every
    /// other thread's stack and block come off the heap (#98).
    ///
    /// Files mapped into the arena are not among them: they are the disc's
    /// own bytes, and the one title that maps any, Alpha Team, maps its eight
    /// archives read-only and views them only to read (#57, #98).
    ///
    // ponytail: what the guest was handed rather than every page it wrote. A
    // guest that wrote somewhere it was never handed, a read-only mapping
    // among them, would resume with that writing lost, where Windows would
    // have stopped it. Upgrade path: a dirty-page map in the arena, kept by
    // `GuestMem`, the one path every write already goes through.
    fn snapshot_regions(&self) -> Vec<(u32, u32)> {
        let mut regions = Vec::new();
        if let Some(image) = &self.image {
            regions.push((image.base, image.size));
        }
        for module in self.modules.modules() {
            regions.push((module.base, module.size));
        }
        for (start, end) in &self.state.claimed {
            regions.push((*start, end - start));
        }
        regions.push((crate::guest::heap::HEAP_BASE, self.state.heap.extent()));
        // The whole of it, and whether or not the main thread is the one
        // running: what lies below where it stands is what an uninitialised
        // local reads.
        regions.push((STACK_TOP - STACK_BYTES, STACK_BYTES));
        regions.push((TIB, PEB + 0x1000 - TIB));
        if let Some(surface) = self.surface {
            regions.push((surface.addr, surface.pitch * surface.height));
        }
        regions.retain(|(_, len)| *len > 0);
        regions
    }

    /// Merged, sorted, non-overlapping 4 KiB page intervals `[start_page, end_page)`
    /// covering every guest-writable region of the arena (excluding read-only
    /// file mappings tracked separately in `state.mapped_files`).
    fn live_page_ranges(&self) -> Vec<(u32, u32)> {
        const PAGE: u32 = 4096;
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        let mut push_bytes = |start: u32, len: u32| {
            if len == 0 {
                return;
            }
            let p_start = start / PAGE;
            let p_end = start.saturating_add(len).div_ceil(PAGE);
            if p_start < p_end {
                ranges.push((p_start, p_end));
            }
        };
        for (start, len) in self.snapshot_regions() {
            push_bytes(start, len);
        }
        push_bytes(crate::loader::ATTACH_PROGRAM, PAGE);
        if self.stub {
            push_bytes(stub::ENTRY, PAGE);
        }
        for (&base, &(size, _)) in &self.state.reserved {
            push_bytes(base, size);
        }
        for &page in self.state.committed.keys() {
            push_bytes(page, PAGE);
        }
        if ranges.is_empty() {
            return ranges;
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (s, e) in ranges {
            if let Some(last) = merged.last_mut() {
                if s <= last.1 {
                    last.1 = last.1.max(e);
                    continue;
                }
            }
            merged.push((s, e));
        }
        merged
    }

    /// Snapshot or restore all scalar & structured `Machine` fields (excluding
    /// sparse arena pages, profile overlay, and host-side config/callbacks).
    fn snap_machine_fields(&mut self, v: &mut dyn crate::snap::Visitor) {
        use crate::snap::Snap;
        // Exhaustively destructure `Machine` so any field added later causes a
        // compile error here rather than being silently omitted from snapshots (#313).
        let Self {
            arena,
            cpu,
            clock,
            engine: _,
            flip_cost: _,
            surface,
            stub,
            flips,
            paced,
            boundary,
            image: _,
            modules,
            bound_names,
            state,
            tree,
            title: _,
            input,
            present_buffer: _,
            pointer_puts,
            pointer_puts_told,
            pointer_shown,
            cursor_told,
            continuations,
            dialog_turns,
            pass_through,
            bound: _,
            render_turns,
            route: _,
            recording,
            screenshots: _,
            snapshot_every: _,
            next_snapshot: _,
            turn,
            close_by,
            polls,
            ask_at,
            delivered,
            next_route_at: _,
            next_mix_cycle: _,
            spins,
            flip_observer: _,
        } = self;

        cpu.snap(v);
        clock.snap(v);
        arena.snap_meta(v);
        crate::snap::snap_option_with(
            v,
            surface,
            || Surface {
                addr: 0,
                width: 0,
                height: 0,
                pitch: 0,
            },
            |s, v| {
                let Surface {
                    addr,
                    width,
                    height,
                    pitch,
                } = s;
                v.u32(addr);
                v.u32(width);
                v.u32(height);
                v.u32(pitch);
            },
        );
        v.bool(stub);
        v.u64(flips);
        v.f64(paced);
        boundary.snap(v);
        modules.snap(v);
        v.usize(bound_names);
        state.snap(v);
        tree.snap_wall(v);
        crate::snap::snap_vec_with(
            v,
            input,
            || ((0u64, 0u64), crate::host::Input::Close),
            |((f, p), ev), v| {
                v.u64(f);
                v.u64(p);
                ev.snap(v);
            },
        );
        v.u64(pointer_puts);
        v.u64(pointer_puts_told);
        v.bool(pointer_shown);
        crate::snap::snap_option(v, cursor_told, |c, v| v.u32(c));
        crate::snap::snap_vec(v, continuations, |(a, b, c), v| {
            v.u32(a);
            v.u32(b);
            v.u32(c);
        });
        crate::snap::snap_vec(v, dialog_turns, |d, v| v.usize(d));
        crate::snap::snap_vec(v, pass_through, |d, v| v.usize(d));
        crate::snap::snap_vec(v, render_turns, |(d, s), v| {
            v.usize(d);
            v.u32(s);
        });
        crate::snap::snap_option(v, recording, |r, v| r.snap(v));
        let mut turn_tag = match turn {
            Turn::Top => 0u8,
            Turn::Burst(_) => 1u8,
            Turn::Switched => 2u8,
        };
        v.u8(&mut turn_tag);
        if v.is_reading() {
            match turn_tag {
                0 => *turn = Turn::Top,
                1 => {
                    let mut end = 0u64;
                    v.u64(&mut end);
                    *turn = Turn::Burst(end);
                }
                2 => *turn = Turn::Switched,
                _ => v.mark_corrupt("Machine::turn tag"),
            }
        } else if let Turn::Burst(end) = turn {
            v.u64(end);
        }
        crate::snap::snap_option(v, close_by, |c, v| v.u64(c));
        v.u64(polls);
        v.u64(ask_at);
        v.usize(delivered);
        spins.snap(v);
    }

    /// Write the entire live machine state (`LLSNAP v2`, #313): provenance,
    /// full CPU/x87/clock/schedule/Win32/COM/audio/movie state, sparse non-zero
    /// 4 KiB arena pages, and profile directory file overlay.
    ///
    /// # Errors
    /// When guest memory cannot be read.
    pub fn snapshot_v2(&mut self) -> Result<Vec<u8>, crate::guest::Fault> {
        use crate::snap::{Visitor, Writer, MAGIC};

        let mut writer = Writer::with_capacity(256 * 1024);
        writer.out.extend_from_slice(MAGIC);
        let provenance = self.provenance();
        for part in [&provenance.build, &provenance.title, &provenance.route] {
            let len = u32::try_from(part.len()).unwrap_or(0);
            writer.out.extend_from_slice(&len.to_le_bytes());
            writer.out.extend_from_slice(part.as_bytes());
        }

        let stashed_files = self
            .state
            .stash_readonly_files(|p| self.tree.holds(p) && !self.tree.is_in_profile(p));
        let mut deferred_files: Vec<(u32, u32, String)> = stashed_files
            .iter()
            .map(|(idx, resolved, bytes)| {
                (
                    u32::try_from(*idx).unwrap_or(u32::MAX),
                    u32::try_from(bytes.len()).unwrap_or(u32::MAX),
                    resolved.clone(),
                )
            })
            .collect();
        self.snap_machine_fields(&mut writer);
        self.state.unstash_readonly_files(stashed_files);
        crate::snap::snap_vec(&mut writer, &mut deferred_files, |(idx, len, path), v| {
            v.u32(idx);
            v.u32(len);
            v.string(path);
        });

        let page_ranges = self.live_page_ranges();
        self.arena.write_sparse_pages(&page_ranges, &mut writer.out);

        let mut profile_files = self
            .tree
            .profile_dir()
            .map(capture_profile_overlay)
            .unwrap_or_default();
        let mut count = u32::try_from(profile_files.len()).unwrap_or(u32::MAX);
        writer.u32(&mut count);
        for (rel, bytes) in &mut profile_files {
            writer.string(rel);
            writer.byte_vec(bytes);
        }
        Ok(writer.out)
    }

    /// Write the machine's full state (`LLSNAP v2`), keyed by what produced it.
    ///
    /// # Errors
    /// When guest memory cannot be read.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, crate::guest::Fault> {
        self.snapshot_v2()
    }

    /// Restore the entire live machine state from an `LLSNAP v2` blob (#313),
    /// enforcing strict provenance (`build`, `title`, and `route`).
    ///
    /// # Errors
    /// When the bytes are not an `LLSNAP v2` snapshot, fail provenance checks,
    /// are truncated/corrupt, or profile restoration fails.
    pub fn restore_v2(&mut self, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        self.restore_v2_inner(bytes, true)
    }

    /// Restore the entire live machine state from an `LLSNAP v2` blob inside an
    /// interactive control session (#313), verifying `build` and `title`
    /// provenance while permitting differing pre-roll routes.
    ///
    /// # Errors
    /// When the bytes are not an `LLSNAP v2` snapshot, fail `build`/`title`
    /// provenance checks, are truncated/corrupt, or profile restoration fails.
    pub fn restore_v2_live(&mut self, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        self.restore_v2_inner(bytes, false)
    }

    fn restore_v2_inner(
        &mut self,
        bytes: &[u8],
        strict_route: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::snap::{Reader, SnapError, Visitor, MAGIC};

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
        if strict_route {
            self.provenance().accepts(&found)?;
        } else {
            self.provenance().accepts_target(&found)?;
        }

        // Capture pre-restore dirty ranges, open read-only file buffers, and
        // mapped file state so pages written after the snapshot are zeroed
        // cleanly and unchanged read-only archives are kept in place with zero copy.
        let prev_page_ranges = self.live_page_ranges();
        let prev_next_mapping = self.state.next_mapping;
        let prev_mapped_files = self.state.mapped_files.clone();
        let mut prev_handles = self.state.take_handles();

        let mut reader = Reader::new(&bytes[at..]);
        self.snap_machine_fields(&mut reader);
        reader.finish("machine state")?;

        let mut deferred_files: Vec<(u32, u32, String)> = Vec::new();
        crate::snap::snap_vec(&mut reader, &mut deferred_files, |(idx, len, path), v| {
            v.u32(idx);
            v.u32(len);
            v.string(path);
        });
        reader.finish("deferred files")?;

        let post_page_ranges = self.live_page_ranges();
        let clear_ranges = merge_page_ranges(&prev_page_ranges, &post_page_ranges);
        self.arena.restore_sparse_pages(&clear_ranges, &mut reader);
        reader.finish("arena pages")?;

        // Read profile overlay before touching disk so a truncated snapshot
        // never partially modifies profile files.
        let mut profile_count = 0u32;
        reader.u32(&mut profile_count);
        let profile_count = profile_count as usize;
        if !reader.check_remaining(profile_count.saturating_mul(8)) {
            return Err(SnapError::Truncated("profile overlay").into());
        }
        let mut profile_files: Vec<(String, Vec<u8>)> = Vec::with_capacity(profile_count);
        for _ in 0..profile_count {
            let mut rel = String::new();
            let mut data = Vec::new();
            reader.string(&mut rel);
            reader.byte_vec(&mut data);
            if reader.failed() {
                break;
            }
            profile_files.push((rel, data));
        }
        reader.finish("profile overlay")?;

        // Sync <data-root>/profile/ back to the exact file set at snapshot time
        // before re-hydrating any mapped or AVI files that may come from profile.
        if let Some(profile_root) = self.tree.profile_dir().map(std::path::Path::to_path_buf) {
            restore_profile_overlay(&profile_root, &profile_files)?;
        }

        // Rehydrate deferred read-only file handle buffers: swap in place from
        // `prev_handles` on live restore (0 copy, 0 I/O) or read from `tree` on cold.
        for (idx, expected_len, resolved_path) in deferred_files {
            let idx = idx as usize;
            let expected_len = expected_len as usize;
            let Some(crate::win32::Object::File {
                path,
                contents: crate::win32::Contents::Held(bytes),
                wrote: false,
                ..
            }) = self.state.handle_slot_mut(idx)
            else {
                return Err(SnapError::Corrupt("deferred file handle").into());
            };
            if !bytes.is_empty() {
                return Err(SnapError::Corrupt("deferred file non-empty").into());
            }
            if let Some(Some(crate::win32::Object::File {
                path: prev_path,
                contents: crate::win32::Contents::Held(prev_bytes),
                wrote: false,
                ..
            })) = prev_handles.get_mut(idx)
            {
                if prev_path == path && prev_bytes.len() == expected_len {
                    std::mem::swap(bytes, prev_bytes);
                    continue;
                }
            }
            let Some(loaded) = self.tree.read(&resolved_path) else {
                return Err(SnapError::Corrupt("deferred file missing from tree").into());
            };
            if loaded.len() != expected_len {
                return Err(SnapError::Corrupt("deferred file length mismatch").into());
            }
            if let Some(crate::win32::Object::File {
                contents: crate::win32::Contents::Held(slot),
                ..
            }) = self.state.handle_slot_mut(idx)
            {
                *slot = loaded;
            }
        }

        // Reconcile read-only file mappings at MAPPING_BASE..next_mapping:
        // - Prefix entries matching what is already mapped in the live arena
        //   require zero bytes copied (keeps Alpha Team's 400 MB archives <1 ms).
        // - Any new/cold entries are populated from `self.tree`.
        // - Any tail mapped after the snapshot is zeroed back cleanly.
        let new_next_mapping = self.state.next_mapping;
        if prev_next_mapping > new_next_mapping {
            self.arena
                .zero_range_raw(new_next_mapping, prev_next_mapping);
        }
        let need_remap: Vec<(u32, u32, String)> = self
            .state
            .mapped_files
            .iter()
            .enumerate()
            .filter(|(i, entry)| prev_mapped_files.get(*i) != Some(*entry))
            .map(|(_, entry)| entry.clone())
            .collect();
        for (map_at, map_len, map_path) in need_remap {
            let Some(file_bytes) = self.tree.read(&map_path) else {
                return Err(SnapError::Corrupt("mapped file missing from tree").into());
            };
            let take = (map_len as usize).min(file_bytes.len());
            self.arena.write(map_at, &file_bytes[..take])?;
            let padded = map_len.next_multiple_of(0x1000);
            if (take as u32) < padded {
                self.arena.zero_range_raw(
                    map_at.saturating_add(take as u32),
                    map_at.saturating_add(padded),
                );
            }
        }

        // Rehydrate AVIFile, DirectShow, and AMMultiMediaStream `Rc<[u8]>`
        // movie buffers through `MovieStore` without duplicating bytes.
        self.state.rehydrate_movies(&mut self.tree)?;
        // Rehydrate registered GDI TrueType fonts lazily from `tree`.
        self.state
            .gdi
            .rehydrate_fonts(|path| self.tree.read(path))
            .map_err(SnapError::Corrupt)?;
        // Rehydrate file-backed DirectMusic styles lazily from `tree`.
        self.state
            .dmusic
            .rehydrate_styles(|path| self.tree.read(path))
            .map_err(SnapError::Corrupt)?;

        if !strict_route {
            let route_len = self.route.as_ref().map_or(0, crate::route::Route::len);
            if self.delivered > route_len {
                self.delivered = route_len;
            }
        }

        // Invalidate decoded fast-block cache & boundary lookup cache and
        // re-sync scheduler/mixer horizons cleanly.
        self.bound.clear();
        self.forget_decoded();
        self.next_mix_cycle = self
            .clock
            .first_cycle_of_audio_frame(self.state.mixer.next_frame().saturating_add(1));
        self.next_route_at = self.peek_next_route_at();
        self.next_snapshot = self
            .snapshot_every
            .map(|every| self.flips.saturating_add(every.max(1)));
        Ok(())
    }

    /// Put the machine back where a snapshot left it (`LLSNAP v2`, #313).
    ///
    /// # Errors
    /// When the snapshot came from another build, title or route, or when it
    /// is not a snapshot at all.
    pub fn resume(&mut self, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        self.restore_v2(bytes)
    }

    /// Throw away every decoded instruction, after memory has changed under
    /// the engine.
    pub fn forget_decoded(&mut self) {
        self.engine.forget();
        self.arena.clear_code();
    }
}

/// Merge two sorted, non-overlapping `[start_page, end_page)` interval lists.
fn merge_page_ranges(a: &[(u32, u32)], b: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut all = Vec::with_capacity(a.len() + b.len());
    all.extend_from_slice(a);
    all.extend_from_slice(b);
    if all.is_empty() {
        return all;
    }
    all.sort_unstable();
    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(all.len());
    for (s, e) in all {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }
    merged
}

/// Capture all regular files under `root` as sorted `(rel_unix_path, bytes)`.
fn capture_profile_overlay(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    fn walk(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<std::path::PathBuf> =
            entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                walk(base, &path, out);
            } else if meta.is_file() {
                if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(base), std::fs::read(&path)) {
                    let rel_unix = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy())
                        .collect::<Vec<_>>()
                        .join("/");
                    out.push((rel_unix, bytes));
                }
            }
        }
    }
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Validate a relative profile overlay path against traversal or absolute escapes.
fn validate_overlay_rel(rel: &str) -> Result<std::path::PathBuf, crate::snap::SnapError> {
    use std::path::{Component, Path, PathBuf};
    if rel.is_empty() || rel.contains('\0') {
        return Err(crate::snap::SnapError::Corrupt("profile path"));
    }
    let mut out = PathBuf::new();
    for comp in Path::new(rel).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            _ => return Err(crate::snap::SnapError::Corrupt("profile path")),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(crate::snap::SnapError::Corrupt("profile path"));
    }
    Ok(out)
}

/// Synchronize `root` on disk to match `files` bit-for-bit, removing any files
/// created after the snapshot and restoring overwritten or deleted files.
fn restore_profile_overlay(
    root: &std::path::Path,
    files: &[(String, Vec<u8>)],
) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;
    let mut wanted: BTreeMap<std::path::PathBuf, &[u8]> = BTreeMap::new();
    for (rel, data) in files {
        let sub = validate_overlay_rel(rel)?;
        wanted.insert(root.join(sub), data.as_slice());
    }

    if root.is_dir() {
        fn sweep(
            dir: &std::path::Path,
            root: &std::path::Path,
            wanted: &BTreeMap<std::path::PathBuf, &[u8]>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                let meta = std::fs::symlink_metadata(&path)?;
                if meta.file_type().is_symlink() {
                    continue;
                }
                if meta.is_dir() {
                    sweep(&path, root, wanted)?;
                    // Remove subdirectories that became empty.
                    if std::fs::read_dir(&path)?.next().is_none() {
                        let _ = std::fs::remove_dir(&path);
                    }
                } else if meta.is_file() && !wanted.contains_key(&path) {
                    std::fs::remove_file(&path)?;
                }
            }
            let _ = root;
            Ok(())
        }
        sweep(root, root, &wanted)?;
    }

    for (path, bytes) in wanted {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let unchanged = std::fs::read(&path).is_ok_and(|cur| cur.as_slice() == bytes);
        if !unchanged {
            std::fs::write(&path, bytes)?;
        }
    }
    Ok(())
}

impl Machine {
    #[cold]
    #[inline(never)]
    fn trace_com_call(
        &self,
        interface: &'static str,
        method: &'static str,
        this: u32,
        caller: u32,
        args: &[u32],
        log: &mut RunLog,
    ) {
        log.line(&format!(
            "call {interface}::{method}({}) on {this:#010x} from {caller:#010x} cursor {}",
            listed(args),
            self.clock.cursor()
        ));
    }

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

        let mut buf = [0u32; INLINE_STACK_ARGS];
        let mut heap = Vec::new();
        let args = self.stack_args(esp + 8, method.args, &mut buf, &mut heap)?;
        if log.trace() >= Trace::Shims {
            self.trace_com_call(interface, method.name, this, caller, args, log);
        }

        let mut ctx = self.ctx(log);
        let answered = (method.run)(&mut ctx, this, args);
        if let Some(what) = ctx.tree.wall() {
            return Err(Stop::NotWritten { what, caller });
        }
        let answer = match answered {
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

    /// Run up to `count` of the guest's own instructions, stopping before a
    /// call across the boundary, without the checks `tick` makes before each
    /// instruction.
    ///
    /// Nothing but the engine has anything to do between calls out: a frame to
    /// present only appears when a shim runs, and a window closed mid-burst is
    /// noticed at its end. What the guest does is the same either way; only
    /// the host's time is saved.
    fn run_guest(&mut self, host: &mut dyn Host, log: &mut RunLog, count: u64) -> Option<Outcome> {
        self.trace_insn(log);
        self.cpu.tsc = self.clock.cycles();
        let (retired, result) =
            self.engine
                .run(&mut self.cpu, &mut self.arena, count, Boundary::RANGE);
        self.clock.retire_many(retired);
        match result {
            Ok(()) => {
                // A clock spin is passed here, where its burst ends, so what
                // follows is what follows the burst that would have reached
                // the same read (#217).
                self.spins
                    .pass(&mut self.cpu, &mut self.clock, &self.arena, &self.state);
                self.stopped(host).then_some(Outcome::Closed)
            }
            Err(Stop::Interrupt(n)) => Some(Outcome::Wall {
                what: format!("int {n:#04x}"),
            }),
            Err(Stop::Fault(fault)) if fault.addr < DOS_MEMORY => self.access_violation(fault, log),
            Err(stop) => Some(Outcome::Wall {
                what: stop.to_string(),
            }),
        }
    }

    /// Whether the person has stopped the run (#140): they asked twice, or
    /// once before the title had a window to be told, or the title was told
    /// and its grace is over. Only the first of those depends on when the
    /// host heard them; the grace is the declared clock's, so a route that
    /// closes stops on the same instruction every time.
    fn stopped(&self, host: &dyn Host) -> bool {
        let closes = host.closes();
        closes >= 2
            || (closes == 1 && self.close_by.is_none() && !self.state.window_made)
            || self.close_by.is_some_and(|by| self.clock.millis() >= by)
    }

    /// Give the next runnable guest thread the processor.
    ///
    /// One host thread runs them all, so switching is putting the registers
    /// back where the parked thread left them. It happens at the end of a
    /// burst — the declared quantum — when a thread ends, waits or gives its
    /// turn away with `Sleep(0)`, and nowhere else: a schedule that depended
    /// on the host's own threads would make every run of this title a
    /// different one (ADR-0005), and one that depended on who is watching
    /// would make a traced or stepped run a different one (#131).
    fn switch_thread(&mut self, log: &mut RunLog) {
        if self.state.threads.len() < 2 {
            return;
        }
        let running = self.state.thread;
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
        // A time-critical thread runs ahead of every other whenever it can,
        // and keeps the processor until it waits (including against another
        // time-critical thread, so a running callback never interleaves on the
        // shared continuation stack, #154, #371).
        let next = (self.state.threads[running].time_critical
            && runnable_at(&self.state.threads[running], now))
        .then_some(running)
        .or_else(|| {
            self.state
                .threads
                .iter()
                .position(|t| t.time_critical && runnable_at(t, now))
        })
        .or_else(|| {
            (1..=count)
                .map(|step| (running + step) % count)
                .find(|i| runnable_at(&self.state.threads[*i], now))
        });
        let Some(next) = next else {
            // Nothing else can run; whoever was running carries on, and if it
            // has ended too the run is over by its own exit.
            return;
        };
        if next == running {
            return;
        }
        self.state.threads[running].cpu = self.cpu;
        self.state.thread = next;
        self.cpu = self.state.threads[next].cpu;
        if log.trace() >= Trace::Shims {
            log.line(&format!(
                "thread {} takes over at {:#010x} cursor {}",
                self.state.threads[next].id,
                self.cpu.eip,
                self.clock.cursor()
            ));
        }
    }

    /// Whether every thread left is waiting for another with no hour to be
    /// woken at, which nothing can move on (ADR-0005, #60).
    ///
    /// Without this a deadlock is a machine that looks busy for ever: each
    /// thread takes its turn, makes its wait again, hands the turn back, and
    /// the cursor never moves.
    fn deadlocked(&self) -> Option<String> {
        if self.state.threads.len() < 2 {
            return None;
        }
        let mut live = 0usize;
        for t in &self.state.threads {
            if t.ended {
                continue;
            }
            if !t.suspended && t.wake_at != u64::MAX {
                return None;
            }
            live += 1;
        }
        if live < 2 {
            return None;
        }
        Some(self.deadlock_message())
    }

    #[cold]
    #[inline(never)]
    fn deadlock_message(&self) -> String {
        format!(
            "every thread is waiting for another: {}",
            self.state
                .threads
                .iter()
                .filter(|t| !t.ended)
                .map(|t| format!(
                    "thread {} {}",
                    t.id,
                    if t.suspended {
                        "suspended".to_owned()
                    } else {
                        format!(
                            "on handle {}",
                            t.waiting_on
                                .iter()
                                .map(|h| format!("{h:#x}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }

    /// Hand the host every block of the mix the declared clock has passed.
    /// Mixing runs whether or not a device takes the samples, gated to actual
    /// audio-frame horizons so bursts within a frame skip `Mixer::due`.
    #[inline(always)]
    fn mix(&mut self, host: &mut dyn Host) {
        if self.clock.cycles() >= self.next_mix_cycle {
            self.mix_slow(host);
        }
    }

    #[cold]
    #[inline(never)]
    fn mix_slow(&mut self, host: &mut dyn Host) {
        let nanos = self.clock.nanos();
        if self.state.dmusic.is_active() {
            if let Some(event) = self.state.dmusic.advance(nanos, &mut self.state.mixer) {
                win32::kernel32::signal_event(&mut self.state, event);
            }
        }
        for block in self.state.mixer.due(nanos, &self.arena) {
            host.audio(&block);
        }
        self.next_mix_cycle = self
            .clock
            .first_cycle_of_audio_frame(self.state.mixer.next_frame().saturating_add(1));
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

    /// Dispatch the next callback for a machine-owned WINMM thread (`MidiPlayer`
    /// or `MmTimer`), or park the thread until its next event is due (#154, #371).
    fn winmm_turn(&mut self, next: Option<(u32, [u32; 5])>) -> Result<(), Stop> {
        let Some((callback, args)) = next else {
            return Err(Stop::Blocked);
        };
        let here = self.cpu.eip;
        self.call_guest(callback, &args, here, 0)?;
        self.clock.retire();
        Ok(())
    }

    /// WINMM's MIDI stream player takes its turn (#154).
    fn midi_player(&mut self, log: &mut RunLog) -> Result<(), Stop> {
        let mut ctx = self.ctx(log);
        let next = win32::winmm::player_turn(&mut ctx)?;
        self.winmm_turn(next)
    }

    /// WINMM's multimedia timer thread takes its turn (#371).
    fn mm_timer(&mut self, log: &mut RunLog) -> Result<(), Stop> {
        let mut ctx = self.ctx(log);
        let next = win32::winmm::timer_turn(&mut ctx);
        self.winmm_turn(next)
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

        let width = frame.width as usize;
        let height = frame.height as usize;
        let pitch = frame.pitch as usize;
        match frame.depth {
            32 => {
                crate::present::copy_bgra32(source, pitch, &mut self.present_buffer, width, height)
            }
            16 => crate::present::rgb565_to_bgra(
                source,
                pitch,
                &mut self.present_buffer,
                width,
                height,
            ),
            8 => crate::present::paletted8_to_bgra(
                source,
                pitch,
                &mut self.present_buffer,
                width,
                height,
                self.state.palette,
                &self.arena,
            ),
            other => {
                log.line(&format!(
                    "flip refused: {other} bits a pixel is not a declared format"
                ));
                return;
            }
        }

        self.clock.present(self.flip_cost);
        log.line_fmt(format_args!(
            "flip {} cursor {} {}x{}x{}",
            self.flips,
            self.clock.cursor(),
            frame.width,
            frame.height,
            frame.depth
        ));
        // Held to the flip's own time before it is shown, so a frame a
        // machine ahead of real time made is shown no sooner than it is due
        // (#217).
        self.pace(host, log);
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
        let shown = self.state.pointer_shown();
        if shown != self.pointer_shown {
            self.pointer_shown = shown;
            host.show_pointer(shown);
            // Which pointer a person is looking at, and so which of the two
            // ways of moving one is in force once the mouse is captured
            // (#141): where the title draws its own, the host freezes the
            // person's and steers by how far the mouse went; where it does
            // not, the pointer is the person's and its position is its own. A
            // run that feels wrong under the hand is first of all a question
            // of which of those it was in (#40, #64).
            log.line(&format!(
                "pointer flip {} {}",
                self.flips,
                if shown {
                    "the person's own"
                } else {
                    "the title draws it, so a captured mouse is steered"
                }
            ));
        }
        // And the shape the title last set it to, which the host draws over
        // the picture (#139).
        if self.state.cursor != self.cursor_told {
            self.cursor_told = self.state.cursor;
            if let Some(cursor) = self.state.cursor {
                host.pointer_shape(crate::win32::user32::shape(&self.state, cursor).as_ref());
                log.line(&format!("pointer shape flip {} {cursor:#x}", self.flips));
            }
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
        self.observe_flip(host);
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
    /// (#55), every page the guest committed — where Alpha Team's own loader
    /// puts its level images (#98) — and the boundary it calls across, are
    /// the only places a healthy run ever executes. Anything else is a return
    /// address that was not one.
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
            || modules.iter().any(|m| inside(m.base, m.size))
            || self.state.claimed.iter().any(|(s, e)| inside(*s, e - s))
            || inside(crate::guest::heap::HEAP_BASE, self.state.heap.extent()))
    }
}

#[cfg(test)]
mod tests {
    /// Every other call core has written in USER32 and DirectInput: none of
    /// them reads what a person did (#215).
    const NOT_POLLS: &str = "RegisterClassA RegisterClassExA CreateWindowExA DestroyWindow ShowWindow \
        BringWindowToTop UpdateWindow InvalidateRect GetUpdateRect FillRect BeginPaint EndPaint \
        SetFocus TranslateAcceleratorA LoadAcceleratorsA FindWindowA GetActiveWindow \
        GetForegroundWindow GetWindowThreadProcessId EnumDisplaySettingsA ChangeDisplaySettingsA \
        TranslateMessage DispatchMessageA DefWindowProcA PostQuitMessage GetSystemMetrics \
        ShowCursor SetWindowsHookExA UnhookWindowsHookEx CallNextHookEx SetCursor LoadCursorA \
        LoadIconA GetClientRect GetWindowRect AdjustWindowRect AdjustWindowRectEx SetWindowPos \
        SetWindowLongA GetWindowLongA MessageBoxA GetDC ReleaseDC SetCursorPos ClipCursor \
        GetClipCursor OffsetRect IntersectRect PtInRect DrawTextA DialogBoxParamA EndDialog GetDlgItem \
        SendMessageA GetDesktopWindow \
        PostMessageA SetActiveWindow SetTimer KillTimer ClientToScreen MoveWindow EnableWindow \
        SetWindowTextA IsWindow SetCapture ReleaseCapture wsprintfA SystemParametersInfoA \
        DirectInputCreateA QueryInterface AddRef Release CreateDevice EnumDevices GetDeviceStatus \
        RunControlPanel Initialize GetCapabilities EnumObjects GetProperty SetProperty Acquire \
        Unacquire SetDataFormat SetEventNotification SetCooperativeLevel GetObjectInfo \
        GetDeviceInfo CreateEffect EnumEffects GetEffectInfo GetForceFeedbackState \
        SendForceFeedbackCommand EnumCreatedEffectObjects Escape Poll SendDeviceData";

    /// Every USER32 and DirectInput call core has written is a poll or named
    /// as not one, so a reader written later cannot be left off `POLLS`, and
    /// renumber the polls of routes recorded meanwhile when it joins (#215).
    #[test]
    fn every_input_library_call_is_a_poll_or_named_as_not_one() {
        use crate::win32::{dinput, user32};
        let not_polls: Vec<&str> = NOT_POLLS.split_whitespace().collect();
        let calls: Vec<&str> = user32::SHIMS
            .iter()
            .chain(dinput::SHIMS)
            .map(|shim| shim.name)
            .chain(
                dinput::INTERFACES
                    .iter()
                    .flat_map(|interface| interface.methods.iter().map(|method| method.name)),
            )
            .collect();
        for call in &calls {
            assert!(
                super::POLLS.contains(call) != not_polls.contains(call),
                "{call} is a poll or not one, and only one"
            );
        }
        for name in super::POLLS.iter().chain(&not_polls) {
            assert!(calls.contains(name), "{name} is a call core has written");
        }
    }

    #[test]
    fn stack_args_handles_zero_inline_and_overflow_fallback() {
        use super::{Machine, INLINE_STACK_ARGS};
        use crate::cpu::Stop;
        use crate::guest::{Fault, GuestMem, DOS_MEMORY};
        use crate::host::headless::Headless;
        use crate::Title;

        let mut host = Headless::new();
        let mut machine = Machine::new(&Title::STUB, &mut host).expect("a machine");
        let base = 0x0020_0000u32;
        for i in 0..20u32 {
            machine
                .arena
                .write_u32(base + i * 4, 0x1000 + i)
                .expect("in arena");
        }

        let mut buf = [0u32; INLINE_STACK_ARGS];
        let mut heap = Vec::new();
        let empty = machine
            .stack_args(base, 0, &mut buf, &mut heap)
            .expect("zero args");
        assert_eq!(empty, &[]);
        assert!(heap.is_empty());

        let inline = machine
            .stack_args(base, INLINE_STACK_ARGS as u32, &mut buf, &mut heap)
            .expect("inline args");
        let expected_inline: Vec<u32> = (0..16).map(|i| 0x1000 + i).collect();
        assert_eq!(inline, expected_inline.as_slice());
        assert!(heap.is_empty());

        let overflow = machine
            .stack_args(base, 20, &mut buf, &mut heap)
            .expect("overflow args");
        let expected_overflow: Vec<u32> = (0..20).map(|i| 0x1000 + i).collect();
        assert_eq!(overflow, expected_overflow.as_slice());

        // Faults report the exact faulting stack slot in both paths.
        let end = u32::try_from(machine.arena.len()).unwrap();
        assert_eq!(
            machine.stack_args(end - 8, 4, &mut buf, &mut heap).err(),
            Some(Stop::Fault(Fault {
                addr: end,
                len: 4,
                write: false,
            }))
        );
        assert_eq!(
            machine
                .stack_args(end - 4 * 17, 20, &mut buf, &mut heap)
                .err(),
            Some(Stop::Fault(Fault {
                addr: end,
                len: 4,
                write: false,
            }))
        );
        // Floor fault on the first slot reports the exact below-floor address.
        assert_eq!(
            machine
                .stack_args(DOS_MEMORY - 4, 4, &mut buf, &mut heap)
                .err(),
            Some(Stop::Fault(Fault {
                addr: DOS_MEMORY - 4,
                len: 4,
                write: false,
            }))
        );
        // Wrapping around u32::MAX reports the exact first faulting slot.
        assert_eq!(
            machine
                .stack_args(u32::MAX - 3, 2, &mut buf, &mut heap)
                .err(),
            Some(Stop::Fault(Fault {
                addr: u32::MAX - 3,
                len: 4,
                write: false,
            }))
        );
    }

    #[test]
    fn next_mix_cycle_gate_matches_unconditional_mix() {
        use super::Machine;
        use crate::host::headless::Headless;
        use crate::sound::{Mixer, BLOCK_FRAMES};
        use crate::Title;

        let mut host = Headless::new();
        let mut machine = Machine::new(&Title::STUB, &mut host).expect("a machine");
        let mut reference = Mixer::default();
        let samples: Vec<i16> = (0..(BLOCK_FRAMES * 6) as i16).collect();
        machine.state.mixer.play(1, samples.clone(), 0);
        reference.play(1, samples, 0);

        let mut ref_blocks = Vec::new();
        // Step across >2 audio blocks in small increments so most bursts fall between frames.
        for _ in 0..10_000 {
            machine.clock.retire_many(5_000);
            machine.mix(&mut host);
            ref_blocks.extend(reference.due(machine.clock.nanos(), &machine.arena));
        }
        assert!(ref_blocks.len() >= 2);
        assert_eq!(machine.state.mixer.next_frame(), reference.next_frame());
        assert_eq!(host.audio_blocks, ref_blocks);
    }

    #[test]
    fn poll_next_route_at_fast_path_delivers_exact_route_events() {
        use super::Machine;
        use crate::cli::Trace;
        use crate::host::headless::Headless;
        use crate::host::{Button, Input};
        use crate::route::Route;
        use crate::runlog::RunLog;
        use crate::Title;

        let mut host = Headless::new();
        let mut machine = Machine::new(&Title::STUB, &mut host).expect("a machine");
        let mut route = Route::new();
        route.push((0, 0), Input::Down(Button::Left));
        route.push((0, 3), Input::Up(Button::Left));
        route.push((2, 2), Input::Move { x: 10, y: 20 });
        machine.route = Some(route);

        let mut log = RunLog::create("poll-fast-path", Trace::Off).expect("log");
        let dir = log.dir().to_path_buf();

        // Flip 0 delivers (0, 0) and arms next_route_at = (0, 3).
        machine.deliver_input(&mut host, &mut log);
        machine.flips += 1;
        assert_eq!(machine.next_route_at, (0, 3));
        assert_eq!(machine.input, vec![((0, 0), Input::Down(Button::Left))]);

        // Polls 1 and 2 hit the fast return; poll 3 delivers (0, 3) and advances to (2, 2).
        for _ in 0..2 {
            machine.poll(&mut host, &mut log);
        }
        assert_eq!(machine.input.len(), 1);
        machine.poll(&mut host, &mut log);
        assert_eq!(machine.input.len(), 2);
        assert_eq!(machine.input[1], ((0, 3), Input::Up(Button::Left)));
        assert_eq!(machine.next_route_at, (2, 2));

        // Flip 1 and its polls deliver nothing.
        machine.deliver_input(&mut host, &mut log);
        machine.flips += 1;
        for _ in 0..5 {
            machine.poll(&mut host, &mut log);
        }
        assert_eq!(machine.input.len(), 2);

        // Flip 2, poll 2 delivers the last event and saturates next_route_at.
        machine.deliver_input(&mut host, &mut log);
        machine.flips += 1;
        machine.poll(&mut host, &mut log);
        assert_eq!(machine.input.len(), 2);
        machine.poll(&mut host, &mut log);
        assert_eq!(machine.input.len(), 3);
        assert_eq!(machine.input[2], ((2, 2), Input::Move { x: 10, y: 20 }));
        assert_eq!(machine.next_route_at, (u64::MAX, u64::MAX));

        drop(log);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn switch_thread_skips_cpu_copy_when_same_thread_continues() {
        use super::Machine;
        use crate::cli::Trace;
        use crate::cpu::Cpu;
        use crate::host::headless::Headless;
        use crate::runlog::RunLog;
        use crate::win32::GuestThread;
        use crate::Title;

        let mut host = Headless::new();
        let mut machine = Machine::new(&Title::STUB, &mut host).expect("a machine");
        let sentinel = 0xdead_beefu32;
        machine.state.threads = vec![
            GuestThread {
                cpu: Cpu {
                    eip: sentinel,
                    ..Cpu::default()
                },
                handle: 1,
                id: 1,
                suspended: false,
                ended: false,
                code: 0,
                tls: Vec::new(),
                wake_at: 0,
                gives_up_at: 0,
                waiting_on: Vec::new(),
                time_critical: false,
            },
            GuestThread {
                cpu: Cpu::default(),
                handle: 2,
                id: 2,
                suspended: false,
                ended: false,
                code: 0,
                tls: Vec::new(),
                wake_at: u64::MAX,
                gives_up_at: 0,
                waiting_on: vec![0x10],
                time_critical: false,
            },
        ];
        machine.state.thread = 0;
        machine.cpu.eip = 0x1234_5678;

        let mut log = RunLog::create("switch-skip", Trace::Off).expect("log");
        let dir = log.dir().to_path_buf();
        machine.switch_thread(&mut log);
        assert_eq!(machine.state.thread, 0);
        assert_eq!(machine.cpu.eip, 0x1234_5678);
        // Because next == running, threads[0].cpu was never overwritten.
        assert_eq!(machine.state.threads[0].cpu.eip, sentinel);

        drop(log);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn deadlocked_short_circuits_with_fewer_than_two_live_threads() {
        use super::Machine;
        use crate::cpu::Cpu;
        use crate::host::headless::Headless;
        use crate::win32::GuestThread;
        use crate::Title;

        let thread = |id: u32, suspended: bool, ended: bool, wake_at: u64, waiting_on: Vec<u32>| {
            GuestThread {
                cpu: Cpu::default(),
                handle: id,
                id,
                suspended,
                ended,
                code: 0,
                tls: Vec::new(),
                wake_at,
                gives_up_at: 0,
                waiting_on,
                time_critical: false,
            }
        };

        let mut host = Headless::new();
        let mut machine = Machine::new(&Title::STUB, &mut host).expect("a machine");

        // Single-threaded (default) short-circuits with None.
        machine.state.threads = vec![thread(1, false, false, u64::MAX, vec![0x10])];
        assert!(machine.deadlocked().is_none());

        // Two threads, one ended: only one live thread, not deadlocked.
        machine.state.threads = vec![
            thread(1, false, false, u64::MAX, vec![0x10]),
            thread(2, false, true, u64::MAX, vec![]),
        ];
        assert!(machine.deadlocked().is_none());

        // Two live threads, one runnable (finite wake_at): not deadlocked.
        machine.state.threads = vec![
            thread(1, false, false, u64::MAX, vec![0x10]),
            thread(2, false, false, 100, vec![0x20]),
        ];
        assert!(machine.deadlocked().is_none());

        // Two live threads, both blocked indefinitely / suspended: deadlocked.
        machine.state.threads = vec![
            thread(1, false, false, u64::MAX, vec![0x10, 0x20]),
            thread(2, true, false, 0, vec![]),
        ];
        assert_eq!(
            machine.deadlocked().as_deref(),
            Some("every thread is waiting for another: thread 1 on handle 0x10, 0x20, thread 2 suspended")
        );
    }
}
