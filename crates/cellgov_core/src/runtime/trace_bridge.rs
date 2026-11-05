//! Bridges runtime effect/yield enums onto their `cellgov_trace` twins
//! and exposes a read-only `MemoryView` to `Lv2Host::dispatch`.

use std::borrow::Cow;

use cellgov_exec::{BarrierKind, YieldReason};
use cellgov_lv2::{InvariantBreakReason, Lv2Runtime};
use cellgov_mem::GuestMemory;
use cellgov_time::GuestTicks;
use cellgov_trace::{
    TracedBarrierKind, TracedEffectKind, TracedInvariantBreakReason, TracedYieldReason,
};

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
        Effect::SharedReadIntent { .. } => TracedEffectKind::SharedReadIntent,
        Effect::ClockRead { .. } => TracedEffectKind::ClockRead,
        Effect::MailboxPop { .. } => TracedEffectKind::MailboxPop,
        Effect::MfcInvalidCommand { .. } => TracedEffectKind::MfcInvalidCommand,
        Effect::SpuSignalWrite { .. } => TracedEffectKind::SpuSignalWrite,
        Effect::SpuLocalStoreWrite { .. } => TracedEffectKind::SpuLocalStoreWrite,
        Effect::SpuLocalStoreRead { .. } => TracedEffectKind::SpuLocalStoreRead,
    }
}

pub(super) fn traced_barrier_kind(kind: BarrierKind) -> TracedBarrierKind {
    match kind {
        BarrierKind::Sync => TracedBarrierKind::Sync,
        BarrierKind::Lwsync => TracedBarrierKind::Lwsync,
        BarrierKind::Ptesync => TracedBarrierKind::Ptesync,
        BarrierKind::SyncL3 => TracedBarrierKind::SyncL3,
        BarrierKind::Eieio => TracedBarrierKind::Eieio,
        BarrierKind::Isync => TracedBarrierKind::Isync,
        BarrierKind::SpuSync => TracedBarrierKind::SpuSync,
        BarrierKind::SpuSyncC => TracedBarrierKind::SpuSyncC,
        BarrierKind::SpuDsync => TracedBarrierKind::SpuDsync,
        BarrierKind::MfcFence => TracedBarrierKind::MfcFence,
        BarrierKind::MfcTagBarrier => TracedBarrierKind::MfcTagBarrier,
        BarrierKind::MfcSync => TracedBarrierKind::MfcSync,
        BarrierKind::MfcEieio => TracedBarrierKind::MfcEieio,
        BarrierKind::MfcBarrier => TracedBarrierKind::MfcBarrier,
    }
}

/// Cross-crate mirror for the lv2-owned [`InvariantBreakReason`]
/// onto its `cellgov_trace` discriminant.
pub(super) fn traced_invariant_break_reason(
    reason: InvariantBreakReason,
) -> TracedInvariantBreakReason {
    match reason {
        InvariantBreakReason::Unspecified => TracedInvariantBreakReason::Unspecified,
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
        YieldReason::ChannelStall => TracedYieldReason::ChannelStall,
    }
}

/// Read-only view of committed memory, the units' local stores and the
/// tick, constructed fresh per `Lv2Host::dispatch` call.
pub(super) struct MemoryView<'a> {
    pub(super) memory: &'a GuestMemory,
    pub(super) registry: &'a crate::registry::UnitRegistry,
    pub(super) current_tick: GuestTicks,
}

impl Lv2Runtime for MemoryView<'_> {
    fn read_committed(&self, addr: u64, len: usize) -> Option<Cow<'_, [u8]>> {
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

    fn read_local_store(&self, unit: cellgov_event::UnitId, lsa: u32, len: u32) -> Option<Vec<u8>> {
        self.registry.get(unit)?.read_local_store(lsa, len).ok()
    }

    fn read_committed_until(
        &self,
        addr: u64,
        max_len: usize,
        terminator: u8,
    ) -> Option<Cow<'_, [u8]>> {
        // `GuestMemory::read` returns None when a range runs past the
        // last mapped byte, so bisect to find the largest readable
        // window <= max_len before scanning for the terminator.
        //
        // The probes and the scan run unlogged: a window can run
        // into a `ReservedZeroReadable` neighbour past the terminator,
        // and those bytes are no read of the guest's. The one logged
        // read below covers the string and its terminator, or the
        // whole window when no terminator is found.
        use cellgov_mem::ByteRange;
        let (window_len, nul_pos) = self.memory.with_reads_unlogged(|mem| {
            let probe = ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)?;
            mem.read(probe)?;
            let mut hi = max_len;
            let mut lo = 1;
            while hi > lo {
                let mid = (lo + hi).div_ceil(2);
                let r = ByteRange::new(cellgov_mem::GuestAddr::new(addr), mid as u64);
                if r.and_then(|r| mem.read(r)).is_some() {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            let window = mem.read(ByteRange::new(
                cellgov_mem::GuestAddr::new(addr),
                lo as u64,
            )?)?;
            Some((lo, window.iter().position(|&b| b == terminator)))
        })?;
        let read_len = nul_pos.map_or(window_len, |p| p + 1);
        let window = self.memory.read(ByteRange::new(
            cellgov_mem::GuestAddr::new(addr),
            read_len as u64,
        )?)?;
        let nul_pos = nul_pos?;
        Some(match window {
            Cow::Borrowed(bytes) => Cow::Borrowed(&bytes[..nul_pos]),
            Cow::Owned(mut bytes) => {
                bytes.truncate(nul_pos);
                Cow::Owned(bytes)
            }
        })
    }

    fn committed_overlap_end(&self, addr: u64, size: u64) -> Option<u64> {
        if size == 0 {
            return None;
        }
        // A window running past the address space cannot be claimed;
        // report the overflow as an overlap ending at the space's end.
        let Some(end) = addr.checked_add(size) else {
            return Some(u64::MAX);
        };
        self.memory
            .regions()
            .filter(|r| {
                let r_end = r.base() + r.size();
                r.base() < end && addr < r_end
            })
            .map(|r| r.base() + r.size())
            .max()
    }

    fn writable(&self, addr: u64, len: usize) -> bool {
        // The commit accepts the same writes, across regions that sit
        // end to end.
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), len as u64)
            .is_some_and(|range| self.memory.validate_write(range, len).is_ok())
    }
}
