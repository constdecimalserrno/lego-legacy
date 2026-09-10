//! Guest memory.
//!
//! The arena is the whole address space the guest can see. Two paths reach it:
//! a checked one for shims and tooling, which refuses an address the guest does
//! not own, and an unchecked one for the engine, which has already bounded the
//! address by decoding the instruction that produced it (ADR-0004).

pub mod heap;

use crate::host::{Host, HostError, Mapping};
use std::fmt;

/// A guest address that is not in the arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub addr: u32,
    pub len: usize,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "guest address {:#010x}+{} is outside the arena",
            self.addr, self.len
        )
    }
}

impl std::error::Error for Fault {}

/// The guest's whole address space.
pub struct Arena {
    mapping: Mapping,
}

impl Arena {
    /// Take `len` bytes from the host as the guest's address space, based at
    /// guest address zero.
    ///
    /// # Errors
    /// When the host cannot map that much.
    pub fn new(host: &mut dyn Host, len: usize) -> Result<Self, HostError> {
        Ok(Self {
            mapping: host.map(len)?,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.mapping.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mapping.is_empty()
    }

    fn in_bounds(&self, addr: u32, len: usize) -> bool {
        (addr as usize)
            .checked_add(len)
            .is_some_and(|end| end <= self.mapping.len())
    }
}

/// The checked path into guest memory: what shims and tooling read and write.
///
/// Every access names the address it refused, so a fault is a diagnosis rather
/// than a crash.
pub trait GuestMem {
    /// # Errors
    /// When the range is outside the arena.
    fn read(&self, addr: u32, out: &mut [u8]) -> Result<(), Fault>;

    /// # Errors
    /// When the range is outside the arena.
    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), Fault>;

    /// # Errors
    /// When the range is outside the arena.
    fn read_u32(&self, addr: u32) -> Result<u32, Fault> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b)?;
        Ok(u32::from_le_bytes(b))
    }

    /// # Errors
    /// When the range is outside the arena.
    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault> {
        self.write(addr, &value.to_le_bytes())
    }

    /// Borrow a run of guest bytes, for a caller that must not copy them: the
    /// pixels the host presents are the guest's own surface.
    ///
    /// # Errors
    /// When the range is outside the arena.
    fn slice(&self, addr: u32, len: usize) -> Result<&[u8], Fault>;

    /// Borrow a run of guest bytes to change in place: a surface the
    /// rasteriser draws into.
    ///
    /// # Errors
    /// When the range is outside the arena.
    fn slice_mut(&mut self, addr: u32, len: usize) -> Result<&mut [u8], Fault>;
}

impl GuestMem for Arena {
    fn read(&self, addr: u32, out: &mut [u8]) -> Result<(), Fault> {
        out.copy_from_slice(self.slice(addr, out.len())?);
        Ok(())
    }

    fn write(&mut self, addr: u32, bytes: &[u8]) -> Result<(), Fault> {
        if !self.in_bounds(addr, bytes.len()) {
            return Err(Fault {
                addr,
                len: bytes.len(),
            });
        }
        // Safety: bounds checked directly above; the mapping is live and
        // writable for its whole length.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.mapping.as_ptr().add(addr as usize),
                bytes.len(),
            );
        }
        Ok(())
    }

    fn slice(&self, addr: u32, len: usize) -> Result<&[u8], Fault> {
        if !self.in_bounds(addr, len) {
            return Err(Fault { addr, len });
        }
        // Safety: bounds checked directly above; the mapping is live and
        // readable for its whole length, and the borrow cannot outlive it.
        unsafe {
            Ok(core::slice::from_raw_parts(
                self.mapping.as_ptr().add(addr as usize),
                len,
            ))
        }
    }

    fn slice_mut(&mut self, addr: u32, len: usize) -> Result<&mut [u8], Fault> {
        if !self.in_bounds(addr, len) {
            return Err(Fault { addr, len });
        }
        // Safety: bounds checked directly above; the mapping is live and
        // writable for its whole length, and the borrow is the arena's only
        // one for as long as it lasts.
        unsafe {
            Ok(core::slice::from_raw_parts_mut(
                self.mapping.as_ptr().add(addr as usize),
                len,
            ))
        }
    }
}

/// The engine's path into guest memory (ADR-0004).
///
/// These skip the bounds check the trait above makes. The engine takes them
/// only after [`Arena::has_window`] has bounded the address once for a whole
/// instruction, which is cheaper than checking every byte of it separately.
impl Arena {
    /// Whether a whole instruction's worth of bytes at `addr` is inside the
    /// arena, so that fetching them needs no further check.
    #[must_use]
    pub fn has_window(&self, addr: u32, len: usize) -> bool {
        self.in_bounds(addr, len)
    }

    /// # Safety
    /// `addr + 1` must be within the arena, which [`Arena::has_window`] is how
    /// the engine knows.
    #[must_use]
    pub unsafe fn read_u8_unchecked(&self, addr: u32) -> u8 {
        self.mapping.as_ptr().add(addr as usize).read()
    }

    /// # Safety
    /// `addr + 4` must be within the arena, which [`Arena::has_window`] is how
    /// the engine knows.
    #[must_use]
    pub unsafe fn read_u32_unchecked(&self, addr: u32) -> u32 {
        u32::from_le(
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u32>()
                .read_unaligned(),
        )
    }
}
