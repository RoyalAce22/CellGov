//! Mailbox identifier and the mailbox FIFO state machine.
//!
//! `MailboxId` is the leaf identifier the runtime hands out at mailbox
//! registration time and is the payload of `Effect::MailboxSend` and
//! `Effect::MailboxReceiveAttempt`. It must exist before the effect
//! enum can be designed; that is why it landed in an earlier slice.
//!
//! `Mailbox` is the abstract FIFO. It owns the queued message words
//! and exposes deterministic `send` / `try_receive` operations. It
//! does not produce block/wake conditions yet; those land in a
//! follow-up slice that wires the FIFO into the runtime's commit
//! pipeline and event queue. Sync state machines do not themselves
//! decide scheduling order -- this type stays free of any scheduler
//! awareness.
//!
//! Messages are stored as raw `u32` words rather than as
//! `cellgov_effects::MailboxMessage` because the workspace DAG runs
//! `effects -> sync`, not the other way around. The integration layer
//! wraps/unwraps at the effect boundary.

/// A stable identifier for a mailbox instance in the runtime.
///
/// `MailboxId`s are assigned by the runtime at mailbox registration time
/// and are recorded in the trace. They must be unique within a single
/// runtime instance; reuse across runs is allowed and expected for
/// replay. There is no `From<u64>` impl on purpose -- ad-hoc id
/// fabrication outside the registry should be visible at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MailboxId(u64);

impl MailboxId {
    /// Construct a `MailboxId` from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying id value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A deterministic FIFO mailbox.
///
/// Backed by a `VecDeque<u32>` which preserves insertion order
/// independent of host `HashMap` iteration order or thread timing.
/// No host-time inputs or hash iteration order influence the
/// result. The FIFO is currently unbounded; capacity / blocking-on-full
/// semantics land as separate slices when there is a real need.
///
/// `Mailbox` owns the queued words and nothing else: no `MailboxId`
/// (the registry that owns the mailbox knows its id), no event-queue
/// handle, no waiter list. Those are integration-layer concerns.
#[derive(Debug, Clone, Default)]
pub struct Mailbox {
    queue: std::collections::VecDeque<u32>,
}

impl Mailbox {
    /// Construct an empty mailbox.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `message` to the back of the FIFO.
    #[inline]
    pub fn send(&mut self, message: u32) {
        self.queue.push_back(message);
    }

    /// Pop the oldest queued message, if any. Returns `None` when the
    /// FIFO is empty; the integration layer translates that into a
    /// block condition for the receiving unit in a future slice.
    #[inline]
    pub fn try_receive(&mut self) -> Option<u32> {
        self.queue.pop_front()
    }

    /// Borrow the oldest queued message without removing it. Useful
    /// for trace and assertion paths that need to inspect mailbox
    /// state at a checkpoint.
    #[inline]
    pub fn peek(&self) -> Option<u32> {
        self.queue.front().copied()
    }

    /// Number of messages currently queued.
    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the FIFO holds any messages.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        assert_eq!(MailboxId::new(42).raw(), 42);
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(MailboxId::default(), MailboxId::new(0));
    }

    #[test]
    fn ordering_is_total() {
        assert!(MailboxId::new(1) < MailboxId::new(2));
        assert_eq!(MailboxId::new(7), MailboxId::new(7));
    }

    #[test]
    fn new_mailbox_is_empty() {
        let m = Mailbox::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert_eq!(m.peek(), None);
    }

    #[test]
    fn send_then_try_receive_returns_in_fifo_order() {
        let mut m = Mailbox::new();
        m.send(1);
        m.send(2);
        m.send(3);
        assert_eq!(m.len(), 3);
        assert_eq!(m.try_receive(), Some(1));
        assert_eq!(m.try_receive(), Some(2));
        assert_eq!(m.try_receive(), Some(3));
        assert_eq!(m.try_receive(), None);
        assert!(m.is_empty());
    }

    #[test]
    fn try_receive_on_empty_returns_none() {
        let mut m = Mailbox::new();
        assert_eq!(m.try_receive(), None);
        // Repeated receives stay None; no spurious side effects.
        assert_eq!(m.try_receive(), None);
        assert!(m.is_empty());
    }

    #[test]
    fn peek_does_not_consume() {
        let mut m = Mailbox::new();
        m.send(0xdead_beef);
        assert_eq!(m.peek(), Some(0xdead_beef));
        assert_eq!(m.peek(), Some(0xdead_beef));
        assert_eq!(m.len(), 1);
        assert_eq!(m.try_receive(), Some(0xdead_beef));
        assert_eq!(m.peek(), None);
    }

    #[test]
    fn interleaved_send_and_receive_preserves_order() {
        // A roundtrip pattern: send a, receive a, send b c, receive b c.
        let mut m = Mailbox::new();
        m.send(10);
        assert_eq!(m.try_receive(), Some(10));
        m.send(20);
        m.send(30);
        assert_eq!(m.try_receive(), Some(20));
        m.send(40);
        assert_eq!(m.try_receive(), Some(30));
        assert_eq!(m.try_receive(), Some(40));
        assert_eq!(m.try_receive(), None);
    }

    #[test]
    fn clone_is_independent() {
        let mut a = Mailbox::new();
        a.send(1);
        a.send(2);
        let mut b = a.clone();
        a.try_receive();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 2);
        assert_eq!(b.try_receive(), Some(1));
        assert_eq!(b.try_receive(), Some(2));
    }
}
