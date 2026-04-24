//! Immutable DMA request packet and direction enum.
//!
//! Completion timing is decided later by an implementation of
//! [`crate::DmaLatencyModel`]; the transfer itself is applied through the
//! commit pipeline.

use cellgov_event::UnitId;
use cellgov_mem::ByteRange;

/// Direction of a modeled DMA transfer.
///
/// Variant order is part of the determinism contract for any containing
/// type that derives `Ord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DmaDirection {
    /// Destination range is globally visible (SPU `put` shape).
    Put = 0,
    /// Source range is globally visible (SPU `get` shape).
    Get = 1,
}

/// An immutable DMA request packet.
///
/// Invariant: `source.length() == destination.length()`. Enforced by
/// [`DmaRequest::new`].
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
    /// # Errors
    ///
    /// Returns `None` if `source.length() != destination.length()`. A
    /// zero-length transfer is permitted.
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

    /// Direction of the transfer.
    #[inline]
    pub const fn direction(self) -> DmaDirection {
        self.direction
    }

    /// Source range; interpretation depends on [`Self::direction`].
    #[inline]
    pub const fn source(self) -> ByteRange {
        self.source
    }

    /// Destination range; interpretation depends on [`Self::direction`].
    #[inline]
    pub const fn destination(self) -> ByteRange {
        self.destination
    }

    /// Unit that issued the request. Used to route the completion wake
    /// event back to the right waiter.
    #[inline]
    pub const fn issuer(self) -> UnitId {
        self.issuer
    }

    /// Length of the transfer in bytes.
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
