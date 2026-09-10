//! What Microsoft wrote, written here instead (ADR-0002).
//!
//! Each entry point a title imports is declared once, with the number of
//! arguments it takes and who cleans them off the stack. A guest calling one
//! arrives through the boundary, the shim answers, and the call returns as if
//! the real library had been there.

pub mod kernel32;
pub mod msvcrt;
pub mod user32;

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

    /// The handle a module already has, if it has one.
    #[must_use]
    pub fn module(&self, name: &str) -> Option<u32> {
        self.handles.iter().find_map(|(h, o)| match o {
            Object::Module { name: n, .. } if n.eq_ignore_ascii_case(name) => Some(*h),
            _ => None,
        })
    }
}

/// What a shim is given.
pub struct Ctx<'a> {
    pub cpu: &'a mut Cpu,
    pub mem: &'a mut Arena,
    pub state: &'a mut State,
    pub title: &'a Title,
    pub log: &'a mut RunLog,
}

impl Ctx<'_> {
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

/// Find the shim behind an import, if one is written.
#[must_use]
pub fn find(dll: &str, name: &str) -> Option<&'static Shim> {
    let bare = dll
        .strip_suffix(".dll")
        .or_else(|| dll.strip_suffix(".DLL"))
        .unwrap_or(dll);
    for table in [kernel32::SHIMS, user32::SHIMS, msvcrt::SHIMS] {
        if let Some(shim) = table
            .iter()
            .find(|s| s.dll.eq_ignore_ascii_case(bare) && s.name == name)
        {
            return Some(shim);
        }
    }
    None
}
