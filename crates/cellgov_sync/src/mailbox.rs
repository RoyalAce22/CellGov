//! Mailbox FIFO state machine and its leaf identifier. Empty-receive
//! and full-send outcomes become block/wake / overrun events
//! upstream; this module does not decide scheduling.
//!
//! Messages are raw `u32` words because the crate DAG runs
//! `effects -> sync`, not the reverse.
//!
//! Spec capacities passed by the caller: SPU outbound and outbound-
//! interrupt mailboxes hold 1 entry (write-blocking); SPU inbound
//! holds 4 (PPE writes overrun, oldest dropped).
// [CBE-Handbook p:533 s:19.6 Mailboxes Table 19-15] SPU_WrOutMbox depth=1 (write-blocking), SPU_RdInMbox depth=4 (read-blocking; PPE write overruns), SPU_WrOutIntrMbox depth=1 (write-blocking).
// [CBE-Handbook p:541 s:19.6.6.2 PPE Side] PPE writes to a full SPU Read Inbound Mailbox do not stall; mailbox message data is lost.
// [CBEA p:132 s:9.5 SPU Mailbox Channels] models the three Cell BE SPU mailbox channels as bounded u32 FIFOs.

/// Stable identifier for a mailbox instance.
///
/// No `Default`: a derived default would alias the first registered
/// mailbox. Use `Option<MailboxId>` for "no mailbox."
/// No `Ord`: opaque handles, not allocation-order keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::Display)]
#[display("{_0}")]
pub struct MailboxId(u64);

impl MailboxId {
    /// Construct from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value. Consumers: trace serialization,
    /// diagnostic output, and the registry's `Vec`-indexed storage.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl crate::registry::RegistryId for MailboxId {
    fn new(raw: u64) -> Self {
        Self::new(raw)
    }
    fn raw(self) -> u64 {
        Self::raw(self)
    }
}

impl crate::registry::RegistryValueHash for Mailbox {
    fn hash_into(&self, hasher: &mut cellgov_mem::Fnv1aHasher) {
        hasher.write(&(self.len() as u64).to_le_bytes());
        for &word in self.iter() {
            hasher.write(&word.to_le_bytes());
        }
    }
}

/// Deterministic bounded FIFO mailbox.
///
/// `capacity` is the only bound on full-send observability; an
/// unbounded queue would mask write-blocking and overrun semantics.
/// No `Default`: zero-capacity is meaningless. Use
/// [`Mailbox::with_capacity`].
#[derive(Debug, Clone)]
pub struct Mailbox {
    queue: std::collections::VecDeque<u32>,
    capacity: usize,
}

