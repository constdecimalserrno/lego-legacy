//! The boundary (ADR-0002).
//!
//! A title's own binaries run as guest instructions. Everything Microsoft
//! wrote is a Rust shim standing where its export table would be: the loader
//! writes a distinct address into each import slot, and a guest that calls one
//! arrives here rather than at code that does not exist.

use crate::guest::{Arena, Fault, GuestMem};
use crate::pe::Import;

/// Where the boundary's addresses live.
///
/// Chosen well above where a title's own images land and below the arena's
/// end, so that an address in this range can only be a call across the
/// boundary and never a guest address that happens to collide.
pub const THUNK_BASE: u32 = 0x0f00_0000;

/// How far apart two entry points are. Sixteen bytes, so a disassembler
/// looking at one sees a plausible instruction boundary rather than the middle
/// of the previous entry.
pub const THUNK_STRIDE: u32 = 16;

/// What one address on the boundary stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The address a call the machine made into guest code returns to. Always
    /// the first slot, so its address is known before anything is bound.
    Return,
    /// A name a title's own image imports.
    Import(Import),
    /// One slot of a COM interface's table.
    Method {
        interface: &'static str,
        slot: usize,
    },
    /// Where the loader's own program jumps when a module answers that it did
    /// not attach (#55). Reaching it is a wall naming the module.
    Attach { module: String },
    /// Where the exception walk jumps when no handler on the chain took the
    /// exception (#54): reaching it is a wall naming what was raised.
    Unhandled { code: u32 },
    /// Where a guest thread returns to when its own function is done
    /// (ADR-0005): reaching it ends that thread rather than the run.
    ThreadEnd,
}

/// The addresses a title's imports were bound to, and what stands behind each.
#[derive(Default)]
pub struct Boundary {
    slots: Vec<Entry>,
}

/// Where guest code returns to when the machine called it.
pub const RETURN_TO_MACHINE: u32 = THUNK_BASE;

impl Boundary {
    #[must_use]
    pub fn new() -> Self {
        // The first slot is where a call the machine made comes back to, so
        // that address exists before any import is bound.
        Self {
            slots: vec![Entry::Return],
        }
    }

    /// Give every import an address of its own and write it into the image's
    /// address table, which is what the guest will call through.
    ///
    /// # Errors
    /// When the address table is outside the arena.
    pub fn bind(&mut self, imports: &[Import], mem: &mut Arena) -> Result<(), Fault> {
        for import in imports {
            let addr = THUNK_BASE + u32::try_from(self.slots.len()).unwrap_or(0) * THUNK_STRIDE;
            mem.write_u32(import.iat, addr)?;
            self.slots.push(Entry::Import(import.clone()));
        }
        Ok(())
    }

    /// Whether an address is anywhere in the boundary's range at all.
    #[must_use]
    pub fn in_range(addr: u32) -> bool {
        Self::RANGE.contains(&addr)
    }

    /// Every address the boundary could give out.
    pub const RANGE: std::ops::Range<u32> = THUNK_BASE..THUNK_BASE + 0x0010_0000;

    /// Whether an address is a call across the boundary.
    #[must_use]
    pub fn holds(&self, addr: u32) -> bool {
        addr >= THUNK_BASE
            && (addr - THUNK_BASE).is_multiple_of(THUNK_STRIDE)
            && ((addr - THUNK_BASE) / THUNK_STRIDE) < self.slots.len() as u32
    }

    /// What a boundary address stands for.
    #[must_use]
    pub fn at(&self, addr: u32) -> Option<&Entry> {
        if !self.holds(addr) {
            return None;
        }
        self.slots
            .get(((addr - THUNK_BASE) / THUNK_STRIDE) as usize)
    }

    /// Give a module a place the loader's program can jump to when its
    /// `DllMain` says it did not attach.
    pub fn bind_attach(&mut self, module: &str) -> u32 {
        let addr = THUNK_BASE + u32::try_from(self.slots.len()).unwrap_or(0) * THUNK_STRIDE;
        self.slots.push(Entry::Attach {
            module: module.to_owned(),
        });
        addr
    }

    /// A place for the exception walk to end at when nothing took it.
    pub fn bind_unhandled(&mut self, code: u32) -> u32 {
        if let Some(slot) = self
            .slots
            .iter()
            .position(|e| *e == Entry::Unhandled { code })
        {
            return THUNK_BASE + u32::try_from(slot).unwrap_or(0) * THUNK_STRIDE;
        }
        let addr = THUNK_BASE + u32::try_from(self.slots.len()).unwrap_or(0) * THUNK_STRIDE;
        self.slots.push(Entry::Unhandled { code });
        addr
    }

    /// A place for one name of a library this emulator writes, asked for by
    /// name rather than bound from an import table: what `GetProcAddress`
    /// hands back (#57).
    pub fn bind_named(&mut self, dll: &str, name: &str) -> u32 {
        let matches = |e: &Entry| match e {
            Entry::Import(import) => import.dll.eq_ignore_ascii_case(dll) && import.name == name,
            _ => false,
        };
        if let Some(slot) = self.slots.iter().position(matches) {
            return THUNK_BASE + u32::try_from(slot).unwrap_or(0) * THUNK_STRIDE;
        }
        let addr = THUNK_BASE + u32::try_from(self.slots.len()).unwrap_or(0) * THUNK_STRIDE;
        self.slots.push(Entry::Import(Import {
            dll: dll.to_owned(),
            name: name.to_owned(),
            // Nothing wrote this address into an import table: the guest was
            // handed it and will call it directly.
            iat: 0,
        }));
        addr
    }

    /// Where a thread returns to when it is done. One address serves every
    /// thread: which one ended is whichever was running.
    pub fn bind_thread_end(&mut self) -> u32 {
        if let Some(slot) = self.slots.iter().position(|e| *e == Entry::ThreadEnd) {
            return THUNK_BASE + u32::try_from(slot).unwrap_or(0) * THUNK_STRIDE;
        }
        let addr = THUNK_BASE + u32::try_from(self.slots.len()).unwrap_or(0) * THUNK_STRIDE;
        self.slots.push(Entry::ThreadEnd);
        addr
    }

    /// Give an interface's slots addresses of their own, and hand back the
    /// address of each in slot order. What a vtable is filled with.
    pub fn bind_interface(&mut self, interface: &'static str, slots: usize) -> Vec<u32> {
        let mut out = Vec::with_capacity(slots);
        for slot in 0..slots {
            let addr = THUNK_BASE + u32::try_from(self.slots.len()).unwrap_or(0) * THUNK_STRIDE;
            self.slots.push(Entry::Method { interface, slot });
            out.push(addr);
        }
        out
    }
}
