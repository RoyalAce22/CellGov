//! Signal registry seam.
//!
//! Mirrors [`crate::mailbox_registry::MailboxRegistry`] for
//! [`SignalRegister`] instances. Owns every signal-notification
//! register known to the runtime, hands out stable [`SignalId`]s from
//! a sequential allocator, and exposes deterministic id-order
//! iteration via [`BTreeMap`] so the runtime never sees `HashMap`
//! insertion order.
//!
//! Currently provides registration, lookup, iteration, and a
//! [`SignalRegistry::state_hash`] that summarizes every register's
//! current value into a single `u64`. Wiring `Effect::SignalUpdate`
//! into the commit pipeline lands as a separate slice on top of this
//! seam, as does folding the signal hash into the runtime's
//! `SyncState` checkpoint emission.

use crate::signal::{SignalId, SignalRegister};
use std::collections::BTreeMap;

/// The runtime's signal-notification register registry.
///
/// Allocates [`SignalId`]s from a sequential counter so ids are
/// stable across runs of the same scenario as long as the
/// registration order is deterministic. Stores registers in a
/// [`BTreeMap`] keyed by `SignalId` so that
/// iteration is in id order. No host-time inputs or hash iteration
/// order influence the result.
#[derive(Debug, Clone, Default)]
pub struct SignalRegistry {
    next_id: u64,
    registers: BTreeMap<SignalId, SignalRegister>,
}

impl SignalRegistry {
    /// Construct an empty registry.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered signal registers.
    #[inline]
    pub fn len(&self) -> usize {
        self.registers.len()
    }

    /// Whether the registry holds any registers.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.registers.is_empty()
    }

    /// Register a fresh zero-initialized signal register and return
    /// its newly-assigned stable id.
    pub fn register(&mut self) -> SignalId {
        let id = SignalId::new(self.next_id);
        self.next_id += 1;
        self.registers.insert(id, SignalRegister::new());
        id
    }

    /// Borrow a register by id, if present.
    #[inline]
    pub fn get(&self, id: SignalId) -> Option<&SignalRegister> {
        self.registers.get(&id)
    }

    /// Mutably borrow a register by id, if present. The commit
    /// pipeline uses this to apply `SignalUpdate` effects in a future
    /// slice.
    #[inline]
    pub fn get_mut(&mut self, id: SignalId) -> Option<&mut SignalRegister> {
        self.registers.get_mut(&id)
    }

    /// Iterate registered signal registers in id order.
    pub fn iter(&self) -> impl Iterator<Item = (SignalId, &SignalRegister)> + '_ {
        self.registers.iter().map(|(id, r)| (*id, r))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = SignalId> + '_ {
        self.registers.keys().copied()
    }

    /// 64-bit deterministic hash of every register's `(id, value)`
    /// pair in id order.
    ///
    /// Will fold into the runtime's `SyncState` checkpoint hash in a
    /// future slice. FNV-1a, no host-time inputs, no external deps.
    /// The empty registry hashes to the FNV-1a empty-input value.
    pub fn state_hash(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = FNV_OFFSET;
        for (id, reg) in self.registers.iter() {
            for b in id.raw().to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(FNV_PRIME);
            }
            for b in reg.value().to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(FNV_PRIME);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let r = SignalRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(r.ids().count(), 0);
    }

    #[test]
    fn register_assigns_sequential_ids() {
        let mut r = SignalRegistry::new();
        let a = r.register();
        let b = r.register();
        let c = r.register();
        assert_eq!(a, SignalId::new(0));
        assert_eq!(b, SignalId::new(1));
        assert_eq!(c, SignalId::new(2));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn registered_registers_start_zero() {
        let mut r = SignalRegistry::new();
        let id = r.register();
        assert_eq!(r.get(id).unwrap().value(), 0);
    }

    #[test]
    fn get_mut_lets_caller_or_in_bits() {
        let mut r = SignalRegistry::new();
        let id = r.register();
        r.get_mut(id).unwrap().or_in(0xa5);
        assert_eq!(r.get(id).unwrap().value(), 0xa5);
    }

    #[test]
    fn get_missing_is_none() {
        let r = SignalRegistry::new();
        assert!(r.get(SignalId::new(99)).is_none());
    }

    #[test]
    fn iter_is_in_id_order() {
        let mut r = SignalRegistry::new();
        for _ in 0..4 {
            r.register();
        }
        let ids: Vec<u64> = r.iter().map(|(id, _)| id.raw()).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn state_hash_of_empty_registry_is_stable() {
        let a = SignalRegistry::new();
        let b = SignalRegistry::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_changes_when_a_register_value_changes() {
        let mut r = SignalRegistry::new();
        let id = r.register();
        let h0 = r.state_hash();
        r.get_mut(id).unwrap().or_in(1);
        let h1 = r.state_hash();
        assert_ne!(h0, h1);
    }

    #[test]
    fn state_hash_distinguishes_register_values() {
        let mut a = SignalRegistry::new();
        let id_a = a.register();
        a.get_mut(id_a).unwrap().or_in(1);

        let mut b = SignalRegistry::new();
        let id_b = b.register();
        b.get_mut(id_b).unwrap().or_in(2);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_round_trips_after_clear() {
        // OR-in then clear must restore the original (empty) hash.
        let mut r = SignalRegistry::new();
        let id = r.register();
        let h0 = r.state_hash();
        r.get_mut(id).unwrap().or_in(0xff);
        assert_ne!(r.state_hash(), h0);
        r.get_mut(id).unwrap().clear();
        assert_eq!(r.state_hash(), h0);
    }

    #[test]
    fn state_hash_distinguishes_id_position() {
        // Same register value, different ids must hash differently.
        // Burn an id slot in the second registry.
        let mut a = SignalRegistry::new();
        let id_a = a.register();
        a.get_mut(id_a).unwrap().or_in(0x42);

        let mut b = SignalRegistry::new();
        let _burn = b.register();
        let id_b = b.register();
        b.get_mut(id_b).unwrap().or_in(0x42);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn or_in_is_commutative_at_registry_level() {
        // OR-in different bits in different orders into the same id
        // must produce the same final hash.
        let mut a = SignalRegistry::new();
        let id_a = a.register();
        a.get_mut(id_a).unwrap().or_in(0x0f);
        a.get_mut(id_a).unwrap().or_in(0xf0);

        let mut b = SignalRegistry::new();
        let id_b = b.register();
        b.get_mut(id_b).unwrap().or_in(0xf0);
        b.get_mut(id_b).unwrap().or_in(0x0f);

        assert_eq!(a.state_hash(), b.state_hash());
    }
}
