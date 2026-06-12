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
    tag_id: Option<u8>,
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
            tag_id: None,
        })
    }

    /// Attach the MFC tag-id the SPU issued under. Completion publishes
    /// `1 << tag_id` to the issuer's tag-status channel.
    #[inline]
    pub const fn with_tag_id(mut self, tag_id: u8) -> Self {
        self.tag_id = Some(tag_id);
        self
    }

    /// MFC tag-id the SPU issued under; `None` for PPU/host-initiated DMA.
    #[inline]
    pub const fn tag_id(self) -> Option<u8> {
        self.tag_id
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
#[path = "tests/request_tests.rs"]
mod tests;
