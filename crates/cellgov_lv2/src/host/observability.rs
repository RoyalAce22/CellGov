//! [`Lv2Observability`]: the inert partition of [`super::Lv2Host`].
//!
//! Instruments and diagnostic aids only: removing a field here
//! changes no guest-visible byte. A field that steers a syscall
//! return value or an effect belongs in [`super::state::Lv2State`]
//! or [`super::derived::Lv2Derived`]. The one carve-out is
//! `pending_invariant_breaks`, whose carry-over across
//! [`super::Lv2Host::clear_observability`] is documented on the
//! field.

use std::collections::{BTreeMap, BTreeSet};

use super::system_ipc_witness::{SystemIpcMapping, SystemIpcWitness};

/// Witness counters, diagnostic logs, and naming aids; inert with
/// respect to guest-visible execution.
#[derive(Debug, Clone, Default)]
pub struct Lv2Observability {
    /// Witness: times the cond\[1\] ring-check arm satisfied a wait
    /// immediately. Expected 0 under the V256 seed; a non-zero
    /// value means a refill wait observed a non-depleted ring.
    pub cond_ring_wakes: u64,
    /// Witness: parks on a cellSysutil cond\[0\] -- the producer-fed
    /// record-finish waits CellGov has no producer for -- keyed by
    /// slot index.
    pub cond0_producer_waits_by_slot: BTreeMap<u64, u64>,
    /// Witness: `sys_cond_signal` dispatch count (drain witness for
    /// the seeded-ring consumer).
    pub cond_signal_dispatches: u64,
    /// Witness: `_sys_prx_unload_module` calls refused because the
    /// target is a resident firmware module.
    pub prx_unload_rejections: u64,
    /// Witness: `sys_cond_signal` dispatches keyed by the target
    /// cond's create-time ipc_key (keyed conds only). Per-slot /
    /// per-facility drain attribution for the seeded-ring consumer.
    pub cond_keyed_signal_counts: BTreeMap<u64, u64>,
    /// Witness: `(attempts, bound)` for `sys_event_port_connect_ipc`
    /// across every namespace. A gap between the two is the count of
    /// connects that named a key no queue is registered under.
    pub event_port_ipc_connects: (u64, u64),
    /// Witness: guest paths sc 480 / 497 answered `CELL_ENOENT`, with
    /// hit counts. The key set names which modules a title asks for
    /// that the corpus cannot serve.
    pub prx_load_misses: BTreeMap<String, u64>,
    /// Witness: null-backend hits keyed by syscall number. The key set
    /// is the boot's unimplemented-syscall inventory; the counts
    /// separate a one-shot probe from a retry loop.
    pub unsupported_syscalls: BTreeMap<u64, u64>,
    /// Witness: system-IPC namespace production counters.
    pub system_ipc_witness: SystemIpcWitness,
    /// Guest ranges where a namespace-keyed shm is mapped, recorded at
    /// 334 / 337 and tested against every committed write to feed the
    /// shm-write witness. Keyed by mapped base so an overlapping
    /// remap replaces its predecessor. Witness input only; no
    /// dispatch decision reads it.
    pub(in crate::host) system_ipc_mappings: BTreeMap<u32, SystemIpcMapping>,
    /// Running count of host-invariant breaks.
    pub invariant_break_count: usize,
    /// Per-site break counts keyed by the static site string passed
    /// to `log_invariant_break`.
    pub invariant_break_sites: BTreeMap<&'static str, u64>,
    /// Drained after each `Lv2Host::dispatch` by the runtime, which
    /// emits one `HostInvariantBreak` trace record per entry.
    /// Commit-time and wake-time paths in `cellgov_core` push after
    /// that drain, so entries can sit here across a step boundary;
    /// `Lv2Host::clear_observability` carries them over for that
    /// reason.
    pub(in crate::host) pending_invariant_breaks: Vec<super::diagnostics::InvariantBreakReason>,
    /// Captured `sys_tty_write` byte stream.
    pub tty_log: Vec<u8>,
    /// Witness: Count of `dispatch_thread_initialize`
    /// invocations. That dispatcher's catch-all `debug_assert!`
    /// guards against being called with the wrong request
    /// variant; silence is non-vacuous only when the dispatch
    /// actually ran.
    pub spu_thread_initialize_dispatches: u64,
    /// Witness: Count of `cond_reacquire_wake` calls.
    /// That function's `debug_assert!(!use_lwmutex, ...)` guards
    /// against an unimplemented lwmutex-cond re-acquire path;
    /// silence is non-vacuous only when the function ran.
    pub cond_reacquire_wake_calls: u64,
    /// Unresolved-import NID -> the libraries the guest asked for it
    /// from, supplied at boot by the GOT patcher. Diagnostic naming
    /// aid: lets `dispatch_unresolved_import` name the library the
    /// guest named instead of only the NID.
    pub unresolved_import_requesters: BTreeMap<u32, BTreeSet<String>>,
    /// Witness: sc 484 calls seen, with a valid option struct.
    pub prx_register_module_count: u64,
    /// Witness: of those, ones that took the CoreOS manual-link branch.
    pub prx_register_module_manual_count: u64,
    /// Witness: GOT slots bound by the manual-link branch.
    pub prx_register_module_linked: u64,
    /// Witness: import NIDs the manual-link branch could not resolve;
    /// their slots keep the guest's own stub address.
    pub prx_register_module_unresolved: u64,
    /// Witness: total `sys_lwmutex_lock` calls across the boot that
    /// failed because the id was not in the table (CELL_ESRCH). A
    /// wrong program-authority-id skips libsysmodule's lwmutex
    /// creation, so every later `cellSysmoduleLoadModule` locks id 0
    /// and bumps this; a non-zero count is that signature. Counts
    /// every occurrence boot-wide, not one window.
    pub lwmutex_unknown_lock_count: u64,
    /// Witness: every non-zero `Lv2Dispatch::Immediate` code any arm
    /// returned, keyed by code. Includes successful returns that carry
    /// a value (kernel ids, pids), not just errors -- the point is
    /// completeness: a code absent here was never returned by LV2, so a
    /// guest reporting it built it itself.
    pub dispatch_nonzero_returns: BTreeMap<u64, u64>,
    /// Witness: `sys_mutex_unlock` calls refused with `CELL_EPERM`
    /// because the caller does not own the mutex.
    pub mutex_unlock_not_owner_count: u64,
    /// Witness: sc 480/497 registry misses on a firmware path,
    /// resolved by registering a stub entry under a real kernel id.
    pub prx_load_hle_stub_count: u64,
    /// Witness: sc 480/497 loads reported `CELL_ENOENT` (non-firmware
    /// path or unusable path bytes).
    pub prx_load_not_found_count: u64,
}

impl Lv2Observability {
    /// Witness: cond\[0\] producer-wait parks, summed over slots.
    pub fn cond0_producer_waits(&self) -> u64 {
        self.cond0_producer_waits_by_slot.values().sum()
    }

    /// Witness tuple for the frontier run: (sc 484 calls, manual-link
    /// calls, GOT slots bound, import NIDs left unresolved).
    pub fn prx_register_module_witness(&self) -> (u64, u64, u64, u64) {
        (
            self.prx_register_module_count,
            self.prx_register_module_manual_count,
            self.prx_register_module_linked,
            self.prx_register_module_unresolved,
        )
    }
}

#[cfg(test)]
#[path = "tests/observability_tests.rs"]
mod tests;
