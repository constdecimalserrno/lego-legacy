//! What Microsoft wrote, written here instead (ADR-0002).
//!
//! Each entry point a title imports is declared once, with the number of
//! arguments it takes and who cleans them off the stack. A guest calling one
//! arrives through the boundary, the shim answers, and the call returns as if
//! the real library had been there.

pub mod advapi32;
pub mod amstream;
pub mod avifile;
pub mod d3d;
pub mod d3drm;
pub mod ddraw;
pub mod dinput;
pub mod dsound;
pub mod gdi32;
pub mod kernel32;
pub mod msacm32;
pub mod msvcrt;
pub mod ole32;
pub mod pipeline;
pub mod scene;
pub mod user32;
pub mod version;
pub mod winmm;

use crate::cpu::{Cpu, Stop};
use crate::guest::heap::Heap;
use crate::guest::{Arena, Fault, GuestMem};
use crate::runlog::RunLog;
use crate::Title;
use std::collections::{BTreeMap, VecDeque};

/// Who takes the arguments off the stack when a call returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Convention {
    /// The callee does, which is how Win32 is called.
    Stdcall,
    /// The caller does, which is how the C runtime is called.
    Cdecl,
}

/// One entry point the emulator implements.
pub struct Shim {
    pub dll: &'static str,
    pub name: &'static str,
    /// How many four-byte arguments it takes.
    pub args: u32,
    pub convention: Convention,
    pub run: ShimFn,
}

/// What a shim does. It reads its arguments, changes the machine, and returns
/// what the guest will find in `eax`.
pub type ShimFn = fn(&mut Ctx<'_>, &[u32]) -> Result<u32, Stop>;

/// An entry point that takes its own arguments off the stack, spelled on one
/// line.
#[must_use]
pub const fn stdcall(dll: &'static str, name: &'static str, args: u32, run: ShimFn) -> Shim {
    Shim {
        dll,
        name,
        args,
        convention: Convention::Stdcall,
        run,
    }
}

/// An entry point whose caller takes the arguments off the stack, spelled on
/// one line.
#[must_use]
pub const fn cdecl(dll: &'static str, name: &'static str, args: u32, run: ShimFn) -> Shim {
    Shim {
        dll,
        name,
        args,
        convention: Convention::Cdecl,
        run,
    }
}

/// One open thing the guest holds a handle to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    /// A module the guest asked for by name.
    Module { name: String, base: u32 },
    /// The process heap, or one the guest made.
    Heap,
    /// One of the three standard streams.
    Std(u32),
    /// A file the guest opened, and where it is reading or writing.
    File {
        path: String,
        contents: Contents,
        at: u32,
        /// What the guest asked to do through this handle, as `CreateFileA`'s
        /// `GENERIC_READ` and `GENERIC_WRITE` bits. What it writes reaches the
        /// profile when it closes the file.
        access: u32,
        /// What it lets another handle on the same file do, as `CreateFileA`'s
        /// `FILE_SHARE_*` bits: a second open is judged by it (#116).
        share: u32,
        /// Whether it actually wrote anything. A file opened for writing and
        /// never written to is not one the profile keeps a copy of.
        wrote: bool,
    },
    /// Something named a thread can wait on: an event, a mutex, a thread.
    Named {
        kind: &'static str,
        name: String,
        /// Whether a wait on it is over now: an event that has been set and
        /// not yet taken, or a mutex nobody holds (#60).
        signalled: bool,
        /// An event that stays signalled until it is reset, rather than one
        /// waiter taking it back down.
        manual: bool,
        /// Which thread holds a mutex and how many times over. A thread may
        /// take one it already holds, and has to release it as often.
        held: Option<(u32, u32)>,
    },
    /// A counted semaphore: how many waits it can answer now, and the most it
    /// may ever hold. Each wait it answers takes one (#115).
    Semaphore { count: u32, max: u32 },
    /// A hook a title asked to put on the window system, by which hook it is.
    /// Nothing on this machine calls one (#60).
    Hook(u32),
    /// An enumeration of the declared tree the guest is walking: what matched,
    /// and how far through it the guest has read.
    Find {
        matches: Vec<(String, u32)>,
        at: usize,
    },
    /// A file the guest asked to see as memory rather than read: where its
    /// bytes are in the arena, and how many of them there are (#57).
    Mapping { at: u32, len: u32 },
    /// A registry key the guest has open.
    Key { path: String },
    /// A MIDI stream a title has open; what it is playing is in `State::midi`
    /// (#154).
    MidiStream,
    /// A multimedia file over guest memory, and where it is reading; `owned`
    /// when the machine put the file there and frees it on close.
    Mmio {
        base: u32,
        len: u32,
        at: u32,
        owned: bool,
    },
}

/// Where an open file's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Contents {
    /// Held by the machine: a file the guest can write through, or one the
    /// tree unpacks, declares or keeps in the profile.
    Held(Vec<u8>),
    /// Where on the pressing's image a file opened only to read lies: its
    /// first byte there and its length. What the guest reads is read from
    /// there when it asks, so that LEGO Island's 157 MB script is not held
    /// whole while the title reads it a piece at a time (#116).
    Lies { start: u64, len: u32 },
}

impl Contents {
    /// How many bytes the file holds.
    #[must_use]
    pub fn size(&self) -> u32 {
        match self {
            Self::Held(bytes) => u32::try_from(bytes.len()).unwrap_or(u32::MAX),
            Self::Lies { len, .. } => *len,
        }
    }
}

/// Where files mapped into the arena start.
///
/// A declared value: above the modules the loader places, which are above the
/// heap, so that a title that maps its whole install directory does not land
/// on anything else.
pub const MAPPING_BASE: u32 = 0x4400_0000;

/// A guest thread the machine has not got to yet (ADR-0005).
///
/// One host thread runs them all, so a thread that is not running has its
/// registers parked here. What the machine switches is which of these is
/// `Machine::cpu`.
pub struct GuestThread {
    /// Where it was up to, while another thread runs.
    pub cpu: crate::cpu::Cpu,
    /// The handle the guest was given for it.
    pub handle: u32,
    /// What `GetCurrentThreadId` answers while it runs.
    pub id: u32,
    /// Made suspended, or suspended since: it is not run until it is resumed.
    pub suspended: bool,
    /// It returned, or called `ExitThread`. Its registers are not run again.
    pub ended: bool,
    /// What it answered when it ended.
    pub code: u32,
    /// Its own thread-local slots, which is what makes them thread-local: a
    /// C runtime keeps one of these per thread and every thread of it would
    /// otherwise be using the last one's (ADR-0005).
    pub tls: Vec<u32>,
    /// The declared millisecond it asked to be woken at, for a thread that is
    /// sleeping or waiting. Until then the machine runs somebody else.
    /// `u64::MAX` is a wait with no timeout: only a signal lifts it.
    pub wake_at: u64,
    /// The hour its wait runs out at, kept apart from `wake_at`, which a
    /// signal clears: a wait woken by something it could not take goes back
    /// down until then (#130).
    pub gives_up_at: u64,
    /// The handles it is parked on, while it waits for them. Signalling any
    /// of them is what wakes it before its timeout runs out (#60, #130).
    pub waiting_on: Vec<u32>,
    /// It runs whenever it can run, ahead of every other thread, and keeps
    /// the processor until it waits: as Windows runs a thread of
    /// `THREAD_PRIORITY_TIME_CRITICAL` on a machine of one processor. Only
    /// WINMM's MIDI stream player is, as Wine's `midiStreamOpen` makes it
    /// (#154); the priority a title asks for its own threads is not honoured
    /// (`SetThreadPriority`).
    pub time_critical: bool,
}

