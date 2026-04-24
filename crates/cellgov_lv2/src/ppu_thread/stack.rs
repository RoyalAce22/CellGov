//! Child-thread stack region and its deterministic bump allocator.

/// A reserved stack block for a child PPU thread.
///
/// Construction enforces `size >= 0x10` (the ABI-required
/// save-area reserve below `r1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadStack {
    pub(crate) base: u64,
    pub(crate) size: u64,
}

impl ThreadStack {
    /// Construct a stack block.
    ///
    /// # Panics
    /// If `size < 0x10`.
    pub fn new(base: u64, size: u64) -> Self {
        assert!(
            size >= 0x10,
            "ThreadStack::new: size {size} < 0x10 would underflow initial_sp",
        );
        Self { base, size }
    }

    /// Lowest address of the reserved block (inclusive).
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Block size in bytes.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Top-of-stack address to load into `r1`.
    ///
    /// # Panics
    /// Debug-only if `size < 0x10`; unreachable via `new` or the
    /// allocator.
    pub fn initial_sp(&self) -> u64 {
        debug_assert!(
            self.size >= 0x10,
            "ThreadStack::initial_sp: size {} < 0x10 would underflow",
            self.size,
        );
        self.base + self.size - 0x10
    }

    /// Upper bound of the reserved block (exclusive).
    pub fn end(&self) -> u64 {
        self.base + self.size
    }
}

/// Deterministic bump allocator for child-thread stacks.
#[derive(Debug, Clone)]
pub struct ThreadStackAllocator {
    next: u64,
}

impl ThreadStackAllocator {
    /// Lowest address the allocator will hand out; sits directly
    /// above the primary thread's 64 KB stack at
    /// `0xD0000000..0xD0010000`.
    pub const CHILD_STACK_BASE: u64 = 0xD001_0000;

    /// Construct a fresh allocator.
    pub fn new() -> Self {
        Self {
            next: Self::CHILD_STACK_BASE,
        }
    }

    /// Allocate a stack block of `size` bytes, aligned to
    /// `max(align, 16)`; `None` on overflow or `size < 0x10`.
    pub fn allocate(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        if size < 0x10 {
            return None;
        }
        let align = align.max(0x10);
        let mask = align - 1;
        let base = self.next.checked_add(mask)? & !mask;
        let end = base.checked_add(size)?;
        self.next = end;
        Some(ThreadStack { base, size })
    }

    /// Peek the next allocation's base for the given alignment
    /// without advancing the allocator.
    pub fn peek_next(&self, align: u64) -> Option<u64> {
        let align = align.max(0x10);
        let mask = align - 1;
        self.next.checked_add(mask).map(|n| n & !mask)
    }
}

impl Default for ThreadStackAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_allocator_three_children_non_overlapping() {
        let mut a = ThreadStackAllocator::new();
        let s1 = a.allocate(0x10_000, 0x10).unwrap();
        let s2 = a.allocate(0x10_000, 0x10).unwrap();
        let s3 = a.allocate(0x10_000, 0x10).unwrap();
        assert_eq!(s1.base, ThreadStackAllocator::CHILD_STACK_BASE);
        assert!(s2.base >= s1.end());
        assert!(s3.base >= s2.end());
        assert_ne!(s1.base, s2.base);
        assert_ne!(s2.base, s3.base);
        assert!(s1.base > 0xD000_FFFF);
    }

    #[test]
    fn stack_allocator_is_deterministic_across_instances() {
        let mut a = ThreadStackAllocator::new();
        let mut b = ThreadStackAllocator::new();
        for _ in 0..4 {
            assert_eq!(a.allocate(0x10_000, 0x10), b.allocate(0x10_000, 0x10));
        }
    }

    #[test]
    fn stack_allocator_honors_alignment() {
        let mut a = ThreadStackAllocator::new();
        let _ = a.allocate(0x4321, 0x10).unwrap();
        let s = a.allocate(0x1000, 0x1000).unwrap();
        assert_eq!(s.base & 0xFFF, 0, "base not 4KB-aligned");
    }

    #[test]
    fn stack_allocator_minimum_alignment_is_16_bytes() {
        let mut a = ThreadStackAllocator::new();
        let s = a.allocate(0x100, 0).unwrap();
        assert_eq!(s.base & 0xF, 0);
    }

    #[test]
    fn stack_allocator_rejects_zero_size() {
        let mut a = ThreadStackAllocator::new();
        assert!(a.allocate(0, 0x10).is_none());
    }

    #[test]
    fn stack_allocator_rejects_size_below_save_area() {
        let mut a = ThreadStackAllocator::new();
        assert!(a.allocate(0x8, 0x10).is_none());
        assert!(a.allocate(0xF, 0x10).is_none());
    }

    #[test]
    fn thread_stack_initial_sp_leaves_16_byte_reserve() {
        let s = ThreadStack {
            base: 0xD001_0000,
            size: 0x10_000,
        };
        assert_eq!(s.initial_sp(), 0xD002_0000 - 0x10);
        assert_eq!(s.end(), 0xD002_0000);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "would underflow")]
    fn thread_stack_initial_sp_debug_asserts_on_tiny_size() {
        let s = ThreadStack {
            base: 0x1000,
            size: 0x8,
        };
        let _ = s.initial_sp();
    }

    #[test]
    fn stack_allocator_returns_none_on_overflow() {
        let mut a = ThreadStackAllocator {
            next: u64::MAX - 0x100,
        };
        assert!(a.allocate(0x1000, 0x10).is_none());
    }
}
