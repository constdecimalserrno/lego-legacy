//! The guest's heap.
//!
//! A title asks the machine for memory and gets an address it can write to.
//! What it gets back is real guest memory in a region declared for the
//! purpose, so a pointer the guest hands to a shim is an address the shim can
//! read.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};

/// Deterministic fast hasher for 32-bit guest heap addresses.
///
/// Heap pointers are multiples of eight; a 64-bit Fibonacci multiply followed
/// by a high-to-low xor spreads both the low bucket bits (`h1`) and top tag
/// bits (`h2`) without reading any host entropy.
#[derive(Default)]
struct AddrHasher(u64);

impl Hasher for AddrHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.wrapping_mul(0x517c_c1b7_2722_0a95) ^ u64::from(b);
        }
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        let x = u64::from(i).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 = x ^ (x >> 32);
    }
}

/// Where allocations come from, and how much there is.
///
/// Declared values. The address sits above every image the roster's titles ask
/// to be loaded at, and clear of `0x1000_0000`: that is where a DLL asks to be
/// loaded by default, and Alpha Team's archive reader commits three quarters
/// of a megabyte there by name. A heap that started at the same place would
/// hand the same bytes out twice (#57).
pub const HEAP_BASE: u32 = 0x2000_0000;
/// Half a gigabyte. Rock Raiders alone reads 79 MB of archives into memory on
/// the way to its front end, and a heap that runs out hands back zero, which a
/// guest happily uses as an address.
pub const HEAP_BYTES: u32 = 0x2000_0000;

/// A bump allocator with a free list, kept by size.
///
/// Freed memory is still only reused whole — a block handed back is handed out
/// again at its own size or not at all — but the block chosen is the smallest
/// one that will do, found by a lookup rather than a walk.
//
// A lookup rather than a scan of every free block, because a title makes that
// the cost that shows (#80): Alpha Team's front end allocates 397,000 times
// over its menu route, and with the C runtime's heap written against this one
// a scan costs more than the interpreted allocator it replaces. Measured: 51.8
// flips a second with a scan, 52.9 with this, against 52.2 with the runtime's
// own.
//
// ponytail: a freed block is never split and neighbours are never joined, so a
// run that asks for one size and then a larger one for ever still grows. The
// splitting is the next thing to write if a title makes that show.
#[derive(Debug)]
pub struct Heap {
    next: u32,
    end: u32,
    /// What is live, by address, with its size. Never iterated in order
    /// (digests and snapshots cover `HEAP_BASE..next` directly).
    live: HashMap<u32, u32, BuildHasherDefault<AddrHasher>>,
    /// What has been handed back, by size: for each size, the addresses of
    /// the blocks of it that are free, kept in FIFO order.
    free: BTreeMap<u32, VecDeque<u32>>,
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
            live: HashMap::default(),
            free: BTreeMap::new(),
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
        // The smallest free block that is big enough and starts where the
        // caller needs it to. Every address this hands out is a multiple of
        // eight, so anything asking for no more than that takes the front
        // candidate in O(1) FIFO order; a wider alignment is rare and looks
        // past the ones that do not suit it while preserving FIFO order.
        let mut found = None;
        for (whole, addrs) in self.free.range_mut(size..) {
            if addrs.front().is_some_and(|&addr| addr % align == 0) {
                let addr = addrs.pop_front().expect("checked non-empty");
                found = Some((*whole, addr, addrs.is_empty()));
                break;
            }
            if let Some(i) = addrs.iter().position(|&addr| addr % align == 0) {
                let addr = addrs.remove(i).expect("index from position");
                found = Some((*whole, addr, addrs.is_empty()));
                break;
            }
        }
        if let Some((whole, addr, emptied)) = found {
            if emptied {
                self.free.remove(&whole);
            }
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
                self.free.entry(size).or_default().push_back(addr);
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

    #[test]
    fn freed_blocks_are_reused_in_strict_fifo_order() {
        let mut h = Heap::new();
        let a = h.alloc(64);
        let b = h.alloc(64);
        let c = h.alloc(64);
        let d = h.alloc(64);
        // Free in a deliberate non-monotonic order: c, a, d, b.
        assert!(h.free(c));
        assert!(h.free(a));
        assert!(h.free(d));
        assert!(h.free(b));
        assert_eq!(h.live_bytes(), 0);
        assert_eq!(h.size_of(a), None);

        // Must come back in exact FIFO order: c, a, d, b.
        assert_eq!(h.alloc(64), c);
        assert_eq!(h.alloc(64), a);
        assert_eq!(h.alloc(64), d);
        assert_eq!(h.alloc(64), b);
        assert_eq!(h.live_bytes(), 256);

        // Wider alignment skipping unaligned front entries must still preserve
        // FIFO order among the remaining entries.
        let mut h2 = Heap::new();
        let _pad = h2.alloc(8); // HEAP_BASE (aligned to 64)
        let u1 = h2.alloc(64); // +8  (not 64-byte aligned)
        let u2 = h2.alloc(64); // +72 (not 64-byte aligned)
        let al = h2.alloc_aligned(64, 64); // 64-byte aligned
        let u3 = h2.alloc(64); // +64 (64-byte aligned)
        let u4 = h2.alloc(8);
        let u5 = h2.alloc(64); // not 64-byte aligned
        let _ = u4;
        assert!(h2.free(u1));
        assert!(h2.free(u2));
        assert!(h2.free(al));
        assert!(h2.free(u5));
        assert!(h2.free(u3));

        // Aligned allocation pulls `al` out of the middle, leaving [u1, u2, u5, u3].
        assert_eq!(h2.alloc_aligned(64, 64), al);
        // Subsequent 8-byte-aligned allocations drain the remaining queue in FIFO order.
        assert_eq!(h2.alloc(64), u1);
        assert_eq!(h2.alloc(64), u2);
        assert_eq!(h2.alloc(64), u5);
        assert_eq!(h2.alloc(64), u3);
    }
}
