//! The guest's heap.
//!
//! A title asks the machine for memory and gets an address it can write to.
//! What it gets back is real guest memory in a region declared for the
//! purpose, so a pointer the guest hands to a shim is an address the shim can
//! read.

use std::collections::BTreeMap;

/// Where allocations come from, and how much there is.
///
/// Declared values: the address is chosen so that it sits above every image
/// the roster's titles ask to be loaded at and below the boundary's own range.
pub const HEAP_BASE: u32 = 0x1000_0000;
/// Half a gigabyte. Rock Raiders alone reads 79 MB of archives into memory on
/// the way to its front end, and a heap that runs out hands back zero, which a
/// guest happily uses as an address.
pub const HEAP_BYTES: u32 = 0x2000_0000;

/// A bump allocator with a free list.
///
// ponytail: first fit over a map of freed blocks, and freed memory is only
// reused whole. A title that thrashes small allocations will fragment it;
// measure before writing anything cleverer.
#[derive(Debug)]
pub struct Heap {
    next: u32,
    end: u32,
    /// What is live, by address, with its size.
    live: BTreeMap<u32, u32>,
    /// What has been handed back, by size, newest first.
    free: Vec<(u32, u32)>,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: HEAP_BASE,
            end: HEAP_BASE + HEAP_BYTES,
            live: BTreeMap::new(),
            free: Vec::new(),
        }
    }

    /// Hand out `size` bytes, aligned the way a Win32 heap aligns.
    ///
    /// Returns zero when the heap is exhausted, which is what the guest reads
    /// as a failed allocation.
    pub fn alloc(&mut self, size: u32) -> u32 {
        self.alloc_aligned(size, 8)
    }

    /// Hand out `size` bytes starting on a multiple of `align`, which must be
    /// a power of two.
    ///
    /// Returns zero when the heap is exhausted.
    pub fn alloc_aligned(&mut self, size: u32, align: u32) -> u32 {
        // A guest asking for more than the address space holds gets a refusal,
        // not a panic: rounding it up must not be the thing that overflows.
        let Some(size) = size.max(1).checked_next_multiple_of(align) else {
            return 0;
        };
        if let Some(i) = self
            .free
            .iter()
            .position(|(addr, s)| *s >= size && addr % align == 0)
        {
            let (addr, whole) = self.free.remove(i);
            self.live.insert(addr, whole);
            return addr;
        }
        // ponytail: whatever lies between here and the next boundary is never
        // handed out, a few kilobytes per region a title reserves. A title
        // that reserves thousands of small regions would want the gap on the
        // free list.
        let Some(addr) = self.next.checked_next_multiple_of(align) else {
            return 0;
        };
        if addr.saturating_add(size) > self.end {
            return 0;
        }
        self.next = addr + size;
        self.live.insert(addr, size);
        addr
    }

    /// How big a live allocation is.
    #[must_use]
    pub fn size_of(&self, addr: u32) -> Option<u32> {
        self.live.get(&addr).copied()
    }

    /// Hand memory back. Freeing something that was never handed out is worth
    /// knowing about, so it is reported rather than ignored.
    pub fn free(&mut self, addr: u32) -> bool {
        match self.live.remove(&addr) {
            Some(size) => {
                self.free.push((addr, size));
                true
            }
            None => false,
        }
    }

    /// How far the heap has grown: everything below this has been handed out
    /// at least once, and is what a state digest has to cover.
    #[must_use]
    pub fn extent(&self) -> u32 {
        self.next - HEAP_BASE
    }

    /// How much has been handed out and not given back.
    #[must_use]
    pub fn live_bytes(&self) -> u32 {
        self.live.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::{Heap, HEAP_BASE};

    #[test]
    fn allocations_do_not_overlap_and_freed_blocks_come_back() {
        let mut h = Heap::new();
        let a = h.alloc(100);
        let b = h.alloc(100);
        assert_eq!(a, HEAP_BASE);
        assert!(b >= a + 100, "a second allocation starts past the first");
        assert_eq!(
            h.size_of(a),
            Some(104),
            "sizes are rounded up to the heap's alignment"
        );

        assert!(h.free(a));
        assert!(
            !h.free(a),
            "freeing twice is refused rather than silently accepted"
        );
        assert_eq!(h.alloc(100), a, "a freed block is handed out again");
    }

    #[test]
    fn an_aligned_allocation_starts_on_its_boundary_however_the_heap_stands() {
        let mut h = Heap::new();
        h.alloc(12);
        let region = h.alloc_aligned(0x40_0000, 0x1_0000);
        assert_eq!(region % 0x1_0000, 0, "{region:#x} is not on a boundary");
        assert!(region > HEAP_BASE, "it does not overlap what came before");
        let next = h.alloc(8);
        assert!(next >= region + 0x40_0000, "nor what comes after");
    }

    #[test]
    fn an_exhausted_heap_returns_zero_rather_than_an_address() {
        let mut h = Heap::new();
        assert_eq!(h.alloc(u32::MAX), 0);
        assert_eq!(h.live_bytes(), 0);
    }
}