/// Everything the guest holds that is not memory.
pub struct State {
    /// Blocks handed to the C runtime's written `free` or `realloc` that this
    /// machine's heap never handed out, so it could not answer for them (#80).
    pub foreign_frees: u64,
    /// Files the guest wrote that the host would not keep, said again when
    /// the run ends: a save that never reached the profile is lost.
    pub saves_refused: u64,
    pub heap: Heap,
    /// Handles the guest has been given, indexed by `(handle - HANDLE_BASE) >> 2`
    /// in ascending creation order. Closed handles leave `None` so handles are
    /// never reused.
    handles: Vec<Option<Object>>,
    /// The command line the guest reads, once it has asked for it.
    pub command_line: Option<u32>,
    /// Optional arguments appended after the quoted image path in
    /// `GetCommandLineA`/`W` (#370).
    pub guest_args: String,
    /// Per-process environment variable overrides set via
    /// `SetEnvironmentVariableA` (keyed ASCII-uppercase; `None` = removed, #370).
    pub environment: BTreeMap<String, Option<String>>,
    /// What the guest will read from `GetLastError`.
    pub last_error: u32,
    /// Every guest thread but the one running, parked (ADR-0005). Empty until
    /// a title makes a second one, which Alpha Team does for its sound.
    pub threads: Vec<GuestThread>,
    /// Which thread of `threads` is the one running, once there is more than
    /// one. The main thread is always the first.
    pub thread: usize,
    /// What the next thread this machine makes will be called.
    pub next_thread_id: u32,
    /// How many thread-local slots `TlsAlloc` has handed out. Which thread's
    /// they are is the thread's own business.
    pub tls_slots: usize,
    /// Which thread holds each critical section the guest has entered, and how
    /// many times over (ADR-0005), as `(section, owner, depth)`. A section
    /// nobody holds is not in here.
    pub sections: Vec<(u32, u32, u32)>,
    /// How many guest threads are currently parked waiting on a critical
    /// section. When zero, `LeaveCriticalSection` skips waking waiters and
    /// `EnterCriticalSection` skips unparking.
    pub section_waiters: u32,
    /// Address ranges the guest has claimed by naming an address to
    /// `VirtualAlloc`, outside the heap. A second claim that overlaps one is
    /// refused, which is what tells a guest's own DLL loader to relocate the
    /// image rather than lay it over the last one (#61).
    pub claimed: Vec<(u32, u32)>,
    /// Every region `VirtualAlloc` reserved, by where it starts: how big it
    /// is and the protection it was reserved with. What `VirtualQuery`
    /// answers from (#118).
    pub reserved: BTreeMap<u32, (u32, u32)>,
    /// Every page committed in them, by address, with the protection it was
    /// committed with.
    pub committed: BTreeMap<u32, u32>,
    /// Where the next file mapped into the arena will go. Mappings sit above
    /// everything else the machine hands out, because Alpha Team maps four
    /// hundred megabytes of archives and never lets go (#57).
    pub next_mapping: u32,
    /// Every file mapped into `MAPPING_BASE..next_mapping` as `(at, len, path)`,
    /// so cold snapshot restores can re-populate mapped disc archives from
    /// `Tree` without storing 400 MB of read-only bytes inside snapshots (#313).
    pub mapped_files: Vec<(u32, u32, String)>,
    /// Which failures the guest asked to have handed back rather than shown:
    /// what `SetErrorMode` was last told. Read when something looks on the
    /// empty floppy drive: with `SEM_FAILCRITICALERRORS` the call fails as
    /// the drive not being ready, and without it the run stops at Windows'
    /// box asking for a disk (#145).
    pub error_mode: u32,
    /// Where the guest believes it is.
    pub current_directory: String,
    /// The label on the disc in the declared machine's drive.
    pub volume_label: String,
    /// Declared time the guest spent waiting inside the last call: a sleep,
    /// or a blocking wait for a movie frame. The machine charges it to the
    /// declared clock when the call returns (ADR-0003).
    pub waited_millis: u64,
    /// The running thread gave the rest of its turn away inside the last
    /// call, by `Sleep(0)`: the machine switches before it runs any more of
    /// it, rather than at its next call (ADR-0005, #115).
    pub yielded: bool,
    /// How many files the declared install tree holds.
    pub installed_files: usize,
    /// The window procedure the guest registered, if it has.
    pub window_procedure: u32,
    /// Whether the guest has made its window, and not destroyed it (#140).
    pub window_made: bool,
    /// The thread that asked to quit, and the code it asked with, until its
    /// queue hands it `WM_QUIT` (#140).
    pub quit: Option<(u32, u32)>,
    /// The thread that made it, whose queue its messages wait in.
    pub window_thread: u32,
    /// The style its window was made with.
    pub window_style: u32,
    /// The instance its window was made with.
    pub window_instance: u32,
    /// Extra per-window bytes requested by `WNDCLASSA.cbWndExtra`, stored via
    /// `SetWindowLongA` at non-negative offsets and read back via
    /// `GetWindowLongA` (#370).
    pub window_extra: Vec<u8>,
    /// Where the pointer is, in the guest's own coordinates.
    pub pointer: (i32, i32),
    /// Where a person last put the pointer, which is what the mouse moved
    /// from: a title that sets the pointer somewhere else has not moved the
    /// mouse.
    pub last_move: (i32, i32),
    /// What the screen is, as the guest last set it.
    pub display_mode: (u32, u32, u32),
    /// Each surface's source colour key, by where its pixels are.
    pub color_keys: BTreeMap<u32, u32>,
    /// The surfaces with a clipper attached, by where their pixels are.
    pub clipped: std::collections::BTreeSet<u32>,
    /// Each clipper's one clip rectangle, by the clipper.
    pub clip_lists: BTreeMap<u32, (i32, i32, i32, i32)>,
    /// The device contexts surfaces have handed out and not yet had back, by
    /// the DC, each with the surface it draws on (#151).
    pub surface_dcs: BTreeMap<u32, u32>,
    /// The DrawDibs a title has open, by their handles, and the handle the
    /// next one will be given (#151).
    pub drawdibs: std::collections::BTreeSet<u32>,
    pub next_drawdib: u32,
    /// The ACM streams a title has open, by their handles, and the handle the
    /// next one will be given, counted from one (#153).
    pub acm_streams: BTreeMap<u32, msacm32::Stream>,
    pub next_acm_stream: u32,
    /// The palette the screen is showing through, if any.
    pub palette: u32,
    /// The frame the guest last flipped, waiting to be presented.
    pub present: Option<Present>,
    /// Every sound playing, mixed on the declared clock.
    pub mixer: crate::sound::Mixer,
    /// DirectSound's buffers and listener.
    pub sound: dsound::Sounds,
    /// The multimedia streams a title has made, by the object it holds.
    pub players: BTreeMap<u32, amstream::Player>,
    /// The movies a title has opened through AVIFile, by the file object it
    /// holds (#151).
    pub avi_files: BTreeMap<u32, avifile::File>,
    /// The bytes of each movie file a title has opened through AVIFile, by its
    /// path as the machine reads it: one copy, however often it is opened.
    pub movie_bytes: BTreeMap<String, std::rc::Rc<[u8]>>,
    /// The frame getters a title has opened on their streams, by the getter
    /// it holds.
    pub frame_getters: BTreeMap<u32, avifile::Getter>,
    /// The CD audio device.
    pub cd: winmm::CdAudio,
    /// The MIDI out's player and the stream on it, once a title has opened
    /// one (#154).
    pub midi: Option<winmm::Midi>,
    /// Active multimedia timers registered via `WINMM!timeSetEvent` (#371),
    /// and the WINMM timer thread that dispatches their callbacks.
    pub mm_timers: Vec<winmm::MmTimer>,
    pub next_mm_timer: u32,
    pub mm_timer_thread: Option<usize>,
    /// Which keys are down, as the guest's own device numbers them.
    pub keys_down: Vec<u32>,
    /// Which virtual keys have gone down since the guest last asked
    /// `GetAsyncKeyState` about each.
    pub keys_pressed: Vec<u32>,
    /// Which keys are down as the title has read them, numbered as
    /// `keys_down`: the key messages it has taken off its queue, which is the
    /// key state Windows keeps for a thread and translates its keys with
    /// (#157), where `keys_down` is what is down now.
    pub keys_read: Vec<u32>,
    /// How far the pointer moved since the guest last looked.
    pub mouse_delta: (i32, i32),
    /// What a person did, waiting in the buffered lane: where in the device's
    /// data format it happened and what the value there became, oldest first.
    /// A title reads one of these lanes or the other, and this machine keeps
    /// both because which one it reads is the title's business.
    pub mouse_data: VecDeque<(u32, u32)>,
    pub keyboard_data: VecDeque<(u32, u32)>,
    /// The number the next buffered event carries. DirectInput's own sequence
    /// numbers only have to rise.
    pub input_sequence: u32,
    /// Whether the pointer's left button is down, and its right one.
    pub mouse_down: bool,
    pub right_down: bool,
    /// When each button last went down and where, so that a second click near
    /// it and soon after it is a double click rather than two clicks (#59).
    /// A button counts only its own clicks.
    pub last_click: (u32, (i32, i32)),
    pub last_right_click: (u32, (i32, i32)),
    /// Whether the class the title registered asked for double clicks.
    pub double_clicks: bool,
    /// The rectangle the title confined the pointer to with `ClipCursor`,
    /// cut to the screen, or none (#147).
    pub clip: Option<[i32; 4]>,
    /// Whether the title has told Windows a screen saver is running, which
    /// Windows 9x keeps to hand back as the one it replaces (#147).
    pub screen_saver_running: bool,
    /// Whether the title's window has an update region, which Windows
    /// answers with `WM_PAINT` once its queue has nothing else, and if so
    /// whether its background is to be erased (#147).
    pub paint_erase: Option<bool>,
    /// Where the title last asked that the pointer be put, until the machine
    /// has told the host about it (#59).
    pub pointer_put: Option<(i32, i32)>,
    /// Windows' own show count for the pointer: it is drawn while this is
    /// zero or more, and a title that draws its own takes it below zero.
    pub cursor_shown: i32,
    /// The cursor the title last set with `SetCursor`, until it has set one.
    pub cursor: Option<u32>,
    /// The cursors it loaded out of its own modules, by the module and the
    /// name, in the order they were loaded (#139).
    pub cursors: Vec<(u32, u32, crate::host::Cursor)>,
    /// Window messages waiting for the title to take them, oldest first.
    pub messages: VecDeque<Message>,
    /// The timers a title has set, oldest first (#138).
    pub timers: Vec<user32::Timer>,
    /// The modal dialog a title has up, while it has one.
    pub dialog: Option<user32::Dialog>,
    /// What a title named its objects, by the object.
    pub names: BTreeMap<u32, String>,
    /// Values a title set on an object and may read back, by the object and
    /// what the value is.
    pub set_values: BTreeMap<(u32, &'static str), u32>,
    /// What immediate mode keeps about its devices and materials.
    pub immediate: d3d::Immediate,
    /// The scene retained mode is building.
    pub scene: scene::Scene,
    /// A viewport rendering, while its visuals are being called.
    pub render: Option<d3drm::RenderPass>,
    /// COM objects, by the guest pointer that names them.
    com: BTreeMap<u32, ComObject>,
    /// Every face of every object, by the object and the interface.
    faces: BTreeMap<(u32, &'static str), u32>,
    /// Freed DirectDraw surface pixel buffers kept by rounded allocation size,
    /// segregated from `State::heap` so small guest `malloc`/COM allocations
    /// never hijack large surface buffers (#338).
    surface_pool: BTreeMap<u32, VecDeque<u32>>,
    /// Freed 8-byte surface COM face slots reused by `make_com` (#338).
    free_com: VecDeque<u32>,
    /// Which vtable an interface's methods live behind, once one is made.
    vtables: BTreeMap<&'static str, u32>,
    /// The registry, as a flat map from a key's whole path to its values.
    ///
    /// Seeded with what a title's installer would have written, and added to
    /// by whatever the guest writes back.
    registry: BTreeMap<String, BTreeMap<String, (u32, Vec<u8>)>>,
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    #[must_use]
    pub fn new() -> Self {
        Self {
            foreign_frees: 0,
            saves_refused: 0,
            heap: Heap::new(),
            handles: Vec::new(),
            command_line: None,
            guest_args: String::new(),
            environment: BTreeMap::new(),
            last_error: 0,
            error_mode: 0,
            // The thread a title starts on is in the list from the start, so
            // that what belongs to a thread has somewhere to live before a
            // second one is made.
            threads: vec![GuestThread {
                cpu: crate::cpu::Cpu::default(),
                handle: 0,
                id: crate::machine::THREAD_ID,
                suspended: false,
                ended: false,
                code: 0,
                tls: Vec::new(),
                wake_at: 0,
                gives_up_at: 0,
                waiting_on: Vec::new(),
                time_critical: false,
            }],
            thread: 0,
            next_thread_id: crate::machine::THREAD_ID,
            claimed: Vec::new(),
            reserved: BTreeMap::new(),
            committed: BTreeMap::new(),
            next_mapping: MAPPING_BASE,
            mapped_files: Vec::new(),
            sections: Vec::new(),
            section_waiters: 0,
            tls_slots: 0,
            current_directory: String::new(),
            volume_label: String::new(),
            waited_millis: 0,
            yielded: false,
            players: BTreeMap::new(),
            avi_files: BTreeMap::new(),
            movie_bytes: BTreeMap::new(),
            frame_getters: BTreeMap::new(),
            cd: winmm::CdAudio::default(),
            midi: None,
            mm_timers: Vec::new(),
            next_mm_timer: 1,
            mm_timer_thread: None,
            mixer: crate::sound::Mixer::default(),
            sound: dsound::Sounds::default(),
            installed_files: 0,
            window_procedure: 0,
            window_made: false,
            quit: None,
            window_thread: crate::machine::THREAD_ID,
            window_style: 0,
            window_instance: 0,
            window_extra: Vec::new(),
            // The middle of the screen: where Windows leaves the pointer when
            // a mode is set, and so where a title that draws its own starts
            // it. The first thing the mouse does is measured from here, and
            // measured from anywhere else it is a move nobody made, and the
            // title's pointer and the person's are apart by that much until an
            // edge of the screen happens to bring one axis back (#48).
            pointer: (320, 240),
            last_move: (320, 240),
            display_mode: (640, 480, 16),
            color_keys: BTreeMap::new(),
            clipped: std::collections::BTreeSet::new(),
            clip_lists: BTreeMap::new(),
            surface_dcs: BTreeMap::new(),
            drawdibs: std::collections::BTreeSet::new(),
            next_drawdib: 1,
            acm_streams: BTreeMap::new(),
            next_acm_stream: 1,
            palette: 0,
            present: None,
            keys_down: Vec::new(),
            keys_pressed: Vec::new(),
            keys_read: Vec::new(),
            mouse_delta: (0, 0),
            mouse_data: VecDeque::new(),
            keyboard_data: VecDeque::new(),
            input_sequence: 0,
            mouse_down: false,
            right_down: false,
            last_click: (0, (0, 0)),
            last_right_click: (0, (0, 0)),
            pointer_put: None,
            double_clicks: false,
            clip: None,
            screen_saver_running: false,
            paint_erase: None,
            cursor_shown: 0,
            cursor: None,
            cursors: Vec::new(),
            messages: VecDeque::new(),
            timers: Vec::new(),
            dialog: None,
            names: BTreeMap::new(),
            set_values: BTreeMap::new(),
            immediate: d3d::Immediate::default(),
            scene: scene::Scene::default(),
            render: None,
            com: BTreeMap::new(),
            faces: BTreeMap::new(),
            surface_pool: BTreeMap::new(),
            free_com: VecDeque::new(),
            vtables: BTreeMap::new(),
            registry: BTreeMap::new(),
        }
    }

    /// Give the guest a handle to something.
    ///
    /// Handles are small multiples of four starting at [`HANDLE_BASE`], as
    /// Windows' are, and below anything the machine hands out as memory, so
    /// one is never mistaken for an address the guest was given.
    pub fn give(&mut self, object: Object) -> u32 {
        let handle = HANDLE_BASE + (self.handles.len() as u32) * 4;
        self.handles.push(Some(object));
        handle
    }

    #[inline(always)]
    fn handle_index(handle: u32) -> Option<usize> {
        let offset = handle.checked_sub(HANDLE_BASE)?;
        if offset & 3 != 0 {
            return None;
        }
        Some((offset >> 2) as usize)
    }

    #[must_use]
    pub fn object(&self, handle: u32) -> Option<&Object> {
        self.handles.get(Self::handle_index(handle)?)?.as_ref()
    }

    pub fn object_mut(&mut self, handle: u32) -> Option<&mut Object> {
        self.handles.get_mut(Self::handle_index(handle)?)?.as_mut()
    }

    /// Take a handle back.
    pub fn close(&mut self, handle: u32) -> bool {
        Self::handle_index(handle)
            .and_then(|i| self.handles.get_mut(i))
            .and_then(Option::take)
            .is_some()
    }

    /// Every currently open object, in ascending handle order.
    pub(super) fn objects(&self) -> impl Iterator<Item = &Object> {
        self.handles.iter().filter_map(Option::as_ref)
    }

    /// Read-only view of all open handles as `(handle, &Object)` (#309).
    pub fn open_handles(&self) -> impl Iterator<Item = (u32, &Object)> {
        self.handles
            .iter()
            .enumerate()
            .filter_map(|(i, o)| o.as_ref().map(|obj| (HANDLE_BASE + (i as u32) * 4, obj)))
    }

    /// Resolve `path` against `self.current_directory` using the same rule as
    /// `Ctx::absolute`.
    #[must_use]
    pub fn absolute_path(&self, path: &str) -> String {
        let path = path.trim();
        let here = &self.current_directory;
        if path.len() > 1 && path.as_bytes().get(1) == Some(&b':') {
            path.to_owned()
        } else if path.starts_with(['\\', '/']) {
            let drive = if here.as_bytes().get(1) == Some(&b':') {
                &here[..2]
            } else {
                ""
            };
            format!("{drive}{path}")
        } else {
            format!("{}\\{path}", here.strip_suffix('\\').unwrap_or(here))
        }
    }

    /// Temporarily detach large immutable read-only `Contents::Held` buffers
    /// from open handles during `LLSNAP v2` capture so 250–400 MB unpacked
    /// archives are not copied into every snapshot (#313).
    pub(crate) fn stash_readonly_files(
        &mut self,
        can_rehydrate: impl Fn(&str) -> bool,
    ) -> Vec<(usize, String, Vec<u8>)> {
        const MIN_DEFER_BYTES: usize = 65_536;
        // GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA.
        const WRITE_ACCESS_BITS: u32 = 0x5000_0006;
        let cwd = self.current_directory.clone();
        let resolve = |path: &str| -> String {
            let path = path.trim();
            if path.len() > 1 && path.as_bytes().get(1) == Some(&b':') {
                path.to_owned()
            } else if path.starts_with(['\\', '/']) {
                let drive = if cwd.as_bytes().get(1) == Some(&b':') {
                    &cwd[..2]
                } else {
                    ""
                };
                format!("{drive}{path}")
            } else {
                format!("{}\\{path}", cwd.strip_suffix('\\').unwrap_or(&cwd))
            }
        };
        let mut stashed = Vec::new();
        for (idx, slot) in self.handles.iter_mut().enumerate() {
            if let Some(Object::File {
                path,
                contents: Contents::Held(bytes),
                access,
                wrote: false,
                ..
            }) = slot
            {
                if (*access & WRITE_ACCESS_BITS) == 0 && bytes.len() >= MIN_DEFER_BYTES {
                    let resolved = resolve(path);
                    if can_rehydrate(&resolved) {
                        stashed.push((idx, resolved, std::mem::take(bytes)));
                    }
                }
            }
        }
        stashed
    }

    /// Put stashed read-only `Contents::Held` buffers back into their handle slots.
    pub(crate) fn unstash_readonly_files(&mut self, stashed: Vec<(usize, String, Vec<u8>)>) {
        for (idx, _, bytes) in stashed {
            if let Some(Some(Object::File {
                contents: Contents::Held(slot),
                ..
            })) = self.handles.get_mut(idx)
            {
                *slot = bytes;
            }
        }
    }

    /// Take the live handle table before a snapshot restore so matching open
    /// read-only archive buffers can be swapped back in with zero copy (#313).
    pub(crate) fn take_handles(&mut self) -> Vec<Option<Object>> {
        std::mem::take(&mut self.handles)
    }

    /// Mutable access to raw handle table slot `idx` for snapshot rehydration.
    pub(crate) fn handle_slot_mut(&mut self, idx: usize) -> Option<&mut Object> {
        self.handles.get_mut(idx)?.as_mut()
    }

    /// Read-only view of primary COM objects (where `obj.identity == addr`) (#309).
    pub fn com_objects(&self) -> impl Iterator<Item = (u32, &ComObject)> {
        self.com
            .iter()
            .filter(|(&addr, obj)| obj.identity == addr)
            .map(|(&addr, obj)| (addr, obj))
    }

    /// What the guest reads out of the keyboard device: one byte per key, the
    /// top bit set on the ones that are down.
    #[must_use]
    pub fn keyboard_state(&self) -> Vec<u8> {
        let mut state = vec![0u8; 256];
        for code in &self.keys_down {
            if let Some(byte) = state.get_mut(*code as usize) {
                *byte = 0x80;
            }
        }
        state
    }

    /// Put one thing a person did in a device's buffered lane.
    ///
    /// A lane nobody reads does not grow without end: past what a device
    /// buffers, the oldest goes, which is what a real device does when a
    /// title stops asking.
    // ponytail: a declared 1024 events a device, which is more than any
    // title asks for; a title that asks for more is the reason to read
    // DIPROP_BUFFERSIZE.
    pub fn buffer_input(&mut self, keyboard: bool, offset: u32, value: u32) {
        const MOST: usize = 1024;
        let lane = if keyboard {
            &mut self.keyboard_data
        } else {
            &mut self.mouse_data
        };
        lane.push_back((offset, value));
        while lane.len() > MOST {
            lane.pop_front();
        }
    }

    /// What the guest reads out of the mouse device: three axes and four
    /// buttons, as DirectInput lays them out. The axes are how far the mouse
    /// moved since the guest last looked, so looking takes them.
    pub fn take_mouse_state(&mut self) -> Vec<u8> {
        let mut state = vec![0u8; 16];
        let (dx, dy) = std::mem::take(&mut self.mouse_delta);
        state[..4].copy_from_slice(&dx.to_le_bytes());
        state[4..8].copy_from_slice(&dy.to_le_bytes());
        state[12] = if self.mouse_down { 0x80 } else { 0 };
        state[13] = if self.right_down { 0x80 } else { 0 };
        state
    }

    /// Make a COM object the guest can call through.
    ///
    /// The object is one word of guest memory pointing at the interface's
    /// table, which is itself guest memory full of boundary addresses.
    ///
    /// # Errors
    /// When the heap is exhausted or the write faults.
    pub fn make_com(
        ctx: &mut Ctx<'_>,
        interface: &'static crate::com::Interface,
        state: ComState,
    ) -> Result<u32, Fault> {
        let vtable = match ctx.state.vtable(interface.name) {
            Some(at) => at,
            None => {
                // The table is bound wider than the methods that are
                // written, so a call into a slot nobody declared arrives at
                // the boundary and stops with a name. A vtable that ended
                // where the written methods end would have the guest read
                // whatever is next in the heap and call it, which is a jump
                // into nothing half a million instructions later.
                let slots = interface.methods.len().max(SLOTS_PER_INTERFACE);
                let addresses = ctx.boundary.bind_interface(interface.name, slots);
                let at = ctx
                    .state
                    .heap
                    .alloc(u32::try_from(addresses.len() * 4).unwrap_or(4));
                for (i, address) in addresses.iter().enumerate() {
                    ctx.mem.write_u32(at + (i as u32) * 4, *address)?;
                }
                ctx.state.set_vtable(interface.name, at);
                at
            }
        };
        let object = ctx
            .state
            .free_com
            .pop_front()
            .unwrap_or_else(|| ctx.state.heap.alloc(8));
        ctx.mem.write_u32(object, vtable)?;
        ctx.state.add_com(object, interface.name, state);
        Ok(object)
    }

    /// Allocate a DirectDraw surface pixel buffer, reusing a same-sized freed
    /// surface buffer from `surface_pool` before bumping `self.heap` (#338).
    pub(crate) fn alloc_surface_pixels(&mut self, size: u32) -> u32 {
        let Some(rounded) = size.max(1).checked_next_multiple_of(8) else {
            return 0;
        };
        if let Some(bucket) = self.surface_pool.get_mut(&rounded) {
            if let Some(addr) = bucket.pop_front() {
                if bucket.is_empty() {
                    self.surface_pool.remove(&rounded);
                }
                return addr;
            }
        }
        self.heap.alloc(size)
    }

    /// Return a DirectDraw-owned pixel buffer to `surface_pool` for reuse by
    /// future surfaces of the same size (#338).
    pub(crate) fn free_surface_pixels(&mut self, addr: u32) {
        if let Some(size) = self.heap.size_of(addr) {
            self.surface_pool.entry(size).or_default().push_back(addr);
        }
    }

    /// Remember a COM object the machine handed out.
    pub fn add_com(&mut self, at: u32, interface: &'static str, state: ComState) {
        self.faces.insert((at, interface), at);
        self.com.insert(
            at,
            ComObject {
                interface,
                refs: 1,
                identity: at,
                app_data: 0,
                state,
            },
        );
    }

    /// Hand back an object's face as another interface: the same face again
    /// if the guest has had it before, as COM promises, and a new one sharing
    /// the object's state if not.
    ///
    /// # Errors
    /// When the heap is exhausted or the write faults.
    pub fn face(
        ctx: &mut Ctx<'_>,
        of: u32,
        interface: &'static crate::com::Interface,
    ) -> Result<u32, Fault> {
        let identity = ctx.state.identity(of);
        let existing = ctx.state.faces.get(&(identity, interface.name)).copied();
        if let Some(at) = existing {
            ctx.state.com_ref(at, 1);
            return Ok(at);
        }
        // The face keeps nothing of its own: what the object keeps is found
        // through its identity, so a change made through one face is seen
        // through every other.
        let at = Self::make_com(ctx, interface, ComState::None)?;
        if let Some(object) = ctx.state.com.get_mut(&at) {
            object.identity = identity;
        }
        ctx.state.faces.remove(&(at, interface.name));
        ctx.state.faces.insert((identity, interface.name), at);
        Ok(at)
    }

    /// The object a face belongs to.
    #[must_use]
    pub fn identity(&self, at: u32) -> u32 {
        self.com.get(&at).map_or(at, |o| o.identity)
    }

    /// The object a guest pointer is a face of, if the machine made it: its
    /// state, what the guest hung on it, and its references, whichever face
    /// the pointer is.
    #[must_use]
    pub fn com(&self, at: u32) -> Option<&ComObject> {
        let object = self.com.get(&at)?;
        if object.identity == at {
            Some(object)
        } else {
            self.com.get(&object.identity)
        }
    }

    pub fn com_mut(&mut self, at: u32) -> Option<&mut ComObject> {
        let identity = {
            let object = self.com.get_mut(&at)?;
            if object.identity == at {
                // Safety: `object` is a unique mutable borrow from `self.com`;
                // we return immediately without touching `self.com` again and
                // tie the returned lifetime to `&mut self`.
                let ptr: *mut ComObject = object;
                return Some(unsafe { &mut *ptr });
            }
            object.identity
        };
        self.com.get_mut(&identity)
    }

    /// Which interface a guest pointer is: the face itself, not the object.
    #[must_use]
    pub fn interface_of(&self, at: u32) -> Option<&'static str> {
        self.com.get(&at).map(|o| o.interface)
    }

    /// Count a reference in or out, and answer with what is left.
    ///
    /// When the last reference on all faces of a DirectDraw surface drops to
    /// zero, its pixel buffer, COM faces, and surface side-table entries are
    /// reclaimed (#338). Releasing a `Device3D`, `RetainedDevice`, or
    /// `StreamSample` also drains bound textures / retries reclamation of its
    /// guarded surface.
    pub fn com_ref(&mut self, at: u32, by: i32) -> u32 {
        let Some(object) = self.com.get_mut(&at) else {
            return 0;
        };
        object.refs = object.refs.saturating_add(by).max(0);
        let left = u32::try_from(object.refs).unwrap_or(0);
        if by < 0 && left == 0 {
            let identity = object.identity;
            match self.com.get(&identity).map(|o| &o.state) {
                Some(ComState::Surface { .. }) => self.reclaim_surface_if_unreferenced(identity),
                Some(&ComState::Device3D { surface }) if !self.has_live_refs(identity) => {
                    for tex in self.immediate.take_device_textures(identity) {
                        self.com_ref(tex, -1);
                    }
                    if surface != 0 {
                        let target = self.identity(surface);
                        self.reclaim_surface_if_unreferenced(target);
                    }
                }
                Some(&ComState::RetainedDevice { surface, .. })
                | Some(&ComState::StreamSample { surface, .. })
                    if !self.has_live_refs(identity) && surface != 0 =>
                {
                    let target = self.identity(surface);
                    self.reclaim_surface_if_unreferenced(target);
                }
                _ => {}
            }
        }
        left
    }

    /// Iterate all `(key, face_addr)` entries in `self.faces` belonging to `identity`.
    fn face_entries(&self, identity: u32) -> impl Iterator<Item = ((u32, &'static str), u32)> + '_ {
        use std::ops::Bound;
        let end = match identity.checked_add(1) {
            Some(next) => Bound::Excluded((next, "")),
            None => Bound::Unbounded,
        };
        self.faces
            .range((Bound::Included((identity, "")), end))
            .map(|(&k, &v)| (k, v))
    }

    /// Whether any face sharing `identity` still holds a live reference.
    fn has_live_refs(&self, identity: u32) -> bool {
        self.face_entries(identity)
            .any(|(_, addr)| self.com.get(&addr).is_some_and(|o| o.refs > 0))
    }

    /// Reclaim a DirectDraw surface's pixel buffer, COM faces, and side-table
    /// entries once every face of `identity` has refcount 0 and no live parent
    /// flipping chain, 3D device, or stream sample still holds it (#338).
    pub(crate) fn reclaim_surface_if_unreferenced(&mut self, identity: u32) {
        let Some(&ComState::Surface {
            pixels,
            owned_pixels,
            back,
            ..
        }) = self.com.get(&identity).map(|o| &o.state)
        else {
            return;
        };

        if self.has_live_refs(identity) {
            return;
        }

        // Guard live internal holders (flipping-chain parent, Device3D,
        // RetainedDevice, StreamSample) that store a surface pointer without
        // bumping its COM refcount.
        if self.com.iter().any(|(&addr, o)| match o.state {
            ComState::Surface { back: b, .. } => b == identity,
            ComState::Device3D { surface }
            | ComState::RetainedDevice { surface, .. }
            | ComState::StreamSample { surface, .. } => {
                surface != 0 && self.identity(surface) == identity && self.has_live_refs(addr)
            }
            _ => false,
        }) {
            return;
        }

        loop {
            let Some((key, face_addr)) = self.face_entries(identity).next() else {
                break;
            };
            self.faces.remove(&key);
            self.com.remove(&face_addr);
            self.free_com.push_back(face_addr);
        }

        self.surface_dcs.retain(|_, &mut s| s != identity);
        self.immediate.remove_handle_for(identity);

        if pixels != 0
            && (self.color_keys.contains_key(&pixels) || self.clipped.contains(&pixels))
            && !self
                .com
                .values()
                .any(|o| matches!(o.state, ComState::Surface { pixels: p, .. } if p == pixels))
        {
            self.color_keys.remove(&pixels);
            self.clipped.remove(&pixels);
        }

        if owned_pixels && pixels != 0 {
            self.free_surface_pixels(pixels);
        }

        if back != 0 {
            self.reclaim_surface_if_unreferenced(back);
        }
    }

    /// The vtable an interface's objects share, once it has one.
    #[must_use]
    pub fn vtable(&self, interface: &'static str) -> Option<u32> {
        self.vtables.get(interface).copied()
    }

    /// Remember where an interface's vtable was put.
    pub fn set_vtable(&mut self, interface: &'static str, at: u32) {
        self.vtables.insert(interface, at);
    }

    /// Give the guest a handle to a registry key, making the key if the guest
    /// is creating it.
    pub fn open_key(&mut self, path: &str) -> u32 {
        self.registry.entry(folded(path)).or_default();
        self.give(Object::Key {
            path: path.to_owned(),
        })
    }

    /// Which key a handle names.
    #[must_use]
    pub fn key_of(&self, handle: u32) -> Option<String> {
        match self.object(handle) {
            Some(Object::Key { path }) => Some(path.clone()),
            _ => None,
        }
    }

    #[must_use]
    pub fn registry_has(&self, path: &str) -> bool {
        self.registry.contains_key(&folded(path))
    }

    /// One value out of the registry, by key and name.
    #[must_use]
    pub fn registry_value(&self, path: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        self.registry
            .get(&folded(path))?
            .get(&folded(name))
            .cloned()
    }

    /// Write a value the guest gave us.
    pub fn set_registry_value(&mut self, path: &str, name: &str, kind: u32, bytes: Vec<u8>) {
        self.registry
            .entry(folded(path))
            .or_default()
            .insert(folded(name), (kind, bytes));
    }

    /// Seed a declared key, as a title's installer would have left it.
    pub fn declare_registry(&mut self, path: &str, name: &str, kind: u32, bytes: Vec<u8>) {
        self.set_registry_value(path, name, kind, bytes);
    }

    /// The handle a module already has, if it has one.
    #[must_use]
    pub fn module(&self, name: &str) -> Option<u32> {
        self.handles.iter().enumerate().find_map(|(i, o)| match o {
            Some(Object::Module { name: n, .. }) if n.eq_ignore_ascii_case(name) => {
                Some(HANDLE_BASE + (i as u32) * 4)
            }
            _ => None,
        })
    }
}

/// First handle value handed to the guest.
const HANDLE_BASE: u32 = 0x0000_0100;

/// A registry path or value name, folded the way Windows compares them.
///
/// The registry is not case-sensitive, and a declared row is a claim about
/// what a title's installer left behind rather than about how the title
/// spells it when it asks. Told apart by case, a title asking for
/// `Software\Mattel\...` would be answered "no such value" for a row that is
/// there, and would fall back to a path nobody declared (#42).
fn folded(name: &str) -> String {
    name.to_uppercase()
}

/// One window message, as a `MSG` holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub window: u32,
    pub message: u32,
    pub wparam: u32,
    pub lparam: u32,
    /// The declared clock when it was posted, in milliseconds.
    pub time: u32,
    pub point: (i32, i32),
}

/// A frame the guest finished, and where to read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Present {
    pub pixels: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub depth: u32,
}

/// How many slots every interface's table is given.
///
/// Larger than any interface on the roster, so that a call past what is
/// written lands on the boundary rather than in the heap.
pub const SLOTS_PER_INTERFACE: usize = 128;

/// One COM object the machine made.
pub struct ComObject {
    pub interface: &'static str,
    pub refs: i32,
    /// The object this is a face of: its own address for the object a call
    /// made, and that object's for every face the guest asked it for. What an
    /// object keeps on the side is kept under this, so every face sees it.
    pub identity: u32,
    /// What the guest hung on this object with `SetAppData`. Its own pointer,
    /// which it expects to read back unchanged.
    pub app_data: u32,
    /// Whatever the interface keeps about this particular object.
    pub state: ComState,
}

/// What an interface remembers about one of its objects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ComState {
    #[default]
    None,
    /// An input device, and which one.
    Device { keyboard: bool },
    /// A sound buffer, whose bytes and playing state DirectSound keeps
    /// (`dsound::Sounds`).
    Sound,
    /// A drawing surface: guest memory, and how to read it.
    Surface {
        pixels: u32,
        /// Whether DirectDraw allocated `pixels` from the guest heap (false
        /// once `SetSurfaceDesc` installs client-owned memory).
        owned_pixels: bool,
        width: u32,
        height: u32,
        pitch: u32,
        /// How a pixel is laid out: what the title asked for, or the screen's.
        format: crate::raster::PixelFormat,
        primary: bool,
        /// What `GetSurfaceDesc` says it is, its `DDSCAPS_*`: what the title
        /// asked for, and where DirectDraw put it (#210).
        caps: u32,
        /// The surface behind this one, when it flips.
        back: u32,
        /// The palette an indexed surface's pixels are looked up in.
        palette: u32,
    },
    /// A palette's own 256 entries, in guest memory.
    Palette { store: u32 },
    /// A multimedia stream: a movie player, whose state is in the players.
    MultiMediaStream,
    /// One stream of a player's movie.
    MediaStream { owner: u32, video: bool },
    /// A sample that a player's video is updated into: the surface it wraps.
    StreamSample { owner: u32, surface: u32 },
    /// One stream of an AVIFile movie: the file object, and which stream.
    AviStream { file: u32, stream: usize },
    /// A three-dimensional device, and the surface it draws into.
    Device3D { surface: u32 },
    /// A buffer of vertices the title fills and then draws from (#57): where
    /// they are, how they are laid out, and how many fit.
    VertexBuffer { store: u32, fvf: u32, count: u32 },
    /// A frame's children or its lights, as they were when a title asked for
    /// them.
    Array { items: Vec<u32> },
    /// What a viewport's `Pick` found, nearest first: each visual, the array
    /// of frames from its scene's root down to it, and its `D3DRMPICKDESC`
    /// as the five words it is written as.
    PickedArray { picks: Vec<(u32, u32, [u32; 5])> },
    /// Something a title draws itself: its callback, and what to pass it.
    UserVisual { callback: u32, argument: u32 },
    /// Retained mode's device: its surface, and how many buffers the title
    /// said it flips between.
    RetainedDevice {
        surface: u32,
        buffers: u32,
        /// The immediate-mode device underneath, once one has been asked for.
        immediate: u32,
    },
    /// A viewport: which device, where on it, and what it clears to.
    Viewport {
        device: u32,
        /// The frame it looks from, for a retained-mode viewport.
        camera: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        background: u32,
        /// The immediate-mode viewport underneath a retained-mode one, once
        /// a title has asked for it.
        immediate: u32,
    },
}

