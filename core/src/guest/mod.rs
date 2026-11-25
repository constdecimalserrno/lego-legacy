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
    /// Page-aligned byte bounding box `[code_min, code_max)` covering every set
    /// bit in `code`, or `(u32::MAX, 0)` when no code pages are marked (#283).
    code_min: u32,
    code_max: u32,
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
            code_min: u32::MAX,
            code_max: 0,
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

    /// Raw arena base pointer, floor, total byte length, and code-page bitmap
    /// pointer for the flat32 `naked_asm!` CPU interpreter (ADR-0009, #256).
    #[inline(always)]
    pub(crate) fn raw_parts_mut(&mut self) -> (*mut u8, u32, usize, *const u64) {
        (
            self.mapping.as_ptr(),
            self.floor,
            self.mapping.len(),
            self.code.as_ptr(),
        )
    }

    /// Page-aligned `[code_min, code_max)` bounding box covering all currently
    /// marked code pages, or `(u32::MAX, 0)` when empty (#283).
    #[inline(always)]
    #[must_use]
    pub(crate) fn code_bbox(&self) -> (u32, u32) {
        (self.code_min, self.code_max)
    }

    /// One subtraction ahead of the one compare this always was: an address
    /// under the floor wraps round to far past the end (#132). No bytes at
    /// all touch nothing there, which is what the rasteriser borrows at a
    /// null pointer when no texture is set.
    #[inline(always)]
    fn in_bounds(&self, addr: u32, len: usize) -> bool {
        self.in_bounds_nonzero(addr, len) || (len == 0 && addr < self.floor)
    }

    /// Bounds check for fixed non-zero lengths, omitting the zero-length
    /// floor exception.
    #[inline(always)]
    fn in_bounds_nonzero(&self, addr: u32, len: usize) -> bool {
        (addr.wrapping_sub(self.floor) as usize)
            .checked_add(len)
            .is_some_and(|end| end <= self.mapping.len() - self.floor as usize)
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
    fn read_u8(&self, addr: u32) -> Result<u8, Fault> {
        let mut b = [0u8; 1];
        self.read(addr, &mut b)?;
        Ok(b[0])
    }

    /// # Errors
    /// When the range is outside the arena.
    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault> {
        self.write(addr, &[value])
    }

    /// # Errors
    /// When the range is outside the arena.
    fn read_u16(&self, addr: u32) -> Result<u16, Fault> {
        let mut b = [0u8; 2];
        self.read(addr, &mut b)?;
        Ok(u16::from_le_bytes(b))
    }

    /// # Errors
    /// When the range is outside the arena.
    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault> {
        self.write(addr, &value.to_le_bytes())
    }

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

    /// # Errors
    /// When the range is outside the arena.
    fn read_u64(&self, addr: u32) -> Result<u64, Fault> {
        let mut b = [0u8; 8];
        self.read(addr, &mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    /// # Errors
    /// When the range is outside the arena.
    fn write_u64(&mut self, addr: u32, value: u64) -> Result<(), Fault> {
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

    #[inline(always)]
    fn read_u8(&self, addr: u32) -> Result<u8, Fault> {
        if !self.in_bounds_nonzero(addr, 1) {
            return Err(Fault {
                addr,
                len: 1,
                write: false,
            });
        }
        // Safety: bounds checked directly above; the mapping is live and readable.
        Ok(unsafe { *self.mapping.as_ptr().add(addr as usize) })
    }

    #[inline(always)]
    fn write_u8(&mut self, addr: u32, value: u8) -> Result<(), Fault> {
        if !self.in_bounds_nonzero(addr, 1) {
            return Err(Fault {
                addr,
                len: 1,
                write: true,
            });
        }
        if self.wrote_over_code(addr, 1) {
            self.note_written(addr, 1);
        }
        if self.watching {
            self.hold_bytes(addr, &[value]);
        }
        // Safety: bounds checked directly above; the mapping is live and writable.
        unsafe {
            *self.mapping.as_ptr().add(addr as usize) = value;
        }
        Ok(())
    }

    #[inline(always)]
    fn read_u16(&self, addr: u32) -> Result<u16, Fault> {
        if !self.in_bounds_nonzero(addr, 2) {
            return Err(Fault {
                addr,
                len: 2,
                write: false,
            });
        }
        // Safety: bounds checked directly above; `read_unaligned` permits any alignment.
        let raw = unsafe {
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u16>()
                .read_unaligned()
        };
        Ok(u16::from_le(raw))
    }

    #[inline(always)]
    fn write_u16(&mut self, addr: u32, value: u16) -> Result<(), Fault> {
        if !self.in_bounds_nonzero(addr, 2) {
            return Err(Fault {
                addr,
                len: 2,
                write: true,
            });
        }
        if self.wrote_over_code(addr, 2) {
            self.note_written(addr, 2);
        }
        if self.watching {
            self.hold_bytes(addr, &value.to_le_bytes());
        }
        // Safety: bounds checked directly above; `write_unaligned` permits any alignment.
        unsafe {
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u16>()
                .write_unaligned(value.to_le());
        }
        Ok(())
    }

    #[inline(always)]
    fn read_u32(&self, addr: u32) -> Result<u32, Fault> {
        if !self.in_bounds_nonzero(addr, 4) {
            return Err(Fault {
                addr,
                len: 4,
                write: false,
            });
        }
        // Safety: bounds checked directly above; `read_unaligned` permits any alignment.
        let raw = unsafe {
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u32>()
                .read_unaligned()
        };
        Ok(u32::from_le(raw))
    }

    #[inline(always)]
    fn write_u32(&mut self, addr: u32, value: u32) -> Result<(), Fault> {
        if !self.in_bounds_nonzero(addr, 4) {
            return Err(Fault {
                addr,
                len: 4,
                write: true,
            });
        }
        if self.wrote_over_code(addr, 4) {
            self.note_written(addr, 4);
        }
        if self.watching {
            self.hold_bytes(addr, &value.to_le_bytes());
        }
        // Safety: bounds checked directly above; `write_unaligned` permits any alignment.
        unsafe {
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u32>()
                .write_unaligned(value.to_le());
        }
        Ok(())
    }

    #[inline(always)]
    fn read_u64(&self, addr: u32) -> Result<u64, Fault> {
        if !self.in_bounds_nonzero(addr, 8) {
            return Err(Fault {
                addr,
                len: 8,
                write: false,
            });
        }
        // Safety: bounds checked directly above; `read_unaligned` permits any alignment.
        let raw = unsafe {
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u64>()
                .read_unaligned()
        };
        Ok(u64::from_le(raw))
    }

    #[inline(always)]
    fn write_u64(&mut self, addr: u32, value: u64) -> Result<(), Fault> {
        if !self.in_bounds_nonzero(addr, 8) {
            return Err(Fault {
                addr,
                len: 8,
                write: true,
            });
        }
        if self.wrote_over_code(addr, 8) {
            self.note_written(addr, 8);
        }
        if self.watching {
            self.hold_bytes(addr, &value.to_le_bytes());
        }
        // Safety: bounds checked directly above; `write_unaligned` permits any alignment.
        unsafe {
            self.mapping
                .as_ptr()
                .add(addr as usize)
                .cast::<u64>()
                .write_unaligned(value.to_le());
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

    #[inline(always)]
    #[must_use]
    pub(crate) fn is_watching(&self) -> bool {
        self.watching
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

    /// A scalar write while watching: skip holding if memory already holds the
    /// exact bytes, otherwise record what was there so a later write in the
    /// same pass can still restore it (#217).
    #[cold]
    fn hold_bytes(&mut self, addr: u32, bytes: &[u8]) {
        if self.slice(addr, bytes.len()).ok() != Some(bytes) {
            self.hold(addr, bytes.len());
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
            let start = (page as u32) * CODE_PAGE;
            let end = start.saturating_add(CODE_PAGE);
            if start < self.code_min {
                self.code_min = start;
            }
            if end > self.code_max {
                self.code_max = end;
            }
        }
    }

    /// Say that the engine has no instructions decoded out of this page any
    /// more, so writing over it costs nothing until it decodes there again.
    pub fn unmark_code(&mut self, addr: u32) {
        let page = (addr / CODE_PAGE) as usize;
        if let Some(word) = self.code.get_mut(page / 64) {
            *word &= !(1 << (page % 64));
            let start = (page as u32) * CODE_PAGE;
            let end = start.saturating_add(CODE_PAGE);
            if start == self.code_min || end == self.code_max {
                self.recompute_code_bbox();
            }
        }
    }

    /// Clear every marked code page and reset the bounding box to empty
    /// `(u32::MAX, 0)` (#283).
    pub fn clear_code(&mut self) {
        self.code.fill(0);
        self.code_min = u32::MAX;
        self.code_max = 0;
    }

    #[cold]
    fn recompute_code_bbox(&mut self) {
        let Some((first_w, &first_bits)) = self.code.iter().enumerate().find(|(_, &w)| w != 0)
        else {
            self.code_min = u32::MAX;
            self.code_max = 0;
            return;
        };
        let first_page = (first_w * 64 + first_bits.trailing_zeros() as usize) as u32;
        let (last_w, &last_bits) = self
            .code
            .iter()
            .enumerate()
            .rev()
            .find(|(_, &w)| w != 0)
            .expect("first_w already found a non-zero bitmap word");
        let last_page = (last_w * 64 + (63 - last_bits.leading_zeros() as usize)) as u32;
        self.code_min = first_page * CODE_PAGE;
        self.code_max = (last_page * CODE_PAGE).saturating_add(CODE_PAGE);
    }

    /// Remember which pages a write landed on. A write spanning more pages
    /// than this keeps, or one page too many before the engine next looks, is
    /// answered by forgetting everything: that is rare, and correct.
    ///
    /// ponytail: eight pages, which is more than any write this roster makes;
    /// a title that writes over its own code in wider strides would want a
    /// range rather than a list.
    #[cold]
    #[inline(never)]
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
    ///
    /// Precondition: `self.in_bounds(addr, len)` holds, so `addr <= mapping.len()`
    /// and the single-page word index is within `self.code`.
    #[inline(always)]
    fn wrote_over_code(&self, addr: u32, len: usize) -> bool {
        if self.code_min >= self.code_max {
            return false;
        }
        if len > 0
            && (addr >= self.code_max
                || u64::from(addr).saturating_add(len as u64) <= u64::from(self.code_min))
        {
            return false;
        }
        let first = (addr / CODE_PAGE) as usize;
        if (addr & (CODE_PAGE - 1)) as usize + len <= CODE_PAGE as usize {
            let word = first >> 6;
            debug_assert!(word < self.code.len());
            // Safety: `self.code` has `(mapping.len() / CODE_PAGE).div_ceil(64) + 1`
            // entries and `addr <= mapping.len()`, so `word < self.code.len()`.
            return unsafe { (*self.code.get_unchecked(word) & (1u64 << (first & 63))) != 0 };
        }
        self.wrote_over_code_multi(first, addr, len)
    }

    #[cold]
    #[inline(never)]
    fn wrote_over_code_multi(&self, first: usize, addr: u32, len: usize) -> bool {
        let last =
            ((addr as usize).saturating_add(len.saturating_sub(1)) / CODE_PAGE as usize).max(first);
        let first_word = first >> 6;
        let last_word = last >> 6;
        let head_mask = !0u64 << (first & 63);
        let tail_mask = !0u64 >> (63 - (last & 63));
        if first_word == last_word {
            return self
                .code
                .get(first_word)
                .is_some_and(|&w| (w & (head_mask & tail_mask)) != 0);
        }
        if self
            .code
            .get(first_word)
            .is_some_and(|&w| (w & head_mask) != 0)
        {
            return true;
        }
        let mid_start = first_word + 1;
        let mid_end = last_word.min(self.code.len());
        if mid_start < mid_end && self.code[mid_start..mid_end].iter().any(|&w| w != 0) {
            return true;
        }
        self.code
            .get(last_word)
            .is_some_and(|&w| (w & tail_mask) != 0)
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

    #[test]
    fn scalar_reads_and_writes_cover_all_widths_and_page_crossings() {
        use super::GuestMem;
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).expect("an arena");

        // Aligned and unaligned addresses, including every straddling boundary across 0x2000.
        for addr in [0x1000u32, 0x1001, 0x1fff] {
            mem.write_u8(addr, 0xa5).unwrap();
            assert_eq!(mem.read_u8(addr).unwrap(), 0xa5);
        }
        for addr in [0x1000u32, 0x1001, 0x1ffe, 0x1fff] {
            mem.write_u16(addr, 0x1234).unwrap();
            assert_eq!(mem.read_u16(addr).unwrap(), 0x1234);
            assert_eq!(mem.slice(addr, 2).unwrap(), 0x1234u16.to_le_bytes());
        }
        for addr in [0x1000u32, 0x1003, 0x1ffc, 0x1ffd, 0x1ffe, 0x1fff] {
            mem.write_u32(addr, 0x89ab_cdef).unwrap();
            assert_eq!(mem.read_u32(addr).unwrap(), 0x89ab_cdef);
            assert_eq!(mem.slice(addr, 4).unwrap(), 0x89ab_cdefu32.to_le_bytes());
        }
        for addr in [0x1000u32, 0x1005, 0x1ff8, 0x1ff9, 0x1ffc, 0x1fff] {
            let v = 0x0123_4567_89ab_cdefu64;
            mem.write_u64(addr, v).unwrap();
            assert_eq!(mem.read_u64(addr).unwrap(), v);
            assert_eq!(mem.slice(addr, 8).unwrap(), v.to_le_bytes());
        }
    }

    #[test]
    fn wrote_over_code_handles_single_page_and_multi_page_writes() {
        use super::GuestMem;
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).expect("an arena");
        mem.mark_code(0x2000);

        // Single-page write on unmarked page stays quiet.
        mem.write_u32(0x1ffc, 0x1111_1111).unwrap();
        assert!(!mem.code_written);

        // Single-page write on marked page 2 triggers invalidation.
        mem.write_u32(0x2000, 0x2222_2222).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [2]);

        // Reset, then cross from unmarked page 1 into marked page 2.
        mem.code_written = false;
        mem.written_code.clear();
        mem.write_u32(0x1ffd, 0x3333_3333).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [1, 2]);

        // Reset, then cross from marked page 2 into unmarked page 3 via write_u64.
        mem.code_written = false;
        mem.written_code.clear();
        mem.write_u64(0x2ffc, 0x4444_5555_6666_7777).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [2, 3]);

        // Multi-page write spanning unmarked -> marked -> unmarked.
        mem.code_written = false;
        mem.written_code.clear();
        mem.write(0x1800, &[0u8; 0x2000]).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [1, 2, 3]);

        // Unmarking silences further writes to page 2.
        mem.unmark_code(0x2000);
        mem.code_written = false;
        mem.written_code.clear();
        mem.write_u64(0x1ffc, 0).unwrap();
        assert!(!mem.code_written);
    }

    #[test]
    fn scalar_faults_report_exact_addr_len_and_write_for_floor_and_end() {
        use super::{Fault, GuestMem, DOS_MEMORY, NULL_PAGE};
        let len = 1 << 21;
        let end = u32::try_from(len).unwrap();
        let mut mem = Arena::new(&mut Headless::new(), len)
            .expect("an arena")
            .with_floor(DOS_MEMORY);

        assert_eq!(
            mem.read_u8(NULL_PAGE - 1),
            Err(Fault {
                addr: NULL_PAGE - 1,
                len: 1,
                write: false,
            })
        );
        assert_eq!(
            mem.write_u16(DOS_MEMORY - 1, 1),
            Err(Fault {
                addr: DOS_MEMORY - 1,
                len: 2,
                write: true,
            })
        );
        assert_eq!(
            mem.read_u32(DOS_MEMORY - 2),
            Err(Fault {
                addr: DOS_MEMORY - 2,
                len: 4,
                write: false,
            })
        );
        assert_eq!(
            mem.write_u64(DOS_MEMORY - 4, 1),
            Err(Fault {
                addr: DOS_MEMORY - 4,
                len: 8,
                write: true,
            })
        );
        assert_eq!(
            mem.read_u64(end - 4),
            Err(Fault {
                addr: end - 4,
                len: 8,
                write: false,
            })
        );
        assert_eq!(
            mem.write_u32(end - 2, 1),
            Err(Fault {
                addr: end - 2,
                len: 4,
                write: true,
            })
        );
        assert_eq!(
            mem.read_u32(u32::MAX),
            Err(Fault {
                addr: u32::MAX,
                len: 4,
                write: false,
            })
        );
        assert_eq!(
            mem.write_u64(u32::MAX - 2, 0),
            Err(Fault {
                addr: u32::MAX - 2,
                len: 8,
                write: true,
            })
        );

        // Exact boundaries remain valid.
        mem.write_u64(DOS_MEMORY, 0xdead_beef_cafe_babe).unwrap();
        assert_eq!(mem.read_u64(DOS_MEMORY).unwrap(), 0xdead_beef_cafe_babe);
        mem.write_u64(end - 8, 0x1122_3344_5566_7788).unwrap();
        assert_eq!(mem.read_u64(end - 8).unwrap(), 0x1122_3344_5566_7788);
    }

    #[test]
    fn scalar_writes_preserve_clock_spin_watch_semantics() {
        use super::GuestMem;
        let mut mem = Arena::new(&mut Headless::new(), 1 << 16).expect("an arena");
        mem.write_u32(0x1000, 0xaaaa_bbbb).unwrap();

        // Identical writes never consume held slots or break the watch.
        mem.watch();
        for _ in 0..32 {
            mem.write_u32(0x1000, 0xaaaa_bbbb).unwrap();
        }
        assert!(mem.unchanged());

        // Writing a new value then restoring the original stays unchanged.
        mem.watch();
        mem.write_u32(0x1000, 0x1234_5678).unwrap();
        mem.write_u32(0x1000, 0xaaaa_bbbb).unwrap();
        assert!(mem.unchanged());

        // Writing a different value without restoring marks memory changed.
        mem.watch();
        mem.write_u64(0x1000, 0xdead_beef_0000_0001).unwrap();
        assert!(!mem.unchanged());
    }

    #[test]
    fn wrote_over_code_multi_word_wise_scan_matches_bit_oracle() {
        // 256 pages = 1 MiB, spanning 4 full u64 bitmap words (pages 0..256).
        let pages = 256usize;
        let mut mem = Arena::new(&mut Headless::new(), pages * 4096).expect("an arena");

        let check = |mem: &Arena, first_page: usize, last_page: usize, marked: Option<usize>| {
            let addr = (first_page * 4096) as u32;
            let len = (last_page - first_page + 1) * 4096;
            let expected = marked.is_some_and(|m| (first_page..=last_page).contains(&m));
            assert_eq!(
                mem.wrote_over_code_multi(first_page, addr, len),
                expected,
                "range {first_page}..={last_page}, marked {marked:?}"
            );
        };

        // Test same-word, adjacent-word (straddling 63/64), and multi-word
        // ranges (spanning 3+ words, like a 150-page 640x480x16 framebuffer).
        let ranges = [
            (0, 1),
            (5, 62),
            (0, 63),
            (63, 64),
            (62, 65),
            (30, 100),
            (63, 128),
            (10, 160),
            (0, 255),
            (128, 255),
        ];

        // Empty bitmap: every range is false.
        for &(a, b) in &ranges {
            check(&mem, a, b, None);
        }

        // Probe every boundary bit: word edges (0, 63, 64, 127, 128, 191, 192, 255)
        // plus interior bits (1, 10, 62, 65, 100, 159, 160, 161).
        for bit in [
            0, 1, 5, 10, 29, 30, 62, 63, 64, 65, 100, 101, 127, 128, 129, 159, 160, 161, 191, 192,
            254, 255,
        ] {
            mem.mark_code((bit * 4096) as u32);
            for &(a, b) in &ranges {
                check(&mem, a, b, Some(bit));
            }
            mem.unmark_code((bit * 4096) as u32);
        }

        // Unaligned sub-page start/end spanning adjacent and multi-word boundaries.
        mem.mark_code(64 * 4096);
        assert!(mem.wrote_over_code(63 * 4096 + 0xfff, 2));
        assert!(!mem.wrote_over_code(63 * 4096 + 0xfff, 1));
        assert!(mem.wrote_over_code(10 * 4096 + 123, 150 * 4096));
        mem.unmark_code(64 * 4096);
        assert!(!mem.wrote_over_code(10 * 4096 + 123, 150 * 4096));
    }

    #[test]
    fn code_bbox_tracks_mark_unmark_clear_and_straddling_writes() {
        use super::GuestMem;
        let mut mem = Arena::new(&mut Headless::new(), 1 << 20).expect("an arena");
        assert_eq!(mem.code_bbox(), (u32::MAX, 0));

        mem.mark_code(0x0002_0123);
        assert_eq!(mem.code_bbox(), (0x0002_0000, 0x0002_1000));

        mem.mark_code(0x0005_0000);
        assert_eq!(mem.code_bbox(), (0x0002_0000, 0x0005_1000));

        mem.mark_code(0x0003_0000);
        assert_eq!(mem.code_bbox(), (0x0002_0000, 0x0005_1000));

        // Unmarking interior page keeps bbox unchanged.
        mem.unmark_code(0x0003_0000);
        assert_eq!(mem.code_bbox(), (0x0002_0000, 0x0005_1000));

        // Writes strictly outside [code_min, code_max) never trigger.
        mem.write_u32(0x0001_fffc, 0xdead_beef).unwrap();
        mem.write_u64(0x0001_fff8, 0x0123_4567_89ab_cdef).unwrap();
        mem.write_u32(0x0005_1000, 0x1122_3344).unwrap();
        assert!(!mem.code_written);

        // Multi-byte writes straddling code_min - 1 and code_max - 1 MUST trigger.
        mem.write_u16(0x0001_ffff, 0x1234).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [0x1f, 0x20]);
        mem.code_written = false;
        mem.written_code.clear();

        mem.write_u32(0x0001_fffd, 0x1234_5678).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [0x1f, 0x20]);
        mem.code_written = false;
        mem.written_code.clear();

        mem.write_u64(0x0001_fff9, 0x0102_0304_0506_0708).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [0x1f, 0x20]);
        mem.code_written = false;
        mem.written_code.clear();

        mem.write_u32(0x0005_0ffe, 0x8765_4321).unwrap();
        assert!(mem.code_written);
        assert_eq!(mem.written_code, [0x50, 0x51]);
        mem.code_written = false;
        mem.written_code.clear();

        // Unmarking min tightens code_min; unmarking max resets to empty.
        mem.unmark_code(0x0002_0000);
        assert_eq!(mem.code_bbox(), (0x0005_0000, 0x0005_1000));
        mem.unmark_code(0x0005_0000);
        assert_eq!(mem.code_bbox(), (u32::MAX, 0));

        // clear_code wipes bitmap and resets bbox to empty.
        mem.mark_code(0x0001_0000);
        mem.mark_code(0x0008_0000);
        assert_eq!(mem.code_bbox(), (0x0001_0000, 0x0008_1000));
        mem.clear_code();
        assert_eq!(mem.code_bbox(), (u32::MAX, 0));
        mem.write_u32(0x0001_0000, 1).unwrap();
        mem.write_u32(0x0008_0000, 2).unwrap();
        assert!(!mem.code_written);
    }
}