impl Mailbox {
    /// Construct an empty mailbox with the given queue depth.
    ///
    /// # Panics
    ///
    /// Debug-asserts `capacity > 0` -- a zero-capacity mailbox can
    /// never accept a message.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "Mailbox capacity must be nonzero");
        Self {
            queue: std::collections::VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Enqueue if room exists. Returns `false` (queue unchanged)
    /// when full. The integration layer translates `false` into the
    /// SPU-stall path for write-blocking outbound channels.
    // [CBE-Handbook p:533 s:19.6 Mailboxes Table 19-15] outbound channels are write-blocking; full-send returns false here so the SPU exec layer can yield.
    #[inline]
    #[must_use = "a false return means the message was dropped or the SPU should stall"]
    pub fn try_send(&mut self, message: u32) -> bool {
        if self.queue.len() >= self.capacity {
            return false;
        }
        self.queue.push_back(message);
        true
    }

    /// Force-enqueue, dropping the oldest entry on full. Models PPE
    /// writes to the SPU Read Inbound Mailbox: the spec says no PPE
    /// stall, mailbox message data is lost.
    // [CBE-Handbook p:541 s:19.6.6.2 PPE Side] PPE write to a full SPU_RdInMbox does not stall; oldest message is overwritten.
    #[inline]
    pub fn force_send(&mut self, message: u32) {
        if self.queue.len() >= self.capacity {
            self.queue.pop_front();
        }
        self.queue.push_back(message);
    }

    /// Pop the oldest queued message. `None` means the integration
    /// layer should translate the attempt into a block condition for
    /// read-blocking channels (SPU_RdInMbox when empty).
    #[inline]
    pub fn try_receive(&mut self) -> Option<u32> {
        self.queue.pop_front()
    }

    /// Inspect the oldest queued message without removing it.
    #[inline]
    pub fn peek(&self) -> Option<u32> {
        self.queue.front().copied()
    }

    /// Number of queued messages.
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the FIFO holds any messages.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Whether the FIFO is at capacity.
    #[inline]
    pub fn is_full(&self) -> bool {
        self.queue.len() >= self.capacity
    }

    /// Number of free slots. Maps to the channel count an `rchcnt`
    /// on a write channel returns, or the `SPU_In_Mbox_Count` field
    /// the PPE reads from `SPU_Mbox_Stat` before writing.
    // [CBE-Handbook p:541 s:19.6.6.2 PPE Side] SPU_In_Mbox_Count is the number of available entries the PPE may safely write before overrun.
    #[inline]
    pub fn remaining_capacity(&self) -> usize {
        self.capacity - self.queue.len()
    }

    /// Configured queue depth.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Front-to-back walk without consuming or cloning. The
    /// state-hash path requires an allocation-free iterator.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &u32> + '_ {
        self.queue.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash<T: Hash>(t: &T) -> u64 {
        let mut h = DefaultHasher::new();
        t.hash(&mut h);
        h.finish()
    }

    #[test]
    fn id_roundtrip() {
        assert_eq!(MailboxId::new(42).raw(), 42);
    }

    #[test]
    fn id_hash_matches_eq() {
        assert_eq!(hash(&MailboxId::new(7)), hash(&MailboxId::new(7)));
        assert_ne!(hash(&MailboxId::new(7)), hash(&MailboxId::new(8)));
    }

    #[test]
    fn id_copy_preserves_value() {
        let a = MailboxId::new(5);
        let b = a;
        assert_eq!(a, b);
        assert_eq!(a.raw(), 5);
    }

    #[test]
    fn id_display_emits_raw_integer() {
        assert_eq!(format!("{}", MailboxId::new(42)), "42");
    }

    #[test]
    fn id_max_roundtrips() {
        assert_eq!(MailboxId::new(u64::MAX).raw(), u64::MAX);
    }

    #[test]
    fn new_mailbox_is_empty_and_has_full_capacity() {
        let m = Mailbox::with_capacity(4);
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert_eq!(m.peek(), None);
        assert_eq!(m.capacity(), 4);
        assert_eq!(m.remaining_capacity(), 4);
        assert!(!m.is_full());
    }

    #[test]
    fn try_send_then_try_receive_returns_in_fifo_order() {
        let mut m = Mailbox::with_capacity(4);
        assert!(m.try_send(1));
        assert!(m.try_send(2));
        assert!(m.try_send(3));
        assert_eq!(m.len(), 3);
        assert_eq!(m.try_receive(), Some(1));
        assert_eq!(m.try_receive(), Some(2));
        assert_eq!(m.try_receive(), Some(3));
        assert_eq!(m.try_receive(), None);
        assert!(m.is_empty());
    }

    #[test]
    fn try_send_returns_false_when_full() {
        let mut m = Mailbox::with_capacity(2);
        assert!(m.try_send(10));
        assert!(m.try_send(20));
        assert!(m.is_full());
        assert!(!m.try_send(30));
        // Queue unchanged on full.
        assert_eq!(m.len(), 2);
        assert_eq!(m.try_receive(), Some(10));
        assert_eq!(m.try_receive(), Some(20));
    }

    #[test]
    fn force_send_overruns_oldest_on_full() {
        let mut m = Mailbox::with_capacity(2);
        m.force_send(1);
        m.force_send(2);
        m.force_send(3); // overrun: drops 1
        assert_eq!(m.len(), 2);
        assert_eq!(m.try_receive(), Some(2));
        assert_eq!(m.try_receive(), Some(3));
    }

    #[test]
    fn force_send_no_overrun_when_room() {
        let mut m = Mailbox::with_capacity(4);
        m.force_send(1);
        m.force_send(2);
        assert_eq!(m.len(), 2);
        assert_eq!(m.try_receive(), Some(1));
    }

    #[test]
    fn try_receive_on_empty_returns_none() {
        let mut m = Mailbox::with_capacity(4);
        assert_eq!(m.try_receive(), None);
        assert_eq!(m.try_receive(), None);
        assert!(m.is_empty());
    }

    #[test]
    fn peek_does_not_consume() {
        let mut m = Mailbox::with_capacity(4);
        assert!(m.try_send(0xdead_beef));
        assert_eq!(m.peek(), Some(0xdead_beef));
        assert_eq!(m.peek(), Some(0xdead_beef));
        assert_eq!(m.len(), 1);
        assert_eq!(m.try_receive(), Some(0xdead_beef));
        assert_eq!(m.peek(), None);
    }

    #[test]
    fn remaining_capacity_tracks_occupancy() {
        let mut m = Mailbox::with_capacity(4);
        assert_eq!(m.remaining_capacity(), 4);
        assert!(m.try_send(1));
        assert_eq!(m.remaining_capacity(), 3);
        m.try_receive();
        assert_eq!(m.remaining_capacity(), 4);
    }

    #[test]
    fn interleaved_send_and_receive_preserves_order() {
        let mut m = Mailbox::with_capacity(4);
        assert!(m.try_send(10));
        assert_eq!(m.try_receive(), Some(10));
        assert!(m.try_send(20));
        assert!(m.try_send(30));
        assert_eq!(m.try_receive(), Some(20));
        assert!(m.try_send(40));
        assert_eq!(m.try_receive(), Some(30));
        assert_eq!(m.try_receive(), Some(40));
        assert_eq!(m.try_receive(), None);
    }

    #[test]
    fn clone_is_independent() {
        let mut a = Mailbox::with_capacity(4);
        assert!(a.try_send(1));
        assert!(a.try_send(2));
        let mut b = a.clone();
        a.try_receive();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 2);
        assert_eq!(b.try_receive(), Some(1));
        assert_eq!(b.try_receive(), Some(2));
    }

    #[test]
    fn iter_walks_front_to_back_without_consuming() {
        let mut m = Mailbox::with_capacity(4);
        assert!(m.try_send(10));
        assert!(m.try_send(20));
        assert!(m.try_send(30));
        let collected: Vec<u32> = m.iter().copied().collect();
        assert_eq!(collected, vec![10, 20, 30]);
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn iter_on_empty_yields_nothing() {
        let m = Mailbox::with_capacity(4);
        assert_eq!(m.iter().count(), 0);
    }
}
