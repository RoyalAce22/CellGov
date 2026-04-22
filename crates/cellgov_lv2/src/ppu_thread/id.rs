//! Guest-facing PPU thread id and its monotonic allocator.

/// Guest-facing PPU thread id. Values below `0x0100_0000` are
/// reserved; the primary is [`Self::PRIMARY`] and child threads
/// allocate from `0x0100_0001` upward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PpuThreadId(u64);

impl PpuThreadId {
    /// The primary thread's id. Reserved -- the allocator never
    /// returns this value.
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
///
/// `peek` and `allocate` agree: `peek` returns `Some(id)` iff
/// the next `allocate` will also return `Some(id)` for the
/// same `id`, including when `id == u64::MAX`.
#[derive(Debug, Clone)]
pub struct PpuThreadIdAllocator {
    /// `None` means exhausted. Storing the post-state this way
    /// (rather than eagerly reserving `next + 1`) is what makes
    /// `u64::MAX` itself reachable.
    next: Option<u64>,
}

impl PpuThreadIdAllocator {
    /// Construct a fresh allocator. The first `allocate` returns
    /// `PpuThreadId(0x0100_0001)`.
    pub fn new() -> Self {
        Self {
            next: Some(PpuThreadId::PRIMARY.raw() + 1),
        }
    }

    /// Allocate the next thread id. Returns `None` only after
    /// `u64::MAX` has itself been handed out.
    pub fn allocate(&mut self) -> Option<PpuThreadId> {
        let cur = self.next?;
        self.next = cur.checked_add(1);
        Some(PpuThreadId(cur))
    }

    /// Peek the id that the next `allocate` will return.
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
impl PpuThreadIdAllocator {
    /// Seed the next-id directly to exercise exhaustion near
    /// `u64::MAX` without allocating 2^64 times.
    pub(crate) fn with_next(next: u64) -> Self {
        Self { next: Some(next) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_id_is_reserved_value() {
        assert_eq!(PpuThreadId::PRIMARY.raw(), 0x0100_0000);
    }

    #[test]
    fn fresh_allocator_starts_above_primary() {
        let mut a = PpuThreadIdAllocator::new();
        let first = a.allocate().unwrap();
        assert_eq!(first.raw(), 0x0100_0001);
        assert_ne!(first, PpuThreadId::PRIMARY);
    }

    #[test]
    fn allocator_is_monotonic() {
        let mut a = PpuThreadIdAllocator::new();
        let ids: Vec<_> = (0..8).map(|_| a.allocate().unwrap()).collect();
        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert_eq!(ids[0].raw(), 0x0100_0001);
        assert_eq!(ids[7].raw(), 0x0100_0008);
    }

    #[test]
    fn allocator_never_returns_primary() {
        let mut a = PpuThreadIdAllocator::new();
        for _ in 0..10_000 {
            let id = a.allocate().unwrap();
            assert_ne!(id, PpuThreadId::PRIMARY);
        }
    }

    #[test]
    fn allocator_can_hand_out_u64_max_slot() {
        let mut a = PpuThreadIdAllocator::with_next(u64::MAX);
        let last = a.allocate().expect("u64::MAX is a valid slot");
        assert_eq!(last.raw(), u64::MAX);
        assert!(a.allocate().is_none());
        assert!(a.allocate().is_none());
    }

    #[test]
    fn peek_agrees_with_allocate() {
        let mut a = PpuThreadIdAllocator::with_next(u64::MAX - 1);
        assert_eq!(a.peek().map(|id| id.raw()), Some(u64::MAX - 1));
        assert_eq!(a.allocate().unwrap().raw(), u64::MAX - 1);
        assert_eq!(a.peek().map(|id| id.raw()), Some(u64::MAX));
        assert_eq!(a.allocate().unwrap().raw(), u64::MAX);
        assert!(a.peek().is_none());
        assert!(a.allocate().is_none());
    }

    #[test]
    fn two_fresh_allocators_produce_the_same_sequence() {
        let mut a = PpuThreadIdAllocator::new();
        let mut b = PpuThreadIdAllocator::new();
        for _ in 0..16 {
            assert_eq!(a.allocate(), b.allocate());
        }
    }
}