/// What a shim is given.
pub struct Ctx<'a> {
    pub cpu: &'a mut Cpu,
    pub mem: &'a mut Arena,
    pub state: &'a mut State,
    pub title: &'a Title,
    pub log: &'a mut RunLog,
    /// What the declared machine has on its disc.
    pub tree: &'a mut crate::install::Tree,
    /// The binaries the title ships beside it, which a guest may ask for more
    /// of while it runs (#55).
    pub modules: &'a mut crate::loader::Modules,
    /// What the declared clock reads, in milliseconds.
    pub clock_millis: u64,
    /// Where an interface's methods get their addresses from.
    pub boundary: &'a mut crate::shim::Boundary,
    /// Set by a shim that needs the guest's own code run before its call can
    /// finish: the function to call and what to pass it. The answer the shim
    /// returned is given to the guest once that call comes back.
    pub call_guest: Option<(u32, Vec<u32>)>,
    /// Set by a method whose answer is a floating-point value, which the
    /// caller reads from the top of the floating-point stack, not `eax`.
    pub float_answer: Option<f32>,
    /// Set alongside `call_guest` by a shim whose answer is the guest call's
    /// own: what a call this machine only arranges — a COM server the title
    /// ships, made by its own class factory — comes back with (#60).
    pub answer_is_the_call_s: bool,
}

