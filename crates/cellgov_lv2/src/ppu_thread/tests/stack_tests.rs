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

#[test]
fn free_last_rewinds_so_the_next_allocation_reuses_the_block() {
    let mut a = ThreadStackAllocator::new();
    let s1 = a.allocate(0x10_000, 0x10).unwrap();
    assert!(a.free_last(&s1));
    let s2 = a.allocate(0x10_000, 0x10).unwrap();
    assert_eq!(s2.base, s1.base, "a freed last block must be reused");
}

#[test]
fn free_last_refuses_a_block_that_is_not_the_most_recent() {
    let mut a = ThreadStackAllocator::new();
    let s1 = a.allocate(0x10_000, 0x10).unwrap();
    let s2 = a.allocate(0x10_000, 0x10).unwrap();
    assert!(!a.free_last(&s1));
    // s2 stays live: the next allocation sits above it.
    let s3 = a.allocate(0x10_000, 0x10).unwrap();
    assert!(s3.base >= s2.end());
}

#[test]
fn a_second_free_last_of_the_same_block_is_refused() {
    let mut a = ThreadStackAllocator::new();
    let s1 = a.allocate(0x10_000, 0x10).unwrap();
    assert!(a.free_last(&s1));
    assert!(!a.free_last(&s1), "a double free must not rewind twice");
    assert_eq!(a.peek_next(0x10), Some(s1.base));
}

#[test]
fn free_last_refuses_a_block_below_the_arena_floor() {
    let mut a = ThreadStackAllocator::new();
    // Ends exactly at the untouched bump pointer, but starts below
    // the floor: honouring it would rewind into the primary stack.
    let bogus = ThreadStack::new(ThreadStackAllocator::CHILD_STACK_BASE - 0x1000, 0x1000);
    assert!(!a.free_last(&bogus));
    assert_eq!(
        a.peek_next(0x10),
        Some(ThreadStackAllocator::CHILD_STACK_BASE),
    );
}

#[test]
fn free_last_refuses_a_block_whose_end_would_wrap() {
    let mut a = ThreadStackAllocator::new();
    let wrapping = ThreadStack::new(u64::MAX, 0x10);
    assert!(!a.free_last(&wrapping));
    assert_eq!(
        a.peek_next(0x10),
        Some(ThreadStackAllocator::CHILD_STACK_BASE),
    );
}

#[test]
fn free_last_of_an_over_aligned_block_leaves_the_next_base_at_that_block() {
    let mut a = ThreadStackAllocator::new();
    let _pad = a.allocate(0x10, 0x10).unwrap();
    let s = a.allocate(0x10_000, 0x1_0000).unwrap();
    assert!(a.free_last(&s));
    // The rewind lands on the aligned base, not on the pre-allocation
    // bump pointer: the alignment padding below `s` stays consumed.
    assert_eq!(a.peek_next(0x10), Some(s.base));
}
