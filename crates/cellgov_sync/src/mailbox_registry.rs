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
mod tests {
    use super::*;

    #[test]
    fn registered_mailboxes_start_empty() {
        let mut r = MailboxRegistry::new();
        let id = r.register(4);
        let m = r.get(id).expect("present");
        assert!(m.is_empty());
        assert_eq!(m.capacity(), 4);
    }

    #[test]
    fn get_mut_lets_caller_send_into_a_mailbox() {
        let mut r = MailboxRegistry::new();
        let id = r.register(4);
        r.get_mut(id).unwrap().force_send(42);
        assert_eq!(r.get(id).unwrap().len(), 1);
        assert_eq!(r.get(id).unwrap().peek(), Some(42));
    }

    #[test]
    fn register_then_try_send_until_full_returns_false() {
        let mut r = MailboxRegistry::new();
        let id = r.register(2);
        let m = r.get_mut(id).unwrap();
        assert!(m.try_send(1));
        assert!(m.try_send(2));
        assert!(!m.try_send(3));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn state_hash_changes_when_a_mailbox_receives_a_send() {
        let mut r = MailboxRegistry::new();
        let id = r.register(4);
        let h0 = r.state_hash();
        r.get_mut(id).unwrap().force_send(7);
        let h1 = r.state_hash();
        assert_ne!(h0, h1);
    }

    #[test]
    fn state_hash_distinguishes_message_contents() {
        let mut a = MailboxRegistry::new();
        let id_a = a.register(4);
        a.get_mut(id_a).unwrap().force_send(1);

        let mut b = MailboxRegistry::new();
        let id_b = b.register(4);
        b.get_mut(id_b).unwrap().force_send(2);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_message_order() {
        let mut a = MailboxRegistry::new();
        let id_a = a.register(4);
        a.get_mut(id_a).unwrap().force_send(1);
        a.get_mut(id_a).unwrap().force_send(2);

        let mut b = MailboxRegistry::new();
        let id_b = b.register(4);
        b.get_mut(id_b).unwrap().force_send(2);
        b.get_mut(id_b).unwrap().force_send(1);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_round_trips_after_drain() {
        let mut r = MailboxRegistry::new();
        let id = r.register(4);
        let h0 = r.state_hash();
        r.get_mut(id).unwrap().force_send(1);
        r.get_mut(id).unwrap().force_send(2);
        assert_ne!(r.state_hash(), h0);
        assert_eq!(r.get_mut(id).unwrap().try_receive(), Some(1));
        assert_eq!(r.get_mut(id).unwrap().try_receive(), Some(2));
        assert_eq!(r.state_hash(), h0);
    }
}
