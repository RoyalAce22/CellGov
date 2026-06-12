//! Modeled completion event for a single [`DmaRequest`].

use crate::request::{DmaDirection, DmaRequest};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_time::GuestTicks;

/// A modeled DMA completion event.
///
/// `Ord` is not derived because [`crate::DmaQueue`] orders completions
/// by `(completion_time, queue-assigned sequence)`, and the sequence is
/// not part of this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmaCompletion {
    request: DmaRequest,
    completion_time: GuestTicks,
}

impl DmaCompletion {
    /// Pair `request` with the tick at which it becomes visible.
    #[inline]
    pub const fn new(request: DmaRequest, completion_time: GuestTicks) -> Self {
        Self {
            request,
            completion_time,
        }
    }

    /// The originating request.
    #[inline]
    pub const fn request(self) -> DmaRequest {
        self.request
    }

    /// Guest tick at which the transfer becomes visible to the rest of
    /// the guest model.
    #[inline]
    pub const fn completion_time(self) -> GuestTicks {
        self.completion_time
    }

    /// Issuer of the underlying request.
    #[inline]
    pub const fn issuer(self) -> UnitId {
        self.request.issuer()
    }

    /// Direction of the underlying request.
    #[inline]
    pub const fn direction(self) -> DmaDirection {
        self.request.direction()
    }

    /// Source range of the underlying request.
    #[inline]
    pub const fn source(self) -> ByteRange {
        self.request.source()
    }

    /// Destination range of the underlying request.
    #[inline]
    pub const fn destination(self) -> ByteRange {
        self.request.destination()
    }

    /// Transfer length in bytes.
    #[inline]
    pub const fn length(self) -> u64 {
        self.request.length()
    }
}

#[cfg(test)]
#[path = "tests/completion_tests.rs"]
mod tests;
