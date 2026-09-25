//! Mailbox registry: thin wrapper over
//! [`crate::Registry<MailboxId, Mailbox>`] that threads
//! [`Mailbox::with_capacity`] through the generic constructor.
//! `Effect::MailboxSend` / `Effect::MailboxReceiveAttempt` flow
//! through the commit pipeline into here.

use crate::mailbox::{Mailbox, MailboxId};
use crate::registry::Registry;
use cellgov_mem::lanes::LaneEntryMut;

/// Runtime mailbox registry.
#[derive(Debug, Clone)]
pub struct MailboxRegistry {
    inner: Registry<MailboxId, Mailbox>,
}

impl Default for MailboxRegistry {
    fn default() -> Self {
        Self {
            inner: Registry::new(cellgov_mem::lanes::source::MAILBOX),
        }
    }
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
    pub fn get_mut(&mut self, id: MailboxId) -> Option<LaneEntryMut<'_, u64, Mailbox>> {
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

    /// The registry's partial of the sync-state sum: per mailbox a
    /// presence lane, the queue length and one lane per queued message.
    #[inline]
    pub fn sync_partial(&self) -> u128 {
        self.inner.sync_partial()
    }

    /// [`Self::sync_partial`] computed from every mailbox.
    pub fn sync_partial_from_scratch(&self) -> u128 {
        self.inner.sync_partial_from_scratch()
    }
}

#[cfg(test)]
#[path = "tests/mailbox_registry_tests.rs"]
mod tests;