/// Call one of the guest's own callbacks several times over, once per set of
/// arguments, stopping early if it says to and `stop` lets it.
///
/// A shim can park one call into guest code and no more (`call_guest`), which
/// is enough for an enumeration with one answer in it and not for one with
/// six. What goes into guest memory here is a short program that makes the
/// calls itself and returns to the machine at the end — the same thing the
/// module loader does to run a DllMain (#55, #57).
///
/// The callback is stdcall, so each call clears its own arguments; with
/// `stop`, a callback that answers zero has asked to stop, which Direct3D
/// spells `D3DENUMRET_CANCEL`. Without it every call is made, as a window is
/// sent each of the messages `DestroyWindow` sends whatever it answers (#140).
///
/// # Errors
/// When the program will not fit in guest memory.
pub fn call_each(
    ctx: &mut Ctx<'_>,
    callback: u32,
    calls: &[Vec<u32>],
    stop: bool,
) -> Result<(), Stop> {
    let mut program: Vec<u8> = Vec::new();
    // Every call ends up jumping to the same `ret`, whose place is only known
    // once they are all written, so the jumps are filled in afterwards.
    let mut holes = Vec::new();
    for args in calls {
        for value in args.iter().rev() {
            program.push(0x68); // push <argument>
            program.extend_from_slice(&value.to_le_bytes());
        }
        program.push(0xb8); // mov eax, <the callback>
        program.extend_from_slice(&callback.to_le_bytes());
        program.extend_from_slice(&[0xff, 0xd0]); // call eax
        if !stop {
            continue;
        }
        program.extend_from_slice(&[0x85, 0xc0]); // test eax, eax
        program.extend_from_slice(&[0x0f, 0x84]); // je <the end>
        holes.push(program.len());
        program.extend_from_slice(&[0, 0, 0, 0]);
    }
    let end = program.len();
    for hole in holes {
        let rel = u32::try_from(end - (hole + 4)).unwrap_or(0);
        program[hole..hole + 4].copy_from_slice(&rel.to_le_bytes());
    }
    program.push(0xc3); // ret, which is back into the machine

    let at = ctx
        .state
        .heap
        .alloc(u32::try_from(program.len()).unwrap_or(0));
    ctx.mem.write(at, &program).map_err(Stop::Fault)?;
    ctx.call_guest = Some((at, Vec::new()));
    Ok(())
}

