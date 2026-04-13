//! Payload types carried inside `Effect` variants.
//!
//! Each type here is a small immutable value: a write byte buffer, a
//! mailbox message word, a wait target classifier, and a fault kind. They
//! live in their own module so the [`crate::effect::Effect`] enum stays
//! scannable, and so the payload types can be tested in isolation.

use cellgov_sync::{BarrierId, MailboxId, SignalId};

/// Max bytes stored inline without heap allocation.
const INLINE_CAP: usize = 16;

/// The bytes a `SharedWriteIntent` will deposit into its target range.
///
/// Payloads up to 16 bytes are stored inline on the stack. Larger
/// payloads spill to a heap-allocated `Vec<u8>`. All current PPU
/// stores (1/2/4/8/16 bytes) and LV2 writes fit within the inline
/// buffer, so the hot path never allocates.
///
/// The runtime checks that `payload.len() == range.length()` at commit
/// validation time; that check is not duplicated here so that this type
/// can be constructed in isolation in tests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WritePayload {
    storage: PayloadStorage,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PayloadStorage {
    Inline { buf: [u8; INLINE_CAP], len: u8 },
    Heap(Vec<u8>),
}

impl WritePayload {
    /// Construct a `WritePayload` from owned bytes.
    #[inline]
    pub fn new(bytes: Vec<u8>) -> Self {
        if bytes.len() <= INLINE_CAP {
            let mut buf = [0u8; INLINE_CAP];
            buf[..bytes.len()].copy_from_slice(&bytes);
            Self {
                storage: PayloadStorage::Inline {
                    buf,
                    len: bytes.len() as u8,
                },
            }
        } else {
            Self {
                storage: PayloadStorage::Heap(bytes),
            }
        }
    }

    /// Construct a `WritePayload` from a byte slice. Avoids an
    /// intermediate `Vec` allocation for slices that fit inline.
    #[inline]
    pub fn from_slice(src: &[u8]) -> Self {
        if src.len() <= INLINE_CAP {
            let mut buf = [0u8; INLINE_CAP];
            buf[..src.len()].copy_from_slice(src);
            Self {
                storage: PayloadStorage::Inline {
                    buf,
                    len: src.len() as u8,
                },
            }
        } else {
            Self {
                storage: PayloadStorage::Heap(src.to_vec()),
            }
        }
    }

    /// View the payload bytes.
    #[inline]
    pub fn bytes(&self) -> &[u8] {
        match &self.storage {
            PayloadStorage::Inline { buf, len } => &buf[..*len as usize],
            PayloadStorage::Heap(v) => v,
        }
    }

    /// Length of the payload in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        match &self.storage {
            PayloadStorage::Inline { len, .. } => *len as usize,
            PayloadStorage::Heap(v) => v.len(),
        }
    }

    /// Whether the payload carries zero bytes. Zero-length writes are
    /// degenerate but legal -- the runtime trace still records them.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
    fn from_slice_matches_new() {
        let data = [0xDE, 0xAD, 0xBE, 0xEF];
        let a = WritePayload::new(data.to_vec());
        let b = WritePayload::from_slice(&data);
        assert_eq!(a, b);
        assert_eq!(b.bytes(), &data);
        assert_eq!(b.len(), 4);
    }

    #[test]
    fn from_slice_empty() {
        let p = WritePayload::from_slice(&[]);
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
        assert_eq!(p.bytes(), &[]);
    }

    #[test]
    fn inline_at_boundary() {
        // Exactly INLINE_CAP bytes should stay inline.
        let data = [0xAB; INLINE_CAP];
        let p = WritePayload::from_slice(&data);
        assert_eq!(p.len(), INLINE_CAP);
        assert_eq!(p.bytes(), &data);
        // Verify it matches the Vec path.
        assert_eq!(p, WritePayload::new(data.to_vec()));
    }

    #[test]
    fn heap_above_boundary() {
        // INLINE_CAP + 1 bytes should spill to heap.
        let data = vec![0xCD; INLINE_CAP + 1];
        let p = WritePayload::from_slice(&data);
        assert_eq!(p.len(), INLINE_CAP + 1);
        assert_eq!(p.bytes(), data.as_slice());
        assert_eq!(p, WritePayload::new(data));
    }

    #[test]
    fn clone_preserves_storage() {
        let inline = WritePayload::from_slice(&[1, 2, 3]);
        let heap = WritePayload::from_slice(&[0xFF; INLINE_CAP + 1]);
        assert_eq!(inline.clone(), inline);
        assert_eq!(heap.clone(), heap);
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
