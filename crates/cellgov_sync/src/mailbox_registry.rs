//! Mailbox registry.
//!
//! Mirrors `cellgov_core::UnitRegistry` for [`Mailbox`] instances.
//! Owns every mailbox known to the runtime, hands out stable
//! [`MailboxId`]s from a sequential allocator, and exposes
//! deterministic id-order iteration via [`BTreeMap`] so the runtime
//! never sees `HashMap` insertion order.
//!
//! Provides registration, lookup, iteration, and a
//! [`MailboxRegistry::state_hash`] that summarizes every mailbox's
//! queue contents into a single `u64`. `Effect::MailboxSend` and
//! `Effect::MailboxReceiveAttempt` flow through the commit pipeline
//! into this registry, and the `SyncState` checkpoint emission in
//! `cellgov_core` folds `state_hash` into its hash.

use crate::mailbox::{Mailbox, MailboxId};
use std::collections::BTreeMap;

/// The runtime's mailbox registry.
///
/// Allocates [`MailboxId`]s from a sequential counter so ids are
/// stable across runs of the same scenario as long as the
/// registration order is deterministic. Stores mailboxes in a
/// [`BTreeMap`] keyed by `MailboxId` so that
/// iteration is in id order. No host-time inputs or hash iteration
/// order influence the result.
#[derive(Debug, Clone, Default)]
pub struct MailboxRegistry {
    next_id: u64,
    mailboxes: BTreeMap<MailboxId, Mailbox>,
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
        self.mailboxes.len()
    }

    /// Whether the registry holds any mailboxes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.mailboxes.is_empty()
    }

    /// Register a fresh empty mailbox and return its newly-assigned
    /// stable id.
    pub fn register(&mut self) -> MailboxId {
        let id = MailboxId::new(self.next_id);
        self.next_id += 1;
        self.mailboxes.insert(id, Mailbox::new());
        id
    }

    /// Register an empty mailbox at a specific id. Used by the runtime
    /// to align MailboxId with UnitId for dynamically created SPUs.
    pub fn register_at(&mut self, id: MailboxId) {
        self.mailboxes.entry(id).or_default();
        if id.raw() >= self.next_id {
            self.next_id = id.raw() + 1;
        }
    }

    /// Borrow a mailbox by id, if present.
    #[inline]
    pub fn get(&self, id: MailboxId) -> Option<&Mailbox> {
        self.mailboxes.get(&id)
    }

    /// Mutably borrow a mailbox by id, if present. The commit pipeline
    /// uses this to apply `MailboxSend` / `MailboxReceiveAttempt`
    /// effects.
    #[inline]
    pub fn get_mut(&mut self, id: MailboxId) -> Option<&mut Mailbox> {
        self.mailboxes.get_mut(&id)
    }

    /// Iterate registered mailboxes in id order.
    pub fn iter(&self) -> impl Iterator<Item = (MailboxId, &Mailbox)> + '_ {
        self.mailboxes.iter().map(|(id, m)| (*id, m))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = MailboxId> + '_ {
        self.mailboxes.keys().copied()
    }

    /// 64-bit deterministic hash of every mailbox's queued messages
    /// in id order.
    ///
    /// Used as part of the `SyncState` checkpoint hash. FNV-1a, no host-time
    /// inputs, no external deps. Walks the underlying [`BTreeMap`] in
    /// id order, then folds (id_le_bytes, message_count_le_bytes,
    /// each_message_le_bytes) into the running hash so that any
    /// difference in id assignment, queue length, or message contents
    /// shows up in the result.
    ///
    /// Replay tooling compares pairs of these values to assert that
    /// two runs reached the same set of mailbox states. The empty
    /// registry hashes to the FNV-1a empty-input value.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, mailbox) in self.mailboxes.iter() {
            hasher.write(&id.raw().to_le_bytes());
            hasher.write(&(mailbox.len() as u64).to_le_bytes());
            // Walk front-to-back, the same order try_receive would
            // observe, so replay sees identical bytes across runs.
            // Mailbox does not expose iter() yet; the helper below
            // clones the queue once and drains the clone.
            for word in mailbox_iter(mailbox) {
                hasher.write(&word.to_le_bytes());
            }
        }
        hasher.finish()
    }
}