/// What `GetDriveTypeA` answers: no drive there, one with removable media
/// such as a floppy drive, a fixed disk, and a CD-ROM.
const DRIVE_NO_ROOT_DIR: u32 = 1;
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
pub(crate) const DRIVE_CDROM: u32 = 5;
/// The error mode in which a title takes critical errors itself.
const SEM_FAILCRITICALERRORS: u32 = 1;
/// A drive with no disk in it: MS-DOS's critical error 2, "drive not ready",
/// counted from `ERROR_WRITE_PROTECT`, 19, as Windows counts them (#145).
const ERROR_NOT_READY: u32 = 21;

impl Ctx<'_> {
    /// Read a NUL-terminated UTF-16 string the guest passed.
    ///
    /// # Errors
    /// When the address is outside the arena.
    pub fn wide_string(&self, addr: u32) -> Result<String, Fault> {
        let mut units = Vec::new();
        let mut at = addr;
        while addr != 0 && units.len() < 0x1_0000 {
            let unit = u16::from_le_bytes(self.mem.slice(at, 2)?.try_into().unwrap_or([0, 0]));
            if unit == 0 {
                break;
            }
            units.push(unit);
            at += 2;
        }
        Ok(String::from_utf16_lossy(&units))
    }

    /// Read a NUL-terminated string the guest passed.
    ///
    /// # Errors
    /// When the address is outside the arena.
    pub fn string(&self, addr: u32) -> Result<String, Fault> {
        if addr == 0 {
            return Ok(String::new());
        }
        let mut out = Vec::new();
        let mut at = addr;
        loop {
            let byte = self.mem.slice(at, 1)?[0];
            if byte == 0 {
                break;
            }
            out.push(byte);
            at += 1;
            if out.len() > 0x1_0000 {
                break;
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Write a NUL-terminated string into guest memory, truncated to fit.
    ///
    /// Returns how many bytes were written, not counting the terminator, which
    /// is what the Win32 calls that do this report.
    ///
    /// # Errors
    /// When the buffer is outside the arena.
    pub fn write_string(&mut self, addr: u32, len: u32, text: &str) -> Result<u32, Fault> {
        if addr == 0 || len == 0 {
            return Ok(0);
        }
        let bytes = text.as_bytes();
        let room = (len - 1) as usize;
        let take = bytes.len().min(room);
        self.mem.write(addr, &bytes[..take])?;
        self.mem.write(addr + take as u32, &[0])?;
        Ok(u32::try_from(take).unwrap_or(0))
    }

    /// Open a file the guest named, through the declared install tree.
    ///
    /// The tree says which of the pressing's own files appear where on the
    /// declared machine's disc. Nothing outside it is reachable: the guest
    /// cannot open the host's files, because it is not on the host.
    pub fn open_guest_file(&mut self, path: &str) -> Option<Vec<u8>> {
        self.tree.read(&self.absolute(path))
    }

    /// Whether the tree holds a file the guest named, without recording a
    /// miss: what a title about to make one asks (#44).
    #[must_use]
    pub fn guest_file_is_there(&self, path: &str) -> bool {
        self.tree.holds(&self.absolute(path))
    }

    /// The declared machine's drives, each letter with what `GetDriveTypeA`
    /// answers for it: a floppy drive when the title declares one, the fixed
    /// disk the title is installed on, and the CD-ROM its pressing is in. The
    /// one place they are listed (#145).
    #[must_use]
    pub(crate) fn drives(&self) -> &'static [(u8, u32)] {
        if self.title.declared.floppy {
            &[
                (b'A', DRIVE_REMOVABLE),
                (b'C', DRIVE_FIXED),
                (b'D', DRIVE_CDROM),
            ]
        } else {
            &[(b'C', DRIVE_FIXED), (b'D', DRIVE_CDROM)]
        }
    }

    /// What `GetDriveTypeA` answers for a drive letter: `DRIVE_NO_ROOT_DIR`
    /// for one the machine does not have.
    #[must_use]
    pub(crate) fn drive_type(&self, letter: u8) -> u32 {
        self.drives()
            .iter()
            .find(|(drive, _)| drive.eq_ignore_ascii_case(&letter))
            .map_or(DRIVE_NO_ROOT_DIR, |&(_, kind)| kind)
    }

    /// Refuse a call that looks on the machine's floppy drive, which has no
    /// disk in it, as Windows does; whether it did.
    ///
    /// Windows shows a box asking for a disk unless the title has said it
    /// takes critical errors itself, and then the call fails as the drive
    /// not being ready (`GetVolumeInformationA`'s remarks). This machine has
    /// no such box, so a title that has not said so stops at a wall.
    ///
    /// # Errors
    /// That wall.
    pub(crate) fn refuse_on_empty_floppy(&mut self, path: &str) -> Result<bool, Stop> {
        let path = self.absolute(path);
        let drive = path.as_bytes();
        if drive.get(1) != Some(&b':') || self.drive_type(drive[0]) != DRIVE_REMOVABLE {
            return Ok(false);
        }
        if self.state.error_mode & SEM_FAILCRITICALERRORS == 0 {
            return Err(Stop::NotWritten {
                what: format!("Windows' box asking for a disk in A:, for {path}"),
                caller: 0,
            });
        }
        self.state.last_error = ERROR_NOT_READY;
        Ok(true)
    }

    /// A path the guest gave us, resolved the way the guest means it, which
    /// is the way `GetFullPathNameA` resolves it: a relative one is against
    /// wherever the guest believes it is, and one rooted without a drive, by
    /// either separator, is on that place's drive.
    #[must_use]
    pub fn absolute(&self, path: &str) -> String {
        let path = path.trim();
        let here = &self.state.current_directory;
        if path.len() > 1 && path.as_bytes().get(1) == Some(&b':') {
            path.to_owned()
        } else if path.starts_with(['\\', '/']) {
            let drive = if here.as_bytes().get(1) == Some(&b':') {
                &here[..2]
            } else {
                ""
            };
            format!("{drive}{path}")
        } else {
            // A drive's root ends in its separator already.
            format!("{}\\{path}", here.strip_suffix('\\').unwrap_or(here))
        }
    }

    /// The declared clock, in milliseconds, as the guest reads it.
    #[must_use]
    pub fn declared_millis(&self) -> u64 {
        self.clock_millis
    }

    /// Put a string somewhere the guest can keep reading it.
    ///
    /// # Errors
    /// When the heap is exhausted or the write faults.
    pub fn intern(&mut self, text: &str) -> Result<u32, Fault> {
        let bytes = text.as_bytes();
        let addr = self
            .state
            .heap
            .alloc(u32::try_from(bytes.len() + 1).unwrap_or(1));
        if addr == 0 {
            return Ok(0);
        }
        self.mem.write(addr, bytes)?;
        self.mem.write(addr + bytes.len() as u32, &[0])?;
        Ok(addr)
    }
}

/// A colour component given as a fraction, as a byte: nothing below zero,
/// everything from one up full, and truncated between, as Direct3D does.
#[must_use]
pub fn component(c: f32) -> u32 {
    if c <= 0.0 {
        0
    } else if c >= 1.0 {
        255
    } else {
        (c * 255.0).floor() as u32
    }
}

/// A `D3DCOLOR` as a screen of this depth stores it: the declared card's 5-6-5
/// at sixteen bits, and the colour's own bytes otherwise.
#[must_use]
pub fn pixel(colour: u32, depth: u32) -> u32 {
    let [b, g, r, a] = colour.to_le_bytes();
    if depth == 16 {
        crate::raster::PixelFormat::RGB565.encode([r, g, b, a])
    } else {
        colour
    }
}

/// Fill one contiguous slice of `bytes`-wide pixels with `value` in little-endian.
fn fill_row(dst: &mut [u8], bytes: usize, value: u32) {
    match bytes {
        1 => dst.fill(value as u8),
        2 => {
            let p = (value as u16).to_le_bytes();
            if p[0] == p[1] {
                dst.fill(p[0]);
            } else {
                for chunk in dst.as_chunks_mut::<2>().0 {
                    *chunk = p;
                }
            }
        }
        3 => {
            let [b0, b1, b2, _] = value.to_le_bytes();
            if b0 == b1 && b1 == b2 {
                dst.fill(b0);
            } else {
                for chunk in dst.as_chunks_mut::<3>().0 {
                    *chunk = [b0, b1, b2];
                }
            }
        }
        4 => {
            let p = value.to_le_bytes();
            if p[0] == p[1] && p[1] == p[2] && p[2] == p[3] {
                dst.fill(p[0]);
            } else {
                for chunk in dst.as_chunks_mut::<4>().0 {
                    *chunk = p;
                }
            }
        }
        0 => {}
        _ => {
            let one = value.to_le_bytes();
            let take = bytes.min(one.len());
            for chunk in dst.chunks_exact_mut(bytes) {
                chunk[..take].copy_from_slice(&one[..take]);
            }
        }
    }
}

/// Fill a rectangle of a surface, `(pixels, pitch, depth)`, with one pixel
/// already in the surface's own format.
///
/// # Errors
/// When the rectangle reaches outside the arena.
pub fn fill(
    ctx: &mut Ctx<'_>,
    (pixels, pitch, depth): (u32, u32, u32),
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    value: u32,
) -> Result<(), Stop> {
    let rows = y1.saturating_sub(y0) as usize;
    if rows == 0 {
        return Ok(());
    }
    let bytes = depth.div_ceil(8) as usize;
    let cols = x1.saturating_sub(x0) as usize;
    let row_bytes = cols * bytes;
    if row_bytes == 0 {
        for y in y0..y1 {
            ctx.mem
                .slice_mut(pixels + y * pitch + x0 * bytes as u32, 0)
                .map_err(Stop::Fault)?;
        }
        return Ok(());
    }
    let pitch_usize = pitch as usize;
    let start =
        u64::from(pixels) + u64::from(y0) * pitch_usize as u64 + u64::from(x0) * bytes as u64;
    let span = (rows - 1) * pitch_usize + row_bytes;
    let start_addr = u32::try_from(start).map_err(|_| {
        Stop::Fault(Fault {
            addr: u32::MAX,
            len: span,
            write: true,
        })
    })?;
    let buf = ctx.mem.slice_mut(start_addr, span).map_err(Stop::Fault)?;
    if pitch_usize == row_bytes {
        fill_row(buf, bytes, value);
    } else {
        fill_row(&mut buf[..row_bytes], bytes, value);
        for row in 1..rows {
            let d = row * pitch_usize;
            if pitch_usize >= row_bytes {
                buf.copy_within(..row_bytes, d);
            } else {
                fill_row(&mut buf[d..d + row_bytes], bytes, value);
            }
        }
    }
    Ok(())
}

/// Every COM interface slice declared across the Win32 modules.
const INTERFACE_SETS: &[&[&crate::com::Interface]] = &[
    dinput::INTERFACES,
    dsound::INTERFACES,
    ddraw::INTERFACES,
    d3drm::INTERFACES,
    d3d::INTERFACES,
    amstream::INTERFACES,
    avifile::INTERFACES,
];

/// The interfaces this machine can hand the guest.
#[must_use]
pub fn interfaces() -> Vec<&'static crate::com::Interface> {
    let mut all = Vec::new();
    for set in INTERFACE_SETS {
        all.extend_from_slice(set);
    }
    all
}

