//! [`UnitRegistry`] state struct and `Clone` impl. Per-method
//! accessors live in sibling submodules.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use std::collections::BTreeMap;

use crate::registry::RegisteredUnit;

/// The runtime's unit registry.
///
/// `UnitId`s come from a monotonic counter; stable across runs when
/// registration order is deterministic. `BTreeMap` keying guarantees
/// id-ordered iteration independent of insertion order.
// Fields `pub(super)` so sibling files implement methods against them;
// do not widen.
#[derive(Default)]
pub struct UnitRegistry {
    pub(super) next_id: u64,
    pub(super) units: BTreeMap<UnitId, Box<dyn RegisteredUnit>>,
    /// Runtime-side status overrides. Written by the commit pipeline,
    /// cleared when the unit next runs. Takes precedence over the
    /// unit's self-reported `status()` for scheduling and hashing.
    pub(super) status_overrides: BTreeMap<UnitId, UnitStatus>,
    /// Per-unit pending `MailboxReceiveAttempt` pops, drained into
    /// `ExecutionContext::received_messages` at next step.
    pub(super) pending_receives: BTreeMap<UnitId, Vec<u32>>,
    /// Per-unit pending syscall return code, drained into
    /// `ExecutionContext::syscall_return` at next step.
    pub(super) pending_syscall_returns: BTreeMap<UnitId, u64>,
    /// Per-unit register writes injected by HLE dispatch; drained
    /// alongside syscall returns.
    pub(super) pending_register_writes: BTreeMap<UnitId, Vec<(u8, u64)>>,
}

impl Clone for UnitRegistry {
    fn clone(&self) -> Self {
        let units = self
            .units
            .iter()
            .map(|(id, unit)| (*id, unit.clone_box()))
            .collect();
        Self {
            next_id: self.next_id,
            units,
            status_overrides: self.status_overrides.clone(),
            pending_receives: self.pending_receives.clone(),
            pending_syscall_returns: self.pending_syscall_returns.clone(),
            pending_register_writes: self.pending_register_writes.clone(),
        }
    }
}
