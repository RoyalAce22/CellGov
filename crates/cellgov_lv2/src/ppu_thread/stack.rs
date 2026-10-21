//! Child-thread stack region and the placement of a new thread's stack
//! in it.

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

/// Lowest address of the child-stack arena; sits directly above the
/// primary thread's stack region.
pub const CHILD_STACK_BASE: u64 = cellgov_ps3_abi::hw::address_space::PS3_CHILD_STACKS_BASE;

/// End (exclusive) of the child-stack arena: the end of the stack
/// segment.
pub const CHILD_STACK_END: u64 =
    CHILD_STACK_BASE + cellgov_ps3_abi::hw::address_space::PS3_CHILD_STACKS_SIZE as u64;

/// Place a stack block of `size` bytes, aligned to `max(align, 16)`,
/// at the lowest address of the child-stack arena where it overlaps no
/// block of `live`.
///
/// `live` holds the `(base, size)` stack of every thread the kernel has
/// not collected. A block outside the arena, such as the primary
/// thread's, does not constrain the result. The placement depends only
/// on `live`, so a collected thread's range is free for the next
/// create, and two runs with the same live set place the same block.
///
/// `None` when no free range of the arena holds the block, or when
/// `size < ABI_MIN_STACK_FRAME`.
pub fn place_child_stack(
    live: impl IntoIterator<Item = (u64, u64)>,
    size: u64,
    align: u64,
) -> Option<ThreadStack> {
    if size < ABI_MIN_STACK_FRAME {
        return None;
    }
    let mask = align.max(0x10) - 1;
    let mut held: Vec<(u64, u64)> = live
        .into_iter()
        .filter(|&(base, len)| len > 0 && base < CHILD_STACK_END)
        .map(|(base, len)| (base, base.saturating_add(len)))
        .filter(|&(_, end)| end > CHILD_STACK_BASE)
        .collect();
    held.sort_unstable();
    let mut cursor = CHILD_STACK_BASE;
    for (start, end) in held
        .into_iter()
        .chain(std::iter::once((CHILD_STACK_END, CHILD_STACK_END)))
    {
        let base = cursor.checked_add(mask)? & !mask;
        if base.checked_add(size)? <= start {
            return Some(ThreadStack { base, size });
        }
        cursor = cursor.max(end);
    }
    None
}

#[cfg(test)]
#[path = "tests/stack_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/child_stack_console_tests.rs"]
mod child_stack_console_tests;