/// One interface, by the name it is declared under.
#[must_use]
pub fn interface_named(name: &str) -> Option<&'static crate::com::Interface> {
    INTERFACE_SETS
        .iter()
        .copied()
        .flatten()
        .copied()
        .find(|i| i.name == name)
}

/// One method of one interface, if it is declared.
#[must_use]
pub fn com_method(interface: &str, slot: usize) -> Option<&'static crate::com::Method> {
    interface_named(interface)?.method(slot)
}

/// Find the shim behind an import, if one is written.
#[must_use]
pub fn find(dll: &str, name: &str) -> Option<&'static Shim> {
    written(dll).find(|s| s.name == name)
}

/// Whether core writes an entry point of a library: what makes a library
/// `LoadLibraryA` asks for, and the title does not ship, one of Microsoft's
/// (#146). ADR-0002 takes any library an import names that the tree does not
/// hold to be Microsoft's, as `Modules::load` does, since a name nothing is
/// written for is still a wall when it is called; a library asked for at run
/// time is the title asking whether the machine has it, and `LoadLibraryA`
/// answers by this and by what the title declares absent.
#[must_use]
pub fn supplies(dll: &str) -> bool {
    written(dll).next().is_some()
}

/// Every entry point written for one library, named as an import names it,
/// with its extension or without, whatever their case.
fn written(dll: &str) -> impl Iterator<Item = &'static Shim> + '_ {
    // Windows compares library names whatever their case, the extension's too.
    let at = dll.len().saturating_sub(4);
    let bare = match dll.get(at..) {
        Some(ext) if ext.eq_ignore_ascii_case(".dll") => &dll[..at],
        _ => dll,
    };
    [
        kernel32::SHIMS,
        user32::SHIMS,
        msvcrt::SHIMS,
        advapi32::SHIMS,
        dinput::SHIMS,
        dsound::SHIMS,
        ddraw::SHIMS,
        gdi32::SHIMS,
        d3drm::SHIMS,
        avifile::SHIMS,
        msacm32::SHIMS,
        ole32::SHIMS,
        version::SHIMS,
        winmm::SHIMS,
    ]
    .into_iter()
    .flatten()
    .filter(move |s| s.dll.eq_ignore_ascii_case(bare))
}

fn resolve_named_kind(s: &str) -> Option<&'static str> {
    match s {
        "mutex" => Some("mutex"),
        "event" => Some("event"),
        "thread" => Some("thread"),
        _ => None,
    }
}

fn resolve_setting_name(s: &str) -> Option<&'static str> {
    match s {
        "texture colours" => Some("texture colours"),
        "texture shades" => Some("texture shades"),
        "dither" => Some("dither"),
        "shades" => Some("shades"),
        "quality" => Some("quality"),
        "texture quality" => Some("texture quality"),
        "render mode" => Some("render mode"),
        _ => None,
    }
}

fn resolve_interface_name(s: &str) -> Option<&'static str> {
    interface_named(s).map(|i| i.name)
}

impl crate::snap::Snap for Contents {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let mut tag = match self {
            Self::Held(_) => 0u8,
            Self::Lies { .. } => 1u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => {
                    let mut bytes = Vec::new();
                    v.byte_vec(&mut bytes);
                    *self = Self::Held(bytes);
                }
                1 => {
                    let (mut start, mut len) = (0u64, 0u32);
                    v.u64(&mut start);
                    v.u32(&mut len);
                    *self = Self::Lies { start, len };
                }
                _ => v.mark_corrupt("win32::Contents tag"),
            }
        } else {
            match self {
                Self::Held(bytes) => v.byte_vec(bytes),
                Self::Lies { start, len } => {
                    v.u64(start);
                    v.u32(len);
                }
            }
        }
    }
}

impl crate::snap::Snap for Object {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let mut tag = match self {
            Self::Module { .. } => 0u8,
            Self::Heap => 1u8,
            Self::Std(_) => 2u8,
            Self::File { .. } => 3u8,
            Self::Named { .. } => 4u8,
            Self::Semaphore { .. } => 5u8,
            Self::Hook(_) => 6u8,
            Self::Find { .. } => 7u8,
            Self::Mapping { .. } => 8u8,
            Self::Key { .. } => 9u8,
            Self::MidiStream => 10u8,
            Self::Mmio { .. } => 11u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => {
                    let (mut name, mut base) = (String::new(), 0u32);
                    v.string(&mut name);
                    v.u32(&mut base);
                    *self = Self::Module { name, base };
                }
                1 => *self = Self::Heap,
                2 => {
                    let mut n = 0u32;
                    v.u32(&mut n);
                    *self = Self::Std(n);
                }
                3 => {
                    let mut path = String::new();
                    let mut contents = Contents::Held(Vec::new());
                    let (mut at, mut access, mut share, mut wrote) = (0u32, 0u32, 0u32, false);
                    v.string(&mut path);
                    contents.snap(v);
                    v.u32(&mut at);
                    v.u32(&mut access);
                    v.u32(&mut share);
                    v.bool(&mut wrote);
                    *self = Self::File {
                        path,
                        contents,
                        at,
                        access,
                        share,
                        wrote,
                    };
                }
                4 => {
                    let mut kind = "";
                    crate::snap::snap_static_str(
                        v,
                        &mut kind,
                        "Object::Named kind",
                        resolve_named_kind,
                    );
                    let mut name = String::new();
                    let (mut signalled, mut manual) = (false, false);
                    let mut held: Option<(u32, u32)> = None;
                    v.string(&mut name);
                    v.bool(&mut signalled);
                    v.bool(&mut manual);
                    crate::snap::snap_option(v, &mut held, |(a, b), v| {
                        v.u32(a);
                        v.u32(b);
                    });
                    *self = Self::Named {
                        kind,
                        name,
                        signalled,
                        manual,
                        held,
                    };
                }
                5 => {
                    let (mut count, mut max) = (0u32, 0u32);
                    v.u32(&mut count);
                    v.u32(&mut max);
                    *self = Self::Semaphore { count, max };
                }
                6 => {
                    let mut h = 0u32;
                    v.u32(&mut h);
                    *self = Self::Hook(h);
                }
                7 => {
                    let mut matches = Vec::new();
                    let mut at = 0usize;
                    crate::snap::snap_vec(v, &mut matches, |(n, sz), v| {
                        v.string(n);
                        v.u32(sz);
                    });
                    v.usize(&mut at);
                    *self = Self::Find { matches, at };
                }
                8 => {
                    let (mut at, mut len) = (0u32, 0u32);
                    v.u32(&mut at);
                    v.u32(&mut len);
                    *self = Self::Mapping { at, len };
                }
                9 => {
                    let mut path = String::new();
                    v.string(&mut path);
                    *self = Self::Key { path };
                }
                10 => *self = Self::MidiStream,
                11 => {
                    let (mut base, mut len, mut at, mut owned) = (0u32, 0u32, 0u32, false);
                    v.u32(&mut base);
                    v.u32(&mut len);
                    v.u32(&mut at);
                    v.bool(&mut owned);
                    *self = Self::Mmio {
                        base,
                        len,
                        at,
                        owned,
                    };
                }
                _ => v.mark_corrupt("win32::Object tag"),
            }
        } else {
            match self {
                Self::Module { name, base } => {
                    v.string(name);
                    v.u32(base);
                }
                Self::Heap | Self::MidiStream => {}
                Self::Std(n) | Self::Hook(n) => v.u32(n),
                Self::File {
                    path,
                    contents,
                    at,
                    access,
                    share,
                    wrote,
                } => {
                    v.string(path);
                    contents.snap(v);
                    v.u32(at);
                    v.u32(access);
                    v.u32(share);
                    v.bool(wrote);
                }
                Self::Named {
                    kind,
                    name,
                    signalled,
                    manual,
                    held,
                } => {
                    crate::snap::snap_static_str(v, kind, "Object::Named kind", resolve_named_kind);
                    v.string(name);
                    v.bool(signalled);
                    v.bool(manual);
                    crate::snap::snap_option(v, held, |(a, b), v| {
                        v.u32(a);
                        v.u32(b);
                    });
                }
                Self::Semaphore { count, max } => {
                    v.u32(count);
                    v.u32(max);
                }
                Self::Find { matches, at } => {
                    crate::snap::snap_vec(v, matches, |(n, sz), v| {
                        v.string(n);
                        v.u32(sz);
                    });
                    v.usize(at);
                }
                Self::Mapping { at, len } => {
                    v.u32(at);
                    v.u32(len);
                }
                Self::Key { path } => v.string(path),
                Self::Mmio {
                    base,
                    len,
                    at,
                    owned,
                } => {
                    v.u32(base);
                    v.u32(len);
                    v.u32(at);
                    v.bool(owned);
                }
            }
        }
    }
}

impl crate::snap::Snap for GuestThread {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            cpu,
            handle,
            id,
            suspended,
            ended,
            code,
            tls,
            wake_at,
            gives_up_at,
            waiting_on,
            time_critical,
        } = self;
        cpu.snap(v);
        v.u32(handle);
        v.u32(id);
        v.bool(suspended);
        v.bool(ended);
        v.u32(code);
        crate::snap::snap_u32_vec(v, tls);
        v.u64(wake_at);
        v.u64(gives_up_at);
        crate::snap::snap_u32_vec(v, waiting_on);
        v.bool(time_critical);
    }
}

impl crate::snap::Snap for Message {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            window,
            message,
            wparam,
            lparam,
            time,
            point,
        } = self;
        v.u32(window);
        v.u32(message);
        v.u32(wparam);
        v.u32(lparam);
        v.u32(time);
        v.i32(&mut point.0);
        v.i32(&mut point.1);
    }
}

impl crate::snap::Snap for Present {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            pixels,
            width,
            height,
            pitch,
            depth,
        } = self;
        v.u32(pixels);
        v.u32(width);
        v.u32(height);
        v.u32(pitch);
        v.u32(depth);
    }
}

