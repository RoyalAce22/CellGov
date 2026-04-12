//! Unit registry seam.
//!
//! Units are not free-floating: they are created via a registry call
//! that assigns a stable `UnitId`, records the constructor parameters
//! in the trace, and makes the unit known to the scheduler. Dynamic
//! spawning (for example, SPU thread group creation) goes through the
//! same seam, even though the dynamic case is currently stubbed to a
//! fixed initial set.
//!
//! This module owns that seam. It assigns `UnitId`s in a single
//! sequential allocator, stores units behind a small object-safe trait,
//! and exposes a deterministic iteration order keyed by id.
//!
//! ## Why an object-safe wrapper trait
//!
//! [`cellgov_exec::ExecutionUnit`] has an associated `Snapshot` type,
//! which makes `dyn ExecutionUnit` not object-safe. The registry needs
//! to hold a heterogeneous collection of unit types, so we define
//! [`RegisteredUnit`] -- an object-safe trait that mirrors the parts of
//! `ExecutionUnit` the scheduler actually needs at runtime
//! (`unit_id`, `status`, `run_until_yield`) and is blanket-implemented
//! for every `U: ExecutionUnit + 'static`.
//!
//! Snapshots are deliberately **not** part of `RegisteredUnit`. They are
//! a separate concern that the runtime trace layer will pick up later
//! via a different seam, after the binary trace format pins how it wants
//! to serialize them. Including them now would force a premature
//! serialization choice.

use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, UnitStatus};
use cellgov_time::Budget;
use std::collections::BTreeMap;

/// Object-safe view of an execution unit, used by the registry and the
/// scheduler.
///
/// Mirrors the runtime-facing methods of [`ExecutionUnit`] minus the
/// associated `Snapshot` type, so the registry can hold
/// `Box<dyn RegisteredUnit>` heterogeneously. There is a blanket impl
/// for every `U: ExecutionUnit + 'static`, so any concrete unit type a
/// crate defines is automatically a `RegisteredUnit`.
///
/// `'static` is required because the registry owns the unit; a
/// borrowed-from-elsewhere unit would not satisfy ownership semantics
/// the runtime needs.
pub trait RegisteredUnit: 'static {
    /// The unit's stable id, assigned at registration time. Must equal
    /// the id the registry handed to the unit's factory closure.
    fn unit_id(&self) -> UnitId;

    /// Coarse runnability state queried by the scheduler.
    fn status(&self) -> UnitStatus;

    /// Run the unit until it yields. Same contract as
    /// [`ExecutionUnit::run_until_yield`].
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
    ) -> ExecutionStepResult;
}

impl<U: ExecutionUnit + 'static> RegisteredUnit for U {
    #[inline]
    fn unit_id(&self) -> UnitId {
        ExecutionUnit::unit_id(self)
    }

    #[inline]
    fn status(&self) -> UnitStatus {
        ExecutionUnit::status(self)
    }

    #[inline]
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
    ) -> ExecutionStepResult {
        ExecutionUnit::run_until_yield(self, budget, ctx)
    }
}

/// The runtime's unit registry.
///
/// Owns every execution unit known to the runtime. Allocates `UnitId`s
/// from a sequential counter (so ids are stable across runs of the same
/// scenario as long as the registration order is deterministic). Stores
/// units in a [`BTreeMap`] keyed by `UnitId` so that iteration is in id
/// order, never in `HashMap` insertion order. No host-time inputs, no
/// hash iteration order.
#[derive(Default)]
pub struct UnitRegistry {
    next_id: u64,
    units: BTreeMap<UnitId, Box<dyn RegisteredUnit>>,
    /// Runtime-side status overrides. When the commit pipeline blocks
    /// a unit (e.g. empty mailbox receive) or wakes one (e.g.
    /// `Effect::WakeUnit`), it sets an override here. The override
    /// takes precedence over the unit's self-reported `status()` for
    /// scheduling and hashing purposes. Cleared when the unit is next
    /// run, so the unit's own status logic resumes control.
    status_overrides: BTreeMap<UnitId, UnitStatus>,
    /// Per-unit inbox for messages delivered by the commit pipeline
    /// (e.g. `MailboxReceiveAttempt` popping from a non-empty
    /// mailbox). Drained by the runtime at the start of the next
    /// `run_until_yield` and passed to the unit via
    /// `ExecutionContext::received_messages`.
    pending_receives: BTreeMap<UnitId, Vec<u32>>,
}

