//! Mailbox FIFO state machine and its leaf identifier. Empty-receive
//! and full-send outcomes become block/wake events upstream; this
//! module does not decide scheduling.
//!
//! Messages are raw `u32` words rather than `cellgov_effects::MailboxMessage`
//! because the crate DAG runs `effects -> sync`, not the reverse.

/// Stable identifier for a mailbox instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MailboxId(u64);

impl MailboxId {
    /// Construct a `MailboxId` from a raw value.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Deterministic unbounded FIFO mailbox.
///
/// Backed by `VecDeque<u32>`; insertion order is the only order.
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

    /// Pop the oldest queued message. `None` means the integration
    /// layer should translate the attempt into a block condition.
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
