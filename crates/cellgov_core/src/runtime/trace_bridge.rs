//! Two distinct concerns currently live here -- they will likely
//! split once either grows:
//!
//! 1. **Trace bridges.** `traced_effect_kind` and
//!    `traced_yield_reason` route `cellgov_effects::Effect` and
//!    `cellgov_exec::YieldReason` variants onto their
//!    `cellgov_trace` twins. `cellgov_trace` sits below both
//!    source crates in the workspace DAG and cannot name the
//!    source variants directly, so the bridges are exhaustive
//!    matches -- adding a new variant on either source enum
//!    breaks compilation here, forcing the trace contract to
//!    update alongside the runtime.
//!
//! 2. **LV2 memory view.** `MemoryView` is the read-only window
//!    the runtime hands to `Lv2Host::dispatch`. It is an LV2
//!    concern, not a trace concern, and lives in this module
//!    only because it is a one-method wrapper today. If it ever
//!    grows (e.g. to expose reservation state or epoch to the
//!    LV2 host), split it into `lv2_memory_view.rs`.

use cellgov_exec::YieldReason;
use cellgov_lv2::Lv2Runtime;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;
use cellgov_trace::{TracedEffectKind, TracedYieldReason};

/// Map an `Effect` onto its `TracedEffectKind` twin.
///
/// Same DAG situation as `traced_yield_reason`: `cellgov_trace` sits
/// below `cellgov_effects` in the workspace and cannot import the
/// source enum, so the bridge is an exhaustive match. Adding a new
/// `Effect` variant breaks compilation here and forces the trace
/// contract to update at the same time.
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
///
/// The two enums live in different crates (`cellgov_exec` and
/// `cellgov_trace`) because the trace crate sits below `cellgov_exec`
/// in the workspace DAG and cannot depend on it. Their discriminants
/// match one-for-one, but Rust does not let us cast between
/// distinct enum types directly, so we route through this exhaustive
/// match. Exhaustiveness is what ties the two enums together: if a new
/// `YieldReason` variant is ever added, this match stops compiling and
/// forces the trace contract to be updated alongside it.
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

/// Read-only view of committed guest memory plus the caller's
/// current guest tick, handed to `Lv2Host::dispatch`. Constructed
/// fresh for each dispatch call so the tick snapshot is tied to
/// the syscall that triggered the dispatch.
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
