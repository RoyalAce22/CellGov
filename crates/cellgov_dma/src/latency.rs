//! Policy seam for modeled DMA completion timing.

use crate::request::DmaRequest;
use cellgov_time::GuestTicks;

/// Fixed-delay latency model: every DMA completes `ticks` after issue.
pub struct FixedLatency {
    ticks: u64,
}

impl FixedLatency {
    /// `ticks == 0` means immediate completion at `now`.
    #[inline]
    pub const fn new(ticks: u64) -> Self {
        Self { ticks }
    }
}

impl DmaLatencyModel for FixedLatency {
    /// # Panics
    ///
    /// Panics if `now + ticks` overflows `u64`. Under the deterministic
    /// time model `now` is bounded by `GuestTicks` advances against a
    /// step-budget cap, so saturation is unreachable in any test or
    /// title-boot flow that completes in finite steps.
    fn completion_time(&self, _req: &DmaRequest, now: GuestTicks) -> GuestTicks {
        now.checked_add(GuestTicks::new(self.ticks))
            .expect("completion time within u64 range")
    }
}

/// Computes the modeled completion time for a DMA request.
///
/// Implementations must be a pure function of `(request, now)` and
/// implementation-owned state, deterministic across runs and hosts, and
/// monotone in `now`. The event queue relies on monotonicity to stay
/// sorted without re-validation.
pub trait DmaLatencyModel {
    /// Guest tick at which `req` is considered complete, given issue at
    /// `now`. Must satisfy `>= now`.
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks) -> GuestTicks;
}

#[cfg(test)]
#[path = "tests/latency_tests.rs"]
mod tests;
