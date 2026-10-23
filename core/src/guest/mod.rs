//! Guest memory.
//!
//! The arena is the whole address space the guest can see, and every way into
//! it is checked: the engine, the shims and tooling all go through
//! [`GuestMem`] or [`Arena::read_and_write`], which refuse a range outside the
//! arena and notice a write over decoded code (ADR-0004).

pub mod heap;

use crate::host::{Host, HostError, Mapping};
use std::fmt;

/// A guest address that is not in the arena, or is under its floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub addr: u32,
    pub len: usize,
    /// Whether it was a write, which is the first thing an access violation
    /// tells the guest's handlers (#132).
    pub write: bool,
}

/// The bottom of a Windows 9x process's address space, which is what every
/// title declares (#132). Matt Pietrek, *Windows 95 System Programming
/// Secrets* (1995): a Win32 program's data selector is expand-down, and
/// "Windows 95 makes the lowest addresses in 4K of a virtual machine
/// inaccessible" with it, so a null pointer faults rather than overwriting
/// memory. Windows NT refuses the whole first 64 KiB instead, and no title
/// declares NT.
pub const NULL_PAGE: u32 = 0x0000_1000;

/// The end of the MS-DOS memory every Win32 process on Windows 9x can see
/// above the null page: the system virtual machine's first megabyte, "the
/// memory image of MS-DOS" and the bottom of the 16-bit global heap, "almost
/// completely unprotected" (Pietrek, the same book). Microsoft's KB 125691
/// says a Win32 process is not to use it. This machine does not have it, so an
/// access there stops the run rather than reading something it made up.
pub const DOS_MEMORY: u32 = 0x0010_0000;

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (addr, len) = (self.addr, self.len);
        if addr < NULL_PAGE {
            write!(
                f,
                "guest address {addr:#010x}+{len} is in the first 4 KiB, which Windows 9x \
                 refuses a Win32 process"
            )
        } else if addr < DOS_MEMORY {
            write!(
                f,
                "guest address {addr:#010x}+{len} is in the MS-DOS memory under the first \
                 megabyte, which this machine does not have"
            )
        } else {
            write!(f, "guest address {addr:#010x}+{len} is outside the arena")
        }
    }
}

impl std::error::Error for Fault {}

/// The guest's whole address space.
pub struct Arena {
    mapping: Mapping,
    /// Below this, every access faults. Zero unless a machine sets it: the
    /// conformance corpus is a 386 in real mode and lives at the bottom.
    floor: u32,
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
    /// A watch on every write, armed by the machine at a clock read, for
    /// whether any byte changes before the next crossing (ADR-0004, #217). It
    /// stops at the first write it can tell changed something, at one it
    /// cannot keep, and at `rdtsc`, a time the clock's millisecond does not
    /// cover: never wrong, only not passed for.
    watching: bool,
    /// Each run handed out to change while watching, and what it held then.
    held: Vec<(u32, u8, [u8; HELD_BYTES])>,
}

