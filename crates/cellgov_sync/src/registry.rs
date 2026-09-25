//! Generic id-keyed registry shared by `MailboxRegistry` and
//! `SignalRegistry`.
//!
//! Values live in a [`LaneMap`] keyed by `id.raw()`: deterministic
//! id-order iteration and a sync-state partial kept on every change.
//! `next_id` advances under `checked_add` so a saturating wrap cannot
//! mint an id that aliases an existing one.

use core::marker::PhantomData;

use cellgov_mem::lanes::{LaneEntryMut, LaneMap, LaneValue};

/// Newtype-over-`u64` contract every registry id type must satisfy.
pub trait RegistryId: Copy + Eq {
    /// Construct from a raw `u64`.
    fn new(raw: u64) -> Self;
    /// Underlying `u64`, the value's object id in the sync-state lanes.
    fn raw(self) -> u64;
}

/// Sequential-id registry.
#[derive(Debug, Clone)]
pub struct Registry<I, V> {
    /// Strictly greater than every registered id.
    next_id: u64,
    values: LaneMap<u64, V>,
    _id: PhantomData<fn() -> I>,
}

impl<I: RegistryId, V: LaneValue> Registry<I, V> {
    /// Construct an empty registry whose values hash under sync-state
    /// source `source`.
    #[inline]
    pub fn new(source: u8) -> Self {
        Self {
            next_id: 0,
            values: LaneMap::new(source, |raw| raw),
            _id: PhantomData,
        }
    }

    /// Number of registered values.
    #[inline]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the registry holds any values.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Register `value` at the next sequential id.
    ///
    /// # Panics
    ///
    /// Panics if the id space is exhausted (`next_id == u64::MAX`).
    pub fn register(&mut self, value: V) -> I {
        let id = I::new(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("id space exhausted");
        let prior = self.values.insert(id.raw(), value);
        debug_assert!(
            prior.is_none(),
            "register(): next_id slot already occupied -- monotonic counter wrapped"
        );
        id
    }

    /// Register `value` at `id`, advancing the next-id counter past
    /// it. Returns `true` on vacant insert, `false` on collision
    /// (existing value preserved).
    ///
    /// # Panics
    ///
    /// Panics if `id.raw() == u64::MAX`, or in debug builds if the
    /// id is already registered.
    #[must_use = "double-registration silently keeps the existing value; check the bool"]
    pub fn register_at(&mut self, id: I, value: V) -> bool {
        if id.raw() >= self.next_id {
            self.next_id = id.raw().checked_add(1).expect("id space exhausted");
        }
        let occupied = self.values.contains_key(id.raw());
        debug_assert!(
            !occupied,
            "register_at(): slot already occupied; double-registration would clobber the existing value"
        );
        if occupied {
            return false;
        }
        self.values.insert(id.raw(), value);
        true
    }

    /// Borrow a value by id.
    #[inline]
    pub fn get(&self, id: I) -> Option<&V> {
        self.values.get(id.raw())
    }

    /// Mutably borrow a value by id, through a guard that updates the
    /// registry's partial when it drops.
    #[inline]
    pub fn get_mut(&mut self, id: I) -> Option<LaneEntryMut<'_, u64, V>> {
        self.values.get_mut(id.raw())
    }

    /// Iterate registered `(id, &value)` pairs in id order.
    pub fn iter(&self) -> impl Iterator<Item = (I, &V)> + '_ {
        self.values.iter().map(|(raw, v)| (I::new(raw), v))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = I> + '_ {
        self.iter().map(|(id, _)| id)
    }

    /// The registry's partial of the sync-state sum: each value's
    /// presence lane and its [`LaneValue`] lanes.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.values.partial()
    }

    /// [`Self::sync_partial`] computed from every value.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.values.partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/registry_tests.rs"]
mod tests;
