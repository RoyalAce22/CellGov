//! Generic Registry id assignment, register_at gaps, iteration order, and state hashing.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TestId(u64);

impl RegistryId for TestId {
    fn new(raw: u64) -> Self {
        Self(raw)
    }
    fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestValue(u32);

impl RegistryValueHash for TestValue {
    fn hash_into(&self, hasher: &mut cellgov_mem::Fnv1aHasher) {
        hasher.write(&self.0.to_le_bytes());
    }
}

fn id(raw: u64) -> TestId {
    TestId::new(raw)
}

#[test]
fn new_is_empty() {
    let r: Registry<TestId, TestValue> = Registry::new();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
    assert_eq!(r.ids().count(), 0);
}

#[test]
fn register_assigns_sequential_ids() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    let a = r.register(TestValue(0));
    let b = r.register(TestValue(0));
    let c = r.register(TestValue(0));
    assert_eq!(a, id(0));
    assert_eq!(b, id(1));
    assert_eq!(c, id(2));
    assert_eq!(r.len(), 3);
}

#[test]
fn get_missing_is_none() {
    let r: Registry<TestId, TestValue> = Registry::new();
    assert!(r.get(id(99)).is_none());
}

#[test]
fn get_mut_missing_is_none() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    assert!(r.get_mut(id(0)).is_none());
    assert!(r.get_mut(id(99)).is_none());
}

#[test]
fn register_at_inserts_and_advances_counter() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    assert!(r.register_at(id(5), TestValue(42)));
    assert_eq!(r.len(), 1);
    assert_eq!(r.get(id(5)), Some(&TestValue(42)));
    assert_eq!(r.register(TestValue(7)), id(6));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "debug_assert! fires before the bool path")]
fn register_at_returns_false_on_double_registration_in_release() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    assert!(r.register_at(id(3), TestValue(1)));
    let second = r.register_at(id(3), TestValue(99));
    assert!(!second);
    assert_eq!(r.len(), 1);
    // Existing value preserved, not clobbered.
    assert_eq!(r.get(id(3)), Some(&TestValue(1)));
}

#[test]
fn iter_is_in_id_order() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    for v in 0..4u32 {
        r.register(TestValue(v));
    }
    let collected: Vec<u64> = r.iter().map(|(i, _)| i.raw()).collect();
    assert_eq!(collected, vec![0, 1, 2, 3]);
}

#[test]
fn iter_skips_gaps_left_by_register_at() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    let _ = r.register_at(id(2), TestValue(20));
    let _ = r.register_at(id(5), TestValue(50));
    let collected: Vec<u64> = r.iter().map(|(i, _)| i.raw()).collect();
    assert_eq!(collected, vec![2, 5]);
}

#[test]
fn state_hash_is_idempotent() {
    let mut r: Registry<TestId, TestValue> = Registry::new();
    let i = r.register(TestValue(7));
    let h1 = r.state_hash();
    let h2 = r.state_hash();
    assert_eq!(h1, h2);
    if let Some(v) = r.get_mut(i) {
        *v = TestValue(8);
    }
    let h3 = r.state_hash();
    let h4 = r.state_hash();
    assert_eq!(h3, h4);
    assert_ne!(h1, h3);
}

#[test]
fn state_hash_distinguishes_id_position() {
    let mut a: Registry<TestId, TestValue> = Registry::new();
    let _ = a.register(TestValue(99));

    let mut b: Registry<TestId, TestValue> = Registry::new();
    let _ = b.register(TestValue(0));
    let _ = b.register(TestValue(99));

    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_empty_is_stable() {
    let a: Registry<TestId, TestValue> = Registry::new();
    let b: Registry<TestId, TestValue> = Registry::new();
    assert_eq!(a.state_hash(), b.state_hash());
}
