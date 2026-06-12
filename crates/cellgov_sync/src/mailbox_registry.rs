//! Mailbox registry: thin wrapper over
//! [`crate::Registry<MailboxId, Mailbox>`] that threads
//! [`Mailbox::with_capacity`] through the generic constructor.
//! `Effect::MailboxSend` / `Effect::MailboxReceiveAttempt` flow
//! through the commit pipeline into here.

use crate::mailbox::{Mailbox, MailboxId};
use crate::registry::Registry;

/// Runtime mailbox registry.
#[derive(Debug, Clone, Default)]
pub struct MailboxRegistry {
    inner: Registry<MailboxId, Mailbox>,
}

impl MailboxRegistry {
    /// Construct an empty registry.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered mailboxes.
    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the registry holds any mailboxes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Register a fresh mailbox; `capacity` is the spec depth (1
    /// for SPU outbound / outbound-interrupt, 4 for SPU inbound).
    pub fn register(&mut self, capacity: usize) -> MailboxId {
        self.inner.register(Mailbox::with_capacity(capacity))
    }

    /// Register at `id`. Returns `true` on vacant insert. See
    /// [`crate::Registry::register_at`] for collision semantics.
    #[must_use = "double-registration silently keeps the existing mailbox; check the bool"]
    pub fn register_at(&mut self, id: MailboxId, capacity: usize) -> bool {
        self.inner.register_at(id, Mailbox::with_capacity(capacity))
    }

    /// Borrow a mailbox by id.
    #[inline]
    pub fn get(&self, id: MailboxId) -> Option<&Mailbox> {
        self.inner.get(id)
    }

    /// Mutably borrow a mailbox by id.
    #[inline]
    pub fn get_mut(&mut self, id: MailboxId) -> Option<&mut Mailbox> {
        self.inner.get_mut(id)
    }

    /// Iterate registered mailboxes in id order.
    pub fn iter(&self) -> impl Iterator<Item = (MailboxId, &Mailbox)> + '_ {
        self.inner.iter()
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = MailboxId> + '_ {
        self.inner.ids()
    }

    /// FNV-1a hash over `(id, len, messages...)` in id order.
    #[inline]
    pub fn state_hash(&self) -> u64 {
        self.inner.state_hash()
    }
}

#[cfg(test)]
#[path = "tests/mailbox_registry_tests.rs"]
mod tests;
