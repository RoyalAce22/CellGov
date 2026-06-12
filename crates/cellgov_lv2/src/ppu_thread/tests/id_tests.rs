//! PPU thread-id allocator tests -- monotonicity, primary-id reservation, and exhaustion.

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

impl PpuThreadIdAllocator {
    /// Seed the next slot directly; exercises exhaustion near
    /// `u64::MAX` without 2^64 allocations.
    pub(crate) fn with_next(next: u64) -> Self {
        Self { next: Some(next) }
    }
}
