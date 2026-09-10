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
pub mod scene;
pub mod user32;
pub mod winmm;

use crate::cpu::{Cpu, Stop};
use crate::guest::heap::Heap;
use crate::guest::{Arena, Fault, GuestMem};
use crate::runlog::RunLog;
use crate::Title;
use std::collections::BTreeMap;

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
    /// A file the guest opened, and where it is reading.
    File {
        path: String,
        bytes: Vec<u8>,
        at: u32,
    },
    /// Something named that only has to be distinct: an event, a mutex.
    Named { kind: &'static str, name: String },
    /// A registry key the guest has open.
    Key { path: String },
    /// A COM object the guest holds a pointer to.
    Com { interface: &'static str, refs: i32 },
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
    /// What the screen is, as the guest last set it.
    pub display_mode: (u32, u32, u32),
    /// Each surface's source colour key, by where its pixels are.
    pub color_keys: BTreeMap<u32, u32>,
    /// The palette the screen is showing through, if any.
    pub palette: u32,
    /// The frame the guest last flipped, waiting to be presented.
    pub present: Option<Present>,
    /// Which viewport the three-dimensional device is drawing through.
    pub current_viewport: u32,
    /// How many times a viewport has been cleared.
    pub cleared: u64,
    /// Every sound playing, mixed on the declared clock.
    pub mixer: crate::sound::Mixer,
    /// The multimedia streams a title has made, by the object it holds.
    pub players: BTreeMap<u32, amstream::Player>,
    /// Which keys are down, as the guest's own device numbers them.
    pub keys_down: Vec<u32>,
    /// How far the pointer moved since the guest last looked.
    pub mouse_delta: (i32, i32),
    /// Whether the pointer's button is down.
    pub mouse_down: bool,
    /// The modal dialog a title has up, while it has one.
    pub dialog: Option<user32::Dialog>,
    /// The scene retained mode is building.
    pub scene: scene::Scene,
    /// COM objects, by the guest pointer that names them.
    com: BTreeMap<u32, ComObject>,
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
            current_directory: String::new(),
            volume_label: String::new(),
            waited_millis: 0,
            players: BTreeMap::new(),
            mixer: crate::sound::Mixer::default(),
            installed_files: 0,
            window_procedure: 0,
            window_made: false,
            quit_asked: false,
            pointer: (0, 0),
            display_mode: (640, 480, 16),
            color_keys: BTreeMap::new(),
            palette: 0,
            present: None,
            current_viewport: 0,
            cleared: 0,
            keys_down: Vec::new(),
            mouse_delta: (0, 0),
            mouse_down: false,
            dialog: None,
            scene: scene::Scene::default(),
            com: BTreeMap::new(),
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

    /// What the guest reads out of the mouse device: three axes and four
    /// buttons, as DirectInput lays them out.
    #[must_use]
    pub fn mouse_state(&self) -> Vec<u8> {
        let mut state = vec![0u8; 16];
        state[..4].copy_from_slice(&self.mouse_delta.0.to_le_bytes());
        state[4..8].copy_from_slice(&self.mouse_delta.1.to_le_bytes());
        state[12] = u8::from(self.mouse_down);
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
        self.com.insert(
            at,
            ComObject {
                interface,
                refs: 1,
                app_data: 0,
                state,
            },
        );
    }

    /// What interface a guest pointer is, if the machine made it.
    #[must_use]
    pub fn com(&self, at: u32) -> Option<&ComObject> {
        self.com.get(&at)
    }

    pub fn com_mut(&mut self, at: u32) -> Option<&mut ComObject> {
        self.com.get_mut(&at)
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
        self.registry.entry(path.to_owned()).or_default();
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
        self.registry.contains_key(path)
    }

    /// One value out of the registry, by key and name.
    #[must_use]
    pub fn registry_value(&self, path: &str, name: &str) -> Option<(u32, Vec<u8>)> {
        self.registry.get(path)?.get(name).cloned()
    }

    /// Write a value the guest gave us.
    pub fn set_registry_value(&mut self, path: &str, name: &str, kind: u32, bytes: Vec<u8>) {
        self.registry
            .entry(path.to_owned())
            .or_default()
            .insert(name.to_owned(), (kind, bytes));
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
    /// A sound buffer: where its samples are, how many it holds, and whether
    /// the guest has started it.
    Sound {
        store: u32,
        bytes: u32,
        playing: bool,
    },
    /// A drawing surface: guest memory, and how to read it.
    Surface {
        pixels: u32,
        width: u32,
        height: u32,
        pitch: u32,
        depth: u32,
        primary: bool,
        /// The surface behind this one, when it flips.
        back: u32,
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
    /// A viewport: which device, where on it, and what it clears to.
    Viewport {
        device: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        background: u32,
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
    /// What the declared clock reads, in milliseconds.
    pub clock_millis: u64,
    /// Where an interface's methods get their addresses from.
    pub boundary: &'a mut crate::shim::Boundary,
    /// Set by a shim that needs the guest's own code run before its call can
    /// finish: the function to call and what to pass it. The answer the shim
    /// returned is given to the guest once that call comes back.
    pub call_guest: Option<(u32, Vec<u32>)>,
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
