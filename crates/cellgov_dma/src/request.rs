//! `DmaRequest` and the DMA direction enum.
//!
//! A DMA request is a pure value packet describing a transfer that the
//! runtime will model. It carries the source and destination byte ranges,
//! the direction of the transfer, and the unit that issued it -- all data,
//! no callbacks, no host pointers, no scheduler hooks. Completion timing
//! is decided later by an implementation of [`crate::DmaLatencyModel`];
//! actual application of the transfer happens through the commit pipeline.
//!
//! The brief is explicit that DMA is not just `memcpy`. The runtime seam
//! must let a future asynchronous backend slot in without rewriting call
//! sites, which is why the request type is decoupled from any synchronous
//! "do it now" entry point.

use cellgov_event::UnitId;
use cellgov_mem::ByteRange;

/// Direction of a modeled DMA transfer.
///
/// `Put` is "guest-private memory out to globally visible memory" --
/// the SPU `put` shape: local store (or other unit-private region)
/// to effective address. `Get` is the reverse. The runtime never cares
/// about host buffers; both endpoints are in the guest model.
///
/// Variant order is part of the determinism contract for any code that
/// derives `Ord` on a containing type, so do not reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DmaDirection {
    /// Move bytes from the source range to the destination range,
    /// where the destination is the globally-visible side.
    Put = 0,
    /// Move bytes from the source range to the destination range,
    /// where the source is the globally-visible side.
    Get = 1,
}

/// An immutable DMA request packet.
///
/// `DmaRequest` is constructed at issue time by the unit that wants the
/// transfer, then handed to the runtime as data. It carries enough state
/// to model the transfer deterministically: the two endpoints, which way
/// the bytes flow, and which unit asked. It does not carry a completion
/// time -- that is computed by the latency model when the request is
/// scheduled, so the same request can be replayed under a different
/// latency policy without rewriting the request itself.
///
/// Validation note: source and destination ranges must have the same
/// length. This is checked at construction
/// ([`DmaRequest::new`] returns `None` on mismatch) so that downstream
/// commit-pipeline validation does not have to special-case it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmaRequest {
    direction: DmaDirection,
    source: ByteRange,
    destination: ByteRange,
    issuer: UnitId,
}

impl DmaRequest {
    /// Construct a `DmaRequest`.
    ///
    /// Returns `None` if `source.length() != destination.length()`.
    /// A zero-length transfer is permitted; it is a degenerate but
    /// well-defined no-op that still flows through the runtime so the
    /// trace records it.
    #[inline]
    pub const fn new(
        direction: DmaDirection,
        source: ByteRange,
        destination: ByteRange,
        issuer: UnitId,
    ) -> Option<Self> {
        if source.length() != destination.length() {
            return None;
        }
        Some(Self {
            direction,
            source,
            destination,
            issuer,
        })
    }

    /// Direction of the modeled transfer.
    #[inline]
    pub const fn direction(self) -> DmaDirection {
        self.direction
    }

    /// Source byte range. The semantics of "source" depend on
    /// [`DmaRequest::direction`].
    #[inline]
    pub const fn source(self) -> ByteRange {
        self.source
    }

    /// Destination byte range. The semantics of "destination" depend
    /// on [`DmaRequest::direction`].
    #[inline]
    pub const fn destination(self) -> ByteRange {
        self.destination
    }

    /// The unit that issued this request. Recorded so the runtime can
    /// route the modeled completion event back to the right waiter and
    /// so the trace attributes the transfer to the right source.
    #[inline]
    pub const fn issuer(self) -> UnitId {
        self.issuer
    }

    /// Length of the transfer in bytes. Always equal to both
    /// `source().length()` and `destination().length()` by construction.
    #[inline]
    pub const fn length(self) -> u64 {
        self.source.length()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_mem::GuestAddr;

    fn range(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).expect("range fits")
    }

    #[test]
    fn direction_ordering_is_locked() {
        assert!(DmaDirection::Put < DmaDirection::Get);
        assert_eq!(DmaDirection::Put as u8, 0);
        assert_eq!(DmaDirection::Get as u8, 1);
    }

    #[test]
    fn construction_basic() {
        let req = DmaRequest::new(
            DmaDirection::Put,
            range(0x1000, 0x40),
            range(0x9000, 0x40),
            UnitId::new(3),
        )
        .expect("equal lengths");
        assert_eq!(req.direction(), DmaDirection::Put);
        assert_eq!(req.source(), range(0x1000, 0x40));
        assert_eq!(req.destination(), range(0x9000, 0x40));
        assert_eq!(req.issuer(), UnitId::new(3));
        assert_eq!(req.length(), 0x40);
    }

    #[test]
    fn mismatched_lengths_rejected() {
        let req = DmaRequest::new(
            DmaDirection::Get,
            range(0x1000, 0x40),
            range(0x9000, 0x80),
            UnitId::new(0),
        );
        assert_eq!(req, None);
    }

    #[test]
    fn zero_length_transfer_allowed() {
        let req = DmaRequest::new(
            DmaDirection::Put,
            range(0x1000, 0),
            range(0x9000, 0),
            UnitId::new(1),
        );
        assert!(req.is_some());
        assert_eq!(req.unwrap().length(), 0);
    }

    #[test]
    fn requests_compare_equal_when_fields_match() {
        let a = DmaRequest::new(
            DmaDirection::Get,
            range(0x100, 0x10),
            range(0x200, 0x10),
            UnitId::new(7),
        )
        .unwrap();
        let b = DmaRequest::new(
            DmaDirection::Get,
            range(0x100, 0x10),
            range(0x200, 0x10),
            UnitId::new(7),
        )
        .unwrap();
        assert_eq!(a, b);
    }
}