impl UnitRegistry {
    /// Construct an empty registry.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of registered units.
    #[inline]
    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether the registry holds any units.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// Register a unit produced by `factory`, which receives the
    /// freshly-allocated `UnitId` and must construct a unit that
    /// reports the same id from [`ExecutionUnit::unit_id`].
    ///
    /// Returns the assigned id. Panics if the constructed unit reports
    /// a different id than the one it was given -- that is a unit
    /// implementation bug, not a recoverable condition, and the runtime
    /// must surface it loudly.
    pub fn register_with<U, F>(&mut self, factory: F) -> UnitId
    where
        U: ExecutionUnit + 'static,
        F: FnOnce(UnitId) -> U,
    {
        let id = UnitId::new(self.next_id);
        self.next_id += 1;
        let unit = factory(id);
        assert_eq!(
            ExecutionUnit::unit_id(&unit),
            id,
            "registered unit reported {} but registry assigned {}",
            ExecutionUnit::unit_id(&unit).raw(),
            id.raw(),
        );
        self.units.insert(id, Box::new(unit));
        id
    }

    /// Borrow a unit by id, if present.
    #[inline]
    pub fn get(&self, id: UnitId) -> Option<&dyn RegisteredUnit> {
        self.units.get(&id).map(|u| u.as_ref())
    }

    /// Mutably borrow a unit by id, if present. The scheduler uses
    /// this to drive `run_until_yield`.
    #[inline]
    pub fn get_mut(&mut self, id: UnitId) -> Option<&mut dyn RegisteredUnit> {
        self.units.get_mut(&id).map(|u| u.as_mut())
    }

