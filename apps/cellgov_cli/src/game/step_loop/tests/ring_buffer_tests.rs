//! `Ring<T, N>`: push, saturation, and oldest-first read-back.

use super::*;

#[test]
fn an_empty_ring_reads_back_nothing() {
    let ring: Ring<u64, 4> = Ring::new();
    assert_eq!(ring.filled(), 0);
    assert_eq!(ring.iter().count(), 0);
}

#[test]
fn a_partial_ring_reads_back_in_push_order() {
    let mut ring: Ring<u64, 4> = Ring::new();
    ring.push(10);
    ring.push(20);
    assert_eq!(ring.filled(), 2);
    assert_eq!(ring.iter().collect::<Vec<_>>(), vec![10, 20]);
}

#[test]
fn a_wrapped_ring_drops_the_oldest_and_reads_oldest_first() {
    let mut ring: Ring<u64, 3> = Ring::new();
    for pc in [1, 2, 3, 4, 5] {
        ring.push(pc);
    }
    assert_eq!(ring.filled(), 3);
    assert_eq!(ring.iter().collect::<Vec<_>>(), vec![3, 4, 5]);
}

#[test]
fn a_default_ring_is_empty() {
    let ring: Ring<u64, 4> = Ring::default();
    assert_eq!(ring.filled(), 0);
    assert_eq!(ring.iter().count(), 0);
}

#[test]
fn a_ring_saturates_at_exactly_its_capacity() {
    let mut ring: Ring<u64, 3> = Ring::new();
    for pc in [1, 2, 3] {
        ring.push(pc);
    }
    assert_eq!(ring.filled(), 3);
    assert_eq!(ring.iter().collect::<Vec<_>>(), vec![1, 2, 3]);
    ring.push(4);
    assert_eq!(ring.filled(), 3, "filled never exceeds N");
    assert_eq!(ring.iter().collect::<Vec<_>>(), vec![2, 3, 4]);
}

#[test]
fn a_tuple_ring_keeps_both_halves_of_each_entry() {
    let mut ring: Ring<(u64, u64), 2> = Ring::new();
    ring.push((0x10, 0x1000));
    ring.push((0x11, 0x1004));
    ring.push((0x12, 0x1008));
    assert_eq!(
        ring.iter().collect::<Vec<_>>(),
        vec![(0x11, 0x1004), (0x12, 0x1008)]
    );
}
