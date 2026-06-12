//! Guest-facing PPU thread id and its monotonic allocator.

/// Guest-facing PPU thread id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PpuThreadId(u64);

impl PpuThreadId {
    /// The primary thread's reserved id; [`PpuThreadIdAllocator`]
    /// never returns this value.
    pub const PRIMARY: PpuThreadId = PpuThreadId(0x0100_0000);

    /// Construct a thread id from a raw u64.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Raw u64 value.
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Monotonic allocator for child-thread ids.
#[derive(Debug, Clone)]
pub struct PpuThreadIdAllocator {
    /// `None` means exhausted. Storing the current slot (rather
    /// than eagerly reserving `next + 1`) keeps `u64::MAX`
    /// reachable as the last hand-out.
    next: Option<u64>,
}

impl PpuThreadIdAllocator {
    /// Construct a fresh allocator; first `allocate` returns
    /// `PpuThreadId(0x0100_0001)`.
    pub fn new() -> Self {
        Self {
            next: Some(PpuThreadId::PRIMARY.raw() + 1),
        }
    }

    /// Allocate the next thread id; `None` only after `u64::MAX`
    /// has itself been handed out.
    pub fn allocate(&mut self) -> Option<PpuThreadId> {
        let cur = self.next?;
        self.next = cur.checked_add(1);
        Some(PpuThreadId(cur))
    }

    /// Peek the id that the next `allocate` would return.
    pub fn peek(&self) -> Option<PpuThreadId> {
        self.next.map(PpuThreadId)
    }
}

impl Default for PpuThreadIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/id_tests.rs"]
mod tests;
