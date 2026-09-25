//! Child-thread stack region and its deterministic bump allocator.

/// Bytes the PPC64 ELFv1 ABI reserves below the initial `r1`.
///
/// Re-exported so a child thread's reserve and the primary's are one
/// value: the ABI crate owns the fact and carries its citation.
pub use cellgov_ps3_abi::hw::address_space::PS3_ABI_MIN_STACK_FRAME as ABI_MIN_STACK_FRAME;

/// A reserved stack block for a child PPU thread.
///
/// Construction enforces `size >= ABI_MIN_STACK_FRAME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadStack {
    pub(crate) base: u64,
    pub(crate) size: u64,
}

impl ThreadStack {
    /// Construct a stack block.
    ///
    /// # Panics
    /// If `size < ABI_MIN_STACK_FRAME`.
    pub fn new(base: u64, size: u64) -> Self {
        assert!(
            size >= ABI_MIN_STACK_FRAME,
            "ThreadStack::new: size {size} < {ABI_MIN_STACK_FRAME} would underflow initial_sp",
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
    /// A whole minimum frame sits above it, so the top of the block
    /// is never itself the SP.
    ///
    /// [CBE-Handbook p:396 s:14.3.1.3] The loader hands the entry
    /// point an R1 that is quadword-aligned and already points at a
    /// reserved initial frame whose back chain is null.
    ///
    /// CellGov seeds every PPU thread's `r1` that way, not only the
    /// primary one.
    ///
    /// # Panics
    /// Debug-only if `size < ABI_MIN_STACK_FRAME`; unreachable via
    /// `new` or the allocator.
    pub fn initial_sp(&self) -> u64 {
        debug_assert!(
            self.size >= ABI_MIN_STACK_FRAME,
            "ThreadStack::initial_sp: size {} < {ABI_MIN_STACK_FRAME} would underflow",
            self.size,
        );
        self.base + self.size - ABI_MIN_STACK_FRAME
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
    /// [`cellgov_ps3_abi::hw::address_space::PS3_CHILD_STACKS_BASE`].
    pub const CHILD_STACK_BASE: u64 = 0xD010_0000;

    /// Construct a fresh allocator.
    pub fn new() -> Self {
        Self {
            next: Self::CHILD_STACK_BASE,
        }
    }

    /// Allocate a stack block of `size` bytes, aligned to
    /// `max(align, 16)`; `None` on overflow or
    /// `size < ABI_MIN_STACK_FRAME`.
    pub fn allocate(&mut self, size: u64, align: u64) -> Option<ThreadStack> {
        if size < ABI_MIN_STACK_FRAME {
            return None;
        }
        let align = align.max(0x10);
        let mask = align - 1;
        let base = self.next.checked_add(mask)? & !mask;
        let end = base.checked_add(size)?;
        self.next = end;
        Some(ThreadStack { base, size })
    }

    /// Return `stack` iff it is the most recent allocation, rewinding
    /// the bump pointer to its base so the next allocation reuses it.
    ///
    /// The single caller is the create-refusal path, which frees the
    /// block it just allocated before anything else can allocate;
    /// `false` (not the last block) means that flow was broken.
    ///
    /// A base below [`Self::CHILD_STACK_BASE`] or a wrapping
    /// `base + size` draws the same `false`: rewinding under the
    /// arena floor would hand out blocks overlapping the primary
    /// thread's stack.
    pub fn free_last(&mut self, stack: &ThreadStack) -> bool {
        let Some(end) = stack.base.checked_add(stack.size) else {
            return false;
        };
        if end != self.next || stack.base < Self::CHILD_STACK_BASE {
            return false;
        }
        self.next = stack.base;
        true
    }

    /// Peek the next allocation's base for the given alignment
    /// without advancing the allocator.
    pub fn peek_next(&self, align: u64) -> Option<u64> {
        let align = align.max(0x10);
        let mask = align - 1;
        self.next.checked_add(mask).map(|n| n & !mask)
    }
}

impl ThreadStackAllocator {
    /// The allocator's term of the sync-state sum: its cursor XOR
    /// [`Self::CHILD_STACK_BASE`], so a fresh allocator's lane is zero.
    pub(crate) fn sync_term(&self) -> u128 {
        cellgov_mem::lanes::value_term(
            cellgov_mem::lanes::source::THREAD_STACKS,
            0,
            &(self.next ^ Self::CHILD_STACK_BASE),
        )
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
