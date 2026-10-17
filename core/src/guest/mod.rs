//! Guest memory.
//!
//! The arena is the whole address space the guest can see, and every way into
//! it is checked: the engine, the shims and tooling all go through
//! [`GuestMem`] or [`Arena::read_and_write`], which refuse a range outside the
//! arena and notice a write over decoded code (ADR-0004).

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
    /// Which pages the engine has decoded instructions out of, one bit each
    /// (ADR-0004). A write to one of them means what is cached for it is no
    /// longer what is there.
    code: Vec<u64>,
    /// Whether a write has landed on such a page since the engine last
    /// looked. Alpha Team loads its own DLLs out of its archives and calls
    /// them, so what is at an address changes under the cache (#58).
    pub code_written: bool,
    /// Which pages have been written over since the engine last looked, so
    /// that it can drop the runs decoded out of those rather than everything
    /// it has (#65).
    pub written_code: Vec<u32>,
    /// More pages were written than the list above keeps, so the engine has
    /// to forget everything rather than work out what changed.
    pub written_all: bool,
}

/// How big a page is for the purpose of watching code, which is the size the
/// engine's own cache is indexed by.
const CODE_PAGE: u32 = 4096;

impl Arena {
    /// Take `len` bytes from the host as the guest's address space, based at
    /// guest address zero.
    ///
    /// # Errors
    /// When the host cannot map that much.
    pub fn new(host: &mut dyn Host, len: usize) -> Result<Self, HostError> {
        Ok(Self {
            code: vec![0; (len / CODE_PAGE as usize).div_ceil(64) + 1],
            code_written: false,
            written_code: Vec::new(),
            written_all: false,
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

/// The checked path into guest memory: what the engine, shims and tooling read
/// and write.
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
        if self.wrote_over_code(addr, bytes.len()) {
            self.note_written(addr, bytes.len());
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
        if self.wrote_over_code(addr, len) {
            self.note_written(addr, len);
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

impl Arena {
    /// Say that the engine has decoded instructions out of this address, so
    /// that a write over them is noticed.
    pub fn mark_code(&mut self, addr: u32) {
        let page = (addr / CODE_PAGE) as usize;
        if let Some(word) = self.code.get_mut(page / 64) {
            *word |= 1 << (page % 64);
        }
    }

    /// Say that the engine has no instructions decoded out of this page any
    /// more, so writing over it costs nothing until it decodes there again.
    pub fn unmark_code(&mut self, addr: u32) {
        let page = (addr / CODE_PAGE) as usize;
        if let Some(word) = self.code.get_mut(page / 64) {
            *word &= !(1 << (page % 64));
        }
    }

    /// Remember which pages a write landed on. A write spanning more pages
    /// than this keeps, or one page too many before the engine next looks, is
    /// answered by forgetting everything: that is rare, and correct.
    ///
    /// ponytail: eight pages, which is more than any write this roster makes;
    /// a title that writes over its own code in wider strides would want a
    /// range rather than a list.
    #[inline(always)]
    fn note_written(&mut self, addr: u32, len: usize) {
        self.code_written = true;
        if self.written_all {
            return;
        }
        if self.written_code.len() >= 8 {
            self.written_code.clear();
            self.written_all = true;
            return;
        }
        let first = addr / CODE_PAGE;
        let last = u32::try_from((addr as usize + len.saturating_sub(1)) / CODE_PAGE as usize)
            .unwrap_or(first)
            .max(first);
        for page in first..=last {
            if !self.written_code.contains(&page) {
                self.written_code.push(page);
            }
        }
    }

    /// Whether a write of `len` bytes at `addr` lands on anything decoded.
    #[inline(always)]
    fn wrote_over_code(&self, addr: u32, len: usize) -> bool {
        let first = (addr / CODE_PAGE) as usize;
        let last = ((addr as usize + len.saturating_sub(1)) / CODE_PAGE as usize).max(first);
        (first..=last).any(|page| {
            self.code
                .get(page / 64)
                .is_some_and(|word| word & (1 << (page % 64)) != 0)
        })
    }

    /// Borrow one run of guest bytes to read and another, apart from it, to
    /// change: what drawing from one surface into another needs, without a
    /// copy of the one drawn from.
    ///
    /// # Errors
    /// When either run is outside the arena, or the two overlap.
    pub fn read_and_write(
        &mut self,
        (read, read_len): (u32, usize),
        (write, write_len): (u32, usize),
    ) -> Result<(&[u8], &mut [u8]), Fault> {
        for (addr, len) in [(read, read_len), (write, write_len)] {
            if !self.in_bounds(addr, len) {
                return Err(Fault { addr, len });
            }
        }
        let (r, w) = (read as usize, write as usize);
        if r < w + write_len && w < r + read_len {
            return Err(Fault {
                addr: write,
                len: write_len,
            });
        }
        // Handing out a run to change is writing it, as `slice_mut` is.
        if self.wrote_over_code(write, write_len) {
            self.note_written(write, write_len);
        }
        // Safety: both runs are bounds checked directly above and do not
        // overlap, so the shared borrow and the exclusive one never alias;
        // the mapping is live for both, and neither outlives the arena's own
        // exclusive borrow.
        unsafe {
            let base = self.mapping.as_ptr();
            Ok((
                core::slice::from_raw_parts(base.add(r), read_len),
                core::slice::from_raw_parts_mut(base.add(w), write_len),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Arena;
    use crate::host::headless::Headless;

    /// A run handed out to change beside one to read is a write, and a write
    /// over decoded code is noticed however it was made (#58).
    #[test]
    fn code_changed_through_a_read_and_a_write_is_noticed() {
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).expect("an arena");
        mem.mark_code(0x2000);
        mem.read_and_write((0x100, 4), (0x2000, 4))
            .expect("in the arena, and apart");
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [2]);
    }
}
