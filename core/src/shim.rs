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

/// One entry point the emulator implements.
pub struct Shim {
    pub dll: &'static str,
    pub name: &'static str,
}

/// Every entry point that is written rather than run.
///
/// Empty on purpose: nothing has been measured as needed yet, and a shim that
/// exists before a guest has asked for it is a guess. Rock Raiders' boot path
/// fills this in #8.
pub const SHIMS: &[Shim] = &[];

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
        (THUNK_BASE..THUNK_BASE + 0x0010_0000).contains(&addr)
    }

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

    /// Whether an entry point is written rather than only named.
    #[must_use]
    pub fn implemented(dll: &str, name: &str) -> bool {
        SHIMS
            .iter()
            .any(|s| s.dll.eq_ignore_ascii_case(dll) && s.name == name)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Every import, in the order the image named them.
    pub fn imports(&self) -> impl Iterator<Item = &Import> {
        self.slots.iter().filter_map(|e| match e {
            Entry::Import(import) => Some(import),
            Entry::Method { .. } | Entry::Return => None,
        })
    }
}
