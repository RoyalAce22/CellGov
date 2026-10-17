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
    /// Drain pending [`InvariantBreakReason`] events.
    pub fn drain_pending_invariant_breaks(&mut self) -> std::vec::Drain<'_, InvariantBreakReason> {
        self.obs.pending_invariant_breaks.drain(..)
    }

    /// Drain pending shared-memory region-install requests emitted by
    /// `sys_mmapper_map_shared_memory` (334) / `search_and_map` (337).
    ///
    /// # Cross-module contract
    ///
    /// Each `(addr, size, ipc_key, mem_id)` must be applied via
    /// `GuestMemory::install_region` before the dispatch's effects
    /// commit; otherwise subsequent guest writes through `addr` trip
    /// `CommitError::OutOfRange`. The window then takes `mem_id`'s
    /// contents from [`Self::take_segment_contents`]. A `Some(ipc_key)`
    /// names the keyed segment this window views; the runtime keeps
    /// keyed views coherent across address spaces.
    pub fn drain_pending_region_installs(
        &mut self,
    ) -> impl Iterator<Item = (u64, usize, Option<u64>, u32)> + '_ {
        self.derived
            .pending_region_installs
            .drain(..)
            .map(|p| (p.addr, p.size, p.ipc_key, p.mem_id))
    }

    /// Drain the 1 MiB-page areas `sys_memory_allocate` took, as
    /// `(base, size)`.
    ///
    /// # Cross-module contract
    ///
    /// The runtime installs each area in the caller's space via
    /// `GuestMemory::install_region` before the dispatch's effects
    /// commit, so the block the same dispatch hands out is backed.
    pub fn drain_pending_area_installs(&mut self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.derived.pending_area_installs.drain(..)
    }

    /// Take the base of the RSX context window
    /// `sys_rsx_context_allocate` opened, if one opened since the last
    /// take.
    ///
    /// # Cross-module contract
    ///
    /// The runtime maps the window in the caller's space via
    /// `GuestMemory::install_region_with_access` before the dispatch's
    /// effects commit: the control block (`region::CONTROL_SIZE` bytes
    /// at the base) and the driver-info and reports blocks above it.
    pub fn take_pending_window_install(&mut self) -> Option<u32> {
        self.derived.pending_window_install.take()
    }

    /// Drain pending shared-memory region-removal requests emitted by
    /// `sys_mmapper_unmap_shared_memory` (335).
    ///
    /// # Cross-module contract
    ///
    /// Each `(addr, size, ipc_key, mem_id)` names a window an earlier
    /// [`Self::drain_pending_region_installs`] item installed. The
    /// runtime gives its bytes to [`Self::park_segment_contents`] under
    /// a `Some(mem_id)` and removes the window from the caller's space via
    /// `GuestMemory::remove_region`. A `Some(ipc_key)` names the keyed segment whose
    /// view the window was; the runtime drops that view.
    pub fn drain_pending_region_removals(
        &mut self,
    ) -> impl Iterator<Item = (u64, usize, Option<u64>, Option<u32>)> + '_ {
        self.derived
            .pending_region_removals
            .drain(..)
            .map(|p| (p.addr, p.size, p.ipc_key, Some(p.mem_id)))
    }

    /// Drain the windows of child processes that exited, in the form of
    /// [`Self::drain_pending_region_removals`].
    ///
    /// # Cross-module contract
    ///
    /// Each window lies in the exited child's space, not the space of
    /// the unit whose dispatch the runtime is draining, so the runtime
    /// drains this from its child-exit path, right after
    /// [`Self::mark_process_exited`]. The mem id is `None` for a
    /// segment the exit freed with the child's ids: no later map can
    /// name it, so its bytes are not kept.
    pub fn drain_released_process_windows(
        &mut self,
    ) -> impl Iterator<Item = (u64, usize, Option<u64>, Option<u32>)> + '_ {
        let handles = &self.state.mmapper_handles;
        self.derived.released_process_windows.drain(..).map(|p| {
            let kept = handles.get(p.mem_id).is_some().then_some(p.mem_id);
            (p.addr, p.size, p.ipc_key, kept)
        })
    }

    /// Keep the bytes an unmapped window of `mem_id` held, for the
    /// segment's next map. A `mem_id` no handle names is an invariant
    /// break: 329 refuses a free while a window maps the segment.
    pub fn park_segment_contents(&mut self, mem_id: u32, bytes: Vec<u8>) {
        if !self.state.mmapper_handles.park(mem_id, bytes) {
            self.log_invariant_break(
                "dispatch.mmapper_park_unknown_mem_id",
                format_args!("unmapped window of mem_id {mem_id:#x}, which no handle names"),
            );
        }
    }

    /// The contents an unmap parked for `mem_id`, taken back for a new
    /// window; `None` for a segment never unmapped.
    pub fn take_segment_contents(&mut self, mem_id: u32) -> Option<Vec<u8>> {
        self.state.mmapper_handles.take_parked(mem_id)
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
    ///
    /// This method formats `details` for the first break only, into
    /// [`super::Lv2Observability::first_invariant_break`]. Each later
    /// break updates the counts and formats nothing.
    pub fn log_invariant_break(&mut self, site: &'static str, details: std::fmt::Arguments<'_>) {
        if self.obs.first_invariant_break.is_none() {
            self.obs.first_invariant_break = Some(format!("{site}: {details}"));
        }
        self.obs
            .pending_invariant_breaks
            .push(InvariantBreakReason::Unspecified);
        self.obs.invariant_break_count = self.obs.invariant_break_count.saturating_add(1);
        let slot = self.obs.invariant_break_sites.entry(site).or_insert(0);
        *slot = slot.saturating_add(1);
    }

    /// Break count for one `log_invariant_break` site string.
    pub fn invariant_break_site_count(&self, site: &str) -> u64 {
        self.obs
            .invariant_break_sites
            .get(site)
            .copied()
            .unwrap_or(0)
    }

    /// `None` means the thread table and the primitive diverged; the
    /// divergence is logged as an invariant break and the caller must
    /// skip the wake to leave surviving waiters intact.
    pub(super) fn resolve_wake_thread(
        &mut self,
        thread: PpuThreadId,
        site: &'static str,
    ) -> Option<UnitId> {
        match self.state.ppu_threads.get(thread) {
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

#[cfg(test)]
#[path = "tests/first_invariant_break_tests.rs"]
mod tests;
