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
        let bytes = self.memory.as_bytes();
        let start = addr as usize;
        let end = start.checked_add(len)?;
        if end <= bytes.len() {
            Some(&bytes[start..end])
        } else {
            None
        }
    }

    fn current_tick(&self) -> GuestTicks {
        self.current_tick
    }
}