    /// Iterate registered units in id order.
    pub fn iter(&self) -> impl Iterator<Item = (UnitId, &dyn RegisteredUnit)> + '_ {
        self.units.iter().map(|(id, u)| (*id, u.as_ref()))
    }

    /// Iterate registered units mutably in id order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (UnitId, &mut dyn RegisteredUnit)> + '_ {
        self.units.iter_mut().map(|(id, u)| (*id, u.as_mut()))
    }

    /// Iterate registered ids in id order.
    pub fn ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.units.keys().copied()
    }

    /// The effective status of a unit: the runtime override if one is
    /// set, otherwise the unit's self-reported `status()`.
    ///
    /// The scheduler and the status hash use this instead of
    /// `unit.status()` directly so that block/wake transitions driven
    /// by the commit pipeline are respected.
    pub fn effective_status(&self, id: UnitId) -> Option<UnitStatus> {
        let unit = self.units.get(&id)?;
        Some(
            self.status_overrides
                .get(&id)
                .copied()
                .unwrap_or_else(|| unit.status()),
        )
    }

    /// Set a runtime-side status override for `id`. Takes precedence
    /// over the unit's self-reported `status()` until cleared via
    /// [`UnitRegistry::clear_status_override`].
    ///
    /// The commit pipeline uses this to block units (e.g. on empty
    /// mailbox receive) and to wake them (e.g. on `WakeUnit`). The
    /// runtime clears the override when it next runs the unit, so the
    /// unit's own status logic resumes control after one step.
    pub fn set_status_override(&mut self, id: UnitId, status: UnitStatus) {
        if self.units.contains_key(&id) {
            self.status_overrides.insert(id, status);
        }
    }

    /// Clear a previously-set status override. After this call,
    /// [`UnitRegistry::effective_status`] delegates to the unit's
    /// self-reported `status()` again.
    pub fn clear_status_override(&mut self, id: UnitId) {
        self.status_overrides.remove(&id);
    }

    /// Push a received message into a unit's per-unit inbox. The
    /// commit pipeline calls this when `MailboxReceiveAttempt`
    /// successfully pops a message from a non-empty mailbox.
    pub fn push_receive(&mut self, id: UnitId, message: u32) {
        self.pending_receives.entry(id).or_default().push(message);
    }

    /// Drain all pending received messages for `id`, returning them
    /// in the order they were pushed. Returns an empty vec if there
    /// are no pending messages. The runtime calls this at the start
    /// of each `run_until_yield` to build the `ExecutionContext`.
    pub fn drain_receives(&mut self, id: UnitId) -> Vec<u32> {
        self.pending_receives.remove(&id).unwrap_or_default()
    }

    /// 64-bit deterministic hash of the ordered set of unit ids whose
    /// effective status is `Runnable`.
    ///
    /// Used as the `RunnableQueue` checkpoint hash. FNV-1a over the `id.raw()`
    /// le-bytes of each runnable unit, in id order. The empty set
    /// (no runnable units) hashes to the FNV-1a empty-input value.
    pub fn runnable_queue_hash(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = FNV_OFFSET;
        for id in self.units.keys() {
            if self.effective_status(*id) == Some(UnitStatus::Runnable) {
                for b in id.raw().to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(FNV_PRIME);
                }
            }
        }
        h
    }

    /// 64-bit deterministic hash of every unit's (id, effective status)
    /// pair in id order.
    ///
    /// Used as the `UnitStatus` checkpoint hash. FNV-1a, no host-time inputs, no
    /// external deps. Walks the underlying [`BTreeMap`] in id order so
    /// the result is independent of registration history.
    ///
    /// Replay tooling compares pairs of these values to assert that
    /// two runs reached the same set of unit statuses. The empty
    /// registry hashes to the FNV-1a empty-input value, which the
    /// runtime trace records on its first checkpoint.
    pub fn status_hash(&self) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut h = FNV_OFFSET;
        for (id, _unit) in self.units.iter() {
            for b in id.raw().to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(FNV_PRIME);
            }
            let status_byte =
                self.effective_status(*id)
                    .expect("unit in map must have effective status") as u8;
            h ^= status_byte as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_effects::Effect;
    use cellgov_exec::{LocalDiagnostics, YieldReason};
    use cellgov_mem::GuestMemory;

    /// A minimal fake unit that tracks how many times it has been
    /// stepped. The same shape as the test impl in `cellgov_exec::unit`,
    /// duplicated here so the registry tests do not depend on private
    /// test code in another crate.
    struct CountingUnit {
        id: UnitId,
        steps: u64,
    }

    impl ExecutionUnit for CountingUnit {
        type Snapshot = u64;

        fn unit_id(&self) -> UnitId {
            self.id
        }

        fn status(&self) -> UnitStatus {
            UnitStatus::Runnable
        }

        fn run_until_yield(
            &mut self,
            budget: Budget,
            _ctx: &ExecutionContext<'_>,
        ) -> ExecutionStepResult {
            self.steps += 1;
            ExecutionStepResult {
                yield_reason: YieldReason::BudgetExhausted,
                consumed_budget: budget,
                emitted_effects: vec![Effect::TraceMarker {
                    marker: self.steps as u32,
                    source: self.id,
                }],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
            }
        }

        fn snapshot(&self) -> u64 {
            self.steps
        }
    }

    /// A unit that lies about its id. Used to verify the registry
    /// rejects units whose `unit_id()` disagrees with the assigned id.
    struct LyingUnit;

    impl ExecutionUnit for LyingUnit {
        type Snapshot = ();

        fn unit_id(&self) -> UnitId {
            UnitId::new(999)
        }

        fn status(&self) -> UnitStatus {
            UnitStatus::Runnable
        }

        fn run_until_yield(
            &mut self,
            budget: Budget,
            _ctx: &ExecutionContext<'_>,
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                emitted_effects: vec![],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
            }
        }

        fn snapshot(&self) {}
    }

    #[test]
    fn new_is_empty() {
        let r = UnitRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(r.ids().count(), 0);
    }

    #[test]
    fn register_assigns_sequential_ids() {
        let mut r = UnitRegistry::new();
        let a = r.register_with(|id| CountingUnit { id, steps: 0 });
        let b = r.register_with(|id| CountingUnit { id, steps: 0 });
        let c = r.register_with(|id| CountingUnit { id, steps: 0 });
        assert_eq!(a, UnitId::new(0));
        assert_eq!(b, UnitId::new(1));
        assert_eq!(c, UnitId::new(2));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn get_returns_registered_unit() {
        let mut r = UnitRegistry::new();
        let id = r.register_with(|id| CountingUnit { id, steps: 0 });
        let u = r.get(id).expect("present");
        assert_eq!(u.unit_id(), id);
        assert_eq!(u.status(), UnitStatus::Runnable);
    }

    #[test]
    fn get_missing_is_none() {
        let r = UnitRegistry::new();
        assert!(r.get(UnitId::new(99)).is_none());
    }

    #[test]
    fn get_mut_drives_run_until_yield() {
        let mut r = UnitRegistry::new();
        let id = r.register_with(|id| CountingUnit { id, steps: 0 });
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let u = r.get_mut(id).expect("present");
        let step = u.run_until_yield(Budget::new(5), &ctx);
        assert_eq!(step.consumed_budget, Budget::new(5));
        assert_eq!(step.emitted_effects.len(), 1);
    }

    #[test]
    fn iter_is_in_id_order() {
        let mut r = UnitRegistry::new();
        for _ in 0..4 {
            r.register_with(|id| CountingUnit { id, steps: 0 });
        }
        let ids: Vec<u64> = r.iter().map(|(id, _)| id.raw()).collect();
        assert_eq!(ids, vec![0, 1, 2, 3]);
    }

    #[test]
    fn ids_iterator_matches_registration_order() {
        let mut r = UnitRegistry::new();
        for _ in 0..3 {
            r.register_with(|id| CountingUnit { id, steps: 0 });
        }
        let collected: Vec<UnitId> = r.ids().collect();
        assert_eq!(
            collected,
            vec![UnitId::new(0), UnitId::new(1), UnitId::new(2)]
        );
    }

    #[test]
    fn iter_mut_can_step_every_unit() {
        let mut r = UnitRegistry::new();
        for _ in 0..3 {
            r.register_with(|id| CountingUnit { id, steps: 0 });
        }
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let mut total = 0;
        for (_, u) in r.iter_mut() {
            let step = u.run_until_yield(Budget::new(1), &ctx);
            total += step.emitted_effects.len();
        }
        assert_eq!(total, 3);
    }

    /// A unit whose status is held in a shared `Rc<Cell<_>>` so tests
    /// can flip it from outside the registry and observe the effect on
    /// `status_hash`.
    struct StatusUnit {
        id: UnitId,
        status: std::rc::Rc<std::cell::Cell<UnitStatus>>,
    }

    impl ExecutionUnit for StatusUnit {
        type Snapshot = ();
        fn unit_id(&self) -> UnitId {
            self.id
        }
        fn status(&self) -> UnitStatus {
            self.status.get()
        }
        fn run_until_yield(
            &mut self,
            budget: Budget,
            _ctx: &ExecutionContext<'_>,
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
                emitted_effects: vec![],
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
            }
        }
        fn snapshot(&self) {}
    }

    fn status_unit(s: UnitStatus) -> (StatusHandle, impl FnOnce(UnitId) -> StatusUnit) {
        let cell = std::rc::Rc::new(std::cell::Cell::new(s));
        let cell_for_factory = cell.clone();
        (StatusHandle(cell), move |id| StatusUnit {
            id,
            status: cell_for_factory,
        })
    }

    struct StatusHandle(std::rc::Rc<std::cell::Cell<UnitStatus>>);
    impl StatusHandle {
        fn set(&self, s: UnitStatus) {
            self.0.set(s);
        }
    }

    #[test]
    fn status_hash_of_empty_registry_is_stable() {
        let a = UnitRegistry::new();
        let b = UnitRegistry::new();
        assert_eq!(a.status_hash(), b.status_hash());
    }

    #[test]
    fn status_hash_changes_when_a_unit_status_changes() {
        let mut r = UnitRegistry::new();
        let (handle, factory) = status_unit(UnitStatus::Runnable);
        r.register_with(factory);
        let h0 = r.status_hash();
        handle.set(UnitStatus::Blocked);
        let h1 = r.status_hash();
        handle.set(UnitStatus::Finished);
        let h2 = r.status_hash();
        assert_ne!(h0, h1);
        assert_ne!(h1, h2);
        assert_ne!(h0, h2);
    }

    #[test]
    fn status_hash_distinguishes_each_status_variant() {
        // Four registries each holding one unit at a different status.
        // All four hashes must differ; if any pair collide, the byte
        // map in status_hash is broken.
        fn one(s: UnitStatus) -> u64 {
            let mut r = UnitRegistry::new();
            let (_h, factory) = status_unit(s);
            r.register_with(factory);
            r.status_hash()
        }
        let all: std::collections::BTreeSet<u64> = [
            one(UnitStatus::Runnable),
            one(UnitStatus::Blocked),
            one(UnitStatus::Faulted),
            one(UnitStatus::Finished),
        ]
        .into_iter()
        .collect();
        assert_eq!(all.len(), 4);
    }

    #[test]
    fn effective_status_returns_unit_self_report_by_default() {
        let mut r = UnitRegistry::new();
        let (handle, factory) = status_unit(UnitStatus::Runnable);
        let id = r.register_with(factory);
        assert_eq!(r.effective_status(id), Some(UnitStatus::Runnable));
        handle.set(UnitStatus::Finished);
        assert_eq!(r.effective_status(id), Some(UnitStatus::Finished));
    }

    #[test]
    fn set_status_override_takes_precedence_over_unit() {
        let mut r = UnitRegistry::new();
        let (_handle, factory) = status_unit(UnitStatus::Runnable);
        let id = r.register_with(factory);
        r.set_status_override(id, UnitStatus::Blocked);
        assert_eq!(r.effective_status(id), Some(UnitStatus::Blocked));
    }

    #[test]
    fn clear_status_override_restores_unit_self_report() {
        let mut r = UnitRegistry::new();
        let (_handle, factory) = status_unit(UnitStatus::Runnable);
        let id = r.register_with(factory);
        r.set_status_override(id, UnitStatus::Blocked);
        assert_eq!(r.effective_status(id), Some(UnitStatus::Blocked));
        r.clear_status_override(id);
        assert_eq!(r.effective_status(id), Some(UnitStatus::Runnable));
    }

    #[test]
    fn status_override_affects_status_hash() {
        let mut r = UnitRegistry::new();
        let (_handle, factory) = status_unit(UnitStatus::Runnable);
        let id = r.register_with(factory);
        let h_runnable = r.status_hash();
        r.set_status_override(id, UnitStatus::Blocked);
        let h_blocked = r.status_hash();
        assert_ne!(h_runnable, h_blocked);
        r.clear_status_override(id);
        assert_eq!(r.status_hash(), h_runnable);
    }

    #[test]
    fn runnable_queue_hash_changes_when_unit_becomes_blocked() {
        let mut r = UnitRegistry::new();
        let (handle, factory) = status_unit(UnitStatus::Runnable);
        let _id = r.register_with(factory);
        let h_runnable = r.runnable_queue_hash();
        handle.set(UnitStatus::Blocked);
        let h_blocked = r.runnable_queue_hash();
        assert_ne!(h_runnable, h_blocked);
        handle.set(UnitStatus::Runnable);
        assert_eq!(r.runnable_queue_hash(), h_runnable);
    }

    #[test]
    fn runnable_queue_hash_empty_when_no_runnable_units() {
        let mut r = UnitRegistry::new();
        let (_h, factory) = status_unit(UnitStatus::Finished);
        r.register_with(factory);
        let empty_reg = UnitRegistry::new();
        assert_eq!(r.runnable_queue_hash(), empty_reg.runnable_queue_hash());
    }

    #[test]
    fn set_status_override_on_unknown_id_is_noop() {
        let mut r = UnitRegistry::new();
        // No units registered. Should not panic.
        r.set_status_override(UnitId::new(99), UnitStatus::Blocked);
        assert!(r.effective_status(UnitId::new(99)).is_none());
    }

    #[test]
    fn status_hash_is_id_position_sensitive() {
        // Same status set, different id assignments must hash
        // differently. Construct two registries with one Runnable
        // unit each, but force the second one to assign a higher id
        // by burning a slot.
        let mut a = UnitRegistry::new();
        let (_ha, factory_a) = status_unit(UnitStatus::Runnable);
        a.register_with(factory_a);

        let mut b = UnitRegistry::new();
        let (_burn, burn_factory) = status_unit(UnitStatus::Finished);
        b.register_with(burn_factory);
        let (_hb, factory_b) = status_unit(UnitStatus::Runnable);
        b.register_with(factory_b);
        assert_ne!(a.status_hash(), b.status_hash());
    }

    #[test]
    #[should_panic(expected = "registered unit reported")]
    fn factory_id_mismatch_panics() {
        let mut r = UnitRegistry::new();
        // LyingUnit always reports id 999, never the assigned id.
        r.register_with(|_assigned| LyingUnit);
    }
}
