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
#[path = "tests/registry_tests.rs"]
mod tests;
