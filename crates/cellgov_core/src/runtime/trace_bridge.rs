//! Bridges runtime effect/yield enums onto their `cellgov_trace` twins
//! and exposes a read-only `MemoryView` to `Lv2Host::dispatch`.

use cellgov_exec::YieldReason;
use cellgov_lv2::Lv2Runtime;
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;
use cellgov_trace::{TracedEffectKind, TracedYieldReason};

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

/// `syscall_lev` is the LEV field of the `sc` instruction; non-zero
/// LEV distinguishes a hypercall from a normal LV2 syscall so the
/// trace stream cannot byte-collide the two.
pub(super) fn traced_yield_reason(y: YieldReason, syscall_lev: Option<u8>) -> TracedYieldReason {
    match y {
        YieldReason::BudgetExhausted => TracedYieldReason::BudgetExhausted,
        YieldReason::MailboxAccess => TracedYieldReason::MailboxAccess,
        YieldReason::DmaSubmitted => TracedYieldReason::DmaSubmitted,
        YieldReason::DmaWait => TracedYieldReason::DmaWait,
        YieldReason::WaitingSync => TracedYieldReason::WaitingSync,
        YieldReason::Syscall => match syscall_lev {
            Some(lev) if lev != 0 => TracedYieldReason::Hypercall,
            _ => TracedYieldReason::Syscall,
        },
        YieldReason::InterruptBoundary => TracedYieldReason::InterruptBoundary,
        YieldReason::Fault => TracedYieldReason::Fault,
        YieldReason::Finished => TracedYieldReason::Finished,
    }
}

/// Read-only view of committed memory plus tick snapshot, constructed
/// fresh per `Lv2Host::dispatch` call.
pub(super) struct MemoryView<'a> {
    pub(super) memory: &'a GuestMemory,
    pub(super) current_tick: GuestTicks,
}

impl Lv2Runtime for MemoryView<'_> {
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]> {
        // Linear `.as_bytes()` only covers the main region; route
        // through `GuestMemory::read` to reach stacks, RSX, and
        // SPU-reserved regions.
        use cellgov_mem::ByteRange;
        let range = ByteRange::new(cellgov_mem::GuestAddr::new(addr), len as u64)?;
        self.memory.read(range)
    }

    fn current_tick(&self) -> GuestTicks {
        self.current_tick
    }

    fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]> {
        // `GuestMemory::read` returns None when a range straddles a
        // region boundary, so bisect to find the largest readable
        // window <= max_len before scanning for the terminator.
        use cellgov_mem::ByteRange;
        let region_remaining = {
            let probe = ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)?;
            self.memory.read(probe)?;
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
