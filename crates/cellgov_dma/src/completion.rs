//! `DmaCompletion` -- modeled completion event for a single
//! [`DmaRequest`].
//!
//! A completion is the runtime's way of saying "this DMA transfer
//! becomes guest-visible at this guest tick". It is a pure value
//! packet: no callbacks, no host pointers, no scheduler hooks. The
//! request that produced it is carried verbatim so the commit pipeline
//! can apply the modeled transfer when the completion fires, without
//! needing a separate side table to look the request up.
//!
//! This module owns the completion value type and its accessors. The
//! DMA queue in [`crate::queue`] orders completions by
//! `(completion_time, sequence)`, and the commit pipeline applies the
//! modeled transfer when each completion fires.

use crate::request::{DmaDirection, DmaRequest};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_time::GuestTicks;

/// A modeled DMA completion event.
///
/// Pairs the original [`DmaRequest`] with the [`GuestTicks`] at which
/// the runtime considers the transfer guest-visible. The completion
/// time is computed by an implementation of
/// [`crate::DmaLatencyModel`] when the request is enqueued; this type
/// just carries the result.
///
/// `DmaCompletion` is `Copy + Eq + Hash`. Ordering is **not** derived:
/// the DMA queue orders completions by
/// `(completion_time, sequence_number)` where the sequence number is
/// queue-assigned, not part of the completion value itself. Deriving
/// `Ord` here would pin a different ordering and fight the queue's
/// tiebreak scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmaCompletion {
    request: DmaRequest,
    completion_time: GuestTicks,
}

impl DmaCompletion {
    /// Construct a `DmaCompletion` for `request`, scheduled to fire at
    /// `completion_time`.
    #[inline]
    pub const fn new(request: DmaRequest, completion_time: GuestTicks) -> Self {
        Self {
            request,
            completion_time,
        }
    }

    /// The request that produced this completion.
    #[inline]
    pub const fn request(self) -> DmaRequest {
        self.request
    }

    /// The guest-time tick at which the runtime considers this
    /// transfer visible to the rest of the guest model.
    #[inline]
    pub const fn completion_time(self) -> GuestTicks {
        self.completion_time
    }

    /// Convenience: the unit that issued the underlying request.
    /// Recorded so the runtime can route the wake event back to the
    /// right waiter.
    #[inline]
    pub const fn issuer(self) -> UnitId {
        self.request.issuer()
    }

    /// Convenience: the direction of the underlying transfer.
    #[inline]
    pub const fn direction(self) -> DmaDirection {
        self.request.direction()
    }

    /// Convenience: the source byte range of the underlying transfer.
    #[inline]
    pub const fn source(self) -> ByteRange {
        self.request.source()
    }

    /// Convenience: the destination byte range of the underlying
    /// transfer.
    #[inline]
    pub const fn destination(self) -> ByteRange {
        self.request.destination()
    }

    /// Convenience: the length of the underlying transfer in bytes.
    #[inline]
    pub const fn length(self) -> u64 {
        self.request.length()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_mem::GuestAddr;

    fn range(start: u64, length: u64) -> ByteRange {
        ByteRange::new(GuestAddr::new(start), length).expect("range fits")
    }

    fn sample_request() -> DmaRequest {
        DmaRequest::new(
            DmaDirection::Put,
            range(0x1000, 0x40),
            range(0x9000, 0x40),
            UnitId::new(3),
        )
        .expect("equal lengths")
    }

    #[test]
    fn construction_carries_request_and_time() {
        let req = sample_request();
        let c = DmaCompletion::new(req, GuestTicks::new(500));
        assert_eq!(c.request(), req);
        assert_eq!(c.completion_time(), GuestTicks::new(500));
    }

    #[test]
    fn convenience_accessors_delegate_to_request() {
        let req = sample_request();
        let c = DmaCompletion::new(req, GuestTicks::new(0));
        assert_eq!(c.issuer(), UnitId::new(3));
        assert_eq!(c.direction(), DmaDirection::Put);
        assert_eq!(c.source(), range(0x1000, 0x40));
        assert_eq!(c.destination(), range(0x9000, 0x40));
        assert_eq!(c.length(), 0x40);
    }

    #[test]
    fn equality_compares_request_and_time() {
        let req = sample_request();
        let a = DmaCompletion::new(req, GuestTicks::new(100));
        let b = DmaCompletion::new(req, GuestTicks::new(100));
        let c = DmaCompletion::new(req, GuestTicks::new(101));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn equality_distinguishes_request() {
        let req_a = sample_request();
        let req_b = DmaRequest::new(
            DmaDirection::Get,
            range(0x1000, 0x40),
            range(0x9000, 0x40),
            UnitId::new(3),
        )
        .unwrap();
        let a = DmaCompletion::new(req_a, GuestTicks::new(50));
        let b = DmaCompletion::new(req_b, GuestTicks::new(50));
        assert_ne!(a, b);
    }

    #[test]
    fn copy_semantics_hold() {
        // DmaCompletion is Copy by construction. This test exists so
        // accidental field changes that break Copy are caught loudly.
        let c = DmaCompletion::new(sample_request(), GuestTicks::new(7));
        let d = c;
        assert_eq!(c, d);
        // Both still usable after the copy.
        assert_eq!(c.completion_time(), d.completion_time());
    }

    #[test]
    fn zero_length_completion_is_well_formed() {
        let req = DmaRequest::new(
            DmaDirection::Put,
            range(0x1000, 0),
            range(0x9000, 0),
            UnitId::new(1),
        )
        .unwrap();
        let c = DmaCompletion::new(req, GuestTicks::ZERO);
        assert_eq!(c.length(), 0);
        assert_eq!(c.completion_time(), GuestTicks::ZERO);
    }

    #[test]
    fn completion_time_zero_is_legal() {
        // A latency model is free to return GuestTicks::ZERO for an
        // immediate completion; the value type does not reject it.
        let c = DmaCompletion::new(sample_request(), GuestTicks::ZERO);
        assert_eq!(c.completion_time(), GuestTicks::ZERO);
    }
}
