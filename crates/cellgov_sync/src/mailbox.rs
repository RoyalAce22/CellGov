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
#[path = "tests/mailbox_tests.rs"]
mod tests;
