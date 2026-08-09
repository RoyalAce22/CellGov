//! Production counters for the firmware system-IPC namespace
//! (`0x8006_0100_0000_xxxx`).
//!
//! Two independent channels share the namespace: the module_start ring
//! (a keyed shm plus its per-facility conds) and the event queue at
//! `...0100`. The counters here answer "did anything produce on this
//! namespace during the run", which no other witness can -- the
//! existing cellSysutil counters are keyed to specific slot conds and
//! go silent for any key outside that pattern.
//!
//! A zero reading is a measurement, not a verdict: the namespace can be
//! dormant in a boot that never reaches its producer.

use std::collections::BTreeMap;

use cellgov_ps3_abi::system_ipc::{SYSTEM_IPC_KEY_NAMESPACE, SYSTEM_IPC_KEY_NAMESPACE_MASK};

/// `true` when `ipc_key` names an object in the firmware system-IPC
/// namespace.
#[inline]
pub fn is_system_ipc_key(ipc_key: u64) -> bool {
    ipc_key & SYSTEM_IPC_KEY_NAMESPACE_MASK == SYSTEM_IPC_KEY_NAMESPACE
}

/// One mapped range of a namespace-keyed shm, recorded at 334 / 337.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SystemIpcMapping {
    pub(super) ipc_key: u64,
    pub(super) base: u32,
    pub(super) size: u32,
}

/// Per-event counters for the system-IPC namespace, split by channel.
///
/// Instrument-only: no field is folded into `Lv2Host::state_hash` and
/// no dispatch arm branches on one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SystemIpcWitness {
    /// Keyed `sys_mmapper_allocate_shared_memory` (332) calls that
    /// minted a fresh `mem_id` for a namespace key.
    pub shm_creates: u64,
    /// Keyed 332 calls that resolved a namespace key to an already
    /// registered `mem_id` -- a second referent of the same shm.
    pub shm_attaches: u64,
    /// Maps (334 / 337) of a namespace-keyed shm into guest memory.
    pub shm_maps: u64,
    /// Committed `SharedWriteIntent` ranges intersecting a recorded
    /// namespace shm mapping.
    ///
    /// Three write paths reach the same bytes without incrementing
    /// this: an alias mapping established outside 334 / 337, an
    /// `Effect::ConditionalStore` (the `stwcx.` / `stdcx.` pair), and
    /// an SPU DMA, which lands through the DMA queue rather than the
    /// effect list.
    pub shm_writes: u64,
    /// `sys_cond_create` (105) calls carrying a namespace ipc_key.
    pub cond_creates: u64,
    /// `sys_cond_wait` (107) calls on a namespace-keyed cond, counting
    /// both parks and the waits a kernel-side arm satisfied
    /// immediately.
    pub cond_waits: u64,
    /// Signals (108 / 109 / 110) aimed at a namespace-keyed cond,
    /// counted at dispatch whether or not a waiter was present.
    pub cond_signals: u64,
    /// `sys_event_queue_create` (128) calls carrying a namespace
    /// ipc_key not yet registered.
    pub event_queue_creates: u64,
    /// Keyed 128 calls naming an already registered namespace key.
    ///
    /// Non-zero marks a fidelity gap, not just traffic: CellGov mints a
    /// fresh queue for the second caller instead of returning the
    /// registered one, so the two callers do not share a queue.
    pub event_queue_references: u64,
    /// `sys_event_port_send` (138) deliveries to a namespace-keyed
    /// queue.
    pub event_queue_enqueues: u64,
    /// Per-key event totals over every counter above that attributes to
    /// a single key. The key set is the namespace inventory a frontier
    /// run reports.
    pub keys_touched: BTreeMap<u64, u64>,
}

impl SystemIpcWitness {
    /// `true` when no namespace event of any kind was observed.
    pub fn is_silent(&self) -> bool {
        *self == Self::default()
    }

    pub(super) fn note_key(&mut self, ipc_key: u64) {
        *self.keys_touched.entry(ipc_key).or_insert(0) += 1;
    }
}

#[cfg(test)]
#[path = "tests/system_ipc_witness_tests.rs"]
mod tests;