/// Internal: walk a mailbox's queued messages front-to-back without
/// consuming them. Equivalent to what a `Mailbox::iter()` method
/// would expose; kept private to the registry because the only
/// current caller is the state-hash path.
fn mailbox_iter(mailbox: &Mailbox) -> impl Iterator<Item = u32> + '_ {
    let mut clone = mailbox.clone();
    std::iter::from_fn(move || clone.try_receive())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_empty() {
        let r = MailboxRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(r.ids().count(), 0);
    }

    #[test]
    fn register_assigns_sequential_ids() {
        let mut r = MailboxRegistry::new();
        let a = r.register();
        let b = r.register();
        let c = r.register();
        assert_eq!(a, MailboxId::new(0));
        assert_eq!(b, MailboxId::new(1));
        assert_eq!(c, MailboxId::new(2));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn registered_mailboxes_start_empty() {
        let mut r = MailboxRegistry::new();
        let id = r.register();
        let m = r.get(id).expect("present");
        assert!(m.is_empty());
    }

    #[test]
    fn get_mut_lets_caller_send_into_a_mailbox() {
        let mut r = MailboxRegistry::new();
        let id = r.register();
        r.get_mut(id).unwrap().send(42);
        assert_eq!(r.get(id).unwrap().len(), 1);
        assert_eq!(r.get(id).unwrap().peek(), Some(42));
    }

    #[test]
    fn get_missing_is_none() {
        let r = MailboxRegistry::new();
        assert!(r.get(MailboxId::new(99)).is_none());
    }

    #[test]
    fn iter_is_in_id_order() {
        let mut r = MailboxRegistry::new();
        for _ in 0..4 {
            r.register();
        }
        let ids: Vec<u64> = r.iter().map(|(id, _)| id.raw()).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn state_hash_of_empty_registry_is_stable() {
        let a = MailboxRegistry::new();
        let b = MailboxRegistry::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_changes_when_a_mailbox_receives_a_send() {
        let mut r = MailboxRegistry::new();
        let id = r.register();
        let h0 = r.state_hash();
        r.get_mut(id).unwrap().send(7);
        let h1 = r.state_hash();
        assert_ne!(h0, h1);
    }

    #[test]
    fn state_hash_distinguishes_message_contents() {
        let mut a = MailboxRegistry::new();
        let id_a = a.register();
        a.get_mut(id_a).unwrap().send(1);

        let mut b = MailboxRegistry::new();
        let id_b = b.register();
        b.get_mut(id_b).unwrap().send(2);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_message_order() {
        let mut a = MailboxRegistry::new();
        let id_a = a.register();
        a.get_mut(id_a).unwrap().send(1);
        a.get_mut(id_a).unwrap().send(2);

        let mut b = MailboxRegistry::new();
        let id_b = b.register();
        b.get_mut(id_b).unwrap().send(2);
        b.get_mut(id_b).unwrap().send(1);

        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_round_trips_after_drain() {
        // Sending then fully draining must restore the original hash:
        // hash is a pure function of (id, queue contents).
        let mut r = MailboxRegistry::new();
        let id = r.register();
        let h0 = r.state_hash();
        r.get_mut(id).unwrap().send(1);
        r.get_mut(id).unwrap().send(2);
        assert_ne!(r.state_hash(), h0);
        assert_eq!(r.get_mut(id).unwrap().try_receive(), Some(1));
        assert_eq!(r.get_mut(id).unwrap().try_receive(), Some(2));
        assert_eq!(r.state_hash(), h0);
    }

    #[test]
    fn state_hash_distinguishes_id_position() {
        // Same single message, different mailbox ids must hash
        // differently. Burn an id slot in the second registry.
        let mut a = MailboxRegistry::new();
        let id_a = a.register();
        a.get_mut(id_a).unwrap().send(99);

        let mut b = MailboxRegistry::new();
        let _burn = b.register();
        let id_b = b.register();
        b.get_mut(id_b).unwrap().send(99);

        assert_ne!(a.state_hash(), b.state_hash());
    }
}
