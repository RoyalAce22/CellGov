//! Payload types carried inside `Effect` variants.
//!
//! Each type here is a small immutable value: a write byte buffer, a
//! mailbox message word, a wait target classifier, and a fault kind. They
//! live in their own module so the [`crate::effect::Effect`] enum stays
//! scannable, and so the payload types can be tested in isolation.

use cellgov_sync::{BarrierId, MailboxId, SignalId};

/// The bytes a `SharedWriteIntent` will deposit into its target range.
///
/// `WritePayload` is a thin wrapper around `Vec<u8>` that exposes the
/// bytes by reference. The wrapper exists so the public effect API does
/// not leak `Vec<u8>` mutability and so future representations (small
/// inline buffer, sharded payloads, etc.) can slot in without changing
/// every call site.
///
/// The runtime checks that `payload.len() == range.length()` at commit
/// validation time; that check is not duplicated here so that this type
/// can be constructed in isolation in tests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WritePayload {
    bytes: Vec<u8>,
}

impl WritePayload {
    /// Construct a `WritePayload` from owned bytes.
    #[inline]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// View the payload bytes.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Length of the payload in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the payload carries zero bytes. Zero-length writes are
    /// degenerate but legal -- the runtime trace still records them.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// A single mailbox message.
///
/// Cell hardware mailboxes carry 32-bit words. This type stays faithful
/// to that width; wider or structured payloads can land later as
/// additional effect variants without disturbing this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MailboxMessage(u32);

impl MailboxMessage {
    /// Construct a mailbox message from a raw 32-bit word.
    #[inline]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the raw 32-bit word.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// What an `Effect::WaitOnEvent` is waiting for.
///
/// Covers the three sync primitive families that have leaf identifiers
/// in `cellgov_sync`: mailboxes, signal notification registers, and
/// barriers (which share their id type with mutexes and semaphores).
/// DMA waits are not represented here -- the
/// [`crate::Effect::DmaEnqueue`] flow uses the dedicated
/// `YieldReason::DmaWait` path on the unit side and a separate
/// completion-correlation mechanism on the runtime side; folding it into
/// `WaitTarget` would require a tag concept that is currently unresolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaitTarget {
    /// Wait for a message to arrive in a mailbox.
    Mailbox(MailboxId),
    /// Wait for a signal notification register to satisfy a condition.
    Signal(SignalId),
    /// Wait at a barrier (or barrier-shaped sync primitive).
    Barrier(BarrierId),
}

/// Coarse classification of a fault raised by a unit or by validation.
///
/// The variant set is deliberately small. Validation faults
/// are produced by the commit pipeline when an effect batch is malformed
/// (length mismatch, out-of-range write, etc.). Guest faults are produced
/// by the unit itself; the `code` is opaque to the runtime and rides
/// through the trace untouched. Architecture-specific fault taxonomies
/// (PPU machine check, SPU invalid channel, etc.) belong in their crates
/// later as wrapper enums on top of `Guest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultKind {
    /// The runtime rejected an effect during pre-commit validation.
    Validation,
    /// The unit raised a fault with an opaque architecture-defined code.
    Guest(u32),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_payload_basic() {
        let p = WritePayload::new(vec![1, 2, 3, 4]);
        assert_eq!(p.len(), 4);
        assert!(!p.is_empty());
        assert_eq!(p.bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn write_payload_empty_is_legal() {
        let p = WritePayload::new(vec![]);
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn write_payload_equality() {
        let a = WritePayload::new(vec![1, 2, 3]);
        let b = WritePayload::new(vec![1, 2, 3]);
        let c = WritePayload::new(vec![1, 2, 4]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn mailbox_message_roundtrip() {
        assert_eq!(MailboxMessage::new(0xdead).raw(), 0xdead);
        assert_eq!(MailboxMessage::default(), MailboxMessage::new(0));
    }

    #[test]
    fn mailbox_message_ordering() {
        assert!(MailboxMessage::new(1) < MailboxMessage::new(2));
    }

    #[test]
    fn wait_target_distinct_variants() {
        let m = WaitTarget::Mailbox(MailboxId::new(1));
        let s = WaitTarget::Signal(SignalId::new(1));
        let b = WaitTarget::Barrier(BarrierId::new(1));
        assert_ne!(m, s);
        assert_ne!(s, b);
        assert_ne!(m, b);
        assert_eq!(m, WaitTarget::Mailbox(MailboxId::new(1)));
    }

    #[test]
    fn wait_target_distinguishes_id() {
        assert_ne!(
            WaitTarget::Mailbox(MailboxId::new(1)),
            WaitTarget::Mailbox(MailboxId::new(2))
        );
    }

    #[test]
    fn fault_kind_validation_equality() {
        assert_eq!(FaultKind::Validation, FaultKind::Validation);
        assert_ne!(FaultKind::Validation, FaultKind::Guest(0));
    }

    #[test]
    fn fault_kind_guest_carries_code() {
        let a = FaultKind::Guest(7);
        let b = FaultKind::Guest(7);
        let c = FaultKind::Guest(8);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
