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
pub const HEAP_BASE: u32 = 0x0100_0000;
pub const HEAP_BYTES: u32 = 0x0400_0000;

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
        // A guest asking for more than the address space holds gets a refusal,
        // not a panic: rounding it up must not be the thing that overflows.
        let Some(size) = size.max(1).checked_next_multiple_of(8) else {
            return 0;
        };
        if let Some(i) = self.free.iter().position(|(_, s)| *s >= size) {
            let (addr, whole) = self.free.remove(i);
            self.live.insert(addr, whole);
            return addr;
        }
        if self.next.saturating_add(size) > self.end {
            return 0;
        }
        let addr = self.next;
        self.next += size;
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
    fn an_exhausted_heap_returns_zero_rather_than_an_address() {
        let mut h = Heap::new();
        assert_eq!(h.alloc(u32::MAX), 0);
        assert_eq!(h.live_bytes(), 0);
    }
}
