//! Generic id-keyed slot registry shared by `MailboxRegistry` and
//! `SignalRegistry`.
//!
//! Storage is `Vec<Option<V>>` indexed by `id.raw() as usize`: O(1)
//! lookup, deterministic id-order iteration, no `BTreeMap`, no `Ord`
//! requirement on the id type. `next_id` advances under
//! `checked_add` so a saturating wrap cannot mint an id that aliases
//! an existing one.

use core::marker::PhantomData;

/// Newtype-over-`u64` contract every registry id type must satisfy.
pub trait RegistryId: Copy + Eq {
    /// Construct from a raw `u64`.
    fn new(raw: u64) -> Self;
    /// Underlying `u64`, used as the slot index.
    fn raw(self) -> u64;
}

/// Per-value contribution to [`Registry::state_hash`]; the id and
/// boundary marker are hashed by the registry itself.
pub trait RegistryValueHash {
    /// Fold the value's fields into `hasher` in a fixed order.
    fn hash_into(&self, hasher: &mut cellgov_mem::Fnv1aHasher);
}

/// Sequential-id, slot-vector registry.
#[derive(Debug, Clone)]
pub struct Registry<I, V> {
    /// Strictly greater than every occupied slot index.
    next_id: u64,
    /// Slot `i` holds the value at `I::new(i)`, or `None` if vacant.
    slots: Vec<Option<V>>,
    /// Cached count of `Some` slots; keeps `len()` O(1).
    count: usize,
    _id: PhantomData<fn() -> I>,
}

impl<I, V> Default for Registry<I, V> {
    fn default() -> Self {
        Self {
            next_id: 0,
            slots: Vec::new(),
            count: 0,
            _id: PhantomData,
        }
    }
}

impl<I: RegistryId, V> Registry<I, V> {
    /// Construct an empty registry.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered values.
    #[inline]
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the registry holds any values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Register `value` at the next sequential id.
    ///
    /// # Panics
    ///
    /// Panics if the id space is exhausted (`next_id == u64::MAX`).
    pub fn register(&mut self, value: V) -> I {
        let id = I::new(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("id space exhausted");
        self.ensure_slot(id.raw());
        debug_assert!(
            self.slots[id.raw() as usize].is_none(),
            "register(): next_id slot already occupied -- monotonic counter wrapped"
        );
        self.slots[id.raw() as usize] = Some(value);
        self.count += 1;
        self.debug_check_count();
        id
    }

    /// Register `value` at `id`, advancing the next-id counter past
    /// it. Returns `true` on vacant insert, `false` on collision
    /// (existing value preserved).
    ///
    /// # Panics
    ///
    /// Panics if `id.raw() == u64::MAX`, or in debug builds if the
    /// slot is already occupied.
    #[must_use = "double-registration silently keeps the existing value; check the bool"]
    pub fn register_at(&mut self, id: I, value: V) -> bool {
        if id.raw() >= self.next_id {
            self.next_id = id.raw().checked_add(1).expect("id space exhausted");
        }
        self.ensure_slot(id.raw());
        let slot = &mut self.slots[id.raw() as usize];
        debug_assert!(
            slot.is_none(),
            "register_at(): slot already occupied; double-registration would clobber the existing value"
        );
        if slot.is_some() {
            return false;
        }
        *slot = Some(value);
        self.count += 1;
        self.debug_check_count();
        true
    }

    /// Borrow a value by id.
    #[inline]
    pub fn get(&self, id: I) -> Option<&V> {
        self.slots.get(id.raw() as usize).and_then(Option::as_ref)
    }

    /// Mutably borrow a value by id.
    #[inline]
    pub fn get_mut(&mut self, id: I) -> Option<&mut V> {
        self.slots
            .get_mut(id.raw() as usize)
            .and_then(Option::as_mut)
    }

    /// Iterate registered `(id, &value)` pairs in id order.
    pub fn iter(&self) -> impl Iterator<Item = (I, &V)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| slot.as_ref().map(|v| (I::new(i as u64), v)))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        self.iter().map(|(id, _)| id)
    }

    /// `resize_with(|| None)` rather than `resize(None)` so the
    /// generic does not require `V: Clone`.
    #[inline]
    fn ensure_slot(&mut self, idx: u64) {
        let need = idx as usize + 1;
        if self.slots.len() < need {
            self.slots.resize_with(need, || None);
        }
    }

    /// Trip-wire for any future mutation site that forgets to
    /// update `count`. Compiles to a no-op in release.
    #[inline]
    fn debug_check_count(&self) {
        #[cfg(debug_assertions)]
        {
            let actual = self.slots.iter().filter(|s| s.is_some()).count();
            debug_assert_eq!(
                self.count, actual,
                "Registry::count desync: cached={} actual={}",
                self.count, actual
            );
        }
    }
}

impl<I: RegistryId, V: RegistryValueHash> Registry<I, V> {
    /// FNV-1a hash over `(id, value)` pairs in id order. Each
    /// value's fields are folded in via
    /// [`RegistryValueHash::hash_into`].
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, value) in self.iter() {
            hasher.write(&id.raw().to_le_bytes());
            value.hash_into(&mut hasher);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
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
}
