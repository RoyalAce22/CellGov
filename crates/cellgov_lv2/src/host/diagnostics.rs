//! Invariant-break observability for [`Lv2Host`].
//!
//! # Cross-module contract
//!
//! Both `record_invariant_break` and `log_invariant_break` push onto
//! `pending_invariant_breaks` and bump `invariant_break_count`. The
//! runtime drains the buffer and emits one
//! `TraceRecord::HostInvariantBreak` per reason via the bridge in
//! `cellgov_core::runtime::trace_bridge`; the lv2 crate does not
//! depend on `cellgov_trace`.

use cellgov_event::UnitId;

use crate::ppu_thread::PpuThreadId;

use super::Lv2Host;

/// Category of a host-side invariant break.
///
/// Variant order must match `TracedInvariantBreakReason` in
/// `cellgov_trace` (bridged via `cellgov_core::runtime::trace_bridge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvariantBreakReason {
    /// Catch-all placeholder; every push currently uses this variant.
    Unspecified,
}

impl Lv2Host {
    /// Running count of host-invariant breaks observed during dispatch.
    #[inline]
    pub fn invariant_break_count(&self) -> usize {
        self.invariant_break_count
    }

    /// Drain pending [`InvariantBreakReason`] events.
    pub fn drain_pending_invariant_breaks(&mut self) -> std::vec::Drain<'_, InvariantBreakReason> {
        self.pending_invariant_breaks.drain(..)
    }

    /// Drain pending shared-memory region-install requests emitted by
    /// `sys_mmapper_map_shared_memory` (334).
    ///
    /// # Cross-module contract
    ///
    /// Each `(addr, size)` must be applied via
    /// `GuestMemory::install_region` before the dispatch's effects
    /// commit; otherwise subsequent guest writes through `addr` trip
    /// `CommitError::OutOfRange`.
    pub fn drain_pending_region_installs(&mut self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.pending_region_installs
            .drain(..)
            .map(|p| (p.addr, p.size))
    }

    /// Debug-panic + log-once for a host-invariant break.
    pub(super) fn record_invariant_break(
        &mut self,
        site: &'static str,
        details: std::fmt::Arguments<'_>,
    ) {
        debug_assert!(false, "lv2 host invariant break at {site}: {details}");
        self.log_invariant_break(site, details);
    }

    /// Log-once without `debug_assert!`, for paths reachable by
    /// guest input during normal operation (e.g. `Unsupported`
    /// syscalls during real boots).
    pub fn log_invariant_break(&mut self, site: &'static str, details: std::fmt::Arguments<'_>) {
        if self.invariant_break_count == 0 {
            #[allow(
                clippy::print_stderr,
                reason = "one-shot diagnostic for guest-reachable invariant breaks; gated on the first occurrence so a hostile guest cannot spam stderr"
            )]
            {
                eprintln!("lv2 host invariant break at {site}: {details}");
            }
        }
        self.pending_invariant_breaks
            .push(InvariantBreakReason::Unspecified);
        self.invariant_break_count = self.invariant_break_count.saturating_add(1);
    }

    /// `None` means the thread table and the primitive diverged; the
    /// divergence is logged as an invariant break and the caller must
    /// skip the wake to leave surviving waiters intact.
    pub(super) fn resolve_wake_thread(
        &mut self,
        thread: PpuThreadId,
        site: &'static str,
    ) -> Option<UnitId> {
        match self.ppu_threads.get(thread) {
            Some(t) => Some(t.unit_id),
            None => {
                self.record_invariant_break(
                    site,
                    format_args!(
                        "PpuThreadId {thread:?} dequeued from a primitive waiter list but \
                         not in PpuThreadTable; wake skipped"
                    ),
                );
                None
            }
        }
    }
}
