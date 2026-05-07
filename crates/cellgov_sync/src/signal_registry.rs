//! Signal registry: thin wrapper over
//! [`crate::Registry<SignalId, SignalRegister>`].
//! `Effect::SignalUpdate` flows through the commit pipeline into
//! here.

use crate::registry::Registry;
use crate::signal::{SignalId, SignalRegister};

/// Runtime signal-notification register registry.
#[derive(Debug, Clone, Default)]
pub struct SignalRegistry {
    inner: Registry<SignalId, SignalRegister>,
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
        self.inner.len()
    }

    /// Whether the registry holds any registers.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Register a fresh zero-initialized signal register.
    pub fn register(&mut self) -> SignalId {
        self.inner.register(SignalRegister::new())
    }

    /// Borrow a register by id.
    #[inline]
    pub fn get(&self, id: SignalId) -> Option<&SignalRegister> {
        self.inner.get(id)
    }

    /// Mutably borrow a register by id.
    #[inline]
    pub fn get_mut(&mut self, id: SignalId) -> Option<&mut SignalRegister> {
        self.inner.get_mut(id)
    }

    /// Iterate registered registers in id order.
    pub fn iter(&self) -> impl Iterator<Item = (SignalId, &SignalRegister)> + '_ {
        self.inner.iter()
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = SignalId> + '_ {
        self.inner.ids()
    }

    /// FNV-1a hash over `(id, value)` pairs in id order.
    #[inline]
    pub fn state_hash(&self) -> u64 {
        self.inner.state_hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
