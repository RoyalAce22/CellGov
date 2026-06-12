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
#[path = "tests/signal_registry_tests.rs"]
mod tests;