impl crate::snap::Snap for ComState {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let mut tag = match self {
            Self::None => 0u8,
            Self::Device { .. } => 1u8,
            Self::Sound => 2u8,
            Self::Surface { .. } => 3u8,
            Self::Palette { .. } => 4u8,
            Self::MultiMediaStream => 5u8,
            Self::MediaStream { .. } => 6u8,
            Self::StreamSample { .. } => 7u8,
            Self::AviStream { .. } => 8u8,
            Self::Device3D { .. } => 9u8,
            Self::VertexBuffer { .. } => 10u8,
            Self::Array { .. } => 11u8,
            Self::PickedArray { .. } => 12u8,
            Self::UserVisual { .. } => 13u8,
            Self::RetainedDevice { .. } => 14u8,
            Self::Viewport { .. } => 15u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => *self = Self::None,
                1 => {
                    let mut keyboard = false;
                    v.bool(&mut keyboard);
                    *self = Self::Device { keyboard };
                }
                2 => *self = Self::Sound,
                3 => {
                    let (mut pixels, mut width, mut height, mut pitch) = (0u32, 0u32, 0u32, 0u32);
                    let mut owned_pixels = false;
                    let mut format = crate::raster::PixelFormat::RGB565;
                    let (mut primary, mut caps, mut back, mut palette) = (false, 0u32, 0u32, 0u32);
                    v.u32(&mut pixels);
                    v.bool(&mut owned_pixels);
                    v.u32(&mut width);
                    v.u32(&mut height);
                    v.u32(&mut pitch);
                    format.snap(v);
                    v.bool(&mut primary);
                    v.u32(&mut caps);
                    v.u32(&mut back);
                    v.u32(&mut palette);
                    *self = Self::Surface {
                        pixels,
                        owned_pixels,
                        width,
                        height,
                        pitch,
                        format,
                        primary,
                        caps,
                        back,
                        palette,
                    };
                }
                4 => {
                    let mut store = 0u32;
                    v.u32(&mut store);
                    *self = Self::Palette { store };
                }
                5 => *self = Self::MultiMediaStream,
                6 => {
                    let (mut owner, mut video) = (0u32, false);
                    v.u32(&mut owner);
                    v.bool(&mut video);
                    *self = Self::MediaStream { owner, video };
                }
                7 => {
                    let (mut owner, mut surface) = (0u32, 0u32);
                    v.u32(&mut owner);
                    v.u32(&mut surface);
                    *self = Self::StreamSample { owner, surface };
                }
                8 => {
                    let (mut file, mut stream) = (0u32, 0usize);
                    v.u32(&mut file);
                    v.usize(&mut stream);
                    *self = Self::AviStream { file, stream };
                }
                9 => {
                    let mut surface = 0u32;
                    v.u32(&mut surface);
                    *self = Self::Device3D { surface };
                }
                10 => {
                    let (mut store, mut fvf, mut count) = (0u32, 0u32, 0u32);
                    v.u32(&mut store);
                    v.u32(&mut fvf);
                    v.u32(&mut count);
                    *self = Self::VertexBuffer { store, fvf, count };
                }
                11 => {
                    let mut items = Vec::new();
                    crate::snap::snap_u32_vec(v, &mut items);
                    *self = Self::Array { items };
                }
                12 => {
                    let mut picks = Vec::new();
                    crate::snap::snap_vec(v, &mut picks, |(a, b, desc), v| {
                        v.u32(a);
                        v.u32(b);
                        for w in desc {
                            v.u32(w);
                        }
                    });
                    *self = Self::PickedArray { picks };
                }
                13 => {
                    let (mut callback, mut argument) = (0u32, 0u32);
                    v.u32(&mut callback);
                    v.u32(&mut argument);
                    *self = Self::UserVisual { callback, argument };
                }
                14 => {
                    let (mut surface, mut buffers, mut immediate) = (0u32, 0u32, 0u32);
                    v.u32(&mut surface);
                    v.u32(&mut buffers);
                    v.u32(&mut immediate);
                    *self = Self::RetainedDevice {
                        surface,
                        buffers,
                        immediate,
                    };
                }
                15 => {
                    let (
                        mut device,
                        mut camera,
                        mut x,
                        mut y,
                        mut width,
                        mut height,
                        mut background,
                        mut immediate,
                    ) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
                    v.u32(&mut device);
                    v.u32(&mut camera);
                    v.u32(&mut x);
                    v.u32(&mut y);
                    v.u32(&mut width);
                    v.u32(&mut height);
                    v.u32(&mut background);
                    v.u32(&mut immediate);
                    *self = Self::Viewport {
                        device,
                        camera,
                        x,
                        y,
                        width,
                        height,
                        background,
                        immediate,
                    };
                }
                _ => v.mark_corrupt("win32::ComState tag"),
            }
        } else {
            match self {
                Self::None | Self::Sound | Self::MultiMediaStream => {}
                Self::Device { keyboard } => v.bool(keyboard),
                Self::Surface {
                    pixels,
                    owned_pixels,
                    width,
                    height,
                    pitch,
                    format,
                    primary,
                    caps,
                    back,
                    palette,
                } => {
                    v.u32(pixels);
                    v.bool(owned_pixels);
                    v.u32(width);
                    v.u32(height);
                    v.u32(pitch);
                    format.snap(v);
                    v.bool(primary);
                    v.u32(caps);
                    v.u32(back);
                    v.u32(palette);
                }
                Self::Palette { store } => v.u32(store),
                Self::MediaStream { owner, video } => {
                    v.u32(owner);
                    v.bool(video);
                }
                Self::StreamSample { owner, surface } => {
                    v.u32(owner);
                    v.u32(surface);
                }
                Self::AviStream { file, stream } => {
                    v.u32(file);
                    v.usize(stream);
                }
                Self::Device3D { surface } => v.u32(surface),
                Self::VertexBuffer { store, fvf, count } => {
                    v.u32(store);
                    v.u32(fvf);
                    v.u32(count);
                }
                Self::Array { items } => crate::snap::snap_u32_vec(v, items),
                Self::PickedArray { picks } => {
                    crate::snap::snap_vec(v, picks, |(a, b, desc), v| {
                        v.u32(a);
                        v.u32(b);
                        for w in desc {
                            v.u32(w);
                        }
                    });
                }
                Self::UserVisual { callback, argument } => {
                    v.u32(callback);
                    v.u32(argument);
                }
                Self::RetainedDevice {
                    surface,
                    buffers,
                    immediate,
                } => {
                    v.u32(surface);
                    v.u32(buffers);
                    v.u32(immediate);
                }
                Self::Viewport {
                    device,
                    camera,
                    x,
                    y,
                    width,
                    height,
                    background,
                    immediate,
                } => {
                    v.u32(device);
                    v.u32(camera);
                    v.u32(x);
                    v.u32(y);
                    v.u32(width);
                    v.u32(height);
                    v.u32(background);
                    v.u32(immediate);
                }
            }
        }
    }
}

impl crate::snap::Snap for ComObject {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let Self {
            interface,
            refs,
            identity,
            app_data,
            state,
        } = self;
        crate::snap::snap_static_str(v, interface, "ComObject interface", resolve_interface_name);
        v.i32(refs);
        v.u32(identity);
        v.u32(app_data);
        state.snap(v);
    }
}

impl crate::snap::Snap for State {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        // Exhaustively destructure every field of `State`: adding a field later
        // is a compile error rather than silent snapshot corruption (#313).
        let Self {
            foreign_frees,
            saves_refused,
            heap,
            handles,
            command_line,
            guest_args,
            environment,
            last_error,
            threads,
            thread,
            next_thread_id,
            tls_slots,
            sections,
            section_waiters,
            claimed,
            reserved,
            committed,
            next_mapping,
            mapped_files,
            error_mode,
            current_directory,
            volume_label,
            waited_millis,
            yielded,
            installed_files,
            window_procedure,
            window_made,
            quit,
            window_thread,
            window_style,
            window_instance,
            window_extra,
            pointer,
            last_move,
            display_mode,
            color_keys,
            clipped,
            clip_lists,
            surface_dcs,
            drawdibs,
            next_drawdib,
            acm_streams,
            next_acm_stream,
            palette,
            present,
            mixer,
            sound,
            players,
            avi_files,
            movie_bytes: _,
            frame_getters,
            cd,
            midi,
            mm_timers,
            next_mm_timer,
            mm_timer_thread,
            keys_down,
            keys_pressed,
            keys_read,
            mouse_delta,
            mouse_data,
            keyboard_data,
            input_sequence,
            mouse_down,
            right_down,
            last_click,
            last_right_click,
            double_clicks,
            clip,
            screen_saver_running,
            paint_erase,
            pointer_put,
            cursor_shown,
            cursor,
            cursors,
            messages,
            timers,
            dialog,
            names,
            set_values,
            immediate,
            scene,
            render,
            com,
            faces,
            surface_pool,
            free_com,
            vtables,
            registry,
        } = self;

        v.u64(foreign_frees);
        v.u64(saves_refused);
        heap.snap(v);
        crate::snap::snap_vec(v, handles, |slot, v| {
            crate::snap::snap_option_with(v, slot, || Object::Heap, |obj, v| obj.snap(v));
        });
        crate::snap::snap_option(v, command_line, |c, v| v.u32(c));
        v.string(guest_args);
        crate::snap::snap_btree_map(
            v,
            environment,
            |k, v| v.string(k),
            |val, v| crate::snap::snap_option(v, val, |s, v| v.string(s)),
        );
        v.u32(last_error);
        crate::snap::snap_vec_with(
            v,
            threads,
            || GuestThread {
                cpu: Cpu::default(),
                handle: 0,
                id: 0,
                suspended: false,
                ended: false,
                code: 0,
                tls: Vec::new(),
                wake_at: 0,
                gives_up_at: 0,
                waiting_on: Vec::new(),
                time_critical: false,
            },
            |t, v| t.snap(v),
        );
        v.usize(thread);
        v.u32(next_thread_id);
        v.usize(tls_slots);
        crate::snap::snap_vec(v, sections, |(a, b, c), v| {
            v.u32(a);
            v.u32(b);
            v.u32(c);
        });
        v.u32(section_waiters);
        crate::snap::snap_vec(v, claimed, |(a, b), v| {
            v.u32(a);
            v.u32(b);
        });
        crate::snap::snap_btree_map(
            v,
            reserved,
            |k, v| v.u32(k),
            |(a, b), v| {
                v.u32(a);
                v.u32(b);
            },
        );
        crate::snap::snap_btree_map(v, committed, |k, v| v.u32(k), |p, v| v.u32(p));
        v.u32(next_mapping);
        crate::snap::snap_vec(v, mapped_files, |(at, len, path), v| {
            v.u32(at);
            v.u32(len);
            v.string(path);
        });
        v.u32(error_mode);
        v.string(current_directory);
        v.string(volume_label);
        v.u64(waited_millis);
        v.bool(yielded);
        v.usize(installed_files);
        v.u32(window_procedure);
        v.bool(window_made);
        crate::snap::snap_option(v, quit, |(t, c), v| {
            v.u32(t);
            v.u32(c);
        });
        v.u32(window_thread);
        v.u32(window_style);
        v.u32(window_instance);
        v.byte_vec(window_extra);
        v.i32(&mut pointer.0);
        v.i32(&mut pointer.1);
        v.i32(&mut last_move.0);
        v.i32(&mut last_move.1);
        v.u32(&mut display_mode.0);
        v.u32(&mut display_mode.1);
        v.u32(&mut display_mode.2);
        crate::snap::snap_btree_map(v, color_keys, |k, v| v.u32(k), |c, v| v.u32(c));
        crate::snap::snap_btree_set(v, clipped, |k, v| v.u32(k));
        crate::snap::snap_btree_map(
            v,
            clip_lists,
            |k, v| v.u32(k),
            |(a, b, c, d), v| {
                v.i32(a);
                v.i32(b);
                v.i32(c);
                v.i32(d);
            },
        );
        crate::snap::snap_btree_map(v, surface_dcs, |k, v| v.u32(k), |s, v| v.u32(s));
        crate::snap::snap_btree_set(v, drawdibs, |k, v| v.u32(k));
        v.u32(next_drawdib);
        crate::snap::snap_btree_map_with(
            v,
            acm_streams,
            u32::default,
            msacm32::Stream::empty_for_snap,
            |k, v| v.u32(k),
            |s, v| s.snap(v),
        );
        v.u32(next_acm_stream);
        v.u32(palette);
        crate::snap::snap_option_with(
            v,
            present,
            || Present {
                pixels: 0,
                width: 0,
                height: 0,
                pitch: 0,
                depth: 0,
            },
            |p, v| p.snap(v),
        );
        mixer.snap(v);
        sound.snap(v);
        crate::snap::snap_btree_map(v, players, |k, v| v.u32(k), |p, v| p.snap(v));
        crate::snap::snap_btree_map_with(
            v,
            avi_files,
            u32::default,
            avifile::File::empty_for_snap,
            |k, v| v.u32(k),
            |f, v| f.snap(v),
        );
        crate::snap::snap_btree_map_with(
            v,
            frame_getters,
            u32::default,
            avifile::Getter::empty_for_snap,
            |k, v| v.u32(k),
            |g, v| g.snap(v),
        );
        cd.snap(v);
        crate::snap::snap_option_with(v, midi, winmm::Midi::empty_for_snap, |m, v| m.snap(v));
        crate::snap::snap_vec(v, mm_timers, |t, v| t.snap(v));
        v.u32(next_mm_timer);
        crate::snap::snap_option(v, mm_timer_thread, |t, v| v.usize(t));
        if v.is_reading() && !v.failed() && mm_timer_thread.is_some_and(|idx| idx >= threads.len())
        {
            v.mark_corrupt("mm_timer_thread");
        }
        crate::snap::snap_u32_vec(v, keys_down);
        crate::snap::snap_u32_vec(v, keys_pressed);
        crate::snap::snap_u32_vec(v, keys_read);
        v.i32(&mut mouse_delta.0);
        v.i32(&mut mouse_delta.1);
        crate::snap::snap_vec_deque(v, mouse_data, |(a, b), v| {
            v.u32(a);
            v.u32(b);
        });
        crate::snap::snap_vec_deque(v, keyboard_data, |(a, b), v| {
            v.u32(a);
            v.u32(b);
        });
        v.u32(input_sequence);
        v.bool(mouse_down);
        v.bool(right_down);
        v.u32(&mut last_click.0);
        v.i32(&mut last_click.1 .0);
        v.i32(&mut last_click.1 .1);
        v.u32(&mut last_right_click.0);
        v.i32(&mut last_right_click.1 .0);
        v.i32(&mut last_right_click.1 .1);
        v.bool(double_clicks);
        crate::snap::snap_option(v, clip, |rect, v| {
            for r in rect {
                v.i32(r);
            }
        });
        v.bool(screen_saver_running);
        crate::snap::snap_option(v, paint_erase, |b, v| v.bool(b));
        crate::snap::snap_option(v, pointer_put, |(x, y), v| {
            v.i32(x);
            v.i32(y);
        });
        v.i32(cursor_shown);
        crate::snap::snap_option(v, cursor, |c, v| v.u32(c));
        crate::snap::snap_vec_with(
            v,
            cursors,
            || (0u32, 0u32, crate::host::Cursor::none()),
            |(m, n, cur), v| {
                v.u32(m);
                v.u32(n);
                cur.snap(v);
            },
        );
        crate::snap::snap_vec_deque_with(
            v,
            messages,
            || Message {
                window: 0,
                message: 0,
                wparam: 0,
                lparam: 0,
                time: 0,
                point: (0, 0),
            },
            |m, v| m.snap(v),
        );
        crate::snap::snap_vec_with(
            v,
            timers,
            || user32::Timer {
                window: 0,
                id: 0,
                period: 0,
                due: 0,
                callback: 0,
            },
            |t, v| t.snap(v),
        );
        crate::snap::snap_option_with(v, dialog, user32::Dialog::empty_for_snap, |d, v| d.snap(v));
        crate::snap::snap_btree_map(v, names, |k, v| v.u32(k), |s, v| v.string(s));
        crate::snap::snap_btree_map_with(
            v,
            set_values,
            || (0u32, ""),
            u32::default,
            |(id, name), v| {
                v.u32(id);
                crate::snap::snap_static_str(v, name, "set_values setting", resolve_setting_name);
            },
            |val, v| v.u32(val),
        );
        immediate.snap(v);
        scene.snap(v);
        crate::snap::snap_option_with(v, render, d3drm::RenderPass::empty_for_snap, |r, v| {
            r.snap(v)
        });
        crate::snap::snap_btree_map_with(
            v,
            com,
            u32::default,
            || ComObject {
                interface: "",
                refs: 0,
                identity: 0,
                app_data: 0,
                state: ComState::None,
            },
            |k, v| v.u32(k),
            |obj, v| obj.snap(v),
        );
        crate::snap::snap_btree_map_with(
            v,
            faces,
            || (0u32, ""),
            u32::default,
            |(id, iface), v| {
                v.u32(id);
                crate::snap::snap_static_str(v, iface, "faces interface", resolve_interface_name);
            },
            |val, v| v.u32(val),
        );
        crate::snap::snap_btree_map(
            v,
            surface_pool,
            |k, v| v.u32(k),
            |dq, v| crate::snap::snap_vec_deque(v, dq, |addr, v| v.u32(addr)),
        );
        crate::snap::snap_vec_deque(v, free_com, |addr, v| v.u32(addr));
        crate::snap::snap_btree_map_with(
            v,
            vtables,
            || "",
            u32::default,
            |iface, v| {
                crate::snap::snap_static_str(v, iface, "vtables interface", resolve_interface_name);
            },
            |val, v| v.u32(val),
        );
        crate::snap::snap_btree_map(
            v,
            registry,
            |k, v| v.string(k),
            |inner, v| {
                crate::snap::snap_btree_map(
                    v,
                    inner,
                    |k, v| v.string(k),
                    |(kind, bytes), v| {
                        v.u32(kind);
                        v.byte_vec(bytes);
                    },
                );
            },
        );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{fill, interface_named, interfaces, ComState, Ctx, Object, State, HANDLE_BASE};
    use crate::cli::Trace;
    use crate::cpu::{Cpu, Stop};
    use crate::guest::{Arena, Fault, GuestMem};
    use crate::host::headless::Headless;
    use crate::install::Tree;
    use crate::loader::Modules;
    use crate::runlog::RunLog;
    use crate::shim::Boundary;
    use crate::Title;

