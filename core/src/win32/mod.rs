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
        bytes: Vec<u8>,
        at: u32,
        /// Whether the guest asked to write through this handle. What it
        /// writes reaches the profile when it closes the file.
        writing: bool,
        /// Whether it actually wrote anything. A file opened for writing and
        /// never written to is not one the profile keeps a copy of.
        wrote: bool,
    },
    /// Something named that only has to be distinct: an event, a mutex.
    Named { kind: &'static str, name: String },
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
    /// A COM object the guest holds a pointer to.
    Com { interface: &'static str, refs: i32 },
    /// A multimedia file over guest memory, and where it is reading; `owned`
    /// when the machine put the file there and frees it on close.
    Mmio {
        base: u32,
        len: u32,
        at: u32,
        owned: bool,
    },
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
    /// sleeping. Until then the machine runs somebody else.
    pub wake_at: u64,
}

/// Everything the guest holds that is not memory.
pub struct State {
    pub heap: Heap,
    /// Handles the guest has been given.
    handles: BTreeMap<u32, Object>,
    next_handle: u32,
    /// Where strings the machine hands the guest are kept.
    pub scratch: Vec<u32>,
    /// The command line the guest reads, once it has asked for it.
    pub command_line: Option<u32>,
    /// What the guest last called, for the run log and for a wall's message.
    pub last_call: Option<String>,
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
    /// many times over (ADR-0005). A section nobody holds is not in here.
    pub sections: BTreeMap<u32, (u32, u32)>,
    /// Address ranges the guest has claimed by naming an address to
    /// `VirtualAlloc`, outside the heap. A second claim that overlaps one is
    /// refused, which is what tells a guest's own DLL loader to relocate the
    /// image rather than lay it over the last one (#61).
    pub claimed: Vec<(u32, u32)>,
    /// Where the next file mapped into the arena will go. Mappings sit above
    /// everything else the machine hands out, because this title maps four
    /// hundred megabytes of archives and never lets go (#57).
    pub next_mapping: u32,
    /// Which failures the guest asked to be shown rather than handed back.
    /// Kept so that `SetErrorMode` can answer with what it was, and read by
    /// nothing: this machine puts no dialog up.
    pub error_mode: u32,
    /// Where the guest believes it is.
    pub current_directory: String,
    /// The label on the disc in the declared machine's drive.
    pub volume_label: String,
    /// Declared time the guest spent waiting inside the last call: a sleep,
    /// or a blocking wait for a movie frame. The machine charges it to the
    /// declared clock when the call returns (ADR-0003).
    pub waited_millis: u64,
    /// How many files the declared install tree holds.
    pub installed_files: usize,
    /// The window procedure the guest registered, if it has.
    pub window_procedure: u32,
    /// Whether the guest has made its window.
    pub window_made: bool,
    /// Whether the guest has asked to stop.
    pub quit_asked: bool,
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
    /// The palette the screen is showing through, if any.
    pub palette: u32,
    /// The frame the guest last flipped, waiting to be presented.
    pub present: Option<Present>,
    /// How many times a viewport has been cleared.
    pub cleared: u64,
    /// Every sound playing, mixed on the declared clock.
    pub mixer: crate::sound::Mixer,
    /// DirectSound's buffers and listener.
    pub sound: dsound::Sounds,
    /// The multimedia streams a title has made, by the object it holds.
    pub players: BTreeMap<u32, amstream::Player>,
    /// The CD audio device.
    pub cd: winmm::CdAudio,
    /// Which keys are down, as the guest's own device numbers them.
    pub keys_down: Vec<u32>,
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
    /// Whether the pointer's button is down.
    pub mouse_down: bool,
    /// When the button last went down and where, so that a second click near
    /// it and soon after it is a double click rather than two clicks (#59).
    pub last_click: (u32, (i32, i32)),
    /// Whether the class the title registered asked for double clicks.
    pub double_clicks: bool,
    /// Where the title last asked that the pointer be put, until the machine
    /// has told the host about it (#59).
    pub pointer_put: Option<(i32, i32)>,
    /// Windows' own show count for the pointer: it is drawn while this is
    /// zero or more, and a title that draws its own takes it below zero.
    pub cursor_shown: i32,
    /// Window messages waiting for the title to take them, oldest first.
    pub messages: VecDeque<Message>,
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
    /// Which vtable an interface's methods live behind, once one is made.
    vtables: BTreeMap<&'static str, u32>,
    /// Settings files, by path, section and key.
    profiles: BTreeMap<String, BTreeMap<String, BTreeMap<String, String>>>,
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
            heap: Heap::new(),
            handles: BTreeMap::new(),
            // Handles are small odd numbers, distinct from any address, so a
            // guest that uses one as a pointer faults instead of reading
            // something plausible.
            next_handle: 0x0000_0100,
            scratch: Vec::new(),
            command_line: None,
            last_call: None,
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
            }],
            thread: 0,
            next_thread_id: crate::machine::THREAD_ID,
            claimed: Vec::new(),
            next_mapping: MAPPING_BASE,
            sections: BTreeMap::new(),
            tls_slots: 0,
            current_directory: String::new(),
            volume_label: String::new(),
            waited_millis: 0,
            players: BTreeMap::new(),
            cd: winmm::CdAudio::default(),
            mixer: crate::sound::Mixer::default(),
            sound: dsound::Sounds::default(),
            installed_files: 0,
            window_procedure: 0,
            window_made: false,
            quit_asked: false,
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
            palette: 0,
            present: None,
            cleared: 0,
            keys_down: Vec::new(),
            mouse_delta: (0, 0),
            mouse_data: VecDeque::new(),
            keyboard_data: VecDeque::new(),
            input_sequence: 0,
            mouse_down: false,
            last_click: (0, (0, 0)),
            pointer_put: None,
            double_clicks: false,
            cursor_shown: 0,
            messages: VecDeque::new(),
            dialog: None,
            names: BTreeMap::new(),
            set_values: BTreeMap::new(),
            immediate: d3d::Immediate::default(),
            scene: scene::Scene::default(),
            render: None,
            com: BTreeMap::new(),
            faces: BTreeMap::new(),
            vtables: BTreeMap::new(),
            profiles: BTreeMap::new(),
            registry: BTreeMap::new(),
        }
    }

    /// Give the guest a handle to something.
    pub fn give(&mut self, object: Object) -> u32 {
        let handle = self.next_handle;
        self.next_handle += 4;
        self.handles.insert(handle, object);
        handle
    }

    #[must_use]
    pub fn object(&self, handle: u32) -> Option<&Object> {
        self.handles.get(&handle)
    }

    pub fn object_mut(&mut self, handle: u32) -> Option<&mut Object> {
        self.handles.get_mut(&handle)
    }

    /// Take a handle back.
    pub fn close(&mut self, handle: u32) -> bool {
        self.handles.remove(&handle).is_some()
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
        let object = ctx.state.heap.alloc(8);
        ctx.mem.write_u32(object, vtable)?;
        ctx.state.add_com(object, interface.name, state);
        Ok(object)
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
        self.com.get(&self.identity(at))
    }

    pub fn com_mut(&mut self, at: u32) -> Option<&mut ComObject> {
        let identity = self.identity(at);
        self.com.get_mut(&identity)
    }

    /// Which interface a guest pointer is: the face itself, not the object.
    #[must_use]
    pub fn interface_of(&self, at: u32) -> Option<&'static str> {
        self.com.get(&at).map(|o| o.interface)
    }

    /// Count a reference in or out, and answer with what is left.
    pub fn com_ref(&mut self, at: u32, by: i32) -> u32 {
        match self.com.get_mut(&at) {
            Some(object) => {
                object.refs = (object.refs + by).max(0);
                u32::try_from(object.refs).unwrap_or(0)
            }
            None => 0,
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

    /// One setting out of a file the guest reads.
    #[must_use]
    pub fn profile_value(&self, path: &str, section: &str, key: &str) -> Option<String> {
        self.profiles.get(path)?.get(section)?.get(key).cloned()
    }

    /// Write a setting the guest gave us.
    pub fn set_profile_value(&mut self, path: &str, section: &str, key: &str, value: String) {
        self.profiles
            .entry(path.to_owned())
            .or_default()
            .entry(section.to_owned())
            .or_default()
            .insert(key.to_owned(), value);
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
        self.handles.iter().find_map(|(h, o)| match o {
            Object::Module { name: n, .. } if n.eq_ignore_ascii_case(name) => Some(*h),
            _ => None,
        })
    }
}

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
    Device { keyboard: bool, acquired: bool },
    /// A sound buffer, whose bytes and playing state DirectSound keeps
    /// (`dsound::Sounds`).
    Sound,
    /// A drawing surface: guest memory, and how to read it.
    Surface {
        pixels: u32,
        width: u32,
        height: u32,
        pitch: u32,
        /// How a pixel is laid out: what the title asked for, or the screen's.
        format: crate::raster::PixelFormat,
        primary: bool,
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
    /// A three-dimensional device, and the surface it draws into.
    Device3D { surface: u32 },
    /// A buffer of vertices the title fills and then draws from (#57): where
    /// they are, how they are laid out, and how many fit.
    VertexBuffer { store: u32, fvf: u32, count: u32 },
    /// A frame's children, as they were when a title asked for them.
    FrameArray { frames: Vec<u32> },
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
/// arguments, stopping early if it says to.
///
/// A shim can park one call into guest code and no more (`call_guest`), which
/// is enough for an enumeration with one answer in it and not for one with
/// six. What goes into guest memory here is a short program that makes the
/// calls itself and returns to the machine at the end — the same thing the
/// module loader does to run a DllMain (#55, #57).
///
/// The callback is stdcall, so each call clears its own arguments; a callback
/// that answers zero has asked to stop, which Direct3D spells
/// `D3DENUMRET_CANCEL`.
///
/// # Errors
/// When the program will not fit in guest memory.
pub fn call_each(ctx: &mut Ctx<'_>, callback: u32, calls: &[Vec<u32>]) -> Result<(), Stop> {
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

    /// A path the guest gave us, resolved the way the guest means it: a
    /// relative one is against wherever the guest believes it is.
    #[must_use]
    pub fn absolute(&self, path: &str) -> String {
        let path = path.trim();
        let rooted = path.len() > 1 && path.as_bytes().get(1) == Some(&b':');
        if rooted || path.starts_with('\\') {
            path.to_owned()
        } else {
            format!("{}\\{}", self.state.current_directory, path)
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

/// A `D3DCOLOR` as a screen of this depth stores it.
#[must_use]
pub fn pixel(colour: u32, depth: u32) -> Vec<u8> {
    let [b, g, r, _] = colour.to_le_bytes();
    match depth {
        16 => {
            let packed = (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
            packed.to_le_bytes().to_vec()
        }
        _ => colour.to_le_bytes()[..depth.div_ceil(8) as usize].to_vec(),
    }
}

/// Fill a rectangle of a surface, `(pixels, pitch, depth)`, with a colour.
///
/// # Errors
/// When the rectangle reaches outside the arena.
pub fn fill(
    ctx: &mut Ctx<'_>,
    (pixels, pitch, depth): (u32, u32, u32),
    (x0, y0, x1, y1): (u32, u32, u32, u32),
    colour: u32,
) -> Result<(), Stop> {
    let bytes = depth.div_ceil(8);
    let one = pixel(colour, depth);
    let row: Vec<u8> = (x0..x1.max(x0)).flat_map(|_| one.iter().copied()).collect();
    for y in y0..y1 {
        ctx.mem
            .write(pixels + y * pitch + x0 * bytes, &row)
            .map_err(Stop::Fault)?;
    }
    ctx.state.cleared += 1;
    Ok(())
}

/// Every entry point that is written rather than only named.
#[must_use]
pub fn shims() -> &'static [Shim] {
    kernel32::SHIMS
}

/// The interfaces this machine can hand the guest.
#[must_use]
pub fn interfaces() -> Vec<&'static crate::com::Interface> {
    let mut all = Vec::new();
    all.extend_from_slice(dinput::INTERFACES);
    all.extend_from_slice(dsound::INTERFACES);
    all.extend_from_slice(ddraw::INTERFACES);
    all.extend_from_slice(d3drm::INTERFACES);
    all.extend_from_slice(d3d::INTERFACES);
    all.extend_from_slice(amstream::INTERFACES);
    all
}

/// One interface, by the name it is declared under.
#[must_use]
pub fn interface_named(name: &str) -> Option<&'static crate::com::Interface> {
    interfaces().into_iter().find(|i| i.name == name)
}

/// One method of one interface, if it is declared.
#[must_use]
pub fn com_method(interface: &str, slot: usize) -> Option<&'static crate::com::Method> {
    interfaces()
        .iter()
        .find(|i| i.name == interface)?
        .method(slot)
}

/// Find the shim behind an import, if one is written.
#[must_use]
pub fn find(dll: &str, name: &str) -> Option<&'static Shim> {
    let bare = dll
        .strip_suffix(".dll")
        .or_else(|| dll.strip_suffix(".DLL"))
        .unwrap_or(dll);
    for table in [
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
        ole32::SHIMS,
        version::SHIMS,
        winmm::SHIMS,
    ] {
        if let Some(shim) = table
            .iter()
            .find(|s| s.dll.eq_ignore_ascii_case(bare) && s.name == name)
        {
            return Some(shim);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::State;

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
}
