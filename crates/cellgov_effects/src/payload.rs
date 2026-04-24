//! Payload types carried inside `Effect` variants: write buffer, mailbox
//! word, wait-target classifier, fault kind.

use cellgov_sync::{BarrierId, MailboxId, SignalId};

/// Max bytes stored inline without heap allocation.
const INLINE_CAP: usize = 16;

/// Bytes a `SharedWriteIntent` or `ConditionalStore` will deposit.
///
/// Payloads up to 16 bytes stay inline on the stack; larger payloads
/// spill to `Vec<u8>`. All current PPU stores (1/2/4/8/16 bytes) and
/// LV2 writes fit inline, so the hot path does not allocate. Length
/// is matched against `range.length()` at commit validation rather
/// than at construction so the type stays test-constructible.
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
    /// Construct from owned bytes.
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

    /// Construct from a slice, avoiding an intermediate `Vec` when the
    /// payload fits inline.
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

    /// Length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        match &self.storage {
            PayloadStorage::Inline { len, .. } => *len as usize,
            PayloadStorage::Heap(v) => v.len(),
        }
    }

    /// Whether the payload carries zero bytes; zero-length writes are
    /// legal and still recorded in the trace.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A single mailbox message: one 32-bit word, matching Cell hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MailboxMessage(u32);

impl MailboxMessage {
    /// Construct from a raw 32-bit word.
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
/// Covers the three `cellgov_sync` primitive families with leaf ids.
/// DMA waits use a separate `YieldReason::DmaWait` path paired with
/// request-tag correlation, not `WaitTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaitTarget {
    /// Wait for a message to arrive in a mailbox.
    Mailbox(MailboxId),
    /// Wait for a signal notification register to satisfy a condition.
    Signal(SignalId),
    /// Wait at a barrier (or barrier-shaped sync primitive).
    Barrier(BarrierId),
}

/// Coarse fault classification.
///
/// Architecture-specific taxonomies (PPU machine check, SPU invalid
/// channel, etc.) layer as wrapper enums over `Guest(u32)`; the runtime
/// treats the code as opaque and passes it through the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FaultKind {
    /// Commit pipeline rejected an effect during pre-commit validation.
    Validation,
    /// Unit raised a fault with an architecture-defined code.
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
        let data = [0xAB; INLINE_CAP];
        let p = WritePayload::from_slice(&data);
        assert_eq!(p.len(), INLINE_CAP);
        assert_eq!(p.bytes(), &data);
        assert_eq!(p, WritePayload::new(data.to_vec()));
    }

    #[test]
    fn heap_above_boundary() {
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