    pub(crate) fn with_test_ctx<R>(arena_len: usize, f: impl FnOnce(&mut Ctx<'_>) -> R) -> R {
        let mut cpu = Cpu::default();
        let mut mem = Arena::new(&mut Headless::new(), arena_len).expect("an arena");
        let mut state = State::new();
        let mut log = RunLog::create("unit-test", Trace::Off).expect("a run log");
        let dir = log.dir().to_path_buf();
        let mut tree = Tree::default();
        let mut modules = Modules::default();
        let mut boundary = Boundary::new();
        let out = {
            let mut ctx = Ctx {
                cpu: &mut cpu,
                mem: &mut mem,
                state: &mut state,
                title: &Title::STUB,
                log: &mut log,
                tree: &mut tree,
                modules: &mut modules,
                clock_millis: 0,
                boundary: &mut boundary,
                call_guest: None,
                float_answer: None,
                answer_is_the_call_s: false,
            };
            f(&mut ctx)
        };
        drop(log);
        let _ = std::fs::remove_dir_all(dir);
        out
    }

    #[test]
    fn a_declared_registry_row_is_found_however_a_title_spells_it() {
        let mut state = State::new();
        state.declare_registry(
            "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
            "InstallPath",
            crate::win32::advapi32::REG_SZ,
            b"C:\\Program Files\\Mattel\\Stunt Track Driver\0".to_vec(),
        );
        // The registry is not case-sensitive, and neither is this one.
        assert!(state
            .registry_value(
                "hklm\\software\\mattel\\stunt track driver\\1.0",
                "installpath"
            )
            .is_some());
        assert!(state.registry_has("HKLM\\Software\\Mattel\\Stunt Track Driver\\1.0"));
        // A value that really is not there is still not there.
        assert!(state
            .registry_value(
                "HKLM\\SOFTWARE\\MATTEL\\STUNT TRACK DRIVER\\1.0",
                "CdDataDir"
            )
            .is_none());
    }

    #[test]
    fn state_handles_lookup_close_misaligned_and_ascending_iteration() {
        let mut state = State::new();
        let h0 = state.give(Object::Module {
            name: "first.dll".to_owned(),
            base: 0x1000,
        });
        let h1 = state.give(Object::Std(1));
        let h2 = state.give(Object::Module {
            name: "first.dll".to_owned(),
            base: 0x2000,
        });
        let h3 = state.give(Object::Std(3));
        assert_eq!([h0, h1, h2, h3], [HANDLE_BASE, 0x104, 0x108, 0x10c]);

        // Under-base, misaligned, and past-end handles all resolve to None.
        for bad in [
            0,
            4,
            HANDLE_BASE - 4,
            HANDLE_BASE + 1,
            HANDLE_BASE + 2,
            HANDLE_BASE + 3,
            0x110,
            u32::MAX,
        ] {
            assert_eq!(state.object(bad), None, "bad handle {bad:#x}");
            assert_eq!(state.object_mut(bad), None, "bad mut handle {bad:#x}");
            assert!(!state.close(bad), "close bad handle {bad:#x}");
        }

        // Ascending order: module() returns the lowest live matching handle.
        assert_eq!(state.module("FIRST.DLL"), Some(h0));

        // Closing leaves a tombstoned slot; handles are never reused, and
        // module()/objects() continue in exact ascending order.
        assert!(state.close(h0));
        assert!(!state.close(h0));
        assert_eq!(state.object(h0), None);
        assert_eq!(state.module("first.dll"), Some(h2));

        let h4 = state.give(Object::Std(4));
        assert_eq!(h4, 0x110);
        let live: Vec<_> = state.objects().cloned().collect();
        assert_eq!(
            live,
            vec![
                Object::Std(1),
                Object::Module {
                    name: "first.dll".to_owned(),
                    base: 0x2000,
                },
                Object::Std(3),
                Object::Std(4),
            ]
        );
    }

    #[test]
    fn state_com_fast_path_and_secondary_face_lookup() {
        with_test_ctx(crate::guest::heap::HEAP_BASE as usize + 0x10_0000, |ctx| {
            let dd = State::make_com(
                ctx,
                &super::ddraw::DIRECT_DRAW,
                ComState::Device { keyboard: true },
            )
            .expect("make_com");
            let dd4 = State::face(ctx, dd, &super::ddraw::DIRECT_DRAW4).expect("face");
            assert_ne!(dd, dd4);
            assert_eq!(ctx.state.com(dd).map(|o| o.identity), Some(dd));
            assert_eq!(ctx.state.com(dd4).map(|o| o.identity), Some(dd));
            assert_eq!(
                ctx.state.com(dd4).map(|o| &o.state),
                Some(&ComState::Device { keyboard: true })
            );
            // Mutate through secondary face and observe via primary (and vice versa).
            ctx.state.com_mut(dd4).expect("face").app_data = 0x1234;
            assert_eq!(ctx.state.com(dd).expect("primary").app_data, 0x1234);
            ctx.state.com_mut(dd).expect("primary").app_data = 0x5678;
            assert_eq!(ctx.state.com(dd4).expect("face").app_data, 0x5678);
            assert!(ctx.state.com(0xdead_beef).is_none());
            assert!(ctx.state.com_mut(0xdead_beef).is_none());
        });
    }

    #[test]
    fn interface_named_matches_all_declared_interfaces() {
        for iface in interfaces() {
            let found = interface_named(iface.name).expect("declared interface");
            assert_eq!(found.name, iface.name);
        }
        assert!(interface_named("IDoesNotExist").is_none());
    }

    #[test]
    fn fill_matches_reference_across_depths_pitches_empty_faults_and_code_pages() {
        fn reference_fill(
            mem: &mut Arena,
            (pixels, pitch, depth): (u32, u32, u32),
            (x0, y0, x1, y1): (u32, u32, u32, u32),
            value: u32,
        ) {
            let bytes = depth.div_ceil(8);
            let one = value.to_le_bytes();
            let row: Vec<u8> = (x0..x1.max(x0))
                .flat_map(|_| one.iter().copied().take(bytes as usize))
                .collect();
            for y in y0..y1 {
                mem.write(pixels + y * pitch + x0 * bytes, &row).unwrap();
            }
        }

        for depth in [8u32, 16, 24, 32] {
            let bytes = depth.div_ceil(8);
            let (w, h) = (19u32, 7u32);
            for (label, pitch, rect) in [
                ("contiguous", w * bytes, (0, 0, w, h)),
                ("strided-full", w * bytes + 12, (0, 0, w, h)),
                ("strided-subrect", w * bytes + 8, (3, 1, 15, 6)),
                ("single-row", w * bytes + 4, (2, 4, 17, 5)),
                ("empty-rows", w * bytes, (0, 3, w, 3)),
                ("empty-cols", w * bytes, (5, 0, 5, h)),
            ] {
                for value in [0x0000_0000u32, 0xffff_ffff, 0x1234_5678] {
                    let mut expected = Arena::new(&mut Headless::new(), 1 << 16).unwrap();
                    let seed: Vec<u8> = (0usize..4096)
                        .map(|i| (i.wrapping_mul(29) ^ 0xa5) as u8)
                        .collect();
                    expected.write(0x2000, &seed).unwrap();
                    reference_fill(&mut expected, (0x2000, pitch, depth), rect, value);

                    with_test_ctx(1 << 16, |ctx| {
                        ctx.mem.write(0x2000, &seed).unwrap();
                        fill(ctx, (0x2000, pitch, depth), rect, value).unwrap();
                        assert_eq!(
                            ctx.mem.slice(0x2000, 4096).unwrap(),
                            expected.slice(0x2000, 4096).unwrap(),
                            "depth={depth} {label} value={value:#x}"
                        );
                    });
                }
            }
        }

        // Code-page invalidation and out-of-bounds fault.
        with_test_ctx(0x4000, |ctx| {
            ctx.mem.mark_code(0x2000);
            fill(ctx, (0x2000, 32, 16), (0, 0, 8, 4), 0x1234).unwrap();
            assert!(ctx.mem.code_written);
            assert!(ctx.mem.written_code.contains(&2));

            let err = fill(ctx, (0x3ff0, 32, 16), (0, 0, 16, 2), 0).unwrap_err();
            assert!(matches!(err, Stop::Fault(Fault { write: true, .. })));
        });
    }
}
