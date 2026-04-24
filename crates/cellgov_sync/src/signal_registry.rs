//! Owns every [`SignalRegister`] in the runtime, allocates sequential
//! [`SignalId`]s, and iterates in id order via `BTreeMap`.
//! `Effect::SignalUpdate` flows through the commit pipeline into this
//! registry.

use crate::signal::{SignalId, SignalRegister};
use std::collections::BTreeMap;

/// Runtime signal-notification register registry.
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

    /// Number of registered registers.
    #[inline]
    pub fn len(&self) -> usize {
        self.registers.len()
    }

    /// Whether the registry holds any registers.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.registers.is_empty()
    }

    /// Register a fresh zero-initialized signal register.
    pub fn register(&mut self) -> SignalId {
        let id = SignalId::new(self.next_id);
        self.next_id += 1;
        self.registers.insert(id, SignalRegister::new());
        id
    }

    /// Borrow a register by id.
    #[inline]
    pub fn get(&self, id: SignalId) -> Option<&SignalRegister> {
        self.registers.get(&id)
    }

    /// Mutably borrow a register by id.
    #[inline]
    pub fn get_mut(&mut self, id: SignalId) -> Option<&mut SignalRegister> {
        self.registers.get_mut(&id)
    }

    /// Iterate registered registers in id order.
    pub fn iter(&self) -> impl Iterator<Item = (SignalId, &SignalRegister)> + '_ {
        self.registers.iter().map(|(id, r)| (*id, r))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = SignalId> + '_ {
        self.registers.keys().copied()
    }

    /// FNV-1a hash over `(id, value)` pairs in id order. Folded into
    /// the `SyncState` checkpoint hash. Empty registry hashes to the
    /// FNV-1a empty-input value.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, reg) in self.registers.iter() {
            hasher.write(&id.raw().to_le_bytes());
            hasher.write(&reg.value().to_le_bytes());
        }
        hasher.finish()
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
