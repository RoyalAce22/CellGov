//! Child-thread stack allocator tests -- non-overlap, alignment, determinism, and initial-SP placement.

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
