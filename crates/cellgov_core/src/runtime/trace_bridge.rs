//! Trace enum bridges (`cellgov_effects::Effect` /
//! `cellgov_exec::YieldReason` onto their `cellgov_trace` twins) and
//! the read-only `MemoryView` the runtime hands to `Lv2Host::dispatch`.
//! The bridges are exhaustive matches: `cellgov_trace` sits below both
//! source crates in the workspace DAG and cannot name the source
//! variants, so adding a new variant breaks compilation here and
//! forces the trace contract to update alongside.

use cellgov_exec::YieldReason;
use cellgov_lv2::Lv2Runtime;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;
use cellgov_trace::{TracedEffectKind, TracedYieldReason};

/// Map an `Effect` onto its `TracedEffectKind` twin.
pub(super) fn traced_effect_kind(e: &cellgov_effects::Effect) -> TracedEffectKind {
    use cellgov_effects::Effect;
    match e {
        Effect::SharedWriteIntent { .. } => TracedEffectKind::SharedWriteIntent,
        Effect::MailboxSend { .. } => TracedEffectKind::MailboxSend,
        Effect::MailboxReceiveAttempt { .. } => TracedEffectKind::MailboxReceiveAttempt,
        Effect::DmaEnqueue { .. } => TracedEffectKind::DmaEnqueue,
        Effect::WaitOnEvent { .. } => TracedEffectKind::WaitOnEvent,
        Effect::WakeUnit { .. } => TracedEffectKind::WakeUnit,
        Effect::SignalUpdate { .. } => TracedEffectKind::SignalUpdate,
        Effect::FaultRaised { .. } => TracedEffectKind::FaultRaised,
        Effect::TraceMarker { .. } => TracedEffectKind::TraceMarker,
        Effect::ReservationAcquire { .. } => TracedEffectKind::ReservationAcquire,
        Effect::ConditionalStore { .. } => TracedEffectKind::ConditionalStore,
        Effect::RsxLabelWrite { .. } => TracedEffectKind::RsxLabelWrite,
        Effect::RsxFlipRequest { .. } => TracedEffectKind::RsxFlipRequest,
    }
}

/// Map a runtime [`YieldReason`] onto its [`TracedYieldReason`] twin.
pub(super) fn traced_yield_reason(y: YieldReason) -> TracedYieldReason {
    match y {
        YieldReason::BudgetExhausted => TracedYieldReason::BudgetExhausted,
        YieldReason::MailboxAccess => TracedYieldReason::MailboxAccess,
        YieldReason::DmaSubmitted => TracedYieldReason::DmaSubmitted,
        YieldReason::DmaWait => TracedYieldReason::DmaWait,
        YieldReason::WaitingSync => TracedYieldReason::WaitingSync,
        YieldReason::Syscall => TracedYieldReason::Syscall,
        YieldReason::InterruptBoundary => TracedYieldReason::InterruptBoundary,
        YieldReason::Fault => TracedYieldReason::Fault,
        YieldReason::Finished => TracedYieldReason::Finished,
    }
}

/// Read-only view of committed memory + tick snapshot, handed to
/// `Lv2Host::dispatch`; constructed fresh per dispatch call.
pub(super) struct MemoryView<'a> {
    pub(super) memory: &'a GuestMemory,
    pub(super) current_tick: GuestTicks,
}

impl Lv2Runtime for MemoryView<'_> {
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
        // Region-aware: addresses outside the main region (stacks,
        // child stacks, RSX, SPU-reserved) are still valid for the
        // host to inspect. Linear `.as_bytes()` only covers main, so
        // route through `GuestMemory::read` which handles the
        // multi-region layout.
        use cellgov_mem::ByteRange;
        let range = ByteRange::new(cellgov_mem::GuestAddr::new(addr), len as u64)?;
        self.memory.read(range)
    }

    fn current_tick(&self) -> GuestTicks {
        self.current_tick
    }

    fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]> {
        // GuestMemory's region-aware read returns None when a range
        // straddles a region boundary, so the natural strategy is:
        // find the region containing `addr`, scan up to the region
        // end (or max_len, whichever is smaller) for the terminator,
        // and return the prefix.
        use cellgov_mem::ByteRange;
        let region_remaining = {
            // GuestMemory exposes containing_region but only the
            // base/size, not the slice; round-trip through `read`
            // with length 1 to confirm the address is mapped, then
            // probe upward.
            let probe = ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)?;
            self.memory.read(probe)?;
            // Now find the largest readable window <= max_len.
            // Bisect: read length doubles until it fails, then we
            // walk down. For paths typically <100 bytes this is
            // fast; the worst case is one miss per region boundary.
            let mut hi = max_len;
            let mut lo = 1;
            while hi > lo {
                let mid = (lo + hi).div_ceil(2);
                let r = ByteRange::new(cellgov_mem::GuestAddr::new(addr), mid as u64);
                if r.and_then(|r| self.memory.read(r)).is_some() {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            lo
        };
        let window_range =
            ByteRange::new(cellgov_mem::GuestAddr::new(addr), region_remaining as u64)?;
        let window = self.memory.read(window_range)?;
        let nul_pos = window.iter().position(|&b| b == terminator)?;
        Some(&window[..nul_pos])
    }

    fn writable(&self, addr: u64, len: usize) -> bool {
        use cellgov_mem::RegionAccess;
        let Some(end) = (addr).checked_add(len as u64) else {
            return false;
        };
        let Some(region) = self.memory.containing_region(addr, end - addr) else {
            return false;
        };
        match region.access() {
            RegionAccess::ReadWrite => true,
            RegionAccess::ReservedZeroReadable | RegionAccess::ReservedStrict => false,
        }
    }
}
