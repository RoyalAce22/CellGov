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
    /// above the primary thread's 1 MiB stack at
    /// `0xD0000000..0xD0100000`. Tracks
    /// [`cellgov_ps3_abi::process_address_space::PS3_CHILD_STACKS_BASE`].
    pub const CHILD_STACK_BASE: u64 = 0xD010_0000;

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
#[path = "tests/stack_tests.rs"]
mod tests;
