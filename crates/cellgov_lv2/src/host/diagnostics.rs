//! Invariant-break observability for [`Lv2Host`].
//!
//! Submodules call `record_invariant_break` (debug-panic + log-once)
//! or `log_invariant_break` (log-once only, for guest-reachable
//! degraded paths). `resolve_wake_thread` is the recurring caller
//! pattern: dequeue-from-primitive-waiter-list followed by table
//! lookup, with the missing-table-entry case routed through
//! `record_invariant_break` and the wake skipped.
//!
//! The `invariant_break_count` field lives on [`Lv2Host`] because
//! submodules fold it via their dispatch paths and it must stay
//! co-located with the rest of the host's state. Only the methods
//! move; the field stays.

use cellgov_event::UnitId;

use crate::ppu_thread::PpuThreadId;

use super::Lv2Host;

impl Lv2Host {
    /// Running count of host-invariant breaks observed during dispatch.
    #[inline]
    pub fn invariant_break_count(&self) -> usize {
        self.invariant_break_count
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
    pub(super) fn log_invariant_break(
        &mut self,
        site: &'static str,
        details: std::fmt::Arguments<'_>,
    ) {
        if self.invariant_break_count == 0 {
            #[allow(
                clippy::print_stderr,
                reason = "one-shot diagnostic for guest-reachable invariant breaks; gated on the first occurrence so a hostile guest cannot spam stderr"
            )]
            {
                eprintln!("lv2 host invariant break at {site}: {details}");
            }
        }
        self.invariant_break_count = self.invariant_break_count.saturating_add(1);
    }

    /// `None` means the thread table and the primitive diverged;
    /// the divergence is logged as an invariant break so the caller
    /// can skip the wake and leave surviving waiters intact.
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
