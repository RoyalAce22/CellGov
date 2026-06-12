//! RSX call-stack LIFO discipline, typed overflow/underflow channels, and state-hash goldens.

use super::*;

#[test]
fn new_stack_is_empty() {
    let s = RsxCallStack::new();
    assert_eq!(s.depth(), 0);
    assert!(s.is_empty());
}

#[test]
fn push_then_pop_round_trips_single_entry() {
    let mut s = RsxCallStack::new();
    assert!(s.push(0x1000).is_ok());
    assert_eq!(s.depth(), 1);
    assert!(!s.is_empty());
    assert_eq!(s.pop(), Ok(0x1000));
    assert!(s.is_empty());
}

#[test]
fn push_pop_is_lifo() {
    let mut s = RsxCallStack::new();
    s.push(0x100).unwrap();
    s.push(0x200).unwrap();
    s.push(0x300).unwrap();
    assert_eq!(s.pop(), Ok(0x300));
    assert_eq!(s.pop(), Ok(0x200));
    assert_eq!(s.pop(), Ok(0x100));
    assert_eq!(s.pop(), Err(CallStackUnderflow));
}

#[test]
fn push_at_capacity_returns_overflow_without_mutating() {
    let mut s = RsxCallStack::new();
    for i in 0..CALL_STACK_DEPTH {
        s.push(i as u32).unwrap();
    }
    let pre = s;
    assert_eq!(s.push(0xDEAD), Err(CallStackOverflow));
    assert_eq!(s, pre, "rejected push must not perturb stack state");
}

#[test]
fn pop_on_empty_returns_underflow() {
    let mut s = RsxCallStack::new();
    assert_eq!(s.pop(), Err(CallStackUnderflow));
}

#[test]
fn overflow_and_underflow_are_distinct_typed_channels() {
    // Compile-time witness: the two error types do not unify, so
    // a caller cannot collapse them without explicit conversion.
    fn is_overflow(_: CallStackOverflow) -> &'static str {
        "overflow"
    }
    fn is_underflow(_: CallStackUnderflow) -> &'static str {
        "underflow"
    }
    let mut s = RsxCallStack::new();
    match s.pop() {
        Err(e) => assert_eq!(is_underflow(e), "underflow"),
        Ok(_) => panic!("empty stack must underflow"),
    }
    for i in 0..CALL_STACK_DEPTH {
        s.push(i as u32).unwrap();
    }
    match s.push(0xCAFE) {
        Err(e) => assert_eq!(is_overflow(e), "overflow"),
        Ok(()) => panic!("full stack must overflow"),
    }
}

#[test]
fn clear_resets_to_pristine_state_hash() {
    let mut s = RsxCallStack::new();
    s.push(0x1234).unwrap();
    s.push(0x5678).unwrap();
    s.clear();
    let fresh = RsxCallStack::new();
    assert_eq!(s, fresh);
    assert_eq!(s.state_hash(), fresh.state_hash());
}

#[test]
fn state_hash_ignores_stale_slots_past_depth() {
    let mut a = RsxCallStack::new();
    a.push(0x111).unwrap();
    a.push(0x222).unwrap();
    let mut b = RsxCallStack::new();
    b.push(0x111).unwrap();
    b.push(0x222).unwrap();
    b.push(0xDEAD).unwrap();
    b.pop().unwrap();
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "depth-2 hash must not depend on bytes beneath the visible top",
    );
}

#[test]
fn partial_eq_is_stricter_than_state_hash() {
    let mut a = RsxCallStack::new();
    a.push(0x111).unwrap();
    a.push(0xDEAD).unwrap();
    a.pop().unwrap();
    let mut b = RsxCallStack::new();
    b.push(0x111).unwrap();
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "hash-equality holds modulo stale slots",
    );
    assert_ne!(a, b, "derived PartialEq surfaces the stale-byte difference");
}

#[test]
fn state_hash_distinguishes_depths() {
    let mut a = RsxCallStack::new();
    a.push(0x100).unwrap();
    let mut b = RsxCallStack::new();
    b.push(0x100).unwrap();
    b.push(0x200).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_entry_values() {
    let mut a = RsxCallStack::new();
    a.push(0x100).unwrap();
    let mut b = RsxCallStack::new();
    b.push(0x200).unwrap();
    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn empty_stack_state_hash_golden() {
    // Doubles as a fixed-seed canary for `cellgov_mem::Fnv1aHasher`:
    // stable only if `Fnv1aHasher::new()` uses the FNV offset basis
    // with no host or run seeding.
    const EXPECTED: u64 = 0x082f_2207_b4e8_8cc4;
    let actual = RsxCallStack::new().state_hash();
    assert_eq!(
        actual, EXPECTED,
        "empty-stack hash drift: got 0x{actual:016x}, expected 0x{EXPECTED:016x}; \
         if CALL_STACK_HASH_FORMAT_VERSION was bumped this golden must move too",
    );
}

#[test]
fn single_entry_state_hash_golden() {
    const EXPECTED: u64 = 0xff54_00f1_b413_16bc;
    let mut s = RsxCallStack::new();
    s.push(0x100).unwrap();
    let actual = s.state_hash();
    assert_eq!(
        actual, EXPECTED,
        "depth-1 hash drift: got 0x{actual:016x}, expected 0x{EXPECTED:016x}; \
         field-order or entry-encoding drift in state_hash",
    );
}
