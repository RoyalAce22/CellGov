//! `DmaLatencyModel` -- the policy seam for modeled DMA completion timing.
//!
//! The runtime decides *when* a DMA completion becomes visible by asking
//! a `DmaLatencyModel` implementation. Keeping the policy behind a trait
//! lets the current deterministic uniform model coexist with later
//! bandwidth/contention-aware models without rewriting the issue path.
//! Implementations must be deterministic: the same request issued at the
//! same `now` must always return the same completion time. No host time,
//! no thread-local state, no `HashMap` iteration order.

use crate::request::DmaRequest;
use cellgov_time::GuestTicks;

/// Fixed-delay latency model: every DMA completes exactly `ticks`
/// guest ticks after issue, regardless of transfer length or
/// direction. Trivially deterministic and monotone.
pub struct FixedLatency {
    ticks: u64,
}

impl FixedLatency {
    /// Construct a fixed-delay model. A `ticks` value of 0 means
    /// immediate completion (at `now`).
    #[inline]
    pub const fn new(ticks: u64) -> Self {
        Self { ticks }
    }
}

impl DmaLatencyModel for FixedLatency {
    fn completion_time(&self, _req: &DmaRequest, now: GuestTicks) -> GuestTicks {
        now.checked_add(GuestTicks::new(self.ticks))
            .expect("completion time within u64 range")
    }
}

/// Computes the modeled completion time for a DMA request.
///
/// Implementations are pure functions of `(request, now)` and any state
/// the model itself owns. They must be deterministic: identical inputs
/// must produce identical outputs across runs and across hosts. They
/// must also be monotone with respect to `now` -- a request issued
/// later must never complete earlier than the same request issued
/// earlier under the same model. The runtime relies on this to keep
/// the event queue sorted without re-validation.
pub trait DmaLatencyModel {
    /// Return the guest-time tick at which `req` should be considered
    /// complete, given that it is being issued at `now`. The returned
    /// time must satisfy `>= now` -- a completion in the past is a
    /// model bug.
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks) -> GuestTicks;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::DmaDirection;
    use cellgov_event::UnitId;
    use cellgov_mem::{ByteRange, GuestAddr};

    /// Test-only model: completion at `now + ceil(length / bytes_per_tick)`.
    /// Pure, deterministic, monotone in `now`.
    struct LinearLatency {
        bytes_per_tick: u64,
    }

    impl DmaLatencyModel for LinearLatency {
        fn completion_time(&self, req: &DmaRequest, now: GuestTicks) -> GuestTicks {
            let len = req.length();
            let ticks = len.div_ceil(self.bytes_per_tick);
            now.checked_add(GuestTicks::new(ticks))
                .expect("completion time within u64 range")
        }
    }

    fn req(length: u64) -> DmaRequest {
        DmaRequest::new(
            DmaDirection::Put,
            ByteRange::new(GuestAddr::new(0x1000), length).unwrap(),
            ByteRange::new(GuestAddr::new(0x9000), length).unwrap(),
            UnitId::new(0),
        )
        .unwrap()
    }

    #[test]
    fn linear_model_basic() {
        let model = LinearLatency { bytes_per_tick: 16 };
        let r = req(64);
        let t = model.completion_time(&r, GuestTicks::new(100));
        assert_eq!(t, GuestTicks::new(104));
    }

    #[test]
    fn linear_model_round_up() {
        let model = LinearLatency { bytes_per_tick: 16 };
        let r = req(17);
        let t = model.completion_time(&r, GuestTicks::new(0));
        // 17 bytes / 16 bytes/tick = 2 ticks (ceil)
        assert_eq!(t, GuestTicks::new(2));
    }

    #[test]
    fn linear_model_zero_length_completes_at_now() {
        let model = LinearLatency { bytes_per_tick: 16 };
        let r = req(0);
        let t = model.completion_time(&r, GuestTicks::new(50));
        assert_eq!(t, GuestTicks::new(50));
    }

    #[test]
    fn linear_model_is_deterministic() {
        let model = LinearLatency { bytes_per_tick: 8 };
        let r = req(40);
        let now = GuestTicks::new(1000);
        let a = model.completion_time(&r, now);
        let b = model.completion_time(&r, now);
        assert_eq!(a, b);
    }

    #[test]
    fn linear_model_is_monotone_in_now() {
        let model = LinearLatency { bytes_per_tick: 8 };
        let r = req(40);
        let earlier = model.completion_time(&r, GuestTicks::new(100));
        let later = model.completion_time(&r, GuestTicks::new(200));
        assert!(earlier < later);
    }
}