/// The most runs the watch keeps, and the longest. Past either, it stops: a
/// pass of LEGO Chess's spin hands out five.
// ponytail: a list judged by a quadratic scan for each byte's first holder,
// so a pass of more than 16 runs, or one wider than 16 bytes, is never
// passed. Upgrade path: ranges held in address order, merged as they come.
const HELD_RUNS: usize = 16;
const HELD_BYTES: usize = 16;

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
            watching: false,
            held: Vec::new(),
            mapping: host.map(len)?,
            floor: 0,
        })
    }

    /// Fault every access below `floor`, as Windows faults the pages it
    /// never maps (#132).
    #[must_use]
    pub fn with_floor(mut self, floor: u32) -> Self {
        self.floor = floor;
        self
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.mapping.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mapping.is_empty()
    }

    /// One subtraction ahead of the one compare this always was: an address
    /// under the floor wraps round to far past the end (#132). No bytes at
    /// all touch nothing there, which is what the rasteriser borrows at a
    /// null pointer when no texture is set.
    fn in_bounds(&self, addr: u32, len: usize) -> bool {
        (addr.wrapping_sub(self.floor) as usize)
            .checked_add(len)
            .is_some_and(|end| end <= self.mapping.len() - self.floor as usize)
            || (len == 0 && addr < self.floor)
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
                write: true,
            });
        }
        if self.wrote_over_code(addr, bytes.len()) {
            self.note_written(addr, bytes.len());
        }
        if self.watching {
            self.compare(addr, bytes);
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
            return Err(Fault {
                addr,
                len,
                write: false,
            });
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
            return Err(Fault {
                addr,
                len,
                write: true,
            });
        }
        if self.wrote_over_code(addr, len) {
            self.note_written(addr, len);
        }
        if self.watching {
            self.hold(addr, len);
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
    /// Watch every write from here: whether any byte changes (ADR-0004,
    /// #217).
    pub(crate) fn watch(&mut self) {
        self.watching = true;
        self.held.clear();
    }

    pub(crate) fn unwatch(&mut self) {
        self.watching = false;
    }

    /// Whether, since [`Arena::watch`], no byte has changed. A byte is judged
    /// by what the first run over it held: a later run found what an earlier
    /// one left there, which says nothing of before. Asked only once the
    /// processor is the same, so the runs are kept cheaply and judged here.
    #[must_use]
    pub(crate) fn unchanged(&self) -> bool {
        let first = |i: usize, byte: u32| {
            !self.held[..i]
                .iter()
                .any(|&(at, len, _)| (at..at + u32::from(len)).contains(&byte))
        };
        self.watching
            && self.held.iter().enumerate().all(|(i, &(at, len, was))| {
                (0..u32::from(len)).all(|k| {
                    let byte = at + k;
                    !first(i, byte)
                        || self
                            .slice(byte, 1)
                            .is_ok_and(|now| now[0] == was[k as usize])
                })
            })
    }

    /// A write while watching: what it writes over is what it writes, or the
    /// watch stops.
    #[cold]
    fn compare(&mut self, addr: u32, bytes: &[u8]) {
        if self.slice(addr, bytes.len()).ok() != Some(bytes) {
            self.watching = false;
        }
    }

    /// A run handed out to change while watching: keep what it holds now, to
    /// compare when asked.
    #[cold]
    fn hold(&mut self, addr: u32, len: usize) {
        if len > HELD_BYTES || self.held.len() == HELD_RUNS {
            self.watching = false;
            return;
        }
        let mut was = [0; HELD_BYTES];
        was[..len].copy_from_slice(self.slice(addr, len).expect("bounds checked"));
        self.held.push((addr, len as u8, was));
    }

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
        for (addr, len, write) in [(read, read_len, false), (write, write_len, true)] {
            if !self.in_bounds(addr, len) {
                return Err(Fault { addr, len, write });
            }
        }
        let (r, w) = (read as usize, write as usize);
        if r < w + write_len && w < r + read_len {
            return Err(Fault {
                addr: write,
                len: write_len,
                write: true,
            });
        }
        // Handing out a run to change is writing it, as `slice_mut` is.
        if self.wrote_over_code(write, write_len) {
            self.note_written(write, write_len);
        }
        self.watching = false;
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

    /// Under the floor every byte faults, a read or a write, and the first
    /// byte above it does not; no bytes at all is never refused (#132).
    #[test]
    fn the_floor_faults_every_byte_under_it_and_none_above() {
        use super::{GuestMem, DOS_MEMORY};
        let mut mem = Arena::new(&mut Headless::new(), 1 << 21)
            .expect("an arena")
            .with_floor(DOS_MEMORY);
        assert!(mem.slice(8, 4).is_err_and(|f| !f.write));
        assert!(mem.write(DOS_MEMORY - 4, &[0; 4]).is_err_and(|f| f.write));
        assert!(mem.slice(DOS_MEMORY - 2, 4).is_err());
        assert!(mem.slice(0, 0).is_ok());
        assert!(mem.write(DOS_MEMORY, &[1; 4]).is_ok());
        assert!(
            mem.slice((1 << 21) - 4, 5).is_err(),
            "and the end is where it was"
        );
    }
}
